mod common;

use std::path::Path;
use std::rc::Rc;

use common::builder::*;
use common::{ExportBytes, ImportBytes, PackageBytes, SummaryBytes, name};
use tempfile::TempDir;
use usagi_pkg::archive::LazyArray;
use usagi_pkg::{
    Error, Mipmap, Native, NativeClass, NativeLookup, ObjectRef, Package, Packages, SearchPath,
    Texture,
};

/// The names of the packages built here, by index.
const NAMES: &[&str] = &[
    "None",
    "Core",
    "Package",
    "Class",
    "Engine",
    "Texture",
    "Animation",
    "Missing",
    "Gone",
    "Object",
    "A",
    "B",
];

fn n(text: &str) -> i32 {
    NAMES.iter().position(|n| *n == text).unwrap() as i32
}

/// Imports: `Engine` (-1), `Engine.Texture` (-2), `Engine.Animation` (-3),
/// `Missing` (-4), `Missing.Gone` (-5), `Core` (-6), `Core.Object` (-7).
fn imports() -> Vec<ImportBytes> {
    let import = |class: &str, outer: i32, object: &str| ImportBytes {
        class_package: n("Core"),
        class_name: n(class),
        outer,
        object_name: n(object),
    };
    vec![
        import("Package", 0, "Engine"),
        import("Class", -1, "Texture"),
        import("Class", -1, "Animation"),
        import("Package", 0, "Missing"),
        import("Class", -4, "Gone"),
        import("Package", 0, "Core"),
        import("Class", -6, "Object"),
    ]
}

fn texture() -> Texture {
    Texture {
        mips: vec![Mipmap {
            data: LazyArray {
                seek_pos: None,
                items: vec![1, 2, 3, 4],
            },
            u_size: 2,
            v_size: 2,
            u_bits: 1,
            v_bits: 1,
        }],
    }
}

/// A texture's data, as at `offset` in the file: a `None`, then its mipmaps.
fn texture_data(offset: usize) -> Vec<u8> {
    let bytes = Native::Texture(texture())
        .write(128, 29, offset + 1)
        .unwrap();
    [vec![0], bytes].concat()
}

/// Package bytes with exports `A`, a texture, and `B`, of class `b_class`,
/// whose data is in the other order: each must move to be written in the
/// order of the export table.
fn swapped(b_class: i32) -> Vec<u8> {
    swapped_with(b_class, 0x0007_0004)
}

/// [`swapped`], with `B`'s flags.
fn swapped_with(b_class: i32, b_flags: u32) -> Vec<u8> {
    let len = texture_data(0).len();
    let package = |data: Vec<Vec<u8>>, offsets: [Option<i32>; 2]| PackageBytes {
        summary: SummaryBytes {
            generations: vec![(1, 5), (2, 12)],
            ..SummaryBytes::default()
        },
        names: NAMES.iter().map(|text| name(text)).collect(),
        imports: imports(),
        exports: vec![
            ExportBytes {
                class: -2,
                object_name: n("A"),
                size: len as i32,
                offset: offsets[0],
                ..ExportBytes::default()
            },
            ExportBytes {
                class: b_class,
                object_name: n("B"),
                flags: b_flags,
                size: len as i32,
                offset: offsets[1],
                ..ExportBytes::default()
            },
        ],
        data,
    };
    let slots: Vec<usize> = Package::from_bytes(&package(vec![], [None, None]).bytes())
        .unwrap()
        .exports()
        .iter()
        .map(|e| e.serial_offset.unwrap() as usize)
        .collect();
    let mut b_data = vec![0; len];
    b_data[1..].fill(0xAB);
    package(
        vec![b_data, texture_data(slots[1])],
        [Some(slots[1] as i32), Some(slots[0] as i32)],
    )
    .bytes()
}

#[test]
fn a_package_written_with_no_changes_is_the_same() {
    let mut b = Builder::new();
    let texture_class = b.import("Core", "Class", 0, "Texture");
    let thing = b.export(texture_class, 0, "Thing");
    b.set_data(thing, vec![0, 1, 2, 3]);
    // No data, and a name stored as UTF-16.
    b.export(0, 0, "None");
    let mut bytes = b.bytes();
    let package = Package::from_bytes(&bytes).unwrap();
    assert_eq!(package.bytes(), bytes);
    let mut asked = false;
    let written = package
        .write_with(&mut |_| {
            asked = true;
            Ok((NativeClass::Object, 0))
        })
        .unwrap();
    assert_eq!(written, bytes);
    // Nothing moved, so nothing was asked about.
    assert!(!asked);

    // Names are written as they're stored, in any encoding and length.
    let long = "L".repeat(70);
    bytes = PackageBytes {
        names: vec![
            name("None"),
            [
                vec![0x83],
                "Ab\0"
                    .encode_utf16()
                    .flat_map(|u| u.to_le_bytes())
                    .collect(),
            ]
            .concat()
            .into_iter()
            .chain(0x00070010u32.to_le_bytes())
            .collect(),
            name(&long),
        ],
        ..PackageBytes::default()
    }
    .bytes();
    let package = Package::from_bytes(&bytes).unwrap();
    assert_eq!(package.names()[2].name.len(), 63);
    assert_eq!(package.write_with(&mut |_| unreachable!()).unwrap(), bytes);
}

