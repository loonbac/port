//! Adaptador del PTY: arranca el shell y expone su tubería.
//!
//! Único módulo que habla con el sistema operativo. La UI y la rejilla no saben
//! que existe un pseudo-terminal.
//!
//! La lectura la hace un hilo propio. Es necesario, no un adorno: el maestro del
//! PTY es un descriptor bloqueante, así que leer desde el hilo de la UI la
//! congelaría en cuanto el shell dejara de escribir. El hilo bloquea, y la UI
//! solo recoge lo que ya está listo.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::session::GridSize;
use crate::sysinfo;

/// Cómo arrancar el shell.
#[derive(Clone, Debug)]
pub struct PtyConfig {
    /// Programa a ejecutar; por defecto, `$SHELL -l`.
    pub command: String,
    /// Argumentos.
    pub args: Vec<String>,
    /// Directorio de trabajo inicial.
    pub cwd: Option<String>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        Self {
            command: shell,
            args: vec!["-l".to_string()],
            cwd: std::env::var("HOME").ok(),
        }
    }
}

/// Proceso de shell conectado a un pseudo-terminal.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    incoming: Receiver<Vec<u8>>,
    /// El hilo lector terminó: el otro extremo cerró el PTY.
    closed: Arc<AtomicBool>,
    /// Descriptor del maestro del PTY. Permite preguntar al núcleo qué grupo de
    /// procesos está en primer plano, que es lo que distingue un job en primer
    /// plano de otro lanzado con `&`.
    master_fd: Option<std::os::raw::c_int>,
    /// Último programa en primer plano ya confirmado como estable.
    stable_app: RefCell<Option<RunningApp>>,
    /// Candidato actual y cuántas sondeos seguidos lo han respaldado.
    pending_app: RefCell<(Option<RunningApp>, u8)>,
}

/// Cuántas lecturas seguidas deben coincidir antes de dar por estable un programa.
/// El renderizador del prompt y otros auxiliares viven menos de 32 ms, así que
/// exigir dos coincidencias evita que aparezcan un fotograma en las pestañas.
const STABILITY_SAMPLES: u8 = 2;

impl Pty {
    pub fn spawn(config: PtyConfig, size: GridSize) -> std::io::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(to_pty_size(size))
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let mut command = CommandBuilder::new(&config.command);
        command.args(&config.args);
        if let Some(cwd) = &config.cwd {
            command.cwd(cwd);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "port");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let (sender, incoming) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let closed_for_thread = Arc::clone(&closed);
        std::thread::Builder::new()
            .name("port-pty-reader".to_string())
            .spawn(move || {
                let mut buffer = [0u8; 65536];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            if sender.send(buffer[..read].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                closed_for_thread.store(true, Ordering::SeqCst);
            })
            .map_err(std::io::Error::other)?;

        Ok(Self {
            master_fd: pair.master.as_raw_fd(),
            master: pair.master,
            writer,
            child,
            incoming,
            closed,
            stable_app: RefCell::new(None),
            pending_app: RefCell::new((None, 0)),
        })
    }

    /// Bytes ya disponibles. **Nunca bloquea**: si no hay nada, devuelve vacío.
    pub fn try_read(&mut self) -> Vec<u8> {
        let first = match self.incoming.try_recv() {
            Ok(chunk) => chunk,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Vec::new(),
        };

        // Si solo hay un fragmento (el caso habitual), lo devolvemos directo
        // sin asignar un segundo vector ni copiar bytes.
        let second = match self.incoming.try_recv() {
            Ok(chunk) => chunk,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return first,
        };

        let mut bytes = first;
        bytes.extend_from_slice(&second);
        while let Ok(chunk) = self.incoming.try_recv() {
            bytes.extend_from_slice(&chunk);
        }
        bytes
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.master
            .resize(to_pty_size(size))
            .map_err(|error| std::io::Error::other(error.to_string()))
    }

