//! `usagi pkg export`: write objects' native data and properties to files.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use std::sync::Arc;

use rayon::prelude::*;

use usagi_io::{Audio, DdsWriter, Image, PngWriter, TgaWriter, Unusable, WavWriter, Writer};
use usagi_pkg::{
    ObjectPattern, ObjectRef, Package, Packages, Palette, SearchPath, Sound, TEXTURE_MIPS_VERSION,
    TextureFormat, Value,
};

use super::Style;
use super::obj::{Object, explain, is_class, not_found, object_json, toml_text, write_t3d};
use crate::cli::{PkgExportArgs, PkgExportLayout, PkgExportNative, PkgPropertiesFormat};

/// A package file that has been read.
struct Input {
    file: String,
    package: Arc<Package>,
    /// The packages of the game it's in, which declare its objects' classes.
    packages: Arc<Packages>,
}

/// Files an object's native data is written as: each one's extension
/// (after the object's name and a dot) and contents.
type Files = Vec<(String, Vec<u8>)>;

/// Why an object's native data can't be written in a file format, and
/// whether that's because it's malformed.
type Problem = (String, bool);

/// `usagi pkg export`: writes each object's native data, after its
/// properties: in a file format where there's one (a sound as `.wav`, a
/// texture as `.dds`, `.tga` or `.png`),
/// otherwise as it is (`.bin`); and if asked, its properties (`.t3d`,
/// `.json` or `.toml`).
/// Objects are converted and written on `threads` threads. Returns whether
/// every object could be exported; problems are written to `err`.
pub fn export(
    args: &PkgExportArgs,
    threads: usize,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<bool, String> {
    if args.native == PkgExportNative::None && args.properties.is_none() {
        return Err(
            "nothing to write: `--native none` without `--properties` writes no files".to_string(),
        );
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
        .map_err(|e| e.to_string())?;
    // The files are read on the worker threads, which is most of the time
    // a large export takes when it isn't writing.
    let read: Vec<Result<Package, String>> = pool.install(|| {
        args.paths
            .par_iter()
            .map(|path| {
                // SAFETY: the files are only read, for as long as the export
                // takes, and the export never writes to them (its files have
                // other names). Another program changing one meanwhile is a
                // risk taken for the speed: see `Package::map`.
                unsafe { Package::map(path) }.map_err(|e| format!("{}: {}", path.display(), e))
            })
            .collect()
    });
    // One set of packages per game, so each package is read once.
    let mut games: HashMap<Vec<SearchPath>, Arc<Packages>> = HashMap::new();
    let inputs = args
        .paths
        .iter()
        .zip(read)
        .map(|(path, package)| {
            let file = path.display().to_string();
            let package = package?;
            let packages = Arc::clone(
                games
                    .entry(super::game::search_paths(path))
                    .or_insert_with_key(|paths| Arc::new(Packages::new(paths.clone()))),
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

    // What needs the game's packages (shared, and not thread-safe) is
    // worked out first; reading, converting and writing each object's data
    // then runs on worker threads.
    let jobs: Vec<Job> = exports
        .iter()
        .zip(destinations)
        .map(|((input, object, end), destination)| Job {
            file: &input.file,
            package: &input.package,
            object,
            end: *end,
            plan: plan(input, object, args),
            base: args.output.join(destination),
        })
        .collect();
    let outcomes: Vec<Result<Outcome, String>> =
        pool.install(|| jobs.par_iter().map(|job| export_one(job, args)).collect());
    // Notes in the order of the objects, whichever finished first.
    let mut converted = 0;
    for outcome in outcomes {
        let outcome = outcome?;
        if let Some(note) = &outcome.note {
            writeln!(err, "{}", note).map_err(|e| e.to_string())?;
        }
        ok &= !outcome.malformed;
        converted += usize::from(outcome.converted);
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

/// An object to export, and how.
struct Job<'a> {
    file: &'a str,
    package: &'a Package,
    object: &'a Object,
    /// Where its properties end and its native data starts.
    end: usize,
    plan: Result<Plan, Problem>,
    /// Its files' path, without their extension.
    base: PathBuf,
}

/// How an object's native data is written, with what converting it needs
/// from the game's other packages.
enum Plan {
    /// As it is, `.bin`, or not at all.
    Bin,
    Sound,
    Texture {
        format: TextureFormat,
        masked: bool,
        palette: Option<Palette>,
    },
}

/// What exporting an object did.
#[derive(Default)]
struct Outcome {
    /// Why its native data was written as `.bin`.
    note: Option<String>,
    /// Whether that's because it's malformed.
    malformed: bool,
    /// Whether its native data was written in a file format.
    converted: bool,
}

/// How to write an object's native data. A problem if it's meant to be in
/// a file format but can't be.
fn plan(input: &Input, object: &Object, args: &PkgExportArgs) -> Result<Plan, Problem> {
    if matches!(args.native, PkgExportNative::Bin | PkgExportNative::None) {
        return Ok(Plan::Bin);
    }
    // Subclasses don't start with their class's data alone (a sound
    // group's isn't a sound's), so only the classes themselves are read.
    if is_class(&input.package, object.export, "Engine.Sound") {
        return Ok(Plan::Sound);
    }
    if !is_class(&input.package, object.export, "Engine.Texture") {
        return Ok(Plan::Bin);
    }
    // A property that isn't stored has its default, which for a texture's
    // is the type's: `TEXF_P8`, no palette, not masked.
    let property = |name: &str| {
        object
            .properties
            .iter()
            .find(|p| p.array_index == 0 && p.name.eq_ignore_ascii_case(name))
            .map(|p| &p.value)
    };
    let format = match property("Format") {
        Some(Value::Byte(value)) => TextureFormat::from_byte(*value),
        _ => TextureFormat::P8,
    };
    let palette = match property("Palette") {
        Some(&Value::Object(palette)) if format == TextureFormat::P8 => {
            find_palette(input, palette)?
        }
        _ => None,
    };
    Ok(Plan::Texture {
        format,
        masked: matches!(property("bMasked"), Some(Value::Bool(true))),
        palette,
    })
}

/// Writes an object's files: its native data and its properties.
fn export_one(job: &Job, args: &PkgExportArgs) -> Result<Outcome, String> {
    if let Some(parent) = job.base.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
    }
    let write = |extension: &str, bytes: &[u8]| {
        // Appended: a flat name has dots in it already.
        let mut path = job.base.clone().into_os_string();
        path.push(".");
        path.push(extension);
        let path = PathBuf::from(path);
        fs::write(&path, bytes).map_err(|e| format!("{}: {}", path.display(), e))
    };
    let (package, object) = (job.package, job.object);
    let mut outcome = Outcome::default();
    let native = &package.export_data(object.export)[job.end..];
    let file = match &job.plan {
        Ok(plan) => convert(package, object.export, job.end, plan, args),
        Err(problem) => Err(problem.clone()),
    };
    let file = file.unwrap_or_else(|(problem, malformed)| {
        outcome.note = Some(format!(
            "usagi: {}: {}: {}; written as .bin",
            job.file, object.reference, problem
        ));
        outcome.malformed = malformed;
        None
    });
    match file {
        _ if args.native == PkgExportNative::None => {}
        Some(files) => {
            for (extension, bytes) in &files {
                write(extension, bytes)?;
            }
            outcome.converted = true;
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
            write_t3d(package, object, Style { color: false }, &mut t3d)
                .expect("writing to memory");
            write("t3d", &t3d)?;
        }
        Some(PkgPropertiesFormat::Json) => {
            let mut json = serde_json::to_vec_pretty(&object_json(package, object)).expect("JSON");
            json.push(b'\n');
            write("json", &json)?;
        }
        Some(PkgPropertiesFormat::Toml) => {
            let toml = toml_text(&object_json(package, object));
            write("toml", toml.as_bytes())?;
        }
    }
    Ok(outcome)
}

/// An object's native data, from `end` (after its properties), in a file
/// format, as `plan` says. `None` if there's no format for its class.
fn convert(
    package: &Package,
    export: usize,
    end: usize,
    plan: &Plan,
    args: &PkgExportArgs,
) -> Result<Option<Files>, Problem> {
    match plan {
        Plan::Bin => Ok(None),
        Plan::Sound => sound(package, export, end).map(Some),
        Plan::Texture {
            format,
            masked,
            palette,
        } => {
            let image = image(package, export, end, *format, palette.as_ref(), *masked)?;
            texture(&image, args).map(Some)
        }
    }
}

/// A sound's file, as `.wav`.
fn sound(package: &Package, export: usize, end: usize) -> Result<Files, Problem> {
    let (sound, _) = package
        .read_native::<Sound>(export, end)
        .map_err(|e| (e.to_string(), true))?;
    let mut bytes = Vec::new();
    WavWriter
        .write(&Audio::from_sound(&sound), &mut bytes)
        .map_err(|refused| (refused.to_string(), false))?;
    Ok(vec![(WavWriter.extension().to_string(), bytes)])
}

/// A texture's image, from its native data.
fn image(
    package: &Package,
    export: usize,
    end: usize,
    format: TextureFormat,
    palette: Option<&Palette>,
    masked: bool,
) -> Result<Image, Problem> {
    if package.summary.version() < TEXTURE_MIPS_VERSION {
        return Err((
            format!(
                "textures before version {} aren't read",
                TEXTURE_MIPS_VERSION
            ),
            false,
        ));
    }
    let (texture, _) = package
        .texture(export, end)
        .map_err(|e| (e.to_string(), true))?;
    Image::from_texture(&texture, format, palette, masked).map_err(|e| match e {
        Unusable::Unsupported(why) => (why, false),
        Unusable::Malformed(why) => (why, true),
    })
}

/// A texture's image as DDS, with every mipmap, or as TGA or PNG, its
/// largest mipmap, or with `--mips`, each in a file. By default, DXT
/// textures are DDS, and others TGA, except `G16`, which TGA can't hold, as
/// PNG.
fn texture(image: &Image, args: &PkgExportArgs) -> Result<Files, Problem> {
    let writer: &dyn Writer<Image> = match args.native {
        PkgExportNative::Png => &PngWriter,
        PkgExportNative::Tga => &TgaWriter,
        PkgExportNative::Dds => &DdsWriter,
        _ if image.format.is_dxt() => &DdsWriter,
        _ if image.format == TextureFormat::G16 => &PngWriter,
        _ => &TgaWriter,
    };
    let refused = |refused: usagi_io::Refused| (refused.to_string(), false);
    let extension = writer.extension();
    if !args.mips || extension == DdsWriter.extension() {
        let mut bytes = Vec::new();
        writer.write(image, &mut bytes).map_err(refused)?;
        return Ok(vec![(extension.to_string(), bytes)]);
    }
    (0..image.levels.len())
        .map(|level| {
            let mut bytes = Vec::new();
            writer
                .write(&image.level(level), &mut bytes)
                .map_err(refused)?;
            let name = match level {
                0 => extension.to_string(),
                _ => format!("mip{}.{}", level, extension),
            };
            Ok((name, bytes))
        })
        .collect()
}

/// The palette a texture refers to, in its package or another of the
/// game's. `None` for `None`.
fn find_palette(input: &Input, palette: ObjectRef) -> Result<Option<Palette>, Problem> {
    if palette == ObjectRef::Null {
        return Ok(None);
    }
    let reference = input.package.reference(palette);
    let Some(scope) = input.packages.resolve(&input.package, palette) else {
        return Err((format!("its palette, {}, can't be found", reference), false));
    };
    if !is_class(&scope.package, scope.export, "Engine.Palette") {
        return Err((format!("its palette, {}, isn't a palette", reference), true));
    }
    let object = Object::read(&input.packages, &scope.package, scope.export, &[]);
    let read = match (object.error, object.end) {
        (None, Some(end)) => scope.package.read_native::<Palette>(scope.export, end),
        (error, _) => Err(error.expect("an error, since the end isn't known")),
    };
    read.map(|(palette, _)| Some(palette))
        .map_err(|e| (format!("its palette, {}: {}", reference, e), true))
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
