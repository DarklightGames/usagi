//! Rewriting UCC's errors and warnings as `path:line: error: message`.
//!
//! UCC prints each one as `<context> : <Severity>, <message>`
//! (`Core/Inc/FFeedbackContextAnsi.h`), where the context, from
//! `FScriptCompiler::GetContext` (`Editor/Src/UnScrCom.cpp`), is the class's
//! file and line, `Z:\Game\Pkg\Classes\Foo.uc(12)`. While it imports a class's
//! `defaultproperties`, the line is always 1 and the context ends with
//! ` : defaultproperties`, and most messages start with the class's path
//! (`Pkg.Foo: Unknown property in defaults: Bar=1`, `Editor/Src/UnEditor.cpp`).
//!
//! Some builds of UCC (Red Orchestra's) set no context while importing
//! defaults, so their messages come with no file or severity at all:
//! `Pkg.Foo: Unknown property in defaults: Bar=1`, or, for those without the
//! class, `FloatProperty::ImportText failed in: abc`. These are recognised by
//! their wording while UCC is importing a class's defaults (after
//! `Importing Defaults for Foo`, in the package of the last heading), and are
//! about that class's file.
//!
//! The rewritten line has the file as a path on this machine (under Wine,
//! UCC prints Windows paths), relative to the current directory when the
//! file is under it, and for `defaultproperties`, the line that the message
//! quotes, or else the line of the `defaultproperties` keyword.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use usagi_core::source::{comment_ranges, decode};

use super::find_ci;
use crate::discover;

/// UCC's severities (`FName(Event)`): `Error` is an error, the rest warnings.
const SEVERITIES: [&str; 4] = ["Error", "Warning", "ExecWarning", "ScriptWarning"];

const DEFAULTPROPERTIES: &str = "defaultproperties";

/// One of UCC's errors or warnings, as it printed it.
#[derive(Debug, PartialEq, Eq)]
struct UccDiagnostic<'a> {
    /// The file, as UCC printed it.
    file: &'a str,
    line: usize,
    in_defaults: bool,
    is_error: bool,
    message: &'a str,
}

/// Reads `<file>(<line>)[ : defaultproperties] : <Severity>, <message>`.
fn parse(line: &str) -> Option<UccDiagnostic<'_>> {
    let (start, severity) = SEVERITIES
        .iter()
        .filter_map(|s| Some((line.find(&format!(" : {}, ", s))?, *s)))
        .min()?;
    let message = &line[start + severity.len() + 5..];
    let mut context = &line[..start];
    let in_defaults = match context.strip_suffix(" : defaultproperties") {
        Some(rest) => {
            context = rest;
            true
        }
        None => false,
    };
    let (file, number) = context.strip_suffix(')')?.rsplit_once('(')?;
    Some(UccDiagnostic {
        file: file.trim_start(),
        line: number.parse().ok()?,
        in_defaults,
        is_error: severity == "Error",
        message,
    })
}

/// The beginnings of the messages that UCC logs while importing defaults
/// (`Editor/Src/UnEditor.cpp`, `ImportProperties`, and `Core/Src/UnProp.cpp`,
/// `ImportText`), after the class's path if they have it, and whether each
/// is an error rather than a warning.
const DEFAULTS_MESSAGES: &[(&str, bool)] = &[
    ("Unknown property in defaults:", true),
    ("Out of bound array default property", true),
    ("Missing '\"' in string default properties", true),
    ("Delegate assignment failed:", true),
    ("Missing '=' in default properties assignment:", false),
    ("Missing ')' in default properties subscript:", false),
    ("redundant data:", true),
    ("BEGIN OBJECT:", true),
    ("Unknown member ", true),
    ("ReadToken: Bad quoted string", false),
];

/// Messages logged while importing defaults that contain these, after a
/// property's name (`FloatProperty::ImportText failed in: abc`,
/// `ObjectProperty Pkg.Foo.Bar: unresolved reference to 'X'`), are errors.
const DEFAULTS_ERRORS_CONTAINING: &[&str] = &[
    "::ImportText",
    ": unresolved reference to ",
    ": unresolved cast in ",
    ": bad cast in ",
];

