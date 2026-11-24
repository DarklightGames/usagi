use usagi_io::{Image, Level, Pixels, Unusable};
use usagi_pkg::archive::LazyArray;
use usagi_pkg::{Color, Mipmap, Palette, Texture, TextureFormat};

fn image(format: TextureFormat, width: usize, height: usize, data: Vec<u8>) -> Image {
    Image {
        format,
        levels: vec![Level {
            width,
            height,
            data,
        }],
        palette: Vec::new(),
    }
}

/// An RGBA8 image of RGBA pixels (stored blue first, as `FColor` is).
fn rgba8(width: usize, height: usize, pixels: &[[u8; 4]]) -> Image {
    let data = pixels
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    image(TextureFormat::Rgba8, width, height, data)
}

fn rgba(image: &Image) -> Vec<[u8; 4]> {
    let Pixels::Rgba(data) = image.pixels() else {
        panic!("not RGBA");
    };
    data.chunks_exact(4)
        .map(|p| p.try_into().unwrap())
        .collect()
}

/// The greatest difference of any channel of any pixel (alpha only if
/// `alpha`).
fn worst(a: &[[u8; 4]], b: &[[u8; 4]], alpha: bool) -> u8 {
    let channels = if alpha { 4 } else { 3 };
    a.iter()
        .zip(b)
        .flat_map(|(a, b)| (0..channels).map(|k| a[k].abs_diff(b[k])))
        .max()
        .unwrap()
}

/// A smooth `width`×`height` image, varying in every channel, by 8 a
/// pixel at most.
fn gradient(width: usize, height: usize) -> Vec<[u8; 4]> {
    (0..width * height)
        .map(|i| {
            let (x, y) = ((i % width) as u8, (i / width) as u8);
            [x * 8, y * 8, (x + y) * 4, 255 - x * 4]
        })
        .collect()
}

fn block_colors(block: &[u8]) -> (u16, u16) {
    (
        u16::from_le_bytes([block[0], block[1]]),
        u16::from_le_bytes([block[2], block[3]]),
    )
}

#[test]
fn only_p8_and_rgba8_are_compressed_and_only_to_dxt() {
    let source = rgba8(1, 1, &[[0; 4]]);
    assert_eq!(
        source.compress(TextureFormat::Rgba8, true),
        Err(Unusable::Unsupported(
            "only DXT1, DXT3 and DXT5 are compressed to, not TEXF_RGBA8".to_string()
        ))
    );
    let l8 = image(TextureFormat::L8, 1, 1, vec![0]);
    assert_eq!(
        l8.compress(TextureFormat::Dxt1, true),
        Err(Unusable::Unsupported(
            "only P8 and RGBA8 textures are compressed, and it's TEXF_L8".to_string()
        ))
    );
    let dxt = source.compress(TextureFormat::Dxt1, false).unwrap();
    assert_eq!(
        dxt.compress(TextureFormat::Dxt5, false),
        Err(Unusable::Unsupported(
            "only P8 and RGBA8 textures are compressed, and it's TEXF_DXT1".to_string()
        ))
    );
}

#[test]
fn mipmaps_go_down_to_one_pixel_box_filtered() {
    let pixels = gradient(16, 4);
    let compressed = rgba8(16, 4, &pixels)
        .compress(TextureFormat::Dxt5, true)
        .unwrap();
    assert_eq!(compressed.format, TextureFormat::Dxt5);
    assert!(compressed.palette.is_empty());
    let sizes: Vec<_> = compressed
        .levels
        .iter()
        .map(|l| (l.width, l.height, l.data.len()))
        .collect();
    assert_eq!(
        sizes,
        [(16, 4, 64), (8, 2, 32), (4, 1, 16), (2, 1, 16), (1, 1, 16)]
    );
    // The last is the average of every pixel.
    let average: [u8; 4] = std::array::from_fn(|k| {
        (pixels.iter().map(|p| f32::from(p[k])).sum::<f32>() / 64.0).round() as u8
    });
    let last = rgba(&compressed.level(4));
    assert!(worst(&last, &[average], true) <= 4, "{last:?} {average:?}");
    // Without mipmaps, only the first.
    let one = rgba8(16, 4, &pixels)
        .compress(TextureFormat::Dxt5, false)
        .unwrap();
    assert_eq!(one.levels, compressed.levels[..1]);
}

