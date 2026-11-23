use usagi_io::{DdsWriter, Image, Level, Pixels, PngWriter, Refused, TgaWriter, Unusable, Writer};
use usagi_pkg::archive::LazyArray;
use usagi_pkg::{Color, Mipmap, Palette, Texture, TextureFormat};

fn mip(width: i32, height: i32, data: &[u8]) -> Mipmap {
    Mipmap {
        data: LazyArray {
            seek_pos: None,
            items: data.to_vec(),
        },
        u_size: width,
        v_size: height,
        u_bits: 0,
        v_bits: 0,
    }
}

fn texture(mips: Vec<Mipmap>) -> Texture {
    Texture { mips }
}

fn image(format: TextureFormat, width: i32, height: i32, data: &[u8]) -> Image {
    Image::from_texture(
        &texture(vec![mip(width, height, data)]),
        format,
        None,
        false,
    )
    .unwrap()
}

fn color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r, g, b, a }
}

fn palette() -> Palette {
    Palette {
        colors: vec![color(1, 2, 3, 4), color(5, 6, 7, 8)],
    }
}

fn written(writer: &impl Writer<Image>, image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    writer.write(image, &mut out).unwrap();
    out
}

#[test]
fn a_p8_texture_is_its_palettes_colours() {
    let texture = texture(vec![mip(2, 1, &[1, 0, 9]), mip(1, 1, &[1])]);
    let image = Image::from_texture(&texture, TextureFormat::P8, Some(&palette()), false).unwrap();
    // The palette is made 256 colours, as the engine's is; extra bytes are
    // dropped.
    assert_eq!(image.palette.len(), 256);
    assert_eq!(image.palette[255], Color::default());
    assert_eq!(
        image.levels,
        [
            Level {
                width: 2,
                height: 1,
                data: vec![1, 0],
            },
            Level {
                width: 1,
                height: 1,
                data: vec![1],
            },
        ]
    );
    assert_eq!(image.pixels(), Pixels::Rgba(vec![5, 6, 7, 8, 1, 2, 3, 4]));
    assert_eq!(image.level(1).pixels(), Pixels::Rgba(vec![5, 6, 7, 8]));
    // Masked, the first colour is transparent black.
    let masked = Image::from_texture(&texture, TextureFormat::P8, Some(&palette()), true).unwrap();
    assert_eq!(masked.pixels(), Pixels::Rgba(vec![5, 6, 7, 8, 0, 0, 0, 0]));
}

#[test]
fn uncompressed_textures_are_decoded() {
    let rgba8 = image(TextureFormat::Rgba8, 1, 1, &[1, 2, 3, 4]);
    assert_eq!(rgba8.pixels(), Pixels::Rgba(vec![3, 2, 1, 4]));
    assert!(rgba8.palette.is_empty());
    let l8 = image(TextureFormat::L8, 2, 1, &[7, 8]);
    assert_eq!(l8.pixels(), Pixels::Gray(vec![7, 8]));
    let g16 = image(TextureFormat::G16, 1, 1, &[0x34, 0x12]);
    assert_eq!(g16.pixels(), Pixels::Gray16(vec![0x1234]));
}

