//! Every class's native data: which C++ `Serialize` writes an object's data
//! after its tagged properties ([`NativeClass`]), reading and writing it
//! ([`Native`]), and checking that what's read writes back the same.

use std::rc::Rc;

use crate::archive::{Archive, Loader, Saver, Serialize};
use crate::class::{
    Class, Const, Enum, Function, PropertyDecl, PropertyKind, State, Struct, TextBuffer,
};
use crate::level::{Level, TerrainInfo, TerrainPrimitive, TerrainSector};
use crate::media::{
    FireTexture, Font, Palette, Prefab, ProceduralSound, Sound, SoundGroup, TEXTURE_MIPS_VERSION,
    Texture,
};
use crate::mesh::{
    ConvexVolume, KMeshProps, LodMesh, MeshAnimation, Primitive, SkeletalMesh, StaticMesh,
    StaticMeshInstance, VertMesh, at_least,
};
use crate::model::{Model, Polys};
use crate::{Error, ObjectRef, Package, Packages, Result};

/// The C++ class whose `Serialize` writes an object's native data: the
/// nearest of its class and superclasses that has its own.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeClass {
    /// No native data: `UObject`, and every class that adds nothing to its
    /// tagged properties (`AActor`, `UMaterial`, `UPackage`, ...).
    Object,
    Class,
    State,
    Function,
    Struct,
    /// A property declaration, of this kind.
    Property(PropertyKind),
    Enum,
    Const,
    TextBuffer,
    Texture,
    FireTexture,
    Palette,
    Font,
    Sound,
    SoundGroup,
    ProceduralSound,
    /// `UPrimitive`, and `UMesh` and `UFluidSurfacePrimitive`, which add
    /// nothing to it.
    Primitive,
    StaticMesh,
    StaticMeshInstance,
    Model,
    Polys,
    KMeshProps,
    ConvexVolume,
    LodMesh,
    SkeletalMesh,
    VertMesh,
    MeshAnimation,
    Level,
    TerrainInfo,
    TerrainSector,
    TerrainPrimitive,
    Prefab,
}

/// What a class's native data is, as [`NativeClass::of_path`] knows it.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeKind {
    /// Read and written as this.
    Known(NativeClass),
    /// A native class whose data usagi doesn't read.
    Unread,
}

/// The native classes, by path, that have their own `Serialize`, or that
/// are known to add nothing (marked `Object`).
const CLASSES: &[(&str, Option<NativeClass>)] = &[
    ("Core.Object", Some(NativeClass::Object)),
    ("Core.Package", Some(NativeClass::Object)),
    ("Core.Class", Some(NativeClass::Class)),
    ("Core.State", Some(NativeClass::State)),
    ("Core.Function", Some(NativeClass::Function)),
    ("Core.Struct", Some(NativeClass::Struct)),
    ("Core.Enum", Some(NativeClass::Enum)),
    ("Core.Const", Some(NativeClass::Const)),
    ("Core.TextBuffer", Some(NativeClass::TextBuffer)),
    ("Engine.Actor", Some(NativeClass::Object)),
    ("Engine.Material", Some(NativeClass::Object)),
    ("Engine.Texture", Some(NativeClass::Texture)),
    ("Engine.Palette", Some(NativeClass::Palette)),
    ("Engine.Font", Some(NativeClass::Font)),
    ("Engine.Sound", Some(NativeClass::Sound)),
    ("Engine.SoundGroup", Some(NativeClass::SoundGroup)),
    ("Engine.ProceduralSound", Some(NativeClass::ProceduralSound)),
    ("Engine.Primitive", Some(NativeClass::Primitive)),
    ("Engine.Mesh", Some(NativeClass::Primitive)),
    ("Engine.FluidSurfacePrimitive", Some(NativeClass::Primitive)),
    ("Engine.StaticMesh", Some(NativeClass::StaticMesh)),
    (
        "Engine.StaticMeshInstance",
        Some(NativeClass::StaticMeshInstance),
    ),
    ("Engine.Model", Some(NativeClass::Model)),
    ("Engine.Polys", Some(NativeClass::Polys)),
    ("Engine.KMeshProps", Some(NativeClass::KMeshProps)),
    ("Engine.ConvexVolume", Some(NativeClass::ConvexVolume)),
    ("Engine.LodMesh", Some(NativeClass::LodMesh)),
    ("Engine.SkeletalMesh", Some(NativeClass::SkeletalMesh)),
    ("Engine.MeshAnimation", Some(NativeClass::MeshAnimation)),
    ("Engine.Level", Some(NativeClass::Level)),
    ("Engine.TerrainInfo", Some(NativeClass::TerrainInfo)),
    ("Engine.TerrainSector", Some(NativeClass::TerrainSector)),
    (
        "Engine.TerrainPrimitive",
        Some(NativeClass::TerrainPrimitive),
    ),
    ("Engine.VertMesh", Some(NativeClass::VertMesh)),
    ("Fire.FireTexture", Some(NativeClass::FireTexture)),
    ("Editor.Prefab", Some(NativeClass::Prefab)),
    // A legacy class whose `Serialize` writes nothing, not even its
    // properties.
    ("Engine.Animation", None),
];

