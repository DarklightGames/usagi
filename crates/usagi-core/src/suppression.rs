//! Suppression comments: `// usagi: ignore[S008]` and
//! `// usagi: ignore-file[naming]`.
//!
//! A comment whose text starts with `usagi:` is a directive. `ignore[...]`
//! suppresses the problems of the lints it names on one line: the comment's
//! own line if code shares it, or the next line if the comment is alone on
//! its line. `ignore-file[...]` suppresses them in the whole file. Between
//! the brackets is a comma-separated list of lint selectors, as
//! [`crate::LintSelection`] takes them. Text after the `]` is free, e.g. a
//! reason.
//!
//! A problem is suppressed if it starts on a suppressed line. Problems that
//! aren't lints, and the two lints about suppressions themselves
//! ([`Lint::UnusedSuppression`] and [`Lint::InvalidSuppression`]), can't be
//! suppressed.

use crate::ast::AstSpan;
use crate::fix::{Edit, Fix, line_aware_deletion};
use crate::lint::{Lint, selector_matches};
use crate::parser::ProgramError;
use crate::source::LineIndex;
use std::ops::{Range, RangeInclusive};

/// The suppression comments of a file, and which of their selectors have
/// suppressed something.
#[derive(Debug, Clone, Default)]
pub struct Suppressions {
    directives: Vec<Directive>,
    /// Problems with the comments themselves: malformed directives and
    /// selectors that match no lint.
    invalid: Vec<ProgramError>,
}

#[derive(Debug, Clone)]
struct Directive {
    /// The whole comment.
    comment: Range<usize>,
    /// The lines it suppresses, or `None` for the whole file.
    lines: Option<RangeInclusive<usize>>,
    selectors: Vec<Selector>,
}

#[derive(Debug, Clone)]
struct Selector {
    text: String,
    range: Range<usize>,
    /// Whether it matches any lint. Unknown selectors are reported as
    /// invalid, never as unused.
    known: bool,
    used: bool,
}

impl Suppressions {
    /// Finds the directives among `comments` (byte ranges into `text`).
    pub fn parse(text: &str, comments: &[Range<usize>]) -> Suppressions {
        let lines = LineIndex::new(text);
        let mut suppressions = Suppressions::default();
        for comment in comments {
            suppressions.parse_comment(text, &lines, comment.clone());
        }
        suppressions
    }

    fn parse_comment(&mut self, text: &str, lines: &LineIndex, comment: Range<usize>) {
        let raw = &text[comment.clone()];
        // `//` or `/*`, and `*/` if the comment is closed.
        let body_end = if raw.starts_with("/*") && raw.len() >= 4 && raw.ends_with("*/") {
            comment.end - 2
        } else {
            comment.end
        };
        let mut at = comment.start + 2;
        at += leading_whitespace(&text[at..body_end]);
        let Some(rest) = text[at..body_end].strip_prefix("usagi:") else {
            return;
        };
        at += "usagi:".len();
        at += leading_whitespace(rest);
        let word_len = text[at..body_end]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(body_end - at);
        let word = &text[at..at + word_len];
        let file = match word {
            "ignore" => false,
            "ignore-file" => true,
            _ => {
                self.invalid(
                    format!(
                        "Unknown directive `usagi: {}`; expected `usagi: ignore[...]` or `usagi: ignore-file[...]`",
                        word
                    ),
                    comment,
                );
                return;
            }
        };
        at += word_len;
        if !text[at..body_end].starts_with('[') {
            self.invalid(
                format!(
                    "`usagi: {}` needs the lints to ignore in brackets, e.g. `usagi: {}[S008]`",
                    word, word
                ),
                comment,
            );
            return;
        }
        at += 1;
        let Some(close) = text[at..body_end].find(']') else {
            self.invalid(format!("`usagi: {}[` has no closing `]`", word), comment);
            return;
        };
        let mut selectors = Vec::new();
        let mut start = at;
        for item in text[at..at + close].split(',') {
            let item_start = start + leading_whitespace(item);
            let trimmed = item.trim();
            start += item.len() + 1;
            if trimmed.is_empty() {
                continue;
            }
            let range = item_start..item_start + trimmed.len();
            let known = Lint::ALL.iter().any(|&l| selector_matches(trimmed, l));
            if !known {
                self.invalid(format!("No lint matches `{}`", trimmed), range.clone());
            }
            selectors.push(Selector {
                text: trimmed.to_string(),
                range,
                known,
                used: false,
            });
        }
        if selectors.is_empty() {
            self.invalid(format!("`usagi: {}[]` names no lints", word), comment);
            return;
        }
        let lines = (!file).then(|| {
            let first = lines.line(comment.start);
            let last = lines.line(comment.end);
            if is_alone_on_line(text, comment.clone()) {
                last + 1..=last + 1
            } else {
                first..=last
            }
        });
        self.directives.push(Directive {
            comment,
            lines,
            selectors,
        });
    }

