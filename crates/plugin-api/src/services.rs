//! Directorio de servicios publicados entre plugins.
//!
//! Aquí vive una sola capacidad: un plugin anuncia lo que expone y otro lo
//! invoca por identificador estable, sin conocer su tipo concreto. Este módulo
//! no sabe nada de hooks, del registro de plugins ni de la ventana; por eso se
//! puede leer y probar por separado.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Argumento de una llamada entre plugins.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Unit,
    Num(f32),
    Text(String),
}

/// Resultado devuelto por un servicio.
#[derive(Debug, Clone, PartialEq)]
pub enum Ret {
    Unit,
    Num(f32),
    Text(String),
    Bool(bool),
}

/// Error al invocar un servicio.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceError {
    /// Ese identificador no corresponde a ningún plugin activo.
    UnknownService(String),
    /// El servicio existe pero no expone esa acción.
    UnknownAction { service: String, action: String },
    /// La acción se ejecutó pero devolvió un fallo.
    Failed(String),
}

/// Capacidad que un plugin publica para que otros puedan utilizarla.
///
/// Es la vía nativa para que un plugin llame a otro sin conocer su tipo
/// concreto: se pide por identificador estable y se invoca una acción por nombre.
pub trait Service: Send + Sync {
    /// Identificador estable con el que se busca el servicio. Suele coincidir con
    /// el `id()` del plugin que lo publica.
    fn id(&self) -> &str;

    /// Nombre legible, para diagnóstico y para la UI de configuración.
    fn name(&self) -> &str;

    /// Acciones que este servicio publica.
    fn actions(&self) -> Vec<&'static str>;

    /// Ejecuta una acción. `None` si la acción no existe en este servicio.
    fn invoke(&self, action: &str, args: &[Arg]) -> Option<Result<Ret, ServiceError>>;
}

/// Directorio de servicios publicados por los plugins.
///
/// Vive detrás de un `Arc` compartido: el registro lo crea, cada plugin publica
/// lo que expone y cualquier otro lo consulta por identificador, sin conocer su
/// implementación.
#[derive(Default)]
pub struct Services {
    inner: RwLock<HashMap<String, Arc<dyn Service>>>,
}

impl Services {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publica un servicio, reemplazando cualquier versión anterior.
    pub fn publish(&self, service: Arc<dyn Service>) {
        let id = service.id().to_string();
        self.inner.write().unwrap().insert(id, service);
    }

    /// `true` si algún plugin activo publica ese servicio.
    pub fn has(&self, service_id: &str) -> bool {
        self.inner.read().unwrap().contains_key(service_id)
    }

    /// Invoca una acción sobre un servicio publicado por otro plugin.
    pub fn call(&self, service_id: &str, action: &str, args: &[Arg]) -> Result<Ret, ServiceError> {
        let service = {
            let map = self.inner.read().unwrap();
            match map.get(service_id) {
                Some(service) => Arc::clone(service),
                None => return Err(ServiceError::UnknownService(service_id.to_string())),
            }
        };

        service.invoke(action, args).unwrap_or_else(|| {
            Err(ServiceError::UnknownAction {
                service: service_id.to_string(),
                action: action.to_string(),
            })
        })
    }

    /// Identificadores de todos los servicios publicados.
    pub fn published(&self) -> Vec<String> {
        let map = self.inner.read().unwrap();
        let mut ids: Vec<String> = map.keys().cloned().collect();
        ids.sort();
        ids
    }
}
