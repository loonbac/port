//! Anfitrión de plugins externos: procesos que hablan el protocolo por stdio.
//!
//! # El requisito que manda sobre el diseño
//!
//! El terminal no puede bloquearse nunca por un plugin. Un plugin colgado, lento
//! o simplemente roto tiene que degradar a "no aporta nada", nunca a "la
//! terminal se congela". Por eso aquí no hay esperas sin plazo: cada petición
//! lleva [`DEFAULT_TIMEOUT_MS`] y, si no llega la respuesta, se continúa como si
//! el plugin no estuviera.
//!
//! # Por qué no se pregunta en cada tecla
//!
//! El plugin declara en el saludo qué combinaciones quiere. El núcleo las
//! guarda en [`ExternalPlugin::bindings`] y las resuelve en local, así que la
//! ruta habitual de teclear no toca ninguna tubería. Solo cuando una
//! combinación coincide se consulta al plugin dueño, y con plazo.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::PluginConfig;
use crate::protocol::{
    Appearance, Binding, Capability, HostRequest, PluginManifest, PluginReply, DEFAULT_TIMEOUT_MS,
    PROTOCOL_VERSION,
};

use super::PluginError;

/// Un plugin en ejecución.
///
/// Se clona con `Arc` para poder consultarlo desde el hilo de entrada sin
/// guardar bloqueos de por medio.
pub struct ExternalPlugin {
    manifest: PluginManifest,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// Respuestas del hilo lector, emparejadas por número de secuencia.
    inbox: Mutex<Receiver<PluginReply>>,
    /// Respuestas que llegaron sin que nadie las pidiera todavia.
    ///
    /// Un plugin puede contestar de mas (un acuse, un evento) y ese mensaje
    /// no puede descartarse: si se tirara, la respuesta de la peticion
    /// siguiente seria leida en su lugar. Se guardan aqui y `recv` los
    /// consulta antes que la tuberia.
    pending: Mutex<VecDeque<PluginReply>>,
    seq: AtomicU64,
    // Llega tras arrancar, asi que van bajo candado: el saludo las rellena.
    capabilities: Mutex<Vec<Capability>>,
    bindings: Mutex<Vec<Binding>>,
    appearance: Mutex<Appearance>,
    timeout: Duration,
}

impl ExternalPlugin {
    /// Arranca el plugin y lee su saludo.
    ///
    /// Si no arranca, no habla el protocolo o se pasa del plazo, devuelve el
    /// error en lugar de propagar un pánico: un plugin roto no puede tumbar la
    /// terminal al arrancar.
    pub fn start(manifest: PluginManifest) -> Result<Arc<Self>, PluginError> {
        let mut child = spawn(&manifest.executable)?;

        let stdin = child
            .stdin
            .take()
            .ok_or(PluginError::NoStdin(manifest.id.clone()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(PluginError::NoStdout(manifest.id.clone()))?;

        let rx = read_loop(stdout, &manifest.id)?;

        let timeout = Duration::from_millis(DEFAULT_TIMEOUT_MS);
        let plugin = Arc::new(Self {
            manifest,
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            inbox: Mutex::new(rx),
            pending: Mutex::new(VecDeque::new()),
            seq: AtomicU64::new(1),
            capabilities: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            appearance: Mutex::new(Appearance::default()),
            timeout,
        });

        plugin.handshake()?;
        Ok(plugin)
    }

    /// Envía el saludo y espera a que el plugin se identifique.
    fn handshake(&self) -> Result<(), PluginError> {
        self.send(&HostRequest::Hello {
            api: PROTOCOL_VERSION,
        })?;

        let reply = self.recv()?;
        match reply {
            PluginReply::Ready {
                name,
                capabilities,
                bindings,
                appearance,
                ..
            } => {
                // El saludo es la unica fuente de identidad y de lo que el
                // plugin puede hacer: se guarda aqui para no volver a
                // preguntarselo en cada pulsacion.
                *self.capabilities.lock().unwrap() = capabilities;
                *self.bindings.lock().unwrap() = bindings;
                *self.appearance.lock().unwrap() = appearance;
                let _ = name;
                Ok(())
            }
            other => Err(PluginError::Protocol(format!(
                "se esperaba Ready, llegó {other:?}"
            ))),
        }
    }

    fn send(&self, request: &HostRequest) -> Result<(), PluginError> {
        let mut guard = self
            .stdin
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))?;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| PluginError::Closed(self.manifest.id.clone()))?;

