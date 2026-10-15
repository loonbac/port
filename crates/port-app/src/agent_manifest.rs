//! Motor de detección de estado de agentes basado en manifiestos TOML embebidos.
//!
//! Este módulo porta el motor de reglas de herdr para agentes que no emiten
//! secuencias OSC 7501, permitiendo deducir su estado (`Unknown`, `Idle`,
//! `Working`, `Blocked`) a partir del contenido visible en pantalla y de las
//! cadenas OSC de título y progreso.
//!
//! # Regiones soportadas y omisiones
//!
//! Regiones implementadas:
//! - `whole_recent`: texto completo del tail reciente de pantalla.
//! - `bottom_lines(N)`: las últimas N líneas de la pantalla.
//! - `bottom_non_empty_lines(N)`: las últimas N líneas no vacías.
//! - `top_non_empty_lines(N)`: las primeras N líneas no vacías.
//! - `osc_title`: título OSC de la terminal.
//! - `osc_progress`: progreso OSC reportado.
//!
//! Regiones no implementadas en PORT (devuelven texto vacío `""` y por tanto
//! sus reglas no coinciden):
//! - `last_non_empty_above_prompt_box`: en `claude.toml` (regla `background_agents_working`).
//! - `after_last_horizontal_rule`: en `claude.toml` (reglas `live_blocked_form`, `generic_permission_prompt`).
//! - `prompt_box_body`: en `claude.toml` (regla `live_prompt_box`).
//! - `after_last_prompt_marker`: en `codex.toml` (reglas `transcript_viewer`, `live_strong_blocker`).
//! - `whole_recent_without_current_prompt_marker`: en `codex.toml` (regla `weak_blocker`).
//! - `before_current_prompt_marker`: en `codex.toml` (regla `screen_working_fallback`).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use port_plugin_api::AgentState;
use regex::Regex;
use serde::Deserialize;

/// Límite máximo de reglas por manifiesto para evitar abusos o desbordamientos.
const MAX_RULES_PER_MANIFEST: usize = 128;

/// Profundidad máxima de anidación de compuertas lógicas (`all`, `any`, `not`).
const MAX_GATE_DEPTH: usize = 8;

/// Región por defecto si una regla no especifica ninguna.
fn default_region() -> String {
    "whole_recent".to_string()
}

/// Contenidos TOML de manifiestos embebidos en el binario sin E/S de disco en caliente.
const BUNDLED_MANIFESTS: &[&str] = &[
    include_str!("../assets/agent-detection/pi.toml"),
    include_str!("../assets/agent-detection/claude.toml"),
    include_str!("../assets/agent-detection/codex.toml"),
    include_str!("../assets/agent-detection/gemini.toml"),
    include_str!("../assets/agent-detection/opencode.toml"),
    include_str!("../assets/agent-detection/cursor.toml"),
    include_str!("../assets/agent-detection/antigravity.toml"),
    include_str!("../assets/agent-detection/droid.toml"),
    include_str!("../assets/agent-detection/amp.toml"),
    include_str!("../assets/agent-detection/grok.toml"),
];

/// Caché estática global de manifiestos compilados, indexada por `id` del manifiesto.
static MANIFEST_REGISTRY: OnceLock<BTreeMap<String, CompiledManifest>> = OnceLock::new();

