//! PNG codec (ISO/IEC 15948). Decoder: all colour types, bit depths 1-16,
//! palette + tRNS, Adam7 interlacing, CRC verification. Encoder: 8/16-bit
//! gray/RGB with optional alpha, adaptive filtering, parallel but deterministic.
use crate::deflate::{adler32, adler32_combine, crc32, crc32_update, deflate_chunk, zlib_decompress, zlib_header, CHUNK};
use crate::image::Image;
use vl_core::{color, par};

const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// Safety limits for untrusted input.
pub const MAX_DIM: u32 = 32768;
pub const MAX_PIXELS: u64 = 1 << 28;

#[derive(Clone, Debug, Default)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlaced: bool,
    pub text: Vec<(String, String)>,
}

/// Decoded samples in file order: `channels` per pixel, each 0..=255 (or 0..=65535 when `depth16`).
#[derive(Clone, Debug)]
pub struct RawImage {
    pub w: usize,
    pub h: usize,
    pub channels: usize,
    pub depth16: bool,
    pub data: Vec<u16>,
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn unfilter(kind: u8, bpp: usize, cur: &mut [u8], prev: &[u8]) -> Result<(), String> {
    let n = cur.len();
    match kind {
        0 => {}
        1 => {
            for i in bpp..n {
                cur[i] = cur[i].wrapping_add(cur[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                cur[i] = cur[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..n {
                let a = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                cur[i] = cur[i].wrapping_add(((a + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..n {
                let (a, c) = if i >= bpp { (cur[i - bpp], prev[i - bpp]) } else { (0, 0) };
                cur[i] = cur[i].wrapping_add(paeth(a, prev[i], c));
            }
        }
        _ => return Err(format!("invalid PNG filter type {kind}")),
    }
    Ok(())
}

const ADAM7: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

pub fn decode_png(bytes: &[u8]) -> Result<(RawImage, PngInfo), String> {
    if bytes.len() < 8 || bytes[..8] != SIG {
        return Err("not a PNG file (bad signature)".into());
    }
    let mut pos = 8;
    let mut info = PngInfo::default();
    let mut have_ihdr = false;
    let mut idat: Vec<u8> = Vec::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut seen_iend = false;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        if pos + 12 + len > bytes.len() {
            return Err("truncated PNG chunk".into());
        }
        let data = &bytes[pos + 8..pos + 8 + len];
        let crc = u32::from_be_bytes([bytes[pos + 8 + len], bytes[pos + 9 + len], bytes[pos + 10 + len], bytes[pos + 11 + len]]);
        if crc32_update(crc32(kind), data) != crc {
            return Err(format!("PNG chunk {} has a bad CRC (file is corrupt)", String::from_utf8_lossy(kind)));
        }
        pos += 12 + len;
        match kind {
            b"IHDR" => {
                if len != 13 {
                    return Err("bad IHDR length".into());
                }
                info.width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                info.height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                info.bit_depth = data[8];
                info.color_type = data[9];
                info.interlaced = data[12] == 1;
                if data[10] != 0 || data[11] != 0 || data[12] > 1 {
                    return Err("unsupported PNG compression/filter/interlace method".into());
                }
                if info.width == 0 || info.height == 0 || info.width > MAX_DIM || info.height > MAX_DIM || info.width as u64 * info.height as u64 > MAX_PIXELS {
                    return Err(format!("PNG dimensions {}x{} are outside the supported range", info.width, info.height));
                }
                let ok = matches!(
                    (info.color_type, info.bit_depth),
                    (0, 1 | 2 | 4 | 8 | 16) | (2, 8 | 16) | (3, 1 | 2 | 4 | 8) | (4, 8 | 16) | (6, 8 | 16)
                );
                if !ok {
                    return Err(format!("invalid PNG colour type {} / bit depth {}", info.color_type, info.bit_depth));
                }
                have_ihdr = true;
            }
            b"PLTE" => palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"tEXt" => {
                if let Some(z) = data.iter().position(|b| *b == 0) {
                    let k: String = data[..z].iter().map(|b| *b as char).collect();
                    let v: String = data[z + 1..].iter().map(|b| *b as char).collect();
                    info.text.push((k, v));
                }
            }
            b"IEND" => {
                seen_iend = true;
                break;
            }
            _ => {}
        }
    }
    if !have_ihdr {
        return Err("PNG has no IHDR chunk".into());
    }
    if !seen_iend && idat.is_empty() {
        return Err("PNG has no image data".into());
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let samples = match info.color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits = info.bit_depth as usize * samples;
    let bpp = bits.div_ceil(8);
    let row_bytes = |pw: usize| (pw * bits).div_ceil(8);
    let passes: Vec<(usize, usize, usize, usize, usize, usize)> = if info.interlaced {
        ADAM7
            .iter()
            .map(|&(x0, y0, dx, dy)| (x0, y0, dx, dy, (w + dx - 1 - x0) / dx, (h + dy - 1 - y0) / dy))
            .map(|(x0, y0, dx, dy, pw, ph)| (x0, y0, dx, dy, if w > x0 { pw } else { 0 }, if h > y0 { ph } else { 0 }))
            .collect()
    } else {
        vec![(0, 0, 1, 1, w, h)]
    };
    let expected: usize = passes.iter().filter(|p| p.4 > 0 && p.5 > 0).map(|p| p.5 * (1 + row_bytes(p.4))).sum();
    let raw = zlib_decompress(&idat, expected, expected + 64).map_err(|e| format!("PNG data: {e}"))?;
    if raw.len() < expected {
        return Err("PNG image data is shorter than the header promises".into());
    }

    let (channels, depth16) = match info.color_type {
        0 => (if trns.len() >= 2 { 2 } else { 1 }, info.bit_depth == 16),
        2 => (if trns.len() >= 6 { 4 } else { 3 }, info.bit_depth == 16),
        3 => (if trns.is_empty() { 3 } else { 4 }, false),
        4 => (2, info.bit_depth == 16),
        _ => (4, info.bit_depth == 16),
    };
    if info.color_type == 3 && palette.is_empty() {
        return Err("palette PNG without a PLTE chunk".into());
    }
    let maxv: u16 = if depth16 { 65535 } else { 255 };
    let mut out = vec![0u16; w * h * channels];
    let key_gray = if info.color_type == 0 && trns.len() >= 2 { Some(u16::from_be_bytes([trns[0], trns[1]])) } else { None };
    let key_rgb = if info.color_type == 2 && trns.len() >= 6 {
        Some([u16::from_be_bytes([trns[0], trns[1]]), u16::from_be_bytes([trns[2], trns[3]]), u16::from_be_bytes([trns[4], trns[5]])])
    } else {
        None
    };
    let bd = info.bit_depth as usize;
    let scale_small = |v: u16| -> u16 {
        match bd {
            1 => v * 255,
            2 => v * 85,
            4 => v * 17,
            _ => v,
        }
    };

    let mut off = 0usize;
    for &(x0, y0, dx, dy, pw, ph) in &passes {
        if pw == 0 || ph == 0 {
            continue;
        }
        let rb = row_bytes(pw);
        let mut prev = vec![0u8; rb];
        let mut cur = vec![0u8; rb];
        for py in 0..ph {
            let ft = raw[off];
            cur.copy_from_slice(&raw[off + 1..off + 1 + rb]);
            off += 1 + rb;
            unfilter(ft, bpp, &mut cur, &prev)?;
            let y = y0 + py * dy;
            // read sample `s` of pixel `px` in this row
            let sample = |px: usize, s: usize| -> u16 {
                match bd {
                    8 => cur[px * samples + s] as u16,
                    16 => {
                        let i = (px * samples + s) * 2;
                        u16::from_be_bytes([cur[i], cur[i + 1]])
                    }
                    _ => {
                        let bit = px * bd;
                        let shift = 8 - bd - (bit % 8);
                        ((cur[bit / 8] >> shift) & ((1u8 << bd) - 1)) as u16
                    }
                }
            };
            for px in 0..pw {
                let x = x0 + px * dx;
                let o = (y * w + x) * channels;
                match info.color_type {
                    0 => {
                        let v = sample(px, 0);
                        out[o] = scale_small(v);
                        if let Some(k) = key_gray {
                            out[o + 1] = if v == k { 0 } else { maxv };
                        }
                    }
                    2 => {
                        let v = [sample(px, 0), sample(px, 1), sample(px, 2)];
                        out[o..o + 3].copy_from_slice(&v);
                        if let Some(k) = key_rgb {
                            out[o + 3] = if v == k { 0 } else { maxv };
                        }
                    }
                    3 => {
                        let idx = sample(px, 0) as usize;
                        let c = palette.get(idx).copied().ok_or("PNG palette index out of range")?;
                        out[o] = c[0] as u16;
                        out[o + 1] = c[1] as u16;
                        out[o + 2] = c[2] as u16;
                        if channels == 4 {
                            out[o + 3] = trns.get(idx).copied().unwrap_or(255) as u16;
                        }
                    }
                    4 => {
                        out[o] = sample(px, 0);
                        out[o + 1] = sample(px, 1);
                    }
                    _ => {
                        for s in 0..4 {
                            out[o + s] = sample(px, s);
                        }
                    }
                }
            }
            std::mem::swap(&mut prev, &mut cur);
        }
    }
    Ok((RawImage { w, h, channels, depth16, data: out }, info))
}

fn srgb16_lut() -> &'static Vec<f32> {
    use std::sync::OnceLock;
    static LUT: OnceLock<Vec<f32>> = OnceLock::new();
    LUT.get_or_init(|| (0..65536u32).map(|i| color::srgb_to_linear(i as f32 / 65535.0)).collect())
}

impl RawImage {
    /// Interpret samples as sRGB-encoded and convert to the linear float working format.
    pub fn to_image(&self) -> Image {
        let mut img = Image::new(self.w, self.h);
        let maxv = if self.depth16 { 65535.0 } else { 255.0 };
        let lut16 = if self.depth16 { Some(srgb16_lut()) } else { None };
        let lin = |v: u16| -> f32 {
            match lut16 {
                Some(l) => l[v as usize],
                None => color::srgb8_to_linear(v as u8),
            }
        };
        for (i, px) in self.data.chunks_exact(self.channels).enumerate() {
            let (r, g, b, a) = match self.channels {
                1 => (px[0], px[0], px[0], maxv),
                2 => (px[0], px[0], px[0], px[1] as f32),
                3 => (px[0], px[1], px[2], maxv),
                _ => (px[0], px[1], px[2], px[3] as f32),
            };
            let o = i * 4;
            img.data[o] = lin(r);
            img.data[o + 1] = lin(g);
            img.data[o + 2] = lin(b);
            img.data[o + 3] = a / maxv;
        }
        img
    }
}

#[derive(Clone, Debug)]
pub struct PngOptions {
    /// 8 or 16 bits per sample.
    pub bit_depth: u8,
    /// Deflate effort 0..=9.
    pub level: u8,
    pub threads: usize,
    /// tEXt key/value pairs (provenance: program hash, seed, engine version...).
    pub text: Vec<(String, String)>,
}

impl Default for PngOptions {
    fn default() -> Self {
        PngOptions { bit_depth: 8, level: 5, threads: par::available_threads(), text: Vec::new() }
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32_update(crc32(kind), data).to_be_bytes());
}

fn filter_row(kind: u8, bpp: usize, cur: &[u8], prev: &[u8], out: &mut [u8]) {
    let n = cur.len();
    match kind {
        0 => out.copy_from_slice(cur),
        1 => {
            out[..bpp.min(n)].copy_from_slice(&cur[..bpp.min(n)]);
            for i in bpp..n {
                out[i] = cur[i].wrapping_sub(cur[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                out[i] = cur[i].wrapping_sub(prev[i]);
            }
        }
        3 => {
            for i in 0..n {
                let a = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                out[i] = cur[i].wrapping_sub(((a + prev[i] as u16) / 2) as u8);
            }
        }
        _ => {
            for i in 0..n {
                let (a, c) = if i >= bpp { (cur[i - bpp], prev[i - bpp]) } else { (0, 0) };
                out[i] = cur[i].wrapping_sub(paeth(a, prev[i], c));
            }
        }
    }
}

/// Encode a PNG from a row callback. `row(y, buf)` must fill `buf` with the raw
/// scanline: `channels` samples per pixel, big-endian when `bit_depth == 16`.
/// The callback lets callers stream from tiles without building a full 8/16-bit copy.
pub fn encode_png_rows<F>(w: usize, h: usize, channels: usize, opts: &PngOptions, row: F) -> Vec<u8>
where
    F: Fn(usize, &mut [u8]) + Sync,
{
    let depth = if opts.bit_depth == 16 { 16 } else { 8 };
    let bpp = channels * depth / 8;
    let rb = w * bpp;
    let color_type = match channels {
        1 => 0,
        2 => 4,
        3 => 2,
        _ => 6,
    };
    let rows_per_band = (CHUNK / (rb + 1)).max(1);
    let nbands = h.div_ceil(rows_per_band).max(1);
    let level = opts.level;
    let parts = par::par_map(nbands, opts.threads, |b| {
        let y0 = b * rows_per_band;
        let y1 = (y0 + rows_per_band).min(h);
        let mut filtered = Vec::with_capacity((y1 - y0) * (rb + 1));
        let mut prev = vec![0u8; rb];
        let mut cur = vec![0u8; rb];
        if y0 > 0 {
            row(y0 - 1, &mut prev);
        }
        let mut cand = vec![0u8; rb];
        let mut best = vec![0u8; rb];
        for y in y0..y1 {
            row(y, &mut cur);
            let mut best_score = u64::MAX;
            let mut best_kind = 0u8;
            for kind in 0..5u8 {
                filter_row(kind, bpp, &cur, &prev, &mut cand);
                let score: u64 = cand.iter().map(|v| (*v as i8).unsigned_abs() as u64).sum();
                if score < best_score {
                    best_score = score;
                    best_kind = kind;
                    std::mem::swap(&mut best, &mut cand);
                }
            }
            filtered.push(best_kind);
            filtered.extend_from_slice(&best);
            std::mem::swap(&mut prev, &mut cur);
        }
        let z = deflate_chunk(&filtered, level, b + 1 == nbands);
        (z, adler32(&filtered), filtered.len() as u64)
    });

    let mut out = Vec::with_capacity(parts.iter().map(|p| p.0.len()).sum::<usize>() + 256);
    out.extend_from_slice(&SIG);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[depth as u8, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"sRGB", &[0]);
    chunk(&mut out, b"gAMA", &45455u32.to_be_bytes());
    for (k, v) in &opts.text {
        let mut d: Vec<u8> = k.chars().filter(|c| (*c as u32) >= 32 && (*c as u32) < 127).take(79).map(|c| c as u8).collect();
        d.push(0);
        d.extend(v.chars().map(|c| if (c as u32) < 256 && c != '\0' { c as u8 } else { b'?' }));
        chunk(&mut out, b"tEXt", &d);
    }
    let mut z: Vec<u8> = Vec::with_capacity(parts.iter().map(|p| p.0.len()).sum::<usize>() + 6);
    z.extend_from_slice(&zlib_header(level));
    let mut adler = 1u32;
    for (data, a, len) in &parts {
        z.extend_from_slice(data);
        adler = adler32_combine(adler, *a, *len);
    }
    z.extend_from_slice(&adler.to_be_bytes());
    for c in z.chunks(1 << 20) {
        chunk(&mut out, b"IDAT", c);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode a linear float image as sRGB PNG. Alpha is written only when `alpha` is true.
pub fn encode_png(img: &Image, alpha: bool, opts: &PngOptions) -> Vec<u8> {
    let channels = if alpha { 4 } else { 3 };
    let sixteen = opts.bit_depth == 16;
    encode_png_rows(img.w, img.h, channels, opts, |y, buf| {
        let src = img.row(y);
        if sixteen {
            for (px, o) in src.chunks_exact(4).zip(buf.chunks_exact_mut(channels * 2)) {
                for c in 0..3 {
                    o[c * 2..c * 2 + 2].copy_from_slice(&color::to_srgb16(px[c]).to_be_bytes());
                }
                if alpha {
                    o[6..8].copy_from_slice(&((px[3].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes());
                }
            }
        } else {
            for (px, o) in src.chunks_exact(4).zip(buf.chunks_exact_mut(channels)) {
                o[0] = color::to_srgb8(px[0]);
                o[1] = color::to_srgb8(px[1]);
                o[2] = color::to_srgb8(px[2]);
                if alpha {
                    o[3] = (px[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::rng::Rng;

    fn test_image(w: usize, h: usize, alpha: bool) -> Image {
        let mut r = Rng::new(9, 9);
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = [x as f32 / w as f32, y as f32 / h as f32, r.f32(), if alpha { ((x + y) % 5) as f32 / 4.0 } else { 1.0 }];
                img.set(x, y, v);
            }
        }
        img
    }

    #[test]
    fn roundtrip_8_and_16_bit() {
        for (w, h) in [(1, 1), (7, 3), (64, 33), (301, 211)] {
            for alpha in [false, true] {
                for depth in [8u8, 16] {
                    let img = test_image(w, h, alpha);
                    let opts = PngOptions { bit_depth: depth, level: 6, threads: 2, text: vec![("vl:test".into(), "ok".into())] };
                    let bytes = encode_png(&img, alpha, &opts);
                    let (raw, info) = decode_png(&bytes).unwrap();
                    assert_eq!((raw.w, raw.h, raw.channels, raw.depth16), (w, h, if alpha { 4 } else { 3 }, depth == 16));
                    assert_eq!(info.text, vec![("vl:test".to_string(), "ok".to_string())]);
                    let back = raw.to_image();
                    // quantisation is the only loss: re-encoding must be byte-identical
                    assert_eq!(encode_png(&back, alpha, &opts), bytes, "{w}x{h} alpha={alpha} depth={depth}");
                    let tol = if depth == 16 { 2e-5 } else { 4e-3 };
                    for (a, b) in img.data.chunks_exact(4).zip(back.data.chunks_exact(4)) {
                        for c in 0..3 {
                            let (sa, sb) = (color::linear_to_srgb(a[c]), color::linear_to_srgb(b[c]));
                            assert!((sa - sb).abs() <= tol, "{sa} vs {sb}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn deterministic_across_threads_and_large_enough_for_many_bands() {
        let img = test_image(900, 700, false);
        let a = encode_png(&img, false, &PngOptions { threads: 1, ..Default::default() });
        let b = encode_png(&img, false, &PngOptions { threads: 4, ..Default::default() });
        assert_eq!(a, b);
        assert!(900 * 700 * 3 > CHUNK * 2, "test must span several compression bands");
        let (raw, _) = decode_png(&a).unwrap();
        assert_eq!(raw.to_image().to_srgb8(), img.to_srgb8());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode_png(b"not a png").is_err());
        let img = test_image(20, 20, false);
        let mut bytes = encode_png(&img, false, &PngOptions::default());
        let n = bytes.len();
        bytes[n / 2] ^= 0x55;
        assert!(decode_png(&bytes).is_err(), "CRC must catch corruption");
        assert!(decode_png(&bytes[..n / 2]).is_err());
    }

    fn check_fixture(name: &str, png: &[u8], expect: &[u8]) {
        let (raw, info) = decode_png(png).unwrap_or_else(|e| panic!("{name}: {e}"));
        let rd = |i: usize| u32::from_le_bytes([expect[i], expect[i + 1], expect[i + 2], expect[i + 3]]) as usize;
        let (w, h, ch, d16) = (rd(0), rd(4), rd(8), rd(12) == 1);
        assert_eq!((raw.w, raw.h, raw.channels, raw.depth16), (w, h, ch, d16), "{name} header {info:?}");
        let want: Vec<u16> = expect[16..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(raw.data, want, "{name} samples");
    }

    /// Fixtures written by an independent encoder (CPython zlib, all five filter
    /// types, every colour type / bit depth, Adam7, palette + tRNS).
    #[test]
    fn decodes_reference_fixtures() {
        macro_rules! fx {
            ($($n:literal),*) => {$(
                check_fixture($n, include_bytes!(concat!("../tests/data/png/", $n, ".png")), include_bytes!(concat!("../tests/data/png/", $n, ".expect")));
            )*};
        }
        fx!("gray1", "gray2", "gray4", "gray8", "gray16", "gray8_trns", "rgb8", "rgb16", "rgb8_trns", "pal1", "pal2", "pal4", "pal8", "pal8_trns", "ga8", "ga16", "rgba8", "rgba16", "rgb8_adam7", "rgba16_adam7", "gray1_adam7", "pal4_adam7", "pillow_rgb", "pillow_rgba", "pillow_p", "pillow_l");
    }
}
