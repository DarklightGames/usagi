//! `usagi lint --add-ignore`: suppressing the reported problems with
//! `// usagi: ignore[...]` comments.

use std::collections::BTreeMap;

use usagi_core::Lint;

use crate::check::{FileReport, Kind};

const DIRECTIVE: &str = "// usagi: ignore[";

/// How a suppression comment names a lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Name,
    Code,
}

impl Style {
    fn spell(self, lint: Lint) -> &'static str {
        match self {
            Style::Name => lint.name(),
            Style::Code => lint.code(),
        }
    }
}

/// The lints to suppress on each line (1-based) of a file's report.
/// The lints about suppressions themselves can't be suppressed.
pub fn targets(report: &FileReport) -> BTreeMap<usize, Vec<Lint>> {
    let mut lines: BTreeMap<usize, Vec<Lint>> = BTreeMap::new();
    for problem in &report.problems {
        let Kind::Lint(lint) = problem.kind else {
            continue;
        };
        if matches!(lint, Lint::UnusedSuppression | Lint::InvalidSuppression) {
            continue;
        }
        let codes = lines.entry(problem.line).or_default();
        if !codes.contains(&lint) {
            codes.push(lint);
        }
    }
    lines
}

/// Adds a suppression comment above each line of `targets`, naming lints in
/// `style`, or adds them to
/// the one already there. Returns the new text and the number of lines
/// that got codes.
pub fn add_ignores(
    text: &str,
    targets: &BTreeMap<usize, Vec<Lint>>,
    style: Style,
) -> (String, usize) {
    let mut out: Vec<String> = Vec::new();
    let mut count = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if let Some(codes) = targets.get(&(i + 1)) {
            let eol = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            let existing = out.last_mut().filter(|last| {
                let body = last.trim_end();
                body.trim_start().starts_with(DIRECTIVE) && body.ends_with(']')
            });
            let mut changed = false;
            match existing {
                Some(last) => {
                    let body_len = last.trim_end().len();
                    let listed = last[..body_len].to_string();
                    // A lint counts as listed however it is spelled.
                    let missing: Vec<&str> = codes
                        .iter()
                        .filter(|&&lint| {
                            !listed
                                .split(['[', ',', ']'])
                                .any(|s| Lint::find(s.trim()) == Some(lint))
                        })
                        .map(|&lint| style.spell(lint))
                        .collect();
                    if !missing.is_empty() {
                        last.insert_str(body_len - 1, &format!(", {}", missing.join(", ")));
                        changed = true;
                    }
                }
                None => {
                    let spelled: Vec<&str> = codes.iter().map(|&l| style.spell(l)).collect();
                    let indent = &line[..line.len() - line.trim_start().len()];
                    out.push(format!(
                        "{}{}{}]{}",
                        indent,
                        DIRECTIVE,
                        spelled.join(", "),
                        eol
                    ));
                    changed = true;
                }
            }
            count += usize::from(changed);
        }
        out.push(line.to_string());
    }
    (out.concat(), count)
}
