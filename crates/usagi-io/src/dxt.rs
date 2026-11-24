//! Decoding and encoding DXT1, DXT3 and DXT5 (S3TC, BC1 to BC3) blocks.
//!
//! The encoder can't be the engine's: UnrealEd compresses with NVIDIA's
//! nvDXTlib 6.17 (`nvDXTcompress`, in `UTexture::Compress`), which the
//! engine's source has only as a compiled library. It's a principal-axis
//! fit refined by least squares, scored by decoding the block as
//! [`decode`] does, and makes what nvDXT is asked for: DXT1 without
//! alpha (`kDXT1`, not `kDXT1a`) that may use three-colour blocks
//! (`bForceDXT1FourColors` is off), and no dithering.

use std::sync::OnceLock;

use usagi_pkg::TextureFormat;

type Rgba = [u8; 4];

/// A `width`×`height` image's DXT blocks, decoded to RGBA. `data` has
/// every 4×4 block it needs, in rows; pixels of blocks past the image's
/// edges are dropped.
pub(crate) fn decode(format: TextureFormat, width: usize, height: usize, data: &[u8]) -> Vec<u8> {
    let block_size = if format == TextureFormat::Dxt1 { 8 } else { 16 };
    let blocks_wide = width.div_ceil(4);
    let mut out = vec![0; width * height * 4];
    for (i, block) in data.chunks_exact(block_size).enumerate() {
        let pixels = match format {
            TextureFormat::Dxt1 => colors(block, false),
            TextureFormat::Dxt3 => with_alpha(colors(&block[8..], true), explicit_alpha(block)),
            _ => with_alpha(colors(&block[8..], true), interpolated_alpha(block)),
        };
        let (bx, by) = (i % blocks_wide * 4, i / blocks_wide * 4);
        for (j, pixel) in pixels.iter().enumerate() {
            let (x, y) = (bx + j % 4, by + j / 4);
            if x < width && y < height {
                let at = (y * width + x) * 4;
                out[at..at + 4].copy_from_slice(pixel);
            }
        }
    }
    out
}

/// A `width`×`height` RGBA image, in rows from the top, encoded as DXT
/// blocks in `format` (`Dxt1`, `Dxt3` or anything else for `Dxt5`). The
/// pixels of blocks past the image's edges are left out of the fit.
pub(crate) fn encode(format: TextureFormat, width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let block_size = if format == TextureFormat::Dxt1 { 8 } else { 16 };
    let mut out = Vec::with_capacity(width.div_ceil(4) * height.div_ceil(4) * block_size);
    for by in (0..height).step_by(4) {
        for bx in (0..width).step_by(4) {
            let pixels: Vec<(usize, Rgba)> = (0..16)
                .filter_map(|j| {
                    let (x, y) = (bx + j % 4, by + j / 4);
                    (x < width && y < height).then(|| {
                        let at = (y * width + x) * 4;
                        (j, rgba[at..at + 4].try_into().expect("4 bytes"))
                    })
                })
                .collect();
            match format {
                TextureFormat::Dxt1 => out.extend(encode_colors(&pixels, true)),
                TextureFormat::Dxt3 => {
                    out.extend(encode_explicit_alpha(&pixels));
                    out.extend(encode_colors(&pixels, false));
                }
                _ => {
                    out.extend(encode_interpolated_alpha(&pixels));
                    out.extend(encode_colors(&pixels, false));
                }
            }
        }
    }
    out
}

/// A 5:6:5 colour as 8 bits a channel, the high bits repeated in the low.
fn rgb565(c: u16) -> [u16; 3] {
    let (r, g, b) = (c >> 11, (c >> 5) & 0x3F, c & 0x1F);
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
}

/// The four colours a colour block's indices choose from. In DXT1, a
/// block whose first colour isn't greater than its second has three
/// colours and transparent black; DXT3's and DXT5's always have four.
fn palette(c0: u16, c1: u16, four: bool) -> [Rgba; 4] {
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u16, b: u16, n: u16| -> Rgba {
        let channel = |k: usize| ((p0[k] * a + p1[k] * b) / n) as u8;
        [channel(0), channel(1), channel(2), 255]
    };
    if four || c0 > c1 {
        [mix(1, 0, 1), mix(0, 1, 1), mix(2, 1, 3), mix(1, 2, 3)]
    } else {
        [mix(1, 0, 1), mix(0, 1, 1), mix(1, 1, 2), [0; 4]]
    }
}

