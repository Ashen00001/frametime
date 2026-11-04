//! Core statistics: average fps, percentile and average-of-worst-N lows.
//!
//! Two common "1% low" definitions exist and they disagree, so both are
//! reported and named:
//! * percentile method: fps at the 99th percentile frame time (1000 / p99);
//! * average-of-worst-N method: fps of the mean of the slowest 1% of frames
//!   (N = ceil(frames * 1%)).

use crate::util::{mean, percentile_sorted, sorted, std_dev};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub frames: usize,
    pub duration_s: f64,
    pub avg_fps: f64,
    pub mean_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub std_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub p999_ms: f64,
    /// 1% low, percentile method (1000 / p99 frame time).
    pub low1_pct_fps: f64,
    /// 0.1% low, percentile method (1000 / p99.9 frame time).
    pub low01_pct_fps: f64,
    /// 1% low, average-of-worst-N method.
    pub low1_avg_fps: f64,
    /// 0.1% low, average-of-worst-N method.
    pub low01_avg_fps: f64,
}

/// fps at the (100 - pct)th percentile of frame time. `sorted_ms` ascending.
pub fn low_percentile_fps(sorted_ms: &[f64], pct: f64) -> f64 {
    1000.0 / percentile_sorted(sorted_ms, 100.0 - pct)
}

/// fps of the mean of the slowest `pct`% of frames (at least one frame).
pub fn low_avg_fps(sorted_ms: &[f64], pct: f64) -> f64 {
    let n = sorted_ms.len();
    if n == 0 {
        return f64::NAN;
    }
    let worst = ((n as f64 * pct / 100.0).ceil() as usize).clamp(1, n);
    1000.0 / mean(&sorted_ms[n - worst..])
}

pub fn compute(ms: &[f64]) -> Stats {
    let s = sorted(ms);
    let n = s.len();
    let total: f64 = s.iter().sum();
    let mean_ms = if n == 0 { f64::NAN } else { total / n as f64 };
    Stats {
        frames: n,
        duration_s: total / 1000.0,
        avg_fps: 1000.0 / mean_ms,
        mean_ms,
        min_ms: s.first().copied().unwrap_or(f64::NAN),
        max_ms: s.last().copied().unwrap_or(f64::NAN),
        std_ms: std_dev(&s),
        p50_ms: percentile_sorted(&s, 50.0),
        p90_ms: percentile_sorted(&s, 90.0),
        p95_ms: percentile_sorted(&s, 95.0),
        p99_ms: percentile_sorted(&s, 99.0),
        p999_ms: percentile_sorted(&s, 99.9),
        low1_pct_fps: low_percentile_fps(&s, 1.0),
        low01_pct_fps: low_percentile_fps(&s, 0.1),
        low1_avg_fps: low_avg_fps(&s, 1.0),
        low01_avg_fps: low_avg_fps(&s, 0.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hundred() -> Vec<f64> {
        (1..=100).map(f64::from).collect()
    }

    #[test]
    fn average_fps_is_frames_over_time() {
        // 4 frames of 10 ms and 1 of 60 ms: 100 ms total for 5 frames = 50 fps
        let st = compute(&[10.0, 10.0, 10.0, 10.0, 60.0]);
        assert!((st.avg_fps - 50.0).abs() < 1e-9);
        assert!((st.duration_s - 0.1).abs() < 1e-12);
    }

    #[test]
    fn percentile_low_matches_hand_calc() {
        let s = hundred();
        // p99 = 99.01 ms -> 1000 / 99.01
        assert!((low_percentile_fps(&s, 1.0) - 1000.0 / 99.01).abs() < 1e-9);
        // p99.9: rank 0.999 * 99 = 98.901 -> 99 + 0.901 = 99.901
        assert!((low_percentile_fps(&s, 0.1) - 1000.0 / 99.901).abs() < 1e-9);
    }

    #[test]
    fn average_of_worst_n_matches_hand_calc() {
        // 100 frames: worst 1% = 1 frame = 100 ms -> 10 fps
        assert!((low_avg_fps(&hundred(), 1.0) - 10.0).abs() < 1e-9);
        // 200 frames: worst 1% = 2 frames (199, 200) -> mean 199.5
        let s: Vec<f64> = (1..=200).map(f64::from).collect();
        assert!((low_avg_fps(&s, 1.0) - 1000.0 / 199.5).abs() < 1e-9);
        // 0.1% of 100 frames still uses at least one frame
        assert!((low_avg_fps(&hundred(), 0.1) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn percentiles_in_summary() {
        let st = compute(&hundred());
        assert!((st.p50_ms - 50.5).abs() < 1e-9);
        assert!((st.p95_ms - 95.05).abs() < 1e-9);
        assert_eq!(st.min_ms, 1.0);
        assert_eq!(st.max_ms, 100.0);
    }
}
