//! Delimited-table helpers plus the PresentMon / CapFrameX CSV parser.

use super::ParseOpts;
use crate::series::{Format, Frame, Series};
use anyhow::{bail, Result};
use std::collections::HashMap;

/// Lowercase and strip everything but ASCII alphanumerics, so
/// `Frame Time [ms]` and `FrameTime` compare sensibly.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

pub fn parse_num(s: &str, decimal_comma: bool) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("na") || t.eq_ignore_ascii_case("n/a") {
        return None;
    }
    let v = if decimal_comma {
        t.replace(',', ".").parse::<f64>().ok()?
    } else {
        t.parse::<f64>().ok()?
    };
    v.is_finite().then_some(v)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn is_frame_header(fields: &[String]) -> bool {
    fields.iter().any(|f| {
        f.contains("msbetweenpresents")
            || f.contains("frametime")
            || f.contains("framerate")
            || f == "fps"
    })
}

/// Locate the header line within the first lines of the file (some tools put
/// a metadata block first). Returns byte offset, delimiter, normalized fields.
pub fn find_header(text: &str) -> Option<(usize, char, Vec<String>)> {
    let mut off = 0;
    for line in text.split_inclusive('\n').take(80) {
        let l = line.trim();
        if !l.is_empty() {
            let best = [',', ';', '\t']
                .into_iter()
                .map(|d| (d, l.split(d).map(norm).collect::<Vec<String>>()))
                .filter(|(_, f)| is_frame_header(f))
                .max_by_key(|(_, f)| f.len());
            if let Some((d, fields)) = best {
                return Some((off, d, fields));
            }
        }
        off += line.len();
    }
    None
}

pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<csv::StringRecord>,
    pub delim: char,
    pub prelude: String,
}

impl Table {
    pub fn read(text: &str) -> Result<Table> {
        let Some((off, delim, _)) = find_header(text) else {
            bail!("no frame-time header found");
        };
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delim as u8)
            .flexible(true)
            .from_reader(&text.as_bytes()[off..]);
        let header = rdr.headers()?.iter().map(norm).collect();
        let rows = rdr.records().filter_map(|r| r.ok()).collect();
        Ok(Table {
            header,
            rows,
            delim,
            prelude: text[..off].to_string(),
        })
    }

    pub fn idx(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }

    pub fn idx_any(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.idx(n))
    }

    pub fn num(&self, rec: &csv::StringRecord, col: Option<usize>) -> Option<f64> {
        col.and_then(|i| rec.get(i))
            .and_then(|s| parse_num(s, self.delim == ';'))
    }
}

