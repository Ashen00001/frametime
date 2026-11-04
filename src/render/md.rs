//! Markdown rendering.

use super::{hist_lines, sparkline, Block};

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

pub fn markdown(doc: &[Block]) -> String {
    let mut o = String::new();
    for b in doc {
        match b {
            Block::Title(t) => o.push_str(&format!("# {t}\n\n")),
            Block::Heading(h) => o.push_str(&format!("## {h}\n\n")),
            Block::Para(p) => o.push_str(&format!("{p}\n\n")),
            Block::Verdict(v) => o.push_str(&format!("**{v}**\n\n")),
            Block::Bullets(v) => {
                for i in v {
                    o.push_str(&format!("- {i}\n"));
                }
                o.push('\n');
            }
            Block::Table { head, rows } => {
                o.push_str(&format!(
                    "| {} |\n",
                    head.iter().map(|h| esc(h)).collect::<Vec<_>>().join(" | ")
                ));
                o.push_str(&format!("|{}|\n", vec![" --- "; head.len()].join("|")));
                for r in rows {
                    o.push_str(&format!(
                        "| {} |\n",
                        r.iter().map(|c| esc(c)).collect::<Vec<_>>().join(" | ")
                    ));
                }
                o.push('\n');
            }
            Block::Finding {
                rank,
                title,
                confidence,
                evidence,
                caveat,
            } => {
                o.push_str(&format!(
                    "{rank}. **{title}** (confidence: {})\n",
                    confidence.label()
                ));
                for e in evidence {
                    o.push_str(&format!("   - {e}\n"));
                }
                o.push_str(&format!("   - _note: {caveat}_\n"));
            }
            Block::Chart { ms, budget_ms } => {
                let (line, top) = sparkline(ms, 80);
                o.push_str(&format!(
                    "```\n{line}\n```\n\nTop of scale {top:.1} ms (p99.9); frame budget {budget_ms:.2} ms.\n\n"
                ));
            }
            Block::Hist { ms } => {
                o.push_str(&format!("```\n{}\n```\n\n", hist_lines(ms).join("\n")));
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_escapes_pipes() {
        let md = markdown(&[Block::Table {
            head: vec!["a|b".into()],
            rows: vec![vec!["x|y".into()]],
        }]);
        assert!(md.contains("a\\|b") && md.contains("x\\|y"));
    }
}
