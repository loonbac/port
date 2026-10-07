//! Gestión de plugins por línea de comandos.
//!
//! Vive separado de la ventana a propósito: `port plugin add` no necesita GPUI
//! ni una sesión gráfica, así que no arranca la interfaz. Instalar un plugin
//! desde una terminal es precisamente el caso más común, y no debería exigir
//! abrir otra terminal.

use port_plugin_api::host;
use port_plugin_api::install;
use port_plugin_api::ConfigFile;

/// Qué pidió la persona en la línea de comandos.
#[derive(Debug)]
pub enum Command {
    /// No hay subcomando de plugin: hay que abrir la terminal.
    RunTerminal,
    Plugin(PluginCommand),
    Help,
    Version,
}

#[derive(Debug)]
pub enum PluginCommand {
    Add { url: String },
    Update { id: String },
    List,
    Remove { id: String },
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Formato estándar de presentación para `port --version`.
pub fn version() -> String {
    format!("port {VERSION}")
}

pub const HELP: &str = "\
PORT: terminal y plataforma de plugins

uso:
  port                        abre la terminal
  port plugin add <url>       instala un plugin desde un repositorio
  port plugin update <id>     actualiza un plugin desde su origen
  port plugin list            muestra los plugins instalados
  port plugin remove <id>     desinstala un plugin
  port --version              muestra la versión
  port --help                 esta ayuda
";

/// Interpreta los argumentos. Devolver `RunTerminal` cuando no hay subcomando
/// es lo que hace que `port` sin argumentos siga abriendo la ventana.
pub fn parse(args: impl Iterator<Item = String>) -> Command {
    let args: Vec<String> = args.skip(1).collect();

    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") | Some("help") => Command::Help,
        Some("--version") | Some("-V") | Some("version") => Command::Version,
        Some("plugin") => match args.get(1).map(String::as_str) {
            Some("add") => match args.get(2) {
                Some(url) => Command::Plugin(PluginCommand::Add { url: url.clone() }),
                None => Command::Help,
            },
            Some("list") => Command::Plugin(PluginCommand::List),
            Some("update") => match args.get(2) {
                Some(id) => Command::Plugin(PluginCommand::Update { id: id.clone() }),
                None => Command::Help,
            },
            Some("remove") | Some("rm") => match args.get(2) {
                Some(id) => Command::Plugin(PluginCommand::Remove { id: id.clone() }),
                None => Command::Help,
            },
            _ => Command::Help,
        },
        _ => Command::RunTerminal,
    }
}

