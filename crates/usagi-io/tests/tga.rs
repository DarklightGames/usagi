use usagi_io::{Image, Level, Reader, TgaReader, TgaWriter, Unusable, Writer};
use usagi_pkg::TextureFormat;

/// Bottom-left origin (descriptor bits 4 and 5 clear).
const BOTTOM: u8 = 0;
/// Top-left origin.
const TOP: u8 = 0x20;
/// Bottom-right origin.
const RIGHT: u8 = 0x10;

/// A TGA file without a colour map.
fn tga(kind: u8, width: u16, height: u16, bits: u8, descriptor: u8, data: &[u8]) -> Vec<u8> {
    mapped(kind, width, height, bits, descriptor, None, data)
}

/// A TGA file with a 2-byte ID field and, if given, a colour map: its
/// first index, entries' bits, entries' bytes and number of entries.
fn mapped(
    kind: u8,
    width: u16,
    height: u16,
    bits: u8,
    descriptor: u8,
    map: Option<(u16, u8, &[u8], u16)>,
    data: &[u8],
) -> Vec<u8> {
    let (first, entry_bits, entries, length) = map.unwrap_or((0, 0, &[], 0));
    let mut file = vec![2, u8::from(map.is_some()), kind];
    file.extend(first.to_le_bytes());
    file.extend(length.to_le_bytes());
    file.push(entry_bits);
    file.extend([0; 4]); // Origin.
    file.extend(width.to_le_bytes());
    file.extend(height.to_le_bytes());
    file.extend([bits, descriptor]);
    file.extend([0xAA, 0xAA]);
    file.extend(entries);
    file.extend(data);
    file
}

fn read(file: &[u8]) -> Result<Image, Unusable> {
    TgaReader.read(file)
}

fn one_level(format: TextureFormat, width: usize, height: usize, data: &[u8]) -> Image {
    Image {
        format,
        levels: vec![Level {
            width,
            height,
            data: data.to_vec(),
        }],
        palette: Vec::new(),
    }
}

/// An RGBA8 image of RGBA pixels (stored blue first, as `FColor` is).
fn rgba8(width: usize, height: usize, pixels: &[[u8; 4]]) -> Image {
    let data: Vec<u8> = pixels
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    one_level(TextureFormat::Rgba8, width, height, &data)
}

#[test]
fn true_colour_tga_follows_its_origin() {
    assert_eq!(TgaReader.extension(), "tga");
    // 32-bit, blue first, rows from the bottom: the first is the last.
    let file = tga(2, 1, 2, 32, BOTTOM | 8, &[3, 2, 1, 4, 7, 6, 5, 8]);
    assert_eq!(read(&file), Ok(rgba8(1, 2, &[[5, 6, 7, 8], [1, 2, 3, 4]])));
    // Rows from the top.
    let file = tga(2, 1, 2, 32, TOP | 8, &[3, 2, 1, 4, 7, 6, 5, 8]);
    assert_eq!(read(&file), Ok(rgba8(1, 2, &[[1, 2, 3, 4], [5, 6, 7, 8]])));
    // Pixels from the right; 24-bit is opaque.
    let file = tga(2, 2, 1, 24, RIGHT, &[3, 2, 1, 6, 5, 4]);
    assert_eq!(
        read(&file),
        Ok(rgba8(2, 1, &[[4, 5, 6, 255], [1, 2, 3, 255]]))
    );
}

#[test]
fn sixteen_bit_colour_has_alpha_only_if_the_descriptor_says() {
    // 5:5:5: red 31, green 0, blue 1, with the top bit clear; then black
    // with it set.
    let pixels = [0x7C01u16, 0x8000];
    let data: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
    let file = tga(2, 2, 1, 16, TOP | 1, &data);
    assert_eq!(
        read(&file),
        Ok(rgba8(2, 1, &[[255, 0, 8, 0], [0, 0, 0, 255]]))
    );
    // Without alpha bits, or 15-bit, it's opaque.
    for bits in [16, 15] {
        let file = tga(2, 2, 1, bits, TOP, &data);
        assert_eq!(
            read(&file),
            Ok(rgba8(2, 1, &[[255, 0, 8, 255], [0, 0, 0, 255]]))
        );
    }
}

#[test]
fn greyscale_tga_is_l8_or_grey_and_alpha() {
    let file = tga(3, 2, 1, 8, TOP, &[10, 20]);
    assert_eq!(
        read(&file),
        Ok(one_level(TextureFormat::L8, 2, 1, &[10, 20]))
    );
    let file = tga(3, 1, 1, 16, TOP | 8, &[10, 99]);
    assert_eq!(read(&file), Ok(rgba8(1, 1, &[[10, 10, 10, 99]])));
}

