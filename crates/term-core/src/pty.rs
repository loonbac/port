//! Adaptador del PTY: arranca el shell y expone su tubería.
//!
//! Único módulo que habla con el sistema operativo. La UI y la rejilla no saben
//! que existe un pseudo-terminal.
//!
//! La lectura la hace un hilo propio. Es necesario, no un adorno: el maestro del
//! PTY es un descriptor bloqueante, así que leer desde el hilo de la UI la
//! congelaría en cuanto el shell dejara de escribir. El hilo bloquea, y la UI
//! solo recoge lo que ya está listo.
//!
//! Lo recogido vive en una cola acotada ([`MAX_PENDING_BYTES`]). El tope no es
//! una optimización: sin él, un programa que escribe más rápido de lo que la UI
//! pinta crece la memoria sin límite. Con tope, lo viejo se descarta y el final
//! del flujo es lo que se ve, que es el comportamiento que hace usable
//! `cat archivo-grande`.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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
    /// Variables de entorno extra para el proceso, además de las que PORT fija.
    ///
    /// Se aplican después de `TERM`, `COLORTERM` y `TERM_PROGRAM`, así que
    /// pueden reemplazarlas. Es el punto por el que un plugin describe el
    /// entorno de sus paneles: el núcleo no necesita conocer a ningún programa
    /// en particular para que otro pueda publicarse dentro de la terminal.
    pub env: Vec<(String, String)>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        Self {
            command: shell,
            args: vec!["-l".to_string()],
            cwd: std::env::var("HOME").ok(),
            env: Vec::new(),
        }
    }
}

