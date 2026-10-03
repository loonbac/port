//! Adaptador del PTY: arranca el shell y expone su tubería.
//!
//! Único módulo que habla con el sistema operativo. La UI y la rejilla no saben
//! que existe un pseudo-terminal.
//!
//! La lectura la hace un hilo propio. Es necesario, no un adorno: el maestro del
//! PTY es un descriptor bloqueante, así que leer desde el hilo de la UI la
//! congelaría en cuanto el shell dejara de escribir. El hilo bloquea, y la UI
//! solo recoge lo que ya está listo.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::session::GridSize;

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
}

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
            master: pair.master,
            writer,
            child,
            incoming,
            closed,
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
        loop {
            match self.incoming.try_recv() {
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
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
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    /// Detecta el programa que se está ejecutando en primer plano.
    ///
    /// Recorre el árbol de hijos del shell por `/proc` y devuelve el primer
    /// descendiente que no es otro shell. Cuando la terminal está en reposo no
    /// hay ningún hijo que no sea el shell, así que devuelve `None`.
    pub fn foreground_app(&self) -> Option<RunningApp> {
        let mut pid = self.process_id()?;

        // El límite evita un bucle infinito si `/proc` devolviera un ciclo.
        for _ in 0..8 {
            let children = read_children(pid);
            if children.is_empty() {
                return None;
            }

            let mut nested_shell = None;
            for child in children {
                let Some(app) = describe_process(child) else {
                    continue;
                };
                if is_shell_binary(&app.bin) {
                    nested_shell = Some(child);
                } else {
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

/// Hijos directos de un proceso, leídos de `/proc/{pid}/task/{pid}/children`.
fn read_children(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .map(|raw| {
            raw.split_whitespace()
                .filter_map(|token| token.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Nombre corto de un proceso, leído de `/proc/{pid}/comm`.
fn describe_process(pid: u32) -> Option<RunningApp> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let bin = comm.trim().to_lowercase();
    if bin.is_empty() {
        return None;
    }
    Some(RunningApp { pid, bin })
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

fn to_pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}
