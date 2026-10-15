//! Detección y arbitraje de estado de agentes por sesión.
//!
//! Este módulo implementa la lógica pura y sin E/S para calcular el estado
//! del agente (`Unknown`, `Idle`, `Working`, `Blocked`) y la bandera `seen`
//! para cada sesión activa de PORT, siguiendo el modelo de herdr.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use port_plugin_api::AgentState;

/// Ventana de tiempo tras la última salida del PTY durante la cual un agente
/// conocido sin OSC se considera en ejecución activa (`Working`).
pub const WORKING_WINDOW: Duration = Duration::from_millis(1000);

/// Intervalo mínimo entre evaluaciones de manifiesto cuando la secuencia de salida (`output_seq`) cambió.
pub const MANIFEST_MIN_INTERVAL: Duration = Duration::from_millis(200);

/// Intervalo máximo tras el cual debe forzarse una reevaluación del manifiesto aunque no haya salida nueva.
pub const MANIFEST_MAX_INTERVAL: Duration = Duration::from_millis(1000);

/// Número de confirmaciones consecutivas de reposo requeridas para pasar de
/// `Working` a `Idle` sin una señal explícita (`idle` o `clear`) del programa.
pub const IDLE_CONFIRMATIONS: u8 = 3;

/// Tiempo máximo que se puede retener el estado `Working` antes de aceptar
/// la transición a `Idle` por anti-parpadeo.
pub const IDLE_CAP: Duration = Duration::from_millis(700);

/// Tabla de binarios de agentes de IA conocidos por PORT.
const KNOWN_AGENTS: &[&str] = &[
    "pi",
    "claude",
    "codex",
    "gemini",
    "opencode",
    "cursor",
    "antigravity",
    "droid",
    "amp",
    "grok",
    "aider",
];

/// Comprueba si el binario del proceso en primer plano corresponde a un agente
/// de IA conocido por PORT.
///
/// Compara el nombre base en minúsculas contra la lista fija de agentes reconocidos.
pub fn is_known_agent(bin: &str) -> bool {
    let file_name = std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(bin);
    let base = file_name
        .strip_suffix(".exe")
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    KNOWN_AGENTS.contains(&base.as_str())
}

/// Mapea el binario del proceso en primer plano al identificador del manifiesto de detección correspondiente.
///
/// Devuelve `Some("pi")`, `Some("claude")`, `Some("codex")`, `Some("gemini")`, `Some("opencode")`,
/// `Some("cursor")`, `Some("agy")`, `Some("droid")`, `Some("amp")`, `Some("grok")` según el agente,
/// o `None` si no existe manifiesto asociado (por ejemplo `aider`).
pub fn manifest_id_for_bin(bin: &str) -> Option<&'static str> {
    let file_name = std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(bin);
    let base = file_name
        .strip_suffix(".exe")
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    match base.as_str() {
        "pi" => Some("pi"),
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "gemini" => Some("gemini"),
        "opencode" => Some("opencode"),
        "cursor" => Some("cursor"),
        "antigravity" => Some("agy"),
        "droid" => Some("droid"),
        "amp" => Some("amp"),
        "grok" => Some("grok"),
        _ => None,
    }
}

/// Señales derivadas del contenido en pantalla y secuencias OSC para evaluación de manifiestos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSignals {
    /// Fragmento reciente de texto visible en la pantalla de la sesión.
    pub screen: String,
    /// Título reportado por secuencias OSC.
    pub osc_title: String,
    /// Progreso reportado por secuencias OSC.
    pub osc_progress: String,
    /// Identificador del manifiesto aplicable al agente.
    pub agent_id: String,
}

/// Señales observadas en una sesión en un instante de evaluación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSignals {
    /// Estado anunciado por el programa vía OSC 7501 (`"working"`, `"blocked"`, `"idle"`, `"clear"`).
    pub program_state: Option<String>,
    /// Secuencia incremental de lecturas del PTY de esta sesión.
    pub output_seq: u64,
    /// Si el programa en primer plano es un agente reconocido.
    pub known_agent: bool,
    /// Señales de pantalla para evaluación de manifiestos (sólo presentes cuando corresponde evaluar).
    pub screen: Option<ScreenSignals>,
}