        let line =
            serde_json::to_string(request).map_err(|e| PluginError::Protocol(e.to_string()))?;
        writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .map_err(|_| PluginError::Closed(self.manifest.id.clone()))
    }

    fn recv(&self) -> Result<PluginReply, PluginError> {
        if let Some(ready) = self
            .pending
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))?
            .pop_front()
        {
            return Ok(ready);
        }

        let inbox = self
            .inbox
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))?;
        inbox.recv_timeout(self.timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => PluginError::Timeout(self.manifest.id.clone()),
            RecvTimeoutError::Disconnected => PluginError::Closed(self.manifest.id.clone()),
        })
    }

    /// Aparta una respuesta que no corresponde a lo que se pedia.
    fn stash(&self, reply: PluginReply) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.push_back(reply);
        }
    }

    /// Ejecuta una acción y devuelve su valor si el plugin contesta a tiempo.
    ///
    /// Devolver `None` es un resultado legitimo: significa "este plugin no
    /// aporta nada aquí", no "algo fue mal".
    pub fn invoke(&self, action: &str) -> Option<serde_json::Value> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        self.send(&HostRequest::Invoke {
            seq,
            action: action.to_string(),
        })
        .ok()?;

        match self.recv() {
            Ok(PluginReply::Result {
                seq: got,
                ok: true,
                value,
            }) if got == seq => Some(value),
            Ok(PluginReply::Result {
                seq: got,
                ok: false,
                ..
            }) if got == seq => None,
            // Una respuesta de otra peticion, o un evento: se aparta para no
            // perderla y no se puede dar por respuesta de esta.
            Ok(other) => {
                self.stash(other);
                None
            }
            // Un timeout aqui significa que el plugin no contesta: se
            // abandona la accion y la terminal sigue.
            Err(_) => None,
        }
    }

    /// Aplica el bloque de configuración del plugin.
    ///
    /// Consume el acuse: dejarlo en la tuberia lo haria pasar por respuesta a
    /// la accion siguiente.
    pub fn configure(&self, values: BTreeMap<String, String>) -> bool {
        if self.send(&HostRequest::Configure { values }).is_err() {
            return false;
        }
        match self.recv() {
            Ok(PluginReply::Ack) => true,
            Ok(other) => {
                self.stash(other);
                false
            }
            Err(_) => false,
        }
    }

    /// Vuelve a leer lo que el plugin declara, sin reiniciar su proceso.
    ///
    /// Aplicar la configuración puede cambiar la apariencia, los atajos y las
    /// capacidades que un plugin anuncia, y la copia cacheada es la del saludo
    /// inicial: sin esta relectura, `opacity` o `family` escritos en
    /// `config.md` no llegarían nunca a `effective_opacity` ni a
    /// `effective_font_family`.
    ///
    /// No se reutiliza [`ExternalPlugin::reload`] a propósito: `reload` mata el
    /// proceso y arranca otro, con lo que se perdería la configuración recién
    /// aplicada, y además vacía las respuestas en cola. Rehacer el saludo solo
    /// vuelve a preguntar, que es lo que hace falta.
    ///
    /// Orden de arranque: `start()` y después `configure_externals()`, así que
    /// tras esta relectura la copia cacheada termina coincidiendo con
    /// `config.md`.
    fn refresh_declaration(&self) -> Result<(), PluginError> {
        self.handshake()
    }

    /// Apariencia que el plugin declaró al arrancar.
    pub fn appearance(&self) -> Appearance {
        self.appearance
            .lock()
            .map(|a| a.clone())
            .unwrap_or_default()
    }

    /// Combinaciones declaradas al arrancar, resueltas en local por el núcleo.
    pub fn bindings(&self) -> Vec<Binding> {
        self.bindings.lock().map(|b| b.clone()).unwrap_or_default()
    }

    /// Capabilities declaradas al arrancar.
    pub fn capabilities(&self) -> Vec<Capability> {
        self.capabilities
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// `true` si el plugin anunció esta capacidad.
    pub fn has(&self, capability: Capability) -> bool {
        self.capabilities
            .lock()
            .map(|c| c.contains(&capability))
            .unwrap_or(false)
    }

    /// Devuelve la acción que corresponde a una tecla, si algún plugin la reclama.
    ///
    /// La resolución es local: no se toca ninguna tubería salvo que haya
    /// coincidencia, y entonces solo se pregunta al plugin dueño.
    pub fn resolve_action(
        plugins: &[Arc<Self>],
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Option<(Arc<Self>, String)> {
        for plugin in plugins {
            for binding in plugin.bindings() {
                if binding.matches(key, ctrl, alt, shift) {
                    return Some((Arc::clone(plugin), binding.action.clone()));
                }
            }
        }
        None
    }

    /// Datos de registro del plugin.
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    /// Reinicia el proceso del plugin.
    ///
    /// Es lo que se usa tras recompilar en desarrollo: matar y volver a lanzar
    /// tarda unos milisegundos, frente a los segundos de recompilar, y mientras
    /// tanto el terminal sigue respondiendo con lo que ya tenia en cache.
    ///
    /// No se reutiliza el hilo de lectura anterior: estaba atado al proceso
    /// viejo y su canal quedaria cerrado para siempre.
    pub fn reload(&self) -> Result<(), PluginError> {
        {
            let mut guard = self
                .child
                .lock()
                .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))?;
            if let Some(mut old) = guard.take() {
                let _ = old.kill();
                let _ = old.wait();
            }
        }
        self.pending.lock().unwrap().clear();

        let mut fresh = spawn(&self.manifest.executable)?;
        let stdin = fresh
            .stdin
            .take()
            .ok_or_else(|| PluginError::NoStdin(self.manifest.id.clone()))?;
        let stdout = fresh
            .stdout
            .take()
            .ok_or_else(|| PluginError::NoStdout(self.manifest.id.clone()))?;

        let rx = read_loop(stdout, &self.manifest.id)?;
        *self
            .inbox
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))? = rx;
        *self
            .stdin
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))? = Some(stdin);
        *self
            .child
            .lock()
            .map_err(|_| PluginError::Poisoned(self.manifest.id.clone()))? = Some(fresh);

        self.handshake()
    }
}