impl NativeClass {
    /// What the class at `path` (`Engine.StaticMesh`, in any case) stores
    /// as native data, if it's a native class that has its own `Serialize`
    /// or is known to add nothing; `None` for any other class, whose native
    /// data is its superclass's.
    pub fn of_path(path: &str) -> Option<NativeKind> {
        let (package, name) = path.split_once('.')?;
        if package.eq_ignore_ascii_case("Core")
            && let Some(kind) = PropertyKind::of_class(name)
        {
            return Some(NativeKind::Known(NativeClass::Property(kind)));
        }
        CLASSES
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(path))
            .map(|(_, class)| match class {
                Some(class) => NativeKind::Known(class.clone()),
                None => NativeKind::Unread,
            })
    }

    /// The class's name, as the engine's C++ names it without its prefix
    /// (`StaticMesh`; `Object` for no native data).
    pub fn name(&self) -> &'static str {
        match self {
            NativeClass::Object => "Object",
            NativeClass::Class => "Class",
            NativeClass::State => "State",
            NativeClass::Function => "Function",
            NativeClass::Struct => "Struct",
            NativeClass::Property(_) => "Property",
            NativeClass::Enum => "Enum",
            NativeClass::Const => "Const",
            NativeClass::TextBuffer => "TextBuffer",
            NativeClass::Texture => "Texture",
            NativeClass::FireTexture => "FireTexture",
            NativeClass::Palette => "Palette",
            NativeClass::Font => "Font",
            NativeClass::Sound => "Sound",
            NativeClass::SoundGroup => "SoundGroup",
            NativeClass::ProceduralSound => "ProceduralSound",
            NativeClass::Primitive => "Primitive",
            NativeClass::StaticMesh => "StaticMesh",
            NativeClass::StaticMeshInstance => "StaticMeshInstance",
            NativeClass::Model => "Model",
            NativeClass::Polys => "Polys",
            NativeClass::KMeshProps => "KMeshProps",
            NativeClass::ConvexVolume => "ConvexVolume",
            NativeClass::LodMesh => "LodMesh",
            NativeClass::SkeletalMesh => "SkeletalMesh",
            NativeClass::VertMesh => "VertMesh",
            NativeClass::MeshAnimation => "MeshAnimation",
            NativeClass::Level => "Level",
            NativeClass::TerrainInfo => "TerrainInfo",
            NativeClass::TerrainSector => "TerrainSector",
            NativeClass::TerrainPrimitive => "TerrainPrimitive",
            NativeClass::Prefab => "Prefab",
        }
    }

    /// Whether its native data holds absolute file offsets (`TLazyArray`'s
    /// `SeekPos`), so that it has to be written again to be moved.
    pub fn has_absolute_offsets(&self) -> bool {
        matches!(
            self,
            NativeClass::Texture
                | NativeClass::FireTexture
                | NativeClass::Sound
                | NativeClass::StaticMesh
                | NativeClass::Model
                | NativeClass::SkeletalMesh
        )
    }

    /// An empty value of the class's native data, to load into.
    fn empty(&self) -> Native {
        match self {
            NativeClass::Object => Native::None,
            NativeClass::Class => Native::Class(Box::default()),
            NativeClass::State => Native::State(Box::default()),
            NativeClass::Function => Native::Function(Box::default()),
            NativeClass::Struct => Native::Struct(Box::default()),
            NativeClass::Property(kind) => Native::Property(Box::new(PropertyDecl {
                kind: kind.clone(),
                ..PropertyDecl::default()
            })),
            NativeClass::Enum => Native::Enum(Enum::default()),
            NativeClass::Const => Native::Const(Const::default()),
            NativeClass::TextBuffer => Native::TextBuffer(TextBuffer::default()),
            NativeClass::Texture => Native::Texture(Texture::default()),
            NativeClass::FireTexture => Native::FireTexture(FireTexture::default()),
            NativeClass::Palette => Native::Palette(Palette::default()),
            NativeClass::Font => Native::Font(Font::default()),
            NativeClass::Sound => Native::Sound(Sound::default()),
            NativeClass::SoundGroup => Native::SoundGroup(SoundGroup::default()),
            NativeClass::ProceduralSound => Native::ProceduralSound(ProceduralSound::default()),
            NativeClass::Primitive => Native::Primitive(Primitive::default()),
            NativeClass::StaticMesh => Native::StaticMesh(Box::default()),
            NativeClass::StaticMeshInstance => {
                Native::StaticMeshInstance(StaticMeshInstance::default())
            }
            NativeClass::Model => Native::Model(Box::default()),
            NativeClass::Polys => Native::Polys(Polys::default()),
            NativeClass::KMeshProps => Native::KMeshProps(KMeshProps::default()),
            NativeClass::ConvexVolume => Native::ConvexVolume(ConvexVolume::default()),
            NativeClass::LodMesh => Native::LodMesh(Box::default()),
            NativeClass::SkeletalMesh => Native::SkeletalMesh(Box::default()),
            NativeClass::VertMesh => Native::VertMesh(Box::default()),
            NativeClass::MeshAnimation => Native::MeshAnimation(MeshAnimation::default()),
            NativeClass::Level => Native::Level(Box::default()),
            NativeClass::TerrainInfo => Native::TerrainInfo(TerrainInfo::default()),
            NativeClass::TerrainSector => Native::TerrainSector(TerrainSector::default()),
            NativeClass::TerrainPrimitive => Native::TerrainPrimitive(TerrainPrimitive::default()),
            NativeClass::Prefab => Native::Prefab(Prefab::default()),
        }
    }
}

