//! `usagi pkg classes`: a census of the classes in packages, and a check
//! that their native data and the packages themselves write back the same.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rayon::prelude::*;
use serde::Serialize;
use usagi_pkg::{NativeLookup, ObjectRef, Package, Packages, SearchPath};
use walkdir::WalkDir;

use super::{Align, GREEN, RED, Style, YELLOW, column, human_size, separated, write_table};
use crate::cli::{PkgClassesArgs, PkgClassesFormat};

/// The extensions of the files searched for in folders.
const EXTENSIONS: &[&str] = &[
    "u", "utx", "usx", "ukx", "uax", "umx", "rom", "ut2", "unr", "uz2",
];

/// The counts for one class.
#[derive(Debug, Default, Clone, Serialize)]
struct ClassCount {
    class: String,
    objects: u64,
    bytes: u64,
    /// The native class whose data the objects have, `unread` or `not found`.
    native: String,
    /// The class whose native data isn't read, or that can't be found.
    #[serde(skip_serializing_if = "Option::is_none")]
    native_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    round_trip: Option<RoundTrip>,
}

/// How many objects of a class wrote back the same, differently, or
/// couldn't be read.
#[derive(Debug, Default, Clone, Copy, Serialize)]
struct RoundTrip {
    /// Their native data's bytes.
    native_bytes: u64,
    same: u64,
    different: u64,
    failed: u64,
}

/// A problem with an object or a package.
#[derive(Debug, Clone, Serialize)]
struct Problem {
    file: String,
    /// The object, as UnrealScript refers to it; `None` for the package.
    #[serde(skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    message: String,
}

/// What's found in one file.
#[derive(Default)]
struct FileCensus {
    classes: HashMap<String, ClassCount>,
    problems: Vec<Problem>,
    /// With `--round-trip`: whether the package wrote back the same.
    written_same: Option<bool>,
}

#[derive(Serialize)]
struct Census {
    files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    packages_written_same: Option<usize>,
    classes: Vec<ClassCount>,
    problems: Vec<Problem>,
}

/// `usagi pkg classes`: counts objects and bytes by class, and with
/// `--round-trip`, writes back every object's native data and every
/// package. Returns whether everything wrote back the same.
pub fn classes(args: &PkgClassesArgs, color: bool, out: &mut impl Write) -> Result<bool, String> {
    let files = find_files(&args.paths)?;
    let censuses: Vec<FileCensus> = files
        .par_iter()
        .map_init(HashMap::new, |games, file| {
            census_of(games, file, args.round_trip)
        })
        .collect();

    let mut classes: BTreeMap<String, ClassCount> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut written_same = 0;
    for census in censuses {
        for (key, count) in census.classes {
            let total = classes.entry(key).or_insert_with(|| ClassCount {
                objects: 0,
                bytes: 0,
                round_trip: count.round_trip.map(|_| RoundTrip::default()),
                ..count.clone()
            });
            total.objects += count.objects;
            total.bytes += count.bytes;
            if let (Some(total), Some(count)) = (&mut total.round_trip, count.round_trip) {
                total.native_bytes += count.native_bytes;
                total.same += count.same;
                total.different += count.different;
                total.failed += count.failed;
            }
        }
        problems.extend(census.problems);
        written_same += usize::from(census.written_same == Some(true));
    }
    let mut classes: Vec<ClassCount> = classes.into_values().collect();
    classes.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.class.cmp(&b.class)));
    let census = Census {
        files: files.len(),
        packages_written_same: args.round_trip.then_some(written_same),
        classes,
        problems,
    };
    let ok = census.problems.is_empty();
    match args.format {
        PkgClassesFormat::Text => text(&census, args, Style { color }, out),
        PkgClassesFormat::Json => serde_json::to_writer_pretty(&mut *out, &census)
            .map_err(io::Error::other)
            .and_then(|()| writeln!(out)),
    }
    .map_err(|e| e.to_string())?;
    Ok(ok || !args.round_trip)
}

/// The package files in `paths`: files as they are, and in folders, files
/// with a package's extension, in order.
fn find_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if !path.is_dir() {
            if !path.exists() {
                return Err(format!("{}: no such file or folder", path.display()));
            }
            files.push(path.clone());
            continue;
        }
        for entry in WalkDir::new(path).sort_by_file_name() {
            let entry = entry.map_err(|e| e.to_string())?;
            let is_package = entry
                .path()
                .extension()
                .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)));
            if entry.file_type().is_file() && is_package {
                files.push(entry.into_path());
            }
        }
    }
    Ok(files)
}