    /// El otro extremo cerró la salida.
    pub fn output_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// El shell terminó.
    pub fn has_exited(&mut self) -> bool {
        if self.output_closed() {
            return true;
        }
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Devuelve el PID del proceso de shell hijo, si está disponible.
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Obtiene el directorio de trabajo actual (cwd) del shell en tiempo real.
    pub fn current_working_directory(&self) -> Option<std::path::PathBuf> {
        let pid = self.process_id()?;
        sysinfo::cwd_of(pid)
    }

    /// Detecta el programa que se está ejecutando en primer plano.
    ///
    /// Recorre el árbol de hijos del shell por `/proc` y devuelve el primer
    /// descendiente que no es otro shell ni un auxiliar del prompt. El resultado
    /// debe confirmarse en dos sondeos seguidos antes de devolverse, para que
    /// procesos muy breves (el prompt, hooks de entorno) nunca lleguen a la UI.
    ///
    /// La confirmación es simétrica: un valor, sea un programa o la ausencia de
    /// él, necesita dos lecturas seguidas para volverse el estable. Sin esto un
    /// programa que termina se quedaría congelado en la pestaña para siempre.
    pub fn foreground_app(&self) -> Option<RunningApp> {
        let raw = self.detect_foreground_app();

        let mut stable = self.stable_app.borrow_mut();
        let mut pending = self.pending_app.borrow_mut();

        if pending.0 == raw {
            pending.1 = pending.1.saturating_add(1);
        } else {
            pending.0 = raw;
            pending.1 = 1;
        }

        if pending.1 >= STABILITY_SAMPLES {
            *stable = pending.0.clone();
        }

        stable.clone()
    }

    /// Lectura cruda del proceso en primer plano, sin estabilizar.
    ///
    /// Solo se aceptan procesos que pertenecen al grupo de primer plano del
    /// terminal. Un job lanzado con `&` sigue siendo hijo del shell, pero el
    /// núcleo lo coloca en otro grupo, así que queda descartado.
    fn detect_foreground_app(&self) -> Option<RunningApp> {
        let foreground = self.foreground_process_group();
        let mut pid = self.process_id()?;

        // El límite evita un bucle infinito si `/proc` devolviera un ciclo.
        for _ in 0..8 {
            let children = sysinfo::children_of(pid);
            if children.is_empty() {
                return None;
            }

            let mut nested_shell = None;
            for child in children {
                let Some(bin) = sysinfo::name_of(child) else {
                    continue;
                };
                let app = RunningApp { pid: child, bin };
                if is_prompt_helper(&app.bin) {
                    continue;
                }
                if is_shell_binary(&app.bin) {
                    nested_shell = Some(child);
                    continue;
                }
                // El shell en sí mismo pertenece al grupo en primer plano: si
                // no hay job corriendo, la terminal está en reposo.
                if is_in_group(app.pid, foreground) {
                    return Some(app);
                }
            }

            match nested_shell {
                Some(shell) => pid = shell,
                None => return None,
            }
        }
        None
    }

    /// Grupo de procesos en primer plano del terminal, según el núcleo.
    fn foreground_process_group(&self) -> Option<i32> {
        let fd = self.master_fd?;
        sysinfo::foreground_group(fd)
    }
}

/// Si un proceso pertenece al grupo indicado. Sin grupo conocido se acepta,
/// para no perder la detección donde la plataforma no lo consulta.
fn is_in_group(pid: u32, group: Option<i32>) -> bool {
    match group {
        None => true,
        Some(group) => sysinfo::process_group_of(pid) == Some(group),
    }
}

/// Programa que se está ejecutando en primer plano dentro de una sesión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    /// PID del proceso.
    pub pid: u32,
    /// Nombre corto del binario, sin ruta ni extensión.
    pub bin: String,
}

impl RunningApp {
    /// Nombre del proceso tal como lo muestra el gestor de tareas del sistema.
    pub fn title(&self) -> String {
        self.bin.clone()
    }
}

/// Si el binario es un shell (o un multiplexor) y por tanto debe atravesarse
/// para encontrar el programa real que corre por delante.
fn is_shell_binary(bin: &str) -> bool {
    matches!(
        bin,
        "fish"
            | "bash"
            | "zsh"
            | "sh"
            | "dash"
            | "ksh"
            | "tcsh"
            | "csh"
            | "nu"
            | "elvish"
            | "xonsh"
            | "pwsh"
            | "powershell"
            | "screen"
            | "tmux"
            | "mosh"
            | "ssh"
            | "login"
    )
}

/// Auxiliares que el shell lanza a su alrededor y que nunca son la app en
/// primer plano: renderizadores del prompt y gestores de entorno.
fn is_prompt_helper(bin: &str) -> bool {
    matches!(
        bin,
        "oh-my-posh"
            | "starship"
            | "posh-git"
            | "omp"
            | "powerline"
            | "direnv"
            | "nix-direnv"
            | "atuin"
    )
}

fn to_pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}