/// An object's native data, as its [`NativeClass`] stores it.
#[derive(Debug, Clone, PartialEq)]
pub enum Native {
    None,
    Class(Box<Class>),
    State(Box<State>),
    Function(Box<Function>),
    Struct(Box<Struct>),
    Property(Box<PropertyDecl>),
    Enum(Enum),
    Const(Const),
    TextBuffer(TextBuffer),
    Texture(Texture),
    FireTexture(FireTexture),
    Palette(Palette),
    Font(Font),
    Sound(Sound),
    SoundGroup(SoundGroup),
    ProceduralSound(ProceduralSound),
    Primitive(Primitive),
    StaticMesh(Box<StaticMesh>),
    StaticMeshInstance(StaticMeshInstance),
    Model(Box<Model>),
    Polys(Polys),
    KMeshProps(KMeshProps),
    ConvexVolume(ConvexVolume),
    LodMesh(Box<LodMesh>),
    SkeletalMesh(Box<SkeletalMesh>),
    VertMesh(Box<VertMesh>),
    MeshAnimation(MeshAnimation),
    Level(Box<Level>),
    TerrainInfo(TerrainInfo),
    TerrainSector(TerrainSector),
    TerrainPrimitive(TerrainPrimitive),
    Prefab(Prefab),
}

