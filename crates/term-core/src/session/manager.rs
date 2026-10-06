//! Gestor de pestañas y espacios de trabajo.
//!
//! Mantiene el conjunto de sesiones vivas, cuál tiene el foco y sus ciclos de
//! vida: crear, seleccionar, cerrar y retirar las que su shell terminó. La UI
//! decide cuándo cerrar la última ventana; aquí solo se administran sesiones.

use std::collections::BTreeMap;

use crate::frame::{Frame, Style};
use crate::input::KeyMode;
use crate::pty::{PtyConfig, RunningApp};

use super::{GridSize, MousePolicy, Session};

/// Gestor de múltiples sesiones de terminal vivas (espacios de trabajo / pestañas).
pub struct SessionManager {
    sessions: BTreeMap<usize, Session>,
    active_id: usize,
    next_id: usize,
    default_config: PtyConfig,
    size: GridSize,
    /// Política de ratón y selección vigente.
    ///
    /// Es del gestor y no de la pestaña: un cambio debe regir también al cambiar
    /// de pestaña, y una sesión nueva debe nacer con la que está vigente.
    mouse_policy: MousePolicy,
}

impl SessionManager {
    /// Inicia el gestor con una primera sesión interactiva (ID = 0).
    pub fn new(config: PtyConfig, size: GridSize) -> std::io::Result<Self> {
        let first = Session::spawn(config.clone(), size)?;
        let mut map = BTreeMap::new();
        map.insert(0, first);
        Ok(Self {
            sessions: map,
            active_id: 0,
            next_id: 1,
            default_config: config,
            size,
            mouse_policy: MousePolicy::default(),
        })
    }

    /// Crea y añade una nueva sesión de terminal con su propio proceso PTY y la activa.
    /// Devuelve el identificador único estable asignado a la nueva sesión.
    pub fn spawn_session(&mut self) -> std::io::Result<usize> {
        self.spawn_session_config(self.default_config.clone())
    }

    /// Crea y añade una sesión que ejecuta el programa indicado y la activa.
    pub fn spawn_session_with(&mut self, config: PtyConfig) -> std::io::Result<usize> {
        self.spawn_session_config(config)
    }

    /// Arranca el PTY de la sesión nueva, la registra y le da el foco.
    fn spawn_session_config(&mut self, config: PtyConfig) -> std::io::Result<usize> {
        let mut session = Session::spawn(config, self.size)?;
        // La sesión nueva hereda la política vigente; su dueño es el gestor.
        session.set_mouse_policy(self.mouse_policy);
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.insert(id, session);
        self.active_id = id;
        Ok(id)
    }

    /// Aplica una política de ratón y selección a todas las sesiones y la deja
    /// como la vigente para las que se creen después.
    ///
    /// Se aplica a todas y no solo a la activa: un cambio de política debe regir
    /// también al cambiar de pestaña.
    pub fn set_mouse_policy(&mut self, policy: MousePolicy) {
        self.mouse_policy = policy;
        for session in self.sessions.values_mut() {
            session.set_mouse_policy(policy);
        }
    }

    /// Selecciona la sesión activa por su ID. Devuelve `true` si el ID existe.
    pub fn select(&mut self, id: usize) -> bool {
        if self.sessions.contains_key(&id) {
            self.active_id = id;
            true
        } else {
            false
        }
    }

    /// Cierra una sesión por ID si hay más de una viva.
    /// Elimina las sesiones cuyo shell ya terminó. Devuelve los identificadores
    /// retirados para que la UI pueda quitar sus pestañas y espacios.
    ///
    /// Sin esto, ejecutar `exit` dejaba la sesión muerta en el gestor y la
    /// ventana se quedaba congelada mostrando la última rejilla.
    pub fn reap_exited(&mut self) -> Vec<usize> {
        let mut removed = Vec::new();
        self.sessions.retain(|&id, session| {
            let alive = !session.has_exited();
            if !alive {
                removed.push(id);
            }
            alive
        });

        if !self.sessions.is_empty() && !self.sessions.contains_key(&self.active_id) {
            self.active_id = *self.sessions.keys().next().unwrap();
        }

        removed
    }