/// Resultado del arbitraje de señales antes de aplicar anti-parpadeo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Estado deducido a partir de las señales.
    pub state: AgentState,
    /// Indica si el estado `Idle` provino de una señal explícita del programa
    /// (`"idle"` o `"clear"`), lo que exime de anti-parpadeo.
    pub explicit_idle: bool,
}

#[allow(dead_code)]
/// Decide de forma pura el estado correspondiente a las señales dadas.
pub fn decide(
    signals: &SessionSignals,
    output_changed: bool,
    last_output_at: Option<Instant>,
    now: Instant,
) -> Decision {
    let manifest_detection = signals.screen.as_ref().map(|sc| {
        crate::agent_manifest::detect(&sc.agent_id, &sc.screen, &sc.osc_title, &sc.osc_progress)
    });
    decide_with_manifest_detection(
        signals,
        manifest_detection,
        output_changed,
        last_output_at,
        now,
    )
}

fn decide_with_manifest_detection(
    signals: &SessionSignals,
    manifest: Option<crate::agent_manifest::ManifestDetection>,
    output_changed: bool,
    last_output_at: Option<Instant>,
    now: Instant,
) -> Decision {
    // 1. Estado OSC anunciado por el programa (fuente autoritativa cuando está presente)
    if let Some(ref st) = signals.program_state {
        let st_norm = st.trim().to_ascii_lowercase();
        match st_norm.as_str() {
            "blocked" => {
                return Decision {
                    state: AgentState::Blocked,
                    explicit_idle: false,
                };
            }
            "working" => {
                return Decision {
                    state: AgentState::Working,
                    explicit_idle: false,
                };
            }
            "idle" | "clear" => {
                return Decision {
                    state: AgentState::Idle,
                    explicit_idle: true,
                };
            }
            _ => {}
        }
    }

    // 2 y 3. Arbitraje por manifiesto sobre la pantalla si está disponible
    if let Some(det) = manifest {
        // 2) si hay screen y visible_blocker -> Blocked
        if det.visible_blocker {
            return Decision {
                state: AgentState::Blocked,
                explicit_idle: false,
            };
        }
        // 3) si hay screen, sin skip y el estado del manifiesto no es Unknown -> ese estado
        if !det.skip && det.state != AgentState::Unknown {
            return Decision {
                state: det.state,
                explicit_idle: false,
            };
        }
    }

    // 4. Sin estado OSC ni manifiesto concluyente: actividad/reposo como hoy
    if !signals.known_agent {
        return Decision {
            state: AgentState::Unknown,
            explicit_idle: false,
        };
    }

    // Agente conocido: comprueba actividad reciente del PTY
    let recent_output = output_changed
        || last_output_at.is_some_and(|at| {
            now.checked_duration_since(at)
                .is_none_or(|d| d <= WORKING_WINDOW)
        });

    if recent_output {
        Decision {
            state: AgentState::Working,
            explicit_idle: false,
        }
    } else {
        Decision {
            state: AgentState::Idle,
            explicit_idle: false,
        }
    }
}

/// Registro interno de una sesión rastreada por el tracker.
#[derive(Debug, Clone)]
struct SessionEntry {
    last_output_seq: u64,
    last_output_at: Option<Instant>,
    last_manifest_at: Option<Instant>,
    last_manifest_output_seq: u64,
    last_manifest_detection: Option<crate::agent_manifest::ManifestDetection>,
    state: AgentState,
    seen: bool,
    pending_idle_started_at: Option<Instant>,
    pending_idle_confirmations: u8,
}

/// Rastreador con estado para arbitrar el estado y la visibilidad de agentes por sesión.
#[derive(Debug, Default)]
pub struct AgentTracker {
    sessions: BTreeMap<usize, SessionEntry>,
}

impl AgentTracker {
    /// Crea un nuevo rastreador vacío.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cantidad de sesiones activamente rastreadas.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Si no hay sesiones rastreadas.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Consulta el estado y la marca `seen` de una sesión, si existe.
    #[allow(dead_code)]
    pub fn get(&self, id: usize) -> Option<(AgentState, bool)> {
        self.sessions.get(&id).map(|s| (s.state, s.seen))
    }

