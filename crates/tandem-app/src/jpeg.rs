//! A small baseline JPEG encoder for "Export page as JPEG" (written for Tandem PDF, MIT).
//!
//! JFIF, 8-bit YCbCr without chroma subsampling, the example quantisation and Huffman
//! tables of ITU-T T.81 Annex K scaled by quality as the IJG convention does (1–100).
//! It only has to write files every reader opens; size is secondary.

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

const LUMA_Q: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51,
    87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120,
    101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const CHROMA_Q: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99,
];

// Annex K.3 Huffman tables: code counts per length 1..16, then the symbols.
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_LUMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_CHROMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14,
    0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09,
    0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65,
    0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9,
    0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca,
    0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea,
    0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32,
    0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16,
    0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39,
    0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64,
    0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86,
    0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8,
    0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9,
    0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

/// Canonical Huffman codes (code, length) by symbol.
fn codes(bits: &[u8; 16], vals: &[u8]) -> [(u16, u8); 256] {
    let mut table = [(0u16, 0u8); 256];
    let mut code: u16 = 0;
    let mut k = 0;
    for (i, count) in bits.iter().enumerate() {
        for _ in 0..*count {
            table[vals[k] as usize] = (code, i as u8 + 1);
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    table
}

fn scaled(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut out = [0u16; 64];
    for i in 0..64 {
        out[i] = ((base[i] as u32 * scale + 50) / 100).clamp(1, 255) as u16;
    }
    out
}

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u8,
}

impl Bits {
    fn put(&mut self, code: u16, len: u8) {
        for i in (0..len).rev() {
            self.acc = (self.acc << 1) | ((code >> i) & 1) as u32;
            self.n += 1;
            if self.n == 8 {
                let byte = self.acc as u8;
                self.out.push(byte);
                if byte == 0xff {
                    self.out.push(0); // byte stuffing
                }
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn flush(&mut self) {
        while self.n != 0 {
            self.put(1, 1);
        }
    }
}

/// (category, bits) of a coefficient.
fn magnitude(v: i32) -> (u8, u16) {
    let a = v.unsigned_abs();
    let size = 32 - a.leading_zeros();
    let bits = if v < 0 { (v - 1) as u32 & ((1 << size) - 1) } else { v as u32 };
    (size as u8, bits as u16)
}

/// Forward DCT of an 8×8 block (separable, with precomputed cosines).
fn fdct(block: &[f32; 64], cos: &[[f32; 8]; 8]) -> [f32; 64] {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut s = 0.0;
            for x in 0..8 {
                s += block[y * 8 + x] * cos[u][x];
            }
            tmp[y * 8 + u] = s;
        }
    }
    let mut out = [0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut s = 0.0;
            for y in 0..8 {
                s += tmp[y * 8 + u] * cos[v][y];
            }
            out[v * 8 + u] = s;
        }
    }
    out
}

/// Encodes `rgb` (width × height × 3 bytes) as a baseline JPEG at `quality` (1–100).
/// `dpi` goes into the JFIF header.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8], quality: u8, dpi: u16) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    assert!(w > 0 && h > 0 && w < 65536 && h < 65536 && rgb.len() >= w * h * 3, "bad image");
    let qt = [scaled(&LUMA_Q, quality), scaled(&CHROMA_Q, quality)];
    let dc = [codes(&DC_LUMA_BITS, &DC_LUMA_VALS), codes(&DC_CHROMA_BITS, &DC_CHROMA_VALS)];
    let ac = [codes(&AC_LUMA_BITS, &AC_LUMA_VALS), codes(&AC_CHROMA_BITS, &AC_CHROMA_VALS)];
    let mut cos = [[0f32; 8]; 8];
    for (u, row) in cos.iter_mut().enumerate() {
        let c = if u == 0 { (0.125f32).sqrt() } else { 0.5 };
        for (x, v) in row.iter_mut().enumerate() {
            *v = c * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
        }
    }

    let mut out = Vec::with_capacity(w * h / 4 + 1024);
    out.extend_from_slice(&[0xff, 0xd8]); // SOI
    // APP0 JFIF with density in dots per inch.
    out.extend_from_slice(&[0xff, 0xe0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 1]);
    out.extend_from_slice(&dpi.to_be_bytes());
    out.extend_from_slice(&dpi.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    for (id, table) in qt.iter().enumerate() {
        out.extend_from_slice(&[0xff, 0xdb, 0, 67, id as u8]);
        for z in ZIGZAG {
            out.push(table[z] as u8);
        }
    }
    // SOF0: 8-bit, three components, no subsampling.
    out.extend_from_slice(&[0xff, 0xc0, 0, 17, 8]);
    out.extend_from_slice(&(h as u16).to_be_bytes());
    out.extend_from_slice(&(w as u16).to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
    for (class_id, bits, vals) in [
        (0x00u8, &DC_LUMA_BITS, &DC_LUMA_VALS[..]),
        (0x10, &AC_LUMA_BITS, &AC_LUMA_VALS[..]),
        (0x01, &DC_CHROMA_BITS, &DC_CHROMA_VALS[..]),
        (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALS[..]),
    ] {
        let len = 2 + 1 + 16 + vals.len();
        out.extend_from_slice(&[0xff, 0xc4]);
        out.extend_from_slice(&(len as u16).to_be_bytes());
        out.push(class_id);
        out.extend_from_slice(bits);
        out.extend_from_slice(vals);
    }
    out.extend_from_slice(&[0xff, 0xda, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    let mut bits = Bits { out, acc: 0, n: 0 };
    let mut previous_dc = [0i32; 3];
    let mut planes = [[0f32; 64]; 3];
    for by in (0..h).step_by(8) {
        for bx in (0..w).step_by(8) {
            for y in 0..8 {
                for x in 0..8 {
                    // Edge blocks repeat the last row and column.
                    let (sx, sy) = ((bx + x).min(w - 1), (by + y).min(h - 1));
                    let i = (sy * w + sx) * 3;
                    let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
                    planes[0][y * 8 + x] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    planes[1][y * 8 + x] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    planes[2][y * 8 + x] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                }
            }
            for c in 0..3 {
                let t = if c == 0 { 0 } else { 1 };
                let coefficients = fdct(&planes[c], &cos);
                let mut q = [0i32; 64];
                for (k, z) in ZIGZAG.iter().enumerate() {
                    q[k] = (coefficients[*z] / qt[t][*z] as f32).round() as i32;
                }
                let diff = q[0] - previous_dc[c];
                previous_dc[c] = q[0];
                let (size, value) = magnitude(diff);
                let (code, len) = dc[t][size as usize];
                bits.put(code, len);
                bits.put(value, size);
                let mut run = 0;
                for coefficient in q.iter().skip(1) {
                    if *coefficient == 0 {
                        run += 1;
                        continue;
                    }
                    while run > 15 {
                        let (code, len) = ac[t][0xf0];
                        bits.put(code, len);
                        run -= 16;
                    }
                    let (size, value) = magnitude(*coefficient);
                    let (code, len) = ac[t][(run << 4 | size as usize) & 0xff];
                    bits.put(code, len);
                    bits.put(value, size);
                    run = 0;
                }
                if run > 0 {
                    let (code, len) = ac[t][0x00];
                    bits.put(code, len);
                }
            }
        }
    }
    bits.flush();
    let mut out = bits.out;
    out.extend_from_slice(&[0xff, 0xd9]); // EOI
    out
}

/// Encodes RGBA pixels (alpha dropped, assumed opaque white behind).
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8], quality: u8, dpi: u16) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.as_chunks::<4>().0 {
        let a = px[3] as u32;
        for c in &px[..3] {
            rgb.push(((*c as u32 * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    encode_rgb(width, height, &rgb, quality, dpi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_well_formed_file() {
        let (w, h) = (37u32, 21u32);
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, ((x + y) * 3) as u8]);
            }
        }
        let jpeg = encode_rgb(w, h, &rgb, 85, 150);
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9]);
        assert_eq!(&jpeg[6..10], b"JFIF");
    }

    #[test]
    fn the_engine_decodes_it_back() {
        // A flat grey image round-trips through the engine's own DCT decoder (an image PDF).
        let (w, h) = (16u32, 16u32);
        let rgb = vec![120u8; (w * h * 3) as usize];
        let jpeg = encode_rgb(w, h, &rgb, 90, 72);
        let mut engine = crate::engine::EngineHost::new();
        let pdf = engine.request(
            serde_json::json!({"op": "pdf-from-images", "images": [{"name": "grey.jpg", "bytes": {"$buffer": 0}}]}),
            vec![jpeg],
        );
        assert!(pdf.is_ok(), "{:?}", pdf.err());
    }

    #[test]
    fn decodes_to_the_same_colours() {
        // A two-colour image survives the encoder and the engine's decoder within JPEG error.
        let (w, h) = (24u32, 16u32);
        let mut rgb = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(if x < 12 { &[200, 40, 40] } else { &[30, 90, 210] });
            }
        }
        let jpeg = encode_rgb(w, h, &rgb, 95, 72);
        let back = crate::imaging::decode(&jpeg).expect("decodes");
        assert_eq!((back.width, back.height), (w, h));
        let px = |x: u32, y: u32| &back.data[((y * w + x) * 4) as usize..((y * w + x) * 4 + 3) as usize];
        for (x, want) in [(3u32, [200i32, 40, 40]), (20, [30, 90, 210])] {
            let got = px(x, 8);
            for c in 0..3 {
                assert!((got[c] as i32 - want[c]).abs() <= 12, "{got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn magnitude_categories() {
        assert_eq!(magnitude(0), (0, 0));
        assert_eq!(magnitude(1), (1, 1));
        assert_eq!(magnitude(-1), (1, 0));
        assert_eq!(magnitude(-3), (2, 0));
        assert_eq!(magnitude(5), (3, 5));
    }
}
