//! The in-memory frame series every parser produces and every analysis reads.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Format {
    PresentMon1,
    PresentMon2,
    CapFrameXCsv,
    CapFrameXJson,
    RtssLog,
    Plain,
    Synthetic,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::PresentMon1 => "PresentMon 1.x CSV",
            Format::PresentMon2 => "PresentMon 2.x CSV",
            Format::CapFrameXCsv => "CapFrameX CSV",
            Format::CapFrameXJson => "CapFrameX JSON",
            Format::RtssLog => "RTSS/Afterburner frametime log",
            Format::Plain => "plain frame-time list (ms)",
            Format::Synthetic => "synthetic",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// Seconds since the start of the capture.
    pub t: f64,
    /// Frame time in milliseconds.
    pub ms: f64,
    pub cpu_busy: Option<f64>,
    pub gpu_busy: Option<f64>,
    pub gpu_wait: Option<f64>,
    pub cpu_wait: Option<f64>,
    pub display_latency: Option<f64>,
    pub until_displayed: Option<f64>,
    pub dropped: Option<bool>,
    pub tearing: Option<bool>,
    /// Index into `Series::modes`.
    pub mode: Option<u16>,
}

impl Frame {
    pub fn new(t: f64, ms: f64) -> Self {
        Frame {
            t,
            ms,
            ..Frame::default()
        }
    }

    /// Display latency if the capture has it (2.x), else the 1.x equivalent.
    pub fn latency(&self) -> Option<f64> {
        self.display_latency.or(self.until_displayed)
    }
}

#[derive(Debug, Clone)]
pub struct Series {
    pub format: Format,
    pub app: Option<String>,
    pub frames: Vec<Frame>,
    pub modes: Vec<String>,
    pub notes: Vec<String>,
    pub source_name: String,
}

impl Series {
    pub fn new(format: Format, frames: Vec<Frame>) -> Self {
        Series {
            format,
            app: None,
            frames,
            modes: Vec::new(),
            notes: Vec::new(),
            source_name: String::new(),
        }
    }

    /// Build a series from bare frame times, with cumulative timestamps.
    pub fn from_ms(format: Format, ms: &[f64]) -> Self {
        let mut t = 0.0;
        let frames = ms
            .iter()
            .map(|&m| {
                let f = Frame::new(t, m);
                t += m / 1000.0;
                f
            })
            .collect();
        Series::new(format, frames)
    }

    pub fn ms_vec(&self) -> Vec<f64> {
        self.frames.iter().map(|f| f.ms).collect()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Wall-clock length of the capture: the sum of frame times.
    pub fn duration_s(&self) -> f64 {
        self.frames.iter().map(|f| f.ms).sum::<f64>() / 1000.0
    }

    pub fn has(&self, pick: impl Fn(&Frame) -> bool) -> bool {
        self.frames.iter().any(pick)
    }

    /// Drop the first `secs` seconds (loading screens, alt-tab) and re-zero time.
    pub fn trim_start(&mut self, secs: f64) {
        if secs <= 0.0 {
            return;
        }
        let keep = self.frames.partition_point(|f| f.t < secs);
        if keep >= self.frames.len() {
            return;
        }
        self.frames.drain(..keep);
        let t0 = self.frames.first().map(|f| f.t).unwrap_or(0.0);
        for f in &mut self.frames {
            f.t -= t0;
        }
        self.notes.push(format!("first {secs:.1} s trimmed"));
    }

    /// Fix up timestamps: shift to zero, and rebuild from frame times when
    /// the source timestamps are missing or not monotonic.
    pub fn normalize_time(&mut self, have_time: bool) {
        let monotonic = have_time && self.frames.windows(2).all(|w| w[1].t >= w[0].t);
        if monotonic && !self.frames.is_empty() {
            let t0 = self.frames[0].t;
            for f in &mut self.frames {
                f.t -= t0;
            }
        } else {
            if have_time {
                self.notes
                    .push("timestamps were not monotonic; rebuilt from frame times".into());
            }
            let mut t = 0.0;
            for f in &mut self.frames {
                f.t = t;
                t += f.ms / 1000.0;
            }
        }
    }
}
