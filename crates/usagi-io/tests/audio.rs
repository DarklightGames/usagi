use usagi_io::{Audio, AudioFormat, Refused, WavWriter, Writer};
use usagi_pkg::Sound;
use usagi_pkg::archive::LazyArray;

/// A minimal WAV file: a RIFF header, `fmt ` and `data` with two samples.
fn wav() -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(40u32.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend([1, 0, 1, 0]); // PCM, mono
    out.extend(44100u32.to_le_bytes());
    out.extend(88200u32.to_le_bytes());
    out.extend([2, 0, 16, 0]); // block align, bits per sample
    out.extend(b"data");
    out.extend(4u32.to_le_bytes());
    out.extend([0xFF, 0x7F, 0x00, 0x80]);
    out
}

fn sound(data: Vec<u8>) -> Sound {
    Sound {
        data: LazyArray {
            seek_pos: None,
            items: data,
        },
        ..Sound::default()
    }
}

#[test]
fn a_wav_file_is_written_as_it_is() {
    let audio = Audio::from_sound(&sound(wav()));
    assert_eq!(audio.format, AudioFormat::Wav);
    let mut out = Vec::new();
    WavWriter.write(&audio, &mut out).unwrap();
    assert_eq!(out, wav());
    assert_eq!(WavWriter.extension(), "wav");
}

#[test]
fn other_sound_files_are_refused() {
    // An MP3 with an ID3 tag; a RIFF file that isn't a WAV; too short; none.
    let mut avi = wav();
    avi[8..12].copy_from_slice(b"AVI ");
    for data in [
        b"ID3\x04\0\0\0\0\0\0".to_vec(),
        avi,
        b"RIFF".to_vec(),
        Vec::new(),
    ] {
        let audio = Audio::new(data.clone());
        assert_eq!(audio.format, AudioFormat::Unknown, "{:?}", data);
        let error = WavWriter.write(&audio, &mut Vec::new()).unwrap_err();
        assert_eq!(
            error,
            Refused("it isn't a WAV file, and converting it isn't supported".to_string())
        );
        assert_eq!(
            error.to_string(),
            "it isn't a WAV file, and converting it isn't supported"
        );
    }
}
