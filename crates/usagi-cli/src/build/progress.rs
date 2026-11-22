//! How `usagi build` reports its progress: plain lines, for pipes, editors
//! and CI, or live progress bars on a terminal.
//!
//! The live view follows UCC's output (see `Editor/Src/UMakeCommandlet.cpp`
//! and `UnScrCom.cpp`): `MakeCommandlet` logs a heading, `<Pkg> - Release`
//! between dashes, for each of the `EditPackages`, then, for a package it
//! compiles, `Parsing <Class>` and `Compiling <Class>` for each class and
//! `Importing Defaults for <Class>` (followed by `... deferring` when it
//! retries a class later). Class names are the `.uc` files' names, so each
//! stage's length is known before it starts.

use std::io::{self, IsTerminal, Write};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use console::{Style, style};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};

use super::ucc::{DumpResult, is_noise, line_color};
use super::{Package, print_header};
use crate::cli::ProgressChoice;

/// Ticks a progress bar from any thread, if there is one.
#[derive(Default)]
pub struct Ticker(Option<ProgressBar>);

impl Ticker {
    pub fn tick(&self) {
        if let Some(bar) = &self.0 {
            bar.inc(1);
        }
    }
}

/// What a build reports, in the order it happens.
pub trait Reporter {
    /// Before finding the out-of-date packages.
    fn finding(&mut self) {}
    /// The packages that will be compiled, before their sources are checked.
    fn plan(&mut self, _label: &str, _packages: &[&Package], _up_to_date: usize) {}
    fn check_started(&mut self, _files: usize) -> Ticker {
        Ticker::default()
    }
    /// After checking the sources; problems are printed next, if any.
    fn check_finished(&mut self, _failed: usize, _files: usize) {}
    /// Just before UCC compiles the packages.
    fn compile_started(&mut self) {}
    fn ucc_line(&mut self, line: &str);
    /// After UCC exits, with the packages it compiled.
    fn compile_finished(&mut self, _compiled: &[&Package]) {}
    fn dumpint_started(&mut self, _packages: usize) -> Ticker {
        Ticker::default()
    }
    fn dumpint_finished(&mut self, results: &[DumpResult]);
    /// The end of the build. `ucc_failed` is UCC's status if it failed and
    /// some package wasn't compiled.
    fn finish(
        &mut self,
        succeeded: usize,
        failed: usize,
        up_to_date: usize,
        ucc_failed: Option<ExitStatus>,
    );
}

/// The reporter for `choice`: live progress on a terminal, or with
/// `always`.
pub fn reporter(choice: ProgressChoice, verbose: bool, color: bool) -> Box<dyn Reporter> {
    let terminal =
        io::stdout().is_terminal() && std::env::var("TERM").map_or(true, |t| t != "dumb");
    let target = match choice {
        ProgressChoice::Never => None,
        ProgressChoice::Auto if !terminal => None,
        _ if terminal => Some(ProgressDrawTarget::stdout()),
        _ => Some(ProgressDrawTarget::term_like(Box::new(AnsiStdout))),
    };
    match target {
        None => Box::new(Plain {
            verbose,
            color,
            plan: Vec::new(),
        }),
        Some(target) => {
            console::set_colors_enabled(color);
            Box::new(Live::new(target, verbose, wants_unicode()))
        }
    }
}

/// Whether the terminal can show the Unicode glyphs: with a UTF-8 locale,
/// or on Windows, in Windows Terminal or VS Code (not the old console).
fn wants_unicode() -> bool {
    if cfg!(windows) {
        return std::env::var_os("WT_SESSION").is_some()
            || std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "vscode");
    }
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .is_some_and(|locale| {
            let locale = locale.to_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        })
}

/// Prints `line` of UCC's output, in colour if it's an error or warning.
fn print_ucc_line(line: &str, color: bool) {
    match line_color(line).filter(|_| color) {
        Some(code) => println!("\x1b[{}m{}\x1b[0m", code, line),
        None => println!("{}", line),
    }
}

