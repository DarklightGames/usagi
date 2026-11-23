use std::fmt;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;

use crate::tables::{self, Tables};
use crate::{Export, Import, NameEntry, ObjectRef, Result, Summary, uz2};

/// A package file: its summary, its name, import and export tables, and its
/// exports' data.
///
/// The tables are checked when the package is read: every name and object
/// reference is in range, no object is inside itself, and every export's
/// data is inside the file. So the methods that look things up can't fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The package's name, which the engine takes from the file name (see
    /// [`package_name`]). It names this package's exports: their paths in
    /// the file don't include it. Empty for a package read from bytes, until
    /// it's set.
    pub name: String,
    pub summary: Summary,
    /// The size of the package in bytes (once decompressed, for a `.uz2`).
    pub size: usize,
    /// The size of the `.uz2` file, if the package was compressed.
    pub compressed_size: Option<usize>,
    tables: Tables,
    bytes: Bytes,
}

/// The bytes of the whole file: in memory, or the file mapped into it.
/// Printed by `Debug` as their length only.
#[derive(Clone)]
enum Bytes {
    Owned(Vec<u8>),
    Mapped(Arc<Mmap>),
}

impl Bytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Bytes::Owned(bytes) => bytes,
            Bytes::Mapped(map) => map,
        }
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Bytes {}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.as_slice().len())
    }
}

/// A package's name, as the engine takes it from the file name: the part
/// before the first `.` (so `Engine` for `Engine.u` and `Engine.u.uz2`).
pub fn package_name(path: &Path) -> String {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    file_name.split('.').next().unwrap_or_default().to_string()
}

impl Package {
    /// Reads a package file into memory. A file whose extension is `.uz2`
    /// (in any case) is decompressed first: like the engine, it's the
    /// extension that says whether a file is compressed.
    ///
    /// The package keeps its own copy of the file, so the file can be
    /// changed, replaced or deleted afterwards, by this program or another.
    /// For reading many large packages quickly, see [`Package::map`].
    pub fn read(path: &Path) -> Result<Package> {
        let package = if crate::edit::is_uz2(path) {
            Package::from_uz2_bytes(&std::fs::read(path)?)?
        } else {
            Package::parse(Bytes::Owned(std::fs::read(path)?))?
        };
        Ok(Package::named(package, path))
    }

    /// Reads a package file by mapping it into memory, rather than copying
    /// it: only the parts that are used are loaded, from the system's file
    /// cache, so many large packages can be open at once. A `.uz2` file is
    /// decompressed into memory, as [`Package::read`] does.
    ///
    /// Use it for short, read-only work on files nothing else is writing,
    /// such as exporting. The mapping lasts as long as the package or any
    /// clone of it, and while it does, on Windows the file can't be
    /// replaced, truncated or (usually) deleted, by this program or another.
    ///
    /// # Safety
    ///
    /// The file must not be changed while the mapping lasts, by this
    /// program or another. On Linux, nothing stops that: the package's data
    /// would change under it (which Rust's guarantees don't allow), and if
    /// the file is truncated, reading the part that's gone kills the
    /// program (`SIGBUS`). An I/O error while reading the mapped file (a
    /// network drive that goes away) is a crash too, not an error.
    pub unsafe fn map(path: &Path) -> Result<Package> {
        if crate::edit::is_uz2(path) {
            return Package::read(path);
        }
        // SAFETY: passed on to the caller. (An empty file maps to an empty
        // slice.)
        let map = File::open(path).and_then(|file| unsafe { Mmap::map(&file) })?;
        let package = Package::parse(Bytes::Mapped(Arc::new(map)))?;
        Ok(Package::named(package, path))
    }

    /// The package with its name taken from its file's.
    fn named(package: Package, path: &Path) -> Package {
        Package {
            name: package_name(path),
            ..package
        }
    }