impl Drop for ExternalPlugin {
    fn drop(&mut self) {
        let _ = self.send(&HostRequest::Shutdown);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Hilo que lee la salida del plugin y la entrega por el canal.
///
/// Descarta las lineas que no son mensajes: un plugin que escribe un aviso por
/// stdout no debe poder tumbar la sesion, pero si_grabar un log en el canal que
/// el host espera como respuesta.
fn read_loop(stdout: ChildStdout, id: &str) -> Result<Receiver<PluginReply>, PluginError> {
    let (tx, rx) = channel();
    thread::Builder::new()
        .name(format!("port-plugin-{id}"))
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(reply) = serde_json::from_str::<PluginReply>(&line) {
                    if tx.send(reply).is_err() {
                        break;
                    }
                }
            }
        })
        .map_err(|e| PluginError::Thread {
            id: id.to_string(),
            source: e,
        })?;
    Ok(rx)
}

/// Lanza un plugin reintentando si el binario esta siendo reescrito.
///
/// `ETXTBSY` aparece justo en el caso interesante: durante el desarrollo se
/// recompila el plugin y se recarga mientras el nucleo lo lanza. El error es
/// transitorio, asi que se reintenta un momento en vez de declararlo roto.
fn spawn(executable: &str) -> Result<Child, PluginError> {
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..5 {
        match Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                last = Some(e);
                thread::sleep(Duration::from_millis(20 * (attempt + 1)));
            }
            Err(e) => {
                return Err(PluginError::Spawn {
                    executable: executable.to_string(),
                    source: e,
                })
            }
        }
    }
    Err(PluginError::Spawn {
        executable: executable.to_string(),
        source: last.unwrap_or_else(|| std::io::Error::other("no se pudo lanzar")),
    })
}

/// Envía a cada plugin externo el bloque de configuración de su id.
///
/// Un fallo al configurar no apaga el plugin: se registra y se continúa, igual
/// que el resto de errores no críticos del anfitrión. La configuración es una
/// comodidad, no un requisito para que el plugin funcione.
pub fn configure_externals(
    plugins: &[Arc<ExternalPlugin>],
    configs: &BTreeMap<String, PluginConfig>,
) {
    for plugin in plugins {
        let id = plugin.manifest().id.clone();
        if let Some(block) = configs.get(&id) {
            if !plugin.configure(block.values.clone()) {
                eprintln!("no se pudo configurar el plugin externo {id}; se ignora");
                continue;
            }
            // El bloque puede haber cambiado lo que el plugin declara: hay que
            // releerlo para que la ventana use los valores configurados. Un
            // fallo al releer no apaga el plugin; se conserva lo que ya había.
            if let Err(e) = plugin.refresh_declaration() {
                eprintln!(
                    "no se pudo releer la declaración del plugin externo {id}: {e}; se ignora"
                );
            }
        }
    }
}

