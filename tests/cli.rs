use frametime::parse::{parse_str, ParseOpts};
use frametime::report::{analyze, AnalyzeOpts};
use frametime::synth::{generate, to_presentmon2_csv, BoundKind, SynthSpec};
use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("frametime-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_frametime"))
}

#[test]
fn csv_roundtrip_keeps_the_analysis() {
    let s = generate(&SynthSpec {
        seconds: 120.0,
        spike_every_s: Some(6.0),
        bound: BoundKind::Cpu,
        ..SynthSpec::default()
    });
    let parsed = parse_str(&to_presentmon2_csv(&s), &ParseOpts::default()).unwrap();
    assert_eq!(parsed.len(), s.len());
    let r = analyze(&parsed, None, &AnalyzeOpts::default());
    assert!((r.periodic.unwrap().period_s - 6.0).abs() < 0.06);
    assert!(r.bound.unwrap().cpu_pct > 90.0);
}

#[test]
fn cli_analyze_json_and_html_and_read_only() {
    let csv = tmp("cap.csv");
    let html = tmp("report.html");
    let ok = bin()
        .args(["synth", "periodic", "--seconds", "90", "--out"])
        .arg(&csv)
        .status()
        .unwrap();
    assert!(ok.success());
    let before = std::fs::read(&csv).unwrap();

    let out = bin()
        .arg("analyze")
        .arg(&csv)
        .arg("--json")
        .arg("--html")
        .arg(&html)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let period = v["periodic"]["period_s"].as_f64().unwrap();
    assert!((period - 5.0).abs() < 0.1);
    assert!(std::fs::read_to_string(&html).unwrap().contains("<svg"));
    assert_eq!(
        before,
        std::fs::read(&csv).unwrap(),
        "capture must stay untouched"
    );

    // never overwrite the input with a report
    let bad = bin()
        .arg("analyze")
        .arg(&csv)
        .arg("--html")
        .arg(&csv)
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert_eq!(before, std::fs::read(&csv).unwrap());
}

#[test]
fn cli_compare_and_no_color() {
    let (a, b) = (tmp("a.csv"), tmp("b.csv"));
    for (p, seed) in [(&a, "1"), (&b, "2")] {
        assert!(bin()
            .args(["synth", "clean", "--seconds", "60", "--seed", seed, "--out"])
            .arg(p)
            .status()
            .unwrap()
            .success());
    }
    let out = bin()
        .arg("compare")
        .arg(&a)
        .arg(&b)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("NO MEASURABLE DIFFERENCE"), "{text}");
    assert!(!text.contains('\x1b'));
}

#[test]
fn cli_rejects_unknown_format() {
    let p = tmp("junk.txt");
    std::fs::write(&p, "hello\nworld\n").unwrap();
    let out = bin().arg("analyze").arg(&p).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unrecognized"));
}
