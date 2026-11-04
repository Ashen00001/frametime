//! RTSS / MSI Afterburner style logs (a named frametime column) and plain lists.

use super::table::{parse_num, Table};
use crate::series::{Format, Frame, Series};
use crate::util::median;
use anyhow::{bail, Result};

pub fn parse_named(text: &str) -> Result<Series> {
    let t = Table::read(text)?;
    let dc = t.delim == ';';
    let ft_col = t.header.iter().position(|h| h.contains("frametime"));
    let fps_col = t
        .header
        .iter()
        .position(|h| h.contains("framerate") || h == "fps");
    let time_col = t.idx_any(&["time", "timestamp", "timeinseconds", "elapsed", "timems"]);
    let time_is_ms = time_col.is_some_and(|i| t.header[i].ends_with("ms"));

    let (col, from_fps) = match (ft_col, fps_col) {
        (Some(c), _) => (c, false),
        (None, Some(c)) => (c, true),
        _ => bail!("no frametime column"),
    };
    let mut frames = Vec::new();
    let mut have_time = false;
    for rec in &t.rows {
        let Some(v) = rec.get(col).and_then(|s| parse_num(s, dc)) else {
            continue;
        };
        let ms = if from_fps {
            if v <= 0.0 {
                continue;
            }
            1000.0 / v
        } else {
            v
        };
        if ms <= 0.0 {
            continue;
        }
        let ts = time_col
            .and_then(|i| rec.get(i))
            .and_then(|s| parse_num(s, dc))
            .map(|x| if time_is_ms { x / 1000.0 } else { x });
        have_time |= ts.is_some();
        frames.push(Frame::new(ts.unwrap_or(0.0), ms));
    }
    let mut s = Series::new(Format::RtssLog, frames);
    if from_fps {
        s.notes.push(
            "only an FPS/framerate column was found; frame times were derived as 1000/fps \
             (coarse: sample-averaged, hides short spikes)"
                .into(),
        );
    }
    unit_guess(&mut s);
    s.normalize_time(have_time && time_col.is_some());
    Ok(s)
}

pub fn parse_plain(text: &str) -> Result<Series> {
    let mut ms = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Ok(v) = l.parse::<f64>() {
            if v.is_finite() && v > 0.0 {
                ms.push(v);
            }
        }
    }
    if ms.is_empty() {
        bail!("no numeric frame times");
    }
    let mut s = Series::from_ms(Format::Plain, &ms);
    unit_guess(&mut s);
    Ok(s)
}

/// A median under 0.2 ms is not a real frame time: treat the data as seconds.
fn unit_guess(s: &mut Series) {
    let ms: Vec<f64> = s.frames.iter().map(|f| f.ms).collect();
    if ms.is_empty() || median(&ms) >= 0.2 {
        return;
    }
    for f in &mut s.frames {
        f.ms *= 1000.0;
    }
    s.notes
        .push("values looked like seconds (median < 0.2); converted to ms".into());
    let mut t = 0.0;
    for f in &mut s.frames {
        f.t = t;
        t += f.ms / 1000.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_list_with_header_and_blank_lines() {
        let s = parse_plain("ms\n16.7\n\n16.9\n33.0\n").unwrap();
        assert_eq!(s.frames.len(), 3);
        assert_eq!(s.frames[2].ms, 33.0);
        assert!((s.frames[2].t - 0.0336).abs() < 1e-9);
    }

    #[test]
    fn plain_seconds_are_converted() {
        let s = parse_plain("0.0069\n0.0070\n0.0071\n").unwrap();
        assert!((s.frames[0].ms - 6.9).abs() < 1e-9);
        assert!(!s.notes.is_empty());
    }

    #[test]
    fn rtss_named_column_semicolon_decimal_comma() {
        let s = parse_named("Time;Frametime [ms]\n0,0;6,9\n0,007;7,1\n").unwrap();
        assert_eq!(s.frames.len(), 2);
        assert!((s.frames[0].ms - 6.9).abs() < 1e-9);
        assert!((s.frames[1].t - 0.007).abs() < 1e-9);
    }

    #[test]
    fn rtss_fps_only_column() {
        let s = parse_named("Framerate\n100\n50\n").unwrap();
        assert!((s.frames[0].ms - 10.0).abs() < 1e-9);
        assert!((s.frames[1].ms - 20.0).abs() < 1e-9);
        assert!(s.notes[0].contains("1000/fps"));
    }
}
