//! Banco de la tubería completa: PTY → parseo → cuadro.
//!
//! Responde una sola pregunta: cuántos MiB por segundo puede tragar PORT, y en
//! qué etapa se va el tiempo. Se ejecuta sin ventana ni GPU, así que mide el
//! núcleo y no el servidor gráfico.
//!
//!   cargo run --release -p port-term-core --example pipeline_bench
//!   cargo run --release -p port-term-core --example pipeline_bench -- --mib 16 --only ascii,csi
//!
//! En perfil de desarrollo los números no significan nada: el programa avisa y
//! sigue, para que nadie compare un `cargo run` con un `--release`.
//!
//! Cada carga se mide en tres etapas:
//!
//! - `parse`: `Processor::advance` sobre el texto, en trozos del tamaño que usa un
//!   cuadro (`Session::PUMP_BUDGET_BYTES`).
//! - `frame`: construcción del cuadro (rejilla → filas → runs) tras cada trozo.
//! - `session`: la tubería real, con un PTY auténtico y `cat` escribiendo tan
//!   rápido como puede. Es la única que incluye el coste del sistema operativo.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use port_term_core::frame;
use port_term_core::pty::PtyConfig;
use port_term_core::session::{GridSize, Session};

/// Tamaño de trozo: el mismo que parsea un cuadro real.
const CHUNK_BYTES: usize = 1024 * 1024;

fn main() {
    let args = Args::parse();
    if args.help {
        print_help();
        return;
    }

    if cfg!(debug_assertions) {
        eprintln!(
            "aviso: perfil de desarrollo; las cifras solo comparan con otro run igual.\n\
             usa --release para números reales.\n"
        );
    }

    let mut report = Vec::new();
    report.push(format!(
        "PORT pipeline bench — {} MiB por carga, {} muestras, commit {}",
        args.mib,
        args.samples,
        git_revision()
    ));
    report.push(format!(
        "perfil: {}, plataforma: {} {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    report.push(String::new());
    report.push(format!(
        "{:<10} {:<8} {:>12} {:>12} {:>10}",
        "carga", "etapa", "p50", "p95", "MiB/s"
    ));

    for workload in Workload::all() {
        if let Some(only) = &args.only {
            if !only.iter().any(|name| name == workload.name()) {
                continue;
            }
        }

        let bytes = workload.generate(args.mib);
        let mib = bytes.len() as f64 / (1024.0 * 1024.0);

        let (parse_samples, frame_samples) = bench_in_memory(&bytes);
        report.push(row(workload.name(), "parse", &parse_samples, mib));
        report.push(row(
            workload.name(),
            "frame",
            &frame_samples,
            mib * parse_samples.len() as f64 / frame_samples.len() as f64,
        ));

        match bench_session(&bytes) {
            Ok(session) => report.push(row(
                workload.name(),
                "session",
                &session.samples,
                session.parsed_mib,
            )),
            Err(error) => report.push(format!(
                "{:<10} {:<8} {:>12} {:>12} {:>10}",
                workload.name(),
                "session",
                "-",
                "-",
                format!("fallo: {error}")
            )),
        }
    }

    let text = report.join("\n");
    println!("{text}");
    if let Err(error) = write_log(&text) {
        eprintln!("no se pudo escribir el registro: {error}");
    }
}

/// Etapas medidas en memoria: parseo y construcción de cuadro.
///
/// Una sola pasada para que ambas etapas vean exactamente el mismo texto y el
/// mismo estado de rejilla.
fn bench_in_memory(bytes: &[u8]) -> (Vec<Duration>, Vec<Duration>) {
    let size = GridSize::new(120, 40);
    let mut term = Term::new(
        Config {
            scrolling_history: 10_000,
            ..Default::default()
        },
        &TermSize(size),
        VoidListener,
    );
    let mut processor: Processor = Processor::new();

    let mut parse = Vec::new();
    let mut build = Vec::new();
    for chunk in bytes.chunks(CHUNK_BYTES) {
        let started = Instant::now();
        processor.advance(&mut term, chunk);
        parse.push(started.elapsed());

        let started = Instant::now();
        let frame = frame::build(&term);
        build.push(started.elapsed());
        std::hint::black_box(&frame);
    }

    (parse, build)
}

struct SessionBench {
    samples: Vec<Duration>,
    parsed_mib: f64,
}

/// La tubería real: un `cat` a toda velocidad contra un PTY de verdad.
fn bench_session(bytes: &[u8]) -> std::io::Result<SessionBench> {
    let path = temp_path("pipeline-bench");
    {
        let mut file = std::fs::File::create(&path)?;
        file.write_all(bytes)?;
    }

    let mut session = Session::spawn(
        PtyConfig {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), format!("cat {}", path.display())],
            cwd: None,
            env: Vec::new(),
        },
        GridSize::new(120, 40),
    )?;

    let started = Instant::now();
    let mut samples = Vec::new();
    loop {
        let began = Instant::now();
        let progressed = session.pump();
        if progressed {
            samples.push(began.elapsed());
        }
        if session.has_exited() && !progressed {
            break;
        }
        if started.elapsed() > Duration::from_secs(600) {
            eprintln!("aviso: la sesión no terminó en diez minutos, se corta la medición");
            break;
        }
        if !progressed {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    let dropped = session.dropped_output_bytes();
    if dropped > 0 {
        eprintln!(
            "aviso: se descartaron {dropped} bytes (actuó el tope de la cola); \
             la cifra es una cota inferior"
        );
    }

    let _ = std::fs::remove_file(&path);

    // La tubería aplica OPOST/ONLCR: cada \n llega como \r\n, y la terminal
    // parsea esos dos bytes. Se cuenta lo que llegó, no lo que se escribió.
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    let parsed = (bytes.len() as u64).saturating_sub(dropped) + newlines;

    Ok(SessionBench {
        samples,
        parsed_mib: parsed as f64 / (1024.0 * 1024.0),
    })
}

/// Cargas de trabajo: cada una castiga una parte distinta del parser.
enum Workload {
    Ascii,
    Sgr,
    Csi,
    Esc,
    Unicode,
}

impl Workload {
    fn all() -> [Workload; 5] {
        [
            Workload::Ascii,
            Workload::Sgr,
            Workload::Csi,
            Workload::Esc,
            Workload::Unicode,
        ]
    }

    fn name(&self) -> &'static str {
        match self {
            Workload::Ascii => "ascii",
            Workload::Sgr => "sgr",
            Workload::Csi => "csi",
            Workload::Esc => "esc",
            Workload::Unicode => "unicode",
        }
    }

    /// Texto de al menos `mib` MiB, siempre en líneas completas.
    fn generate(&self, mib: usize) -> Vec<u8> {
        let target = mib * 1024 * 1024;
        let mut out = Vec::with_capacity(target + 4096);
        let mut line = 0usize;
        while out.len() < target {
            match self {
                Workload::Ascii => {
                    out.extend_from_slice(
                        b"the quick brown fox jumps over the lazy dog, 0123456789 and back again\r\n",
                    );
                }
                Workload::Sgr => {
                    let color = line % 256;
                    out.extend_from_slice(
                        format!(
                            "\x1b[38;5;{color}m$ \x1b[1mbold\x1b[0m texto normal \
                             \x1b[4munderline\x1b[0m \x1b[7mreverse\x1b[0m fin\r\n"
                        )
                        .as_bytes(),
                    );
                }
                Workload::Csi => {
                    let row = line % 40 + 1;
                    let column = line % 120 + 1;
                    out.extend_from_slice(
                        format!("\x1b[{row};{column}Hcelda\x1b[2K\x1b[{row};{column}H").as_bytes(),
                    );
                }
                Workload::Esc => {
                    out.extend_from_slice(b"\x1b7\x1b[2J\x1b[Htexto\x1b[?25l\x1b[?25h\x1b8\r\n");
                }
                Workload::Unicode => {
                    out.extend_from_slice(
                        "日本語のテキストと emoji 🙂🚀 y acentos áéíóú ñ\r\n".as_bytes(),
                    );
                }
            }
            line += 1;
        }
        out
    }
}

struct TermSize(GridSize);

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.0.rows
    }

    fn screen_lines(&self) -> usize {
        self.0.rows
    }

    fn columns(&self) -> usize {
        self.0.columns
    }
}

