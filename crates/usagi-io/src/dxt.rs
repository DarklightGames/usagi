//! Decoding DXT1, DXT3 and DXT5 (S3TC, BC1 to BC3) blocks.

use usagi_pkg::TextureFormat;

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

/// A 5:6:5 colour as 8 bits a channel, the high bits repeated in the low.
fn rgb565(c: u16) -> [u16; 3] {
    let (r, g, b) = (c >> 11, (c >> 5) & 0x3F, c & 0x1F);
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
}

/// A colour block's 16 pixels. In DXT1, a block whose first colour isn't
/// greater than its second has three colours and transparent black;
/// DXT3's and DXT5's always have four.
fn colors(block: &[u8], four: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    let mix = |a: u16, b: u16, n: u16| -> [u8; 4] {
        let channel = |k: usize| ((p0[k] * a + p1[k] * b) / n) as u8;
        [channel(0), channel(1), channel(2), 255]
    };
    let palette = if four || c0 > c1 {
        [mix(1, 0, 1), mix(0, 1, 1), mix(2, 1, 3), mix(1, 2, 3)]
    } else {
        [mix(1, 0, 1), mix(0, 1, 1), mix(1, 1, 2), [0; 4]]
    };
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[(indices >> (2 * i) & 3) as usize])
}

/// DXT3's alpha: 4 bits a pixel.
fn explicit_alpha(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(block[..8].try_into().expect("8 bytes"));
    std::array::from_fn(|i| (bits >> (4 * i) & 0xF) as u8 * 17)
}

/// DXT5's alpha: two values, and 3 bits a pixel choosing one of them or
/// a value between them; with the first not greater, 0 and 255 are two of
/// the choices.
fn interpolated_alpha(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (u16::from(block[0]), u16::from(block[1]));
    let mut values = [0u8; 8];
    values[0] = block[0];
    values[1] = block[1];
    if a0 > a1 {
        for k in 1..7 {
            values[k + 1] = ((a0 * (7 - k as u16) + a1 * k as u16) / 7) as u8;
        }
    } else {
        for k in 1..5 {
            values[k + 1] = ((a0 * (5 - k as u16) + a1 * k as u16) / 5) as u8;
        }
        values[7] = 255;
    }
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&block[2..8]);
    let bits = u64::from_le_bytes(bits);
    std::array::from_fn(|i| values[(bits >> (3 * i) & 7) as usize])
}

fn with_alpha(mut pixels: [[u8; 4]; 16], alpha: [u8; 16]) -> [[u8; 4]; 16] {
    for (pixel, a) in pixels.iter_mut().zip(alpha) {
        pixel[3] = a;
    }
    pixels
}
