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
                let mut buffer = [0u8; 8192];
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
        let mut bytes = Vec::new();
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
}

fn to_pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}
