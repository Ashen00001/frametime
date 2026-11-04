//! Runs every analysis over one capture and assembles the report.

use crate::bound::{self, BoundReport};
use crate::findings::{self, Finding};
use crate::periodic::{self, Periodic};
use crate::present::{self, PresentReport};
use crate::sensors::{self, SensorCfg, SensorReport, Sensors};
use crate::series::{Format, Series};
use crate::startup::{self, StartupReport};
use crate::stats::{self, Stats};
use crate::stutter::{self, Budget, Spike, StutterCfg, StutterReport};
use crate::util::{percentile_sorted, sorted};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct AnalyzeOpts {
    pub target_hz: f64,
    pub spike_mult: f64,
    pub window: usize,
    pub sensor_offset: f64,
    pub vram_gb: Option<f64>,
    pub ram_gb: Option<f64>,
    pub phases: usize,
}

impl Default for AnalyzeOpts {
    fn default() -> Self {
        AnalyzeOpts {
            target_hz: 144.0,
            spike_mult: 2.0,
            window: 61,
            sensor_offset: 0.0,
            vram_gb: None,
            ram_gb: None,
            phases: 3,
        }
    }
}

impl AnalyzeOpts {
    pub fn stutter_cfg(&self) -> StutterCfg {
        StutterCfg {
            mult: self.spike_mult,
            window: self.window,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub file: String,
    pub format: Format,
    pub format_label: String,
    pub app: Option<String>,
    pub frames: usize,
    pub duration_s: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseStats {
    pub label: String,
    pub start_s: f64,
    pub end_s: f64,
    pub frames: usize,
    pub avg_fps: f64,
    pub low1_pct_fps: f64,
    pub p99_ms: f64,
    pub spikes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub source: SourceInfo,
    pub stats: Stats,
    pub budget: Budget,
    pub stutter: StutterReport,
    pub bound: Option<BoundReport>,
    pub periodic: Option<Periodic>,
    pub startup: Option<StartupReport>,
    pub present: Option<PresentReport>,
    pub sensors: Option<SensorReport>,
    pub phases: Vec<PhaseStats>,
    pub findings: Vec<Finding>,
}

pub fn phase_stats(series: &Series, spikes: &[Spike], n: usize) -> Vec<PhaseStats> {
    let n = n.max(1);
    let dur = series.duration_s();
    (0..n)
        .filter_map(|k| {
            let (a, b) = (dur * k as f64 / n as f64, dur * (k + 1) as f64 / n as f64);
            let last = k + 1 == n;
            let ms: Vec<f64> = series
                .frames
                .iter()
                .filter(|f| f.t >= a && (f.t < b || last))
                .map(|f| f.ms)
                .collect();
            if ms.len() < 2 {
                return None;
            }
            let s = sorted(&ms);
            Some(PhaseStats {
                label: format!("{a:.0}-{b:.0} s"),
                start_s: a,
                end_s: b,
                frames: ms.len(),
                avg_fps: 1000.0 / (ms.iter().sum::<f64>() / ms.len() as f64),
                low1_pct_fps: stats::low_percentile_fps(&s, 1.0),
                p99_ms: percentile_sorted(&s, 99.0),
                spikes: spikes
                    .iter()
                    .filter(|sp| sp.t >= a && (sp.t < b || last))
                    .count(),
            })
        })
        .collect()
}

pub fn analyze(series: &Series, sensors_in: Option<&Sensors>, opts: &AnalyzeOpts) -> Report {
    let ms = series.ms_vec();
    let spikes = stutter::detect(series, opts.stutter_cfg());
    let stats = stats::compute(&ms);
    let budget = stutter::budget(&ms, opts.target_hz);
    let stutter_r = stutter::report(series, &spikes, opts.stutter_cfg());
    let util = sensors_in.and_then(|s| sensors::gpu_util_per_frame(series, s, opts.sensor_offset));
    let bound = bound::analyze(series, &spikes, util.as_deref());
    let periodic = periodic::detect(&stutter::events(&spikes), series.duration_s());
    let startup = startup::detect(series, &spikes);
    let present = present::analyze(series);
    let sensors_r = sensors_in.map(|s| {
        sensors::analyze(
            series,
            &spikes,
            s,
            &SensorCfg {
                offset_s: opts.sensor_offset,
                vram_gb: opts.vram_gb,
                ram_gb: opts.ram_gb,
            },
        )
    });
    let mut r = Report {
        source: SourceInfo {
            file: series.source_name.clone(),
            format: series.format,
            format_label: series.format.label().to_string(),
            app: series.app.clone(),
            frames: series.len(),
            duration_s: series.duration_s(),
            notes: series.notes.clone(),
        },
        stats,
        budget,
        stutter: stutter_r,
        bound,
        periodic,
        startup,
        present,
        sensors: sensors_r,
        phases: phase_stats(series, &spikes, opts.phases),
        findings: Vec::new(),
    };
    r.findings = findings::build(&r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{generate, SynthSpec};

    #[test]
    fn periodic_run_end_to_end() {
        let s = generate(&SynthSpec {
            seconds: 120.0,
            spike_every_s: Some(5.0),
            ..SynthSpec::default()
        });
        let r = analyze(&s, None, &AnalyzeOpts::default());
        let p = r.periodic.as_ref().expect("periodic found");
        assert!((p.period_s - 5.0).abs() < 0.05);
        assert!(r.findings.iter().any(|f| f.title.contains("repeat")));
        assert!(r.startup.is_none());
        assert_eq!(r.phases.len(), 3);
    }

    #[test]
    fn clean_run_has_no_problem_findings() {
        let s = generate(&SynthSpec::default());
        let r = analyze(&s, None, &AnalyzeOpts::default());
        assert!(r.stutter.spike_frames == 0);
        assert!(
            r.findings.iter().all(|f| f.score < 30.0),
            "{:?}",
            r.findings
        );
    }
}