impl Native {
    /// Reads into `self` when `ar` is loading; writes `self` when it's
    /// saving.
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        match self {
            Native::None => Ok(()),
            Native::Class(v) => v.serialize(ar),
            Native::State(v) => v.serialize(ar),
            Native::Function(v) => v.serialize(ar),
            Native::Struct(v) => v.serialize(ar),
            Native::Property(v) => v.serialize(ar),
            Native::Enum(v) => v.serialize(ar),
            Native::Const(v) => v.serialize(ar),
            Native::TextBuffer(v) => v.serialize(ar),
            Native::Texture(v) => {
                at_least(ar, TEXTURE_MIPS_VERSION, "textures")?;
                v.serialize(ar)
            }
            Native::FireTexture(v) => {
                at_least(ar, TEXTURE_MIPS_VERSION, "textures")?;
                v.serialize(ar)
            }
            Native::Palette(v) => v.serialize(ar),
            Native::Font(v) => v.serialize(ar),
            Native::Sound(v) => v.serialize(ar),
            Native::SoundGroup(v) => v.serialize(ar),
            Native::ProceduralSound(v) => v.serialize(ar),
            Native::Primitive(v) => v.serialize(ar),
            Native::StaticMesh(v) => v.serialize(ar),
            Native::StaticMeshInstance(v) => v.serialize(ar),
            Native::Model(v) => v.serialize(ar),
            Native::Polys(v) => v.serialize(ar),
            Native::KMeshProps(v) => v.serialize(ar),
            Native::ConvexVolume(v) => v.serialize(ar),
            Native::LodMesh(v) => v.serialize(ar),
            Native::SkeletalMesh(v) => v.serialize(ar),
            Native::VertMesh(v) => v.serialize(ar),
            Native::MeshAnimation(v) => v.serialize(ar),
            Native::Level(v) => v.serialize(ar),
            Native::TerrainInfo(v) => v.serialize(ar),
            Native::TerrainSector(v) => v.serialize(ar),
            Native::TerrainPrimitive(v) => v.serialize(ar),
            Native::Prefab(v) => v.serialize(ar),
        }
    }

    /// Writes it for a package with these versions, as it will be at
    /// `file_offset` in the file (which its lazy arrays' `SeekPos` depend
    /// on).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidData`] if its class isn't read at this version.
    pub fn write(
        &mut self,
        version: u16,
        licensee_version: u16,
        file_offset: usize,
    ) -> Result<Vec<u8>> {
        let mut saver = Saver::new(version, licensee_version).at(file_offset);
        self.serialize(&mut saver)?;
        Ok(saver.into_bytes())
    }
}

/// An export's native data, and where it is in the export's data.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeData {
    pub native: Native,
    /// Where it starts: after the state frame and tagged properties, or
    /// for a class, after its state frame.
    pub start: usize,
    /// Where it ends: the end of the data, or for a class, where its
    /// default properties start.
    pub end: usize,
}

impl Package {
    /// Export `export`'s native data, read from `start` as `class`'s. For
    /// any class but a class, it must end where the data does, as the
    /// engine checks.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidData`] if the data is malformed, has bytes after
    /// the native data, or `class` isn't read at the package's version.
    ///
    /// # Panics
    ///
    /// If `export` is out of range.
    pub fn native_data(
        &self,
        export: usize,
        class: &NativeClass,
        start: usize,
    ) -> Result<NativeData> {
        let data = self.export_data(export);
        let file_offset = self.exports()[export].serial_offset.unwrap_or(0) as usize;
        let mut loader = Loader::new(self, data).at(file_offset);
        loader.seek(start);
        let mut native = class.empty();
        native
            .serialize(&mut loader)
            .map_err(crate::properties::data_error)?;
        let end = loader.pos();
        if *class != NativeClass::Class && end != data.len() {
            return Err(Error::InvalidData {
                offset: end,
                reason: format!(
                    "{} bytes are left after the native data ({})",
                    data.len() - end,
                    class.name()
                ),
            });
        }
        Ok(NativeData { native, start, end })
    }

    /// Reads export `export`'s native data as `class`'s, from `start`, and
    /// writes it again where it is, to check that it gives the same bytes.
    /// Then writes it at another offset ([`ROUND_TRIP_MOVE`] bytes further), reads that
    /// back, and writes it where it was, to check that it can be moved: the
    /// bytes must be the same again.
    ///
    /// # Errors
    ///
    /// As [`Package::native_data`].
    pub fn round_trip(
        &self,
        export: usize,
        class: &NativeClass,
        start: usize,
    ) -> Result<RoundTrip> {
        let version = self.summary.version();
        let licensee_version = self.summary.licensee_version();
        let mut read = self.native_data(export, class, start)?;
        let offset = self.exports()[export].serial_offset.unwrap_or(0) as usize + start;
        let original = &self.export_data(export)[start..read.end];
        // What was read at this version can be written at it.
        let written = |native: &mut Native, offset| {
            native
                .write(version, licensee_version, offset)
                .expect("native data that was read")
        };
        let mut difference = first_difference(original, &written(&mut read.native, offset));
        if difference.is_none() {
            let moved = written(&mut read.native, offset + ROUND_TRIP_MOVE);
            let mut loader = Loader::new(self, &moved).at(offset + ROUND_TRIP_MOVE);
            let mut back = class.empty();
            // If what was written can't be read back, it's different from
            // the start.
            difference = back.serialize(&mut loader).map_or(Some(0), |()| {
                first_difference(original, &written(&mut back, offset))
            });
        }
        Ok(RoundTrip {
            size: original.len(),
            difference: difference.map(|i| start + i),
        })
    }
}