    /// Reads a package from the bytes of the whole file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Package> {
        Package::parse(Bytes::Owned(bytes.to_vec()))
    }

    /// Reads a package from the bytes of a whole `.uz2` file.
    pub fn from_uz2_bytes(bytes: &[u8]) -> Result<Package> {
        let package = Package::parse(Bytes::Owned(uz2::decompress(bytes)?))?;
        Ok(Package {
            compressed_size: Some(bytes.len()),
            ..package
        })
    }

    fn parse(bytes: Bytes) -> Result<Package> {
        let data = bytes.as_slice();
        let summary = Summary::read(data)?;
        let tables = tables::read(data, &summary)?;
        Ok(Package {
            name: String::new(),
            summary,
            size: data.len(),
            compressed_size: None,
            tables,
            bytes,
        })
    }

    /// The name table.
    pub fn names(&self) -> &[NameEntry] {
        &self.tables.names
    }

    /// The import table. Import `i` is [`ObjectRef::Import(i)`], stored on
    /// disk as `-i - 1`.
    pub fn imports(&self) -> &[Import] {
        &self.tables.imports
    }

    /// The export table. Export `i` is [`ObjectRef::Export(i)`], stored on
    /// disk as `i + 1`.
    pub fn exports(&self) -> &[Export] {
        &self.tables.exports
    }

    /// An export's data: `SerialSize` bytes at `SerialOffset`. Empty if it
    /// has none.
    ///
    /// # Panics
    ///
    /// If `export` is out of range.
    pub fn export_data(&self, export: usize) -> &[u8] {
        let export = &self.tables.exports[export];
        match export.serial_offset {
            Some(offset) => {
                &self.bytes.as_slice()[offset as usize..][..export.serial_size as usize]
            }
            None => &[],
        }
    }

    /// The name with this index. Every name index in an [`Import`] or
    /// [`Export`] of this package is valid.
    ///
    /// # Panics
    ///
    /// If `index` is out of range.
    pub fn name(&self, index: usize) -> &str {
        &self.tables.names[index].name
    }

    /// An object's name; `None` for [`ObjectRef::Null`].
    pub fn object_name(&self, object: ObjectRef) -> Option<&str> {
        let index = match object {
            ObjectRef::Null => return None,
            ObjectRef::Export(i) => self.tables.exports[i].object_name,
            ObjectRef::Import(i) => self.tables.imports[i].object_name,
        };
        Some(self.name(index))
    }

    /// An object's path: its outers' names and its own, outermost first,
    /// joined with `.`; `None` for [`ObjectRef::Null`].
    ///
    /// An import's path starts with the package it's in. An export's path
    /// doesn't include this package's own name, which isn't stored in the
    /// file (the engine takes it from the file name).
    pub fn path(&self, object: ObjectRef) -> Option<String> {
        let mut parts = Vec::new();
        let mut current = object;
        while let Some(name) = self.object_name(current) {
            parts.push(name);
            current = self.outer(current);
        }
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        Some(parts.join("."))
    }

    /// An object's outer: the object it's inside. [`ObjectRef::Null`] for a
    /// top-level import (a package), an export directly in this package, or
    /// [`ObjectRef::Null`] itself.
    pub fn outer(&self, object: ObjectRef) -> ObjectRef {
        self.tables.outer(object)
    }

    /// The outermost object an object is in: for an import, the package
    /// it comes from (a top-level `Core.Package` import, which is a file);
    /// for an export directly in this package, the export itself. The object
    /// itself if it has no outer; [`ObjectRef::Null`] for
    /// [`ObjectRef::Null`].
    pub fn top_level(&self, object: ObjectRef) -> ObjectRef {
        let mut current = object;
        loop {
            match self.outer(current) {
                ObjectRef::Null => return current,
                outer => current = outer,
            }
        }
    }

    /// The name of an export's class: `Class` if the export is itself a
    /// class (`ClassIndex` is 0).
    pub fn class_name(&self, export: &Export) -> &str {
        self.object_name(export.class).unwrap_or("Class")
    }

    /// The name of an object's class: an export's (see
    /// [`class_name`](Package::class_name)) or an import's; `None` for
    /// [`ObjectRef::Null`].
    pub fn object_class_name(&self, object: ObjectRef) -> Option<&str> {
        match object {
            ObjectRef::Null => None,
            ObjectRef::Export(i) => Some(self.class_name(&self.tables.exports[i])),
            ObjectRef::Import(i) => Some(self.name(self.tables.imports[i].class_name)),
        }
    }

    /// An object's path with its package: for an export, this package's
    /// [`name`](Package::name) and its [`path`](Package::path)
    /// (`MyPkg.Walls.Brick`); for an import, its path. `None` for
    /// [`ObjectRef::Null`].
    pub fn full_path(&self, object: ObjectRef) -> Option<String> {
        let path = self.path(object)?;
        Some(match object {
            ObjectRef::Export(_) => format!("{}.{}", self.name, path),
            _ => path,
        })
    }

    /// An object as UnrealScript and T3D refer to it: its class name and
    /// its [`full_path`](Package::full_path), as in
    /// `Texture'MyPkg.Walls.Brick'`; `None` (the T3D text for no object) for
    /// [`ObjectRef::Null`].
    pub fn reference(&self, object: ObjectRef) -> String {
        match (self.object_class_name(object), self.full_path(object)) {
            (Some(class), Some(path)) => format!("{}'{}'", class, path),
            _ => "None".to_string(),
        }
    }
}
