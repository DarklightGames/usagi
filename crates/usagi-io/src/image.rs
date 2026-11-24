//! Textures: the [`Image`] model, compressing it to DXT, writing it as
//! PNG, TGA or DDS, and reading it from TGA.

use usagi_pkg::{Color, Palette, Texture, TextureFormat};

use crate::{Reader, Refused, Unusable, Writer};

/// One mipmap of an [`Image`]: its size, and its pixels in the image's
/// format, exactly as many bytes as that takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// An image: its pixel format, its mipmaps, largest first, and for `P8`,
/// its palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// One of the formats [`Image::from_texture`] accepts.
    pub format: TextureFormat,
    pub levels: Vec<Level>,
    /// For `P8`, the 256 colours its pixels index, as the engine draws
    /// them; empty otherwise.
    pub palette: Vec<Color>,
}

/// An image's pixels, decoded, in rows from the top, left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pixels {
    /// Red, green, blue, alpha.
    Rgba(Vec<u8>),
    /// 8-bit greyscale.
    Gray(Vec<u8>),
    /// 16-bit greyscale.
    Gray16(Vec<u16>),
}

impl Image {
    /// A texture's image (`UTexture`'s mipmaps), in its `format`. A `P8`
    /// texture's pixels are indices into `palette`; with `masked`
    /// (`bMasked`), its first colour is transparent black, as the engine
    /// draws it (`FStaticTexture::GetTextureData`). Any data after what a
    /// mipmap's size takes is ignored, as the engine ignores it, and so are
    /// the mipmaps from the first after the largest that has no data at
    /// all, which shipped textures have.
    ///
    /// # Errors
    ///
    /// [`Unusable::Unsupported`] for a format other than `P8`, `RGBA8`,
    /// `L8`, `G16` and `DXT1`, `DXT3` and `DXT5` (the engine never makes or
    /// converts the others, so how their pixels are laid out isn't known), for a texture with
    /// no mipmaps or whose largest has no data, and for a `P8` one with no
    /// palette.
    /// [`Unusable::Malformed`] for a mipmap with no pixels, or fewer bytes
    /// than its size takes.
    pub fn from_texture(
        texture: &Texture,
        format: TextureFormat,
        palette: Option<&Palette>,
        masked: bool,
    ) -> Result<Image, Unusable> {
        if bytes_for(format, 1, 1).is_none() {
            return Err(Unusable::Unsupported(format!(
                "its format, {}, isn't supported",
                format.name()
            )));
        }
        if texture.mips.is_empty() {
            return Err(Unusable::Unsupported("it has no mipmaps".to_string()));
        }
        let palette = match (format, palette) {
            (TextureFormat::P8, None) => {
                return Err(Unusable::Unsupported("it has no palette".to_string()));
            }
            (TextureFormat::P8, Some(palette)) => {
                let mut colors = palette.colors.clone();
                colors.resize(256, Color::default());
                if masked {
                    colors[0] = Color::default();
                }
                colors
            }
            _ => Vec::new(),
        };
        if texture.mips[0].data.items.is_empty() {
            return Err(Unusable::Unsupported(
                "its largest mipmap has no data".to_string(),
            ));
        }
        let levels = texture
            .mips
            .iter()
            // Shipped textures can have mipmaps with no data at all; only
            // those before the first are used.
            .take_while(|mip| !mip.data.items.is_empty())
            .enumerate()
            .map(|(i, mip)| {
                let size = |value: i32| usize::try_from(value).ok().filter(|&v| v > 0);
                let (Some(width), Some(height)) = (size(mip.u_size), size(mip.v_size)) else {
                    return Err(Unusable::Malformed(format!(
                        "mipmap {} is {}×{}",
                        i, mip.u_size, mip.v_size
                    )));
                };
                let len = bytes_for(format, width, height).expect("a supported format");
                let data = &mip.data.items;
                if data.len() < len {
                    return Err(Unusable::Malformed(format!(
                        "mipmap {} has {} bytes, not the {} that {}×{} {} takes",
                        i,
                        data.len(),
                        len,
                        width,
                        height,
                        format.name()
                    )));
                }
                Ok(Level {
                    width,
                    height,
                    data: data[..len].to_vec(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Image {
            format,
            levels,
            palette,
        })
    }

    /// The image with only mipmap `level`.
    ///
    /// # Panics
    ///
    /// If there's no such mipmap.
    pub fn level(&self, level: usize) -> Image {
        Image {
            format: self.format,
            levels: vec![self.levels[level].clone()],
            palette: self.palette.clone(),
        }
    }

    /// The pixels of its largest mipmap, decoded.
    pub fn pixels(&self) -> Pixels {
        let level = &self.levels[0];
        let data = &level.data;
        match self.format {
            TextureFormat::P8 => Pixels::Rgba(
                data.iter()
                    .flat_map(|&i| {
                        let c = self.palette[usize::from(i)];
                        [c.r, c.g, c.b, c.a]
                    })
                    .collect(),
            ),
            TextureFormat::Rgba8 => Pixels::Rgba(
                data.chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect(),
            ),
            TextureFormat::L8 => Pixels::Gray(data.clone()),
            TextureFormat::G16 => Pixels::Gray16(
                data.chunks_exact(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect(),
            ),
            dxt => Pixels::Rgba(crate::dxt::decode(dxt, level.width, level.height, data)),
        }
    }

    /// The image compressed to `format`, `DXT1`, `DXT3` or `DXT5`, as
    /// UnrealEd's `UTexture::Compress` does: from its largest mipmap,
    /// with, if `mipmaps`, every smaller mipmap down to 1×1, each the
    /// average of 2×2 pixels of the one before (nvDXT's box filter). DXT1
    /// has no alpha: UnrealEd asks for DXT1 without it.
    ///
    /// The engine compresses with NVIDIA's nvDXTlib, whose source isn't
    /// in the engine's, so the blocks aren't bit for bit the ones UnrealEd
    /// would make. `UTexture::Compress`'s border and fade options, which
    /// UnrealEd's `TEXTURE IMPORT` sets for modulated and additive
    /// textures, aren't supported.
    ///
    /// # Errors
    ///
    /// [`Unusable::Unsupported`] for a `format` other than those three,
    /// and for an image that isn't `P8` or `RGBA8`, the only formats the
    /// engine compresses.
    pub fn compress(&self, format: TextureFormat, mipmaps: bool) -> Result<Image, Unusable> {
        if !matches!(
            format,
            TextureFormat::Dxt1 | TextureFormat::Dxt3 | TextureFormat::Dxt5
        ) {
            return Err(Unusable::Unsupported(format!(
                "only DXT1, DXT3 and DXT5 are compressed to, not {}",
                format.name()
            )));
        }
        let rgba = match (self.format, self.pixels()) {
            (TextureFormat::P8 | TextureFormat::Rgba8, Pixels::Rgba(rgba)) => rgba,
            _ => {
                return Err(Unusable::Unsupported(format!(
                    "only P8 and RGBA8 textures are compressed, and it's {}",
                    self.format.name()
                )));
            }
        };
        let (mut width, mut height) = self.size();
        let mut levels = vec![Level {
            width,
            height,
            data: crate::dxt::encode(format, width, height, &rgba),
        }];
        // Each mipmap is made from the one before, unrounded.
        let mut pixels: Vec<f32> = rgba.iter().map(|&v| f32::from(v)).collect();
        while mipmaps && (width > 1 || height > 1) {
            (pixels, width, height) = downsample(&pixels, width, height);
            let rounded: Vec<u8> = pixels.iter().map(|&v| v.round() as u8).collect();
            levels.push(Level {
                width,
                height,
                data: crate::dxt::encode(format, width, height, &rounded),
            });
        }
        Ok(Image {
            format,
            levels,
            palette: Vec::new(),
        })
    }

    /// The size of its largest mipmap.
    fn size(&self) -> (usize, usize) {
        (self.levels[0].width, self.levels[0].height)
    }
}

/// A `width`×`height` RGBA image halved in each dimension that's more
/// than 1, each pixel the average of the 2×2 (or 2×1) it covers. Of an
/// odd number of rows or columns, the last is left out.
fn downsample(pixels: &[f32], width: usize, height: usize) -> (Vec<f32>, usize, usize) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let (dx, dy) = (usize::from(width > 1), usize::from(height > 1));
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (x0, y0) = (x * 2, y * 2);
            let at = |x: usize, y: usize, k: usize| pixels[(y * width + x) * 4 + k];
            out.extend((0..4).map(|k| {
                (at(x0, y0, k) + at(x0 + dx, y0, k) + at(x0, y0 + dy, k) + at(x0 + dx, y0 + dy, k))
                    / 4.0
            }));
        }
    }
    (out, w, h)
}

/// How many bytes a `width`×`height` mipmap takes in `format`; `None` if
/// it isn't one an [`Image`] can be. A DXT mipmap is whole 4×4 blocks.
fn bytes_for(format: TextureFormat, width: usize, height: usize) -> Option<usize> {
    let pixels = width * height;
    let blocks = width.div_ceil(4) * height.div_ceil(4);
    match format {
        TextureFormat::P8 | TextureFormat::L8 => Some(pixels),
        TextureFormat::G16 => Some(pixels * 2),
        TextureFormat::Rgba8 => Some(pixels * 4),
        TextureFormat::Dxt1 => Some(blocks * 8),
        TextureFormat::Dxt3 | TextureFormat::Dxt5 => Some(blocks * 16),
        _ => None,
    }
}

/// Writes PNG files of an image's largest mipmap, in as many bits as it
/// has: RGBA, or 8- or 16-bit greyscale.
#[derive(Debug, Clone, Copy, Default)]
pub struct PngWriter;

impl Writer<Image> for PngWriter {
    fn extension(&self) -> &'static str {
        "png"
    }

    fn write(&self, image: &Image, out: &mut Vec<u8>) -> Result<(), Refused> {
        let (width, height) = image.size();
        let (color, depth, data) = match image.pixels() {
            Pixels::Rgba(data) => (png::ColorType::Rgba, png::BitDepth::Eight, data),
            Pixels::Gray(data) => (png::ColorType::Grayscale, png::BitDepth::Eight, data),
            Pixels::Gray16(data) => (
                png::ColorType::Grayscale,
                png::BitDepth::Sixteen,
                data.iter().flat_map(|v| v.to_be_bytes()).collect(),
            ),
        };
        let dimension = |v: usize| u32::try_from(v).expect("a mipmap's size is an i32");
        let mut encoder = png::Encoder::new(&mut *out, dimension(width), dimension(height));
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&data))
            .expect("writing a PNG to memory");
        Ok(())
    }
}

