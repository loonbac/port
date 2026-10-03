//! Introspección de procesos del sistema.
//!
//! La terminal necesita tres datos del proceso shell para dar contexto a la
//! interfaz: el directorio de trabajo actual, el grupo de procesos en primer
//! plano del terminal y el nombre del programa que se está ejecutando.
//!
//! Cada sistema operativo los expone de forma distinta, así que este módulo
//! aísla la diferencia detrás de un contrato común. Cuando una plataforma no
//! tiene forma de obtener un dato, se devuelve `None` y la interfaz simplemente
//! omite ese dato, en vez de fallar.

/// Directorio de trabajo actual de un proceso.
pub fn cwd_of(pid: u32) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Grupo de procesos al que pertenece un proceso.
pub fn process_group_of(pid: u32) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        // El nombre del proceso puede contener espacios y paréntesis, así que
        // los campos útiles son los que van tras el cierre del nombre.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let tail = stat.rsplit_once(')')?.1;
        let fields: Vec<&str> = tail.split_whitespace().collect();
        // tail[0] = state, tail[1] = ppid, tail[2] = pgrp
        fields.get(2)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// PIDs de los hijos directos de un proceso.
pub fn children_of(pid: u32) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            .map(|raw| {
                raw.split_whitespace()
                    .filter_map(|token| token.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Vec::new()
    }
}

/// Nombre corto del ejecutable de un proceso, en minúsculas.
pub fn name_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let name = comm.trim().to_lowercase();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Grupo de procesos que el terminal considera en primer plano.
///
/// Se consulta al núcleo a través del descriptor del PTY. Si la plataforma no
/// permite consultarlo, se devuelve `None` y quien llama decide si debe
///aplicar la comprobación.
pub fn foreground_group(tty_fd: std::os::raw::c_int) -> Option<i32> {
    #[cfg(unix)]
    {
        // SAFETY: `tcgetpgrp` solo lee el estado del terminal asociado al
        // descriptor; no puede dejar memoria en un estado inválido.
        let pgid = unsafe { libc::tcgetpgrp(tty_fd) };
        if pgid < 0 {
            None
        } else {
            Some(pgid)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tty_fd;
        None
    }
}
