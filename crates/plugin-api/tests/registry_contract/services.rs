//! Directorio de servicios: llamadas entre plugins.

use port_plugin_api::{Plugin, PluginRegistry};

struct CountingService;

impl port_plugin_api::Service for CountingService {
    fn id(&self) -> &str {
        "counter"
    }
    fn name(&self) -> &str {
        "Counter"
    }
    fn actions(&self) -> Vec<&'static str> {
        vec!["add", "read"]
    }
    fn invoke(
        &self,
        action: &str,
        args: &[port_plugin_api::Arg],
    ) -> Option<std::result::Result<port_plugin_api::Ret, port_plugin_api::ServiceError>> {
        match action {
            "add" => {
                let amount = match args.first() {
                    Some(port_plugin_api::Arg::Num(n)) => *n,
                    _ => {
                        return Some(Err(port_plugin_api::ServiceError::Failed(
                            "add requiere un número".to_string(),
                        )))
                    }
                };
                Some(Ok(port_plugin_api::Ret::Num(100.0 + amount)))
            }
            "read" => Some(Ok(port_plugin_api::Ret::Num(100.0))),
            _ => None,
        }
    }
}

struct ServicePublishingPlugin;

impl Plugin for ServicePublishingPlugin {
    fn id(&self) -> &'static str {
        "counter"
    }
    fn name(&self) -> &'static str {
        "Counter Plugin"
    }
    fn services(&self) -> Vec<std::sync::Arc<dyn port_plugin_api::Service>> {
        vec![std::sync::Arc::new(CountingService)]
    }
}

#[test]
fn registering_a_plugin_publishes_its_services() {
    use port_plugin_api::{Arg, Ret, ServiceError};

    let mut registry = PluginRegistry::new();
    let services = registry.services();

    // Antes de registrar el plugin, el servicio no existe.
    assert!(!services.has("counter"));
    assert_eq!(
        services.call("counter", "read", &[]),
        Err(ServiceError::UnknownService("counter".to_string()))
    );

    registry.register(ServicePublishingPlugin);
    assert!(services.has("counter"));
    assert_eq!(services.published(), vec!["counter".to_string()]);

    // Invocación con argumentos y retorno de valor.
    assert_eq!(
        services.call("counter", "add", &[Arg::Num(5.0)]),
        Ok(Ret::Num(105.0))
    );

    // Acción inexistente: error explícito, no silencio.
    assert_eq!(
        services.call("counter", "nope", &[]),
        Err(ServiceError::UnknownAction {
            service: "counter".to_string(),
            action: "nope".to_string()
        })
    );
}

#[test]
fn a_service_action_can_fail_with_a_reason() {
    use port_plugin_api::{Arg, Ret, ServiceError};

    let mut registry = PluginRegistry::new();
    registry.register(ServicePublishingPlugin);
    let services = registry.services();

    // `add` sin argumento debe explicar por qué falló.
    assert_eq!(
        services.call("counter", "add", &[Arg::Unit]),
        Err(ServiceError::Failed("add requiere un número".to_string()))
    );
    // Y con el argumento correcto, funciona.
    assert_eq!(
        services.call("counter", "add", &[Arg::Num(1.0)]),
        Ok(Ret::Num(101.0))
    );
}
