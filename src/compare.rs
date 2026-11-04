//! Compare two runs. Noise is estimated from the runs themselves: each is cut
//! into time blocks and block-bootstrapped, so a difference only counts when
//! its 95% interval excludes zero *and* it is at least 2% in size.
//!
//! This captures variability inside each run only. Two different sessions of
//! the same scene differ more than that, so treat "better"/"worse" as a lower
//! bar than a repeated-measures test.

use crate::report::{analyze, AnalyzeOpts, PhaseStats, Report};
use crate::series::Series;
use crate::stutter;
use crate::util::{percentile_sorted, sorted, Rng};
use rayon::prelude::*;
use serde::Serialize;

pub const MIN_EFFECT_PCT: f64 = 2.0;
const TARGET_BLOCK_S: f64 = 5.0;
const MAX_BLOCKS: usize = 20;
const MIN_BLOCKS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    Better,
    Worse,
    /// Difference is smaller than run-internal variability.
    WithinNoise,
    /// Statistically visible but under 2%: not worth acting on.
    Negligible,
    /// Too little data to say.
    Inconclusive,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Better => "better",
            Verdict::Worse => "worse",
            Verdict::WithinNoise => "within noise",
            Verdict::Negligible => "negligible (<2%)",
            Verdict::Inconclusive => "inconclusive",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricCmp {
    pub key: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    pub higher_is_better: bool,
    pub a: f64,
    pub b: f64,
    pub change_pct: Option<f64>,
    /// 95% interval of (b - a) from the block bootstrap.
    pub diff_ci_low: f64,
    pub diff_ci_high: f64,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseCmp {
    pub label: String,
    pub a_avg_fps: f64,
    pub b_avg_fps: f64,
    pub avg_fps_change_pct: f64,
    pub a_low1_fps: f64,
    pub b_low1_fps: f64,
    pub low1_change_pct: f64,
    pub a_spikes: usize,
    pub b_spikes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub a: Report,
    pub b: Report,
    pub metrics: Vec<MetricCmp>,
    pub phases: Vec<PhaseCmp>,
    pub overall: String,
    pub warnings: Vec<String>,
    pub blocks_a: usize,
    pub blocks_b: usize,
    pub resamples: usize,
}

struct Block {
    ms: Vec<f64>,
    spikes: usize,
    over: usize,
    absdiff: f64,
}

fn make_blocks(series: &Series, spike_idx: &[usize], budget_ms: f64) -> Vec<Block> {
    let dur = series.duration_s().max(1e-9);
    let nb = ((dur / TARGET_BLOCK_S) as usize).clamp(1, MAX_BLOCKS);
    let mut blocks: Vec<Block> = (0..nb)
        .map(|_| Block {
            ms: Vec::new(),
            spikes: 0,
            over: 0,
            absdiff: 0.0,
        })
        .collect();
    let bix = |t: f64| ((t / dur * nb as f64) as usize).min(nb - 1);
    let mut prev: Option<(usize, f64)> = None;
    for f in &series.frames {
        let b = bix(f.t);
        blocks[b].ms.push(f.ms);
        if f.ms > budget_ms * stutter::BUDGET_TOLERANCE {
            blocks[b].over += 1;
        }
        if let Some((pb, pm)) = prev {
            if pb == b {
                blocks[b].absdiff += (f.ms - pm).abs();
            }
        }
        prev = Some((b, f.ms));
    }
    for &i in spike_idx {
        blocks[bix(series.frames[i].t)].spikes += 1;
    }
    blocks.retain(|b| b.ms.len() >= 2);
    blocks
}

type MetricFn = fn(&[&Block], f64) -> f64;

fn all_ms(bl: &[&Block]) -> Vec<f64> {
    sorted(
        &bl.iter()
            .flat_map(|b| b.ms.iter().copied())
            .collect::<Vec<_>>(),
    )
}
fn total_ms(bl: &[&Block]) -> f64 {
    bl.iter().map(|b| b.ms.iter().sum::<f64>()).sum()
}
fn count(bl: &[&Block]) -> f64 {
    bl.iter().map(|b| b.ms.len()).sum::<usize>() as f64
}

fn m_avg(bl: &[&Block], _: f64) -> f64 {
    1000.0 * count(bl) / total_ms(bl)
}
fn m_low1(bl: &[&Block], _: f64) -> f64 {
    1000.0 / percentile_sorted(&all_ms(bl), 99.0)
}
fn m_low01(bl: &[&Block], _: f64) -> f64 {
    1000.0 / percentile_sorted(&all_ms(bl), 99.9)
}
fn m_p99(bl: &[&Block], _: f64) -> f64 {
    percentile_sorted(&all_ms(bl), 99.0)
}
fn m_p50(bl: &[&Block], _: f64) -> f64 {
    percentile_sorted(&all_ms(bl), 50.0)
}
fn m_stutter(bl: &[&Block], _: f64) -> f64 {
    bl.iter().map(|b| b.spikes).sum::<usize>() as f64 / (total_ms(bl) / 60_000.0)
}
fn m_jitter(bl: &[&Block], _: f64) -> f64 {
    let n: usize = bl.iter().map(|b| b.ms.len() - 1).sum();
    bl.iter().map(|b| b.absdiff).sum::<f64>() / n.max(1) as f64
}
fn m_over(bl: &[&Block], _: f64) -> f64 {
    100.0 * bl.iter().map(|b| b.over).sum::<usize>() as f64 / count(bl)
}

const METRICS: [(&str, &str, &str, bool, MetricFn); 8] = [
    ("avg_fps", "average fps", "fps", true, m_avg),
    ("low1_pct_fps", "1% low (percentile)", "fps", true, m_low1),
    (
        "low01_pct_fps",
        "0.1% low (percentile)",
        "fps",
        true,
        m_low01,
    ),
    ("p50_ms", "median frame time", "ms", false, m_p50),
    ("p99_ms", "p99 frame time", "ms", false, m_p99),
    (
        "stutter_per_min",
        "stutter spikes",
        "/min",
        false,
        m_stutter,
    ),
    ("jitter_ms", "frame-to-frame jitter", "ms", false, m_jitter),
    ("over_budget_pct", "frames over budget", "%", false, m_over),
];

fn resample<'a>(blocks: &'a [Block], rng: &mut Rng) -> Vec<&'a Block> {
    (0..blocks.len())
        .map(|_| &blocks[rng.below(blocks.len())])
        .collect()
}