#[test]
fn packages_before_version_68_arent_written() {
    let bytes = PackageBytes {
        summary: SummaryBytes {
            file_version: 67,
            ..SummaryBytes::default()
        },
        names: vec![name("None")],
        ..PackageBytes::default()
    }
    .bytes();
    let package = Package::from_bytes(&bytes).unwrap();
    let error = package.write_with(&mut |_| unreachable!()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid data at offset 0: packages before version 68 aren't written"
    );
}

#[test]
fn exports_that_move_are_written_again_if_they_hold_offsets() {
    let package = Package::from_bytes(&swapped(-7)).unwrap();
    let old_b = package.export_data(1).to_vec();
    let mut asked = Vec::new();
    let written = package
        .write_with(&mut |export| {
            asked.push(export);
            let class = [NativeClass::Texture, NativeClass::Object][export].clone();
            Ok((class, 1))
        })
        .unwrap();
    assert_eq!(asked, [0, 1]);
    let package = Package::from_bytes(&written).unwrap();
    // The texture is where B was, with its lazy array's offset changed.
    let offset = package.exports()[0].serial_offset.unwrap() as usize;
    assert_eq!(package.export_data(0), texture_data(offset));
    let read = package.native_data(0, &NativeClass::Texture, 1).unwrap();
    let Native::Texture(read) = read.native else {
        panic!("not a texture");
    };
    assert_eq!(read.mips[0].data.items, [1, 2, 3, 4]);
    // B is copied.
    assert_eq!(package.export_data(1), old_b);
    // And it's laid out as the engine lays it out: written again, it's the
    // same.
    assert_eq!(
        package.write_with(&mut |_| unreachable!()).unwrap(),
        written
    );
}

