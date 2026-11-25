//! Printing problems and statistics.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

use annotate_snippets::{Level, Renderer, Snippet};
use serde::Serialize;
use usagi_core::Applicability;
use usagi_core::source::LineIndex;

use crate::check::{FileReport, Kind, Outcome, Problem};
use crate::cli::OutputFormat;
use crate::discover;

#[derive(Serialize)]
struct JsonProblem<'a> {
    path: String,
    line: usize,
    column: usize,
    end_line: usize,
    end_column: usize,
    code: &'a str,
    name: &'a str,
    severity: &'a str,
    message: &'a str,
    /// The safe fix's title, if the problem has one.
    fix: Option<&'a str>,
}

/// Prints every problem in `reports`, which must already be sorted.
pub fn problems(
    out: &mut dyn Write,
    reports: &[FileReport],
    format: OutputFormat,
    cwd: &Path,
    color: bool,
) -> io::Result<()> {
    let all = || {
        reports.iter().flat_map(|r| {
            let path = discover::display(&r.path, cwd);
            r.problems.iter().map(move |p| (r, path.clone(), p))
        })
    };
    match format {
        OutputFormat::Quiet => {}
        OutputFormat::Full => {
            let renderer = if color {
                Renderer::styled()
            } else {
                Renderer::plain()
            };
            for (report, path, problem) in all() {
                writeln!(out, "{}\n", render(&renderer, report, &path, problem))?;
            }
        }
        OutputFormat::Concise => {
            for (_, path, p) in all() {
                writeln!(
                    out,
                    "{}:{}:{}: {} {}{}",
                    path,
                    p.line,
                    p.column,
                    p.kind.code(),
                    if safe_fix(p).is_some() { "[*] " } else { "" },
                    one_line(&p.message)
                )?;
            }
        }
        OutputFormat::Tsv => {
            for (_, path, p) in all() {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    p.kind.code(),
                    p.kind.name(),
                    path,
                    p.line,
                    one_line(&p.message)
                )?;
            }
        }
        OutputFormat::Json => {
            let problems: Vec<_> = all()
                .map(|(_, path, p)| JsonProblem {
                    path,
                    line: p.line,
                    column: p.column,
                    end_line: p.end_line,
                    end_column: p.end_column,
                    code: p.kind.code(),
                    name: p.kind.name(),
                    severity: if p.is_error { "error" } else { "warning" },
                    message: &p.message,
                    fix: safe_fix(p),
                })
                .collect();
            serde_json::to_writer_pretty(&mut *out, &problems)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

fn render(renderer: &Renderer, report: &FileReport, path: &str, problem: &Problem) -> String {
    let level = if problem.is_error {
        Level::Error
    } else {
        Level::Warning
    };
    let title = match problem.kind {
        Kind::Lint(lint) => format!("{} [{}]", problem.message, lint.name()),
        _ => problem.message.clone(),
    };
    let source = &report.source;
    let message = level.title(&title).id(problem.kind.code());
    if source.is_empty() {
        return renderer.render(message).to_string();
    }
    // Show only the lines the problem covers.
    let index = LineIndex::new(source);
    let first = index.line_range(source, problem.line);
    let last = index.line_range(source, problem.end_line.max(problem.line));
    let text = &source[first.start..last.end];
    let start = problem.start.clamp(first.start, last.end) - first.start;
    let end = problem.end.clamp(first.start, last.end) - first.start;
    let end = if end > start {
        end
    } else {
        // Point at one character, or past the end of an empty line.
        text[start..]
            .chars()
            .next()
            .map_or(start, |c| start + c.len_utf8())
    };
    // annotate-snippets prints a tab as spaces but doesn't move the
    // underline to match, so expand the tabs here.
    let (text, start, end) = expand_tabs(text, start, end);
    // ...and it works out the header's column from the expanded text, but a
    // tab is one column everywhere else.
    let expanded_column = text[..start].chars().count() + 1;
    let snippet = Snippet::source(&text)
        .line_start(problem.line)
        .origin(path)
        .annotation(level.span(start..end));
    let help = safe_fix(problem).map(|title| format!("{} (fixable with --fix)", title));
    let mut message = message.snippet(snippet);
    if let Some(help) = &help {
        message = message.footer(Level::Help.title(help));
    }
    renderer.render(message).to_string().replacen(
        &format!(" {}:{}:{}", path, problem.line, expanded_column),
        &format!(" {}:{}:{}", path, problem.line, problem.column),
        1,
    )
}

/// How many columns a tab takes when a problem's source is shown.
const TAB_WIDTH: usize = 4;

/// `text` with each tab replaced by spaces up to the next tab stop, and the
/// byte offsets `start` and `end` moved to match.
fn expand_tabs(text: &str, start: usize, end: usize) -> (String, usize, usize) {
    let mut expanded = String::with_capacity(text.len());
    let (mut new_start, mut new_end) = (start, end);
    let mut column = 0;
    for (i, c) in text.char_indices() {
        if i == start {
            new_start = expanded.len();
        }
        if i == end {
            new_end = expanded.len();
        }
        match c {
            '\t' => {
                let width = TAB_WIDTH - column % TAB_WIDTH;
                expanded.extend(std::iter::repeat_n(' ', width));
                column += width;
            }
            '\n' => {
                expanded.push(c);
                column = 0;
            }
            _ => {
                expanded.push(c);
                column += 1;
            }
        }
    }
    if end == text.len() {
        new_end = expanded.len();
    }
    if start == text.len() {
        new_start = expanded.len();
    }
    (expanded, new_start, new_end)
}

/// The title of `problem`'s fix, if it can be applied with `--fix`.
fn safe_fix(problem: &Problem) -> Option<&str> {
    match &problem.fix {
        Some((title, Applicability::Safe)) => Some(title),
        _ => None,
    }
}

/// Prints a unified diff of each file that fixes changed, coloured if
/// `color`: the file headers bold, hunk headers cyan, removed lines red and
/// added lines green.
pub fn diffs(
    out: &mut dyn Write,
    reports: &[FileReport],
    cwd: &Path,
    color: bool,
) -> io::Result<()> {
    for report in reports {
        if let Some(original) = &report.original {
            let path = discover::display(&report.path, cwd);
            let diff = similar::TextDiff::from_lines(original.as_str(), report.source.as_str());
            let text = diff
                .unified_diff()
                .header(&format!("a/{}", path), &format!("b/{}", path))
                .to_string();
            if !color {
                write!(out, "{text}")?;
                continue;
            }
            for line in text.split_inclusive('\n') {
                // The colour ends before the line ending, `\r` included.
                let body = line.trim_end_matches(['\r', '\n']);
                let end = &line[body.len()..];
                let code = if body.starts_with("---") || body.starts_with("+++") {
                    "1"
                } else if body.starts_with("@@") {
                    "36"
                } else if body.starts_with('-') {
                    "31"
                } else if body.starts_with('+') {
                    "32"
                } else {
                    write!(out, "{line}")?;
                    continue;
                };
                write!(out, "\x1b[{code}m{body}\x1b[0m{end}")?;
            }
        }
    }
    Ok(())
}

fn one_line(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

/// Prints how many files had each outcome.
pub fn outcome_statistics(out: &mut dyn Write, reports: &[FileReport]) -> io::Result<()> {
    let mut counts: BTreeMap<Outcome, usize> = BTreeMap::new();
    for report in reports {
        *counts.entry(report.outcome).or_default() += 1;
    }
    writeln!(out, "{} files", reports.len())?;
    for (outcome, count) in counts {
        writeln!(out, "  {:14} {:>6}", outcome.name(), count)?;
    }
    Ok(())
}

/// Prints how many problems, in how many files, each lint found.
pub fn lint_statistics(out: &mut dyn Write, reports: &[FileReport]) -> io::Result<()> {
    // (code, name) -> (problems, files)
    let mut counts: BTreeMap<(&str, &str), (usize, usize)> = BTreeMap::new();
    for report in reports {
        let mut seen = HashSet::new();
        for problem in &report.problems {
            let key = (problem.kind.code(), problem.kind.name());
            let entry = counts.entry(key).or_default();
            entry.0 += 1;
            if seen.insert(key) {
                entry.1 += 1;
            }
        }
    }
    for ((code, name), (problems, files)) in counts {
        writeln!(
            out,
            "{:6} {:40} {:>8} problems in {:>6} files",
            code, name, problems, files
        )?;
    }
    Ok(())
}
