//! The formatter, `usagi format`: see `docs/FORMATTING.md`.
//!
//! [`format_text`] parses a file, prints it in the house style, and checks the
//! result: it must parse to the same program as the original, with the same
//! comments. The formatter is allowed a few changes that don't change what
//! the code does (sorting modifiers, adding braces, removing extra
//! semicolons), so programs are compared after [`normalize`]-ing both.

mod normalize;
mod printer;

pub use normalize::normalize;

use crate::ast::Program;
use crate::parser::{ParsingError, parse_program};
use std::fmt;

/// How to indent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    /// This many spaces per level.
    Spaces(usize),
    /// One tab per level.
    Tabs,
}

/// The formatter's settings: the `[format]` table of `usagi.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    pub indent: Indent,
    /// The longest a line may be. Not used yet.
    pub line_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            indent: Indent::Spaces(4),
            line_width: 120,
        }
    }
}

/// Why a file couldn't be formatted.
#[derive(Debug)]
pub enum FormatError {
    /// The file doesn't parse.
    Parse(ParsingError),
    /// The file has something the formatter can't print, such as a comment
    /// where it has no place for one. `offset` is where, in bytes.
    Unformattable { offset: usize, message: String },
    /// The formatter's output isn't the same program as its input. This is a
    /// bug in the formatter. `output` is what it printed.
    Changed { reason: String, output: String },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Parse(e) => write!(f, "the file doesn't parse: {e:?}"),
            FormatError::Unformattable { message, .. } => f.write_str(message),
            FormatError::Changed { reason: why, .. } => write!(
                f,
                "formatting would change the program ({why}); this is a bug in usagi"
            ),
        }
    }
}

/// Formats an UnrealScript file's text. Its line endings are kept: if it has
/// any CRLF, the output's are all CRLF.
pub fn format_text(text: &str, options: &FormatOptions) -> Result<String, FormatError> {
    let crlf = text.contains("\r\n");
    let original = text;
    let text = text.replace("\r\n", "\n");
    let before = parse_program(&text).map_err(FormatError::Parse)?;
    let output = printer::print(&text, &before.comments, &before.program, options).map_err(
        |(offset, message)| FormatError::Unformattable {
            // An offset into `original`, which may have CRLFs.
            offset: original_offset(original, offset),
            message,
        },
    )?;
    verify(&text, before, output).map(|output| {
        if crlf {
            output.replace('\n', "\r\n")
        } else {
            output
        }
    })
}

/// Checks that `output`, the formatted `text`, is the same program as
/// `before`, `text`'s parse, with the same comments.
fn verify(
    text: &str,
    before: crate::parser::ProgramResult,
    output: String,
) -> Result<String, FormatError> {
    let after = match parse_program(&output) {
        Ok(after) => after,
        Err(e) => {
            return Err(FormatError::Changed {
                reason: format!("the output doesn't parse: {e:?}"),
                output,
            });
        }
    };
    let comments = |text: &str, ranges: &[std::ops::Range<usize>]| -> Vec<String> {
        ranges
            .iter()
            .map(|r| {
                text[r.clone()]
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    };
    if comments(text, &before.comments) != comments(&output, &after.comments) {
        return Err(FormatError::Changed {
            reason: "the comments differ".to_string(),
            output,
        });
    }
    if !equivalent(before.program, after.program) {
        return Err(FormatError::Changed {
            reason: "the code differs".to_string(),
            output,
        });
    }
    Ok(output)
}

/// The offset in `original` of `offset` in `original` with its CRLFs made LF.
fn original_offset(original: &str, offset: usize) -> usize {
    let mut removed = 0;
    for (i, _) in original.match_indices("\r\n") {
        if i - removed >= offset {
            break;
        }
        removed += 1;
    }
    offset + removed
}

/// Whether `a` and `b` are the same program, apart from the changes the
/// formatter is allowed to make. Spans, and the case of keywords and
/// identifiers, are ignored.
pub fn equivalent(mut a: Program, mut b: Program) -> bool {
    normalize(&mut a);
    normalize(&mut b);
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified(text: &str, output: &str) -> Result<String, FormatError> {
        verify(text, parse_program(text).unwrap(), output.to_string())
    }

    #[test]
    fn output_that_isnt_the_same_program_is_refused() {
        let text = "class A extends Object;\nvar int X; // X\n";
        assert!(verified(text, text).is_ok());
        for (output, reason) in [
            (
                "class A extends Object;\nvar int X // X\n",
                "the output doesn't parse",
            ),
            (
                "class A extends Object;\nvar int X; // Y\n",
                "the comments differ",
            ),
            (
                "class A extends Object;\nvar int X;\n",
                "the comments differ",
            ),
            (
                "class A extends Object;\nvar int Y; // X\n",
                "the code differs",
            ),
        ] {
            let error = format!("{:?}", verified(text, output).unwrap_err());
            assert!(error.starts_with("Changed {"), "{error}");
            assert!(error.contains(&format!("reason: \"{reason}")), "{error}");
            assert!(error.contains(&format!("output: {output:?}")), "{error}");
        }
    }

    #[test]
    fn errors_are_described() {
        let parse = format_text("class", &FormatOptions::default()).unwrap_err();
        assert!(parse.to_string().starts_with("the file doesn't parse: "));
        let unformattable = FormatError::Unformattable {
            offset: 0,
            message: "a comment here can't be placed".to_string(),
        };
        assert_eq!(unformattable.to_string(), "a comment here can't be placed");
        let changed = FormatError::Changed {
            reason: "the code differs".to_string(),
            output: String::new(),
        };
        assert_eq!(
            changed.to_string(),
            "formatting would change the program (the code differs); this is a bug in usagi"
        );
    }
}
