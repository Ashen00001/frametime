//! Present mode, tearing, dropped frames and display latency.

use crate::series::Series;
use crate::util::{mean, percentile_sorted, sorted};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ModeShare {
    pub mode: String,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyStats {
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Median latency expressed in median frame times.
    pub p50_in_frames: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresentReport {
    pub modes: Vec<ModeShare>,
    /// Share of frames presented via DWM composition ("Composed: ...").
    pub composed_pct: f64,
    pub mode_switches: usize,
    pub tearing_pct: Option<f64>,
    pub dropped_pct: Option<f64>,
    pub dropped_count: Option<usize>,
    pub latency: Option<LatencyStats>,
}

pub fn analyze(series: &Series) -> Option<PresentReport> {
    let moded: Vec<Option<u16>> = series.frames.iter().map(|f| f.mode).collect();
    let n_mode = moded.iter().flatten().count();
    let mut modes = Vec::new();
    let mut composed = 0usize;
    for (i, name) in series.modes.iter().enumerate() {
        let c = moded.iter().filter(|m| **m == Some(i as u16)).count();
        if name.to_ascii_lowercase().starts_with("composed") {
            composed += c;
        }
        modes.push(ModeShare {
            mode: name.clone(),
            pct: 100.0 * c as f64 / n_mode.max(1) as f64,
        });
    }
    modes.sort_by(|a, b| b.pct.total_cmp(&a.pct));
    let switches = moded
        .windows(2)
        .filter(|w| w[0].is_some() && w[1].is_some() && w[0] != w[1])
        .count();

    let tear: Vec<bool> = series.frames.iter().filter_map(|f| f.tearing).collect();
    let drop: Vec<bool> = series.frames.iter().filter_map(|f| f.dropped).collect();
    let lat: Vec<f64> = series.frames.iter().filter_map(|f| f.latency()).collect();
    let latency = (!lat.is_empty()).then(|| {
        let s = sorted(&lat);
        let med_ft = percentile_sorted(&sorted(&series.ms_vec()), 50.0);
        LatencyStats {
            avg_ms: mean(&s),
            p50_ms: percentile_sorted(&s, 50.0),
            p95_ms: percentile_sorted(&s, 95.0),
            p99_ms: percentile_sorted(&s, 99.0),
            max_ms: s.last().copied().unwrap_or(f64::NAN),
            p50_in_frames: percentile_sorted(&s, 50.0) / med_ft,
        }
    });
    if n_mode == 0 && tear.is_empty() && drop.is_empty() && latency.is_none() {
        return None;
    }
    let pct = |v: &[bool]| 100.0 * v.iter().filter(|&&b| b).count() as f64 / v.len().max(1) as f64;
    Some(PresentReport {
        modes,
        composed_pct: 100.0 * composed as f64 / n_mode.max(1) as f64,
        mode_switches: switches,
        tearing_pct: (!tear.is_empty()).then(|| pct(&tear)),
        dropped_pct: (!drop.is_empty()).then(|| pct(&drop)),
        dropped_count: (!drop.is_empty()).then(|| drop.iter().filter(|&&b| b).count()),
        latency,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::{Format, Frame};

    fn series() -> Series {
        let mut frames = Vec::new();
        for i in 0..10 {
            let mut f = Frame::new(i as f64 * 0.01, 10.0);
            f.mode = Some(if i < 6 { 0 } else { 1 });
            f.tearing = Some(i % 5 == 0);
            f.dropped = Some(i == 3);
            f.until_displayed = Some(25.0);
            frames.push(f);
        }
        let mut s = Series::new(Format::PresentMon1, frames);
        s.modes = vec!["Composed: Flip".into(), "Hardware: Independent Flip".into()];
        s
    }

    #[test]
    fn shares_and_latency() {
        let r = analyze(&series()).unwrap();
        assert!((r.composed_pct - 60.0).abs() < 1e-9);
        assert_eq!(r.mode_switches, 1);
        assert_eq!(r.modes[0].mode, "Composed: Flip");
        assert!((r.tearing_pct.unwrap() - 20.0).abs() < 1e-9);
        assert!((r.dropped_pct.unwrap() - 10.0).abs() < 1e-9);
        let l = r.latency.unwrap();
        assert!((l.p50_ms - 25.0).abs() < 1e-9);
        assert!((l.p50_in_frames - 2.5).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_report_without_columns() {
        let s = Series::from_ms(Format::Plain, &[7.0, 7.0, 7.0]);
        assert!(analyze(&s).is_none());
    }
}