/// Resultado de la detección del motor de manifiestos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestDetection {
    /// Estado deducido por la regla ganadora, o `Unknown` si no hubo coincidencia.
    pub state: AgentState,
    /// `true` sólo si la regla ganadora declara `visible_blocker = true` y su estado es `Blocked`.
    pub visible_blocker: bool,
    /// Bandera `skip_state_update` de la regla ganadora.
    pub skip: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct AgentManifest {
    id: String,
    #[serde(default, rename = "version")]
    _version: Option<String>,
    #[serde(default, rename = "min_engine_version")]
    _min_engine_version: Option<u32>,
    #[serde(default, rename = "updated_at")]
    _updated_at: Option<String>,
    #[serde(default, rename = "aliases")]
    _aliases: Vec<String>,
    #[serde(default)]
    rules: Vec<ManifestRule>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct ManifestRule {
    id: String,
    #[serde(default)]
    state: Option<ManifestState>,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default, rename = "visible_idle")]
    _visible_idle: bool,
    #[serde(default)]
    visible_blocker: bool,
    #[serde(default, rename = "visible_working")]
    _visible_working: bool,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<ManifestGate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct ManifestGate {
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<ManifestGate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ManifestState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl From<ManifestState> for AgentState {
    fn from(value: ManifestState) -> Self {
        match value {
            ManifestState::Idle => AgentState::Idle,
            ManifestState::Working => AgentState::Working,
            ManifestState::Blocked => AgentState::Blocked,
            ManifestState::Unknown => AgentState::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct CompiledManifest {
    rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    #[allow(dead_code)]
    id: String,
    state: AgentState,
    priority: i32,
    region: String,
    visible_blocker: bool,
    skip_state_update: bool,
    gate: CompiledGate,
}

#[derive(Debug, Clone)]
struct CompiledGate {
    all: Vec<CompiledGate>,
    any: Vec<CompiledGate>,
    not_gate: Vec<CompiledGate>,
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

fn manifest_gate_from_rule(rule: &ManifestRule) -> ManifestGate {
    ManifestGate {
        all: rule.all.clone(),
        any: rule.any.clone(),
        not_gate: rule.not_gate.clone(),
        contains: rule.contains.clone(),
        regex: rule.regex.clone(),
        line_regex: rule.line_regex.clone(),
    }
}

fn validate_manifest(manifest: &AgentManifest) -> Result<(), String> {
    if manifest.rules.is_empty() {
        return Err("el manifiesto debe contener al menos una regla".to_string());
    }
    if manifest.rules.len() > MAX_RULES_PER_MANIFEST {
        return Err(format!(
            "el manifiesto excede el límite de {MAX_RULES_PER_MANIFEST} reglas"
        ));
    }
    for rule in &manifest.rules {
        if rule.id.trim().is_empty() {
            return Err("el id de regla no puede estar vacío".to_string());
        }
        let gate = manifest_gate_from_rule(rule);
        validate_gate(&gate, 0)?;
    }
    Ok(())
}

fn validate_gate(gate: &ManifestGate, depth: usize) -> Result<(), String> {
    if depth > MAX_GATE_DEPTH {
        return Err(format!("profundidad de compuerta excede {MAX_GATE_DEPTH}"));
    }
    for nested in &gate.all {
        validate_gate(nested, depth + 1)?;
    }
    for nested in &gate.any {
        validate_gate(nested, depth + 1)?;
    }
    for nested in &gate.not_gate {
        validate_gate(nested, depth + 1)?;
    }
    Ok(())
}

fn compile_gate(gate: &ManifestGate) -> Result<CompiledGate, String> {
    Ok(CompiledGate {
        all: gate
            .all
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        any: gate
            .any
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        not_gate: gate
            .not_gate
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        contains: gate
            .contains
            .iter()
            .map(|needle| needle.to_lowercase())
            .collect(),
        regex: gate
            .regex
            .iter()
            .map(|pat| Regex::new(pat).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?,
        line_regex: gate
            .line_regex
            .iter()
            .map(|pat| Regex::new(pat).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?,
    })
}

fn compile_manifest(manifest: &AgentManifest) -> Result<CompiledManifest, String> {
    validate_manifest(manifest)?;
    let mut compiled_rules = Vec::with_capacity(manifest.rules.len());
    for rule in &manifest.rules {
        let gate = compile_gate(&manifest_gate_from_rule(rule))?;
        let state = rule
            .state
            .map(AgentState::from)
            .unwrap_or(AgentState::Unknown);
        compiled_rules.push(CompiledRule {
            id: rule.id.clone(),
            state,
            priority: rule.priority,
            region: rule.region.clone(),
            visible_blocker: rule.visible_blocker,
            skip_state_update: rule.skip_state_update,
            gate,
        });
    }
    Ok(CompiledManifest {
        rules: compiled_rules,
    })
}

fn parse_manifest(raw: &str) -> Result<AgentManifest, String> {
    toml::from_str::<AgentManifest>(raw).map_err(|err| err.to_string())
}

fn get_manifest_registry() -> &'static BTreeMap<String, CompiledManifest> {
    MANIFEST_REGISTRY.get_or_init(|| {
        let mut map = BTreeMap::new();
        for raw in BUNDLED_MANIFESTS {
            if let Ok(manifest) = parse_manifest(raw) {
                let id = manifest.id.clone();
                if let Ok(compiled) = compile_manifest(&manifest) {
                    map.insert(id, compiled);
                }
            }
        }
        map
    })
}

fn compiled_gate_matches(gate: &CompiledGate, text: &str, lower_text: &str) -> bool {
    if !gate
        .contains
        .iter()
        .all(|needle| lower_text.contains(needle))
    {
        return false;
    }

    if !gate.regex.iter().all(|regex| regex.is_match(text)) {
        return false;
    }

    if !gate
        .line_regex
        .iter()
        .all(|regex| text.lines().any(|line| regex.is_match(line)))
    {
        return false;
    }

    if !gate
        .all
        .iter()
        .all(|nested| compiled_gate_matches(nested, text, lower_text))
    {
        return false;
    }

    if !gate.any.is_empty()
        && !gate
            .any
            .iter()
            .any(|nested| compiled_gate_matches(nested, text, lower_text))
    {
        return false;
    }

    if gate
        .not_gate
        .iter()
        .any(|nested| compiled_gate_matches(nested, text, lower_text))
    {
        return false;
    }

    true
}

fn compiled_rule_matches(rule: &CompiledRule, text: &str) -> bool {
    let lower_text = text.to_lowercase();
    compiled_gate_matches(&rule.gate, text, &lower_text)
}

fn line_start_offset(content: &str, lines: &[&str], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(content.len())
}

fn safe_slice_from(content: &str, byte_offset: usize) -> &str {
    let mut offset = byte_offset.min(content.len());
    while offset < content.len() && !content.is_char_boundary(offset) {
        offset += 1;
    }
    &content[offset..]
}

fn safe_slice_to(content: &str, byte_offset: usize) -> &str {
    let mut offset = byte_offset.min(content.len());
    while offset > 0 && !content.is_char_boundary(offset) {
        offset -= 1;
    }
    &content[..offset]
}

fn slice_from_line_index<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    let byte_offset = line_start_offset(content, lines, index);
    safe_slice_from(content, byte_offset)
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(count);
    slice_from_line_index(content, &lines, start)
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(start_index) = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    slice_from_line_index(content, &lines, start_index)
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(end_index) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    let byte_offset = line_start_offset(content, &lines, end_index + 1);
    safe_slice_to(content, byte_offset)
}

fn parse_region_count(spec: &str, name: &str) -> Option<usize> {
    spec.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|count| count.trim().parse::<usize>().ok())
}

fn parse_top_non_empty_lines(spec: &str) -> Option<usize> {
    let count = spec
        .strip_prefix("top_non_empty_lines")?
        .strip_prefix('(')?
        .strip_suffix(')')?
        .trim();
    if count.starts_with('0') || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    count.parse::<usize>().ok()
}

fn region<'a>(spec: &str, screen: &'a str, osc_title: &'a str, osc_progress: &'a str) -> &'a str {
    let trimmed = spec.trim();
    match trimmed {
        "osc_title" => osc_title,
        "osc_progress" => osc_progress,
        "whole_recent" => screen,
        _ => {
            if let Some(count) = parse_region_count(trimmed, "bottom_lines") {
                bottom_lines(screen, count)
            } else if let Some(count) = parse_region_count(trimmed, "bottom_non_empty_lines") {
                bottom_non_empty_lines(screen, count)
            } else if let Some(count) = parse_top_non_empty_lines(trimmed) {
                top_non_empty_lines(screen, count)
            } else {
                ""
            }
        }
    }
}

/// Evalúa el manifiesto del agente sobre la pantalla y las secuencias OSC.
///
/// Si no existe manifiesto para `agent_id` o ninguna regla coincide, devuelve
/// `ManifestDetection` con estado `Unknown`, `visible_blocker: false` y `skip: false`.
pub fn detect(
    agent_id: &str,
    screen: &str,
    osc_title: &str,
    osc_progress: &str,
) -> ManifestDetection {
    let registry = get_manifest_registry();
    let Some(manifest) = registry.get(agent_id) else {
        return ManifestDetection {
            state: AgentState::Unknown,
            visible_blocker: false,
            skip: false,
        };
    };

    let mut winning_rule: Option<&CompiledRule> = None;

    for rule in &manifest.rules {
        let region_text = region(&rule.region, screen, osc_title, osc_progress);
        if compiled_rule_matches(rule, region_text) {
            match winning_rule {
                Some(prev) if prev.priority >= rule.priority => {}
                _ => winning_rule = Some(rule),
            }
        }
    }

    let Some(rule) = winning_rule else {
        return ManifestDetection {
            state: AgentState::Unknown,
            visible_blocker: false,
            skip: false,
        };
    };

    let state = rule.state;
    let visible_blocker = rule.visible_blocker && state == AgentState::Blocked;
    let skip = rule.skip_state_update;

    ManifestDetection {
        state,
        visible_blocker,
        skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pi_working_literal_y_working_border() {
        // Regla working_literal (contains "Working...")
        let screen_literal = "Iniciando tarea...\nWorking...\n";
        let det = detect("pi", screen_literal, "", "");
        assert_eq!(det.state, AgentState::Working);
        assert!(!det.visible_blocker);
        assert!(!det.skip);

        // Regla working_literal (line_regex con spinner)
        let screen_spinner = "Prompt anterior\n⠋ Working\n";
        let det_spinner = detect("pi", screen_spinner, "", "");
        assert_eq!(det_spinner.state, AgentState::Working);

        // Regla working_border en bottom_non_empty_lines(12)
        let screen_border = "Línea 1\nLínea 2\n── ⠹ Working ────────\n";
        let det_border = detect("pi", screen_border, "", "");
        assert_eq!(det_border.state, AgentState::Working);

        // Pantalla sin actividad
        let screen_idle = "user@box:~$ ls -la\ntotal 0\n";
        let det_idle = detect("pi", screen_idle, "", "");
        assert_eq!(det_idle.state, AgentState::Unknown);
    }

    #[test]
    fn test_agente_sin_manifiesto_devuelve_unknown() {
        let det = detect("desconocido_xyz", "Working...", "", "");
        assert_eq!(
            det,
            ManifestDetection {
                state: AgentState::Unknown,
                visible_blocker: false,
                skip: false,
            }
        );
    }

    #[test]
    fn test_prioridad_y_desempate() {
        // Probamos con un manifiesto sintético
        let toml_content = r#"
id = "test_prio"

[[rules]]
id = "rule_low"
state = "idle"
priority = 10
contains = ["marker"]

[[rules]]
id = "rule_high"
state = "working"
priority = 20
contains = ["marker"]

[[rules]]
id = "rule_tie_first"
state = "blocked"
priority = 30
visible_blocker = true
contains = ["tie"]

[[rules]]
id = "rule_tie_second"
state = "idle"
priority = 30
contains = ["tie"]
"#;

        let parsed = parse_manifest(toml_content).expect("debe parsear");
        let compiled = compile_manifest(&parsed).expect("debe compilar");

        let mut reg = BTreeMap::new();
        reg.insert("test_prio".to_string(), compiled);

        // 1. Prioridad mayor (working vs idle)
        let mut winning: Option<&CompiledRule> = None;
        for rule in &reg.get("test_prio").unwrap().rules {
            let r_text = region(&rule.region, "marker here", "", "");
            if compiled_rule_matches(rule, r_text) {
                match winning {
                    Some(prev) if prev.priority >= rule.priority => {}
                    _ => winning = Some(rule),
                }
            }
        }
        assert_eq!(winning.unwrap().id, "rule_high");
        assert_eq!(winning.unwrap().state, AgentState::Working);

        // 2. Desempate: gana la primera del archivo (rule_tie_first sobre rule_tie_second)
        let mut winning_tie: Option<&CompiledRule> = None;
        for rule in &reg.get("test_prio").unwrap().rules {
            let r_text = region(&rule.region, "tie here", "", "");
            if compiled_rule_matches(rule, r_text) {
                match winning_tie {
                    Some(prev) if prev.priority >= rule.priority => {}
                    _ => winning_tie = Some(rule),
                }
            }
        }
        assert_eq!(winning_tie.unwrap().id, "rule_tie_first");
        assert_eq!(winning_tie.unwrap().state, AgentState::Blocked);
    }

    #[test]
    fn test_compuertas_any_all_not() {
        let toml_content = r#"
id = "test_gates"

[[rules]]
id = "rule_complex"
state = "blocked"
priority = 10
visible_blocker = true
contains = ["base"]
all = [
    { any = [{ contains = ["apple"] }, { contains = ["banana"] }] }
]
not = [
    { contains = ["forbidden"] }
]
"#;
        let parsed = parse_manifest(toml_content).expect("debe parsear");
        let compiled = compile_manifest(&parsed).expect("debe compilar");
        let rule = &compiled.rules[0];

        // Cumple base + any (apple) + sin not -> Coincide
        assert!(compiled_rule_matches(rule, "base with apple"));
        // Cumple base + any (banana) + sin not -> Coincide
        assert!(compiled_rule_matches(rule, "base with banana"));
        // No cumple any -> Falla
        assert!(!compiled_rule_matches(rule, "base with orange"));
        // No cumple base -> Falla
        assert!(!compiled_rule_matches(rule, "apple and banana"));
        // Cumple base + any pero contiene forbidden -> Falla por not
        assert!(!compiled_rule_matches(rule, "base with apple forbidden"));
    }

    #[test]
    fn test_contains_insensible_a_mayusculas() {
        let toml_content = r#"
id = "test_case"

[[rules]]
id = "rule_case"
state = "working"
contains = ["wOrKiNg..."]
"#;
        let parsed = parse_manifest(toml_content).expect("debe parsear");
        let compiled = compile_manifest(&parsed).expect("debe compilar");
        let rule = &compiled.rules[0];

        assert!(compiled_rule_matches(rule, "WORKING..."));
        assert!(compiled_rule_matches(rule, "working..."));
        assert!(compiled_rule_matches(rule, "wOrKiNg..."));
        assert!(!compiled_rule_matches(rule, "idle..."));
    }

    #[test]
    fn test_regiones_bottom_non_empty_lines_y_osc_title() {
        // osc_title
        let reg_title = region("osc_title", "pantalla", "título activo", "50%");
        assert_eq!(reg_title, "título activo");

        // osc_progress
        let reg_prog = region("osc_progress", "pantalla", "título", "75%");
        assert_eq!(reg_prog, "75%");

        // bottom_non_empty_lines(2)
        let screen = "l1\n\nl2\n\nl3\n\n";
        let reg_bottom = region("bottom_non_empty_lines(2)", screen, "", "");
        assert_eq!(reg_bottom, "l2\n\nl3\n\n");

        // top_non_empty_lines(2)
        let reg_top = region("top_non_empty_lines(2)", screen, "", "");
        assert_eq!(reg_top, "l1\n\nl2\n");
    }

    #[test]
    fn test_region_desconocida_devuelve_vacio() {
        let reg_desconocida = region("region_inexistente_123", "texto en pantalla", "t", "p");
        assert_eq!(reg_desconocida, "");

        let reg_claude_omitted =
            region("after_last_horizontal_rule", "texto en pantalla", "t", "p");
        assert_eq!(reg_claude_omitted, "");
    }

    #[test]
    fn test_carga_todos_los_manifiestos_embebidos_parsean_y_compilan() {
        let reg = get_manifest_registry();
        let expected_ids = [
            "pi", "claude", "codex", "gemini", "opencode", "cursor", "agy", "droid", "amp", "grok",
        ];
        for id in expected_ids {
            let manifest = reg.get(id);
            assert!(
                manifest.is_some(),
                "el manifiesto para '{id}' debe existir en el registro compilado"
            );
            assert!(
                !manifest.unwrap().rules.is_empty(),
                "el manifiesto para '{id}' debe tener reglas compiladas"
            );
        }
    }
}
