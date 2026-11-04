//! Format detection and parsers for frame-time captures.
//!
//! Supported: PresentMon CSV (1.x and 2.x), CapFrameX CSV and JSON,
//! RTSS/MSI Afterburner frame-time logs, and a plain one-column ms file.

mod json;
mod rtss;
mod table;

use crate::series::{Format, Series};
use anyhow::{bail, Context, Result};
use std::path::Path;

pub use table::{find_header, norm, parse_num, Table};

#[derive(Debug, Clone, Default)]
pub struct ParseOpts {
    /// Keep only this process (case-insensitive substring) in multi-process captures.
    pub process: Option<String>,
    /// Which run to read from a CapFrameX JSON with several runs (0-based).
    pub run: usize,
}

pub fn load(path: &Path, opts: &ParseOpts) -> Result<Series> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut s = parse_str(text, opts).with_context(|| format!("parsing {}", path.display()))?;
    s.source_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(s)
}

pub fn parse_str(text: &str, opts: &ParseOpts) -> Result<Series> {
    let series = match detect(text) {
        Some(Format::CapFrameXJson) => json::parse(text, opts)?,
        Some(Format::PresentMon1) => table::parse_presentmon(text, Format::PresentMon1, opts)?,
        Some(Format::PresentMon2) => table::parse_presentmon(text, Format::PresentMon2, opts)?,
        Some(Format::CapFrameXCsv) => table::parse_presentmon(text, Format::CapFrameXCsv, opts)?,
        Some(Format::RtssLog) => rtss::parse_named(text)?,
        Some(Format::Plain) => rtss::parse_plain(text)?,
        _ => bail!(
            "unrecognized capture format. Supported: PresentMon CSV (1.x/2.x), CapFrameX CSV/JSON, \
             RTSS/Afterburner frametime log, or a plain one-column file of frame times in ms"
        ),
    };
    if series.frames.len() < 2 {
        bail!("capture has fewer than 2 usable frames");
    }
    Ok(series)
}

/// Guess the format from the content (header line), not the file extension.
pub fn detect(text: &str) -> Option<Format> {
    let t = text.trim_start();
    if t.starts_with('{') {
        return t.contains("\"Runs\"").then_some(Format::CapFrameXJson);
    }
    if let Some((_, _, fields)) = find_header(text) {
        let has = |n: &str| fields.iter().any(|f| f == n);
        let pm_cols = has("processid") || has("swapchainaddress") || has("application");
        if pm_cols && (has("msbetweenpresents") || has("frametime")) {
            let v2 = has("frametime")
                || has("cpubusy")
                || has("gpubusy")
                || has("cpustarttime")
                || has("cpustartqpc")
                || has("displaylatency");
            return Some(if v2 {
                Format::PresentMon2
            } else {
                Format::PresentMon1
            });
        }
        if has("msbetweenpresents") || (has("timeinseconds") && has("msuntildisplayed")) {
            return Some(Format::CapFrameXCsv);
        }
        return Some(Format::RtssLog);
    }
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    first
        .parse::<f64>()
        .ok()
        .map(|_| Format::Plain)
        .or_else(|| {
            // header-less-but-labelled single column, e.g. "ms"
            let second = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .nth(1)?;
            second.parse::<f64>().ok().map(|_| Format::Plain)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_format() {
        assert_eq!(detect("1\n2\n"), Some(Format::Plain));
        assert_eq!(detect("{\"Runs\":[]}"), Some(Format::CapFrameXJson));
        assert_eq!(
            detect("Application,ProcessID,MsBetweenPresents\nfake.exe,1,6.9\n"),
            Some(Format::PresentMon1)
        );
        assert_eq!(
            detect("Application,ProcessID,FrameTime,CPUBusy\nfake.exe,1,6.9,3\n"),
            Some(Format::PresentMon2)
        );
        assert_eq!(
            detect("TimeInSeconds,MsBetweenPresents,MsUntilDisplayed,Dropped\n0.0,6.9,8,0\n"),
            Some(Format::CapFrameXCsv)
        );
        assert_eq!(detect("Time;Frametime\n0;6.9\n"), Some(Format::RtssLog));
        assert_eq!(detect("hello world\n"), None);
    }
}
