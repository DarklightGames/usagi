//! `usagi format`: formatting files in place, or checking that they're
//! formatted.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use pest::error::InputLocation;
use usagi_core::format::{FormatError, FormatOptions, format_text};
use usagi_core::source::LineIndex;
use usagi_core::{ParsingError, decode_source, encode};

use crate::discover;

/// What formatting one file came to.
enum Formatted {
    /// It's formatted already.
    Unchanged,
    /// Formatted, in the file's own encoding.
    Changed(Vec<u8>),
    /// It couldn't be formatted, for this reason: `line:column: message`.
    Failed(String),
}

/// Formats `text`'s bytes, in their own encoding.
fn format_bytes(bytes: &[u8], options: &FormatOptions) -> Formatted {
    let source = decode_source(bytes);
    match format_text(&source.text, options) {
        Ok(output) if output == source.text => Formatted::Unchanged,
        Ok(output) => match encode(&output, source.encoding) {
            Some(bytes) => Formatted::Changed(bytes),
            // Formatting only moves characters that are already in the file.
            None => Formatted::Failed("the output can't be written in the file's encoding".into()),
        },
        Err(e) => Formatted::Failed(describe(&source.text, &e)),
    }
}

/// `line:column: message` for a file that couldn't be formatted.
fn describe(text: &str, error: &FormatError) -> String {
    let offset = match error {
        FormatError::Parse(ParsingError::PestError(e)) => match e.location {
            InputLocation::Pos(p) => p,
            InputLocation::Span((start, _)) => start,
        },
        FormatError::Unformattable { offset, .. } => *offset,
        _ => 0,
    };
    let message = match error {
        FormatError::Parse(ParsingError::PestError(e)) => {
            format!("the file doesn't parse: {}", e.variant.message())
        }
        e => e.to_string(),
    };
    let (line, column) = LineIndex::new(text).line_column(text, offset);
    format!("{line}:{column}: {message}")
}

/// Formats standard input to standard output. Returns an error if it
/// couldn't be formatted.
pub fn stdin(options: &FormatOptions) -> Result<bool, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let output = match format_bytes(&bytes, options) {
        Formatted::Unchanged => bytes,
        Formatted::Changed(output) => output,
        Formatted::Failed(message) => return Err(format!("<stdin>:{message}")),
    };
    io::stdout()
        .lock()
        .write_all(&output)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Formats `files` in place, or with `check`, lists those that would change.
/// Returns whether nothing would change; an error if a file couldn't be
/// formatted when checking, or couldn't be read or written.
pub fn files(
    files: &[PathBuf],
    options: &FormatOptions,
    check: bool,
    jobs: usize,
    cwd: &Path,
) -> Result<bool, String> {
    use rayon::prelude::*;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .start_handler(|_| crate::check::lower_priority())
        .stack_size(256 * 1024 * 1024)
        .build()
        .map_err(|e| e.to_string())?;
    let results: Vec<Result<Formatted, String>> = pool.install(|| {
        files
            .par_iter()
            .map(|path| {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                Ok(format_bytes(&bytes, options))
            })
            .collect()
    });
    let mut out = io::stdout().lock();
    let (mut changed, mut unchanged, mut failed) = (0, 0, 0);
    for (path, result) in files.iter().zip(results) {
        let shown = discover::display(path, cwd);
        match result.map_err(|e| format!("{shown}: {e}"))? {
            Formatted::Unchanged => unchanged += 1,
            Formatted::Changed(bytes) => {
                changed += 1;
                if check {
                    writeln!(out, "Would reformat: {shown}").map_err(|e| e.to_string())?;
                } else {
                    std::fs::write(path, bytes).map_err(|e| format!("{shown}: {e}"))?;
                }
            }
            Formatted::Failed(message) => {
                failed += 1;
                eprintln!("{shown}:{message}");
            }
        }
    }
    let files = |n: usize| if n == 1 { "file" } else { "files" };
    let mut summary = if check {
        format!(
            "{changed} {} would be reformatted, {unchanged} already formatted",
            files(changed)
        )
    } else {
        format!(
            "{changed} {} reformatted, {unchanged} left unchanged",
            files(changed)
        )
    };
    if failed > 0 {
        summary.push_str(&format!(", {failed} couldn't be formatted"));
    }
    eprintln!("{summary}");
    if check && failed > 0 {
        return Err(format!("{failed} {} couldn't be formatted", files(failed)));
    }
    Ok(!check || changed == 0)
}
