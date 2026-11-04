use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use frametime::compare::compare;
use frametime::parse::{self, ParseOpts};
use frametime::render::{self, Style};
use frametime::report::{analyze, AnalyzeOpts};
use frametime::sensors::{self, Sensors};
use frametime::series::Series;
use frametime::synth::{self, BoundKind, SynthSpec};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "frametime",
    version,
    about = "Trace performance problems in frame-time captures, and compare runs honestly. Read-only: never changes a capture, a game or a system setting."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyze one capture
    Analyze {
        /// PresentMon / CapFrameX / RTSS capture or a plain list of frame times (ms)
        file: PathBuf,
        /// HWiNFO or Afterburner sensor CSV to line up with the frames
        #[arg(long)]
        sensors: Option<PathBuf>,
        #[command(flatten)]
        common: Common,
        #[command(flatten)]
        out: OutOpts,
    },
    /// Compare two captures (A = before, B = after)
    Compare {
        a: PathBuf,
        b: PathBuf,
        #[command(flatten)]
        common: Common,
        #[command(flatten)]
        out: OutOpts,
    },
    /// Write a synthetic demo capture (PresentMon 2.x CSV) for trying the tool
    Synth {
        #[arg(value_enum)]
        kind: Kind,
        /// Output file
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 60.0)]
        seconds: f64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Clean,
    Periodic,
    Startup,
    CpuBound,
    GpuBound,
    Random,
}

#[derive(Args)]
struct Common {
    /// Target refresh rate for the frame-budget view
    #[arg(long, default_value_t = 144.0)]
    target_hz: f64,
    /// A frame is a spike above this multiple of the rolling median
    #[arg(long, default_value_t = 2.0)]
    spike_mult: f64,
    /// Rolling-median window in frames
    #[arg(long, default_value_t = 61)]
    window: usize,
    /// Number of equal time slices for per-phase stats
    #[arg(long, default_value_t = 3)]
    phases: usize,
    /// Ignore the first N seconds (loading screens, alt-tab)
    #[arg(long, default_value_t = 0.0)]
    trim_start: f64,
    /// Keep only this process (substring) in multi-process PresentMon captures
    #[arg(long)]
    process: Option<String>,
    /// Which run to read from a multi-run CapFrameX JSON (0-based)
    #[arg(long, default_value_t = 0)]
    run: usize,
    /// Seconds to add to capture time to get sensor-log time
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    sensor_offset: f64,
    /// GPU memory size, to judge VRAM fullness from MB columns
    #[arg(long)]
    vram_gb: Option<f64>,
    /// System RAM size, to judge RAM pressure from MB columns
    #[arg(long)]
    ram_gb: Option<f64>,
}

#[derive(Args)]
struct OutOpts {
    /// Machine-readable JSON on stdout
    #[arg(long, conflicts_with = "markdown")]
    json: bool,
    /// Markdown on stdout
    #[arg(long)]
    markdown: bool,
    /// Also write a self-contained HTML report to this path
    #[arg(long)]
    html: Option<PathBuf>,
    /// Terminal width for charts
    #[arg(long, default_value_t = 80)]
    width: usize,
}

impl Common {
    fn parse_opts(&self) -> ParseOpts {
        ParseOpts {
            process: self.process.clone(),
            run: self.run,
        }
    }

    fn analyze_opts(&self) -> AnalyzeOpts {
        AnalyzeOpts {
            target_hz: self.target_hz,
            spike_mult: self.spike_mult,
            window: self.window,
            sensor_offset: self.sensor_offset,
            vram_gb: self.vram_gb,
            ram_gb: self.ram_gb,
            phases: self.phases,
        }
    }

    fn load(&self, p: &Path) -> Result<Series> {
        let mut s = parse::load(p, &self.parse_opts())?;
        s.trim_start(self.trim_start);
        if s.len() < 2 {
            bail!("{}: nothing left after trimming", p.display());
        }
        Ok(s)
    }
}

fn load_sensors(p: &Path) -> Result<Sensors> {
    let bytes = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    sensors::parse(text.trim_start_matches('\u{feff}'))
        .with_context(|| format!("parsing sensors {}", p.display()))
}

fn write_html(path: &Path, inputs: &[&Path], html: &str) -> Result<()> {
    for i in inputs {
        if let (Ok(a), Ok(b)) = (path.canonicalize(), i.canonicalize()) {
            if a == b {
                bail!("refusing to overwrite the input capture {}", i.display());
            }
        }
    }
    std::fs::write(path, html).with_context(|| format!("writing {}", path.display()))
}

fn emit(doc: &[render::Block], json: String, out: &OutOpts, inputs: &[&Path]) -> Result<()> {
    if out.json {
        println!("{json}");
    } else if out.markdown {
        print!("{}", render::markdown(doc));
    } else {
        print!("{}", render::terminal(doc, &Style::auto(), out.width));
    }
    if let Some(p) = &out.html {
        write_html(p, inputs, &render::html(doc))?;
        eprintln!("wrote {}", p.display());
    }
    Ok(())
}

fn run() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Analyze {
            file,
            sensors,
            common,
            out,
        } => {
            let series = common.load(&file)?;
            let sens = sensors.as_deref().map(load_sensors).transpose()?;
            let opts = common.analyze_opts();
            let r = analyze(&series, sens.as_ref(), &opts);
            let doc = render::analysis_doc(&r, &series, opts.target_hz);
            emit(&doc, serde_json::to_string_pretty(&r)?, &out, &[&file])
        }
        Cmd::Compare { a, b, common, out } => {
            let (sa, sb) = (common.load(&a)?, common.load(&b)?);
            let c = compare(&sa, &sb, &common.analyze_opts());
            let name = |p: &Path| {
                p.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            };
            let doc = render::compare_doc(&c, &name(&a), &name(&b));
            emit(&doc, serde_json::to_string_pretty(&c)?, &out, &[&a, &b])
        }
        Cmd::Synth {
            kind,
            out,
            seconds,
            seed,
        } => {
            let mut spec = SynthSpec {
                seconds,
                seed,
                ..SynthSpec::default()
            };
            match kind {
                Kind::Clean => {}
                Kind::Periodic => spec.spike_every_s = Some(5.0),
                Kind::Startup => spec.startup_stutter_s = Some(8.0),
                Kind::CpuBound => {
                    spec.bound = BoundKind::Cpu;
                    spec.base_ms = 12.5;
                }
                Kind::GpuBound => {
                    spec.bound = BoundKind::Gpu;
                    spec.base_ms = 11.0;
                }
                Kind::Random => spec.random_spike_rate_hz = 0.2,
            }
            let s = synth::generate(&spec);
            std::fs::write(&out, synth::to_presentmon2_csv(&s))?;
            eprintln!("wrote {} ({} frames)", out.display(), s.len());
            Ok(())
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
