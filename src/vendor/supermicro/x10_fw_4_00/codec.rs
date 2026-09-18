//! AST2100 block stream: 32-bit little-endian words containing MSB-first bits,
//! JPEG entropy-coded blocks and vector-quantized palette blocks. No JS runtime.
use super::tables::*;
use anyhow::{Result, bail, ensure};
use image::{Rgba, RgbaImage};

struct Bits<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, count: usize) -> Result<u32> {
        ensure!(count <= 24, "invalid bit count");
        let mut value = 0;
        for _ in 0..count {
            let word = self.pos / 32;
            let in_word = self.pos % 32;
            let i = word * 4 + 3 - in_word / 8;
            ensure!(i < self.bytes.len(), "truncated AST bitstream");
            value = (value << 1) | u32::from((self.bytes[i] >> (7 - in_word % 8)) & 1);
            self.pos += 1;
        }
        Ok(value)
    }
    fn signed(&mut self, n: usize) -> Result<i32> {
        if n == 0 {
            return Ok(0);
        }
        ensure!(n <= 16, "invalid JPEG magnitude");
        let v = self.take(n)? as i32;
        Ok(if v < (1 << (n - 1)) {
            v - ((1 << n) - 1)
        } else {
            v
        })
    }
}

fn huffman(bits: &mut Bits<'_>, counts: &[u8], values: &[u8]) -> Result<u8> {
    let (mut code, mut first, mut offset) = (0u32, 0u32, 0usize);
    for &count in counts.iter().take(17).skip(1) {
        code = (code << 1) | bits.take(1)?;
        if code >= first && code < first + u32::from(count) {
            return values
                .get(offset + (code - first) as usize)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("invalid Huffman table"));
        }
        first = (first + u32::from(count)) << 1;
        offset += usize::from(count);
    }
    bail!("invalid Huffman code")
}

fn coefficients(bits: &mut Bits<'_>, chroma: bool, previous: &mut i32) -> Result<[i32; 64]> {
    let (dc_counts, dc_values, ac_counts, ac_values) = if chroma {
        (
            &DC_C_COUNTS[..],
            &DC_C_VALUES[..],
            &AC_C_COUNTS[..],
            &AC_C_VALUES[..],
        )
    } else {
        (
            &DC_Y_COUNTS[..],
            &DC_Y_VALUES[..],
            &AC_Y_COUNTS[..],
            &AC_Y_VALUES[..],
        )
    };
    let mut c = [0; 64];
    let size = huffman(bits, dc_counts, dc_values)? as usize;
    *previous = previous
        .checked_add(bits.signed(size)?)
        .ok_or_else(|| anyhow::anyhow!("DC overflow"))?;
    ensure!(previous.abs() <= 32767, "invalid DC coefficient");
    c[0] = *previous;
    let mut i = 1;
    while i < 64 {
        let rs = huffman(bits, ac_counts, ac_values)?;
        if rs == 0 {
            break;
        }
        if rs == 0xf0 {
            i += 16;
            ensure!(i <= 64, "invalid AC zero run");
            continue;
        }
        let size = (rs & 15) as usize;
        ensure!(size > 0 && size <= 10, "invalid AC magnitude");
        i += (rs >> 4) as usize;
        ensure!(i < 64, "invalid AC run");
        c[ZIGZAG[i]] = bits.signed(size)?;
        i += 1;
    }
    Ok(c)
}