    /// Cierra una sesión por su identificador.
    ///
    /// La última sesión viva no se puede cerrar desde aquí: una terminal sin
    /// sesiones debe cerrar su ventana, no quedarse hungueada. Eso lo decide la
    /// UI, que llama a `remove_window` cuando el gestor se queda vacío.
    pub fn close(&mut self, id: usize) -> bool {
        if self.sessions.len() > 1 && self.sessions.contains_key(&id) {
            self.sessions.remove(&id);
            if self.active_id == id {
                if let Some(&first_key) = self.sessions.keys().next() {
                    self.active_id = first_key;
                }
            }
            true
        } else {
            false
        }
    }

    /// Referencia a la sesión activa.
    pub fn active_session(&self) -> &Session {
        &self.sessions[&self.active_id]
    }

    /// Referencia mutable a la sesión activa.
    pub fn active_session_mut(&mut self) -> &mut Session {
        self.sessions
            .get_mut(&self.active_id)
            .expect("sesión activa")
    }

    /// ID de la sesión activa actual.
    pub fn active_id(&self) -> usize {
        self.active_id
    }

    /// Para compatibilidad con active_index.
    pub fn active_index(&self) -> usize {
        self.active_id
    }

    /// Cantidad de sesiones vivas.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Devuelve `true` si no hay sesiones.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Bombea todas las sesiones vivas. Devuelve `true` si la sesión activa tuvo cambios.
    pub fn pump(&mut self) -> bool {
        let mut active_changed = false;
        for (&id, session) in self.sessions.iter_mut() {
            let changed = session.pump();
            if id == self.active_id && changed {
                active_changed = true;
            }
        }
        active_changed
    }

    /// Redimensiona todas las sesiones a la geometría dada.
    pub fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        self.size = size;
        for session in self.sessions.values_mut() {
            session.resize(size)?;
        }
        Ok(())
    }

    /// Tamaño actual en celdas.
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Cuadro de la sesión activa para renderizado.
    pub fn frame(&self) -> Frame {
        self.active_session().frame()
    }

    /// Escribe bytes en el PTY de la sesión activa.
    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.active_session_mut().write(bytes)
    }

    /// Modo de cursor de la sesión activa.
    pub fn cursor_key_mode(&self) -> KeyMode {
        self.active_session().cursor_key_mode()
    }

    /// Estilo por defecto de la sesión activa.
    pub fn default_style(&self) -> Style {
        self.active_session().default_style()
    }

    /// Mapa de todas las sesiones vivas.
    pub fn sessions(&self) -> &BTreeMap<usize, Session> {
        &self.sessions
    }

    /// Programas en primer plano de todas las sesiones, por ID de sesión.
    pub fn running_apps(&self) -> Vec<(usize, RunningApp)> {
        self.sessions
            .iter()
            .filter_map(|(&id, session)| session.foreground_app().map(|app| (id, app)))
            .collect()
    }

    /// `true` si alguna sesión tiene un programa en primer plano.
    pub fn has_running_app(&self) -> bool {
        self.sessions.values().any(|s| s.foreground_app().is_some())
    }

    /// Obtiene el directorio de trabajo de una sesión por su ID.
    pub fn session_cwd(&self, id: usize) -> Option<std::path::PathBuf> {
        self.sessions
            .get(&id)
            .and_then(|s| s.current_working_directory())
    }

    /// Obtiene el nombre de la carpeta de una sesión por su ID.
    pub fn session_folder_name(&self, id: usize) -> Option<String> {
        self.sessions.get(&id).and_then(|s| s.current_folder_name())
    }

    /// Directorio de trabajo de la sesión activa actual.
    pub fn active_cwd(&self) -> Option<std::path::PathBuf> {
        self.session_cwd(self.active_id)
    }

    /// Programa en primer plano de la sesión indicada, si lo hay.
    pub fn session_app(&self, id: usize) -> Option<RunningApp> {
        self.sessions.get(&id).and_then(|s| s.foreground_app())
    }

    /// Programa en primer plano de la sesión activa actual, si lo hay.
    pub fn active_app(&self) -> Option<RunningApp> {
        self.session_app(self.active_id)
    }
}
