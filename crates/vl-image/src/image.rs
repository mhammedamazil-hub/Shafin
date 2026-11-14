//! The in-memory raster: linear-light RGBA `f32`, straight alpha, origin top-left.
use vl_core::color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Rect {
    pub fn new(x: i64, y: i64, w: i64, h: i64) -> Rect {
        Rect { x, y, w: w.max(0), h: h.max(0) }
    }
    pub fn of_size(w: usize, h: usize) -> Rect {
        Rect { x: 0, y: 0, w: w as i64, h: h as i64 }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn x1(&self) -> i64 {
        self.x + self.w
    }
    pub fn y1(&self) -> i64 {
        self.y + self.h
    }
    pub fn area(&self) -> i64 {
        self.w.max(0) * self.h.max(0)
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.x1().min(o.x1());
        let y1 = self.y1().min(o.y1());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        Rect::new(x0, y0, self.x1().max(o.x1()) - x0, self.y1().max(o.y1()) - y0)
    }
    pub fn expand(&self, m: i64) -> Rect {
        Rect::new(self.x - m, self.y - m, self.w + 2 * m, self.h + 2 * m)
    }
    pub fn contains(&self, x: i64, y: i64) -> bool {
        x >= self.x && y >= self.y && x < self.x1() && y < self.y1()
    }
    pub fn to_array(&self) -> [i64; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

#[derive(Clone, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    /// RGBA, linear light, straight alpha; `len == w * h * 4`.
    pub data: Vec<f32>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.w, self.h)
    }
}