    /// Determina si corresponde evaluar el motor de manifiestos para una sesión en el instante `now`.
    ///
    /// Devuelve `true` si la sesión nunca se ha evaluado, si la secuencia de salida cambió y han transcurrido
    /// al menos `MANIFEST_MIN_INTERVAL` (200 ms), o si han transcurrido `MANIFEST_MAX_INTERVAL` (1000 ms)
    /// desde la última evaluación.
    pub fn is_manifest_due(&self, session_id: usize, output_seq: u64, now: Instant) -> bool {
        let Some(rec) = self.sessions.get(&session_id) else {
            return true;
        };
        let Some(last_at) = rec.last_manifest_at else {
            return true;
        };
        let elapsed = now.checked_duration_since(last_at).unwrap_or_default();
        if elapsed >= MANIFEST_MAX_INTERVAL {
            return true;
        }
        if output_seq != rec.last_manifest_output_seq && elapsed >= MANIFEST_MIN_INTERVAL {
            return true;
        }
        false
    }

    /// Actualiza el estado de las sesiones a partir de sus señales en el instante `now`.
    ///
    /// Devuelve únicamente los cambios respecto a la evaluación anterior `(sesión, estado, seen)`.
    /// Las sesiones ausentes en `signals` se podan del rastreador.
    pub fn update(
        &mut self,
        signals: impl Iterator<Item = (usize, SessionSignals)>,
        active_id: usize,
        now: Instant,
    ) -> Vec<(usize, AgentState, bool)> {
        let mut changes = Vec::new();
        let mut present = BTreeSet::new();

        for (id, sig) in signals {
            present.insert(id);

            if let Some(rec) = self.sessions.get_mut(&id) {
                let output_changed = sig.output_seq != rec.last_output_seq;
                if output_changed {
                    rec.last_output_seq = sig.output_seq;
                    rec.last_output_at = Some(now);
                }

                let manifest_detection = if let Some(ref sc) = sig.screen {
                    let det = crate::agent_manifest::detect(
                        &sc.agent_id,
                        &sc.screen,
                        &sc.osc_title,
                        &sc.osc_progress,
                    );
                    rec.last_manifest_at = Some(now);
                    rec.last_manifest_output_seq = sig.output_seq;
                    rec.last_manifest_detection = Some(det);
                    Some(det)
                } else if sig.known_agent {
                    rec.last_manifest_detection
                } else {
                    rec.last_manifest_detection = None;
                    None
                };

                let decision = decide_with_manifest_detection(
                    &sig,
                    manifest_detection,
                    output_changed,
                    rec.last_output_at,
                    now,
                );

                let next_state =
                    if rec.state == AgentState::Working && decision.state == AgentState::Idle {
                        if decision.explicit_idle {
                            rec.pending_idle_started_at = None;
                            rec.pending_idle_confirmations = 0;
                            AgentState::Idle
                        } else {
                            let hold = match rec.pending_idle_started_at {
                                None => {
                                    rec.pending_idle_started_at = Some(now);
                                    rec.pending_idle_confirmations = 0;
                                    true
                                }
                                Some(started_at) => {
                                    let elapsed =
                                        now.checked_duration_since(started_at).unwrap_or_default();
                                    if elapsed >= IDLE_CAP {
                                        rec.pending_idle_started_at = None;
                                        rec.pending_idle_confirmations = 0;
                                        false
                                    } else {
                                        rec.pending_idle_confirmations =
                                            rec.pending_idle_confirmations.saturating_add(1);
                                        if rec.pending_idle_confirmations >= IDLE_CONFIRMATIONS {
                                            rec.pending_idle_started_at = None;
                                            rec.pending_idle_confirmations = 0;
                                            false
                                        } else {
                                            true
                                        }
                                    }
                                }
                            };
                            if hold {
                                AgentState::Working
                            } else {
                                AgentState::Idle
                            }
                        }
                    } else {
                        rec.pending_idle_started_at = None;
                        rec.pending_idle_confirmations = 0;
                        decision.state
                    };

                let next_seen = if next_state != AgentState::Idle || id == active_id {
                    true
                } else if (rec.state == AgentState::Working || rec.state == AgentState::Blocked)
                    && next_state == AgentState::Idle
                {
                    false
                } else {
                    rec.seen
                };

                if next_state != rec.state || next_seen != rec.seen {
                    rec.state = next_state;
                    rec.seen = next_seen;
                    changes.push((id, next_state, next_seen));
                }
            } else {
                let last_output_at = if sig.output_seq > 0 { Some(now) } else { None };
                let (manifest_detection, last_manifest_at, last_manifest_output_seq) =
                    if let Some(ref sc) = sig.screen {
                        let det = crate::agent_manifest::detect(
                            &sc.agent_id,
                            &sc.screen,
                            &sc.osc_title,
                            &sc.osc_progress,
                        );
                        (Some(det), Some(now), sig.output_seq)
                    } else {
                        (None, None, 0)
                    };
                let decision = decide_with_manifest_detection(
                    &sig,
                    manifest_detection,
                    false,
                    last_output_at,
                    now,
                );
                let state = decision.state;
                let seen = true;

                self.sessions.insert(
                    id,
                    SessionEntry {
                        last_output_seq: sig.output_seq,
                        last_output_at,
                        last_manifest_at,
                        last_manifest_output_seq,
                        last_manifest_detection: manifest_detection,
                        state,
                        seen,
                        pending_idle_started_at: None,
                        pending_idle_confirmations: 0,
                    },
                );
                changes.push((id, state, seen));
            }
        }

        self.sessions.retain(|id, _| present.contains(id));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_known_agent_identifica_agentes_y_descarta_utilidades() {
        assert!(is_known_agent("pi"));
        assert!(is_known_agent("PI"));
        assert!(is_known_agent("/usr/local/bin/claude"));
        assert!(is_known_agent("codex"));
        assert!(is_known_agent("gemini"));
        assert!(is_known_agent("opencode"));
        assert!(is_known_agent("cursor"));
        assert!(is_known_agent("antigravity"));
        assert!(is_known_agent("droid"));
        assert!(is_known_agent("amp"));
        assert!(is_known_agent("grok"));
        assert!(is_known_agent("aider"));

        assert!(!is_known_agent("bash"));
        assert!(!is_known_agent("zsh"));
        assert!(!is_known_agent("vim"));
        assert!(!is_known_agent("cargo"));
        assert!(!is_known_agent(""));
    }

    #[test]
    fn decide_pura_cubriendo_todos_los_casos() {
        let now = Instant::now();

        // 1. blocked vía OSC
        let sig_blocked = SessionSignals {
            program_state: Some("blocked".into()),
            output_seq: 10,
            known_agent: true,
            screen: None,
        };
        assert_eq!(
            decide(&sig_blocked, false, None, now),
            Decision {
                state: AgentState::Blocked,
                explicit_idle: false,
            }
        );

        // 2. working vía OSC
        let sig_working = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 0,
            known_agent: false,
            screen: None,
        };
        assert_eq!(
            decide(&sig_working, false, None, now),
            Decision {
                state: AgentState::Working,
                explicit_idle: false,
            }
        );

        // 3. clear/idle vía OSC (exención explícita de anti-parpadeo)
        let sig_clear = SessionSignals {
            program_state: Some("clear".into()),
            output_seq: 5,
            known_agent: true,
            screen: None,
        };
        assert_eq!(
            decide(&sig_clear, false, None, now),
            Decision {
                state: AgentState::Idle,
                explicit_idle: true,
            }
        );
        let sig_idle = SessionSignals {
            program_state: Some("idle".into()),
            output_seq: 5,
            known_agent: true,
            screen: None,
        };
        assert_eq!(
            decide(&sig_idle, false, None, now),
            Decision {
                state: AgentState::Idle,
                explicit_idle: true,
            }
        );

        // 4. agente desconocido sin OSC -> Unknown
        let sig_unknown = SessionSignals {
            program_state: None,
            output_seq: 100,
            known_agent: false,
            screen: None,
        };
        assert_eq!(
            decide(&sig_unknown, true, Some(now), now),
            Decision {
                state: AgentState::Unknown,
                explicit_idle: false,
            }
        );

        // 5. actividad reciente (secuencia cambió o dentro de WORKING_WINDOW) -> Working
        let sig_agent = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        assert_eq!(
            decide(&sig_agent, true, None, now),
            Decision {
                state: AgentState::Working,
                explicit_idle: false,
            }
        );
        let recent_time = now - Duration::from_millis(200);
        assert_eq!(
            decide(&sig_agent, false, Some(recent_time), now),
            Decision {
                state: AgentState::Working,
                explicit_idle: false,
            }
        );

        // 6. reposo (agente conocido sin salida reciente) -> Idle no explícito
        let old_time = now - Duration::from_millis(2000);
        assert_eq!(
            decide(&sig_agent, false, Some(old_time), now),
            Decision {
                state: AgentState::Idle,
                explicit_idle: false,
            }
        );
        assert_eq!(
            decide(&sig_agent, false, None, now),
            Decision {
                state: AgentState::Idle,
                explicit_idle: false,
            }
        );
    }