#[test]
fn unusable_textures() {
    let unusable = |texture: &Texture, format, palette: Option<&Palette>| {
        Image::from_texture(texture, format, palette, false).unwrap_err()
    };
    let one = texture(vec![mip(1, 1, &[0; 4])]);
    for format in [
        TextureFormat::Rgba7,
        TextureFormat::Rgb16,
        TextureFormat::Rgb8,
        TextureFormat::NoData,
        TextureFormat::RrrGggBbb,
        TextureFormat::Unknown(40),
    ] {
        let error = unusable(&one, format, None);
        assert_eq!(
            error,
            Unusable::Unsupported(format!("its format, {}, isn't supported", format.name()))
        );
    }
    assert_eq!(
        unusable(&texture(Vec::new()), TextureFormat::L8, None).to_string(),
        "it has no mipmaps"
    );
    assert_eq!(
        unusable(&one, TextureFormat::P8, None),
        Unusable::Unsupported("it has no palette".to_string())
    );
    for (width, height) in [(0, 1), (1, -1)] {
        let bad = texture(vec![mip(1, 1, &[0]), mip(width, height, &[0])]);
        assert_eq!(
            unusable(&bad, TextureFormat::L8, None),
            Unusable::Malformed(format!("mipmap 1 is {}×{}", width, height))
        );
    }
    assert_eq!(
        unusable(&texture(vec![mip(1, 1, &[])]), TextureFormat::L8, None),
        Unusable::Unsupported("its largest mipmap has no data".to_string())
    );
    let short = texture(vec![mip(4, 4, &[0; 7])]);
    let error = unusable(&short, TextureFormat::Dxt1, None);
    assert_eq!(
        error,
        Unusable::Malformed("mipmap 0 has 7 bytes, not the 8 that 4×4 TEXF_DXT1 takes".to_string())
    );
    assert_eq!(
        error.to_string(),
        "mipmap 0 has 7 bytes, not the 8 that 4×4 TEXF_DXT1 takes"
    );
    // Mipmaps from the first after the largest with no data are dropped.
    let gap = texture(vec![
        mip(4, 1, &[1, 2, 3, 4]),
        mip(2, 1, &[5, 6]),
        mip(1, 1, &[]),
        mip(1, 1, &[7]),
    ]);
    let image = Image::from_texture(&gap, TextureFormat::L8, None, false).unwrap();
    assert_eq!(image.levels.len(), 2);
    let error: Box<dyn std::error::Error> = Box::new(error);
    assert!(error.source().is_none());
}

/// A colour block: two 5:6:5 colours, and each pixel's index.
fn color_block(c0: u16, c1: u16, indices: [u8; 16]) -> Vec<u8> {
    let bits = indices
        .iter()
        .enumerate()
        .fold(0u32, |bits, (i, &index)| bits | u32::from(index) << (2 * i));
    let mut block = c0.to_le_bytes().to_vec();
    block.extend(c1.to_le_bytes());
    block.extend(bits.to_le_bytes());
    block
}

const RED: u16 = 0xF800;
const BLUE: u16 = 0x001F;

/// The pixels of a decoded image, as RGBA.
fn rgba(image: &Image) -> Vec<[u8; 4]> {
    let Pixels::Rgba(data) = image.pixels() else {
        panic!("not RGBA");
    };
    data.chunks_exact(4)
        .map(|p| p.try_into().unwrap())
        .collect()
}

#[test]
fn dxt1_blocks_have_four_colours_or_three_and_transparency() {
    let indices = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    // The first colour greater: two thirds and one third between them.
    let four = image(TextureFormat::Dxt1, 4, 4, &color_block(RED, BLUE, indices));
    assert_eq!(
        rgba(&four)[..4],
        [
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [170, 0, 85, 255],
            [85, 0, 170, 255]
        ]
    );
    // Not greater: halfway, and transparent black.
    let three = image(TextureFormat::Dxt1, 4, 4, &color_block(BLUE, RED, indices));
    assert_eq!(
        rgba(&three)[..4],
        [
            [0, 0, 255, 255],
            [255, 0, 0, 255],
            [127, 0, 127, 255],
            [0; 4]
        ]
    );
    // 5:6:5 to 8 bits: the high bits repeated.
    let gray = image(
        TextureFormat::Dxt1,
        4,
        4,
        &color_block(0b10000_100000_10000, 0, [0; 16]),
    );
    assert_eq!(rgba(&gray)[0], [132, 130, 132, 255]);
}

#[test]
fn dxt_blocks_past_the_edges_are_cropped() {
    // 5×1: two blocks across, the second's first column only.
    let mut data = color_block(RED, BLUE, [1; 16]);
    data.extend(color_block(RED, BLUE, [0; 16]));
    let wide = image(TextureFormat::Dxt1, 5, 1, &data);
    let mut expected = vec![[0, 0, 255, 255]; 4];
    expected.push([255, 0, 0, 255]);
    assert_eq!(rgba(&wide), expected);
    // A 1×1 mipmap is a whole block.
    let small = image(TextureFormat::Dxt1, 1, 1, &color_block(RED, BLUE, [0; 16]));
    assert_eq!(rgba(&small), [[255, 0, 0, 255]]);
}