#[test]
fn colour_mapped_tga_is_read_as_its_colours() {
    // 24-bit entries starting at index 5; 8-bit indices.
    let entries = [3, 2, 1, 6, 5, 4];
    let file = mapped(1, 2, 1, 8, TOP, Some((5, 24, &entries, 2)), &[6, 5]);
    assert_eq!(
        read(&file),
        Ok(rgba8(2, 1, &[[4, 5, 6, 255], [1, 2, 3, 255]]))
    );
    // 16-bit indices, into 32-bit entries.
    let entries = [3, 2, 1, 9];
    let file = mapped(1, 1, 1, 16, TOP, Some((0, 32, &entries, 1)), &[0, 0]);
    assert_eq!(read(&file), Ok(rgba8(1, 1, &[[1, 2, 3, 9]])));
    // 16-bit entries, whose top bit is alpha with alpha bits; 15-bit
    // ones, whose isn't.
    let entries = 0x7C00u16.to_le_bytes();
    let file = mapped(1, 1, 1, 8, TOP | 1, Some((0, 16, &entries, 1)), &[0]);
    assert_eq!(read(&file), Ok(rgba8(1, 1, &[[255, 0, 0, 0]])));
    let file = mapped(1, 1, 1, 8, TOP | 1, Some((0, 15, &entries, 1)), &[0]);
    assert_eq!(read(&file), Ok(rgba8(1, 1, &[[255, 0, 0, 255]])));
    // A true-colour image's colour map is skipped.
    let file = mapped(2, 1, 1, 24, TOP, Some((0, 24, &[9, 9, 9], 1)), &[3, 2, 1]);
    assert_eq!(read(&file), Ok(rgba8(1, 1, &[[1, 2, 3, 255]])));
}

#[test]
fn what_tga_writer_writes_is_read_back() {
    for image in [
        rgba8(
            2,
            2,
            &[
                [1, 2, 3, 4],
                [5, 6, 7, 8],
                [9, 10, 11, 12],
                [13, 14, 15, 16],
            ],
        ),
        one_level(TextureFormat::L8, 1, 2, &[7, 8]),
    ] {
        let mut file = Vec::new();
        TgaWriter.write(&image, &mut file).unwrap();
        assert_eq!(read(&file), Ok(image));
    }
}

#[test]
fn run_length_encoded_tga_runs_may_cross_rows() {
    // 3×2, 32-bit: a run of four (the first row and the next's first),
    // then two raw pixels.
    let mut data = vec![0x83, 3, 2, 1, 4];
    data.extend([0x01, 7, 6, 5, 8, 11, 10, 9, 12]);
    let file = tga(10, 3, 2, 32, TOP | 8, &data);
    let mut expected = vec![[1, 2, 3, 4]; 4];
    expected.extend([[5, 6, 7, 8], [9, 10, 11, 12]]);
    assert_eq!(read(&file), Ok(rgba8(3, 2, &expected)));
    // Greyscale, with the last run going past the end; colour-mapped.
    let file = tga(11, 2, 1, 8, TOP, &[0x85, 7]);
    assert_eq!(read(&file), Ok(one_level(TextureFormat::L8, 2, 1, &[7, 7])));
    let file = mapped(9, 2, 1, 8, TOP, Some((0, 24, &[3, 2, 1], 1)), &[0x81, 0]);
    assert_eq!(
        read(&file),
        Ok(rgba8(2, 1, &[[1, 2, 3, 255], [1, 2, 3, 255]]))
    );
}

#[test]
fn unusable_tga_files() {
    let malformed = |why: &str| Err(Unusable::Malformed(why.to_string()));
    let unsupported = |why: &str| Err(Unusable::Unsupported(why.to_string()));
    assert_eq!(read(&[0; 17]), malformed("it's too short for a TGA header"));
    assert_eq!(
        read(&tga(32, 1, 1, 8, TOP, &[0])),
        unsupported("its TGA image type, 32, isn't supported")
    );
    for (kind, bits) in [(2, 8), (3, 24), (1, 24), (10, 12)] {
        assert_eq!(
            read(&tga(kind, 1, 1, bits, TOP, &[0; 4])),
            unsupported(&format!(
                "a {}-bit TGA of type {} isn't supported",
                bits, kind
            ))
        );
    }
    assert_eq!(
        read(&mapped(1, 1, 1, 8, TOP, Some((0, 8, &[0], 1)), &[0])),
        unsupported("its colour map's entries are 8 bits, which isn't supported")
    );
    assert_eq!(read(&tga(2, 0, 1, 32, TOP, &[])), malformed("it's 0×1"));
    assert_eq!(read(&tga(2, 1, 0, 32, TOP, &[])), malformed("it's 1×0"));
    // A colour map missing, cut short, or without the index.
    assert_eq!(
        read(&tga(1, 1, 1, 8, TOP, &[0])),
        malformed("it's colour-mapped, but has no colour map")
    );
    assert_eq!(
        read(&mapped(1, 1, 1, 8, TOP, Some((0, 24, &[1, 2], 1)), &[])),
        malformed("it ends in its colour map")
    );
    for index in [0, 2] {
        assert_eq!(
            read(&mapped(
                1,
                1,
                1,
                8,
                TOP,
                Some((1, 24, &[1, 2, 3], 1)),
                &[index]
            )),
            malformed(&format!(
                "a pixel's colour index, {}, isn't in its colour map",
                index
            ))
        );
    }
    let short = "it ends before its last pixel";
    // The ID field past the end, raw pixels missing, or a run-length
    // packet or its pixels.
    let mut file = tga(2, 1, 1, 32, TOP, &[]);
    file.truncate(19);
    assert_eq!(read(&file), malformed(short));
    assert_eq!(read(&tga(2, 1, 1, 32, TOP, &[1, 2, 3])), malformed(short));
    assert_eq!(
        read(&tga(10, 2, 1, 32, TOP, &[0x80, 1, 2, 3, 4])),
        malformed(short)
    );
    assert_eq!(
        read(&tga(10, 2, 1, 32, TOP, &[0x01, 1, 2, 3, 4])),
        malformed(short)
    );
}