fn verdict_for(
    diff: f64,
    a: f64,
    lo: f64,
    hi: f64,
    higher_better: bool,
    low_conf: bool,
) -> Verdict {
    let rel = (a != 0.0).then(|| diff / a.abs() * 100.0);
    let dir = |d: f64| {
        if (d > 0.0) == higher_better {
            Verdict::Better
        } else {
            Verdict::Worse
        }
    };
    if low_conf {
        return match rel {
            Some(r) if r.abs() >= 15.0 => dir(diff),
            _ => Verdict::Inconclusive,
        };
    }
    if !(lo > 0.0 || hi < 0.0) {
        return Verdict::WithinNoise;
    }
    match rel {
        Some(r) if r.abs() < MIN_EFFECT_PCT => Verdict::Negligible,
        _ => dir(diff),
    }
}

fn pct(a: f64, b: f64) -> f64 {
    (b - a) / a * 100.0
}

pub fn compare(a: &Series, b: &Series, opts: &AnalyzeOpts) -> CompareReport {
    let ra = analyze(a, None, opts);
    let rb = analyze(b, None, opts);
    let budget = 1000.0 / opts.target_hz;
    let sa: Vec<usize> = stutter::detect(a, opts.stutter_cfg())
        .iter()
        .map(|s| s.idx)
        .collect();
    let sb: Vec<usize> = stutter::detect(b, opts.stutter_cfg())
        .iter()
        .map(|s| s.idx)
        .collect();
    let (ba, bb) = (make_blocks(a, &sa, budget), make_blocks(b, &sb, budget));
    let low_conf = ba.len() < MIN_BLOCKS || bb.len() < MIN_BLOCKS;
    let total = (ra.stats.frames + rb.stats.frames).max(1);
    let resamples = (30_000_000 / total).clamp(100, 1000);

    let all_a: Vec<&Block> = ba.iter().collect();
    let all_b: Vec<&Block> = bb.iter().collect();
    let metrics: Vec<MetricCmp> = METRICS
        .iter()
        .map(|&(key, name, unit, hib, f)| {
            let (pa, pb) = (f(&all_a, budget), f(&all_b, budget));
            let mut diffs: Vec<f64> = (0..resamples)
                .into_par_iter()
                .map(|r| {
                    let mut rng = Rng::new(0xF4A3_u64 ^ (r as u64).wrapping_mul(0x9E37_79B9));
                    let xa = resample(&ba, &mut rng);
                    let xb = resample(&bb, &mut rng);
                    f(&xb, budget) - f(&xa, budget)
                })
                .collect();
            diffs.sort_by(|x, y| x.total_cmp(y));
            let (mut lo, mut hi) = (
                percentile_sorted(&diffs, 2.5),
                percentile_sorted(&diffs, 97.5),
            );
            // Rare-event counts: the block bootstrap is too optimistic, so also
            // require a Poisson rate test to agree before calling a difference.
            let rate = |bl: &[&Block]| match key {
                "stutter_per_min" => Some((
                    bl.iter().map(|b| b.spikes).sum::<usize>() as f64,
                    total_ms(bl),
                )),
                "over_budget_pct" => {
                    Some((bl.iter().map(|b| b.over).sum::<usize>() as f64, count(bl)))
                }
                _ => None,
            };
            if let (Some((ca, ea)), Some((cb, eb))) = (rate(&all_a), rate(&all_b)) {
                let se = (ca / (ea * ea) + cb / (eb * eb)).sqrt();
                let z = if se > 0.0 {
                    (cb / eb - ca / ea).abs() / se
                } else {
                    0.0
                };
                // 99% level: counts are clustered, so plain Poisson overstates significance.
                if z < 2.58 {
                    lo = lo.min(0.0);
                    hi = hi.max(0.0);
                }
            }
            MetricCmp {
                key,
                name,
                unit,
                higher_is_better: hib,
                a: pa,
                b: pb,
                change_pct: (pa != 0.0).then(|| pct(pa, pb)),
                diff_ci_low: lo,
                diff_ci_high: hi,
                verdict: verdict_for(pb - pa, pa, lo, hi, hib, low_conf),
            }
        })
        .collect();

    let phases = ra
        .phases
        .iter()
        .zip(&rb.phases)
        .map(|(x, y): (&PhaseStats, &PhaseStats)| PhaseCmp {
            label: x.label.clone(),
            a_avg_fps: x.avg_fps,
            b_avg_fps: y.avg_fps,
            avg_fps_change_pct: pct(x.avg_fps, y.avg_fps),
            a_low1_fps: x.low1_pct_fps,
            b_low1_fps: y.low1_pct_fps,
            low1_change_pct: pct(x.low1_pct_fps, y.low1_pct_fps),
            a_spikes: x.spikes,
            b_spikes: y.spikes,
        })
        .collect();

    let mut warnings = Vec::new();
    if low_conf {
        warnings.push(format!(
            "small sample: {} and {} usable time blocks (need {MIN_BLOCKS}+ each, i.e. roughly 30 s+); \
             only changes of 15% or more are called, everything else is inconclusive",
            ba.len(),
            bb.len()
        ));
    }
    if ra.source.app != rb.source.app {
        warnings.push(format!(
            "different applications: {:?} vs {:?}",
            ra.source.app, rb.source.app
        ));
    }
    let (da, db) = (ra.stats.duration_s, rb.stats.duration_s);
    if da.max(db) > 1.3 * da.min(db) {
        warnings.push(format!(
            "capture lengths differ a lot ({da:.0} s vs {db:.0} s); lows and counts depend on length"
        ));
    }
    warnings.push(
        "noise is estimated from variability inside each run only; two separate sessions of the same \
         scene vary more, so repeat the test before trusting small changes"
            .into(),
    );

    let key = ["avg_fps", "low1_pct_fps", "p99_ms", "stutter_per_min"];
    let keyed: Vec<&MetricCmp> = metrics.iter().filter(|m| key.contains(&m.key)).collect();
    let better: Vec<&str> = keyed
        .iter()
        .filter(|m| m.verdict == Verdict::Better)
        .map(|m| m.name)
        .collect();
    let worse: Vec<&str> = keyed
        .iter()
        .filter(|m| m.verdict == Verdict::Worse)
        .map(|m| m.name)
        .collect();
    let overall = if keyed.iter().all(|m| m.verdict == Verdict::Inconclusive) {
        "INCONCLUSIVE: not enough data to separate the runs from noise".to_string()
    } else if !better.is_empty() && worse.is_empty() {
        format!("B is better than A (beyond noise): {}", better.join(", "))
    } else if better.is_empty() && !worse.is_empty() {
        format!("B is worse than A (beyond noise): {}", worse.join(", "))
    } else if !better.is_empty() {
        format!(
            "MIXED: better {}; worse {}",
            better.join(", "),
            worse.join(", ")
        )
    } else {
        "NO MEASURABLE DIFFERENCE: changes are within noise or under 2%".to_string()
    };

    CompareReport {
        a: ra,
        b: rb,
        metrics,
        phases,
        overall,
        warnings,
        blocks_a: ba.len(),
        blocks_b: bb.len(),
        resamples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{generate, SynthSpec};

    fn find<'a>(r: &'a CompareReport, k: &str) -> &'a MetricCmp {
        r.metrics.iter().find(|m| m.key == k).unwrap()
    }

    #[test]
    fn real_improvement_is_called() {
        let a = generate(&SynthSpec {
            seconds: 120.0,
            base_ms: 10.0,
            random_spike_rate_hz: 0.3,
            seed: 1,
            ..SynthSpec::default()
        });
        let b = generate(&SynthSpec {
            seconds: 120.0,
            base_ms: 7.0,
            seed: 2,
            ..SynthSpec::default()
        });
        let r = compare(&a, &b, &AnalyzeOpts::default());
        assert_eq!(find(&r, "avg_fps").verdict, Verdict::Better);
        assert_eq!(find(&r, "stutter_per_min").verdict, Verdict::Better);
        assert!(r.overall.starts_with("B is better"), "{}", r.overall);
        assert!(find(&r, "avg_fps").change_pct.unwrap() > 30.0);
    }

    #[test]
    fn regression_is_called() {
        let a = generate(&SynthSpec {
            seconds: 120.0,
            seed: 1,
            ..SynthSpec::default()
        });
        let b = generate(&SynthSpec {
            seconds: 120.0,
            base_ms: 9.0,
            seed: 2,
            ..SynthSpec::default()
        });
        let r = compare(&a, &b, &AnalyzeOpts::default());
        assert_eq!(find(&r, "avg_fps").verdict, Verdict::Worse);
        assert!(r.overall.starts_with("B is worse"), "{}", r.overall);
    }

    #[test]
    fn identical_setups_are_within_noise() {
        let spec = SynthSpec {
            seconds: 120.0,
            random_spike_rate_hz: 0.1,
            ..SynthSpec::default()
        };
        let a = generate(&SynthSpec {
            seed: 11,
            ..spec.clone()
        });
        let b = generate(&SynthSpec { seed: 12, ..spec });
        let r = compare(&a, &b, &AnalyzeOpts::default());
        for m in &r.metrics {
            assert!(
                matches!(m.verdict, Verdict::WithinNoise | Verdict::Negligible),
                "{} was {:?} ({:?}%)",
                m.key,
                m.verdict,
                m.change_pct
            );
        }
        assert!(r.overall.starts_with("NO MEASURABLE"), "{}", r.overall);
    }

    #[test]
    fn short_captures_are_flagged_small_sample() {
        let a = generate(&SynthSpec {
            seconds: 10.0,
            seed: 1,
            ..SynthSpec::default()
        });
        let b = generate(&SynthSpec {
            seconds: 10.0,
            base_ms: 7.3,
            seed: 2,
            ..SynthSpec::default()
        });
        let r = compare(&a, &b, &AnalyzeOpts::default());
        assert!(r.warnings.iter().any(|w| w.contains("small sample")));
        // a ~5% change on 10 s of data must not be called
        assert_eq!(find(&r, "avg_fps").verdict, Verdict::Inconclusive);
        assert!(r.overall.starts_with("INCONCLUSIVE"));
    }

    #[test]
    fn verdict_rules() {
        assert_eq!(
            verdict_for(5.0, 100.0, 3.0, 7.0, true, false),
            Verdict::Better
        );
        assert_eq!(
            verdict_for(5.0, 100.0, 3.0, 7.0, false, false),
            Verdict::Worse
        );
        assert_eq!(
            verdict_for(5.0, 100.0, -3.0, 7.0, true, false),
            Verdict::WithinNoise
        );
        assert_eq!(
            verdict_for(1.0, 100.0, 0.5, 1.5, true, false),
            Verdict::Negligible
        );
        assert_eq!(
            verdict_for(20.0, 100.0, 0.0, 0.0, true, true),
            Verdict::Better
        );
    }
}
