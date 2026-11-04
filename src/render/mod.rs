//! Output. Reports are first turned into a small document model (`Block`s),
//! then rendered as terminal text, Markdown or a self-contained HTML page.

mod doc;
mod html;
mod md;
mod term;

pub use doc::{analysis_doc, compare_doc};
pub use html::html;
pub use md::markdown;
pub use term::{terminal, Style};

use crate::findings::Confidence;
use crate::util::{percentile_sorted, sorted};

#[derive(Debug, Clone)]
pub enum Block {
    Title(String),
    Heading(String),
    Para(String),
    Bullets(Vec<String>),
    Table {
        head: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Finding {
        rank: usize,
        title: String,
        confidence: Confidence,
        evidence: Vec<String>,
        caveat: String,
    },
    Verdict(String),
    /// Frame times over the run, with the frame-budget line.
    Chart {
        ms: Vec<f64>,
        budget_ms: f64,
    },
    Hist {
        ms: Vec<f64>,
    },
}

const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Shrink to `width` points, keeping the maximum of each bucket so spikes survive.
pub fn downsample_max(ms: &[f64], width: usize) -> Vec<f64> {
    let n = ms.len();
    if n <= width || width == 0 {
        return ms.to_vec();
    }
    (0..width)
        .map(|k| {
            let (a, b) = (k * n / width, ((k + 1) * n / width).max(k * n / width + 1));
            ms[a..b.min(n)].iter().copied().fold(0.0, f64::max)
        })
        .collect()
}

/// Returns (sparkline, top-of-scale ms).
pub fn sparkline(ms: &[f64], width: usize) -> (String, f64) {
    let top = (percentile_sorted(&sorted(ms), 99.9) * 1.05).max(1e-9);
    let s = downsample_max(ms, width)
        .iter()
        .map(|&v| LEVELS[((v / top * 7.0).round().clamp(0.0, 7.0)) as usize])
        .collect();
    (s, top)
}

pub struct HistBin {
    pub lo: f64,
    pub hi: f64,
    pub count: usize,
    pub pct: f64,
}

/// 12 equal bins from the fastest frame to p99.9, plus an overflow bin (hi = inf).
pub fn histogram(ms: &[f64]) -> Vec<HistBin> {
    let s = sorted(ms);
    if s.is_empty() {
        return Vec::new();
    }
    let lo = s[0];
    let hi = percentile_sorted(&s, 99.9).max(lo + 1e-6);
    let bins = 12;
    let w = (hi - lo) / bins as f64;
    let mut c = vec![0usize; bins + 1];
    for &v in &s {
        let k = if v > hi {
            bins
        } else {
            (((v - lo) / w) as usize).min(bins - 1)
        };
        c[k] += 1;
    }
    let n = s.len() as f64;
    c.iter()
        .enumerate()
        .filter(|(k, n)| *k < bins || **n > 0)
        .map(|(k, &count)| HistBin {
            lo: if k == bins { hi } else { lo + w * k as f64 },
            hi: if k == bins {
                f64::INFINITY
            } else {
                lo + w * (k + 1) as f64
            },
            count,
            pct: 100.0 * count as f64 / n,
        })
        .collect()
}

pub fn hist_lines(ms: &[f64]) -> Vec<String> {
    let bins = histogram(ms);
    let max = bins.iter().map(|b| b.count).max().unwrap_or(1).max(1);
    bins.iter()
        .map(|b| {
            let len = (b.count as f64 / max as f64 * 30.0).round() as usize;
            let len = if b.count > 0 { len.max(1) } else { 0 };
            let range = if b.hi.is_finite() {
                format!("{:>7.2} - {:<7.2}", b.lo, b.hi)
            } else {
                format!("{:>7.2} +        ", b.lo)
            };
            format!("{range} ms |{:<30}| {:>5.1}%", "█".repeat(len), b.pct)
        })
        .collect()
}

pub fn fx(x: f64, d: usize) -> String {
    if x.is_finite() {
        format!("{x:.d$}")
    } else {
        "n/a".into()
    }
}

pub fn signed_pct(x: Option<f64>) -> String {
    x.map_or("n/a".into(), |v| format!("{v:+.1}%"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_keeps_spikes() {
        let mut v = vec![1.0; 1000];
        v[503] = 99.0;
        let d = downsample_max(&v, 50);
        assert_eq!(d.len(), 50);
        assert!(d.contains(&99.0));
    }

    #[test]
    fn sparkline_marks_spike_with_full_block() {
        let mut v = vec![7.0; 500];
        v[250] = 70.0;
        v[251] = 70.0;
        v[252] = 70.0;
        let (s, _) = sparkline(&v, 100);
        assert_eq!(s.chars().count(), 100);
        assert!(s.contains('█'));
    }

    #[test]
    fn histogram_counts_everything() {
        let mut v = vec![7.0; 990];
        v.extend(vec![50.0; 10]);
        let h = histogram(&v);
        assert_eq!(h.iter().map(|b| b.count).sum::<usize>(), 1000);
        assert!(!hist_lines(&v).is_empty());
    }
}