    fn invalid(&mut self, message: String, range: Range<usize>) {
        self.invalid
            .push(problem(Lint::InvalidSuppression, message, range, None));
    }

    /// Whether `error`, which starts on `line`, is suppressed. Marks the
    /// selectors that suppress it as used. The problems from
    /// [`Suppressions::problems`] are added after suppressing, so they are
    /// never suppressed.
    pub fn suppress(&mut self, error: &ProgramError, line: usize) -> bool {
        let Some(lint) = error.lint else {
            return false;
        };
        let mut suppressed = false;
        for directive in &mut self.directives {
            if directive.lines.as_ref().is_some_and(|l| !l.contains(&line)) {
                continue;
            }
            for selector in &mut directive.selectors {
                if selector.known && selector_matches(&selector.text, lint) {
                    selector.used = true;
                    suppressed = true;
                }
            }
        }
        suppressed
    }

    /// Removes the suppressed problems from `errors`.
    pub fn retain(&mut self, text: &str, errors: &mut Vec<ProgramError>) {
        let lines = LineIndex::new(text);
        errors.retain(|e| !self.suppress(e, lines.line(e.span.start)));
    }

    /// The problems with the suppressions: invalid directives and selectors,
    /// and selectors that suppressed nothing, each with a fix that removes
    /// it. `text` is the source the comments are in.
    pub fn problems(&self, text: &str) -> Vec<ProgramError> {
        let mut problems = self.invalid.clone();
        for directive in &self.directives {
            let all_unused = directive.selectors.iter().all(|s| !s.used);
            let place = match directive.lines {
                Some(_) => "here",
                None => "in this file",
            };
            for (i, selector) in directive.selectors.iter().enumerate() {
                // A lint that needs the project is checked after this, so
                // that its suppression isn't known to be unused yet.
                let later = Lint::ALL
                    .iter()
                    .any(|&l| l.needs_project() && selector_matches(&selector.text, l));
                if !selector.known || selector.used || later {
                    continue;
                }
                let range = if all_unused {
                    comment_deletion(text, directive.comment.clone())
                } else {
                    selector_deletion(&directive.selectors, i)
                };
                let title = if all_unused {
                    "Remove the suppression comment".to_string()
                } else {
                    format!("Remove `{}`", selector.text)
                };
                problems.push(problem(
                    Lint::UnusedSuppression,
                    format!("`{}` suppresses nothing {}", selector.text, place),
                    selector.range.clone(),
                    Some(Fix::safe(title, vec![Edit::delete(range)])),
                ));
            }
        }
        problems
    }
}

fn problem(lint: Lint, message: String, range: Range<usize>, fix: Option<Fix>) -> ProgramError {
    ProgramError {
        message,
        span: AstSpan {
            start: range.start,
            end: range.end,
            line: None,
        },
        severity: lint.severity(),
        lint: Some(lint),
        fix,
    }
}

fn leading_whitespace(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn is_blank(s: &str) -> bool {
    s.chars().all(|c| c == ' ' || c == '\t' || c == '\r')
}

/// The text before `range` on its first line, and after it on its last.
fn surroundings(text: &str, range: Range<usize>) -> (&str, &str) {
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let after = &text[range.end..];
    let line_end = after.find('\n').map_or(text.len(), |i| range.end + i);
    (&text[line_start..range.start], &text[range.end..line_end])
}

fn is_alone_on_line(text: &str, range: Range<usize>) -> bool {
    let (before, after) = surroundings(text, range);
    is_blank(before) && is_blank(after)
}

/// The range to delete to remove a comment: its line if nothing else is on
/// it, or else the comment and the spaces between it and the code.
fn comment_deletion(text: &str, comment: Range<usize>) -> Range<usize> {
    let (before, after) = surroundings(text, comment.clone());
    let spaces = |s: &str| s.len() - s.trim_end_matches([' ', '\t']).len();
    if !is_blank(before) {
        comment.start - spaces(before)..comment.end
    } else if !is_blank(after) {
        comment.start..comment.end + leading_whitespace(after)
    } else {
        line_aware_deletion(text, comment)
    }
}

/// The range to delete to remove the `i`th of `selectors` and a comma next
/// to it. There is more than one selector.
fn selector_deletion(selectors: &[Selector], i: usize) -> Range<usize> {
    match selectors.get(i + 1) {
        Some(next) => selectors[i].range.start..next.range.start,
        None => selectors[i - 1].range.end..selectors[i].range.end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unclosed_block_comment() {
        // The parser rejects one, but the directive is still read.
        let text = "/* usagi: ignore[S003]";
        let suppressions = Suppressions::parse(text, &[0..text.len()]);
        assert_eq!(suppressions.directives[0].selectors[0].text, "S003");
        assert_eq!(suppressions.directives[0].lines, Some(2..=2));
    }
}