/// Lines, as `usagi build` printed them before it had a live view.
struct Plain {
    verbose: bool,
    color: bool,
    /// The heading and the packages, printed when compiling starts.
    plan: Vec<String>,
}

impl Reporter for Plain {
    fn finding(&mut self) {
        println!("Checking for out-of-date packages...");
    }

    fn plan(&mut self, label: &str, packages: &[&Package], _up_to_date: usize) {
        self.plan = std::iter::once(format!(" Build started for {} ", label))
            .chain(
                packages
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.status.describe())),
            )
            .collect();
    }

    fn compile_started(&mut self) {
        let mut lines = self.plan.drain(..);
        if let Some(heading) = lines.next() {
            print_header(&heading, None);
        }
        for line in lines {
            println!("{}", line);
        }
    }

    fn ucc_line(&mut self, line: &str) {
        if self.verbose || !is_noise(line) {
            print_ucc_line(line, self.color);
        }
    }

    fn dumpint_finished(&mut self, results: &[DumpResult]) {
        for result in results.iter().filter(|r| !r.ok || self.verbose) {
            for line in &result.lines {
                print_ucc_line(line, self.color);
            }
        }
        let failed: Vec<&str> = results
            .iter()
            .filter(|r| !r.ok)
            .map(|r| r.package.as_str())
            .collect();
        println!(
            "Exported localization: {} succeeded, {} failed{}",
            results.len() - failed.len(),
            failed.len(),
            if failed.is_empty() {
                String::new()
            } else {
                format!(" ({})", failed.join(", "))
            }
        );
    }

    fn finish(
        &mut self,
        succeeded: usize,
        failed: usize,
        up_to_date: usize,
        ucc_failed: Option<ExitStatus>,
    ) {
        if let Some(status) = ucc_failed {
            println!("{}", describe_exit(status));
        }
        let text = format!(
            " Build: {} succeeded, {} failed, 0 skipped, {} up-to-date ",
            succeeded, failed, up_to_date
        );
        let code = if failed == 0 { "92" } else { "91" };
        print_header(&text, self.color.then_some(code));
    }
}

fn describe_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("UCC exited with status {}", code),
        None => "UCC was terminated".to_string(),
    }
}

/// The characters the live view draws with.
struct Glyphs {
    ok: &'static str,
    failed: &'static str,
    waiting: &'static str,
    warning: &'static str,
    rule: &'static str,
    spinner: &'static [&'static str],
    bar: &'static str,
}

const UNICODE: Glyphs = Glyphs {
    ok: "✓",
    failed: "✗",
    waiting: "·",
    warning: "⚠",
    rule: "─",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "],
    bar: "━╸━",
};

const ASCII: Glyphs = Glyphs {
    ok: "+",
    failed: "x",
    waiting: "-",
    warning: "!",
    rule: "-",
    spinner: &["|", "/", "-", "\\", " "],
    bar: "=> ",
};

/// How many waiting packages are listed; the rest are counted.
const MAX_WAITING: usize = 4;
const BAR_WIDTH: usize = 32;
/// The narrowest the "N classes" column is: as wide as "not compiled".
const MIN_COUNT_WIDTH: usize = 12;
/// The narrowest the names column is: as wide as "Checked sources".
const MIN_NAME_WIDTH: usize = 15;
const TICK: Duration = Duration::from_millis(80);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Analyzing,
    Parsing,
    Compiling,
    Defaults,
}

