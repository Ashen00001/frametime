//! Periodic stutter detection.
//!
//! Stutter events (clusters of spike frames) are treated as points in time.
//! A Rayleigh periodogram scans candidate periods: for period P every event
//! is mapped to a phase on the circle, and R = |mean of unit vectors| says how
//! tightly the events line up (1 = perfect repeat, ~0 = no relation). Events
//! at multiples of P also score 1 at P/2, P/3..., so the longest period that
//! scores near the maximum is reported. A candidate must also pass an
//! interval check (most gaps between events are about P or 2P) before being
//! reported, which keeps random spike times from producing false positives.

use crate::findings::Confidence;
use rayon::prelude::*;
use serde::Serialize;
use std::f64::consts::PI;

pub const MIN_PERIOD_S: f64 = 0.5;
pub const MIN_EVENTS: usize = 6;
const MAX_GRID: usize = 300_000;

#[derive(Debug, Clone, Serialize)]
pub struct Periodic {
    pub period_s: f64,
    /// Rayleigh resultant length in 0..=1 (how strongly events repeat).
    pub strength: f64,
    pub events: usize,
    /// Share of gaps between consecutive events that are ~1 period long.
    pub gaps_one_period_pct: f64,
    /// Share of gaps that are ~1, 2 or 3 periods long.
    pub gaps_matching_pct: f64,
    pub cycles: f64,
    pub confidence: Confidence,
}

fn resultant(events: &[f64], period: f64) -> f64 {
    let (mut c, mut s) = (0.0, 0.0);
    for &t in events {
        let a = 2.0 * PI * t / period;
        c += a.cos();
        s += a.sin();
    }
    (c * c + s * s).sqrt() / events.len() as f64
}

pub fn detect(events: &[f64], duration_s: f64) -> Option<Periodic> {
    let n = events.len();
    let pmax = duration_s / 4.0;
    if n < MIN_EVENTS || pmax <= MIN_PERIOD_S * 1.5 {
        return None;
    }
    let mut grid = Vec::new();
    let mut p = MIN_PERIOD_S;
    while p <= pmax && grid.len() < MAX_GRID {
        grid.push(p);
        p += (0.05 * p * p / duration_s).max(1e-4);
    }
    let r: Vec<f64> = grid.par_iter().map(|&p| resultant(events, p)).collect();
    let rmax = r.iter().copied().fold(0.0, f64::max);
    if rmax < 0.75 {
        return None;
    }
    // Longest local maximum scoring within 5% of the best.
    let mut best: Option<usize> = None;
    for i in 0..grid.len() {
        let left = if i > 0 { r[i - 1] } else { 0.0 };
        let right = r.get(i + 1).copied().unwrap_or(0.0);
        if r[i] >= 0.95 * rmax && r[i] >= left && r[i] >= right {
            best = Some(i);
        }
    }
    let i = best?;
    let period = grid[i];

    let gaps: Vec<f64> = events.windows(2).map(|w| w[1] - w[0]).collect();
    let near = |g: f64, k: f64| (g - k * period).abs() <= 0.15 * k * period;
    let one = gaps.iter().filter(|&&g| near(g, 1.0)).count();
    let any = gaps
        .iter()
        .filter(|&&g| near(g, 1.0) || near(g, 2.0) || near(g, 3.0))
        .count();
    let g = gaps.len() as f64;
    let (one_pct, any_pct) = (100.0 * one as f64 / g, 100.0 * any as f64 / g);
    if one_pct < 50.0 || any_pct < 70.0 {
        return None;
    }
    let strength = r[i];
    let confidence = if strength >= 0.9 && n >= 8 && one_pct >= 80.0 {
        Confidence::High
    } else if strength >= 0.8 && n >= 6 {
        Confidence::Medium
    } else {
        Confidence::Low
    };
    Some(Periodic {
        period_s: period,
        strength,
        events: n,
        gaps_one_period_pct: one_pct,
        gaps_matching_pct: any_pct,
        cycles: duration_s / period,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::Rng;

    fn train(period: f64, dur: f64, jitter: f64, seed: u64) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        let mut v = Vec::new();
        let mut t = 2.3;
        while t < dur {
            v.push(t + jitter * (rng.next_f64() - 0.5));
            t += period;
        }
        v
    }

    #[test]
    fn recovers_injected_period() {
        let ev = train(5.0, 120.0, 0.04, 1);
        let p = detect(&ev, 120.0).expect("periodic");
        assert!((p.period_s - 5.0).abs() < 0.05, "got {}", p.period_s);
        assert!(p.strength > 0.95);
        assert_eq!(p.confidence, Confidence::High);
    }

    #[test]
    fn recovers_period_with_missing_events() {
        let mut ev = train(7.5, 150.0, 0.05, 2);
        ev.remove(4);
        ev.remove(10);
        let p = detect(&ev, 150.0).expect("periodic");
        assert!((p.period_s - 7.5).abs() < 0.1, "got {}", p.period_s);
    }

    #[test]
    fn sub_second_periods_are_not_confused_with_multiples() {
        let ev = train(2.0, 60.0, 0.02, 3);
        let p = detect(&ev, 60.0).expect("periodic");
        assert!((p.period_s - 2.0).abs() < 0.05, "got {}", p.period_s);
    }

    #[test]
    fn random_events_are_not_periodic() {
        let mut rng = Rng::new(99);
        for trial in 0..20 {
            let mut ev: Vec<f64> = (0..15).map(|_| rng.next_f64() * 120.0).collect();
            ev.sort_by(|a, b| a.total_cmp(b));
            assert!(
                detect(&ev, 120.0).is_none(),
                "false positive in trial {trial}"
            );
        }
    }

    #[test]
    fn too_few_events() {
        assert!(detect(&[1.0, 6.0, 11.0], 60.0).is_none());
    }
}