/// A colour block's 16 pixels.
fn colors(block: &[u8], four: bool) -> [Rgba; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let palette = palette(c0, c1, four);
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[(indices >> (2 * i) & 3) as usize])
}

/// DXT3's alpha: 4 bits a pixel.
fn explicit_alpha(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(block[..8].try_into().expect("8 bytes"));
    std::array::from_fn(|i| (bits >> (4 * i) & 0xF) as u8 * 17)
}

/// The eight values DXT5's alpha indices choose from: the two stored,
/// and values between them; with the first not greater, 0 and 255 are
/// two of the choices.
fn alpha_values(a0: u8, a1: u8) -> [u8; 8] {
    let (w0, w1) = (u16::from(a0), u16::from(a1));
    let mut values = [a0, a1, 0, 0, 0, 0, 0, 0];
    if a0 > a1 {
        for k in 1..7 {
            values[k + 1] = ((w0 * (7 - k as u16) + w1 * k as u16) / 7) as u8;
        }
    } else {
        for k in 1..5 {
            values[k + 1] = ((w0 * (5 - k as u16) + w1 * k as u16) / 5) as u8;
        }
        values[7] = 255;
    }
    values
}

/// DXT5's alpha: two values, and 3 bits a pixel choosing one of
/// [`alpha_values`].
fn interpolated_alpha(block: &[u8]) -> [u8; 16] {
    let values = alpha_values(block[0], block[1]);
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&block[2..8]);
    let bits = u64::from_le_bytes(bits);
    std::array::from_fn(|i| values[(bits >> (3 * i) & 7) as usize])
}

fn with_alpha(mut pixels: [Rgba; 16], alpha: [u8; 16]) -> [Rgba; 16] {
    for (pixel, a) in pixels.iter_mut().zip(alpha) {
        pixel[3] = a;
    }
    pixels
}

/// DXT3's alpha: each pixel's to the nearest of the 16 values 4 bits hold.
fn encode_explicit_alpha(pixels: &[(usize, Rgba)]) -> [u8; 8] {
    let mut bits = 0u64;
    for &(j, pixel) in pixels {
        bits |= u64::from((u16::from(pixel[3]) + 8) / 17) << (4 * j);
    }
    bits.to_le_bytes()
}

/// DXT5's alpha: the block's least and greatest alpha, with six values
/// between them, or, leaving out 0 and 255, which the other mode has
/// anyway, its least and greatest with four between; whichever is closer.
fn encode_interpolated_alpha(pixels: &[(usize, Rgba)]) -> [u8; 8] {
    let alphas = || pixels.iter().map(|&(_, pixel)| pixel[3]);
    let (min, max) = (
        alphas().min().expect("a pixel"),
        alphas().max().expect("a pixel"),
    );
    let inner = || alphas().filter(|&a| a != 0 && a != 255);
    // With no alpha but 0 and 255, the six-value mode's own two serve.
    let (lo, hi) = (inner().min().unwrap_or(0), inner().max().unwrap_or(0));
    let candidates = [(max, min), (lo, hi)];
    candidates
        .into_iter()
        .map(|(a0, a1)| {
            let values = alpha_values(a0, a1);
            let mut bits = 0u64;
            let mut error = 0u32;
            for &(j, pixel) in pixels {
                let (index, distance) = values
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| (i, u32::from(v.abs_diff(pixel[3])).pow(2)))
                    .min_by_key(|&(_, d)| d)
                    .expect("eight values");
                bits |= (index as u64) << (3 * j);
                error += distance;
            }
            let mut block = [0u8; 8];
            block[0] = a0;
            block[1] = a1;
            block[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
            (error, block)
        })
        .min_by_key(|&(error, _)| error)
        .expect("two candidates")
        .1
}

/// A colour block: its two colours, and each pixel's index.
#[derive(Clone, Copy)]
struct Fit {
    error: u32,
    c0: u16,
    c1: u16,
    indices: [u8; 16],
}

impl Fit {
    fn four(&self) -> bool {
        self.c0 > self.c1
    }