/// Whether `message`, logged while importing defaults, is an error (`true`)
/// or a warning (`false`), if it's one of the importer's.
fn defaults_severity(message: &str) -> Option<bool> {
    let message = message.trim();
    let unprefixed = match message.split_once(": ") {
        Some((class, rest)) if is_object_path(class) => rest,
        _ => message,
    };
    DEFAULTS_MESSAGES
        .iter()
        .find(|(start, _)| unprefixed.starts_with(start))
        .map(|&(_, is_error)| is_error)
        .or_else(|| {
            DEFAULTS_ERRORS_CONTAINING
                .iter()
                .any(|s| message.contains(s))
                .then_some(true)
        })
}

/// Turns UCC's diagnostics into `path:line: severity: message`.
pub struct Rewriter {
    /// The sources being compiled, by `<pkg>/classes/<class>.uc` in lower
    /// case.
    files: HashMap<String, PathBuf>,
    cwd: PathBuf,
    /// Where Wine's drives are, to find files that aren't in `files`.
    dosdevices: Option<PathBuf>,
    /// Files read to find `defaultproperties` lines.
    sources: HashMap<PathBuf, Option<String>>,
    /// The package of UCC's last heading.
    package: Option<String>,
    /// The class whose defaults UCC is importing.
    defaults_class: Option<String>,
}

impl Rewriter {
    pub fn new<'a>(files: impl IntoIterator<Item = &'a PathBuf>, cwd: PathBuf) -> Self {
        let files = files
            .into_iter()
            .filter_map(|path| Some((tail_key(&path.to_string_lossy())?, path.clone())))
            .collect();
        Rewriter {
            files,
            cwd,
            dosdevices: wine_dosdevices(),
            sources: HashMap::new(),
            package: None,
            defaults_class: None,
        }
    }

    /// `line`, rewritten if it's one of UCC's errors or warnings.
    pub fn rewrite<'a>(&mut self, line: &'a str) -> Cow<'a, str> {
        let trimmed = line.trim();
        if let Some(package) = super::progress::heading(line) {
            self.package = Some(package.to_string());
            self.defaults_class = None;
        } else if let Some(class) = trimmed.strip_prefix("Importing Defaults for ") {
            self.defaults_class = Some(class.trim().to_string());
        } else if [
            "Analyzing...",
            "Parsing ",
            "Compiling ",
            "Success",
            "Failure",
        ]
        .iter()
        .any(|p| trimmed.starts_with(p))
        {
            self.defaults_class = None;
        }
        let Some(diagnostic) = parse(line) else {
            return self
                .rewrite_defaults(line)
                .map_or(Cow::Borrowed(line), Cow::Owned);
        };
        let indent = &line[..line.len() - line.trim_start().len()];
        let path = self.resolve(diagnostic.file);
        let (shown, number, message) = match &path {
            Some(path) => {
                let (number, message) = if diagnostic.in_defaults {
                    self.defaults_line(path, diagnostic.message)
                } else {
                    (diagnostic.line, diagnostic.message.to_string())
                };
                (discover::display(path, &self.cwd), number, message)
            }
            None => {
                let message = if diagnostic.in_defaults {
                    defaults_message(diagnostic.message)
                } else {
                    diagnostic.message.to_string()
                };
                (diagnostic.file.to_string(), diagnostic.line, message)
            }
        };
        Cow::Owned(format!(
            "{}{}:{}: {}: {}",
            indent,
            shown,
            number,
            if diagnostic.is_error {
                "error"
            } else {
                "warning"
            },
            message
        ))
    }

    /// `line`, rewritten if it's a message of the defaults importer logged
    /// without a context, while importing a class's defaults.
    fn rewrite_defaults(&mut self, line: &str) -> Option<String> {
        let class = self.defaults_class.as_deref()?;
        let package = self.package.as_deref()?;
        let indent = &line[..line.len() - line.trim_start().len()];
        let mut message = line.trim();
        // In case the severity is shown, as it is in UCC.log.
        let mut severity = None;
        for (prefix, is_error) in [("Error: ", true), ("Warning: ", false)] {
            if let Some(rest) = message.strip_prefix(prefix) {
                message = rest;
                severity = Some(is_error);
            }
        }
        let is_error = severity.or(defaults_severity(message))?;
        let key = format!("{}/classes/{}.uc", package, class).to_lowercase();
        let path = self.files.get(&key)?.clone();
        let (number, message) = self.defaults_line(&path, message);
        Some(format!(
            "{}{}:{}: {}: {}",
            indent,
            discover::display(&path, &self.cwd),
            number,
            if is_error { "error" } else { "warning" },
            message
        ))
    }

    /// The file on this machine that UCC called `file`.
    fn resolve(&self, file: &str) -> Option<PathBuf> {
        if let Some(path) = tail_key(file).and_then(|key| self.files.get(&key)) {
            return Some(path.clone());
        }
        let dosdevices = self.dosdevices.as_deref()?;
        from_wine_path(dosdevices, file)
    }

    /// The line in `path` that a `defaultproperties` `message` is about, and
    /// the message to show.
    fn defaults_line(&mut self, path: &Path, message: &str) -> (usize, String) {
        let source = self
            .sources
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read(path).ok().map(|bytes| decode(&bytes)));
        let number = source
            .as_deref()
            .and_then(|text| find_defaults_line(text, message))
            .unwrap_or(1);
        (number, defaults_message(message))
    }
}

