//! Turn reports into the document model.

use super::{fx, signed_pct, Block};
use crate::compare::CompareReport;
use crate::report::Report;
use crate::series::Series;

fn s(x: &str) -> String {
    x.to_string()
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
}

const DISCLAIMER: &str = "Findings are hints with evidence, not diagnoses. A pattern in the numbers is consistent with a cause; it does not prove one. Re-capture to confirm before changing settings.";

pub fn analysis_doc(r: &Report, series: &Series, target_hz: f64) -> Vec<Block> {
    let mut d = vec![Block::Title(format!("frametime: {}", r.source.file))];
    d.push(Block::Para(format!(
        "{} | app: {} | {} frames | {:.1} s | target {:.0} Hz",
        r.source.format_label,
        r.source.app.as_deref().unwrap_or("unknown"),
        r.source.frames,
        r.source.duration_s,
        target_hz
    )));
    if !r.source.notes.is_empty() {
        d.push(Block::Bullets(r.source.notes.clone()));
    }

    d.push(Block::Heading(s("What to look at (ranked)")));
    if r.findings.is_empty() {
        d.push(Block::Para(s(
            "Nothing stands out: no stutter pattern, bound problem or sensor correlation crossed a threshold.",
        )));
    }
    for (i, f) in r.findings.iter().take(7).enumerate() {
        d.push(Block::Finding {
            rank: i + 1,
            title: f.title.clone(),
            confidence: f.confidence,
            evidence: f.evidence.clone(),
            caveat: f.caveat.clone(),
        });
    }

    d.push(Block::Heading(s("Frame time over the run")));
    d.push(Block::Chart {
        ms: series.ms_vec(),
        budget_ms: r.budget.budget_ms,
    });
    d.push(Block::Heading(s("Frame-time distribution")));
    d.push(Block::Hist {
        ms: series.ms_vec(),
    });

    let st = &r.stats;
    d.push(Block::Heading(s("Core stats")));
    d.push(Block::Table {
        head: row(&["metric", "value"]),
        rows: vec![
            row(&["average fps", &fx(st.avg_fps, 1)]),
            row(&[
                "frame time mean / median",
                &format!("{} / {} ms", fx(st.mean_ms, 2), fx(st.p50_ms, 2)),
            ]),
            row(&[
                "p90 / p95",
                &format!("{} / {} ms", fx(st.p90_ms, 2), fx(st.p95_ms, 2)),
            ]),
            row(&[
                "p99 / p99.9",
                &format!("{} / {} ms", fx(st.p99_ms, 2), fx(st.p999_ms, 2)),
            ]),
            row(&[
                "min / max",
                &format!("{} / {} ms", fx(st.min_ms, 2), fx(st.max_ms, 2)),
            ]),
            row(&[
                "1% low, percentile method",
                &format!("{} fps", fx(st.low1_pct_fps, 1)),
            ]),
            row(&[
                "1% low, average of worst 1%",
                &format!("{} fps", fx(st.low1_avg_fps, 1)),
            ]),
            row(&[
                "0.1% low, percentile method",
                &format!("{} fps", fx(st.low01_pct_fps, 1)),
            ]),
            row(&[
                "0.1% low, average of worst 0.1%",
                &format!("{} fps", fx(st.low01_avg_fps, 1)),
            ]),
            row(&[
                "frame-time std dev / CV",
                &format!("{} ms / {}", fx(st.std_ms, 2), fx(r.stutter.cv, 2)),
            ]),
            row(&[
                "jitter (mean frame-to-frame change)",
                &format!("{} ms", fx(r.stutter.jitter_ms, 2)),
            ]),
        ],
    });
    d.push(Block::Para(s(
        "Percentile method: fps at the 99th (99.9th) percentile frame time. Average-of-worst-N: fps of the mean of the slowest 1% (0.1%) of frames. They differ; compare like with like.",
    )));

    let sr = &r.stutter;
    d.push(Block::Heading(s("Stutter")));
    d.push(Block::Bullets(vec![
        format!(
            "{} spike frames ({:.2}%) in {} events ({:.1}/min); spike = frame > {:.1}x rolling median ({} frames) and at least 2 ms over it",
            sr.spike_frames, sr.spike_frames_pct, sr.spike_events, sr.events_per_min, sr.mult, sr.window_frames
        ),
        format!(
            "hitches over {}: {}",
            sr.hitches.iter().map(|h| format!("{:.0} ms", h.over_ms)).collect::<Vec<_>>().join(" / "),
            sr.hitches.iter().map(|h| h.count.to_string()).collect::<Vec<_>>().join(" / ")
        ),
    ]));
    if !sr.worst.is_empty() {
        d.push(Block::Table {
            head: row(&[
                "worst spikes at (s)",
                "frame (ms)",
                "local median (ms)",
                "x median",
            ]),
            rows: sr
                .worst
                .iter()
                .map(|w| {
                    vec![
                        fx(w.t_s, 2),
                        fx(w.ms, 1),
                        fx(w.local_median_ms, 2),
                        fx(w.ratio, 1),
                    ]
                })
                .collect(),
        });
    }

    let b = &r.budget;
    d.push(Block::Heading(format!(
        "Frame budget at {:.0} Hz ({:.2} ms)",
        b.target_hz, b.budget_ms
    )));
    d.push(Block::Bullets(vec![
        format!(
            "{:.1}% of frames over budget (10% tolerance), covering {:.1}% of the time",
            b.over_budget_pct, b.time_over_budget_pct
        ),
        format!(
            "{:.1}% of frames took two refresh intervals or more",
            b.missed_refresh_pct
        ),
    ]));

    if let Some(bd) = &r.bound {
        d.push(Block::Heading(s("CPU- vs GPU-bound")));
        d.push(Block::Bullets(vec![
            format!(
                "CPU-bound {:.0}% | GPU-bound {:.0}% | neither {:.0}% (by time)",
                bd.cpu_pct, bd.gpu_pct, bd.other_pct
            ),
            format!("timeline [{}]  C=CPU G=GPU .=neither", bd.timeline),
            format!("source: {}", bd.source),
        ]));
    }
    if let Some(p) = &r.periodic {
        d.push(Block::Heading(s("Periodic stutter")));
        d.push(Block::Para(format!(
            "period {:.3} s, strength {:.2}, {} events, confidence {}",
            p.period_s,
            p.strength,
            p.events,
            p.confidence.label()
        )));
    }
    if let Some(p) = &r.present {
        d.push(Block::Heading(s("Presentation and latency")));
        let mut v: Vec<String> = p
            .modes
            .iter()
            .take(4)
            .map(|m| format!("{}: {:.1}%", m.mode, m.pct))
            .collect();
        if let Some(x) = p.dropped_pct {
            v.push(format!("dropped frames: {x:.2}%"));
        }
        if let Some(x) = p.tearing_pct {
            v.push(format!("allows tearing: {x:.0}% of frames"));
        }
        if let Some(l) = &p.latency {
            v.push(format!(
                "display latency avg {:.1} / p50 {:.1} / p95 {:.1} / p99 {:.1} ms",
                l.avg_ms, l.p50_ms, l.p95_ms, l.p99_ms
            ));
        }
        d.push(Block::Bullets(v));
    }
    if let Some(sn) = &r.sensors {
        d.push(Block::Heading(s("Sensors")));
        d.push(Block::Para(format!(
            "{:.0}% of frames matched a sensor sample (interval {:.1} s, offset {:+.1} s)",
            sn.coverage_pct, sn.sample_interval_s, sn.offset_s
        )));
        d.push(Block::Table {
            head: row(&["channel", "min", "avg", "max"]),
            rows: sn
                .channels
                .iter()
                .map(|c| vec![c.channel.clone(), fx(c.min, 1), fx(c.avg, 1), fx(c.max, 1)])
                .collect(),
        });
        if !sn.conditions.is_empty() {
            d.push(Block::Table {
                head: row(&[
                    "condition",
                    "spikes with it",
                    "of spikes",
                    "of whole run",
                    "enrichment",
                ]),
                rows: sn
                    .conditions
                    .iter()
                    .map(|c| {
                        vec![
                            c.name.clone(),
                            format!("{}/{}", c.spikes_with, c.spikes_total),
                            format!("{:.0}%", c.share_of_spikes_pct),
                            format!("{:.0}%", c.share_of_run_pct),
                            c.enrichment.map_or("n/a".into(), |e| format!("{e:.1}x")),
                        ]
                    })
                    .collect(),
            });
        }
        if !sn.notes.is_empty() {
            d.push(Block::Bullets(sn.notes.clone()));
        }
    }
    if r.phases.len() > 1 {
        d.push(Block::Heading(s("Phases")));
        d.push(Block::Table {
            head: row(&["phase", "frames", "avg fps", "1% low", "p99 ms", "spikes"]),
            rows: r
                .phases
                .iter()
                .map(|p| {
                    vec![
                        p.label.clone(),
                        p.frames.to_string(),
                        fx(p.avg_fps, 1),
                        fx(p.low1_pct_fps, 1),
                        fx(p.p99_ms, 2),
                        p.spikes.to_string(),
                    ]
                })
                .collect(),
        });
    }
    d.push(Block::Para(s(DISCLAIMER)));
    d
}