impl Stage {
    fn describe(self) -> &'static str {
        match self {
            Stage::Analyzing => "analyzing",
            Stage::Parsing => "parsing",
            Stage::Compiling => "compiling",
            Stage::Defaults => "importing defaults",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowState {
    Waiting,
    Active,
    Done,
}

/// A package being compiled.
struct Row {
    name: String,
    classes: u64,
    state: RowState,
    stage: Stage,
    /// How many classes each stage has done.
    parsed: u64,
    compiled: u64,
    defaults: u64,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    errors: usize,
    warnings: usize,
    /// UCC's errors and warnings, shown at the end.
    diagnostics: Vec<String>,
}

impl Row {
    /// How far the stage has got, at most the number of classes.
    fn stage_done(&self) -> u64 {
        let done = match self.stage {
            Stage::Analyzing => 0,
            Stage::Parsing => self.parsed,
            Stage::Compiling => self.compiled,
            Stage::Defaults => self.defaults,
        };
        done.min(self.classes)
    }

    /// Progress through parsing and compiling, out of twice the classes.
    fn overall(&self) -> u64 {
        if self.state == RowState::Done {
            return 2 * self.classes;
        }
        self.parsed.min(self.classes) + self.compiled.min(self.classes)
    }
}

/// Progress bars on a terminal: a row for each package, with the one being
/// compiled animated, the next few waiting, and a bar for the whole build.
/// Finished steps are printed above them as lines that stay. UCC's errors
/// and warnings are collected and printed at the end.
struct Live {
    multi: MultiProgress,
    glyphs: &'static Glyphs,
    verbose: bool,
    started: Instant,
    width: usize,
    /// The width of the counts column ("N classes", "N files", ...).
    count_width: usize,
    rows: Vec<Row>,
    current: Option<usize>,
    /// The step in progress: checking, a package, or localization.
    active: Option<ProgressBar>,
    /// The packages waiting to be compiled.
    waiting: Option<ProgressBar>,
    overall: Option<ProgressBar>,
    /// UCC's errors and warnings before any package's heading.
    diagnostics: Vec<String>,
    /// UCC's other output, shown at the end if it failed.
    other: Vec<String>,
    /// The output of localization exports that failed.
    dumpint_failures: Vec<DumpResult>,
}

impl Live {
    fn new(target: ProgressDrawTarget, verbose: bool, unicode: bool) -> Self {
        Live {
            multi: MultiProgress::with_draw_target(target),
            glyphs: if unicode { &UNICODE } else { &ASCII },
            verbose,
            started: Instant::now(),
            width: 0,
            count_width: MIN_COUNT_WIDTH,
            rows: Vec::new(),
            current: None,
            active: None,
            waiting: None,
            overall: None,
            diagnostics: Vec::new(),
            other: Vec::new(),
            dumpint_failures: Vec::new(),
        }
    }

    fn println(&self, line: impl AsRef<str>) {
        // Only fails if stdout is closed, and then nothing can be shown.
        let _ = self.multi.println(line);
    }

    fn style(&self, template: &str) -> ProgressStyle {
        ProgressStyle::with_template(template)
            .expect("a valid template")
            .tick_strings(self.glyphs.spinner)
            .progress_chars(self.glyphs.bar)
    }

    /// A new animated bar for the step in progress, above the waiting list.
    fn start_active(&mut self, template: &str, len: u64) -> ProgressBar {
        let bar = ProgressBar::new(len).with_style(self.style(template));
        let bar = match self.waiting.as_ref().or(self.overall.as_ref()) {
            Some(below) => self.multi.insert_before(below, bar),
            None => self.multi.add(bar),
        };
        bar.enable_steady_tick(TICK);
        self.active = Some(bar.clone());
        bar
    }

    fn end_active(&mut self, line: String) {
        if let Some(bar) = self.active.take() {
            bar.finish_and_clear();
            self.multi.remove(&bar);
        }
        self.println(line);
    }

    fn ok(&self) -> String {
        style(self.glyphs.ok).green().bold().to_string()
    }

    fn failed(&self) -> String {
        style(self.glyphs.failed).red().bold().to_string()
    }

    fn update_waiting(&mut self) {
        let waiting: Vec<&Row> = self
            .rows
            .iter()
            .filter(|r| r.state == RowState::Waiting)
            .collect();
        let Some(bar) = &self.waiting else {
            return;
        };
        if waiting.is_empty() {
            bar.finish_and_clear();
            self.multi.remove(bar);
            self.waiting = None;
            return;
        }
        let dim = Style::new().dim();
        let mut lines: Vec<String> = waiting
            .iter()
            .take(MAX_WAITING)
            .map(|row| {
                format!(
                    " {} {:w$}  {}",
                    dim.apply_to(self.glyphs.waiting),
                    row.name,
                    dim.apply_to(format!("waiting   {}", classes(row.classes))),
                    w = self.width
                )
            })
            .collect();
        if waiting.len() > MAX_WAITING {
            lines.push(
                dim.apply_to(format!("   and {} more", waiting.len() - MAX_WAITING))
                    .to_string(),
            );
        }
        bar.set_message(lines.join("\n"));
    }

    fn update_overall(&self) {
        if let Some(bar) = &self.overall {
            bar.set_position(self.rows.iter().map(Row::overall).sum());
        }
    }

    fn update_current(&self) {
        let (Some(index), Some(bar)) = (self.current, &self.active) else {
            return;
        };
        let row = &self.rows[index];
        bar.set_prefix(format!(
            "{:w$}  {}",
            style(&row.name).bold(),
            style(row.stage.describe()).cyan(),
            w = self.width
        ));
        bar.set_length(row.classes.max(1));
        bar.set_position(row.stage_done());
        self.update_overall();
    }

    /// Starts compiling the package called `name`, if it's one of the rows.
    fn begin_row(&mut self, name: &str) {
        let Some(index) = self
            .rows
            .iter()
            .position(|r| r.state == RowState::Waiting && r.name.eq_ignore_ascii_case(name))
        else {
            return;
        };
        let template = format!(
            " {{spinner:.cyan}} {{prefix}}\n   {{bar:{}.cyan/dim}} {{pos:>3}}/{{len}}  {{msg:.dim}}",
            BAR_WIDTH
        );
        self.start_active(&template, 1);
        let row = &mut self.rows[index];
        row.state = RowState::Active;
        row.stage = Stage::Analyzing;
        row.started = Some(Instant::now());
        self.current = Some(index);
        self.update_current();
        self.update_waiting();
    }

    /// Finishes the package being compiled, printing whether it compiled.
    fn end_row(&mut self, compiled: bool) {
        let Some(index) = self.current.take() else {
            return;
        };
        let row = &mut self.rows[index];
        row.state = RowState::Done;
        row.elapsed = row.started.map(|s| s.elapsed());
        let line = self.row_line(index, compiled);
        self.end_active(line);
        self.update_overall();
    }

    fn row_line(&self, index: usize, compiled: bool) -> String {
        let row = &self.rows[index];
        let time = row
            .elapsed
            .map(|e| format!("  {}", style(format!("{:>6}", seconds(e))).dim()))
            .unwrap_or_default();
        let warnings = if row.warnings > 0 {
            format!(
                "  {}",
                style(format!(
                    "{} {}",
                    self.glyphs.warning,
                    count(row.warnings, "warning")
                ))
                .yellow()
            )
        } else {
            String::new()
        };
        if compiled {
            return format!(
                " {} {:w$}  {:<c$}{}{}",
                self.ok(),
                row.name,
                classes(row.classes),
                time,
                warnings,
                w = self.width,
                c = self.count_width,
            );
        }
        let why = if row.errors > 0 {
            count(row.errors, "error")
        } else {
            "not compiled".to_string()
        };
        format!(
            " {} {:w$}  {}{}{}",
            self.failed(),
            style(&row.name).red(),
            style(format!("{:<c$}", why, c = self.count_width)).red(),
            time,
            warnings,
            w = self.width
        )
    }

    /// Clears the bars still showing.
    fn clear(&mut self) {
        for bar in [self.active.take(), self.waiting.take(), self.overall.take()]
            .into_iter()
            .flatten()
        {
            bar.finish_and_clear();
            self.multi.remove(&bar);
        }
    }

    fn print_diagnostics(&self, title: &str, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let rule = self.glyphs.rule;
        println!();
        println!(
            " {}",
            style(format!(
                "{}{} {} {}",
                rule,
                rule,
                title,
                rule.repeat(56usize.saturating_sub(title.chars().count()))
            ))
            .bold()
        );
        for line in lines {
            let styled = match line_color(line) {
                Some("91") => style(line).red().to_string(),
                Some(_) => style(line).yellow().to_string(),
                None => line.clone(),
            };
            println!(" {}", styled);
        }
    }
}

/// A UCC heading's package: `Pkg` from `---- Pkg - Release ----`.
fn heading(line: &str) -> Option<&str> {
    let text = line.trim().strip_prefix("---")?.trim_matches('-').trim();
    let (package, kind) = text.rsplit_once(" - ")?;
    matches!(kind, "Release" | "Debug").then_some(package.trim())
}

fn classes(n: u64) -> String {
    count(n as usize, "class").replace("classs", "classes")
}

fn count(n: usize, what: &str) -> String {
    format!("{} {}{}", n, what, if n == 1 { "" } else { "s" })
}

fn seconds(duration: Duration) -> String {
    format!("{:.1}s", duration.as_secs_f64())
}

impl Reporter for Live {
    fn plan(&mut self, label: &str, packages: &[&Package], up_to_date: usize) {
        self.width = packages
            .iter()
            .map(|p| p.name.chars().count())
            .max()
            .unwrap_or(0)
            .max(MIN_NAME_WIDTH);
        // Every count shown in the column: each package's classes, the
        // files checked (all of them) and the packages exported.
        let files: usize = packages.iter().map(|p| p.files.len()).sum();
        self.count_width = packages
            .iter()
            .map(|p| classes(p.files.len() as u64))
            .chain([count(files, "file"), count(packages.len(), "package")])
            .map(|text| text.chars().count())
            .max()
            .unwrap_or(0)
            .max(MIN_COUNT_WIDTH);
        self.rows = packages
            .iter()
            .map(|p| Row {
                name: p.name.clone(),
                classes: p.files.len() as u64,
                state: RowState::Waiting,
                stage: Stage::Analyzing,
                parsed: 0,
                compiled: 0,
                defaults: 0,
                started: None,
                elapsed: None,
                errors: 0,
                warnings: 0,
                diagnostics: Vec::new(),
            })
            .collect();
        let total = packages.len() + up_to_date;
        self.println(format!(
            "\n {} {}  {}\n",
            style("usagi build").bold().cyan(),
            style(label).bold(),
            style(format!(
                "({} of {} {} out of date)",
                packages.len(),
                total,
                if total == 1 { "package" } else { "packages" }
            ))
            .dim()
        ));
        let waiting = self
            .multi
            .add(ProgressBar::new(0).with_style(self.style("{msg}")));
        self.waiting = Some(waiting);
        self.update_waiting();
        let classes: u64 = self.rows.iter().map(|r| r.classes).sum();
        let template = format!(
            "\n   {{prefix:.bold}} {{bar:{}.green/dim}} {{percent:>3}}%  {{elapsed:.dim}}",
            BAR_WIDTH
        );
        let overall = self
            .multi
            .add(ProgressBar::new(2 * classes.max(1)).with_style(self.style(&template)))
            .with_prefix("Overall");
        overall.enable_steady_tick(TICK);
        self.overall = Some(overall);
    }

    fn check_started(&mut self, files: usize) -> Ticker {
        let template = format!(
            " {{spinner:.cyan}} {{prefix:.bold}}\n   {{bar:{}.cyan/dim}} {{pos:>3}}/{{len}} files",
            BAR_WIDTH
        );
        let bar = self.start_active(&template, files as u64);
        bar.set_prefix("Checking sources");
        Ticker(Some(bar))
    }

    fn check_finished(&mut self, failed: usize, files: usize) {
        let elapsed = self
            .active
            .as_ref()
            .map(|b| b.elapsed())
            .unwrap_or_default();
        let line = if failed == 0 {
            format!(
                " {} {:w$}  {:<c$}  {}",
                self.ok(),
                "Checked sources",
                count(files, "file"),
                style(format!("{:>6}", seconds(elapsed))).dim(),
                w = self.width,
                c = self.count_width,
            )
        } else {
            format!(
                " {} {:w$}  {}",
                self.failed(),
                style("Checked sources").red(),
                style(format!(
                    "{} of {} didn't parse",
                    failed,
                    count(files, "file")
                ))
                .red(),
                w = self.width
            )
        };
        self.end_active(line);
        if failed > 0 {
            self.clear();
            println!();
        }
    }

    fn ucc_line(&mut self, line: &str) {
        if self.verbose && !line.trim().is_empty() {
            let styled = match line_color(line) {
                Some("91") => style(line).red().to_string(),
                Some(_) => style(line).yellow().to_string(),
                None => style(line).dim().to_string(),
            };
            self.println(styled);
        }
        if let Some(package) = heading(line) {
            // UCC stops at a package that fails, so the last one compiled,
            // unless it reported errors.
            let ok = self.current.is_some_and(|i| self.rows[i].errors == 0);
            self.end_row(ok);
            self.begin_row(package);
            return;
        }
        let trimmed = line.trim();
        // UCC's own summary line isn't a problem of the package.
        let summary = trimmed.starts_with("Failure");
        if let Some(color) = line_color(line).filter(|_| !summary) {
            let Some(index) = self.current else {
                self.diagnostics.push(trimmed.to_string());
                return;
            };
            let row = &mut self.rows[index];
            row.diagnostics.push(trimmed.to_string());
            match color {
                "91" => row.errors += 1,
                _ => row.warnings += 1,
            }
            return;
        }
        let Some(index) = self.current else {
            if !is_noise(line) {
                self.other.push(trimmed.to_string());
            }
            return;
        };
        let row = &mut self.rows[index];
        let class = if trimmed == "Analyzing..." {
            row.stage = Stage::Analyzing;
            None
        } else if let Some(class) = trimmed.strip_prefix("Parsing ") {
            row.stage = Stage::Parsing;
            row.parsed += 1;
            Some(class)
        } else if let Some(class) = trimmed.strip_prefix("Compiling ") {
            row.stage = Stage::Compiling;
            row.compiled += 1;
            Some(class)
        } else if let Some(class) = trimmed.strip_prefix("Importing Defaults for ") {
            row.stage = Stage::Defaults;
            row.defaults += 1;
            Some(class)
        } else if trimmed == "... deferring" {
            // The class's defaults will be imported again.
            row.defaults = row.defaults.saturating_sub(1);
            None
        } else {
            if !is_noise(line) {
                self.other.push(trimmed.to_string());
            }
            None
        };
        if let (Some(class), Some(bar)) = (class, &self.active) {
            bar.set_message(class.to_string());
        }
        self.update_current();
    }

    fn compile_finished(&mut self, compiled: &[&Package]) {
        let is_compiled = |name: &str| compiled.iter().any(|p| p.name.eq_ignore_ascii_case(name));
        if let Some(index) = self.current {
            let ok = is_compiled(&self.rows[index].name);
            self.end_row(ok);
        }
        // Packages UCC never got to, or whose heading wasn't seen.
        for index in 0..self.rows.len() {
            if self.rows[index].state == RowState::Waiting {
                self.rows[index].state = RowState::Done;
                let line = self.row_line(index, is_compiled(&self.rows[index].name));
                self.println(line);
            }
        }
        self.update_waiting();
        self.update_overall();
    }

    fn dumpint_started(&mut self, packages: usize) -> Ticker {
        let template = format!(
            " {{spinner:.cyan}} {{prefix:.bold}}\n   {{bar:{}.cyan/dim}} {{pos:>3}}/{{len}} packages",
            BAR_WIDTH
        );
        let bar = self.start_active(&template, packages as u64);
        bar.set_prefix("Exporting localization");
        Ticker(Some(bar))
    }

    fn dumpint_finished(&mut self, results: &[DumpResult]) {
        let elapsed = self
            .active
            .as_ref()
            .map(|b| b.elapsed())
            .unwrap_or_default();
        let failed: Vec<&str> = results
            .iter()
            .filter(|r| !r.ok)
            .map(|r| r.package.as_str())
            .collect();
        let line = if failed.is_empty() {
            format!(
                " {} {:w$}  {:<c$}  {}",
                self.ok(),
                "Localization",
                count(results.len(), "package"),
                style(format!("{:>6}", seconds(elapsed))).dim(),
                w = self.width,
                c = self.count_width,
            )
        } else {
            format!(
                " {} {:w$}  {}",
                self.failed(),
                style("Localization").red(),
                style(format!(
                    "{} of {} failed ({})",
                    failed.len(),
                    results.len(),
                    failed.join(", ")
                ))
                .red(),
                w = self.width
            )
        };
        self.end_active(line);
        self.dumpint_failures
            .extend(results.iter().filter(|r| !r.ok).cloned());
    }

    fn finish(
        &mut self,
        succeeded: usize,
        failed: usize,
        up_to_date: usize,
        ucc_failed: Option<ExitStatus>,
    ) {
        self.clear();
        self.print_diagnostics("UCC", &self.diagnostics);
        for row in &self.rows {
            self.print_diagnostics(&row.name, &row.diagnostics);
        }
        for result in &self.dumpint_failures {
            self.print_diagnostics(&format!("Localization: {}", result.package), &result.lines);
        }
        if let Some(status) = ucc_failed {
            self.print_diagnostics("UCC output", &self.other);
            println!("\n {}", style(describe_exit(status)).red());
        }
        let mut parts = Vec::new();
        if succeeded > 0 || failed == 0 {
            parts.push(format!("{} compiled", succeeded));
        }
        if failed > 0 {
            parts.push(style(format!("{} failed", failed)).red().to_string());
        }
        parts.push(format!("{} up to date", up_to_date));
        let summary = parts.join(&style(" · ").dim().to_string());
        let time = style(seconds(self.started.elapsed())).dim();
        if failed == 0 {
            println!(
                "\n {} {}  {}  {}\n",
                self.ok(),
                style("Build succeeded").green().bold(),
                summary,
                time
            );
        } else {
            println!(
                "\n {} {}  {}  {}\n",
                self.failed(),
                style("Build failed").red().bold(),
                summary,
                time
            );
        }
    }
}

/// Standard output as a terminal, for `--progress always` when it isn't one.
#[derive(Debug)]
struct AnsiStdout;

impl AnsiStdout {
    fn write(&self, s: &str) -> io::Result<()> {
        io::stdout().lock().write_all(s.as_bytes())
    }
}

impl TermLike for AnsiStdout {
    fn width(&self) -> u16 {
        80
    }

    fn move_cursor_up(&self, n: usize) -> io::Result<()> {
        self.write(&format!("\x1b[{}A", n))
    }

    fn move_cursor_down(&self, n: usize) -> io::Result<()> {
        self.write(&format!("\x1b[{}B", n))
    }

    fn move_cursor_right(&self, n: usize) -> io::Result<()> {
        self.write(&format!("\x1b[{}C", n))
    }

    fn move_cursor_left(&self, n: usize) -> io::Result<()> {
        self.write(&format!("\x1b[{}D", n))
    }

    fn write_line(&self, s: &str) -> io::Result<()> {
        self.write(&format!("{}\n", s))
    }

    fn write_str(&self, s: &str) -> io::Result<()> {
        self.write(s)
    }

    fn clear_line(&self) -> io::Result<()> {
        self.write("\r\x1b[2K")
    }

    fn flush(&self) -> io::Result<()> {
        io::stdout().flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headings() {
        let line = format!("{}MyPkg - Release{}", "-".repeat(27), "-".repeat(28));
        assert_eq!(heading(&line), Some("MyPkg"));
        assert_eq!(heading("------Core - Debug------\r"), Some("Core"));
        assert_eq!(heading("Parsing Foo"), None);
        assert_eq!(heading("------------ Not a heading ------"), None);
    }

    #[test]
    fn counts() {
        assert_eq!(classes(1), "1 class");
        assert_eq!(classes(3), "3 classes");
        assert_eq!(count(2, "error"), "2 errors");
    }
}