/// Counts one file's classes, and with `round_trip`, writes back its
/// objects and the package. `games` holds the packages of each game the
/// files are in, for the thread.
fn census_of(
    games: &mut HashMap<Vec<SearchPath>, Packages>,
    file: &Path,
    round_trip: bool,
) -> FileCensus {
    let name = file.display().to_string();
    let mut census = FileCensus::default();
    let package = match Package::read(file) {
        Ok(package) => Rc::new(package),
        Err(e) => {
            census.problems.push(Problem {
                file: name,
                object: None,
                message: e.to_string(),
            });
            return census;
        }
    };
    // The package itself isn't added to the set: every file would stay in
    // memory. Its own classes are found in it all the same.
    let packages = games
        .entry(super::game::search_paths(file))
        .or_insert_with_key(|paths| Packages::new(paths.clone()));
    for export in 0..package.exports().len() {
        let e = &package.exports()[export];
        let class = match e.class {
            ObjectRef::Null => "Core.Class".to_string(),
            class => package.full_path(class).expect("a class"),
        };
        let lookup = packages.native_class(&package, export);
        let count = census
            .classes
            .entry(class.to_lowercase())
            .or_insert_with(|| {
                let (native, native_path) = match &lookup {
                    NativeLookup::Known(native) => (native.name().to_string(), None),
                    NativeLookup::Unread(path) => ("unread".to_string(), Some(path.clone())),
                    NativeLookup::NotFound(path) => ("not found".to_string(), Some(path.clone())),
                };
                ClassCount {
                    class,
                    native,
                    native_path,
                    round_trip: round_trip.then(RoundTrip::default),
                    ..ClassCount::default()
                }
            });
        count.objects += 1;
        count.bytes += u64::from(e.serial_size);
        let (Some(totals), NativeLookup::Known(native)) = (&mut count.round_trip, &lookup) else {
            continue;
        };
        if e.serial_size == 0 {
            continue;
        }
        let result = packages
            .native_start(&package, export, native)
            .and_then(|start| package.round_trip(export, native, start))
            .map(|r| (r.size, r.difference));
        let problem = match result {
            Ok((size, None)) => {
                totals.native_bytes += size as u64;
                totals.same += 1;
                None
            }
            Ok((size, Some(offset))) => {
                totals.native_bytes += size as u64;
                totals.different += 1;
                Some(format!(
                    "its native data ({}) writes back differently from offset {}",
                    native.name(),
                    offset
                ))
            }
            Err(error) => {
                totals.failed += 1;
                Some(error.to_string())
            }
        };
        if let Some(message) = problem {
            census.problems.push(Problem {
                file: name.clone(),
                object: Some(package.reference(ObjectRef::Export(export))),
                message,
            });
        }
    }
    if round_trip {
        let problem = match packages.write(&package) {
            Ok(bytes) => first_difference(package.bytes(), &bytes).map(|offset| {
                format!("the package writes back differently from offset {}", offset)
            }),
            Err(error) => Some(format!("the package can't be written: {}", error)),
        };
        census.written_same = Some(problem.is_none());
        if let Some(message) = problem {
            census.problems.push(Problem {
                file: name,
                object: None,
                message,
            });
        }
    }
    census
}

/// Where two byte strings first differ, if they do.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => Some(i),
        None if a.len() != b.len() => Some(a.len().min(b.len())),
        None => None,
    }
}

fn text(
    census: &Census,
    args: &PkgClassesArgs,
    style: Style,
    out: &mut impl Write,
) -> io::Result<()> {
    let count = |n: u64| {
        if args.humanize {
            separated(n)
        } else {
            n.to_string()
        }
    };
    let size = |n: u64| {
        if args.humanize {
            human_size(n)
        } else {
            n.to_string()
        }
    };
    let columns = [
        column("Class", Align::Left, None),
        column("Objects", Align::Right, None),
        column("Bytes", Align::Right, None),
        column("Native data", Align::Left, None),
        column("Native bytes", Align::Right, None),
        column("Same", Align::Right, Some(GREEN)),
        column("Different", Align::Right, Some(RED)),
        column("Failed", Align::Right, Some(RED)),
    ];
    let rows: Vec<Vec<String>> = census
        .classes
        .iter()
        .map(|c| {
            let native = match &c.native_path {
                Some(path) => format!("{} ({})", c.native, path),
                None => c.native.clone(),
            };
            let nonzero = |n: u64| if n == 0 { String::new() } else { count(n) };
            let (native_bytes, same, different, failed) = match c.round_trip {
                Some(r) => (
                    size(r.native_bytes),
                    nonzero(r.same),
                    nonzero(r.different),
                    nonzero(r.failed),
                ),
                None => Default::default(),
            };
            vec![
                c.class.clone(),
                count(c.objects),
                size(c.bytes),
                native,
                native_bytes,
                same,
                different,
                failed,
            ]
        })
        .collect();
    write_table(out, style, &columns, &rows)?;
    writeln!(out)?;
    let files = format!(
        "{} file{}",
        count(census.files as u64),
        if census.files == 1 { "" } else { "s" }
    );
    match census.packages_written_same {
        Some(same) => writeln!(
            out,
            "{}; {} written back the same",
            files,
            count(same as u64)
        )?,
        None => writeln!(out, "{}", files)?,
    }
    if !census.problems.is_empty() {
        writeln!(out)?;
        for problem in &census.problems {
            let file = style.paint(YELLOW, &problem.file);
            match &problem.object {
                Some(object) => writeln!(out, "{}: {}: {}", file, object, problem.message)?,
                None => writeln!(out, "{}: {}", file, problem.message)?,
            }
        }
    }
    Ok(())
}