/// `<pkg>/classes/<class>.uc`, in lower case, from the end of `path`.
fn tail_key(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    let [.., package, classes, file] = parts.as_slice() else {
        return None;
    };
    (!package.is_empty() && classes.eq_ignore_ascii_case("classes"))
        .then(|| format!("{}/classes/{}", package, file).to_lowercase())
}

/// The folder of Wine's drive links, `$WINEPREFIX/dosdevices`, if there is
/// one: never on Windows, where paths are already right.
fn wine_dosdevices() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    let prefix = match std::env::var_os("WINEPREFIX") {
        Some(prefix) if !prefix.is_empty() => PathBuf::from(prefix),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".wine"),
    };
    Some(prefix.join("dosdevices"))
}

/// The file that the Windows path `file` (`Z:\Game\Foo.uc`) is under Wine,
/// whose drives are links in `dosdevices`, matching names in any case.
fn from_wine_path(dosdevices: &Path, file: &str) -> Option<PathBuf> {
    let mut parts = file.split('\\');
    let drive = parts.next()?;
    let is_drive =
        drive.len() == 2 && drive.ends_with(':') && drive.as_bytes()[0].is_ascii_alphabetic();
    if !is_drive {
        return None;
    }
    let mut path = dosdevices.join(drive.to_ascii_lowercase());
    for part in parts.filter(|p| !p.is_empty()) {
        path = find_ci(&path, part)?;
    }
    let path = path.canonicalize().ok()?;
    path.is_file().then_some(path)
}

/// A `defaultproperties` message, without the class's path that most start
/// with (`Pkg.Foo: `), since the file is shown.
fn defaults_message(message: &str) -> String {
    let message = message.trim_end();
    let message = match message.split_once(": ") {
        Some((class, rest)) if is_object_path(class) => rest,
        _ => message,
    };
    format!("{}: {}", DEFAULTPROPERTIES, message)
}

