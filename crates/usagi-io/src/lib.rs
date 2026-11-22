//! Exports Unreal Engine 2 package data to common file formats.
//!
//! Objects' native data, as [`usagi_pkg`] reads it, is turned into a few
//! game-independent asset models by adapters (a `Sound` into an
//! [`Audio`]), and [`Writer`]s write a model to a file format. N kinds of
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

pub use audio::{Audio, AudioFormat, WavWriter};

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

/// Why a writer can't write an asset without loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}