/// Writes TGA files of an image's largest mipmap, uncompressed, as the
/// engine's exporter does (`UTextureExporterTGA`): 32-bit with alpha, or
/// 8-bit greyscale. A 16-bit greyscale image is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct TgaWriter;

impl Writer<Image> for TgaWriter {
    fn extension(&self) -> &'static str {
        "tga"
    }

    fn write(&self, image: &Image, out: &mut Vec<u8>) -> Result<(), Refused> {
        let (width, height) = image.size();
        let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
            return Err(Refused(format!("{}×{} is too big for TGA", width, height)));
        };
        // Image type, bits per pixel, alpha bits; pixels in TGA's order
        // (blue first).
        let (kind, bits, alpha, pixels): (u8, u8, u8, Vec<u8>) = match image.pixels() {
            Pixels::Rgba(data) => (
                2,
                32,
                8,
                data.chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect(),
            ),
            Pixels::Gray(data) => (3, 8, 0, data),
            Pixels::Gray16(_) => {
                return Err(Refused(
                    "TGA can't hold 16-bit greyscale without loss".to_string(),
                ));
            }
        };
        out.extend([0, 0, kind]);
        out.extend([0; 9]); // No colour map; origin 0, 0.
        out.extend(w.to_le_bytes());
        out.extend(h.to_le_bytes());
        // Rows from the bottom, as the descriptor's origin bit (clear) says.
        out.extend([bits, alpha]);
        let row = pixels.len() / height;
        for y in (0..height).rev() {
            out.extend(&pixels[y * row..(y + 1) * row]);
        }
        // TGA 2.0's footer: no extension or developer area.
        out.extend([0; 8]);
        out.extend(b"TRUEVISION-XFILE.\0");
        Ok(())
    }
}