#[test]
fn a_column_is_halved_only_in_height() {
    let pixels = [[0, 0, 0, 255], [255, 255, 255, 255]];
    let compressed = rgba8(1, 2, &pixels)
        .compress(TextureFormat::Dxt3, true)
        .unwrap();
    assert_eq!(compressed.levels.len(), 2);
    assert_eq!(
        (compressed.levels[1].width, compressed.levels[1].height),
        (1, 1)
    );
    // An odd column is left out: 3×1 halves to the first two.
    let odd = rgba8(3, 1, &[[0, 0, 0, 255], [100, 100, 100, 255], [255; 4]])
        .compress(TextureFormat::Dxt3, true)
        .unwrap();
    assert_eq!(odd.levels.len(), 2);
    assert!(worst(&rgba(&odd.level(1)), &[[50, 50, 50, 255]], true) <= 4);
}

#[test]
fn compressed_images_are_close_to_their_source() {
    let pixels = gradient(8, 8);
    let source = rgba8(8, 8, &pixels);
    for format in [
        TextureFormat::Dxt1,
        TextureFormat::Dxt3,
        TextureFormat::Dxt5,
    ] {
        let decoded = rgba(&source.compress(format, false).unwrap());
        // Red follows x and green y, so no line of colours holds a block
        // exactly.
        assert!(worst(&decoded, &pixels, false) <= 16, "{format:?}");
        if format != TextureFormat::Dxt1 {
            assert!(worst(&decoded, &pixels, true) <= 16, "{format:?}");
        }
    }
    // Pixels past the edges are left out: a 5×5 image is four blocks.
    let small = gradient(5, 5);
    let compressed = rgba8(5, 5, &small)
        .compress(TextureFormat::Dxt1, false)
        .unwrap();
    assert_eq!(compressed.levels[0].data.len(), 32);
    assert!(worst(&rgba(&compressed), &small, false) <= 16);
}

#[test]
fn a_p8_image_is_compressed_from_its_palettes_colours() {
    let palette = Palette {
        colors: vec![
            Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            Color {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        ],
    };
    let texture = Texture {
        mips: vec![Mipmap {
            data: LazyArray {
                seek_pos: None,
                items: vec![0, 1, 1, 0],
            },
            u_size: 2,
            v_size: 2,
            u_bits: 1,
            v_bits: 1,
        }],
    };
    let p8 = Image::from_texture(&texture, TextureFormat::P8, Some(&palette), false).unwrap();
    let compressed = p8.compress(TextureFormat::Dxt1, false).unwrap();
    assert_eq!(
        rgba(&compressed),
        [
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255]
        ]
    );
}

#[test]
fn solid_colours_are_matched_as_nearly_as_565_allows() {
    for value in 0..=255u8 {
        let color = [value, 255 - value, value / 3, 255];
        for format in [TextureFormat::Dxt1, TextureFormat::Dxt5] {
            let compressed = rgba8(4, 4, &[color; 16]).compress(format, false).unwrap();
            let decoded = rgba(&compressed);
            assert!(
                worst(&decoded, &[color; 16], true) <= 1,
                "{value} {format:?}"
            );
        }
    }
}