/// `true` si el proceso del plugin sigue vivo.
pub fn is_alive(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// Sender reutilizable para eventos espontáneos desde el hilo lector.
///
/// El hilo de lectura no debe poder escribir en stdout (competiría con las
/// respuestas), así que los eventos salen por un canal aparte.
pub type EventSender = Sender<(String, serde_json::Value)>;

/// Receptor de eventos espontáneos de todos los plugins.
pub type EventReceiver = Receiver<(String, serde_json::Value)>;

/// Canal para eventos espontáneos.
pub fn event_channel() -> (EventSender, EventReceiver) {
    channel()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn manifest(executable: &Path) -> PluginManifest {
        PluginManifest {
            id: "probe".into(),
            name: "Probe".into(),
            version: "0.1.0".into(),
            executable: executable.to_string_lossy().to_string(),
            source: "test".into(),
            capabilities: vec![Capability::Appearance],
        }
    }

    /// Escribe un plugin de prueba que habla el protocolo segun `mode`.
    ///
    /// El delimiteur `r##` deja el script legible: los JSON ya traen llaves y
    /// comillas, y escaparlos dentro del raw string lo hacia ilegible.
    const PROBE: &str = r##"#!/bin/sh
if [ "$MODE" = crash ]; then exit 3; fi
if [ "$MODE" = garbage ]; then
  printf 'esto no es json\n'
  while IFS= read -r line; do :; done
  exit 0
fi
# Solo "ok" se anuncia; "silent" arranca y se calla, que es justo el caso
# que el anfitrion tiene que cortar con el plazo.
if [ "$MODE" = ok ]; then
  printf '{"t":"ready","name":"probe","version":"0.1.0","capabilities":["appearance"],"bindings":[{"key":"t","ctrl":true,"shift":true,"action":"new_tab"}],"appearance":{"opacity":0.5}}\n'
fi
opacity=0
configured=0
while IFS= read -r line; do
  case "$line" in
    # El saludo de arranque ya se imprimió arriba. Tras aplicar la
    # configuración hay que volver a declarar la apariencia cuando el anfitrión
    # rehace el saludo.
    *hello*)
      if [ "$configured" = 1 ]; then
        printf '{"t":"ready","name":"probe","version":"0.1.0","capabilities":["appearance"],"bindings":[{"key":"t","ctrl":true,"shift":true,"action":"new_tab"}],"appearance":{"opacity":0.5}}\n'
      fi
      ;;
    *configure*)
      configured=1
      opacity=$(printf '%s' "$line" | sed -n 's/.*"opacity":"\([^"]*\)".*/\1/p')
      printf '{"t":"ack"}\n'
      ;;
    *invoke*)
      seq=$(printf '%s' "$line" | sed -n 's/.*"seq":\([0-9]*\).*/\1/p')
      printf '{"t":"result","seq":%s,"ok":true,"value":{"v":1,"opacity":"%s"}}\n' "$seq" "$opacity"
      ;;
  esac
done
"##;

    fn write_probe(mode: &str) -> PathBuf {
        // Un directorio por llamada: `cargo test` corre las pruebas en
        // paralelo, y dos pruebas con el mismo modo se pisarian el script
        // mientras la otra lo esta ejecutando.
        let dir = std::env::temp_dir().join(format!(
            "port-probe-{mode}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("probe");
        let tmp = dir.join("probe.tmp");
        std::fs::write(&tmp, PROBE.replace("$MODE", mode)).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        path
    }

    #[test]
    fn a_well_behaved_plugin_starts_and_answers() {
        let exe = write_probe("ok");
        let plugin = ExternalPlugin::start(manifest(&exe)).expect("el plugin deberia arrancar");
        assert!(plugin.has(Capability::Appearance));
    }

    #[test]
    fn invoking_a_running_plugin_returns_its_value() {
        let exe = write_probe("ok");
        let plugin = ExternalPlugin::start(manifest(&exe)).unwrap();
        let value = plugin.invoke("new_tab").expect("deberia devolver un valor");
        assert_eq!(value["v"], 1);
    }

    #[test]
    fn la_configuracion_llega_al_plugin_externo() {
        let exe = write_probe("ok");
        let plugin = ExternalPlugin::start(manifest(&exe)).unwrap();

        let mut block = PluginConfig::new();
        block.set("opacity", "0.5");
        let mut configs = BTreeMap::new();
        configs.insert("probe".to_string(), block);

        configure_externals(&[Arc::clone(&plugin)], &configs);

        let value = plugin.invoke("new_tab").expect("deberia responder");
        assert_eq!(value["opacity"], "0.5", "el bloque debe llegar al plugin");
    }

    #[test]
    fn a_silent_plugin_times_out_instead_of_hanging() {
        let exe = write_probe("silent");
        let started = Instant::now();
        let result = ExternalPlugin::start(manifest(&exe));
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "el timeout tiene que cortar, no esperar indefinidamente"
        );
    }

    #[test]
    fn a_plugin_that_crash_on_start_reports_an_error() {
        let exe = write_probe("crash");
        assert!(ExternalPlugin::start(manifest(&exe)).is_err());
    }

    #[test]
    fn a_plugin_that_speaks_nonsense_is_rejected() {
        let exe = write_probe("garbage");
        assert!(ExternalPlugin::start(manifest(&exe)).is_err());
    }

    #[test]
    fn a_missing_executable_is_an_error_not_a_panic() {
        let m = manifest(Path::new("/no/existe/port-plugin-fantasma"));
        assert!(ExternalPlugin::start(m).is_err());
    }
}