pub fn parse_presentmon(text: &str, fmt: Format, opts: &ParseOpts) -> Result<Series> {
    let t = Table::read(text)?;
    let app = t.idx("application");
    let swap = t.idx("swapchainaddress");
    let ft = t.idx("frametime").or_else(|| t.idx("msbetweenpresents"));
    let Some(ft) = ft else {
        bail!("no FrameTime / MsBetweenPresents column");
    };
    let time_s = t.idx("timeinseconds");
    let time_ms = t.idx("cpustarttime");
    let cpu_busy = t.idx("cpubusy");
    let gpu_busy = t.idx("gpubusy");
    let gpu_wait = t.idx("gpuwait");
    let cpu_wait = t.idx("cpuwait");
    let disp_lat = t.idx("displaylatency");
    let until_disp = t.idx("msuntildisplayed");
    let dropped = t.idx("dropped");
    let tearing = t.idx("allowstearing");
    let mode = t.idx("presentmode");
    let ftype = t.idx("frametype");

    let mut modes: Vec<String> = Vec::new();
    let mut mode_ix: HashMap<String, u16> = HashMap::new();
    let mut groups: HashMap<(String, String), Vec<Frame>> = HashMap::new();
    let mut skipped_repeated = 0usize;
    let mut have_time = false;

    for rec in &t.rows {
        if let Some(i) = ftype {
            let v = rec.get(i).unwrap_or("").trim();
            if !v.is_empty() && !v.eq_ignore_ascii_case("application") {
                skipped_repeated += 1;
                continue;
            }
        }
        let Some(ms) = t.num(rec, Some(ft)) else {
            continue;
        };
        if ms <= 0.0 {
            continue;
        }
        let ts = t
            .num(rec, time_s)
            .or_else(|| t.num(rec, time_ms).map(|m| m / 1000.0));
        have_time |= ts.is_some();
        let mut f = Frame::new(ts.unwrap_or(0.0), ms);
        f.cpu_busy = t.num(rec, cpu_busy);
        f.gpu_busy = t.num(rec, gpu_busy);
        f.gpu_wait = t.num(rec, gpu_wait);
        f.cpu_wait = t.num(rec, cpu_wait);
        f.display_latency = t.num(rec, disp_lat);
        f.until_displayed = t.num(rec, until_disp);
        f.dropped = dropped.and_then(|i| rec.get(i)).and_then(parse_bool);
        f.tearing = tearing.and_then(|i| rec.get(i)).and_then(parse_bool);
        if let Some(m) = mode.and_then(|i| rec.get(i)).map(str::trim) {
            if !m.is_empty() && m != "Unknown" {
                let next = modes.len() as u16;
                let ix = *mode_ix.entry(m.to_string()).or_insert_with(|| {
                    modes.push(m.to_string());
                    next
                });
                f.mode = Some(ix);
            }
        }
        let key = (
            app.and_then(|i| rec.get(i))
                .unwrap_or("")
                .trim()
                .to_string(),
            swap.and_then(|i| rec.get(i))
                .unwrap_or("")
                .trim()
                .to_string(),
        );
        groups.entry(key).or_default().push(f);
    }

    if groups.is_empty() {
        bail!("no usable frame rows");
    }
    let mut notes = Vec::new();
    let mut keys: Vec<_> = groups.iter().map(|(k, v)| (k.clone(), v.len())).collect();
    keys.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let chosen = match &opts.process {
        Some(p) => {
            let p = p.to_ascii_lowercase();
            keys.iter()
                .find(|(k, _)| k.0.to_ascii_lowercase().contains(&p))
                .map(|(k, _)| k.clone())
        }
        None => keys.first().map(|(k, _)| k.clone()),
    };
    let Some(chosen) = chosen else {
        bail!("no process matching the --process filter");
    };
    if keys.len() > 1 {
        let others: Vec<String> = keys
            .iter()
            .filter(|(k, _)| *k != chosen)
            .take(4)
            .map(|(k, n)| {
                let name = if k.0.is_empty() { "(unnamed)" } else { &k.0 };
                format!("{name} ({n} frames)")
            })
            .collect();
        notes.push(format!(
            "capture holds {} process/swapchain groups; analyzing the largest ({}). Others: {}",
            keys.len(),
            if chosen.0.is_empty() {
                "(unnamed)"
            } else {
                &chosen.0
            },
            others.join(", ")
        ));
    }
    if skipped_repeated > 0 {
        notes.push(format!(
            "{skipped_repeated} non-Application rows (generated/repeated frames) skipped"
        ));
    }

    let frames = groups.remove(&chosen).unwrap_or_default();
    let mut s = Series::new(fmt, frames);
    s.app = (!chosen.0.is_empty())
        .then(|| chosen.0.clone())
        .or_else(|| {
            // CapFrameX CSV keeps the app name in a metadata block above the header.
            let mut lines = t.prelude.lines().filter(|l| !l.trim().is_empty());
            let first = lines.next()?;
            let second = lines.next()?;
            let d = t.delim;
            (norm(first.split(d).next()?) == "application")
                .then(|| second.split(d).next().map(|x| x.trim().to_string()))
                .flatten()
        });
    s.modes = modes;
    s.notes = notes;
    s.normalize_time(have_time);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PM1: &str = "Application,ProcessID,SwapChainAddress,Runtime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,Dropped,TimeInSeconds,MsBetweenPresents,MsBetweenDisplayChange,MsInPresentAPI,MsUntilRenderComplete,MsUntilDisplayed\n\
fake.exe,100,0xA,DXGI,0,0,0,Hardware: Independent Flip,0,0.000,10.0,10.0,0.1,2.0,12.0\n\
fake.exe,100,0xA,DXGI,0,0,1,Composed: Flip,1,0.010,12.0,12.0,0.1,2.0,15.0\n\
other.exe,200,0xB,DXGI,0,0,0,Composed: Flip,0,0.005,16.7,16.7,0.1,2.0,20.0\n";

    #[test]
    fn presentmon1_columns() {
        let s = parse_presentmon(PM1, Format::PresentMon1, &ParseOpts::default()).unwrap();
        assert_eq!(s.app.as_deref(), Some("fake.exe"));
        assert_eq!(s.frames.len(), 2);
        assert_eq!(s.frames[0].ms, 10.0);
        assert_eq!(s.frames[1].dropped, Some(true));
        assert_eq!(s.frames[1].tearing, Some(true));
        assert_eq!(s.frames[1].until_displayed, Some(15.0));
        assert_eq!(s.modes.len(), 2);
        assert!(s.notes.iter().any(|n| n.contains("2 process")));
    }

    #[test]
    fn presentmon1_process_filter() {
        let o = ParseOpts {
            process: Some("OTHER".into()),
            run: 0,
        };
        let s = parse_presentmon(PM1, Format::PresentMon1, &o).unwrap();
        assert_eq!(s.app.as_deref(), Some("other.exe"));
        assert_eq!(s.frames.len(), 1);
    }

    #[test]
    fn presentmon2_columns() {
        let csv = "Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,FrameType,CPUStartTime,FrameTime,CPUBusy,CPUWait,GPULatency,GPUTime,GPUBusy,GPUWait,DisplayLatency\n\
fake.exe,1,0xA,DXGI,0,0,0,Hardware: Independent Flip,Application,0.0,7.0,6.0,0.5,1.0,6.5,6.0,0.5,14.0\n\
fake.exe,1,0xA,DXGI,0,0,0,Hardware: Independent Flip,Application,7.0,8.0,7.5,0.1,1.0,7.0,6.5,1.0,15.0\n\
fake.exe,1,0xA,DXGI,0,0,0,Hardware: Independent Flip,Repeated,15.0,8.0,7.5,0.1,1.0,7.0,6.5,1.0,15.0\n";
        let s = parse_presentmon(csv, Format::PresentMon2, &ParseOpts::default()).unwrap();
        assert_eq!(s.frames.len(), 2);
        assert_eq!(s.frames[0].cpu_busy, Some(6.0));
        assert_eq!(s.frames[1].gpu_wait, Some(1.0));
        assert_eq!(s.frames[1].display_latency, Some(15.0));
        assert!((s.frames[1].t - 0.007).abs() < 1e-9);
        assert!(s.notes.iter().any(|n| n.contains("skipped")));
    }

    #[test]
    fn capframex_csv_with_metadata_block() {
        let csv = "Application,GPU,CPU\nfake.exe,GPU X,CPU Y\n\nTimeInSeconds,MsBetweenPresents,MsUntilDisplayed,Dropped\n0.00,7.0,9.0,0\n0.007,8.0,10.0,0\n";
        let s = parse_presentmon(csv, Format::CapFrameXCsv, &ParseOpts::default()).unwrap();
        assert_eq!(s.app.as_deref(), Some("fake.exe"));
        assert_eq!(s.frames.len(), 2);
        assert_eq!(s.frames[1].ms, 8.0);
    }
}