#[test]
fn dxt1_uses_three_colours_when_closer_but_never_transparency() {
    // Black, white and halfway: three colours hold them exactly, four
    // can't.
    let mut pixels = [[0, 0, 0, 255]; 16];
    pixels[1] = [255, 255, 255, 255];
    pixels[2] = [127, 127, 127, 255];
    // And transparency is ignored.
    pixels[3] = [0, 0, 0, 0];
    let dxt1 = rgba8(4, 4, &pixels)
        .compress(TextureFormat::Dxt1, false)
        .unwrap();
    let (c0, c1) = block_colors(&dxt1.levels[0].data);
    assert!(c0 <= c1);
    let decoded = rgba(&dxt1);
    assert!(decoded.iter().all(|p| p[3] == 255));
    assert!(worst(&decoded, &pixels, false) <= 1);
    // DXT3 and DXT5 always have four.
    let dxt5 = rgba8(4, 4, &pixels)
        .compress(TextureFormat::Dxt5, false)
        .unwrap();
    let (c0, c1) = block_colors(&dxt5.levels[0].data[8..]);
    assert!(c0 > c1);
}

#[test]
fn colours_varying_against_each_other_are_fit() {
    // Red and green change in opposite directions, so their sum doesn't.
    let pixels: Vec<[u8; 4]> = (0..16)
        .map(|i| [(i * 17) as u8, 255 - (i * 17) as u8, 128, 255])
        .collect();
    let compressed = rgba8(4, 4, &pixels)
        .compress(TextureFormat::Dxt1, false)
        .unwrap();
    // Sixteen colours evenly along a line, in four: none is more than
    // half the gap between two, and every one is on it.
    let decoded = rgba(&compressed);
    assert!(worst(&decoded, &pixels, false) <= 35);
    assert!(
        decoded
            .iter()
            .all(|p| u16::from(p[0]) + u16::from(p[1]) == 255)
    );
}

#[test]
fn dxt3_alpha_is_rounded_to_four_bits() {
    let alphas = [
        0, 8, 9, 17, 25, 26, 128, 247, 248, 255, 34, 51, 68, 85, 102, 119,
    ];
    let pixels: Vec<[u8; 4]> = alphas.iter().map(|&a| [0, 0, 0, a]).collect();
    let compressed = rgba8(4, 4, &pixels)
        .compress(TextureFormat::Dxt3, false)
        .unwrap();
    let decoded: Vec<u8> = rgba(&compressed).iter().map(|p| p[3]).collect();
    assert_eq!(
        decoded,
        [
            0, 0, 17, 17, 17, 34, 136, 255, 255, 255, 34, 51, 68, 85, 102, 119
        ]
    );
}

#[test]
fn dxt5_alpha_leaves_out_0_and_255_when_that_is_closer() {
    let alpha_of = |alphas: &[u8]| {
        let pixels: Vec<[u8; 4]> = alphas.iter().map(|&a| [9, 9, 9, a]).collect();
        let compressed = rgba8(4, 4, &pixels)
            .compress(TextureFormat::Dxt5, false)
            .unwrap();
        let block = compressed.levels[0].data[..2].to_vec();
        let decoded: Vec<u8> = rgba(&compressed).iter().map(|p| p[3]).collect();
        (block, decoded)
    };
    // 0 and 255 with values close together: six values between 100 and
    // 110, and 0 and 255 besides, hold them all.
    let mut alphas = [100u8; 16];
    alphas[0] = 0;
    alphas[1] = 255;
    alphas[2] = 110;
    let (block, decoded) = alpha_of(&alphas);
    assert!(block[0] <= block[1]);
    assert_eq!(decoded, alphas);
    // A spread: eight values from least to greatest.
    let spread: Vec<u8> = (0..16).map(|i| (i * 17) as u8).collect();
    let (block, decoded) = alpha_of(&spread);
    assert_eq!(block, [255, 0]);
    assert!(
        decoded
            .iter()
            .zip(&spread)
            .all(|(a, b)| a.abs_diff(*b) <= 18)
    );
    // Only 0 and 255: the six-value mode's own.
    let binary: Vec<u8> = (0..16).map(|i| if i % 2 == 0 { 0 } else { 255 }).collect();
    let (_, decoded) = alpha_of(&binary);
    assert_eq!(decoded, binary);
}
