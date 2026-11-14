//! Small linear-algebra kit (f32) used by the compiler and the renderer.
use crate::dmath;
use std::ops::{Add, AddAssign, Div, Index, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[inline]
pub const fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = vec3(0.0, 0.0, 0.0);
    pub const ONE: Vec3 = vec3(1.0, 1.0, 1.0);
    pub const X: Vec3 = vec3(1.0, 0.0, 0.0);
    pub const Y: Vec3 = vec3(0.0, 1.0, 0.0);
    pub const Z: Vec3 = vec3(0.0, 0.0, 1.0);

    #[inline]
    pub const fn splat(v: f32) -> Vec3 {
        vec3(v, v, v)
    }
    #[inline]
    pub fn from_array(a: [f32; 3]) -> Vec3 {
        vec3(a[0], a[1], a[2])
    }
    #[inline]
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
    #[inline]
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: Vec3) -> Vec3 {
        vec3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }
    /// Normalised copy; returns +Y for the zero vector instead of NaNs.
    #[inline]
    pub fn normalize(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 {
            self / l
        } else {
            Vec3::Y
        }
    }
    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        vec3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        vec3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    #[inline]
    pub fn abs(self) -> Vec3 {
        vec3(self.x.abs(), self.y.abs(), self.z.abs())
    }
    #[inline]
    pub fn max_comp(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }
    #[inline]
    pub fn min_comp(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }
    #[inline]
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    #[inline]
    pub fn map(self, f: impl Fn(f32) -> f32) -> Vec3 {
        vec3(f(self.x), f(self.y), f(self.z))
    }
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    #[inline]
    pub fn sum(self) -> f32 {
        self.x + self.y + self.z
    }
    #[inline]
    pub fn clamp(self, lo: f32, hi: f32) -> Vec3 {
        self.map(|v| v.clamp(lo, hi))
    }
    /// Index of the largest absolute component.
    #[inline]
    pub fn max_dim(self) -> usize {
        let a = self.abs();
        if a.x >= a.y && a.x >= a.z {
            0
        } else if a.y >= a.z {
            1
        } else {
            2
        }
    }
}

impl Index<usize> for Vec3 {
    type Output = f32;
    #[inline]
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }
}
impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        vec3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        vec3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, s: f32) -> Vec3 {
        vec3(self.x * s, self.y * s, self.z * s)
    }
}
impl Mul<Vec3> for f32 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        v * self
    }
}
impl Mul<Vec3> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, o: Vec3) -> Vec3 {
        vec3(self.x * o.x, self.y * o.y, self.z * o.z)
    }
}
impl MulAssign<f32> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}
impl MulAssign<Vec3> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, o: Vec3) {
        *self = *self * o;
    }
}
impl Div<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, s: f32) -> Vec3 {
        let inv = 1.0 / s;
        vec3(self.x * inv, self.y * inv, self.z * inv)
    }
}
impl Div<Vec3> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, o: Vec3) -> Vec3 {
        vec3(self.x / o.x, self.y / o.y, self.z / o.z)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        vec3(-self.x, -self.y, -self.z)
    }
}

/// Branch-light orthonormal basis around a unit vector (Duff et al. 2017).
#[inline]
pub fn onb(n: Vec3) -> (Vec3, Vec3) {
    let sign = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (vec3(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x), vec3(b, sign + n.y * n.y * a, -n.y))
}

#[inline]
pub fn reflect(i: Vec3, n: Vec3) -> Vec3 {
    i - n * (2.0 * i.dot(n))
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e0 == e1 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub o: Vec3,
    pub d: Vec3,
}