    #[test]
    fn tracker_working_a_idle_en_sesion_no_activa_deja_seen_false() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Tick 1: Sesión 1 trabajando en segundo plano (activa es 2)
        let sig_work = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_work)].into_iter(), 2, now);
        assert_eq!(changes, vec![(1, AgentState::Working, true)]);

        // Tick 2: Pasa a Idle explícito con clear en sesión no activa -> seen = false
        let sig_idle = SessionSignals {
            program_state: Some("clear".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_idle)].into_iter(), 2, now);
        assert_eq!(changes, vec![(1, AgentState::Idle, false)]);
    }

    #[test]
    fn tracker_working_a_idle_en_sesion_activa_deja_seen_true() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Tick 1: Sesión 1 trabajando en primer plano (activa es 1)
        let sig_work = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_work)].into_iter(), 1, now);
        assert_eq!(changes, vec![(1, AgentState::Working, true)]);

        // Tick 2: Pasa a Idle explícito en sesión activa -> seen = true
        let sig_idle = SessionSignals {
            program_state: Some("clear".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_idle)].into_iter(), 1, now);
        assert_eq!(changes, vec![(1, AgentState::Idle, true)]);
    }

    #[test]
    fn tracker_anti_parpadeo_mantiene_working_hasta_3_confirmaciones() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Tick 1: Sesión 1 con actividad PTY (working)
        let sig_work = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_work)].into_iter(), 1, now);
        assert_eq!(changes, vec![(1, AgentState::Working, true)]);

        // Cesa la actividad PTY. Tiempo avanza más allá de WORKING_WINDOW.
        let quiet_time = now + Duration::from_millis(1500);
        let sig_quiet = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: None,
        };

        // Confirmación 1: se retiene Working
        let changes = tracker.update([(1, sig_quiet.clone())].into_iter(), 1, quiet_time);
        assert!(changes.is_empty());
        assert_eq!(tracker.get(1), Some((AgentState::Working, true)));

        // Confirmación 2: se retiene Working
        let time_c2 = quiet_time + Duration::from_millis(50);
        let changes = tracker.update([(1, sig_quiet.clone())].into_iter(), 1, time_c2);
        assert!(changes.is_empty());
        assert_eq!(tracker.get(1), Some((AgentState::Working, true)));

        // Confirmación 3: se retiene Working
        let time_c3 = quiet_time + Duration::from_millis(100);
        let changes = tracker.update([(1, sig_quiet.clone())].into_iter(), 1, time_c3);
        assert!(changes.is_empty());
        assert_eq!(tracker.get(1), Some((AgentState::Working, true)));

        // Confirmación 4 (3 alcanzadas): pasa finalmente a Idle
        let time_c4 = quiet_time + Duration::from_millis(150);
        let changes = tracker.update([(1, sig_quiet)].into_iter(), 1, time_c4);
        assert_eq!(changes, vec![(1, AgentState::Idle, true)]);
    }

    #[test]
    fn tracker_anti_parpadeo_respeta_idle_cap() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Tick 1: Sesión 1 con actividad PTY (working)
        let sig_work = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        tracker.update([(1, sig_work)].into_iter(), 1, now);

        let quiet_time = now + Duration::from_millis(1500);
        let sig_quiet = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: None,
        };

        // Primera observación de reposo
        tracker.update([(1, sig_quiet.clone())].into_iter(), 1, quiet_time);

        // Siguiente observación tras IDLE_CAP (700 ms): debe aceptar Idle de inmediato
        let time_after_cap = quiet_time + IDLE_CAP + Duration::from_millis(10);
        let changes = tracker.update([(1, sig_quiet)].into_iter(), 1, time_after_cap);
        assert_eq!(changes, vec![(1, AgentState::Idle, true)]);
    }

    #[test]
    fn tracker_cambios_se_emiten_solo_cuando_cambian() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        let sig = SessionSignals {
            program_state: Some("blocked".into()),
            output_seq: 0,
            known_agent: true,
            screen: None,
        };

        // Primer tick: se emite el estado inicial
        let c1 = tracker.update([(1, sig.clone())].into_iter(), 1, now);
        assert_eq!(c1, vec![(1, AgentState::Blocked, true)]);

        // Ticks sucesivos sin cambio: no emiten nada
        let c2 = tracker.update(
            [(1, sig.clone())].into_iter(),
            1,
            now + Duration::from_millis(50),
        );
        assert!(c2.is_empty());
        let c3 = tracker.update([(1, sig)].into_iter(), 1, now + Duration::from_millis(100));
        assert!(c3.is_empty());
    }

    #[test]
    fn tracker_sesion_desaparecida_se_poda() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        let sig1 = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let sig2 = SessionSignals {
            program_state: Some("blocked".into()),
            output_seq: 2,
            known_agent: true,
            screen: None,
        };

        // Rastrea ambas sesiones
        tracker.update([(1, sig1), (2, sig2.clone())].into_iter(), 1, now);
        assert_eq!(tracker.len(), 2);
        assert!(tracker.get(1).is_some());
        assert!(tracker.get(2).is_some());

        // Sesión 1 desaparece
        tracker.update([(2, sig2)].into_iter(), 2, now + Duration::from_millis(50));
        assert_eq!(tracker.len(), 1);
        assert!(tracker.get(1).is_none());
        assert!(tracker.get(2).is_some());
    }

    #[test]
    fn tracker_enfocar_sesion_idle_no_vista_la_marca_como_vista() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Sesión 1 en working en segundo plano (activa = 2)
        let sig_work = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        tracker.update([(1, sig_work)].into_iter(), 2, now);

        // Pasa a idle en segundo plano -> seen = false
        let sig_idle = SessionSignals {
            program_state: Some("clear".into()),
            output_seq: 1,
            known_agent: true,
            screen: None,
        };
        let changes = tracker.update([(1, sig_idle.clone())].into_iter(), 2, now);
        assert_eq!(changes, vec![(1, AgentState::Idle, false)]);

        // El usuario enfoca la sesión 1 (active_id = 1) -> seen pasa a true
        let changes = tracker.update(
            [(1, sig_idle)].into_iter(),
            1,
            now + Duration::from_millis(50),
        );
        assert_eq!(changes, vec![(1, AgentState::Idle, true)]);
        assert_eq!(tracker.get(1), Some((AgentState::Idle, true)));
    }

    #[test]
    fn test_manifest_id_for_bin_mapea_correctamente() {
        assert_eq!(manifest_id_for_bin("pi"), Some("pi"));
        assert_eq!(manifest_id_for_bin("/usr/bin/claude"), Some("claude"));
        assert_eq!(manifest_id_for_bin("codex.exe"), Some("codex"));
        assert_eq!(manifest_id_for_bin("gemini"), Some("gemini"));
        assert_eq!(manifest_id_for_bin("opencode"), Some("opencode"));
        assert_eq!(manifest_id_for_bin("cursor"), Some("cursor"));
        assert_eq!(manifest_id_for_bin("antigravity"), Some("agy"));
        assert_eq!(manifest_id_for_bin("droid"), Some("droid"));
        assert_eq!(manifest_id_for_bin("amp"), Some("amp"));
        assert_eq!(manifest_id_for_bin("grok"), Some("grok"));
        assert_eq!(manifest_id_for_bin("aider"), None);
        assert_eq!(manifest_id_for_bin("bash"), None);
    }

    #[test]
    fn test_tracker_visible_blocker_del_manifiesto_vence_a_actividad() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // Pantalla con visible_blocker para droid (cumple execute_selection_blocker)
        let screen_blocker =
            "Ejecutar comando?\n> yes, allow\n↑↓ to navigate\nenter to select\nesc to cancel\n";
        let sig = SessionSignals {
            program_state: None,
            output_seq: 5, // Hay actividad reciente
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: screen_blocker.into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "droid".into(),
            }),
        };

        // Aunque hay actividad (output_seq: 5), visible_blocker del manifiesto gana -> Blocked
        let changes = tracker.update([(1, sig)].into_iter(), 1, now);
        assert_eq!(changes, vec![(1, AgentState::Blocked, true)]);
    }

    #[test]
    fn test_tracker_skip_state_update_deja_pasar_a_actividad_reposo() {
        let mut tracker = AgentTracker::new();
        let now = Instant::now();

        // claude: regla transcript_viewer en bottom_non_empty_lines(3) activa skip_state_update
        let screen_transcript = "output\nshowing detailed transcript\nctrl+o to toggle\n";
        let sig_with_activity = SessionSignals {
            program_state: None,
            output_seq: 1, // Actividad reciente
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: screen_transcript.into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "claude".into(),
            }),
        };

        // Con actividad y skip_state_update -> pasa a Working
        let changes = tracker.update([(1, sig_with_activity)].into_iter(), 1, now);
        assert_eq!(changes, vec![(1, AgentState::Working, true)]);

        // Sin actividad reciente (tras superar IDLE_CAP de anti-parpadeo) -> pasa a Idle
        let later = now + Duration::from_millis(1500);
        let sig_quiet = SessionSignals {
            program_state: None,
            output_seq: 1,
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: screen_transcript.into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "claude".into(),
            }),
        };
        tracker.update([(1, sig_quiet.clone())].into_iter(), 1, later);
        let changes = tracker.update(
            [(1, sig_quiet)].into_iter(),
            1,
            later + IDLE_CAP + Duration::from_millis(10),
        );
        assert_eq!(changes, vec![(1, AgentState::Idle, true)]);
    }

    #[test]
    fn test_tracker_is_manifest_due_respeta_intervalos() {
        let mut tracker = AgentTracker::new();
        let t0 = Instant::now();

        // 1. Nunca evaluado -> true
        assert!(tracker.is_manifest_due(1, 0, t0));

        // Registramos evaluación en t0
        let sig = SessionSignals {
            program_state: None,
            output_seq: 10,
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: "".into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "pi".into(),
            }),
        };
        tracker.update([(1, sig)].into_iter(), 1, t0);

        // 2. Mismo output_seq antes de MANIFEST_MAX_INTERVAL (1000 ms) -> false
        let t_50ms = t0 + Duration::from_millis(50);
        assert!(!tracker.is_manifest_due(1, 10, t_50ms));

        // 3. output_seq cambió pero antes de MANIFEST_MIN_INTERVAL (200 ms) -> false
        let t_100ms = t0 + Duration::from_millis(100);
        assert!(!tracker.is_manifest_due(1, 11, t_100ms));

        // 4. output_seq cambió y pasaron >= MANIFEST_MIN_INTERVAL (200 ms) -> true
        let t_250ms = t0 + Duration::from_millis(250);
        assert!(tracker.is_manifest_due(1, 11, t_250ms));

        // 5. Mismo output_seq pero pasaron >= MANIFEST_MAX_INTERVAL (1000 ms) -> true
        let t_1050ms = t0 + Duration::from_millis(1050);
        assert!(tracker.is_manifest_due(1, 10, t_1050ms));
    }

    #[test]
    fn test_decide_con_pantalla_manifiesto() {
        let now = Instant::now();

        let droid_blocker =
            "Ejecutar comando?\n> yes, allow\n↑↓ to navigate\nenter to select\nesc to cancel\n";

        // 1. OSC state vence a pantalla
        let sig_osc_and_screen = SessionSignals {
            program_state: Some("working".into()),
            output_seq: 10,
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: droid_blocker.into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "droid".into(),
            }),
        };
        assert_eq!(
            decide(&sig_osc_and_screen, false, None, now),
            Decision {
                state: AgentState::Working,
                explicit_idle: false,
            }
        );

        // 2. Manifiesto detecta visible_blocker sin OSC -> Blocked
        let sig_screen_blocker = SessionSignals {
            program_state: None,
            output_seq: 10,
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: droid_blocker.into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "droid".into(),
            }),
        };
        assert_eq!(
            decide(&sig_screen_blocker, true, None, now),
            Decision {
                state: AgentState::Blocked,
                explicit_idle: false,
            }
        );

        // 3. Manifiesto detecta working sin OSC -> Working
        let sig_screen_working = SessionSignals {
            program_state: None,
            output_seq: 0,
            known_agent: true,
            screen: Some(ScreenSignals {
                screen: "Working...\n".into(),
                osc_title: "".into(),
                osc_progress: "".into(),
                agent_id: "pi".into(),
            }),
        };
        assert_eq!(
            decide(&sig_screen_working, false, None, now),
            Decision {
                state: AgentState::Working,
                explicit_idle: false,
            }
        );
    }
}
