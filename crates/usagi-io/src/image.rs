//! Textures: the [`Image`] model, and writing it as PNG, TGA or DDS.

use std::fmt;

use usagi_pkg::{Color, Palette, Texture, TextureFormat};

use crate::{Refused, Writer};

/// Why a texture can't be made into an [`Image`].
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

    /// The size of its largest mipmap.
    fn size(&self) -> (usize, usize) {
        (self.levels[0].width, self.levels[0].height)
    }
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