    fn bytes(&self) -> [u8; 8] {
        let mut out = [0; 8];
        out[..2].copy_from_slice(&self.c0.to_le_bytes());
        out[2..4].copy_from_slice(&self.c1.to_le_bytes());
        let bits = (0..16).fold(0u32, |bits, j| bits | u32::from(self.indices[j]) << (2 * j));
        out[4..].copy_from_slice(&bits.to_le_bytes());
        out
    }
}

/// The colour block closest to `pixels` (by squared distance in RGB) that
/// a few candidate pairs of colours give, in four-colour order, or with
/// `three` (DXT1), in three-colour order too.
fn encode_colors(pixels: &[(usize, Rgba)], three: bool) -> [u8; 8] {
    let first = pixels[0].1;
    let solid = pixels.iter().all(|&(_, p)| p[..3] == first[..3]);
    let mut best = if solid {
        // A colour exactly, or as near as can be, from the colours two
        // 5:6:5 colours mix to.
        let [two_thirds, half] = single_color_tables();
        let pair = |table: &[[[u8; 2]; 256]; 3]| {
            let [r, g, b] = [0, 1, 2].map(|k| table[k][usize::from(first[k])]);
            (pack(r[0], g[0], b[0]), pack(r[1], g[1], b[1]))
        };
        let (c0, c1) = pair(two_thirds);
        let fit = evaluate(pixels, c0, c1, three);
        if three {
            let (c0, c1) = pair(half);
            better(fit, evaluate(pixels, c0, c1, three))
        } else {
            fit
        }
    } else {
        let (lo, hi) = principal_extremes(pixels);
        evaluate(pixels, quantize(lo), quantize(hi), three)
    };
    // Refine the colours by least squares for the indices chosen, while
    // that helps.
    for _ in 0..4 {
        let Some((e0, e1)) = least_squares(pixels, &best) else {
            break;
        };
        let refined = evaluate(pixels, quantize(e0), quantize(e1), three);
        if refined.error >= best.error {
            break;
        }
        best = refined;
    }
    best.bytes()
}

fn better(a: Fit, b: Fit) -> Fit {
    if b.error < a.error { b } else { a }
}

/// The best block for two colours: in four-colour order, or with `three`,
/// in three-colour order if that's closer. Transparent black is never
/// chosen: UnrealEd asks for DXT1 without alpha.
fn evaluate(pixels: &[(usize, Rgba)], a: u16, b: u16, three: bool) -> Fit {
    let (hi, lo) = (a.max(b), a.min(b));
    let four = assign(pixels, hi, lo);
    if three {
        better(four, assign(pixels, lo, hi))
    } else {
        four
    }
}

/// Each pixel's nearest colour of those `c0` and `c1` give, as DXT1
/// decodes them (which DXT3 and DXT5 do too, when `c0 > c1`).
fn assign(pixels: &[(usize, Rgba)], c0: u16, c1: u16) -> Fit {
    let palette = palette(c0, c1, false);
    let choices = if c0 > c1 { 4 } else { 3 };
    let mut fit = Fit {
        error: 0,
        c0,
        c1,
        indices: [0; 16],
    };
    for &(j, pixel) in pixels {
        let (index, distance) = palette[..choices]
            .iter()
            .enumerate()
            .map(|(i, c)| (i, distance(c, &pixel)))
            .min_by_key(|&(_, d)| d)
            .expect("three or four colours");
        fit.indices[j] = index as u8;
        fit.error += distance;
    }
    fit
}

fn distance(a: &Rgba, b: &Rgba) -> u32 {
    (0..3).map(|k| u32::from(a[k].abs_diff(b[k])).pow(2)).sum()
}

