//! Optional sensor log (HWiNFO / MSI Afterburner CSV) merged onto the frames by time.
//!
//! Sensor time is measured from the first sensor row. Capture time `t` maps to
//! sensor time `t + offset` (`--sensor-offset`), because PresentMon captures
//! carry no wall clock to align on. Sensors sample slowly (0.5-2 s), so a spike
//! is matched against the sample in force at that moment and the one before it.

use crate::parse::{norm, parse_num};
use crate::series::Series;
use crate::stutter::Spike;
use crate::util::{mean, percentile_sorted, sorted};
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Channel {
    GpuClock,
    GpuTemp,
    GpuPower,
    GpuUtil,
    VramUsedMb,
    VramPct,
    RamUsedMb,
    RamPct,
    CpuClock,
    CpuTemp,
    CpuPower,
    CpuUtil,
    ThermalLimit,
    PowerLimit,
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Channel::GpuClock => "GPU clock (MHz)",
            Channel::GpuTemp => "GPU temp (C)",
            Channel::GpuPower => "GPU power (W)",
            Channel::GpuUtil => "GPU utilization (%)",
            Channel::VramUsedMb => "VRAM used (MB)",
            Channel::VramPct => "VRAM used (%)",
            Channel::RamUsedMb => "RAM used (MB)",
            Channel::RamPct => "RAM used (%)",
            Channel::CpuClock => "CPU clock (MHz)",
            Channel::CpuTemp => "CPU temp (C)",
            Channel::CpuPower => "CPU power (W)",
            Channel::CpuUtil => "CPU utilization (%)",
            Channel::ThermalLimit => "thermal limit flag",
            Channel::PowerLimit => "power limit flag",
        }
    }
}

