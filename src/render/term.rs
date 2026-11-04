//! Terminal rendering: aligned tables, optional ANSI colour, sparkline, histogram.

use super::{hist_lines, sparkline, Block};
use crate::findings::Confidence;
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    /// Colour only on a TTY and when `NO_COLOR` is unset (https://no-color.org).
    pub fn auto() -> Style {
        Style {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    pub fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn cell(&self, s: &str) -> String {
        match s {
            "better" => self.paint("32", s),
            "worse" => self.paint("31", s),
            "inconclusive" | "within noise" | "negligible (<2%)" => self.paint("33", s),
            _ => s.to_string(),
        }
    }
}

fn numeric(c: &str) -> bool {
    c.starts_with(|ch: char| ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '[')
        || c == "n/a"
}

fn table(head: &[String], rows: &[Vec<String>], st: &Style) -> String {
    let cols = head.len();
    let mut w: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate().take(cols) {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let right: Vec<bool> = (0..cols)
        .map(|i| {
            i > 0 && !rows.is_empty() && rows.iter().all(|r| r.get(i).is_some_and(|c| numeric(c)))
        })
        .collect();
    let pad = |s: &str, i: usize| {
        let gap = " ".repeat(w[i].saturating_sub(s.chars().count()));
        if right[i] {
            format!("{gap}{s}")
        } else {
            format!("{s}{gap}")
        }
    };
    let mut out = String::new();
    let h: Vec<String> = head
        .iter()
        .enumerate()
        .map(|(i, c)| st.paint("1", &pad(c, i)))
        .collect();
    out.push_str(&format!("  {}\n", h.join("  ")));
    let total: usize = w.iter().sum::<usize>() + 2 * (cols - 1);
    out.push_str(&format!("  {}\n", st.paint("2", &"─".repeat(total))));
    for r in rows {
        let cells: Vec<String> = (0..cols)
            .map(|i| {
                let raw = r.get(i).map(String::as_str).unwrap_or("");
                let padded = pad(raw, i);
                let trimmed = raw.trim();
                if st.color && trimmed != padded.trim() {
                    padded
                } else {
                    padded.replacen(trimmed, &st.cell(trimmed), 1)
                }
            })
            .collect();
        out.push_str(&format!("  {}\n", cells.join("  ").trim_end()));
    }
    out
}

pub fn terminal(doc: &[Block], st: &Style, width: usize) -> String {
    let mut o = String::new();
    for b in doc {
        match b {
            Block::Title(t) => o.push_str(&format!("{}\n", st.paint("1;36", t))),
            Block::Heading(h) => o.push_str(&format!("\n{}\n", st.paint("1", h))),
            Block::Para(p) => o.push_str(&format!("{p}\n")),
            Block::Verdict(v) => o.push_str(&format!("\n{}\n\n", st.paint("1;33", v))),
            Block::Bullets(v) => {
                for i in v {
                    o.push_str(&format!("  - {i}\n"));
                }
            }
            Block::Table { head, rows } => o.push_str(&table(head, rows, st)),
            Block::Finding {
                rank,
                title,
                confidence,
                evidence,
                caveat,
            } => {
                let tag = match confidence {
                    Confidence::High => st.paint("32", "[high]"),
                    Confidence::Medium => st.paint("33", "[medium]"),
                    Confidence::Low => st.paint("31", "[low]"),
                };
                o.push_str(&format!("{rank}. {} {tag}\n", st.paint("1", title)));
                for e in evidence {
                    o.push_str(&format!("     {e}\n"));
                }
                o.push_str(&format!(
                    "     {}\n",
                    st.paint("2", &format!("note: {caveat}"))
                ));
            }
            Block::Chart { ms, budget_ms } => {
                let (line, top) = sparkline(ms, width.saturating_sub(4).clamp(20, 120));
                o.push_str(&format!("  {line}\n"));
                o.push_str(&format!(
                    "  top of scale {top:.1} ms (p99.9); frame budget {budget_ms:.2} ms is {:.0}% of the way up\n",
                    100.0 * budget_ms / top
                ));
            }
            Block::Hist { ms } => {
                for l in hist_lines(ms) {
                    o.push_str(&format!("  {l}\n"));
                }
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_table_is_aligned_without_color() {
        let doc = vec![Block::Table {
            head: vec!["metric".into(), "A".into()],
            rows: vec![
                vec!["average fps".into(), "144.0".into()],
                vec!["p99".into(), "9.5".into()],
            ],
        }];
        let out = terminal(&doc, &Style { color: false }, 80);
        assert!(!out.contains('\x1b'));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[2].len(), lines[3].len());
        assert!(lines[3].ends_with("9.5"));
    }

    #[test]
    fn color_when_enabled() {
        let doc = vec![Block::Verdict("x".into())];
        assert!(terminal(&doc, &Style { color: true }, 80).contains('\x1b'));
    }
}
