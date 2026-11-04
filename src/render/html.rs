//! Self-contained HTML report (inline CSS and SVG, no scripts, no network).

use super::{downsample_max, histogram, Block};
use crate::findings::Confidence;
use crate::util::{percentile_sorted, sorted};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn chart_svg(ms: &[f64], budget: f64) -> String {
    let (w, h) = (900.0, 220.0);
    let pts = downsample_max(ms, 900);
    let top = (percentile_sorted(&sorted(ms), 99.9) * 1.2)
        .max(budget * 1.5)
        .max(1e-6);
    let y = |v: f64| h - (v / top).min(1.0) * (h - 10.0) - 5.0;
    let n = pts.len().max(2) - 1;
    let line: Vec<String> = pts
        .iter()
        .enumerate()
        .map(|(i, &v)| format!("{:.1},{:.1}", i as f64 / n as f64 * w, y(v)))
        .collect();
    format!(
        "<svg viewBox=\"0 0 {w} {h}\" role=\"img\" aria-label=\"frame time over the run\"><line x1=\"0\" x2=\"{w}\" y1=\"{by:.1}\" y2=\"{by:.1}\" class=\"budget\"/><polyline points=\"{}\" class=\"ft\"/><text x=\"4\" y=\"{ty:.1}\" class=\"lbl\">budget {budget:.2} ms; top of chart {top:.1} ms</text></svg>",
        line.join(" "),
        by = y(budget),
        ty = y(budget) - 4.0,
    )
}

fn hist_svg(ms: &[f64]) -> String {
    let bins = histogram(ms);
    let max = bins.iter().map(|b| b.count).max().unwrap_or(1).max(1) as f64;
    let bw = 900.0 / bins.len().max(1) as f64;
    let mut s = String::from(
        "<svg viewBox=\"0 0 900 180\" role=\"img\" aria-label=\"frame time histogram\">",
    );
    for (i, b) in bins.iter().enumerate() {
        let bh = b.count as f64 / max * 130.0;
        let label = if b.hi.is_finite() {
            format!("{:.1}", b.lo)
        } else {
            format!("{:.1}+", b.lo)
        };
        s.push_str(&format!(
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" class=\"bar\"><title>{label} ms: {} frames ({:.2}%)</title></rect><text x=\"{:.1}\" y=\"172\" class=\"lbl\">{label}</text>",
            i as f64 * bw + 2.0,
            150.0 - bh,
            bw - 4.0,
            bh,
            b.count,
            b.pct,
            i as f64 * bw + 2.0,
        ));
    }
    s.push_str("</svg>");
    s
}

const CSS: &str = ":root{--bg:#fff;--fg:#1b1f24;--mut:#59636e;--line:#d0d7de;--acc:#0969da;--bad:#cf222e;--ok:#1a7f37;--warn:#9a6700}\
@media(prefers-color-scheme:dark){:root{--bg:#0d1117;--fg:#e6edf3;--mut:#8b949e;--line:#30363d;--acc:#58a6ff;--bad:#ff7b72;--ok:#3fb950;--warn:#d29922}}\
body{background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif;max-width:960px;margin:0 auto;padding:16px}\
h1{font-size:1.5em}h2{font-size:1.15em;margin-top:1.6em;border-bottom:1px solid var(--line)}\
table{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}th,td{padding:4px 8px;border-bottom:1px solid var(--line);text-align:right}th:first-child,td:first-child{text-align:left}\
svg{width:100%;height:auto}.ft{fill:none;stroke:var(--acc);stroke-width:1}.budget{stroke:var(--warn);stroke-dasharray:4 4}.bar{fill:var(--acc)}.lbl{fill:var(--mut);font-size:10px}\
.verdict{font-size:1.2em;font-weight:600;padding:10px;border:1px solid var(--line);border-radius:6px}.note{color:var(--mut)}\
.high{color:var(--ok)}.medium{color:var(--warn)}.low{color:var(--bad)}.better{color:var(--ok)}.worse{color:var(--bad)}";

pub fn html(doc: &[Block]) -> String {
    let title = doc
        .iter()
        .find_map(|b| {
            if let Block::Title(t) = b {
                Some(t.as_str())
            } else {
                None
            }
        })
        .unwrap_or("frametime report");
    let mut o = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{CSS}</style></head><body>",
        esc(title)
    );
    for b in doc {
        match b {
            Block::Title(t) => o.push_str(&format!("<h1>{}</h1>", esc(t))),
            Block::Heading(h) => o.push_str(&format!("<h2>{}</h2>", esc(h))),
            Block::Para(p) => o.push_str(&format!("<p>{}</p>", esc(p))),
            Block::Verdict(v) => o.push_str(&format!("<p class=\"verdict\">{}</p>", esc(v))),
            Block::Bullets(v) => {
                o.push_str("<ul>");
                for i in v {
                    o.push_str(&format!("<li>{}</li>", esc(i)));
                }
                o.push_str("</ul>");
            }
            Block::Table { head, rows } => {
                o.push_str("<table><thead><tr>");
                for h in head {
                    o.push_str(&format!("<th>{}</th>", esc(h)));
                }
                o.push_str("</tr></thead><tbody>");
                for r in rows {
                    o.push_str("<tr>");
                    for c in r {
                        let class = match c.as_str() {
                            "better" => " class=\"better\"",
                            "worse" => " class=\"worse\"",
                            _ => "",
                        };
                        o.push_str(&format!("<td{class}>{}</td>", esc(c)));
                    }
                    o.push_str("</tr>");
                }
                o.push_str("</tbody></table>");
            }
            Block::Finding {
                rank,
                title,
                confidence,
                evidence,
                caveat,
            } => {
                let (cls, label) = match confidence {
                    Confidence::High => ("high", "high"),
                    Confidence::Medium => ("medium", "medium"),
                    Confidence::Low => ("low", "low"),
                };
                o.push_str(&format!(
                    "<h3>{rank}. {} <span class=\"{cls}\">[{label} confidence]</span></h3><ul>",
                    esc(title)
                ));
                for e in evidence {
                    o.push_str(&format!("<li>{}</li>", esc(e)));
                }
                o.push_str(&format!("</ul><p class=\"note\">{}</p>", esc(caveat)));
            }
            Block::Chart { ms, budget_ms } => o.push_str(&chart_svg(ms, *budget_ms)),
            Block::Hist { ms } => o.push_str(&hist_svg(ms)),
        }
    }
    o.push_str("</body></html>");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_is_self_contained() {
        let h = html(&[
            Block::Title("<x>".into()),
            Block::Chart {
                ms: vec![7.0; 100],
                budget_ms: 6.94,
            },
        ]);
        assert!(h.contains("&lt;x&gt;"));
        assert!(!h.contains("<script"));
        assert!(!h.contains("http"));
        assert!(h.contains("<polyline"));
    }
}