/// Map a sensor column header to a channel (best effort, case-insensitive).
pub fn classify(header: &str) -> Option<Channel> {
    let h = header.to_lowercase();
    let has = |s: &str| h.contains(s);
    let pct = h.contains('%');
    if has("limit") || has("throttl") || has("perf cap") || has("perfcap") {
        return if has("therm") || has("temp") {
            Some(Channel::ThermalLimit)
        } else if has("power") || has("pwr") || has("tdp") {
            Some(Channel::PowerLimit)
        } else {
            None
        };
    }
    let gpu = has("gpu");
    let mem = has("mem") || has("vram") || has("dedicated");
    if gpu && mem {
        if has("clock") || has("controller") || has("temp") || has("power") {
            return None;
        }
        if has("alloc") || has("used") || has("usage") || has("dedicated") {
            return Some(if pct {
                Channel::VramPct
            } else {
                Channel::VramUsedMb
            });
        }
        return None;
    }
    if gpu {
        return if has("clock") {
            Some(Channel::GpuClock)
        } else if has("power") && !has("max") {
            Some(Channel::GpuPower)
        } else if has("temp") && !has("hot") {
            Some(Channel::GpuTemp)
        } else if (has("util") || has("usage") || has("load"))
            && !has("d3d")
            && !has("bus")
            && !has("fan")
        {
            Some(Channel::GpuUtil)
        } else {
            None
        };
    }
    if (has("physical memory") || has("ram") || has("memory usage"))
        && !has("virtual")
        && !has("page")
        && !has("avail")
        && !has("free")
    {
        return Some(if pct {
            Channel::RamPct
        } else {
            Channel::RamUsedMb
        });
    }
    let cpu = has("cpu") || has("core") || has("tctl") || has("tdie");
    if !cpu {
        return None;
    }
    if has("power") {
        Some(Channel::CpuPower)
    } else if has("clock") {
        Some(Channel::CpuClock)
    } else if has("temp") || has("tctl") || has("tdie") || has("\u{b0}c") {
        Some(Channel::CpuTemp)
    } else if has("usage") || has("util") || has("load") {
        Some(Channel::CpuUtil)
    } else {
        None
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sensors {
    pub t: Vec<f64>,
    pub data: BTreeMap<Channel, Vec<f64>>,
}

fn time_of_day(s: &str) -> Option<f64> {
    let tok = s.split([' ', 'T']).rfind(|p| p.contains(':'))?;
    let mut it = tok.split(':');
    let h: f64 = it.next()?.trim().parse().ok()?;
    let m: f64 = it.next()?.trim().parse().ok()?;
    let sec: f64 = it
        .next()
        .unwrap_or("0")
        .trim()
        .replace(',', ".")
        .parse()
        .ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn parse_val(s: &str, decimal_comma: bool) -> f64 {
    match s.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" => 1.0,
        "no" | "false" => 0.0,
        other => parse_num(other, decimal_comma).unwrap_or(f64::NAN),
    }
}

pub fn parse(text: &str) -> Result<Sensors> {
    let mut off = None;
    let mut pos = 0;
    let mut delim = ',';
    for line in text.split_inclusive('\n').take(80) {
        let l = line.trim();
        for d in [',', ';', '\t'] {
            let f: Vec<String> = l.split(d).map(norm).collect();
            if f.len() >= 3
                && f.iter().any(|x| {
                    x == "time" || x == "timestamp" || x == "datetime" || x.starts_with("elapsed")
                })
            {
                off = Some(pos);
                delim = d;
            }
        }
        if off.is_some() {
            break;
        }
        pos += line.len();
    }
    let Some(off) = off else {
        bail!("no sensor header with a Time/Timestamp column found");
    };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim as u8)
        .flexible(true)
        .from_reader(&text.as_bytes()[off..]);
    let headers: Vec<String> = rdr.headers()?.iter().map(str::to_string).collect();
    let nh: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let tcol = ["time", "timestamp", "datetime"]
        .iter()
        .find_map(|n| nh.iter().position(|h| h == n))
        .or_else(|| nh.iter().position(|h| h.starts_with("elapsed")));
    let Some(tcol) = tcol else {
        bail!("no time column");
    };
    let time_ms = nh[tcol].ends_with("ms");
    let dc = delim == ';';
    let mut cols: BTreeMap<Channel, usize> = BTreeMap::new();
    for (i, h) in headers.iter().enumerate() {
        if i != tcol {
            if let Some(c) = classify(h) {
                cols.entry(c).or_insert(i);
            }
        }
    }
    if cols.is_empty() {
        bail!("no recognizable sensor columns (GPU/CPU clock, temp, power, usage, memory...)");
    }
    let mut s = Sensors::default();
    let mut day = 0.0;
    let mut prev = f64::NEG_INFINITY;
    let mut first: Option<f64> = None;
    for rec in rdr.records().filter_map(|r| r.ok()) {
        let Some(raw) = rec.get(tcol) else { continue };
        let Some(mut t) = (if raw.contains(':') {
            time_of_day(raw)
        } else {
            parse_num(raw, dc).map(|x| if time_ms { x / 1000.0 } else { x })
        }) else {
            continue;
        };
        if raw.contains(':') {
            t += day;
            if t < prev - 1.0 {
                day += 86_400.0;
                t += 86_400.0;
            }
        }
        prev = t;
        let t0 = *first.get_or_insert(t);
        s.t.push(t - t0);
        for (c, &i) in &cols {
            s.data
                .entry(*c)
                .or_default()
                .push(rec.get(i).map(|v| parse_val(v, dc)).unwrap_or(f64::NAN));
        }
    }
    if s.t.len() < 2 {
        bail!("sensor file has fewer than 2 rows");
    }
    for c in [Channel::ThermalLimit, Channel::PowerLimit] {
        let flag_like = s.data.get(&c).is_some_and(|v| {
            v.iter()
                .filter(|x| x.is_finite())
                .all(|&x| x == 0.0 || x == 1.0)
        });
        if !flag_like {
            s.data.remove(&c);
        }
    }
    Ok(s)
}

impl Sensors {
    pub fn interval(&self) -> f64 {
        let d: Vec<f64> = self.t.windows(2).map(|w| w[1] - w[0]).collect();
        percentile_sorted(&sorted(&d), 50.0)
    }

    /// Index of the sample in force at sensor time `t` (sample-and-hold).
    pub fn index_at(&self, t: f64) -> Option<usize> {
        let j = self.t.partition_point(|&x| x <= t);
        if j == 0 {
            return None;
        }
        let last = *self.t.last()?;
        (t <= last + 2.0 * self.interval()).then_some(j - 1)
    }
}

pub fn frame_indices(series: &Series, s: &Sensors, offset: f64) -> Vec<Option<usize>> {
    series
        .frames
        .iter()
        .map(|f| s.index_at(f.t + offset))
        .collect()
}

pub fn gpu_util_per_frame(series: &Series, s: &Sensors, offset: f64) -> Option<Vec<Option<f64>>> {
    let u = s.data.get(&Channel::GpuUtil)?;
    Some(
        frame_indices(series, s, offset)
            .iter()
            .map(|i| i.map(|i| u[i]).filter(|x| x.is_finite()))
            .collect(),
    )
}

#[derive(Debug, Clone)]
pub struct SensorCfg {
    pub offset_s: f64,
    pub vram_gb: Option<f64>,
    pub ram_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSummary {
    pub channel: String,
    pub min: f64,
    pub avg: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Condition {
    pub name: String,
    pub spikes_with: usize,
    pub spikes_total: usize,
    pub share_of_spikes_pct: f64,
    pub share_of_run_pct: f64,
    /// share_of_spikes / share_of_run (None when the condition never occurs outside spikes).
    pub enrichment: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorReport {
    pub coverage_pct: f64,
    pub sample_interval_s: f64,
    pub offset_s: f64,
    pub channels: Vec<ChannelSummary>,
    pub conditions: Vec<Condition>,
    pub notes: Vec<String>,
}

fn states(s: &Sensors, cfg: &SensorCfg) -> Vec<(String, Vec<Option<bool>>)> {
    let map = |c: Channel, f: &dyn Fn(f64) -> bool| -> Option<Vec<Option<bool>>> {
        s.data.get(&c).map(|v| {
            v.iter()
                .map(|&x| x.is_finite().then(|| f(x)))
                .collect::<Vec<_>>()
        })
    };
    let drop_state = |c: Channel| {
        s.data.get(&c).map(|v| {
            let r = percentile_sorted(&sorted(v), 95.0);
            v.iter()
                .map(|&x| x.is_finite().then_some(x < 0.85 * r))
                .collect::<Vec<_>>()
        })
    };
    let mut out: Vec<(String, Vec<Option<bool>>)> = Vec::new();
    let mut add = |n: &str, v: Option<Vec<Option<bool>>>| {
        if let Some(v) = v {
            out.push((n.to_string(), v));
        }
    };
    add(
        "thermal-limit flag active",
        map(Channel::ThermalLimit, &|x| x > 0.5),
    );
    add(
        "power-limit flag active",
        map(Channel::PowerLimit, &|x| x > 0.5),
    );
    add(
        "GPU clock below 85% of its 95th percentile",
        drop_state(Channel::GpuClock),
    );
    add(
        "CPU clock below 85% of its 95th percentile",
        drop_state(Channel::CpuClock),
    );
    add("GPU temp >= 83 C", map(Channel::GpuTemp, &|x| x >= 83.0));
    add("CPU temp >= 90 C", map(Channel::CpuTemp, &|x| x >= 90.0));
    let vram = map(Channel::VramPct, &|x| x >= 92.0).or_else(|| {
        let gb = cfg.vram_gb?;
        map(Channel::VramUsedMb, &|x| x / (gb * 1024.0) >= 0.92)
    });
    add("VRAM >= 92% full", vram);
    let ram = map(Channel::RamPct, &|x| x >= 90.0).or_else(|| {
        let gb = cfg.ram_gb?;
        map(Channel::RamUsedMb, &|x| x / (gb * 1024.0) >= 0.90)
    });
    add("RAM >= 90% used", ram);
    out
}

pub fn analyze(series: &Series, spikes: &[Spike], s: &Sensors, cfg: &SensorCfg) -> SensorReport {
    let idx = frame_indices(series, s, cfg.offset_s);
    let covered = idx.iter().flatten().count();
    let mut notes = Vec::new();
    let dur = series.duration_s();
    let s_dur = s.t.last().copied().unwrap_or(0.0);
    if s_dur + cfg.offset_s < dur * 0.95 {
        notes.push(format!(
            "sensor log is shorter ({s_dur:.0} s) than the capture ({dur:.0} s) at this offset"
        ));
    }
    if s.interval() > 1.5 {
        notes.push(format!(
            "sensor interval is {:.1} s: short spikes can only be matched loosely",
            s.interval()
        ));
    }
    let channels = s
        .data
        .iter()
        .map(|(c, v)| {
            let f: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
            ChannelSummary {
                channel: c.label().to_string(),
                min: f.iter().copied().fold(f64::INFINITY, f64::min),
                avg: mean(&f),
                max: f.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            }
        })
        .collect();
    let at = |st: &[Option<bool>], i: usize| -> Option<bool> {
        let a = st[i];
        let b = i.checked_sub(1).and_then(|p| st[p]);
        match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(false) || b.unwrap_or(false)),
        }
    };
    let conditions = states(s, cfg)
        .into_iter()
        .map(|(name, st)| {
            let (mut run_n, mut run_t) = (0usize, 0usize);
            for i in idx.iter().flatten() {
                if let Some(b) = at(&st, *i) {
                    run_t += 1;
                    run_n += usize::from(b);
                }
            }
            let (mut sp_n, mut sp_t) = (0usize, 0usize);
            for sp in spikes {
                if let Some(Some(i)) = idx.get(sp.idx) {
                    if let Some(b) = at(&st, *i) {
                        sp_t += 1;
                        sp_n += usize::from(b);
                    }
                }
            }
            let sp_share = 100.0 * sp_n as f64 / sp_t.max(1) as f64;
            let run_share = 100.0 * run_n as f64 / run_t.max(1) as f64;
            Condition {
                name,
                spikes_with: sp_n,
                spikes_total: sp_t,
                share_of_spikes_pct: sp_share,
                share_of_run_pct: run_share,
                enrichment: (run_share > 0.0).then(|| sp_share / run_share),
            }
        })
        .collect();
    SensorReport {
        coverage_pct: 100.0 * covered as f64 / series.len().max(1) as f64,
        sample_interval_s: s.interval(),
        offset_s: cfg.offset_s,
        channels,
        conditions,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::Format;

    const HW: &str = "Date,Time,Core Clocks (avg) [MHz],GPU Clock [MHz],GPU Temperature [\u{b0}C],GPU Core Load [%],GPU Memory Usage [%],Physical Memory Used [MB],Performance Limit - Thermal [Yes/No]\n\
01.01.2026,10:00:00.000,4200,1900,70,90,50,12000,No\n\
01.01.2026,10:00:01.000,4200,1900,71,90,50,12000,No\n\
01.01.2026,10:00:02.000,4200,1100,84,90,95,12100,Yes\n\
01.01.2026,10:00:03.000,4200,1900,72,90,50,12100,No\n";

    #[test]
    fn classify_headers() {
        assert_eq!(classify("GPU Clock [MHz]"), Some(Channel::GpuClock));
        assert_eq!(classify("GPU Memory Clock [MHz]"), None);
        assert_eq!(classify("GPU Memory Usage [%]"), Some(Channel::VramPct));
        assert_eq!(
            classify("GPU Memory Allocated [MB]"),
            Some(Channel::VramUsedMb)
        );
        assert_eq!(classify("GPU1 temperature"), Some(Channel::GpuTemp));
        assert_eq!(classify("CPU Package Power [W]"), Some(Channel::CpuPower));
        assert_eq!(
            classify("Physical Memory Used [MB]"),
            Some(Channel::RamUsedMb)
        );
        assert_eq!(
            classify("Performance Limit - Thermal"),
            Some(Channel::ThermalLimit)
        );
        assert_eq!(classify("Fan speed"), None);
    }

    #[test]
    fn parses_hwinfo_style_and_aligns_by_time() {
        let s = parse(HW).unwrap();
        assert_eq!(s.t, vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(s.data[&Channel::ThermalLimit], vec![0.0, 0.0, 1.0, 0.0]);
        assert_eq!(s.index_at(0.5), Some(0));
        assert_eq!(s.index_at(2.0), Some(2));
        assert_eq!(s.index_at(2.9), Some(2));
        assert_eq!(s.index_at(100.0), None);
        assert_eq!(s.index_at(-1.0), None);
    }

    #[test]
    fn midnight_wrap_and_offset() {
        let csv =
            "Time,GPU Clock [MHz],GPU Temperature [C]\n23:59:59.0,1900,70\n00:00:01.0,1900,70\n";
        let s = parse(csv).unwrap();
        assert!((s.t[1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn spikes_coinciding_with_throttle_are_flagged() {
        let s = parse(HW).unwrap();
        // 4 s capture at 100 fps, one 60 ms hitch at t = 2.0 s
        let mut ms = vec![10.0; 400];
        ms[200] = 60.0;
        let series = Series::from_ms(Format::Plain, &ms);
        let spikes = crate::stutter::detect(&series, Default::default());
        assert_eq!(spikes.len(), 1);
        let cfg = SensorCfg {
            offset_s: 0.0,
            vram_gb: None,
            ram_gb: None,
        };
        let r = analyze(&series, &spikes, &s, &cfg);
        let thermal = r
            .conditions
            .iter()
            .find(|c| c.name.starts_with("thermal"))
            .unwrap();
        assert_eq!(thermal.spikes_with, 1);
        assert!(thermal.enrichment.unwrap() >= 1.9);
        let vram = r
            .conditions
            .iter()
            .find(|c| c.name.starts_with("VRAM"))
            .unwrap();
        assert_eq!(vram.spikes_with, 1);
        assert!(r.coverage_pct > 90.0);
    }

    #[test]
    fn rejects_non_sensor_file() {
        assert!(parse("a,b\n1,2\n").is_err());
    }
}
