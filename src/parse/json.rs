//! CapFrameX JSON captures.

use super::ParseOpts;
use crate::series::{Format, Frame, Series};
use anyhow::{bail, Context, Result};
use serde_json::Value;

fn nums(v: Option<&Value>) -> Vec<Option<f64>> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn bools(v: Option<&Value>) -> Vec<Option<bool>> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| x.as_bool().or_else(|| x.as_f64().map(|n| n != 0.0)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse(text: &str, opts: &ParseOpts) -> Result<Series> {
    let root: Value = serde_json::from_str(text).context("invalid JSON")?;
    let runs = root
        .get("Runs")
        .and_then(Value::as_array)
        .context("no Runs array")?;
    let Some(run) = runs.get(opts.run) else {
        bail!(
            "run {} requested but file has {} run(s)",
            opts.run,
            runs.len()
        );
    };
    let cap = run.get("CaptureData").context("run has no CaptureData")?;
    let mbp = nums(cap.get("MsBetweenPresents"));
    if mbp.is_empty() {
        bail!("CaptureData has no MsBetweenPresents");
    }
    let times = nums(cap.get("TimeInSeconds"));
    let until_disp = nums(cap.get("MsUntilDisplayed"));
    let dropped = bools(cap.get("Dropped"));
    let tearing = bools(cap.get("AllowsTearing"));

    let mut frames = Vec::with_capacity(mbp.len());
    for (i, ms) in mbp.iter().enumerate() {
        let Some(ms) = ms.filter(|m| *m > 0.0) else {
            continue;
        };
        let mut f = Frame::new(times.get(i).copied().flatten().unwrap_or(0.0), ms);
        f.until_displayed = until_disp.get(i).copied().flatten();
        f.dropped = dropped.get(i).copied().flatten();
        f.tearing = tearing.get(i).copied().flatten();
        frames.push(f);
    }
    let have_time = !times.is_empty();
    let mut s = Series::new(Format::CapFrameXJson, frames);
    let info = root.get("Info");
    s.app = ["ProcessName", "GameName"].iter().find_map(|k| {
        info.and_then(|i| i.get(*k))
            .and_then(Value::as_str)
            .filter(|x| !x.is_empty())
            .map(String::from)
    });
    if runs.len() > 1 {
        s.notes.push(format!(
            "file holds {} runs; using run {} (use --run to pick another)",
            runs.len(),
            opts.run
        ));
    }
    s.normalize_time(have_time);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capframex_json_run() {
        let j = r#"{"Info":{"ProcessName":"fake.exe"},"Runs":[{"CaptureData":{
            "TimeInSeconds":[0.0,0.007,0.015],
            "MsBetweenPresents":[7.0,8.0,9.0],
            "MsUntilDisplayed":[9.0,10.0,11.0],
            "Dropped":[false,true,false]}},{"CaptureData":{"MsBetweenPresents":[1.0,2.0]}}]}"#;
        let s = parse(j, &ParseOpts::default()).unwrap();
        assert_eq!(s.app.as_deref(), Some("fake.exe"));
        assert_eq!(s.frames.len(), 3);
        assert_eq!(s.frames[1].dropped, Some(true));
        assert_eq!(s.frames[2].until_displayed, Some(11.0));
        assert!(s.notes[0].contains("2 runs"));
        let o = ParseOpts {
            process: None,
            run: 1,
        };
        assert_eq!(parse(j, &o).unwrap().frames.len(), 2);
        let bad = ParseOpts {
            process: None,
            run: 5,
        };
        assert!(parse(j, &bad).is_err());
    }
}