/// Reads TGA files: colour-mapped (types 1 and 9), true-colour (2 and
/// 10) and greyscale (3 and 11), uncompressed or run-length encoded.
/// Greyscale 8-bit images are read as `L8`; everything else as `RGBA8`:
/// colour-mapped pixels as their colours, 15- and 16-bit ones (5:5:5) with
/// each channel's high bits repeated in the low, and 16-bit greyscale as
/// grey and alpha. A 16-bit colour's top bit is its alpha only if the
/// descriptor says there's alpha; 32-bit colours' alpha is always read.
/// The descriptor's origin is followed: rows from the bottom or the top,
/// and pixels from the left or the right. Run-length packets may cross
/// rows, and the last may go past the image's end.
#[derive(Debug, Clone, Copy, Default)]
pub struct TgaReader;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TgaKind {
    Mapped,
    TrueColor,
    Gray,
}

/// A 5:5:5 colour (blue in the low bits), with its top bit as alpha if
/// `alpha`, as RGBA.
fn rgb555(v: u16, alpha: bool) -> [u8; 4] {
    let channel = |shift: u16| {
        let c = ((v >> shift) & 0x1F) as u8;
        c << 3 | c >> 2
    };
    let a = if alpha && v & 0x8000 == 0 { 0 } else { 255 };
    [channel(10), channel(5), channel(0), a]
}

