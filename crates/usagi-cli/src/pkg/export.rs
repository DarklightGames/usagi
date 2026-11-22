//! `usagi pkg export`: write objects' native data and properties to files.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use std::rc::Rc;

use usagi_io::{Audio, WavWriter, Writer};
use usagi_pkg::{ObjectPattern, ObjectRef, Package, Packages, SearchPath, Sound};

use super::Style;
use super::obj::{Object, explain, is_class, not_found, object_json, toml_text, write_t3d};
use crate::cli::{PkgExportArgs, PkgExportLayout, PkgExportNative, PkgPropertiesFormat};

/// A package file that has been read.
struct Input {
    file: String,
    package: Rc<Package>,
    /// The packages of the game it's in, which declare its objects' classes.
    packages: Rc<Packages>,
}

/// `usagi pkg export`: writes each object's native data, after its
/// properties: in a file format where there's one (a sound as `.wav`),
/// otherwise as it is (`.bin`); and if asked, its properties (`.t3d`,
/// `.json` or `.toml`).
/// Returns whether every object could be exported; problems are written to
/// `err`.
pub fn export(
    args: &PkgExportArgs,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<bool, String> {
    if args.native == PkgExportNative::None && args.properties.is_none() {
        return Err(
            "nothing to write: `--native none` without `--properties` writes no files".to_string(),
        );
    }
    // One set of packages per game, so each package is read once.
    let mut games: HashMap<Vec<SearchPath>, Rc<Packages>> = HashMap::new();
    let inputs = args
        .paths
        .iter()
        .map(|path| {
            let file = path.display().to_string();
            let package = Package::read(path).map_err(|e| format!("{}: {}", file, e))?;
            let packages = Rc::clone(
                games
                    .entry(super::game::search_paths(path))
                    .or_insert_with_key(|paths| Rc::new(Packages::new(paths.clone()))),
            );
            Ok(Input {
                file,
                package: packages.add(package),
                packages,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selected = select(&inputs, args)?;

    let mut ok = true;
    let mut exports = Vec::new();
    for (input, export) in selected {
        let object = Object::read(&input.packages, &input.package, export, &[]);
        match (&object.error, object.end) {
            (None, Some(end)) => exports.push((input, object, end)),
            (error, _) => {
                let error = error.as_ref().expect("an error, since the end isn't known");
                writeln!(
                    err,
                    "usagi: {}: {}: {}",
                    input.file, object.reference, error
                )
                .map_err(|e| e.to_string())?;
                ok = false;
            }
        }
    }

    // Every destination, before anything is written. Different objects
    // only collide if the same package is given twice.
    let mut destinations: Vec<PathBuf> = Vec::new();
    let mut seen: HashMap<String, &str> = HashMap::new();
    let mut collisions = Vec::new();
    for (input, object, _) in &exports {
        let destination = destination(&input.package, object.export, args.layout);
        let key = destination.to_string_lossy().to_lowercase();
        match seen.get(&key) {
            Some(&other) => collisions.push(format!(
                "{} and {} would both be written to {}",
                other,
                object.reference,
                destination.display()
            )),
            None => {
                seen.insert(key, &object.reference);
            }
        }
        destinations.push(destination);
    }
    if !collisions.is_empty() {
        return Err(collisions.join("\n"));
    }

    let mut converted = 0;
    for ((input, object, end), destination) in exports.iter().zip(&destinations) {
        let base = args.output.join(destination);
        if let Some(parent) = base.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
        }
        let write = |extension: &str, bytes: &[u8]| {
            // Appended: a flat name has dots in it already.
            let mut path = base.clone().into_os_string();
            path.push(".");
            path.push(extension);
            let path = PathBuf::from(path);
            fs::write(&path, bytes).map_err(|e| format!("{}: {}", path.display(), e))
        };
        let native = &input.package.export_data(object.export)[*end..];
        let file = match args.native {
            PkgExportNative::Auto => convert(&input.package, object.export, *end),
            PkgExportNative::Bin | PkgExportNative::None => Ok(None),
        };
        let file = file.unwrap_or_else(|(problem, malformed)| {
            let note = format!(
                "usagi: {}: {}: {}; written as .bin",
                input.file, object.reference, problem
            );
            let _ = writeln!(err, "{}", note);
            ok &= !malformed;
            None
        });
        match file {
            _ if args.native == PkgExportNative::None => {}
            Some((extension, bytes)) => {
                write(extension, &bytes)?;
                converted += 1;
                if args.raw {
                    write("bin", native)?;
                }
            }
            None => write("bin", native)?,
        }
        match args.properties {
            None => {}
            Some(PkgPropertiesFormat::T3d) => {
                let mut t3d = Vec::new();
                write_t3d(&input.package, object, Style { color: false }, &mut t3d)
                    .expect("writing to memory");
                write("t3d", &t3d)?;
            }
            Some(PkgPropertiesFormat::Json) => {
                let mut json =
                    serde_json::to_vec_pretty(&object_json(&input.package, object)).expect("JSON");
                json.push(b'\n');
                write("json", &json)?;
            }
            Some(PkgPropertiesFormat::Toml) => {
                let toml = toml_text(&object_json(&input.package, object));
                write("toml", toml.as_bytes())?;
            }
        }
    }
    let plural =
        |n: usize, one: &str, many: &str| format!("{} {}", n, if n == 1 { one } else { many });
    let formats = if converted > 0 {
        format!(" ({} in a file format)", converted)
    } else {
        String::new()
    };
    writeln!(
        out,
        "Exported {}{} to {}",
        plural(exports.len(), "object", "objects"),
        formats,
        args.output.display()
    )
    .map_err(|e| e.to_string())?;
    Ok(ok)
}

/// An object's native data, from `end` (after its properties), in a file
/// format: its extension and contents. `None` if there's no format for its
/// class. A problem (and whether it's that the data is malformed) if there
/// is one but it can't be written in it.
fn convert(
    package: &Package,
    export: usize,
    end: usize,
) -> Result<Option<(&'static str, Vec<u8>)>, (String, bool)> {
    // A sound's subclasses don't start with a sound's data.
    if !is_class(package, export, "Engine.Sound") {
        return Ok(None);
    }
    let (sound, _) = package
        .read_native::<Sound>(export, end)
        .map_err(|e| (e.to_string(), true))?;
    let mut bytes = Vec::new();
    WavWriter
        .write(&Audio::from_sound(&sound), &mut bytes)
        .map_err(|refused| (refused.to_string(), false))?;
    Ok(Some((WavWriter.extension(), bytes)))
}

/// The exports to write, in the order of the files and then of their export
/// tables: those matching the patterns (or every export), of the classes
/// asked for, except groups. A pattern only needs to match in one of the files.
fn select<'a>(
    inputs: &'a [Input],
    args: &PkgExportArgs,
) -> Result<Vec<(&'a Input, usize)>, String> {
    let patterns = args
        .objects
        .iter()
        .map(|text| text.parse::<ObjectPattern>())
        .collect::<Result<Vec<_>, _>>()?;
    let mut found: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); inputs.len()];
    if patterns.is_empty() {
        for (input, found) in inputs.iter().zip(&mut found) {
            found.extend(0..input.package.exports().len());
        }
    }
    for pattern in &patterns {
        let mut any = false;
        for (input, found) in inputs.iter().zip(&mut found) {
            let matches = input.package.find_exports(pattern);
            any |= !matches.is_empty();
            found.extend(matches);
        }
        if !any {
            return Err(not_found_in(inputs, pattern));
        }
    }
    Ok(inputs
        .iter()
        .zip(found)
        .flat_map(|(input, found)| found.into_iter().map(move |export| (input, export)))
        // Groups are only containers: they're the folders or the path.
        .filter(|&(input, export)| !is_class(&input.package, export, "Core.Package"))
        .filter(|&(input, export)| {
            args.class.is_empty()
                || args
                    .class
                    .iter()
                    .any(|class| is_class(&input.package, export, class))
        })
        .collect())
}

/// Why `pattern` matches no export in any of the files.
fn not_found_in(inputs: &[Input], pattern: &ObjectPattern) -> String {
    if let [input] = inputs {
        return not_found(&input.package, pattern, &input.file);
    }
    if pattern.is_glob() {
        return format!(
            "{} matches no objects in the {} packages",
            pattern,
            inputs.len()
        );
    }
    inputs
        .iter()
        .find_map(|input| explain(&input.package, pattern, &input.file))
        .unwrap_or_else(|| format!("{} isn't in any of the {} packages", pattern, inputs.len()))
}

/// Where an export's files go in the output directory, without their
/// extension.
fn destination(package: &Package, export: usize, layout: PkgExportLayout) -> PathBuf {
    let object = ObjectRef::Export(export);
    match layout {
        PkgExportLayout::Flat => PathBuf::from(package.reference(object)),
        PkgExportLayout::Folder => {
            let mut path = PathBuf::from(&package.name);
            path.push(package.class_name(&package.exports()[export]));
            let groups = package.path(object).expect("an export");
            path.extend(groups.split('.'));
            path
        }
    }
}