/// Separable reference IDCT. Kept deliberately independent of the firmware's
/// fixed-point implementation; rounding can differ by a few levels.
fn idct(c: &[i32; 64], q: &[u8; 64], basis: &[[f64; 8]; 8]) -> [u8; 64] {
    if c[1..].iter().all(|&v| v == 0) {
        return [((c[0] as f64 * q[0] as f64 / 8.0 + 128.0)
            .round()
            .clamp(0.0, 255.0)) as u8; 64];
    }
    let mut tmp = [[0.0; 8]; 8];
    let mut out = [0; 64];
    for v in 0..8 {
        for (x, b) in basis.iter().enumerate() {
            tmp[v][x] = (0..8)
                .map(|u| b[u] * c[v * 8 + u] as f64 * q[v * 8 + u] as f64)
                .sum();
        }
    }
    for (y, b) in basis.iter().enumerate() {
        for x in 0..8 {
            let val = (0..8).map(|v| b[v] * tmp[v][x]).sum::<f64>() / 4.0 + 128.0;
            out[y * 8 + x] = val.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn rgb(y: u8, cb: u8, cr: u8) -> Rgba<u8> {
    let y = (1.164 * (y as f64 - 16.0)).round() as i32;
    let cb = cb as f64 - 128.0;
    let cr = cr as f64 - 128.0;
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    Rgba([
        clamp(y + (1.597656 * cr).round() as i32),
        clamp(y + (-0.390625 * cb).round() as i32 + (-0.8125 * cr).round() as i32),
        clamp(y + (2.015625 * cb).round() as i32),
        255,
    ])
}

pub struct Decoder {
    frame: RgbaImage,
    basis: [[f64; 8]; 8],
}
impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}
impl Decoder {
    pub fn new() -> Self {
        let mut basis = [[0.0; 8]; 8];
        for (x, row) in basis.iter_mut().enumerate() {
            for (u, v) in row.iter_mut().enumerate() {
                *v = if u == 0 {
                    1.0 / 2.0f64.sqrt()
                } else {
                    (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos()
                };
            }
        }
        Self {
            frame: RgbaImage::new(0, 0),
            basis,
        }
    }
    pub fn decode(&mut self, width: u32, height: u32, data: &[u8]) -> Result<&RgbaImage> {
        ensure!(
            width > 0 && height > 0 && width <= 1920 && height <= 1280,
            "unsupported frame dimensions {width}x{height}"
        );
        ensure!(
            data.len() >= 8 && data.len() <= 16 * 1024 * 1024,
            "invalid AST payload length"
        );
        let (yq, cq) = (data[0] as usize, data[1] as usize);
        ensure!(yq < 12 && cq < 12, "unsupported quantization selector");
        let mode = u16::from_be_bytes([data[2], data[3]]);
        // Firmware names mode 422 but decodes it as four Y blocks (4:2:0).
        let subsampled = match mode {
            422 | 420 => true,
            444 => false,
            _ => bail!("unsupported AST color mode {mode}"),
        };
        let tile = if subsampled { 16 } else { 8 };
        let cols = width.div_ceil(tile);
        let rows = height.div_ceil(tile);
        let mut frame = if self.frame.dimensions() == (width, height) {
            self.frame.clone()
        } else {
            RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]))
        };
        let mut bits = Bits::new(&data[4..]);
        let (mut tx, mut ty) = (0, 0);
        let mut dc = [0; 3];
        let mut palette = [0x008080u32, 0xff8080, 0x808080, 0xc08080];
        for _ in 0..(cols * rows * 4 + 4096) {
            let code = bits.take(4)?;
            if code == 9 {
                self.frame = frame;
                return Ok(&self.frame);
            }
            ensure!(
                matches!(code, 0 | 4 | 5 | 6 | 7 | 8 | 12 | 13 | 14 | 15),
                "unsupported AST block code {code}"
            );
            if code & 8 != 0 {
                tx = bits.take(8)?;
                ty = bits.take(8)?;
            }
            ensure!(tx < cols && ty < rows, "AST tile outside framebuffer");
            if matches!(code, 0 | 4 | 8 | 12) {
                let low = code & 4 != 0;
                let qy = &QUANT_Y[if low { 0 } else { yq }];
                let qc = &QUANT_C[if low { 0 } else { cq }];
                let yn = if subsampled { 4 } else { 1 };
                let mut blocks = Vec::with_capacity(yn + 2);
                for _ in 0..yn {
                    blocks.push(idct(
                        &coefficients(&mut bits, false, &mut dc[0])?,
                        qy,
                        &self.basis,
                    ));
                }
                blocks.push(idct(
                    &coefficients(&mut bits, true, &mut dc[1])?,
                    qc,
                    &self.basis,
                ));
                blocks.push(idct(
                    &coefficients(&mut bits, true, &mut dc[2])?,
                    qc,
                    &self.basis,
                ));
                for y in 0..tile {
                    for x in 0..tile {
                        let yi = if subsampled {
                            (y / 8 * 2 + x / 8) as usize
                        } else {
                            0
                        };
                        let yv = blocks[yi][(y % 8 * 8 + x % 8) as usize];
                        let ci = if subsampled {
                            (y / 2 * 8 + x / 2) as usize
                        } else {
                            (y * 8 + x) as usize
                        };
                        let (px, py) = (tx * tile + x, ty * tile + y);
                        if px < width && py < height {
                            frame.put_pixel(px, py, rgb(yv, blocks[yn][ci], blocks[yn + 1][ci]));
                        }
                    }
                }
            } else {
                ensure!(
                    !subsampled,
                    "VQ blocks in subsampled mode are not supported"
                );
                let n = match code & 7 {
                    5 => 1,
                    6 => 2,
                    7 => 4,
                    _ => unreachable!(),
                };
                let mut indices = [0; 4];
                for index in indices.iter_mut().take(n) {
                    let update = bits.take(1)? != 0;
                    *index = bits.take(2)? as usize;
                    if update {
                        palette[*index] = bits.take(24)?;
                    }
                }
                for y in 0..8 {
                    for x in 0..8 {
                        let pi = match n {
                            1 => 0,
                            2 => bits.take(1)? as usize,
                            _ => bits.take(2)? as usize,
                        };
                        let c = palette[indices[pi]];
                        let (px, py) = (tx * 8 + x, ty * 8 + y);
                        if px < width && py < height {
                            frame.put_pixel(px, py, rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
                        }
                    }
                }
            }
            tx += 1;
            if tx >= cols {
                tx = 0;
                ty = (ty + 1) % rows;
            }
        }
        bail!("AST stream has no frame-end marker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(s: &str) -> Vec<u8> {
        let mut bits = s.to_string();
        while !bits.len().is_multiple_of(32) {
            bits.push('0');
        }
        let mut out = vec![7, 7, 1, 188];
        for b in bits.as_bytes().chunks(32) {
            out.extend(
                u32::from_str_radix(std::str::from_utf8(b).unwrap(), 2)
                    .unwrap()
                    .to_le_bytes(),
            );
        }
        out
    }
    #[test]
    fn vq_full_frame_and_incremental_preservation() {
        // Two white 8x8 tiles, then update only the second tile to black.
        let white = "0101".to_owned() + "100" + "111010111000000010000000";
        let mut d = Decoder::new();
        let p = stream(&(white + "0101000" + "1001"));
        let f = d.decode(16, 8, &p).unwrap();
        assert!(f.get_pixel(0, 0)[0] > 250);
        assert!(f.get_pixel(15, 7)[0] > 250);
        let p = stream("110100000001000000001000001000010000000100000001001");
        let f = d.decode(16, 8, &p).unwrap();
        assert!(f.get_pixel(0, 0)[0] > 250);
        assert_eq!(f.get_pixel(8, 0)[0], 0);
    }
    #[test]
    fn rejects_truncated_and_invalid_dimensions() {
        let mut d = Decoder::new();
        assert!(d.decode(640, 480, &[0; 5]).is_err());
        assert!(d.decode(9999, 480, &[0; 8]).is_err());
    }
    #[test]
    fn jpeg_dc_block_and_entropy_sign() {
        // JPEG tile: Y DC=0 (00), EOB (1010), two chroma DC=0/EOB (00 00), end (1001).
        let p = stream("0000001010000000001001");
        let mut d = Decoder::new();
        let f = d.decode(8, 8, &p).unwrap();
        assert!(f.pixels().all(|p| *p == rgb(128, 128, 128)));
        let mut c = [0; 64];
        c[0] = 80;
        assert_eq!(idct(&c, &[1; 64], &d.basis), [138; 64]);
        let p = stream("010111");
        let mut b = Bits::new(&p[4..]);
        assert_eq!(b.signed(3).unwrap(), -5);
        assert_eq!(b.signed(3).unwrap(), 7);
    }
    #[test]
    fn failed_delta_never_publishes_partially_decoded_frame() {
        let mut d = Decoder::new();
        let white = "0101100111010111000000010000000";
        let p = stream(&(white.to_owned() + "1001"));
        let before = d.decode(8, 8, &p).unwrap().clone();
        // Valid black VQ tile followed by unsupported block code 1.
        let bad = stream("01011000001000010000000100000000001");
        assert!(d.decode(8, 8, &bad).is_err());
        assert_eq!(d.decode(8, 8, &stream("1001")).unwrap(), &before);
    }
}