/// The pixels furthest apart along the axis they vary most on.
fn principal_extremes(pixels: &[(usize, Rgba)]) -> ([f32; 3], [f32; 3]) {
    let colors: Vec<[f32; 3]> = pixels
        .iter()
        .map(|&(_, p)| [0, 1, 2].map(|k| f32::from(p[k])))
        .collect();
    let n = colors.len() as f32;
    let mean = [0, 1, 2].map(|k| colors.iter().map(|c| c[k]).sum::<f32>() / n);
    let deviation = |c: &[f32; 3]| [0, 1, 2].map(|k| c[k] - mean[k]);
    let mut covariance = [[0f32; 3]; 3];
    for c in &colors {
        let d = deviation(c);
        for i in 0..3 {
            for j in 0..3 {
                covariance[i][j] += d[i] * d[j];
            }
        }
    }
    // Power iteration, from the pixel furthest from the mean. Being one of
    // the pixels' differences from the mean, which aren't all zero, it
    // isn't one the covariance takes to zero, and nor is any step after.
    let length = |v: &[f32; 3]| v.iter().map(|x| x * x).sum::<f32>();
    let mut axis = colors
        .iter()
        .map(deviation)
        .max_by(|a, b| length(a).total_cmp(&length(b)))
        .expect("a pixel");
    for _ in 0..8 {
        let next = [0, 1, 2].map(|i| (0..3).map(|j| covariance[i][j] * axis[j]).sum::<f32>());
        let norm = length(&next).sqrt();
        axis = next.map(|v| v / norm);
    }
    let project = |c: &[f32; 3]| (0..3).map(|k| c[k] * axis[k]).sum::<f32>();
    let by_projection = |a: &&[f32; 3], b: &&[f32; 3]| project(a).total_cmp(&project(b));
    let lo = *colors.iter().min_by(by_projection).expect("a pixel");
    let hi = *colors.iter().max_by(by_projection).expect("a pixel");
    (lo, hi)
}

/// The two colours that, with each pixel's index as `fit` has it, come
/// closest to the pixels; `None` if every pixel has the same weight.
fn least_squares(pixels: &[(usize, Rgba)], fit: &Fit) -> Option<([f32; 3], [f32; 3])> {
    // How much of the first colour each index is.
    let weights: [f32; 4] = if fit.four() {
        [1.0, 0.0, 2.0 / 3.0, 1.0 / 3.0]
    } else {
        [1.0, 0.0, 0.5, 0.0]
    };
    let (mut aa, mut ab, mut bb) = (0f32, 0f32, 0f32);
    let (mut ax, mut bx) = ([0f32; 3], [0f32; 3]);
    for &(j, pixel) in pixels {
        let a = weights[usize::from(fit.indices[j])];
        let b = 1.0 - a;
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for k in 0..3 {
            ax[k] += a * f32::from(pixel[k]);
            bx[k] += b * f32::from(pixel[k]);
        }
    }
    let determinant = aa * bb - ab * ab;
    if determinant.abs() < 1e-6 {
        return None;
    }
    let e0 = [0, 1, 2].map(|k| (ax[k] * bb - bx[k] * ab) / determinant);
    let e1 = [0, 1, 2].map(|k| (bx[k] * aa - ax[k] * ab) / determinant);
    Some((e0, e1))
}

/// The nearest 5:6:5 colour.
fn quantize(c: [f32; 3]) -> u16 {
    let channel = |v: f32, max: f32| (v * max / 255.0).round().clamp(0.0, max) as u8;
    pack(
        channel(c[0], 31.0),
        channel(c[1], 63.0),
        channel(c[2], 31.0),
    )
}

fn pack(r: u8, g: u8, b: u8) -> u16 {
    u16::from(r) << 11 | u16::from(g) << 5 | u16::from(b)
}

/// For each channel (red, green, blue) and 8-bit value, the pair of 5- or
/// 6-bit values whose mix comes nearest it: two thirds of the first and
/// one of the second, then half each.
fn single_color_tables() -> &'static [[[[u8; 2]; 256]; 3]; 2] {
    static TABLES: OnceLock<[[[[u8; 2]; 256]; 3]; 2]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let table = |bits: u32, mix: fn(u16, u16) -> u16| {
            let expand = |v: u16| v << (8 - bits) | v >> (2 * bits - 8);
            let size = 1u16 << bits;
            let mut table = [[0u8; 2]; 256];
            for (value, entry) in table.iter_mut().enumerate() {
                let mut best = u16::MAX;
                for a in 0..size {
                    for b in 0..size {
                        let error = mix(expand(a), expand(b)).abs_diff(value as u16);
                        if error < best {
                            best = error;
                            *entry = [a as u8, b as u8];
                        }
                    }
                }
            }
            table
        };
        let two_thirds = |a: u16, b: u16| (2 * a + b) / 3;
        let half = |a: u16, b: u16| (a + b) / 2;
        [
            [
                table(5, two_thirds),
                table(6, two_thirds),
                table(5, two_thirds),
            ],
            [table(5, half), table(6, half), table(5, half)],
        ]
    })
}
