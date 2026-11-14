//! Deterministic procedural noise. Integer hashing + IEEE-exact arithmetic only
//! (no tables, no libm), so every backend and platform sees the same texture.
use vl_core::math::{vec3, Vec3};

#[inline]
fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

#[inline]
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Three independent uniforms in [0,1) for an integer lattice cell.
#[inline]
pub fn cell_random(x: i32, y: i32, z: i32, seed: u32) -> Vec3 {
    let h = hash3(x, y, z, seed);
    vec3(unit(h), unit(h.wrapping_mul(0x9e37_79b9) ^ 0x68bc_21eb), unit(h.wrapping_mul(0x85eb_ca6b) ^ 0x02e5_be93))
}

#[inline]
fn grad(h: u32, x: f32, y: f32, z: f32) -> f32 {
    // 12 edge directions of the cube (Perlin 2002)
    match h & 15 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        11 => -y - z,
        12 => x + y,
        13 => -y + z,
        14 => -x + y,
        _ => -y - z,
    }
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Signed gradient noise, roughly in [-1, 1].
pub fn snoise(p: Vec3, seed: u32) -> f32 {
    // keep lattice coordinates in a range where f32 -> i32 is exact and safe
    let p = vec3(p.x.clamp(-1.0e6, 1.0e6), p.y.clamp(-1.0e6, 1.0e6), p.z.clamp(-1.0e6, 1.0e6));
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (x, y, z) = (p.x - fx, p.y - fy, p.z - fz);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let g = |dx: i32, dy: i32, dz: i32| grad(hash3(ix + dx, iy + dy, iz + dz, seed), x - dx as f32, y - dy as f32, z - dz as f32);
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Gradient noise remapped to [0, 1].
#[inline]
pub fn noise(p: Vec3, seed: u32) -> f32 {
    (snoise(p, seed) * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Fractal Brownian motion in [0, 1].
pub fn fbm(p: Vec3, octaves: u32, lacunarity: f32, gain: f32, seed: u32) -> f32 {
    let octaves = octaves.clamp(1, 12);
    let (mut sum, mut amp, mut norm, mut f) = (0.0f32, 1.0f32, 0.0f32, 1.0f32);
    for o in 0..octaves {
        sum += amp * snoise(p * f, seed.wrapping_add(o * 131));
        norm += amp;
        amp *= gain;
        f *= lacunarity;
    }
    (sum / norm * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Turbulence (sum of |noise|) in [0, 1].
pub fn turbulence(p: Vec3, octaves: u32, lacunarity: f32, gain: f32, seed: u32) -> f32 {
    let octaves = octaves.clamp(1, 12);
    let (mut sum, mut amp, mut norm, mut f) = (0.0f32, 1.0f32, 0.0f32, 1.0f32);
    for o in 0..octaves {
        sum += amp * snoise(p * f, seed.wrapping_add(o * 131)).abs();
        norm += amp;
        amp *= gain;
        f *= lacunarity;
    }
    (sum / norm).clamp(0.0, 1.0)
}

/// Worley / Voronoi noise: (F1 distance, F2 distance, random id of the nearest cell in [0,1)).
pub fn voronoi(p: Vec3, seed: u32) -> Vec3 {
    let p = vec3(p.x.clamp(-1.0e6, 1.0e6), p.y.clamp(-1.0e6, 1.0e6), p.z.clamp(-1.0e6, 1.0e6));
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0.0f32);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy, cz) = (ix + dx, iy + dy, iz + dz);
                let r = cell_random(cx, cy, cz, seed);
                let q = vec3(cx as f32 + r.x, cy as f32 + r.y, cz as f32 + r.z);
                let d = (q - p).length_sq();
                if d < f1 {
                    f2 = f1;
                    f1 = d;
                    id = unit(hash3(cx, cy, cz, seed ^ 0x5bd1_e995));
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
    }
    vec3(f1.sqrt(), f2.sqrt(), id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_and_statistics() {
        let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0f64);
        let n = 40_000;
        for i in 0..n {
            let p = vec3(i as f32 * 0.137, (i % 311) as f32 * 0.291, (i % 97) as f32 * 0.713);
            let v = snoise(p, 1);
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v as f64;
            let f = fbm(p, 5, 2.0, 0.5, 1);
            assert!((0.0..=1.0).contains(&f));
            let w = voronoi(p, 3);
            assert!(w.x >= 0.0 && w.x <= w.y && w.y < 2.5 && (0.0..1.0).contains(&w.z));
        }
        assert!(lo < -0.6 && hi > 0.6 && lo >= -1.01 && hi <= 1.01, "range {lo}..{hi}");
        assert!((sum / n as f64).abs() < 0.03);
    }

    #[test]
    fn continuity_and_lattice_zeros() {
        // gradient noise is zero on lattice points and continuous in between
        assert_eq!(snoise(vec3(3.0, -7.0, 12.0), 5), 0.0);
        let a = snoise(vec3(0.5, 0.25, 0.75), 2);
        let b = snoise(vec3(0.5005, 0.25, 0.75), 2);
        assert!((a - b).abs() < 0.01);
        // different seeds decorrelate
        assert!((snoise(vec3(0.4, 0.3, 0.2), 1) - snoise(vec3(0.4, 0.3, 0.2), 2)).abs() > 1e-4);
    }

    #[test]
    fn pinned_bit_patterns() {
        // Regression + cross-platform check. These bit patterns were captured from this
        // implementation on x86_64 Linux (they are not an independent reference); noise uses
        // only integer hashing and IEEE-exact + - * / sqrt, so any other IEEE-754 platform
        // must reproduce them exactly. A change here means every procedural texture changed.
        let p = vec3(1.37, -2.91, 0.55);
        let got = [snoise(p, 7).to_bits(), fbm(p, 4, 2.0, 0.5, 7).to_bits(), turbulence(p, 3, 2.0, 0.5, 7).to_bits(), voronoi(p, 7).x.to_bits(), voronoi(p, 7).z.to_bits(), cell_random(3, -4, 5, 7).y.to_bits()];
        assert_eq!(got, [3171947232, 1056505622, 1021566513, 1046803204, 1054806830, 1064892695]);
    }
}