/// A colour of 2, 3 or 4 bytes, as TGA stores it (blue first), as RGBA.
fn tga_color(bytes: &[u8], alpha: bool) -> [u8; 4] {
    match bytes.len() {
        2 => rgb555(u16::from_le_bytes([bytes[0], bytes[1]]), alpha),
        3 => [bytes[2], bytes[1], bytes[0], 255],
        _ => [bytes[2], bytes[1], bytes[0], bytes[3]],
    }
}

impl Reader<Image> for TgaReader {
    fn extension(&self) -> &'static str {
        "tga"
    }

    /// # Errors
    ///
    /// [`Unusable::Unsupported`] for another type of image, or a bit depth
    /// its type can't have, and [`Unusable::Malformed`] for a file too
    /// short for its header, colour map or pixels, a colour-mapped image
    /// without a colour map or with an index past its end, or an image 0
    /// pixels wide or high.
    fn read(&self, file: &[u8]) -> Result<Image, Unusable> {
        let Some(header) = file.get(..18) else {
            return Err(Unusable::Malformed(
                "it's too short for a TGA header".to_string(),
            ));
        };
        let word = |at: usize| usize::from(u16::from_le_bytes([header[at], header[at + 1]]));
        let (has_map, map_first, map_length, map_bits) =
            (header[1] == 1, word(3), word(5), header[7]);
        let (width, height, bits, descriptor) = (word(12), word(14), header[16], header[17]);
        let alpha = descriptor & 0x0F != 0;
        let (kind, rle) = match header[2] {
            1 => (TgaKind::Mapped, false),
            2 => (TgaKind::TrueColor, false),
            3 => (TgaKind::Gray, false),
            9 => (TgaKind::Mapped, true),
            10 => (TgaKind::TrueColor, true),
            11 => (TgaKind::Gray, true),
            other => {
                return Err(Unusable::Unsupported(format!(
                    "its TGA image type, {}, isn't supported",
                    other
                )));
            }
        };
        let size = match (kind, bits) {
            (TgaKind::Mapped | TgaKind::Gray, 8) => 1,
            (TgaKind::Mapped | TgaKind::Gray, 16) | (TgaKind::TrueColor, 15 | 16) => 2,
            (TgaKind::TrueColor, 24) => 3,
            (TgaKind::TrueColor, 32) => 4,
            _ => {
                return Err(Unusable::Unsupported(format!(
                    "a {}-bit TGA of type {} isn't supported",
                    bits, header[2]
                )));
            }
        };
        if width == 0 || height == 0 {
            return Err(Unusable::Malformed(format!("it's {}×{}", width, height)));
        }
        let short = || Unusable::Malformed("it ends before its last pixel".to_string());
        let mut data = file.get(18 + usize::from(header[0])..).ok_or_else(short)?;
        // The colour map, as RGBA; skipped for images that don't use it.
        let mut map = Vec::new();
        if has_map {
            let entry = match map_bits {
                15 | 16 => 2,
                24 => 3,
                32 => 4,
                other => {
                    return Err(Unusable::Unsupported(format!(
                        "its colour map's entries are {} bits, which isn't supported",
                        other
                    )));
                }
            };
            let bytes = data
                .get(..map_length * entry)
                .ok_or_else(|| Unusable::Malformed("it ends in its colour map".to_string()))?;
            if kind == TgaKind::Mapped {
                map = bytes
                    .chunks_exact(entry)
                    .map(|c| tga_color(c, map_bits == 16 && alpha))
                    .collect();
            }
            data = &data[map_length * entry..];
        } else if kind == TgaKind::Mapped {
            return Err(Unusable::Malformed(
                "it's colour-mapped, but has no colour map".to_string(),
            ));
        }
        let count = width * height;
        // The pixels in the file's order, `size` bytes each.
        let pixels = if rle {
            let mut pixels = Vec::with_capacity(count * size);
            while pixels.len() < count * size {
                let (&packet, rest) = data.split_first().ok_or_else(short)?;
                let run = usize::from(packet & 0x7F) + 1;
                let (repeated, length) = if packet & 0x80 != 0 {
                    (run, 1)
                } else {
                    (1, run)
                };
                let run_data = rest.get(..length * size).ok_or_else(short)?;
                for _ in 0..repeated {
                    pixels.extend(run_data);
                }
                data = &rest[length * size..];
            }
            pixels.truncate(count * size);
            pixels
        } else {
            data.get(..count * size).ok_or_else(short)?.to_vec()
        };
        let gray = kind == TgaKind::Gray && size == 1;
        let out_size = if gray { 1 } else { 4 };
        let (from_right, from_top) = (descriptor & 0x10 != 0, descriptor & 0x20 != 0);
        let mut out = vec![0; count * out_size];
        for (i, pixel) in pixels.chunks_exact(size).enumerate() {
            let (x, y) = (i % width, i / width);
            let x = if from_right { width - 1 - x } else { x };
            let y = if from_top { y } else { height - 1 - y };
            if gray {
                out[y * width + x] = pixel[0];
                continue;
            }
            let rgba = match kind {
                TgaKind::Gray => [pixel[0], pixel[0], pixel[0], pixel[1]],
                TgaKind::TrueColor => tga_color(pixel, alpha),
                TgaKind::Mapped => {
                    let index = if size == 1 {
                        usize::from(pixel[0])
                    } else {
                        usize::from(u16::from_le_bytes([pixel[0], pixel[1]]))
                    };
                    *index
                        .checked_sub(map_first)
                        .and_then(|entry| map.get(entry))
                        .ok_or_else(|| {
                            Unusable::Malformed(format!(
                                "a pixel's colour index, {}, isn't in its colour map",
                                index
                            ))
                        })?
                }
            };
            // RGBA8 is stored blue first.
            let at = (y * width + x) * 4;
            out[at..at + 4].copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        Ok(Image {
            format: if gray {
                TextureFormat::L8
            } else {
                TextureFormat::Rgba8
            },
            levels: vec![Level {
                width,
                height,
                data: out,
            }],
            palette: Vec::new(),
        })
    }
}

