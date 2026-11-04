//! Synthetic captures. Used by the test-suite and by `frametime synth`
//! to make demo files for the README (no real game data anywhere).

use crate::series::{Format, Frame, Series};
use crate::util::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundKind {
    /// No CPU/GPU busy columns.
    None,
    Cpu,
    Gpu,
}

#[derive(Debug, Clone)]
pub struct SynthSpec {
    pub seconds: f64,
    pub base_ms: f64,
    pub noise_ms: f64,
    /// Inject a spike every this many seconds.
    pub spike_every_s: Option<f64>,
    pub spike_ms: f64,
    /// Heavy random stutter during the first N seconds.
    pub startup_stutter_s: Option<f64>,
    /// Random (non-periodic) spikes per second.
    pub random_spike_rate_hz: f64,
    pub bound: BoundKind,
    pub seed: u64,
}

impl Default for SynthSpec {
    fn default() -> Self {
        SynthSpec {
            seconds: 60.0,
            base_ms: 1000.0 / 144.0,
            noise_ms: 0.15,
            spike_every_s: None,
            spike_ms: 45.0,
            startup_stutter_s: None,
            random_spike_rate_hz: 0.0,
            bound: BoundKind::None,
            seed: 1,
        }
    }
}

pub fn generate(spec: &SynthSpec) -> Series {
    let mut rng = Rng::new(spec.seed);
    let mut t = 0.0;
    let mut next_spike = spec.spike_every_s.unwrap_or(f64::INFINITY);
    let mut frames = Vec::new();
    while t < spec.seconds {
        let mut ms = (spec.base_ms + spec.noise_ms * rng.gauss()).max(spec.base_ms * 0.5);
        if let Some(p) = spec.spike_every_s {
            if t >= next_spike {
                ms += spec.spike_ms;
                next_spike += p;
            }
        }
        if let Some(s) = spec.startup_stutter_s {
            if t < s && rng.next_f64() < 0.2 {
                ms += 30.0 + rng.next_f64() * 90.0;
            }
        }
        if spec.random_spike_rate_hz > 0.0
            && rng.next_f64() < spec.random_spike_rate_hz * ms / 1000.0
        {
            ms += spec.spike_ms;
        }
        let mut f = Frame::new(t, ms);
        match spec.bound {
            BoundKind::None => {}
            BoundKind::Cpu => {
                f.cpu_busy = Some(ms * 0.96);
                f.gpu_busy = Some(ms * 0.55);
                f.gpu_wait = Some(ms * 0.02);
                f.cpu_wait = Some(ms * 0.01);
            }
            BoundKind::Gpu => {
                f.cpu_busy = Some(ms * 0.5);
                f.gpu_busy = Some(ms * 0.97);
                f.gpu_wait = Some(ms * 0.4);
                f.cpu_wait = Some(ms * 0.3);
            }
        }
        frames.push(f);
        t += ms / 1000.0;
    }
    let mut s = Series::new(Format::Synthetic, frames);
    s.app = Some("SyntheticGame.exe".into());
    s
}

/// Serialize as a PresentMon 2.x style CSV (the columns the analyzer reads).
pub fn to_presentmon2_csv(series: &Series) -> String {
    let app = series.app.as_deref().unwrap_or("SyntheticGame.exe");
    let mut out = String::from(
        "Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,FrameType,CPUStartTime,FrameTime,CPUBusy,CPUWait,GPUBusy,GPUWait,DisplayLatency\n",
    );
    let num = |v: Option<f64>| v.map(|x| format!("{x:.4}")).unwrap_or_default();
    for f in &series.frames {
        out.push_str(&format!(
            "{app},1000,0x1,DXGI,0,0,0,Hardware: Independent Flip,Application,{:.4},{:.4},{},{},{},{},{}\n",
            f.t * 1000.0,
            f.ms,
            num(f.cpu_busy),
            num(f.cpu_wait),
            num(f.gpu_busy),
            num(f.gpu_wait),
            num(f.latency()),
        ));
    }
    out
}