/// Ejecuta un comando de plugin y devuelve el código de salida.
///
/// Los errores se imprimen y se traducen a código distinto de cero; nunca se
/// lanza un pánico por una entrada del usuario.
pub fn run(command: PluginCommand) -> i32 {
    match command {
        PluginCommand::Add { url } => match install::install(&url) {
            Ok(manifest) => {
                // Instalar no apaga: el plugin queda habilitado para el próximo
                // arranque, aunque antes se hubiera desactivado desde el menú.
                let _ = ConfigFile::set_enabled(&ConfigFile::default_path(), &manifest.id, true);
                println!("instalado {} {}", manifest.name, manifest.version);
                println!("  ejecutable: {}", manifest.executable);
                println!("  reinicia PORT para cargarlo");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        PluginCommand::Update { id } => {
            let Some(manifest) = host::installed().into_iter().find(|m| m.id == id) else {
                eprintln!("no está instalado: {id}");
                return 1;
            };
            if manifest.source.is_empty() {
                eprintln!(
                    "el plugin {id} no tiene origen registrado: reinstálalo con `port plugin add <fuente>`"
                );
                return 1;
            }
            match install::install(&manifest.source) {
                Ok(updated) => {
                    // Reemplaza el binario dejando el estado de habilitación a
                    // gusto del usuario: actualizar no es una forma de encender.
                    println!("actualizado {} {}", updated.name, updated.version);
                    println!("  reinicia PORT para cargarlo");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        PluginCommand::List => {
            let manifests = host::installed();
            if manifests.is_empty() {
                println!(
                    "no hay plugins instalados en {}",
                    host::plugins_dir().display()
                );
                return 0;
            }
            for manifest in manifests {
                let caps: Vec<String> = manifest
                    .capabilities
                    .iter()
                    .map(|c| format!("{c:?}").to_lowercase())
                    .collect();
                println!(
                    "{:<20} {:<10} {}",
                    manifest.id,
                    manifest.version,
                    caps.join(", ")
                );
            }
            0
        }
        PluginCommand::Remove { id } => match host::uninstall(&id) {
            Ok(true) => {
                println!("eliminado {id}");
                0
            }
            Ok(false) => {
                eprintln!("no está instalado: {id}");
                1
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("port".to_string())
            .chain(list.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn no_arguments_opens_the_terminal() {
        assert!(matches!(parse(args(&[]).into_iter()), Command::RunTerminal));
    }

    #[test]
    fn unknown_subcommand_opens_the_terminal_rather_than_failing() {
        // Un flag futuro que no-sea-comando no debe impedir abrir la terminal.
        assert!(matches!(
            parse(args(&["--new"]).into_iter()),
            Command::RunTerminal
        ));
    }

    #[test]
    fn help_is_recognised() {
        assert!(matches!(
            parse(args(&["--help"]).into_iter()),
            Command::Help
        ));
        assert!(matches!(parse(args(&["help"]).into_iter()), Command::Help));
    }

    #[test]
    fn plugin_add_takes_the_url() {
        match parse(args(&["plugin", "add", "https://github.com/u/p"]).into_iter()) {
            Command::Plugin(PluginCommand::Add { url }) => {
                assert_eq!(url, "https://github.com/u/p")
            }
            other => panic!("no se reconoce plugin add: {other:?}"),
        }
    }

    #[test]
    fn plugin_add_without_url_asks_for_help() {
        assert!(matches!(
            parse(args(&["plugin", "add"]).into_iter()),
            Command::Help
        ));
    }

    #[test]
    fn list_and_remove_are_recognised() {
        assert!(matches!(
            parse(args(&["plugin", "list"]).into_iter()),
            Command::Plugin(PluginCommand::List)
        ));
        match parse(args(&["plugin", "rm", "demo"]).into_iter()) {
            Command::Plugin(PluginCommand::Remove { id }) => assert_eq!(id, "demo"),
            other => panic!("no se reconoce plugin rm: {other:?}"),
        }
    }

    #[test]
    fn listing_with_no_plugins_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("port-cli-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("PORT_PLUGIN_DIR", &dir) };
        assert_eq!(run(PluginCommand::List), 0);
        unsafe { std::env::remove_var("PORT_PLUGIN_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugin_update_takes_the_id() {
        match parse(args(&["plugin", "update", "demo"]).into_iter()) {
            Command::Plugin(PluginCommand::Update { id }) => assert_eq!(id, "demo"),
            other => panic!("no se reconoce plugin update: {other:?}"),
        }
    }

    #[test]
    fn updating_an_unknown_plugin_fails_cleanly() {
        // Ningún plugin instalado puede llamarse así: el directorio real
        // basta y no hay que pisar `PORT_PLUGIN_DIR`, que es global.
        assert_eq!(
            run(PluginCommand::Update {
                id: "no-existe".into()
            }),
            1
        );
    }

    #[test]
    fn updating_a_legacy_plugin_asks_for_a_reinstall() {
        // Una instalación antigua no guardó el origen: no hay de dónde
        // actualizar y el mensaje debe mandar a reinstalar.
        let dir = std::env::temp_dir().join(format!("port-cli-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("legacy.json"),
            r#"{"id":"legacy","name":"Legacy","version":"0.1.0","executable":"/no/existe/legacy","capabilities":[]}"#,
        )
        .unwrap();
        unsafe { std::env::set_var("PORT_PLUGIN_DIR", &dir) };
        let code = run(PluginCommand::Update {
            id: "legacy".into(),
        });
        unsafe { std::env::remove_var("PORT_PLUGIN_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(code, 1, "sin origen no se puede actualizar");
    }

    #[test]
    fn removing_an_unknown_plugin_fails_cleanly() {
        assert_eq!(
            run(PluginCommand::Remove {
                id: "no-existe".into()
            }),
            1
        );
    }

    #[test]
    fn version_is_recognised() {
        assert!(matches!(
            parse(args(&["--version"]).into_iter()),
            Command::Version
        ));
        assert!(matches!(
            parse(args(&["-V"]).into_iter()),
            Command::Version
        ));
        assert!(matches!(
            parse(args(&["version"]).into_iter()),
            Command::Version
        ));
    }

    #[test]
    fn version_does_not_open_the_terminal() {
        assert!(!matches!(
            parse(args(&["--version"]).into_iter()),
            Command::RunTerminal
        ));
    }

    #[test]
    fn version_output_format_contains_crate_version() {
        let text = version();
        assert_eq!(text, format!("port {}", env!("CARGO_PKG_VERSION")));
        assert!(HELP.contains("--version"));
    }
}
