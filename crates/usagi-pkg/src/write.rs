//! Writing packages, laid out as `UObject::SavePackage` lays them out
//! (`Core/Src/UnObj.cpp`): the summary, the names, every export's data in
//! the order of the export table, the imports and the exports.

use std::rc::Rc;

use crate::archive::compact_index;
use crate::native::{NativeClass, NativeLookup};
use crate::{Error, Package, Packages, Result};

/// The first version with a GUID and generations, rather than a heritage
/// table: the oldest that's written.
pub const WRITE_MIN_VERSION: u16 = 68;

impl Package {
    /// The package's bytes, laid out as the engine saves it, with its
    /// names, imports and exports in the same order, and so at the same
    /// indices: object references and names in exports' data stay right.
    ///
    /// An export's data is copied, unless it moves and its native data
    /// holds absolute file offsets: then it's written again at its new
    /// offset. `native` gives the native class of an export that moves, and
    /// where its native data starts (see
    /// [`Packages::native_class`] and [`Packages::native_start`]); an error
    /// from it stops the write, since an export can't be moved if its
    /// native data isn't known.
    ///
    /// A package saved by the engine, written with no changes, gives the
    /// same bytes.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidData`] if the package is older than
    /// [`WRITE_MIN_VERSION`], or an export that moves can't be written
    /// again.
    pub fn write_with(
        &self,
        native: &mut dyn FnMut(usize) -> Result<(NativeClass, usize)>,
    ) -> Result<Vec<u8>> {
        let version = self.summary.version();
        if version < WRITE_MIN_VERSION {
            return Err(Error::InvalidData {
                offset: 0,
                reason: format!(
                    "packages before version {} aren't written",
                    WRITE_MIN_VERSION
                ),
            });
        }
        let mut out = Vec::with_capacity(self.size);
        let summary_size = 56 + 8 * self.summary.generations.len();
        out.resize(summary_size, 0);

        let name_offset = out.len();
        for (i, name) in self.names().iter().enumerate() {
            out.extend_from_slice(self.raw_name(i));
            out.extend_from_slice(&name.flags.to_le_bytes());
        }

        let mut offsets = Vec::with_capacity(self.exports().len());
        for (i, export) in self.exports().iter().enumerate() {
            let Some(old_offset) = export.serial_offset else {
                offsets.push(None);
                continue;
            };
            let offset = out.len();
            let data = self.export_data(i);
            if offset == old_offset as usize {
                out.extend_from_slice(data);
            } else {
                let (class, start) = native(i)?;
                if class.has_absolute_offsets() {
                    let mut read = self.native_data(i, &class, start)?;
                    let written = read
                        .native
                        .write(version, self.summary.licensee_version(), offset + start)
                        .expect("native data that was read at this version");
                    out.extend_from_slice(&data[..start]);
                    out.extend_from_slice(&written);
                    out.extend_from_slice(&data[read.end..]);
                } else {
                    out.extend_from_slice(data);
                }
            }
            offsets.push(Some(offset));
        }

        let import_offset = out.len();
        for import in self.imports() {
            out.extend(index(import.class_package));
            out.extend(index(import.class_name));
            out.extend_from_slice(&import.outer.raw().to_le_bytes());
            out.extend(index(import.object_name));
        }

        let export_offset = out.len();
        for (export, offset) in self.exports().iter().zip(&offsets) {
            out.extend(compact_index(export.class.raw()));
            out.extend(compact_index(export.super_struct.raw()));
            out.extend_from_slice(&export.outer.raw().to_le_bytes());
            out.extend(index(export.object_name));
            out.extend_from_slice(&export.flags.0.to_le_bytes());
            out.extend(compact_index(to_i32(export.serial_size as usize)));
            if let Some(offset) = offset {
                out.extend(compact_index(to_i32(*offset)));
            }
        }

        let s = &self.summary;
        let mut summary = Vec::with_capacity(summary_size);
        for value in [
            s.tag,
            s.file_version,
            s.package_flags.0,
            s.name_count,
            to_u32(name_offset),
            s.export_count,
            to_u32(export_offset),
            s.import_count,
            to_u32(import_offset),
            s.guid.a,
            s.guid.b,
            s.guid.c,
            s.guid.d,
            to_u32(s.generations.len()),
        ] {
            summary.extend_from_slice(&value.to_le_bytes());
        }
        for generation in &s.generations {
            summary.extend_from_slice(&generation.export_count.to_le_bytes());
            summary.extend_from_slice(&generation.name_count.to_le_bytes());
        }
        out[..summary_size].copy_from_slice(&summary);
        Ok(out)
    }
}

/// A name or count as a compact index.
fn index(value: usize) -> Vec<u8> {
    compact_index(to_i32(value))
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).expect("a package smaller than 2 GiB")
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("a package smaller than 4 GiB")
}

impl Packages {
    /// Package `package`'s bytes, as [`Package::write_with`] writes them,
    /// with the native classes of exports that move found in these
    /// packages.
    ///
    /// # Errors
    ///
    /// As [`Package::write_with`]; and [`Error::InvalidData`] if an export
    /// that moves is of a class that can't be found, or whose native data
    /// isn't read.
    pub fn write(&self, package: &Rc<Package>) -> Result<Vec<u8>> {
        package.write_with(&mut |export| {
            let cant_move = |reason: String| Error::InvalidData {
                offset: 0,
                reason: format!(
                    "{} can't be moved: {}",
                    package.reference(crate::ObjectRef::Export(export)),
                    reason
                ),
            };
            match self.native_class(package, export) {
                NativeLookup::Known(class) => {
                    let start = self.native_start(package, export, &class)?;
                    Ok((class, start))
                }
                NativeLookup::Unread(path) => {
                    Err(cant_move(format!("the native data of {} isn't read", path)))
                }
                NativeLookup::NotFound(path) => {
                    Err(cant_move(format!("its class {} can't be found", path)))
                }
            }
        })
    }
}