struct Args {
    mib: usize,
    samples: usize,
    only: Option<Vec<String>>,
    help: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Args {
            mib: 4,
            samples: 40,
            only: None,
            help: false,
        };
        let mut rest = std::env::args().skip(1);
        while let Some(flag) = rest.next() {
            match flag.as_str() {
                "--mib" => {
                    if let Some(value) = rest.next() {
                        args.mib = value.parse().unwrap_or(args.mib).max(1);
                    }
                }
                "--samples" => {
                    if let Some(value) = rest.next() {
                        args.samples = value.parse().unwrap_or(args.samples).max(1);
                    }
                }
                "--only" => {
                    if let Some(value) = rest.next() {
                        args.only = Some(
                            value
                                .split(',')
                                .map(|item| item.trim().to_string())
                                .collect(),
                        );
                    }
                }
                "--help" | "-h" => args.help = true,
                other => {
                    eprintln!("opción desconocida: {other}");
                    args.help = true;
                }
            }
        }
        args
    }
}

fn print_help() {
    println!(
        "pipeline_bench — mide la tubería PTY → parseo → cuadro\n\n\
         uso: pipeline_bench [--mib N] [--samples N] [--only a,b] [--help]\n\n\
         --mib N       MiB por carga de trabajo (por defecto 4)\n\
         --samples N   muestras p50/p95 (por defecto 40)\n\
         --only a,b    solo estas cargas: ascii, sgr, csi, esc, unicode\n\n\
         el registro se escribe en bench/ y en bench/last.txt\n\
         ejecútalo con --release: en debug las cifras no sirven"
    );
}

fn row(workload: &str, stage: &str, samples: &[Duration], mib: f64) -> String {
    let (p50, p95) = percentiles(samples);
    let total: Duration = samples.iter().sum();
    let rate = if total.as_secs_f64() > 0.0 {
        mib / total.as_secs_f64()
    } else {
        0.0
    };
    format!(
        "{:<10} {:<8} {:>12} {:>12} {:>10.1}",
        workload,
        stage,
        micros(p50),
        micros(p95),
        rate
    )
}

fn micros(duration: Duration) -> String {
    format!("{:.0} µs", duration.as_secs_f64() * 1e6)
}

/// p50 y p95 por rango más cercano.
fn percentiles(samples: &[Duration]) -> (Duration, Duration) {
    if samples.is_empty() {
        return (Duration::ZERO, Duration::ZERO);
    }
    let mut sorted = samples.to_vec();
    sorted.sort();
    let pick = |p: f64| {
        let index = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
        sorted[index.saturating_sub(1).min(sorted.len() - 1)]
    };
    (pick(50.0), pick(95.0))
}

fn temp_path(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("port-{name}-{stamp}.bin"))
}

fn git_revision() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "sin-git".to_string())
}

/// Guarda el informe en `bench/`, con marca de tiempo y como `last.txt`.
fn write_log(text: &str) -> std::io::Result<()> {
    let dir = PathBuf::from("bench");
    std::fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    std::fs::write(dir.join(format!("{stamp}-{}.txt", git_revision())), text)?;
    std::fs::write(dir.join("last.txt"), text)?;
    Ok(())
}