impl Ray {
    #[inline]
    pub fn new(o: Vec3, d: Vec3) -> Ray {
        Ray { o, d }
    }
    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        self.o + self.d * t
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    pub fn new(a: Vec3, b: Vec3) -> Aabb {
        Aabb { min: a.min(b), max: a.max(b) }
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }
    #[inline]
    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    #[inline]
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    #[inline]
    pub fn surface_area(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let d = self.size();
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
    /// Overlap volume with another box (0 when disjoint).
    pub fn overlap(&self, o: &Aabb) -> Vec3 {
        (self.max.min(o.max) - self.min.max(o.min)).max(Vec3::ZERO)
    }
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        if self.is_empty() {
            return *self;
        }
        let mut out = Aabb::EMPTY;
        for i in 0..8 {
            let p = vec3(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            out.grow(m.point(p));
        }
        out
    }
    /// Slab test. `inv_d` is 1/direction (may contain infinities).
    #[inline]
    pub fn hit(&self, o: Vec3, inv_d: Vec3, tmin: f32, tmax: f32) -> bool {
        let t0 = (self.min - o) * inv_d;
        let t1 = (self.max - o) * inv_d;
        let lo = t0.min(t1);
        let hi = t0.max(t1);
        let a = lo.x.max(lo.y).max(lo.z).max(tmin);
        let b = hi.x.min(hi.y).min(hi.z).min(tmax);
        a <= b
    }
    /// Entry distance of the slab test, or `f32::INFINITY` on a miss.
    #[inline]
    pub fn hit_dist(&self, o: Vec3, inv_d: Vec3, tmin: f32, tmax: f32) -> f32 {
        let t0 = (self.min - o) * inv_d;
        let t1 = (self.max - o) * inv_d;
        let lo = t0.min(t1);
        let hi = t0.max(t1);
        let a = lo.x.max(lo.y).max(lo.z).max(tmin);
        let b = hi.x.min(hi.y).min(hi.z).min(tmax);
        if a <= b {
            a
        } else {
            f32::INFINITY
        }
    }
}

/// Row-major 4x4 affine matrix (the last row is always 0 0 0 1 in practice).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub const IDENTITY: Mat4 =
        Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    pub fn translate(t: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[0][3] = t.x;
        m.0[1][3] = t.y;
        m.0[2][3] = t.z;
        m
    }
    pub fn scale(s: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[0][0] = s.x;
        m.0[1][1] = s.y;
        m.0[2][2] = s.z;
        m
    }
    pub fn rotate_x(deg: f32) -> Mat4 {
        let (s, c) = dmath::sincos(dmath::radians(deg));
        Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, c, -s, 0.0], [0.0, s, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }
    pub fn rotate_y(deg: f32) -> Mat4 {
        let (s, c) = dmath::sincos(dmath::radians(deg));
        Mat4([[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }
    pub fn rotate_z(deg: f32) -> Mat4 {
        let (s, c) = dmath::sincos(dmath::radians(deg));
        Mat4([[c, -s, 0.0, 0.0], [s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }
    /// Euler rotation: about X, then Y, then Z (fixed world axes). Degrees.
    pub fn rotate_xyz(deg: Vec3) -> Mat4 {
        Mat4::rotate_z(deg.z).mul(&Mat4::rotate_y(deg.y)).mul(&Mat4::rotate_x(deg.x))
    }
    /// Matrix whose columns are the given basis vectors and origin.
    pub fn from_basis(x: Vec3, y: Vec3, z: Vec3, o: Vec3) -> Mat4 {
        Mat4([[x.x, y.x, z.x, o.x], [x.y, y.y, z.y, o.y], [x.z, y.z, z.z, o.z], [0.0, 0.0, 0.0, 1.0]])
    }
    /// Rotation that maps +Y onto `dir` (used for "axis:" / from-to shapes).
    pub fn align_y(dir: Vec3) -> Mat4 {
        let y = dir.normalize();
        let (t, b) = onb(y);
        // (t, y, b) must be right-handed: t x y = b  <=>  y x b = t
        let z = t.cross(y);
        let _ = b;
        Mat4::from_basis(t, y, z, Vec3::ZERO)
    }
    #[inline]
    pub fn mul(&self, o: &Mat4) -> Mat4 {
        let mut r = [[0.0f32; 4]; 4];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.0[i][0] * o.0[0][j] + self.0[i][1] * o.0[1][j] + self.0[i][2] * o.0[2][j] + self.0[i][3] * o.0[3][j];
            }
        }
        Mat4(r)
    }
    #[inline]
    pub fn point(&self, p: Vec3) -> Vec3 {
        let m = &self.0;
        vec3(
            m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
            m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
            m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
        )
    }
    #[inline]
    pub fn vector(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        vec3(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }
    /// Transform a normal with the transpose of THIS matrix (call on the inverse).
    #[inline]
    pub fn normal_t(&self, n: Vec3) -> Vec3 {
        let m = &self.0;
        vec3(
            m[0][0] * n.x + m[1][0] * n.y + m[2][0] * n.z,
            m[0][1] * n.x + m[1][1] * n.y + m[2][1] * n.z,
            m[0][2] * n.x + m[1][2] * n.y + m[2][2] * n.z,
        )
    }
    pub fn translation(&self) -> Vec3 {
        vec3(self.0[0][3], self.0[1][3], self.0[2][3])
    }
    /// Inverse of an affine matrix (upper 3x3 + translation). Returns None if singular.
    pub fn inverse(&self) -> Option<Mat4> {
        let m = &self.0;
        let (a, b, c) = (m[0][0] as f64, m[0][1] as f64, m[0][2] as f64);
        let (d, e, f) = (m[1][0] as f64, m[1][1] as f64, m[1][2] as f64);
        let (g, h, i) = (m[2][0] as f64, m[2][1] as f64, m[2][2] as f64);
        let (co0, co1, co2) = (e * i - f * h, f * g - d * i, d * h - e * g);
        let det = a * co0 + b * co1 + c * co2;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv = 1.0 / det;
        let r = [
            [co0 * inv, (c * h - b * i) * inv, (b * f - c * e) * inv],
            [co1 * inv, (a * i - c * g) * inv, (c * d - a * f) * inv],
            [co2 * inv, (b * g - a * h) * inv, (a * e - b * d) * inv],
        ];
        let (tx, ty, tz) = (m[0][3] as f64, m[1][3] as f64, m[2][3] as f64);
        let mut out = Mat4::IDENTITY;
        for k in 0..3 {
            out.0[k][0] = r[k][0] as f32;
            out.0[k][1] = r[k][1] as f32;
            out.0[k][2] = r[k][2] as f32;
            out.0[k][3] = (-(r[k][0] * tx + r[k][1] * ty + r[k][2] * tz)) as f32;
        }
        Some(out)
    }
    pub fn to_rows(&self) -> [f32; 12] {
        let m = &self.0;
        [m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1], m[2][2], m[2][3]]
    }
    pub fn from_rows(r: &[f32; 12]) -> Mat4 {
        Mat4([[r[0], r[1], r[2], r[3]], [r[4], r[5], r[6], r[7]], [r[8], r[9], r[10], r[11]], [0.0, 0.0, 0.0, 1.0]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrip() {
        let m = Mat4::translate(vec3(1.0, -2.0, 3.0)).mul(&Mat4::rotate_xyz(vec3(20.0, 35.0, -50.0))).mul(&Mat4::scale(vec3(2.0, 0.5, 3.0)));
        let inv = m.inverse().unwrap();
        let p = vec3(0.3, 0.7, -1.1);
        let q = inv.point(m.point(p));
        assert!((p - q).length() < 1e-5);
    }

    #[test]
    fn align_y_is_rotation() {
        for d in [vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0), vec3(1.0, 2.0, -3.0), vec3(-1.0, 0.0, 0.0)] {
            let m = Mat4::align_y(d);
            let y = m.vector(Vec3::Y);
            assert!((y - d.normalize()).length() < 1e-5);
            let (x, z) = (m.vector(Vec3::X), m.vector(Vec3::Z));
            assert!((x.cross(y) - z).length() < 1e-5, "right-handed");
            assert!((x.length() - 1.0).abs() < 1e-5 && (z.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn rotate_y_convention() {
        // +90 degrees about Y maps +Z to +X (right-handed, counter-clockwise seen from +Y).
        let v = Mat4::rotate_y(90.0).vector(Vec3::Z);
        assert!((v - Vec3::X).length() < 1e-6);
    }

    #[test]
    fn aabb_slab() {
        let b = Aabb::new(vec3(-1.0, -1.0, -1.0), vec3(1.0, 1.0, 1.0));
        let o = vec3(0.0, 0.0, 5.0);
        let d = vec3(0.0, 0.0, -1.0);
        let inv = vec3(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        assert!(b.hit(o, inv, 0.0, 100.0));
        assert!((b.hit_dist(o, inv, 0.0, 100.0) - 4.0).abs() < 1e-6);
        assert!(!b.hit(vec3(3.0, 0.0, 5.0), inv, 0.0, 100.0));
    }
}