/// How far [`Package::round_trip`] moves native data to check that it can
/// be moved.
pub const ROUND_TRIP_MOVE: usize = 0x10001;

/// What [`Package::round_trip`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundTrip {
    /// The size of the native data that was read.
    pub size: usize,
    /// Where in the export's data the bytes written first differ from the
    /// ones read; `None` if they're the same.
    pub difference: Option<usize>,
}

/// Where two byte strings first differ, if they do.
pub(crate) fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => Some(i),
        None if a.len() != b.len() => Some(a.len().min(b.len())),
        None => None,
    }
}

/// What an export's native data is, as far as its classes can be found.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeLookup {
    /// Read and written as this class's.
    Known(NativeClass),
    /// Its class, or a superclass, is a native class (this one, by path)
    /// whose native data usagi doesn't read.
    Unread(String),
    /// Its class, or a superclass, can't be found: this one, by path.
    NotFound(String),
}

/// How many superclasses are followed before giving up.
const MAX_SUPERCLASSES: usize = 64;

impl Packages {
    /// The native class of export `export` of `package`: the nearest of
    /// its class and superclasses that stores native data of its own, or
    /// is known to add none.
    pub fn native_class(&self, package: &Rc<Package>, export: usize) -> NativeLookup {
        let class = package.exports()[export].class;
        if class == ObjectRef::Null {
            return NativeLookup::Known(NativeClass::Class);
        }
        let path = package.full_path(class).expect("a class");
        if let Some(lookup) = self.natives.borrow().get(&path.to_lowercase()) {
            return lookup.clone();
        }
        let lookup = self.find_native_class(package, class, path.clone());
        self.natives
            .borrow_mut()
            .insert(path.to_lowercase(), lookup.clone());
        lookup
    }

    fn find_native_class(
        &self,
        package: &Rc<Package>,
        class: ObjectRef,
        path: String,
    ) -> NativeLookup {
        let mut path = path;
        let mut scope = self.resolve(package, class);
        for _ in 0..MAX_SUPERCLASSES {
            match NativeClass::of_path(&path) {
                Some(NativeKind::Known(class)) => return NativeLookup::Known(class),
                Some(NativeKind::Unread) => return NativeLookup::Unread(path),
                None => {}
            }
            let Some(s) = scope else {
                return NativeLookup::NotFound(path);
            };
            let super_field = match s.package.structure(s.export) {
                Ok(Some(structure)) => structure.field.super_field,
                _ => return NativeLookup::NotFound(path),
            };
            let Some(super_path) = s.package.full_path(super_field) else {
                // Only `Core.Object` has no superclass.
                return NativeLookup::NotFound(path);
            };
            path = super_path;
            scope = self.resolve(&s.package, super_field);
        }
        NativeLookup::NotFound(path)
    }

    /// Where export `export`'s native data starts, for `class`: after its
    /// state frame, and unless it's a class, its tagged properties, read
    /// by their declarations.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidData`] if its state frame or properties are
    /// malformed.
    pub fn native_start(
        &self,
        package: &Rc<Package>,
        export: usize,
        class: &NativeClass,
    ) -> Result<usize> {
        if *class == NativeClass::Class {
            return Ok(package.state_frame(export)?.1);
        }
        let mut properties = self.properties(package, export)?;
        for property in properties.by_ref() {
            property?;
        }
        Ok(properties.offset())
    }
}

#[cfg(test)]
mod tests {
    use super::first_difference;

    #[test]
    fn where_bytes_first_differ() {
        assert_eq!(first_difference(&[1, 2], &[1, 2]), None);
        assert_eq!(first_difference(&[1, 2], &[1, 3]), Some(1));
        assert_eq!(first_difference(&[1, 2], &[1, 2, 3]), Some(2));
        assert_eq!(first_difference(&[1, 2, 3], &[1]), Some(1));
    }
}
