//! Ranked findings. Every finding carries its evidence, a confidence and a
//! caveat; none states a cause as fact.

use crate::report::Report;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }

    fn weight(self) -> f64 {
        match self {
            Confidence::High => 1.0,
            Confidence::Medium => 0.85,
            Confidence::Low => 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub title: String,
    /// Rough 0-100 impact score used for ranking.
    pub score: f64,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
    pub caveat: String,
}

fn f(
    title: String,
    score: f64,
    confidence: Confidence,
    evidence: Vec<String>,
    caveat: &str,
) -> Finding {
    Finding {
        title,
        score: score.clamp(0.0, 100.0),
        confidence,
        evidence,
        caveat: caveat.to_string(),
    }
}

pub fn build(r: &Report) -> Vec<Finding> {
    let mut out = Vec::new();
    let (st, b, sr) = (&r.stats, &r.budget, &r.stutter);

    if st.avg_fps < b.target_hz * 0.9 {
        out.push(f(
            format!(
                "Average {:.0} fps is {:.0}% below the {:.0} Hz target",
                st.avg_fps,
                100.0 * (1.0 - st.avg_fps / b.target_hz),
                b.target_hz
            ),
            0.8 * 100.0 * (1.0 - st.avg_fps / b.target_hz),
            Confidence::High,
            vec![
                format!("median frame {:.2} ms vs budget {:.2} ms", b.median_ms, b.budget_ms),
                format!(
                    "{:.1}% of frames over budget, {:.1}% of time spent in them, {:.1}% took 2+ refreshes",
                    b.over_budget_pct, b.time_over_budget_pct, b.missed_refresh_pct
                ),
            ],
            "Expected if you cap fps or run VRR below the panel's refresh on purpose; change --target-hz to your real goal.",
        ));
    }

    if st.low1_pct_fps < 0.6 * st.avg_fps {
        out.push(f(
            format!(
                "1% lows ({:.0} fps) are only {:.0}% of the average: uneven frame delivery",
                st.low1_pct_fps,
                100.0 * st.low1_pct_fps / st.avg_fps
            ),
            80.0 * (1.0 - st.low1_pct_fps / st.avg_fps),
            Confidence::High,
            vec![
                format!(
                    "1% low {:.0} fps (percentile method), {:.0} fps (average-of-worst-1%)",
                    st.low1_pct_fps, st.low1_avg_fps
                ),
                format!(
                    "0.1% low {:.0} fps (percentile), {:.0} fps (average-of-worst-0.1%)",
                    st.low01_pct_fps, st.low01_avg_fps
                ),
                format!(
                    "p50 {:.2} ms, p99 {:.2} ms, p99.9 {:.2} ms",
                    st.p50_ms, st.p99_ms, st.p999_ms
                ),
            ],
            "Lows rest on very few frames (1% of the capture); short captures make them noisy.",
        ));
    }

    let hitch100 = sr
        .hitches
        .iter()
        .find(|h| h.over_ms == 100.0)
        .map_or(0, |h| h.count);
    if sr.events_per_min >= 2.0 || hitch100 > 0 {
        let worst = sr.worst.first();
        out.push(f(
            format!(
                "{} stutter events ({} spike frames), worst {:.0} ms",
                sr.spike_events,
                sr.spike_frames,
                worst.map_or(0.0, |w| w.ms)
            ),
            (20.0 + sr.events_per_min * 2.0 + hitch100 as f64 * 5.0).min(85.0),
            Confidence::High,
            vec![
                format!(
                    "{:.1} events/min; spike = frame > {:.1}x rolling median (+2 ms)",
                    sr.events_per_min, sr.mult
                ),
                format!(
                    "hitches over 33/50/100/200 ms: {}",
                    sr.hitches
                        .iter()
                        .map(|h| h.count.to_string())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ),
            ],
            "Counts what happened, not why. See the other findings for possible patterns.",
        ));
    }

    if let Some(p) = &r.periodic {
        out.push(f(
            format!("Hitches repeat about every {:.2} s", p.period_s),
            75.0 * p.strength,
            p.confidence,
            vec![
                format!(
                    "{} events over {:.1} cycles; alignment strength {:.2} (1.00 = perfect)",
                    p.events, p.cycles, p.strength
                ),
                format!(
                    "{:.0}% of gaps are ~1 period, {:.0}% are 1-3 periods",
                    p.gaps_one_period_pct, p.gaps_matching_pct
                ),
            ],
            "A regular rhythm often points at something periodic (a timer, autosave, streaming, a background app or overlay poll) but this tool cannot tell which.",
        ));
    }

    if let Some(s) = &r.startup {
        out.push(f(
            format!("Stutter is concentrated in the first {:.0} s, settling around {:.0} s", s.head_s, s.settle_s),
            55.0,
            Confidence::Medium,
            vec![
                format!(
                    "{} of {} spikes ({:.0}%) in the first {:.0} s: {:.2}/s vs {:.2}/s afterwards",
                    s.spikes_head, s.spikes_total, s.share_pct, s.head_s, s.rate_head_per_s, s.rate_tail_per_s
                ),
                format!("p99 frame {:.1} ms early vs {:.1} ms later", s.head_p99_ms, s.tail_p99_ms),
            ],
            "Consistent with shader compilation or asset streaming, but also with a loading phase or something else starting up. A second run with a warm shader cache would help tell.",
        ));
    }

    if let Some(bd) = &r.bound {
        let weak = bd.source.contains("weak");
        let conf = if weak {
            Confidence::Low
        } else {
            Confidence::Medium
        };
        let slow = st.avg_fps < b.target_hz * 0.98;
        let mut ev = vec![
            format!(
                "time share: CPU-bound {:.0}%, GPU-bound {:.0}%, neither {:.0}%",
                bd.cpu_pct, bd.gpu_pct, bd.other_pct
            ),
            format!("source: {}", bd.source),
            format!("timeline (C=CPU G=GPU .=neither): [{}]", bd.timeline),
        ];
        if bd.spikes_cpu + bd.spikes_gpu + bd.spikes_other > 0 {
            ev.push(format!(
                "spikes while CPU-bound / GPU-bound / neither: {} / {} / {}",
                bd.spikes_cpu, bd.spikes_gpu, bd.spikes_other
            ));
        }
        if bd.cpu_pct >= 50.0 {
            out.push(f(
                format!("Mostly CPU-limited ({:.0}% of the run)", bd.cpu_pct),
                if slow { 60.0 } else { 15.0 },
                conf,
                ev,
                "Lowering graphics settings will not help much here; resolution/upscaling changes should barely move fps. A frame cap also looks like this if the source is utilization-based.",
            ));
        } else if bd.gpu_pct >= 50.0 {
            out.push(f(
                format!("Mostly GPU-limited ({:.0}% of the run)", bd.gpu_pct),
                if slow { 55.0 } else { 15.0 },
                conf,
                ev,
                "Resolution, upscaling and heavy graphics options should move fps here.",
            ));
        } else {
            out.push(f(
                format!(
                    "Neither CPU nor GPU is saturated for {:.0}% of the run",
                    bd.other_pct
                ),
                if slow { 35.0 } else { 10.0 },
                conf,
                ev,
                "Typical of a frame cap, vsync or waiting on something outside the CPU/GPU.",
            ));
        }
    }

    if let Some(p) = &r.present {
        if p.composed_pct >= 50.0 {
            out.push(f(
                format!("{:.0}% of frames use composed presentation", p.composed_pct),
                40.0,
                Confidence::Medium,
                vec![p
                    .modes
                    .iter()
                    .take(3)
                    .map(|m| format!("{}: {:.0}%", m.mode, m.pct))
                    .collect::<Vec<_>>()
                    .join("; ")],
                "Composed present paths go through the desktop compositor, which can add latency and pacing noise. Windowed mode and some overlays cause this legitimately.",
            ));
        }
        if p.mode_switches > 2 {
            out.push(f(
                format!("Present mode changed {} times during the run", p.mode_switches),
                30.0,
                Confidence::Medium,
                vec![format!("{} distinct modes seen", p.modes.len())],
                "Mode flips can come from overlays, notifications or alt-tab and often coincide with hitches.",
            ));
        }
        if let (Some(pct), Some(n)) = (p.dropped_pct, p.dropped_count) {
            if pct >= 1.0 {
                out.push(f(
                    format!("{pct:.1}% of frames were dropped (never displayed)"),
                    (pct * 8.0).min(70.0) + 20.0,
                    Confidence::High,
                    vec![format!("{n} dropped frames")],
                    "Common when fps far exceeds the refresh rate without vsync; harmless then.",
                ));
            }
        }
        if let Some(t) = p.tearing_pct {
            if t >= 50.0 {
                out.push(f(
                    format!("{t:.0}% of frames allow tearing"),
                    10.0,
                    Confidence::High,
                    vec!["AllowsTearing is set for most frames".into()],
                    "Normal with VRR or vsync off; informational.",
                ));
            }
        }
        if let Some(l) = &p.latency {
            if l.p50_in_frames > 3.5 || l.p99_ms > 3.0 * l.p50_ms {
                out.push(f(
                    format!("Display latency is high or spiky (median {:.1} ms)", l.p50_ms),
                    35.0,
                    Confidence::Low,
                    vec![
                        format!("median {:.1} ms = {:.1} frames; p95 {:.1} ms; p99 {:.1} ms; max {:.1} ms", l.p50_ms, l.p50_in_frames, l.p95_ms, l.p99_ms, l.max_ms),
                    ],
                    "A deep render queue, vsync or frame generation can all raise this; different columns mean different things between PresentMon versions.",
                ));
            }
        }
    }

    if let Some(s) = &r.sensors {
        for c in &s.conditions {
            let enr = c.enrichment.unwrap_or(f64::INFINITY);
            if c.spikes_with >= 3 && c.share_of_spikes_pct >= 50.0 && enr >= 1.5 {
                let conf = if c.spikes_total >= 10 && enr >= 2.0 {
                    Confidence::Medium
                } else {
                    Confidence::Low
                };
                out.push(f(
                    format!("Spikes coincide with: {}", c.name),
                    50.0 + 10.0 * enr.min(3.0),
                    conf,
                    vec![
                        format!(
                            "{} of {} spikes ({:.0}%) vs {:.0}% of the whole run",
                            c.spikes_with, c.spikes_total, c.share_of_spikes_pct, c.share_of_run_pct
                        ),
                    ],
                    "Correlation only, and sensors sample slowly. Clock drops can also be the GPU idling during the hitch rather than causing it.",
                ));
            } else if c.share_of_run_pct >= 20.0 {
                out.push(f(
                    format!("{} for {:.0}% of the run", c.name, c.share_of_run_pct),
                    40.0,
                    Confidence::Medium,
                    vec![format!(
                        "{:.0}% of spikes also fall in that state",
                        c.share_of_spikes_pct
                    )],
                    "A standing condition, not tied to individual spikes.",
                ));
            }
        }
    }

    if sr.cv > 0.25 && sr.spike_frames_pct < 1.0 {
        out.push(f(
            format!(
                "Frame pacing is uneven without distinct spikes (CV {:.2})",
                sr.cv
            ),
            30.0 + 40.0 * (sr.cv - 0.25),
            Confidence::Medium,
            vec![format!(
                "mean change between consecutive frames {:.2} ms; p99/p50 {:.2}",
                sr.jitter_ms, sr.p99_over_p50
            )],
            "Frame-rate limiters, vsync and varying scene load all produce broad spread.",
        ));
    }

    out.sort_by(|a, b| {
        (b.score * b.confidence.weight()).total_cmp(&(a.score * a.confidence.weight()))
    });
    out
}
