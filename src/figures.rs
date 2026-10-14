//! Cited figures in markdown: `29.8%<!-- src: corpus/frame/gap.md -->`.
//!
//! A figure about the data is written by a script into a committed file. Prose that repeats the
//! figure cites that file with an HTML comment straight after the number (GitHub does not render
//! the comment). `s106 check-figures` finds every cited figure under a folder and fails when the
//! cited file is missing or no longer contains the figure, so a regenerated file that changes a
//! number fails CI until the prose quoting the old number is fixed. The cited path is relative to
//! the folder being checked. Figures inside code fences and inline code are examples, not
//! citations, and are skipped. Untagged numbers are not checked.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;

/// A number (digits, optional thousands commas and decimals, optional %) directly followed by a
/// `<!-- src: path -->` comment.
static CITED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9][0-9,]*(?:\.[0-9]+)?%?)<!--\s*src:\s*([^\s>]+)\s*-->").unwrap()
});

/// One cited figure that fails the check.
#[derive(Debug, PartialEq)]
pub struct Problem {
    /// the markdown file, relative to the checked folder
    pub doc: PathBuf,
    pub line: usize,
    pub figure: String,
    /// why the figure fails, naming the cited file
    pub why: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {} {}",
            self.doc.display(),
            self.line,
            self.figure,
            self.why
        )
    }
}

/// What `check` found: how many cited figures it read, and the ones that failed.
#[derive(Debug, Default)]
pub struct Report {
    pub cited: usize,
    pub problems: Vec<Problem>,
}

/// Check every cited figure in every `.md` file under `root`.
pub fn check(root: &Path) -> Result<Report> {
    let mut docs = Vec::new();
    markdown_files(root, &mut docs)?;
    docs.sort();
    let mut report = Report::default();
    for doc in docs {
        let text =
            std::fs::read_to_string(&doc).with_context(|| format!("reading {}", doc.display()))?;
        let rel = doc.strip_prefix(root).unwrap_or(&doc).to_path_buf();
        for (line, figure, src) in citations(&text) {
            report.cited += 1;
            let why = match std::fs::read_to_string(root.join(&src)) {
                Err(e) => format!("cites {src}, which cannot be read: {e}"),
                Ok(cited) if !contains_figure(&cited, &figure) => format!("is not in {src}"),
                Ok(_) => continue,
            };
            report.problems.push(Problem {
                doc: rel.clone(),
                line,
                figure,
                why,
            });
        }
    }
    Ok(report)
}

/// The cited figures in one markdown text: (1-based line, figure, cited path). Skips code fences
/// and inline code.
pub fn citations(text: &str) -> Vec<(usize, String, String)> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let prose = strip_inline_code(line);
        for c in CITED.captures_iter(&prose) {
            out.push((i + 1, c[1].to_string(), c[2].to_string()));
        }
    }
    out
}

/// `line` with every `inline code` span blanked out.
fn strip_inline_code(line: &str) -> String {
    line.split('`')
        .enumerate()
        .map(|(i, part)| if i % 2 == 0 { part } else { "" })
        .collect::<Vec<_>>()
        .join("``")
}

/// Whether `figure` appears in `text` as a whole number: not part of a longer number, so 29.8
/// is not found in 129.8 or 29.85. A figure written with thousands commas (1,234) is also found
/// without them (1234), as a CSV stores it.
pub fn contains_figure(text: &str, figure: &str) -> bool {
    let bare = figure.replace(',', "");
    whole_match(text, figure) || (bare != figure && whole_match(text, &bare))
}

fn whole_match(text: &str, figure: &str) -> bool {
    text.match_indices(figure).any(|(at, _)| {
        !continues_number(text[..at].chars().rev())
            && !continues_number(text[at + figure.len()..].chars())
    })
}

/// Whether the characters next to a match (walking away from it) extend the number: a digit, or
/// "." or "," then a digit. "29.8." ending a sentence and ",29.8," in a CSV do not.
fn continues_number(mut chars: impl Iterator<Item = char>) -> bool {
    match chars.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('.' | ',') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                markdown_files(&path, out)?;
            }
        } else if name.ends_with(".md") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cited_figures_and_skips_code() {
        let text = "Share: 29.8%<!-- src: gap.md -->.\n\
                    Total 1,234<!--src:t.csv--> homes, 12 untagged.\n\
                    ```\n5%<!-- src: x.md -->\n```\n\
                    Syntax: `7%<!-- src: y.md -->` and 8<!-- src: z.md -->";
        assert_eq!(
            citations(text),
            vec![
                (1, "29.8%".into(), "gap.md".into()),
                (2, "1,234".into(), "t.csv".into()),
                (6, "8".into(), "z.md".into()),
            ]
        );
    }

    #[test]
    fn figure_must_be_a_whole_number() {
        assert!(contains_figure("| v1 | 29.8% |", "29.8%"));
        assert!(contains_figure("share 29.8.", "29.8"));
        assert!(contains_figure("a,29.8,b", "29.8"));
        assert!(!contains_figure("129.8", "29.8"));
        assert!(!contains_figure("29.85", "29.8"));
        assert!(!contains_figure("29.8,5", "29.8"));
        assert!(!contains_figure("29.7%", "29.8%"));
        assert!(contains_figure("units\n34233\n", "34,233"));
        assert!(!contains_figure("341233", "34,233"));
    }

    #[test]
    fn check_reports_stale_and_missing_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("frame")).unwrap();
        std::fs::write(root.join("frame/gap.md"), "| v1 | 29.8% |\n").unwrap();
        std::fs::write(root.join("frame/x.gz"), [0x1f, 0x8b, 0xff, 0xfe]).unwrap();
        std::fs::write(
            root.join("frame/README.md"),
            "Old: 29.7%<!-- src: frame/gap.md -->\nNew: 29.8%<!-- src: frame/gap.md -->\n\
             Gone: 4<!-- src: frame/none.csv -->\nPacked: 5<!-- src: frame/x.gz -->\n",
        )
        .unwrap();
        let report = check(root).unwrap();
        assert_eq!(report.cited, 4);
        let lines: Vec<String> = report.problems.iter().map(|p| p.to_string()).collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "frame/README.md:1: 29.7% is not in frame/gap.md");
        assert!(
            lines[1]
                .starts_with("frame/README.md:3: 4 cites frame/none.csv, which cannot be read: ")
        );
        assert!(
            lines[2].contains("frame/x.gz, which cannot be read: ") && lines[2].contains("UTF-8")
        );
    }
}
