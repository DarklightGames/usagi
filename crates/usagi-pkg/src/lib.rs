//! Reads Unreal Engine 2 package files (`.u`, `.utx`, `.usx`, `.ukx`,
//! `.uax`, `.ut2`, ...), as written by UT2004 and other UE2 games, and their
//! `.uz2` compressed form.
//!
//! The format is described in `docs/PACKAGE_FORMAT.md` in the repository.
//! The summary and the name, import and export tables are read, and in each
//! export's data, its tagged properties ([`Package::properties`]) and its
//! native data, for every class that has any in UT2004's and Red
//! Orchestra's packages: classes, states, functions and property
//! declarations, textures, sounds, fonts, meshes, animations, BSP models,
//! levels and terrain ([`Package::native_data`], [`Native`]).
//!
//! Native data is read and written by one function per type, as the engine
//! does ([`archive::Serialize`]), so what's read writes back the same
//! ([`Package::round_trip`]). [`Packages`] finds the packages a package
//! refers to, the native class whose data an object has
//! ([`Packages::native_class`]), and reads tagged properties by their
//! declarations ([`Packages::properties`]). A package can be written again,
//! as the engine lays it out ([`Package::write_with`], [`Packages::write`]).
//! [`diff`] compares two versions of a package.
//!
//! ```no_run
//! let package = usagi_pkg::Package::read("System/Engine.u".as_ref())?;
//! println!("version {}", package.summary.version());
//! # Ok::<(), usagi_pkg::Error>(())
//! ```

pub mod archive;
mod class;
pub mod diff;
mod edit;
mod error;
mod flags;
pub mod level;
pub mod math;
mod media;
pub mod mesh;
pub mod model;
mod native;
mod package;
mod packages;
mod pattern;
mod properties;
mod reader;
pub mod script;
mod summary;
mod tables;
pub mod uz2;
pub mod volatile;
mod write;

pub use class::{
    Class, Const, Dependency, Enum, Field, Function, PropertyDecl, PropertyKind, State, StateFrame,
    Struct, TextBuffer,
};
pub use edit::set_package_flags;
pub use error::{Error, Result};
pub use flags::{ObjectFlags, PackageFlags};
pub use media::{
    CharRemap, FONT_CHARACTERS_LICENSEE_VERSION, FireTexture, Font, FontCharacter, Mipmap,
    OldFontCharacter, OldFontPage, Palette, Prefab, ProceduralSound,
    SOUND_GROUP_SOUNDS_LICENSEE_VERSION, Sound, SoundGroup, Spark, TEXTURE_MIPS_VERSION, Texture,
};
pub use native::{
    Native, NativeClass, NativeData, NativeKind, NativeLookup, ROUND_TRIP_MOVE, RoundTrip,
};
pub use package::{Package, package_name};
pub use packages::{Decl, Lookup, Packages, Scope, SearchPath, Type};
pub use pattern::ObjectPattern;
pub use properties::{Properties, Property, PropertyType, TAGGED_STRUCTS_LICENSEE_VERSION, Value};
pub use summary::{
    Generation, Guid, Heritage, PACKAGE_FILE_TAG, Summary, TRIPWIRE_PACKAGE_FILE_TAG,
};
pub use tables::{Export, Import, NameEntry, ObjectRef};
pub use write::WRITE_MIN_VERSION;
