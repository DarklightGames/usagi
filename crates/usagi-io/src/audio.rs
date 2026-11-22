//! Sounds: the [`Audio`] model, and writing it as WAV.

use usagi_pkg::Sound;

use crate::{Refused, Writer};

/// A sound file's format, from its contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// A RIFF file whose form is `WAVE`.
    Wav,
    /// Anything else.
    Unknown,
}

/// A sound: a whole sound file, as the engine keeps it, and its format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audio {
    pub format: AudioFormat,
    pub data: Vec<u8>,
}

impl Audio {
    /// A sound file, its format worked out from its contents.
    pub fn new(data: Vec<u8>) -> Audio {
        let is_wav = data.get(0..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WAVE");
        let format = if is_wav {
            AudioFormat::Wav
        } else {
            AudioFormat::Unknown
        };
        Audio { format, data }
    }

    /// A sound's file (`USound::Data`). Its `FileType` isn't used: the
    /// engine only tells console data (`PS2`) apart by it, and gives the
    /// rest to the audio driver, which reads the file's own header.
    pub fn from_sound(sound: &Sound) -> Audio {
        Audio::new(sound.data.items.clone())
    }
}

/// Writes WAV files. A sound that's a WAV file already is written as it
/// is; any other is refused, since converting it isn't supported.
#[derive(Debug, Clone, Copy, Default)]
pub struct WavWriter;

impl Writer<Audio> for WavWriter {
    fn extension(&self) -> &'static str {
        "wav"
    }

    fn write(&self, audio: &Audio, out: &mut Vec<u8>) -> Result<(), Refused> {
        match audio.format {
            AudioFormat::Wav => {
                out.extend_from_slice(&audio.data);
                Ok(())
            }
            AudioFormat::Unknown => Err(Refused(
                "it isn't a WAV file, and converting it isn't supported".to_string(),
            )),
        }
    }
}
