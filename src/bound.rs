//! CPU-bound vs GPU-bound classification over time.
//!
//! With PresentMon 2.x columns each frame is classified by how much of the
//! frame the CPU and GPU were busy (CPUBusy / GPUBusy relative to FrameTime).
//! Without them, a merged sensor file's GPU utilization is used as a much
//! weaker fallback.

use crate::series::{Frame, Series};
use crate::stutter::Spike;
use serde::Serialize;

/// A unit counts as the limiter when it is busy for at least this share of the frame.
pub const BUSY_FRAC: f64 = 0.85;
const TIMELINE_BINS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BoundClass {
    Cpu,
    Gpu,
    /// Neither saturated: frame cap, vsync, waiting on something else.
    Other,
}

impl BoundClass {
    fn glyph(self) -> char {
        match self {
            BoundClass::Cpu => 'C',
            BoundClass::Gpu => 'G',
            BoundClass::Other => '.',
        }
    }
}

pub fn classify_frame(f: &Frame) -> Option<BoundClass> {
    let (c, g) = (f.cpu_busy?, f.gpu_busy?);
    let (cr, gr) = (c / f.ms, g / f.ms);
    Some(if gr >= BUSY_FRAC && gr >= cr {
        BoundClass::Gpu
    } else if cr >= BUSY_FRAC {
        BoundClass::Cpu
    } else {
        BoundClass::Other
    })
}

fn classify_util(util: f64) -> BoundClass {
    if util >= 95.0 {
        BoundClass::Gpu
    } else if util < 85.0 {
        BoundClass::Cpu
    } else {
        BoundClass::Other
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundReport {
    pub source: &'static str,
    pub cpu_pct: f64,
    pub gpu_pct: f64,
    pub other_pct: f64,
    /// 60 time bins, one glyph each: C = CPU-bound, G = GPU-bound, . = neither.
    pub timeline: String,
    pub spikes_cpu: usize,
    pub spikes_gpu: usize,
    pub spikes_other: usize,
    pub mean_gpu_wait_ms: Option<f64>,
}

/// `gpu_util[i]` is the merged sensor GPU utilization (%) for frame i, if any.
pub fn analyze(
    series: &Series,
    spikes: &[Spike],
    gpu_util: Option<&[Option<f64>]>,
) -> Option<BoundReport> {
    let has_cols = series
        .frames
        .iter()
        .any(|f| f.cpu_busy.is_some() && f.gpu_busy.is_some());
    let (source, classes): (&'static str, Vec<Option<BoundClass>>) = if has_cols {
        (
            "PresentMon CPUBusy/GPUBusy",
            series.frames.iter().map(classify_frame).collect(),
        )
    } else {
        let u = gpu_util?;
        (
            "sensor GPU utilization (weak: cannot tell CPU-bound from a frame cap)",
            u.iter().map(|x| x.map(classify_util)).collect(),
        )
    };
    let mut w = [0.0f64; 3];
    for (f, c) in series.frames.iter().zip(&classes) {
        if let Some(c) = c {
            w[*c as usize] += f.ms;
        }
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let dur = series.duration_s().max(1e-9);
    let mut votes = vec![[0.0f64; 3]; TIMELINE_BINS];
    for (f, c) in series.frames.iter().zip(&classes) {
        if let Some(c) = c {
            let b = ((f.t / dur * TIMELINE_BINS as f64) as usize).min(TIMELINE_BINS - 1);
            votes[b][*c as usize] += f.ms;
        }
    }
    let timeline = votes
        .iter()
        .map(|v| {
            if v.iter().sum::<f64>() == 0.0 {
                ' '
            } else {
                let k = (0..3).max_by(|&a, &b| v[a].total_cmp(&v[b])).unwrap_or(2);
                [BoundClass::Cpu, BoundClass::Gpu, BoundClass::Other][k].glyph()
            }
        })
        .collect();
    let mut sp = [0usize; 3];
    for s in spikes {
        if let Some(Some(c)) = classes.get(s.idx) {
            sp[*c as usize] += 1;
        }
    }
    let waits: Vec<f64> = series.frames.iter().filter_map(|f| f.gpu_wait).collect();
    Some(BoundReport {
        source,
        cpu_pct: 100.0 * w[0] / total,
        gpu_pct: 100.0 * w[1] / total,
        other_pct: 100.0 * w[2] / total,
        timeline,
        spikes_cpu: sp[0],
        spikes_gpu: sp[1],
        spikes_other: sp[2],
        mean_gpu_wait_ms: (!waits.is_empty())
            .then(|| waits.iter().sum::<f64>() / waits.len() as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{generate, BoundKind, SynthSpec};

    #[test]
    fn single_frame_rules() {
        let mut f = Frame::new(0.0, 10.0);
        f.cpu_busy = Some(9.5);
        f.gpu_busy = Some(5.0);
        assert_eq!(classify_frame(&f), Some(BoundClass::Cpu));
        f.cpu_busy = Some(4.0);
        f.gpu_busy = Some(9.8);
        assert_eq!(classify_frame(&f), Some(BoundClass::Gpu));
        f.cpu_busy = Some(4.0);
        f.gpu_busy = Some(5.0);
        assert_eq!(classify_frame(&f), Some(BoundClass::Other));
        f.gpu_busy = None;
        assert_eq!(classify_frame(&f), None);
    }

    #[test]
    fn cpu_bound_series() {
        let s = generate(&SynthSpec {
            bound: BoundKind::Cpu,
            ..SynthSpec::default()
        });
        let r = analyze(&s, &[], None).unwrap();
        assert!(r.cpu_pct > 90.0, "cpu {}", r.cpu_pct);
        assert!(r.timeline.chars().all(|c| c == 'C'));
    }

    #[test]
    fn gpu_bound_series() {
        let s = generate(&SynthSpec {
            bound: BoundKind::Gpu,
            ..SynthSpec::default()
        });
        let r = analyze(&s, &[], None).unwrap();
        assert!(r.gpu_pct > 90.0, "gpu {}", r.gpu_pct);
        assert!(r.mean_gpu_wait_ms.is_some());
    }

    #[test]
    fn no_columns_no_report_but_sensor_fallback_works() {
        let s = generate(&SynthSpec::default());
        assert!(analyze(&s, &[], None).is_none());
        let util: Vec<Option<f64>> = vec![Some(60.0); s.len()];
        let r = analyze(&s, &[], Some(&util)).unwrap();
        assert!(r.cpu_pct > 99.0);
        assert!(r.source.contains("weak"));
    }
}