fn is_object_path(text: &str) -> bool {
    text.contains('.')
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// The 1-based line of `text`'s `defaultproperties` block that `message`
/// quotes: the whole line (`Unknown property in defaults: Bar=1`), or else
/// its value (`FloatProperty::ImportText failed in: abc`). Otherwise the
/// line of the `defaultproperties` keyword.
fn find_defaults_line(text: &str, message: &str) -> Option<usize> {
    let comments = comment_ranges(text);
    let in_comment = |offset: usize| comments.iter().any(|r| r.contains(&offset));
    let lines: Vec<(usize, &str)> = text
        .split('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .collect();
    let keyword = lines.iter().position(|&(start, line)| {
        let trimmed = line.trim_start();
        let offset = start + line.len() - trimmed.len();
        trimmed
            .get(..DEFAULTPROPERTIES.len())
            .is_some_and(|word| word.eq_ignore_ascii_case(DEFAULTPROPERTIES))
            && !trimmed[DEFAULTPROPERTIES.len()..]
                .starts_with(|c: char| c.is_alphanumeric() || c == '_')
            && !in_comment(offset)
    })?;
    let block = &lines[keyword + 1..];
    let normalize = |s: &str| s.trim().trim_end_matches(';').trim_end().to_string();
    let message = normalize(message);
    let value_of = |line: &str| line.split_once('=').map(|(_, value)| normalize(value));
    // The whole line, after a space or a colon: `...in defaults: X=1`.
    let quoting = block
        .iter()
        .enumerate()
        .filter(|&(_, &(_, line))| {
            let line = normalize(line);
            !line.is_empty()
                && message
                    .strip_suffix(line.as_str())
                    .is_some_and(|before| before.is_empty() || before.ends_with([' ', '\t', ':']))
        })
        .map(|(i, _)| i);
    let found = if message.starts_with("redundant data:") {
        // The importer reports the second of two lines that set a value.
        let matches: Vec<usize> = quoting.collect();
        matches.get(1).or(matches.first()).copied()
    } else {
        quoting.into_iter().next()
    };
    let found = found.or_else(|| {
        // A value in quotes: `...: unresolved reference to 'Texture'A.B''`.
        let (_, rest) = message
            .rsplit_once(" to '")
            .or_else(|| message.rsplit_once(" in '"))?;
        let needle = rest.strip_suffix('\'')?;
        (!needle.is_empty()).then_some(())?;
        block
            .iter()
            .position(|&(_, line)| value_of(line).is_some_and(|v| v.contains(needle)))
    });
    let found = found.or_else(|| {
        // A struct's member: `Unknown member X in Y`.
        let rest = message.strip_prefix("Unknown member ")?;
        let (member, _) = rest.split_once(' ')?;
        let member = format!("{}=", member.to_lowercase());
        block.iter().position(|&(_, line)| {
            value_of(line).is_some_and(|v| {
                v.to_lowercase()
                    .replace(' ', "")
                    .split(['(', ','])
                    .any(|part| part.starts_with(&member))
            })
        })
    });
    let found = found.or_else(|| {
        // The rest of a value: `FloatProperty::ImportText failed in: abc`.
        let (_, needle) = message.rsplit_once(": ")?;
        let needle = needle.trim();
        if needle.is_empty() {
            return None;
        }
        block
            .iter()
            .position(|&(_, line)| value_of(line).is_some_and(|v| v.contains(needle)))
    });
    let index = found.map_or(keyword, |i| keyword + 1 + i);
    Some(index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ucc_diagnostics() {
        assert_eq!(
            parse(r"Z:\Game (x86)\A\Classes\B.uc(12) : Error, Bad or missing expression in 'If'"),
            Some(UccDiagnostic {
                file: r"Z:\Game (x86)\A\Classes\B.uc",
                line: 12,
                in_defaults: false,
                is_error: true,
                message: "Bad or missing expression in 'If'",
            })
        );
        let defaults = parse(
            r"Z:\A\Classes\B.uc(1) : defaultproperties : ExecWarning, A.B: Missing '=' in default properties assignment: X 1",
        )
        .unwrap();
        assert!(defaults.in_defaults);
        assert!(!defaults.is_error);
        assert_eq!(
            defaults.message,
            "A.B: Missing '=' in default properties assignment: X 1"
        );
        assert_eq!(parse("Success - 0 error(s), 0 warning(s)"), None);
        assert_eq!(parse("B.uc : Error, no line"), None);
        assert_eq!(parse("B.uc(x) : Error, bad line"), None);
    }

    #[test]
    fn strips_the_class_from_defaults_messages() {
        assert_eq!(
            defaults_message("A.B: Unknown property in defaults: X=1"),
            "defaultproperties: Unknown property in defaults: X=1"
        );
        assert_eq!(
            defaults_message("FloatProperty::ImportText: Bad termination in: 1 "),
            "defaultproperties: FloatProperty::ImportText: Bad termination in: 1"
        );
        assert_eq!(
            defaults_message("ReadToken: Bad quoted string"),
            "defaultproperties: ReadToken: Bad quoted string"
        );
        assert_eq!(
            defaults_message("A..B: odd"),
            "defaultproperties: A..B: odd"
        );
    }

    const SOURCE: &str = "class B extends Object;\n\
                          // defaultproperties in a comment\n\
                          var float F;\n\
                          defaultproperties\n\
                          {\n\
                          \x20   X=1\n\
                          \x20   F=abc;\n\
                          }\n";

    #[test]
    fn finds_defaults_lines() {
        let line = |message| find_defaults_line(SOURCE, message);
        assert_eq!(line("A.B: Unknown property in defaults: X=1"), Some(6));
        assert_eq!(line("FloatProperty::ImportText failed in: abc"), Some(7));
        assert_eq!(line("Out of bound array default property (3/2)"), Some(4));
        assert_eq!(line("Bad parenthesised struct in: "), Some(4));
        assert_eq!(line("Unknown member Q in: zzz"), Some(4));
        assert_eq!(line("A.B: Unknown property in defaults: XX=1"), Some(4));
        assert_eq!(find_defaults_line("class B;\n", "x"), None);
        let source =
            "defaultproperties\n{\n    X=1\n    S=(A=1,B=2)\n    O=Texture'A.B'\n    X=1\n}\n";
        let line = |message| find_defaults_line(source, message);
        assert_eq!(line("redundant data:     X=1"), Some(6));
        assert_eq!(line("Unknown member B in S"), Some(4));
        assert_eq!(line("Unknown member Q in S"), Some(1));
        assert_eq!(line("Unknown member"), Some(1));
        assert_eq!(
            line("ObjectProperty A.C.O: unresolved reference to 'Texture'A.B''"),
            Some(5)
        );
        assert_eq!(
            line("ObjectProperty A.C.O: bad cast in 'Mesh'A.B''"),
            Some(1)
        );
        assert_eq!(line("ObjectProperty A.C.O: bad cast in ''"), Some(1));
        assert_eq!(line("x to 'unterminated"), Some(1));
        let single = "defaultproperties\n{\n    X=1\n}\n";
        assert_eq!(find_defaults_line(single, "redundant data: X=1"), Some(3));
        assert_eq!(find_defaults_line("defaultpropertiesX\n", "x"), None);
    }

    #[test]
    fn rewrites_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("A/Classes/B.uc");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SOURCE).unwrap();
        let files = [file];
        let mut rewriter = Rewriter::new(&files, dir.path().to_path_buf());
        rewriter.dosdevices = None;
        assert_eq!(
            rewriter.rewrite(r"Z:\Game\A\Classes\b.uc(3) : Error, Bad"),
            "A/Classes/B.uc:3: error: Bad"
        );
        assert_eq!(
            rewriter.rewrite(
                r"  Z:\Game\A\Classes\B.uc(1) : defaultproperties : Error, A.B: Unknown property in defaults: X=1"
            ),
            "  A/Classes/B.uc:6: error: defaultproperties: Unknown property in defaults: X=1"
        );
        assert_eq!(
            rewriter.rewrite(r"Z:\Game\Z\Classes\C.uc(2) : ScriptWarning, Odd"),
            r"Z:\Game\Z\Classes\C.uc:2: warning: Odd"
        );
        assert_eq!(
            rewriter.rewrite(
                r"Z:\Game\Z\Classes\C.uc(1) : defaultproperties : Warning, Z.C: redundant"
            ),
            r"Z:\Game\Z\Classes\C.uc:1: warning: defaultproperties: redundant"
        );
        assert_eq!(rewriter.rewrite("Compiling B"), "Compiling B");

        // A file that's gone keeps the line UCC gave.
        std::fs::remove_file(&files[0]).unwrap();
        let mut rewriter = Rewriter::new(&files, dir.path().to_path_buf());
        assert_eq!(
            rewriter.rewrite(r"Z:\A\Classes\B.uc(1) : defaultproperties : Error, A.B: x"),
            "A/Classes/B.uc:1: error: defaultproperties: x"
        );
    }

    #[test]
    fn rewrites_defaults_messages_without_a_context() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("A/Classes/B.uc");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SOURCE).unwrap();
        let files = [file];
        let mut rewriter = Rewriter::new(&files, dir.path().to_path_buf());
        let unknown = "A.B: Unknown property in defaults:     X=1";
        // Not while importing defaults.
        assert_eq!(rewriter.rewrite(unknown), unknown);
        rewriter.rewrite("-------------------A - Release-------------------");
        assert_eq!(rewriter.rewrite(unknown), unknown);
        rewriter.rewrite("Importing Defaults for B");
        assert_eq!(
            rewriter.rewrite(unknown),
            "A/Classes/B.uc:6: error: defaultproperties: Unknown property in defaults:     X=1"
        );
        assert_eq!(
            rewriter.rewrite("FloatProperty::ImportText failed in: abc"),
            "A/Classes/B.uc:7: error: defaultproperties: FloatProperty::ImportText failed in: abc"
        );
        assert_eq!(
            rewriter.rewrite("A.B: Missing '=' in default properties assignment: X 1"),
            "A/Classes/B.uc:4: warning: defaultproperties: Missing '=' in default properties assignment: X 1"
        );
        assert_eq!(
            rewriter.rewrite("Warning: redundant data: X=1"),
            "A/Classes/B.uc:6: warning: defaultproperties: redundant data: X=1"
        );
        assert_eq!(rewriter.rewrite("Something else"), "Something else");
        // A class whose file isn't known.
        rewriter.rewrite("Importing Defaults for C");
        assert_eq!(rewriter.rewrite(unknown), unknown);
        rewriter.rewrite("Compiling B");
        rewriter.rewrite("Importing Defaults for B");
        rewriter.rewrite("Success - 0 error(s), 0 warning(s)");
        assert_eq!(rewriter.rewrite(unknown), unknown);
    }

    #[test]
    fn knows_the_defaults_messages() {
        assert_eq!(
            defaults_severity("A.B: Delegate assignment failed: X=Y"),
            Some(true)
        );
        assert_eq!(
            defaults_severity("ReadToken: Bad quoted string"),
            Some(false)
        );
        assert_eq!(
            defaults_severity("ObjectProperty A.B.C: unresolved reference to 'X'"),
            Some(true)
        );
        assert_eq!(defaults_severity("Loading A.u"), None);
    }

    #[cfg(unix)]
    #[test]
    fn finds_files_through_wine_drives() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        std::fs::create_dir_all(game.join("Sub")).unwrap();
        std::fs::write(game.join("Sub/File.uc"), "").unwrap();
        let dosdevices = dir.path().join("dosdevices");
        std::fs::create_dir(&dosdevices).unwrap();
        std::os::unix::fs::symlink(&game, dosdevices.join("g:")).unwrap();
        let found = from_wine_path(&dosdevices, r"G:\SUB\file.uc").unwrap();
        assert_eq!(found, game.join("Sub/File.uc").canonicalize().unwrap());
        assert_eq!(from_wine_path(&dosdevices, r"G:\Sub"), None);
        assert_eq!(from_wine_path(&dosdevices, r"G:\Missing.uc"), None);
        assert_eq!(from_wine_path(&dosdevices, r"H:\Sub\File.uc"), None);
        assert_eq!(from_wine_path(&dosdevices, r"Sub\File.uc"), None);
        assert_eq!(from_wine_path(&dosdevices, r"1:\Sub\File.uc"), None);

        let mut rewriter = Rewriter::new(&[], dir.path().canonicalize().unwrap());
        rewriter.dosdevices = Some(dosdevices);
        assert_eq!(
            rewriter.rewrite(r"G:\Sub\File.uc(4) : Warning, Hm"),
            "Game/Sub/File.uc:4: warning: Hm"
        );
    }
}