#[test]
fn dxt3_blocks_have_four_bits_of_alpha() {
    let mut alpha = 0u64;
    for i in 0..16 {
        alpha |= (i as u64) << (4 * i);
    }
    let mut data = alpha.to_le_bytes().to_vec();
    // Four colours even though the first isn't greater.
    data.extend(color_block(BLUE, RED, [3; 16]));
    let image = image(TextureFormat::Dxt3, 4, 4, &data);
    let pixels = rgba(&image);
    assert_eq!(pixels[0], [170, 0, 85, 0]);
    assert_eq!(pixels[1][3], 17);
    assert_eq!(pixels[15][3], 255);
}

/// A DXT5 alpha block: two values, and each pixel's index.
fn alpha_block(a0: u8, a1: u8, indices: [u8; 16]) -> Vec<u8> {
    let bits = indices
        .iter()
        .enumerate()
        .fold(0u64, |bits, (i, &index)| bits | u64::from(index) << (3 * i));
    let mut block = vec![a0, a1];
    block.extend(&bits.to_le_bytes()[..6]);
    block
}

#[test]
fn dxt5_blocks_have_interpolated_alpha() {
    let indices = [0, 1, 2, 3, 4, 5, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7];
    let alphas = |a0, a1| {
        let mut data = alpha_block(a0, a1, indices);
        data.extend(color_block(BLUE, RED, [0; 16]));
        let image = image(TextureFormat::Dxt5, 4, 4, &data);
        let pixels = rgba(&image);
        assert_eq!(pixels[0][..3], [0, 0, 255]);
        pixels.iter().take(9).map(|p| p[3]).collect::<Vec<_>>()
    };
    // The first greater: six values between them.
    assert_eq!(alphas(210, 0), [210, 0, 180, 150, 120, 90, 60, 30, 30]);
    // Not greater: four between them, then 0 and 255.
    assert_eq!(alphas(0, 200), [0, 200, 40, 80, 120, 160, 0, 255, 255]);
}

#[test]
fn png_holds_every_kind_of_pixels() {
    let decode = |bytes: &[u8]| {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut data = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut data).unwrap();
        (
            info.width,
            info.height,
            info.color_type,
            info.bit_depth,
            data,
        )
    };
    assert_eq!(PngWriter.extension(), "png");
    let rgba8 = image(TextureFormat::Rgba8, 2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        decode(&written(&PngWriter, &rgba8)),
        (
            2,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            vec![3, 2, 1, 4, 7, 6, 5, 8]
        )
    );
    let l8 = image(TextureFormat::L8, 1, 1, &[9]);
    assert_eq!(
        decode(&written(&PngWriter, &l8)),
        (
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            vec![9]
        )
    );
    let g16 = image(TextureFormat::G16, 1, 1, &[0x34, 0x12]);
    assert_eq!(
        decode(&written(&PngWriter, &g16)),
        (
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            vec![0x12, 0x34]
        )
    );
}