impl Image {
    /// Transparent black.
    pub fn new(w: usize, h: usize) -> Image {
        Image { w, h, data: vec![0.0; w * h * 4] }
    }
    pub fn filled(w: usize, h: usize, rgba: [f32; 4]) -> Image {
        let mut data = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            data.extend_from_slice(&rgba);
        }
        Image { w, h, data }
    }
    pub fn rect(&self) -> Rect {
        Rect::of_size(self.w, self.h)
    }
    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        (y * self.w + x) * 4
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [f32; 4] {
        let i = self.idx(x, y);
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, p: [f32; 4]) {
        let i = self.idx(x, y);
        self.data[i..i + 4].copy_from_slice(&p);
    }
    /// Clamped fetch (edge replicate).
    #[inline]
    pub fn get_clamped(&self, x: i64, y: i64) -> [f32; 4] {
        self.get(x.clamp(0, self.w as i64 - 1) as usize, y.clamp(0, self.h as i64 - 1) as usize)
    }
    pub fn row(&self, y: usize) -> &[f32] {
        &self.data[y * self.w * 4..(y + 1) * self.w * 4]
    }
    pub fn row_mut(&mut self, y: usize) -> &mut [f32] {
        let w = self.w;
        &mut self.data[y * w * 4..(y + 1) * w * 4]
    }
    /// Copy of a sub-rectangle; areas outside the image are transparent.
    pub fn crop(&self, r: Rect) -> Image {
        let mut out = Image::new(r.w.max(0) as usize, r.h.max(0) as usize);
        let i = r.intersect(&self.rect());
        if i.is_empty() {
            return out;
        }
        for y in i.y..i.y1() {
            let src = &self.row(y as usize)[(i.x as usize) * 4..(i.x1() as usize) * 4];
            let dx = (i.x - r.x) as usize;
            let dy = (y - r.y) as usize;
            let w = out.w;
            out.data[(dy * w + dx) * 4..(dy * w + dx) * 4 + src.len()].copy_from_slice(src);
        }
        out
    }
    /// Overwrite (no blending) the region at (x, y) with `src`.
    pub fn blit(&mut self, src: &Image, x: i64, y: i64) {
        let dst = Rect::new(x, y, src.w as i64, src.h as i64).intersect(&self.rect());
        if dst.is_empty() {
            return;
        }
        for yy in dst.y..dst.y1() {
            let sy = (yy - y) as usize;
            let sx = (dst.x - x) as usize;
            let s = &src.row(sy)[sx * 4..(sx + dst.w as usize) * 4];
            let w = self.w;
            let o = (yy as usize * w + dst.x as usize) * 4;
            self.data[o..o + s.len()].copy_from_slice(s);
        }
    }
    /// Bilinear sample at continuous pixel coordinates (pixel centres at +0.5), premultiplied-correct.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut acc = [0.0f32; 4];
        for (dx, dy, wgt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let p = self.get_clamped(x0 + dx, y0 + dy);
            acc[0] += p[0] * p[3] * wgt;
            acc[1] += p[1] * p[3] * wgt;
            acc[2] += p[2] * p[3] * wgt;
            acc[3] += p[3] * wgt;
        }
        if acc[3] > 1e-8 {
            [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3]]
        } else {
            [0.0; 4]
        }
    }
    /// 8-bit sRGB RGBA bytes (alpha linear).
    pub fn to_srgb8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for p in self.data.chunks_exact(4) {
            out.push(color::to_srgb8(p[0]));
            out.push(color::to_srgb8(p[1]));
            out.push(color::to_srgb8(p[2]));
            out.push((p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
        out
    }
    pub fn from_srgb8(w: usize, h: usize, channels: usize, bytes: &[u8]) -> Image {
        let mut img = Image::new(w, h);
        for (i, px) in bytes.chunks_exact(channels).take(w * h).enumerate() {
            let (r, g, b, a) = match channels {
                1 => (px[0], px[0], px[0], 255),
                2 => (px[0], px[0], px[0], px[1]),
                3 => (px[0], px[1], px[2], 255),
                _ => (px[0], px[1], px[2], px[3]),
            };
            let o = i * 4;
            img.data[o] = color::srgb8_to_linear(r);
            img.data[o + 1] = color::srgb8_to_linear(g);
            img.data[o + 2] = color::srgb8_to_linear(b);
            img.data[o + 3] = a as f32 / 255.0;
        }
        img
    }
    pub fn has_alpha(&self) -> bool {
        self.data.chunks_exact(4).any(|p| p[3] < 1.0)
    }
    /// Stable 64-bit content hash of the exact float bits (FNV-1a over bytes).
    pub fn content_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |b: u8| {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for b in (self.w as u32).to_le_bytes().iter().chain((self.h as u32).to_le_bytes().iter()) {
            eat(*b);
        }
        for v in &self.data {
            for b in v.to_bits().to_le_bytes() {
                eat(b);
            }
        }
        h
    }
}

/// FNV-1a 64 over raw bytes (used for file / pixel fingerprints in reports).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_ops() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, -5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 0, 5, 5));
        assert_eq!(a.union(&b), Rect::new(0, -5, 15, 15));
        assert!(a.intersect(&Rect::new(20, 20, 5, 5)).is_empty());
        assert!(a.contains(9, 9) && !a.contains(10, 9));
    }

    #[test]
    fn crop_and_blit() {
        let mut img = Image::new(8, 6);
        for y in 0..6 {
            for x in 0..8 {
                img.set(x, y, [x as f32, y as f32, 0.0, 1.0]);
            }
        }
        let c = img.crop(Rect::new(6, 4, 4, 4));
        assert_eq!(c.get(0, 0), [6.0, 4.0, 0.0, 1.0]);
        assert_eq!(c.get(3, 3), [0.0; 4]);
        let mut dst = Image::new(8, 6);
        dst.blit(&c, -1, -1);
        assert_eq!(dst.get(0, 0), [7.0, 5.0, 0.0, 1.0]);
        assert_eq!(img.content_hash(), img.clone().content_hash());
        assert_ne!(img.content_hash(), dst.content_hash());
    }

    #[test]
    fn srgb8_roundtrip() {
        let bytes: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
        let img = Image::from_srgb8(16, 16, 4, &bytes);
        assert_eq!(img.to_srgb8(), bytes);
    }
}