pub fn compare_doc(c: &CompareReport, a_name: &str, b_name: &str) -> Vec<Block> {
    let mut d = vec![Block::Title(format!(
        "frametime compare: {a_name} (A) vs {b_name} (B)"
    ))];
    d.push(Block::Verdict(c.overall.clone()));
    d.push(Block::Para(format!(
        "A: {} frames, {:.0} s, {} time blocks | B: {} frames, {:.0} s, {} time blocks | {} bootstrap resamples",
        c.a.stats.frames, c.a.stats.duration_s, c.blocks_a, c.b.stats.frames, c.b.stats.duration_s, c.blocks_b, c.resamples
    )));
    d.push(Block::Table {
        head: row(&["metric", "A", "B", "change", "95% CI of B-A", "verdict"]),
        rows: c
            .metrics
            .iter()
            .map(|m| {
                vec![
                    format!("{} ({})", m.name, m.unit),
                    fx(m.a, 2),
                    fx(m.b, 2),
                    signed_pct(m.change_pct),
                    format!("[{}, {}]", fx(m.diff_ci_low, 2), fx(m.diff_ci_high, 2)),
                    m.verdict.label().to_string(),
                ]
            })
            .collect(),
    });
    if !c.phases.is_empty() {
        d.push(Block::Heading(s("Per phase")));
        d.push(Block::Table {
            head: row(&[
                "phase",
                "avg fps A",
                "avg fps B",
                "change",
                "1% low A",
                "1% low B",
                "change",
                "spikes A/B",
            ]),
            rows: c
                .phases
                .iter()
                .map(|p| {
                    vec![
                        p.label.clone(),
                        fx(p.a_avg_fps, 1),
                        fx(p.b_avg_fps, 1),
                        signed_pct(Some(p.avg_fps_change_pct)),
                        fx(p.a_low1_fps, 1),
                        fx(p.b_low1_fps, 1),
                        signed_pct(Some(p.low1_change_pct)),
                        format!("{}/{}", p.a_spikes, p.b_spikes),
                    ]
                })
                .collect(),
        });
        d.push(Block::Para(s(
            "Phase rows have no significance test; treat changes under about 5% as noise.",
        )));
    }
    d.push(Block::Heading(s("Caveats")));
    d.push(Block::Bullets(c.warnings.clone()));
    d
}