/// A TGA file's header, for an image of `kind`, `bits` and `alpha` bits.
fn tga_header(kind: u8, width: u16, height: u16, bits: u8, alpha: u8) -> Vec<u8> {
    let mut header = vec![0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    header.extend(width.to_le_bytes());
    header.extend(height.to_le_bytes());
    header.extend([bits, alpha]);
    header
}

fn tga_footer() -> Vec<u8> {
    let mut footer = vec![0; 8];
    footer.extend(b"TRUEVISION-XFILE.\0");
    footer
}

#[test]
fn tga_is_written_from_the_bottom_row() {
    assert_eq!(TgaWriter.extension(), "tga");
    // 1×2 RGBA8: the bottom row first, blue first.
    let rgba8 = image(TextureFormat::Rgba8, 1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let mut expected = tga_header(2, 1, 2, 32, 8);
    expected.extend([5, 6, 7, 8, 1, 2, 3, 4]);
    expected.extend(tga_footer());
    assert_eq!(written(&TgaWriter, &rgba8), expected);
    let l8 = image(TextureFormat::L8, 1, 2, &[7, 8]);
    let mut expected = tga_header(3, 1, 2, 8, 0);
    expected.extend([8, 7]);
    expected.extend(tga_footer());
    assert_eq!(written(&TgaWriter, &l8), expected);
}

#[test]
fn tga_refuses_what_it_cant_hold() {
    let g16 = image(TextureFormat::G16, 1, 1, &[0, 0]);
    assert_eq!(
        TgaWriter.write(&g16, &mut Vec::new()),
        Err(Refused(
            "TGA can't hold 16-bit greyscale without loss".to_string()
        ))
    );
    for (width, height) in [(65536, 1), (1, 65536)] {
        let big = image(TextureFormat::L8, width, height, &vec![0; 65536]);
        assert_eq!(
            TgaWriter.write(&big, &mut Vec::new()),
            Err(Refused(format!("{}×{} is too big for TGA", width, height)))
        );
    }
}

/// A DDS file's header.
fn dds_header(width: u32, height: u32, size: u32, mips: u32, four_cc: &[u8; 4]) -> Vec<u8> {
    let (flags, caps) = if mips > 1 {
        (0x000A_1007u32, 0x0040_1008u32)
    } else {
        (0x0008_1007, 0x1000)
    };
    let mut header = b"DDS ".to_vec();
    for value in [124, flags, height, width, size, 0, mips] {
        header.extend(u32::to_le_bytes(value));
    }
    header.extend([0; 44]);
    header.extend([32, 0, 0, 0, 4, 0, 0, 0]);
    header.extend(four_cc);
    header.extend([0; 20]);
    header.extend(caps.to_le_bytes());
    header.extend([0; 16]);
    header
}

#[test]
fn dds_holds_dxt_textures_and_their_mipmaps() {
    assert_eq!(DdsWriter.extension(), "dds");
    let blocks: Vec<u8> = (0..16).collect();
    let mips = texture(vec![
        mip(8, 4, &blocks),
        mip(4, 2, &blocks[..8]),
        mip(2, 1, &blocks[..8]),
        mip(1, 1, &blocks[..8]),
    ]);
    let image = Image::from_texture(&mips, TextureFormat::Dxt1, None, false).unwrap();
    let mut expected = dds_header(8, 4, 16, 4, b"DXT1");
    expected.extend(&blocks);
    for _ in 0..3 {
        expected.extend(&blocks[..8]);
    }
    let out = written(&DdsWriter, &image);
    assert_eq!(out.len(), 128 + 40);
    assert_eq!(out, expected);
    for (format, four_cc) in [
        (TextureFormat::Dxt3, b"DXT3"),
        (TextureFormat::Dxt5, b"DXT5"),
    ] {
        let one = self::image(format, 4, 4, &blocks);
        let mut expected = dds_header(4, 4, 16, 1, four_cc);
        expected.extend(&blocks);
        assert_eq!(written(&DdsWriter, &one), expected);
    }
}

#[test]
fn dds_refuses_what_it_cant_hold() {
    let l8 = image(TextureFormat::L8, 1, 1, &[0]);
    assert_eq!(
        DdsWriter.write(&l8, &mut Vec::new()),
        Err(Refused(
            "only DXT textures are written as DDS, and it's TEXF_L8".to_string()
        ))
    );
    let block = [0u8; 8];
    for second in [mip(4, 4, &block), mip(8, 2, &[0; 16])] {
        let odd = texture(vec![mip(8, 4, &[0; 16]), second]);
        let image = Image::from_texture(&odd, TextureFormat::Dxt1, None, false).unwrap();
        assert_eq!(
            DdsWriter.write(&image, &mut Vec::new()),
            Err(Refused(
                "its mipmaps aren't each half the size of the one before".to_string()
            ))
        );
    }
}
