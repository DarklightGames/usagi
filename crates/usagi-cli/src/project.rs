//! The index of a project's classes, built from the files that were linted.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use usagi_core::{ClassId, Lint, Origin, Parent, Problem, Project, SymbolTable, project_lint};

use crate::check::{self, FileReport};
use crate::discover;

/// Indexes the classes of `reports`, taking their symbol tables, and of
/// `others`, files that weren't checked. A file's
/// source root is the first of `roots` (the paths that were given) that
/// contains it, so that a class in an earlier root replaces one of the same
/// name in a later one.
pub fn build(
    reports: &mut [FileReport],
    others: Vec<(PathBuf, SymbolTable)>,
    roots: &[PathBuf],
) -> Project {
    let default = [PathBuf::from(".")];
    let roots = if roots.is_empty() {
        &default[..]
    } else {
        roots
    };
    let checked = reports
        .iter_mut()
        .filter_map(|report| Some((report.path.clone(), report.symbols.take()?)));
    let files = checked
        .chain(others)
        .map(|(path, table)| {
            let root = roots
                .iter()
                .position(|root| path.starts_with(root))
                .unwrap_or(0);
            let origin = Origin {
                root,
                package: package_of(&path),
                path,
            };
            (origin, table)
        })
        .collect();
    Project::build(files)
}

/// The package a source file is compiled into: the folder that holds its
/// `Classes` folder, as `<Package>/Classes/<Class>.uc`.
fn package_of(path: &Path) -> Option<String> {
    let folders: Vec<_> = path.parent()?.components().collect();
    let classes = folders.iter().rposition(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|c| c.eq_ignore_ascii_case("classes"))
    })?;
    let package = folders.get(classes.checked_sub(1)?)?;
    Some(package.as_os_str().to_string_lossy().into_owned())
}

/// Runs the lints that look at other classes (see
/// [`usagi_core::project_lint`]) on every class, and adds what they find to
/// the reports of their files. A lint is reported for a file where
/// `is_enabled` says so.
pub fn add_lints(
    reports: &mut [FileReport],
    project: &Project,
    is_enabled: &dyn Fn(&Path, Lint) -> bool,
) {
    let classes: HashMap<&Path, ClassId> = project
        .classes()
        .map(|class| (project.origin(class).path.as_path(), class))
        .collect();
    for report in reports {
        let Some(&class) = classes.get(report.path.as_path()) else {
            continue;
        };
        let errors = project_lint::lint(project, class, &report.source);
        check::add_project_errors(report, &errors, is_enabled);
    }
}

/// Prints how many classes there are, which extend a class that isn't in
/// the project, and the problems with the classes as a whole.
pub fn statistics(out: &mut dyn Write, project: &Project, cwd: &Path) -> io::Result<()> {
    let show = |origin: &Origin| discover::display(&origin.path, cwd);
    writeln!(out, "project: {} classes", project.len())?;
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for class in project.classes() {
        if let Parent::Missing(parent) = project.parent(class) {
            *missing.entry(parent.to_string()).or_default() += 1;
        }
    }
    let total: usize = missing.values().sum();
    writeln!(
        out,
        "  classes that extend a class that isn't in the project: {}",
        total
    )?;
    for (parent, count) in &missing {
        writeln!(out, "    {:40} {:>6}", parent, count)?;
    }
    for problem in project.problems() {
        match problem {
            Problem::Cycle(classes) => {
                let names: Vec<&str> = classes.iter().map(|&c| project.name(c)).collect();
                writeln!(
                    out,
                    "  classes extend each other in a circle: {}",
                    names.join(" -> ")
                )?;
            }
            Problem::DuplicateClass {
                name,
                kept,
                dropped,
            } => writeln!(
                out,
                "  class {} is declared twice in one package: {} is used, {} isn't",
                name.as_str(),
                show(kept),
                show(dropped)
            )?,
            Problem::AmbiguousClass { name, used, also } => writeln!(
                out,
                "  class {} is in two packages: a bare name means {}, not {}",
                name.as_str(),
                show(used),
                show(also)
            )?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_is_the_folder_of_classes() {
        let package = |path: &str| package_of(Path::new(path));
        assert_eq!(
            package("Engine/Classes/Actor.uc").as_deref(),
            Some("Engine")
        );
        assert_eq!(
            package("./a/Mod/CLASSES/Sub/Pawn.uc").as_deref(),
            Some("Mod")
        );
        assert_eq!(package("Loose.uc"), None);
        assert_eq!(package("Mod/Pawn.uc"), None);
        assert_eq!(package("Classes/Pawn.uc"), None); // no folder above it
    }
}