/// Writes DDS files of a DXT image, with every mipmap, as the engine's
/// exporter does (`UTextureExporterDDS`). Any other image is refused, as is
/// one whose mipmaps aren't each half the size of the one before, which a
/// DDS file can't hold.
#[derive(Debug, Clone, Copy, Default)]
pub struct DdsWriter;

impl Writer<Image> for DdsWriter {
    fn extension(&self) -> &'static str {
        "dds"
    }

    fn write(&self, image: &Image, out: &mut Vec<u8>) -> Result<(), Refused> {
        let four_cc: &[u8; 4] = match image.format {
            TextureFormat::Dxt1 => b"DXT1",
            TextureFormat::Dxt3 => b"DXT3",
            TextureFormat::Dxt5 => b"DXT5",
            other => {
                return Err(Refused(format!(
                    "only DXT textures are written as DDS, and it's {}",
                    other.name()
                )));
            }
        };
        let chained = image.levels.windows(2).all(|pair| {
            pair[1].width == (pair[0].width / 2).max(1)
                && pair[1].height == (pair[0].height / 2).max(1)
        });
        if !chained {
            return Err(Refused(
                "its mipmaps aren't each half the size of the one before".to_string(),
            ));
        }
        let (width, height) = image.size();
        let dword = |v: usize| u32::try_from(v).expect("a mipmap's size is an i32");
        let mips = image.levels.len();
        // DDSD_CAPS, HEIGHT, WIDTH, PIXELFORMAT, LINEARSIZE, and MIPMAPCOUNT.
        let mut flags = 0x1 | 0x2 | 0x4 | 0x1000 | 0x80000;
        // DDSCAPS_TEXTURE, and COMPLEX and MIPMAP.
        let mut caps = 0x1000;
        if mips > 1 {
            flags |= 0x20000;
            caps |= 0x8 | 0x400000;
        }
        out.extend(b"DDS ");
        for value in [
            124,
            flags,
            dword(height),
            dword(width),
            dword(image.levels[0].data.len()),
            0,
            dword(mips),
        ] {
            out.extend(u32::to_le_bytes(value));
        }
        out.extend([0; 44]); // Reserved.
        // The pixel format: its size, DDPF_FOURCC, and the format.
        out.extend(32u32.to_le_bytes());
        out.extend(4u32.to_le_bytes());
        out.extend(four_cc);
        out.extend([0; 20]); // No bit count or masks.
        out.extend(u32::to_le_bytes(caps));
        out.extend([0; 16]); // Caps 2 to 4, reserved.
        for level in &image.levels {
            out.extend(&level.data);
        }
        Ok(())
    }
}