#[test]
fn an_export_that_cant_be_read_cant_be_moved() {
    let package = Package::from_bytes(&swapped(-7)).unwrap();
    let error = package
        .write_with(&mut |_| {
            Err(Error::InvalidData {
                offset: 0,
                reason: "no".to_string(),
            })
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid data at offset 0: no");
    // Nor if its data isn't what its class says.
    let error = package
        .write_with(&mut |_| Ok((NativeClass::StaticMesh, 1)))
        .unwrap_err();
    assert!(error.to_string().contains("the data ends"), "{}", error);
    // Nor if it can't be written at this version.
    let mut bytes = swapped(-7);
    bytes[4] = 83;
    let package = Package::from_bytes(&bytes).unwrap();
    let error = package
        .write_with(&mut |_| Ok((NativeClass::Texture, 1)))
        .unwrap_err();
    assert!(error.to_string().contains("before version 84"), "{}", error);
}

#[test]
fn packages_find_the_classes_of_exports_that_move() {
    let packages = Packages::new(Vec::new());
    let package = Rc::new(Package::from_bytes(&swapped(-7)).unwrap());
    let written = packages.write(&package).unwrap();
    let written = Package::from_bytes(&written).unwrap();
    let offset = written.exports()[0].serial_offset.unwrap() as usize;
    assert_eq!(written.export_data(0), texture_data(offset));

    // B's state frame is malformed, so where its native data starts isn't
    // known.
    let framed = Rc::new(Package::from_bytes(&swapped_with(-7, 0x0207_0004)).unwrap());
    let error = packages.write(&framed).unwrap_err().to_string();
    assert!(error.contains("out of range"), "{}", error);

    let error = |class: i32| {
        let mut package = Package::from_bytes(&swapped(class)).unwrap();
        package.name = "MyPkg".to_string();
        packages.write(&Rc::new(package)).unwrap_err().to_string()
    };
    assert_eq!(
        error(-3),
        "invalid data at offset 0: Animation'MyPkg.B' can't be moved: \
         the native data of Engine.Animation isn't read"
    );
    assert_eq!(
        error(-5),
        "invalid data at offset 0: Gone'MyPkg.B' can't be moved: \
         its class Missing.Gone can't be found"
    );
}

/// `Other.u`: class `Middle`, a subclass of `Engine.Actor`.
fn other(dir: &Path) {
    let mut b = Builder::new();
    let engine = b.package("Engine");
    let actor = b.import("Core", "Class", engine, "Actor");
    b.class("Middle", actor, 0, Vec::new());
    b.write(dir, "Other.u");
}

fn find(package: &Package, name: &str) -> usize {
    (0..package.exports().len())
        .find(|&i| package.name(package.exports()[i].object_name) == name)
        .unwrap()
}

#[test]
fn native_classes_through_superclasses() {
    let dir = TempDir::new().unwrap();
    other(dir.path());
    let mut b = Builder::new();
    let engine = b.package("Engine");
    let texture = b.import("Core", "Class", engine, "Texture");
    let vert_mesh = b.import("Core", "Class", engine, "Animation");
    let other_package = b.package("Other");
    let middle = b.import("Core", "Class", other_package, "Middle");
    let missing = b.package("Missing");
    let gone = b.import("Core", "Class", missing, "Gone");
    // Classes: a texture, a vertex mesh, an actor through `Other.u`, one
    // with no superclass, one that's its own, and one that can't be read.
    let my_texture = b.class("MyTexture", texture, 0, Vec::new());
    let my_mesh = b.class("MyMesh", vert_mesh, 0, Vec::new());
    let mine = b.class("Mine", middle, 0, Vec::new());
    let root = b.class("Root", 0, 0, Vec::new());
    let ouroboros = b.export(0, 0, "Ouroboros");
    b.set_class(ouroboros, ouroboros, 0, Vec::new());
    let broken = b.export(0, 0, "Broken");
    b.set_data(broken, vec![0x40]);
    let objects = [
        ("Tex", texture),
        ("MyTex", my_texture),
        ("Mesh", my_mesh),
        ("Thing", mine),
        ("Orphan", gone),
        ("Rooted", root),
        ("Loop", ouroboros),
        ("Bad", broken),
    ];
    for (object, class) in objects {
        let export = b.export(class, 0, object);
        b.set_data(export, vec![0]);
    }
    let mut package = Package::from_bytes(&b.bytes()).unwrap();
    package.name = "Mod".to_string();
    let package = Rc::new(package);
    let packages = Packages::new(vec![SearchPath::from_pattern(dir.path(), "*.u")]);
    let lookup = |name: &str| packages.native_class(&package, find(&package, name));
    let known = NativeLookup::Known;
    let not_found = |path: &str| NativeLookup::NotFound(path.to_string());
    assert_eq!(lookup("MyTexture"), known(NativeClass::Class));
    assert_eq!(lookup("Tex"), known(NativeClass::Texture));
    assert_eq!(lookup("MyTex"), known(NativeClass::Texture));
    // Again, from what was found.
    assert_eq!(lookup("MyTex"), known(NativeClass::Texture));
    assert_eq!(
        lookup("Mesh"),
        NativeLookup::Unread("Engine.Animation".to_string())
    );
    assert_eq!(lookup("Thing"), known(NativeClass::Object));
    assert_eq!(lookup("Orphan"), not_found("Missing.Gone"));
    assert_eq!(lookup("Rooted"), not_found("Mod.Root"));
    assert_eq!(lookup("Loop"), not_found("Mod.Ouroboros"));
    assert_eq!(lookup("Bad"), not_found("Mod.Broken"));

    // Where their native data starts.
    let start = |name: &str, class: NativeClass| {
        packages.native_start(&package, find(&package, name), &class)
    };
    assert_eq!(start("Tex", NativeClass::Texture).unwrap(), 1);
    assert_eq!(start("MyTexture", NativeClass::Class).unwrap(), 0);
    assert!(start("Bad", NativeClass::Class).is_ok());
    // A malformed state frame.
    let framed = b.export(texture, 0, "Framed");
    b.set_data(framed, vec![0x40]);
    b.set_flags(framed, 0x0207_0004);
    let framed_class = b.export(0, 0, "FramedClass");
    b.set_data(framed_class, vec![0x40]);
    b.set_flags(framed_class, 0x0207_0004);
    let package = Rc::new(Package::from_bytes(&b.bytes()).unwrap());
    for (name, class) in [
        ("Framed", NativeClass::Texture),
        ("FramedClass", NativeClass::Class),
    ] {
        assert!(
            packages
                .native_start(&package, find(&package, name), &class)
                .is_err()
        );
    }
    let cut = b.export(texture, 0, "Cut");
    b.set_data(cut, vec![0x40]);
    let package = Rc::new(Package::from_bytes(&b.bytes()).unwrap());
    assert!(
        packages
            .native_start(&package, find(&package, "Cut"), &NativeClass::Texture)
            .is_err()
    );
    let _ = ObjectRef::Null;
}
