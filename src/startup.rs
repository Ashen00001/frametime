//! Startup / shader-compilation pattern: heavy stutter early that settles.

use crate::series::Series;
use crate::stutter::Spike;
use crate::util::{percentile_sorted, sorted};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StartupReport {
    pub head_s: f64,
    pub spikes_head: usize,
    pub spikes_total: usize,
    pub share_pct: f64,
    pub rate_head_per_s: f64,
    pub rate_tail_per_s: f64,
    /// Time of the last spike inside the head window: roughly when it settled.
    pub settle_s: f64,
    pub head_p99_ms: f64,
    pub tail_p99_ms: f64,
}

pub fn detect(series: &Series, spikes: &[Spike]) -> Option<StartupReport> {
    let dur = series.duration_s();
    if dur < 20.0 {
        return None;
    }
    let head_s = (0.25 * dur).min(30.0);
    let head: Vec<&Spike> = spikes.iter().filter(|s| s.t < head_s).collect();
    let tail_count = spikes.len() - head.len();
    if head.len() < 5 {
        return None;
    }
    let share = head.len() as f64 / spikes.len() as f64;
    let rate_head = head.len() as f64 / head_s;
    let tail_dur = dur - head_s;
    // Floor the tail at half an event so a perfectly clean tail gives a finite ratio.
    let rate_tail = (tail_count as f64).max(0.5) / tail_dur;
    if share < 0.5 || rate_head / rate_tail < 3.0 {
        return None;
    }
    let p99 = |keep: &dyn Fn(f64) -> bool| {
        let v: Vec<f64> = series
            .frames
            .iter()
            .filter(|f| keep(f.t))
            .map(|f| f.ms)
            .collect();
        percentile_sorted(&sorted(&v), 99.0)
    };
    Some(StartupReport {
        head_s,
        spikes_head: head.len(),
        spikes_total: spikes.len(),
        share_pct: 100.0 * share,
        rate_head_per_s: rate_head,
        rate_tail_per_s: tail_count as f64 / tail_dur,
        settle_s: head.last().map(|s| s.t).unwrap_or(0.0),
        head_p99_ms: p99(&|t| t < head_s),
        tail_p99_ms: p99(&|t| t >= head_s),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stutter::{detect as spikes_of, StutterCfg};
    use crate::synth::{generate, SynthSpec};

    #[test]
    fn finds_early_shader_stutter() {
        let s = generate(&SynthSpec {
            startup_stutter_s: Some(8.0),
            ..SynthSpec::default()
        });
        let sp = spikes_of(&s, StutterCfg::default());
        let r = detect(&s, &sp).expect("startup pattern");
        assert!(
            r.settle_s > 3.0 && r.settle_s <= 9.0,
            "settle {}",
            r.settle_s
        );
        assert!(r.share_pct > 90.0);
    }

    #[test]
    fn steady_run_has_no_startup_pattern() {
        let s = generate(&SynthSpec::default());
        let sp = spikes_of(&s, StutterCfg::default());
        assert!(detect(&s, &sp).is_none());
    }

    #[test]
    fn periodic_spikes_are_not_startup() {
        let s = generate(&SynthSpec {
            spike_every_s: Some(5.0),
            ..SynthSpec::default()
        });
        let sp = spikes_of(&s, StutterCfg::default());
        assert!(detect(&s, &sp).is_none());
    }
}
