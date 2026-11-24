//! Exports Unreal Engine 2 package data to common file formats.
//!
//! Objects' native data, as [`usagi_pkg`] reads it, is turned into a few
//! game-independent asset models by adapters (a `Sound` into an
//! [`Audio`], a `Texture` into an [`Image`]), [`Writer`]s write a model to a
//! file format, and [`Reader`]s read one from a file format. N kinds of
//! object and M formats take N + M pieces of code, not N × M. The design is
//! in `docs/ASSET_PIPELINE.md` in the repository.
//!
//! ```no_run
//! use usagi_io::{Audio, WavWriter, Writer};
//! # fn sound() -> usagi_pkg::Sound { unimplemented!() }
//! let audio = Audio::from_sound(&sound());
//! let mut file = Vec::new();
//! WavWriter.write(&audio, &mut file)?;
//! # Ok::<(), usagi_io::Refused>(())
//! ```

mod audio;
mod dxt;
mod image;

pub use audio::{Audio, AudioFormat, WavWriter};
pub use image::{DdsWriter, Image, Level, Pixels, PngWriter, TgaReader, TgaWriter};

use std::fmt;

/// Writes one kind of asset to one file format.
pub trait Writer<Asset> {
    /// The format's file extension, without the dot: `wav`.
    fn extension(&self) -> &'static str;

    /// Writes `asset` to `out`.
    ///
    /// # Errors
    ///
    /// [`Refused`] if the format can't hold the asset without loss.
    fn write(&self, asset: &Asset, out: &mut Vec<u8>) -> Result<(), Refused>;
}

/// Reads one kind of asset from one file format.
pub trait Reader<Asset> {
    /// The format's file extension, without the dot: `tga`.
    fn extension(&self) -> &'static str;

    /// Reads an asset from a whole file.
    ///
    /// # Errors
    ///
    /// [`Unusable`] if the file isn't one the reader takes, or is cut short.
    fn read(&self, file: &[u8]) -> Result<Asset, Unusable>;
}

/// Why something can't be made into an asset: a texture into an
/// [`Image`], or a file read into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unusable {
    /// Something usagi doesn't convert, such as a format.
    Unsupported(String),
    /// Data the engine would fail on, or read past the end of.
    Malformed(String),
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unusable::Unsupported(why) | Unusable::Malformed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Unusable {}

/// Why a writer can't write an asset without loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}