/// Proceso de shell conectado a un pseudo-terminal.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Salida ya leída del PTY y todavía sin parsear, acotada por
    /// [`MAX_PENDING_BYTES`].
    inbox: Arc<Mutex<Inbox>>,
    /// El hilo lector terminó: el otro extremo cerró el PTY.
    closed: Arc<AtomicBool>,
    /// Descriptor del maestro del PTY. Permite preguntar al núcleo qué grupo de
    /// procesos está en primer plano, que es lo que distingue un job en primer
    /// plano de otro lanzado con `&`.
    /// Descriptor del master, solo para consultar el grupo en primer plano.
    ///
    /// Es un concepto de Unix: en Windows no existe equivalente a `tcgetpgrp`,
    /// asi que el campo no se compila alli en lugar de valuar cero y fingir
    /// que funciona.
    #[cfg(unix)]
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
        // Al final: lo que pida quien lanza el panel gana sobre lo que fija PORT.
        for (name, value) in &config.env {
            command.env(name, value);
        }

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

        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let closed = Arc::new(AtomicBool::new(false));
        let closed_for_thread = Arc::clone(&closed);
        let inbox_for_thread = Arc::clone(&inbox);
        std::thread::Builder::new()
            .name("port-pty-reader".to_string())
            .spawn(move || {
                let mut buffer = [0u8; 65536];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            lock_inbox(&inbox_for_thread).push(&buffer[..read]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                closed_for_thread.store(true, Ordering::SeqCst);
            })
            .map_err(std::io::Error::other)?;

        Ok(Self {
            #[cfg(unix)]
            master_fd: pair.master.as_raw_fd(),
            master: pair.master,
            writer,
            child,
            inbox,
            closed,
            stable_app: RefCell::new(None),
            pending_app: RefCell::new((None, 0)),
        })
    }

    /// Hasta `budget` bytes de salida ya disponibles.
    ///
    /// **Nunca bloquea**: si no hay nada, devuelve vacío. La cuota existe porque
    /// la UI llama una vez por cuadro: entregar todo el atraso de golpe convierte
    /// un diluvio de salida en una pausa de la interfaz. Lo que no entra en la
    /// cuota se queda en la cola para el cuadro siguiente.
    pub fn try_read(&mut self, budget: usize) -> Vec<u8> {
        lock_inbox(&self.inbox).take(budget)
    }

    /// Bytes que esperan en la cola sin parsear.
    pub fn pending_bytes(&self) -> usize {
        lock_inbox(&self.inbox).pending()
    }

    /// Bytes que el tope de la cola ha descartado desde el arranque.
    pub fn dropped_bytes(&self) -> u64 {
        lock_inbox(&self.inbox).dropped
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

            pid = nested_shell?;
        }
        None
    }

    /// Grupo de procesos en primer plano del terminal, según el núcleo.
    #[cfg(unix)]
    fn foreground_process_group(&self) -> Option<i32> {
        let fd = self.master_fd?;
        sysinfo::foreground_group(fd)
    }

    /// Sin equivalente en Windows: `tcgetpgrp` no existe fuera de Unix.
    ///
    /// Se devuelve `None` en vez de inventar un numero. La consecuencia es
    /// visible y acotada: la deteccion de que programa esta en primer plano no
    /// funciona ahi, y las funciones que dependen de ella lo dicen.
    #[cfg(not(unix))]
    fn foreground_process_group(&self) -> Option<i32> {
        None
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

/// Cuánta salida del PTY se conserva sin parsear.
///
/// Cuatro MiB son varias pantallas de scrollback en el peor caso de texto denso,
/// y una imagen sixel completa en el mejor. Por encima de eso, lo que llega
/// tarde ya no se puede pintar: descartarlo es preferible a crecer sin límite.
pub const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;

/// Cola de salida sin parsear, acotada por [`MAX_PENDING_BYTES`].
///
/// Es un `Vec` lineal con un índice de cabeza, no un ring de bytes: empujar y
/// tomar son memoria contigua (memcpy), sin recorrer el buffer byte a byte. La
/// compactación se paga una vez cada varios MiB y no en cada lectura.
#[derive(Default)]
struct Inbox {
    bytes: Vec<u8>,
    /// Cuántos bytes del principio ya se entregaron o se descartaron.
    head: usize,
    /// Bytes que el tope descartó desde el arranque.
    dropped: u64,
}

impl Inbox {
    /// Bytes esperando ser entregados.
    fn pending(&self) -> usize {
        self.bytes.len() - self.head
    }

    /// Añade lo recién leído, descartando lo más viejo si hace falta.
    fn push(&mut self, chunk: &[u8]) {
        // Un trozo que por sí solo supera el tope reemplaza todo lo anterior: lo
        // único que puede quedar es su final, que es lo más nuevo del flujo.
        if chunk.len() >= MAX_PENDING_BYTES {
            self.dropped += (self.pending() + chunk.len() - MAX_PENDING_BYTES) as u64;
            self.bytes.clear();
            self.head = 0;
            self.bytes.extend_from_slice(&chunk[chunk.len() - MAX_PENDING_BYTES..]);
            return;
        }

        let overflow = (self.pending() + chunk.len()).saturating_sub(MAX_PENDING_BYTES);
        if overflow > 0 {
            self.head += overflow;
            self.dropped += overflow as u64;
        }
        self.bytes.extend_from_slice(chunk);
        self.compact();
    }

    /// Retira hasta `budget` bytes, de lo más viejo a lo más nuevo.
    fn take(&mut self, budget: usize) -> Vec<u8> {
        let end = (self.head + budget).min(self.bytes.len());
        let taken = self.bytes[self.head..end].to_vec();
        self.head = end;
        self.compact();
        taken
    }

    /// Devuelve el buffer a cero cuando la cabeza ya ocupa demasiado.
    ///
    /// Sin esto, el `Vec` solo crecería: la cabeza avanza y los bytes entregados
    /// quedan al frente para siempre. Compactar a la mitad del tope deja el
    /// coste amortizado aunque el flujo no pare.
    fn compact(&mut self) {
        if self.head == 0 {
            return;
        }
        if self.head == self.bytes.len() {
            self.bytes.clear();
            self.head = 0;
        } else if self.head >= MAX_PENDING_BYTES / 2 {
            self.bytes.drain(..self.head);
            self.head = 0;
        }
    }
}

/// Toma el candado de la cola sobreviviendo a un pánico ajeno.
///
/// Que otro hilo haya entrado en pánico no debe cortar la salida del shell: la
/// cola sigue siendo un buffer de bytes válido.
fn lock_inbox(inbox: &Arc<Mutex<Inbox>>) -> MutexGuard<'_, Inbox> {
    inbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
