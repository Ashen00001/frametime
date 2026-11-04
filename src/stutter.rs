//! Spike (stutter) detection, hitch counts, pacing and frame-budget views.

use crate::series::Series;
use crate::util::{mean, median, percentile_sorted, sorted, std_dev};
use rayon::prelude::*;
use serde::Serialize;

/// A frame must also exceed the local median by this much, so micro-jitter at
/// very high frame rates is not called stutter.
pub const MIN_EXCESS_MS: f64 = 2.0;
/// Fixed hitch thresholds: ~30 fps, 20 fps, 10 fps, 5 fps.
pub const HITCH_THRESHOLDS_MS: [f64; 4] = [33.4, 50.0, 100.0, 200.0];
/// Spikes closer together than this belong to one stutter event.
pub const EVENT_GAP_S: f64 = 0.25;
/// Frames within this factor of the budget count as on-budget.
pub const BUDGET_TOLERANCE: f64 = 1.10;

#[derive(Debug, Clone)]
pub struct Spike {
    pub idx: usize,
    pub t: f64,
    pub ms: f64,
    pub median_ms: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct StutterCfg {
    /// Spike if frame time exceeds this multiple of the rolling median.
    pub mult: f64,
    /// Rolling-median window in frames (centered).
    pub window: usize,
}

impl Default for StutterCfg {
    fn default() -> Self {
        StutterCfg {
            mult: 2.0,
            window: 61,
        }
    }
}

pub fn rolling_median(ms: &[f64], window: usize) -> Vec<f64> {
    let h = window.max(3) / 2;
    let n = ms.len();
    (0..n)
        .into_par_iter()
        .map(|i| {
            let lo = i.saturating_sub(h);
            let hi = (i + h + 1).min(n);
            let mut w = ms[lo..hi].to_vec();
            let mid = w.len() / 2;
            *w.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1
        })
        .collect()
}

pub fn detect(series: &Series, cfg: StutterCfg) -> Vec<Spike> {
    let ms = series.ms_vec();
    let med = rolling_median(&ms, cfg.window);
    series
        .frames
        .iter()
        .enumerate()
        .filter(|(i, f)| f.ms > cfg.mult * med[*i] && f.ms - med[*i] >= MIN_EXCESS_MS)
        .map(|(i, f)| Spike {
            idx: i,
            t: f.t,
            ms: f.ms,
            median_ms: med[i],
        })
        .collect()
}

/// Cluster spikes into events (first spike time of each cluster).
pub fn events(spikes: &[Spike]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for s in spikes {
        if s.t - last > EVENT_GAP_S {
            out.push(s.t);
        }
        last = s.t;
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct SpikeOut {
    pub t_s: f64,
    pub ms: f64,
    pub local_median_ms: f64,
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hitch {
    pub over_ms: f64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StutterReport {
    pub mult: f64,
    pub window_frames: usize,
    pub spike_frames: usize,
    pub spike_frames_pct: f64,
    pub spike_events: usize,
    pub events_per_min: f64,
    pub worst: Vec<SpikeOut>,
    pub hitches: Vec<Hitch>,
    /// Mean absolute difference between consecutive frame times.
    pub jitter_ms: f64,
    /// Coefficient of variation of frame time (std / mean).
    pub cv: f64,
    pub p99_over_p50: f64,
}

pub fn report(series: &Series, spikes: &[Spike], cfg: StutterCfg) -> StutterReport {
    let ms = series.ms_vec();
    let n = ms.len();
    let dur_min = (series.duration_s() / 60.0).max(1e-9);
    let ev = events(spikes);
    let mut worst: Vec<&Spike> = spikes.iter().collect();
    worst.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    let worst = worst
        .into_iter()
        .take(5)
        .map(|s| SpikeOut {
            t_s: s.t,
            ms: s.ms,
            local_median_ms: s.median_ms,
            ratio: s.ms / s.median_ms,
        })
        .collect();
    let hitches = HITCH_THRESHOLDS_MS
        .iter()
        .map(|&t| Hitch {
            over_ms: t,
            count: ms.iter().filter(|&&m| m > t).count(),
        })
        .collect();
    let diffs: Vec<f64> = ms.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let s = sorted(&ms);
    let m = mean(&ms);
    StutterReport {
        mult: cfg.mult,
        window_frames: cfg.window,
        spike_frames: spikes.len(),
        spike_frames_pct: 100.0 * spikes.len() as f64 / n.max(1) as f64,
        spike_events: ev.len(),
        events_per_min: ev.len() as f64 / dur_min,
        worst,
        hitches,
        jitter_ms: mean(&diffs),
        cv: std_dev(&ms) / m,
        p99_over_p50: percentile_sorted(&s, 99.0) / percentile_sorted(&s, 50.0),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Budget {
    pub target_hz: f64,
    pub budget_ms: f64,
    pub avg_fps: f64,
    /// Share of frames slower than the budget (with a 10% tolerance).
    pub over_budget_pct: f64,
    /// Share of frames that took at least two refresh intervals.
    pub missed_refresh_pct: f64,
    /// Share of wall-clock time spent in over-budget frames.
    pub time_over_budget_pct: f64,
    pub median_ms: f64,
}

pub fn budget(ms: &[f64], target_hz: f64) -> Budget {
    let b = 1000.0 / target_hz;
    let n = ms.len().max(1) as f64;
    let over: Vec<f64> = ms
        .iter()
        .copied()
        .filter(|&m| m > b * BUDGET_TOLERANCE)
        .collect();
    let total: f64 = ms.iter().sum();
    Budget {
        target_hz,
        budget_ms: b,
        avg_fps: 1000.0 / mean(ms),
        over_budget_pct: 100.0 * over.len() as f64 / n,
        missed_refresh_pct: 100.0 * ms.iter().filter(|&&m| m >= 2.0 * b).count() as f64 / n,
        time_over_budget_pct: 100.0 * over.iter().sum::<f64>() / total,
        median_ms: median(ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::Format;

    #[test]
    fn rolling_median_ignores_isolated_spike() {
        let mut ms = vec![7.0; 100];
        ms[50] = 70.0;
        let m = rolling_median(&ms, 31);
        assert!(m.iter().all(|&x| (x - 7.0).abs() < 1e-9));
    }

    #[test]
    fn detects_injected_spikes_only() {
        let mut ms = vec![7.0; 300];
        for i in [40, 120, 250] {
            ms[i] = 40.0;
        }
        // mild jitter must not count: 12 ms is < 2x of 7 ms
        ms[200] = 12.0;
        let s = Series::from_ms(Format::Plain, &ms);
        let sp = detect(&s, StutterCfg::default());
        let idx: Vec<usize> = sp.iter().map(|s| s.idx).collect();
        assert_eq!(idx, vec![40, 120, 250]);
    }

    #[test]
    fn tiny_frames_need_minimum_excess() {
        // 1 ms frames with a 2.5 ms frame: ratio 2.5 but only 1.5 ms excess
        let mut ms = vec![1.0; 100];
        ms[10] = 2.5;
        let s = Series::from_ms(Format::Plain, &ms);
        assert!(detect(&s, StutterCfg::default()).is_empty());
    }

    #[test]
    fn events_merge_adjacent_spikes() {
        let sp = |t: f64| Spike {
            idx: 0,
            t,
            ms: 50.0,
            median_ms: 7.0,
        };
        let ev = events(&[sp(1.0), sp(1.05), sp(1.1), sp(3.0)]);
        assert_eq!(ev, vec![1.0, 3.0]);
    }

    #[test]
    fn hitch_counts_and_budget() {
        let mut ms = vec![6.94; 96];
        ms.extend([40.0, 60.0, 120.0, 250.0]);
        let s = Series::from_ms(Format::Plain, &ms);
        let r = report(&s, &[], StutterCfg::default());
        let counts: Vec<usize> = r.hitches.iter().map(|h| h.count).collect();
        assert_eq!(counts, vec![4, 3, 2, 1]);
        let b = budget(&ms, 144.0);
        assert!((b.over_budget_pct - 4.0).abs() < 1e-9);
        assert!((b.missed_refresh_pct - 4.0).abs() < 1e-9);
    }
}
