//! `usagi`: parse and lint Unreal Engine 2 UnrealScript, and build mods.

mod build;
mod check;
mod cli;
mod config;
mod discover;
mod format;
mod graph;
mod output;
mod pkg;
mod project;
mod suppress;

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use usagi_core::{Applicability, Lint, ProgramErrorSeverity};

use check::{CheckOptions, FileReport};
use cli::{Cli, ColorChoice, Command, FileArgs, LintCommand, OutputFormat, PkgCommand, RuleArgs};
use config::{ConfigError, Overrides, Settings};

fn main() -> ExitCode {
    restore_sigpipe();
    let cli = Cli::parse();
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("usagi: {}", message);
            ExitCode::from(2)
        }
    }
}

/// Runs a command. Returns whether it found nothing to report.
fn run(cli: Cli) -> Result<bool, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let color = match cli.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    let load_root =
        || config::load(cli.config.as_deref(), cli.no_config, &cwd).map_err(|ConfigError(e)| e);
    let load = || config::with_default_mod(load_root()?).map_err(|ConfigError(e)| e);
    match &cli.command {
        Command::Parse(args) => {
            let config = load()?;
            let settings = settings(&config, &args.files, cli.jobs, Overrides::default(), &cwd)?;
            let reports = check_files(
                &args.files.paths,
                &settings,
                Checking::Parse { ast: args.ast },
            )?;
            let mut out = io::stdout().lock();
            if args.ast {
                for report in &reports {
                    if let Some(ast) = &report.ast {
                        writeln!(out, "{}\n{}", discover::display(&report.path, &cwd), ast)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            output::problems(&mut out, &reports, args.format, &cwd, color)
                .map_err(|e| e.to_string())?;
            if args.statistics {
                output::outcome_statistics(&mut out, &reports).map_err(|e| e.to_string())?;
            }
            Ok(!reports.iter().any(|r| r.outcome.is_failure()))
        }
        Command::Lint(args) => {
            if let Some(LintCommand::Rule(args)) = &args.command {
                return rule(args);
            }
            let config = load()?;
            let overrides = Overrides {
                select: args.select.clone(),
                extend_select: args.extend_select.clone(),
                ignore: args.ignore.clone(),
                ..Overrides::default()
            };
            let settings = settings(&config, &args.files, cli.jobs, overrides, &cwd)?;
            if let Some(style) = args.add_ignore {
                let (reports, _) = lint_files(args, &settings, false)?;
                let style = match style {
                    cli::IgnoreStyle::Name => suppress::Style::Name,
                    cli::IgnoreStyle::Code => suppress::Style::Code,
                };
                let (comments, files) = add_ignores(&reports, style, &cwd)?;
                eprintln!(
                    "Added suppression comments for {} {} in {} {}",
                    comments,
                    plural(comments, "line"),
                    files,
                    plural(files, "file")
                );
                return Ok(true);
            }
            let fixing = args.fix || args.diff;
            let (reports, project) = lint_files(args, &settings, fixing)?;
            let mut out = io::stdout().lock();
            let fixed: usize = reports.iter().map(|r| r.fixed).sum();
            let fixed_files = reports.iter().filter(|r| r.fixed > 0).count();
            if args.diff {
                output::diffs(&mut out, &reports, &cwd, color).map_err(|e| e.to_string())?;
                eprintln!(
                    "Would fix {} {} in {} {}",
                    fixed,
                    plural(fixed, "problem"),
                    fixed_files,
                    plural(fixed_files, "file")
                );
                return Ok(fixed == 0);
            }
            if args.fix {
                for report in &reports {
                    if let Some(bytes) = &report.fixed_bytes {
                        std::fs::write(&report.path, bytes).map_err(|e| {
                            format!("{}: {}", discover::display(&report.path, &cwd), e)
                        })?;
                    }
                }
            }
            output::problems(&mut out, &reports, args.format, &cwd, color)
                .map_err(|e| e.to_string())?;
            if args.fix {
                eprintln!(
                    "Fixed {} {} in {} {}",
                    fixed,
                    plural(fixed, "problem"),
                    fixed_files,
                    plural(fixed_files, "file")
                );
            } else {
                let fixable = reports
                    .iter()
                    .flat_map(|r| &r.problems)
                    .filter(|p| matches!(&p.fix, Some((_, Applicability::Safe))))
                    .count();
                if fixable > 0 && !matches!(args.format, OutputFormat::Json | OutputFormat::Tsv) {
                    eprintln!(
                        "{} {} can be fixed with `usagi lint --fix` (preview with `--diff`)",
                        fixable,
                        plural(fixable, "problem")
                    );
                }
            }
            if args.statistics {
                output::lint_statistics(&mut out, &reports).map_err(|e| e.to_string())?;
                if let Some(project) = &project {
                    project::statistics(&mut out, project, &cwd).map_err(|e| e.to_string())?;
                }
            }
            Ok(!reports.iter().any(|r| {
                r.outcome.is_failure()
                    || r.problems
                        .iter()
                        .any(|p| matches!(p.kind, check::Kind::Lint(_)))
            }))
        }
        Command::Graph(args) => {
            let paths = paths_or_cwd(&args.paths);
            let (files, errors) = discover::all(&paths);
            if let Some(error) = errors.first() {
                return Err(error.clone());
            }
            let jobs = cli.jobs.map_or_else(config::default_jobs, usize::from);
            let tables = check::index_all(&files, jobs)?;
            let project = project::build(&mut [], tables, &paths);
            let options = graph::Options {
                classes: args.class.clone(),
                ancestors: args.ancestors,
                descendants: args.descendants,
                depth: args.depth,
                packages: args.packages,
            };
            let graph = graph::build(&project, &options, &|path| discover::display(path, &cwd))?;
            graph::write(&mut io::stdout().lock(), &graph, args.format)
                .map_err(|e| e.to_string())?;
            Ok(true)
        }
        Command::Format(args) => {
            let config = load()?;
            let options = config::format_options(&config).map_err(|ConfigError(e)| e)?;
            if args.paths.len() == 1 && args.paths[0].as_os_str() == "-" {
                return format::stdin(&options);
            }
            let files = FileArgs {
                paths: args.paths.clone(),
                include: args.include.clone(),
                exclude: args.exclude.clone(),
                strict: false,
                no_strict: false,
            };
            let settings = settings(&config, &files, cli.jobs, Overrides::default(), &cwd)?;
            let default = [PathBuf::from(".")];
            let paths = if files.paths.is_empty() {
                &default[..]
            } else {
                &files.paths[..]
            };
            let (found, errors) = discover::files(paths, &settings);
            if let Some(error) = errors.first() {
                return Err(error.clone());
            }
            format::files(&found, &options, args.check, settings.jobs, &cwd)
        }
        Command::Pkg(args) => match &args.command {
            PkgCommand::List(args) => {
                pkg::list(args, color, &mut io::stdout().lock())?;
                Ok(true)
            }
            PkgCommand::Flags(args) => {
                pkg::flags(args, color, &mut io::stdout().lock())?;
                Ok(true)
            }
            PkgCommand::Obj(args) => {
                pkg::obj::obj(args, color, &mut io::stdout().lock(), &mut io::stderr())
            }
            PkgCommand::Export(args) => {
                let threads = cli.jobs.map_or_else(config::default_jobs, usize::from);
                pkg::export::export(args, threads, &mut io::stdout().lock(), &mut io::stderr())
            }
            PkgCommand::Diff(args) => {
                pkg::diff::diff(args, color, &mut io::stdout().lock(), &mut io::stderr())
            }
        },
        Command::Build(args) => {
            let config = load_root()?;
            let mod_name = if args.no_mod {
                None
            } else {
                args.mod_name
                    .clone()
                    .or_else(|| config.file.build.mod_name.clone())
            };
            config::check_root_build(&config, mod_name.is_some()).map_err(|ConfigError(e)| e)?;
            let root = match &args.root {
                Some(root) => std::path::absolute(cwd.join(root)).unwrap_or(cwd.join(root)),
                None => config::game_root(&config),
            };
            let config = match &mod_name {
                Some(mod_name) => {
                    config::merge_mod(config, &root, mod_name).map_err(|ConfigError(e)| e)?
                }
                None => config,
            };
            let build = &config.file.build;
            build::run(&build::Options {
                root,
                mod_name,
                ini: build.ini.clone(),
                locked: build.locked.clone(),
                clean: args.clean,
                dumpint: args.dumpint,
                debug: args.debug,
                ignore_dependencies: args.ignore_dependencies,
                check: !args.no_check,
                strict: config.file.strict.unwrap_or(false),
                jobs: cli
                    .jobs
                    .or(config.file.jobs.map(|n| n.get()))
                    .map_or_else(config::default_jobs, usize::from),
                verbose: args.verbose,
                color,
                progress: args.progress.or(build.progress).unwrap_or_default(),
                wine: build.wine.clone().unwrap_or_else(|| "wine".to_string()),
                version: build.version.clone(),
                version_class: build.version_class.clone(),
            })
        }
    }
}

/// `usagi lint rule`: explains a lint, or lists every lint.
fn rule(args: &RuleArgs) -> Result<bool, String> {
    let mut out = io::stdout().lock();
    if args.all {
        for &lint in Lint::ALL {
            writeln!(
                out,
                "{}  {:40} {}",
                lint.code(),
                lint.name(),
                lint.summary()
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(true);
    }
    let name = args.lint.as_deref().unwrap_or_default();
    let lint = Lint::find(name).ok_or_else(|| format!("unknown lint {:?}", name))?;
    let severity = match lint.severity() {
        ProgramErrorSeverity::Error => "error",
        ProgramErrorSeverity::Warning => "warning",
    };
    writeln!(
        out,
        "{} {}\ncategory: {}\nseverity: {}\n\n{}",
        lint.code(),
        lint.name(),
        lint.category(),
        severity,
        lint.summary()
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Rust ignores `SIGPIPE`, so writing to a closed pipe (`usagi ... | head`)
/// is an error, which would be reported. Restore the default, so that `usagi`
/// stops quietly, like other command-line tools.
fn restore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: called first thing in `main`, before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// Writes suppression comments for the lint problems in `reports`. Returns
/// the number of lines suppressed and files changed.
fn add_ignores(
    reports: &[FileReport],
    style: suppress::Style,
    cwd: &Path,
) -> Result<(usize, usize), String> {
    let (mut comments, mut files) = (0, 0);
    for report in reports {
        let targets = suppress::targets(report);
        if targets.is_empty() {
            continue;
        }
        let show = || discover::display(&report.path, cwd);
        let bytes = std::fs::read(&report.path).map_err(|e| format!("{}: {}", show(), e))?;
        let decoded = usagi_core::decode_source(&bytes);
        let (text, count) = suppress::add_ignores(&decoded.text, &targets, style);
        if count == 0 {
            continue;
        }
        let bytes = usagi_core::encode(&text, decoded.encoding)
            .ok_or_else(|| format!("{}: can't store the comments in its encoding", show()))?;
        std::fs::write(&report.path, bytes).map_err(|e| format!("{}: {}", show(), e))?;
        comments += count;
        files += 1;
    }
    Ok((comments, files))
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{}s", word)
    }
}

fn settings(
    config: &config::LoadedConfig,
    files: &FileArgs,
    jobs: Option<u16>,
    overrides: Overrides,
    cwd: &Path,
) -> Result<Settings, String> {
    let overrides = Overrides {
        include: files.include.clone(),
        exclude: files.exclude.clone(),
        strict: files.strict(),
        jobs,
        ..overrides
    };
    Settings::new(config, overrides, cwd).map_err(|ConfigError(e)| e)
}

/// Lints every file, and indexes the project's classes when something needs
/// them: `--statistics`, or a lint that looks at other classes. Those lints
/// run on the index, and their problems are added to the files' reports.
fn lint_files(
    args: &cli::LintArgs,
    settings: &Settings,
    fix: bool,
) -> Result<(Vec<FileReport>, Option<usagi_core::project::Project>), String> {
    let project_lints = settings.project_lints_enabled();
    let index = !args.no_project && (args.statistics || project_lints);
    let mut reports = check_files(
        &args.files.paths,
        settings,
        Checking::Lint {
            fix,
            project: index,
        },
    )?;
    if !index {
        return Ok((reports, None));
    }
    // The files that `include` and `exclude` skip are part of the project,
    // whose classes extend them.
    let others = discover::unselected(&paths_or_cwd(&args.files.paths), settings);
    let others = check::index_all(&others, settings.jobs)?;
    let project = project::build(&mut reports, others, &args.files.paths);
    if project_lints {
        project::add_lints(&mut reports, &project, &enabled_in(settings));
    }
    Ok((reports, Some(project)))
}

/// Whether a lint is reported for a file, by the settings.
fn enabled_in(settings: &Settings) -> impl Fn(&Path, Lint) -> bool + Sync + '_ {
    move |path, lint| {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        settings.is_enabled(&absolute, lint)
    }
}

/// `paths`, or the current directory if there are none.
fn paths_or_cwd(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    }
}

/// What to check files for.
#[derive(Clone, Copy)]
enum Checking {
    Parse {
        ast: bool,
    },
    Lint {
        fix: bool,
        /// Keep the symbol tables, to index the classes.
        project: bool,
    },
}

/// Checks every file under `paths`, in parallel. Reports come back sorted
/// by path, with each file's problems sorted by position.
fn check_files(
    paths: &[PathBuf],
    settings: &Settings,
    checking: Checking,
) -> Result<Vec<FileReport>, String> {
    let (lint, ast, fix, project) = match checking {
        Checking::Parse { ast } => (false, ast, false, false),
        Checking::Lint { fix, project } => (true, false, fix, project),
    };
    let paths = paths_or_cwd(paths);
    let (files, errors) = discover::files(&paths, settings);
    if let Some(error) = errors.first() {
        return Err(error.clone());
    }
    let is_enabled = enabled_in(settings);
    let options = CheckOptions {
        strict: settings.strict,
        lint,
        ast,
        project,
        fix,
        is_enabled: &is_enabled,
        jobs: settings.jobs,
    };
    check::check_all(&files, &options)
}
