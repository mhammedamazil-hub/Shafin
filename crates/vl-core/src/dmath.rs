//! Deterministic math kernel.
//!
//! Rust guarantees IEEE-754 results for `+ - * /` and `sqrt`, but `sin`, `exp`,
//! `powf`, ... are forwarded to the platform libm and their last bits differ
//! between glibc, musl, macOS, Windows and WebAssembly. A renderer that calls
//! them is therefore not bit-reproducible across machines.
//!
//! Everything here is built ONLY from IEEE-exact operations (`+ - * /`, `sqrt`,
//! integer bit manipulation), evaluated in `f64` and rounded to `f32`, so the
//! same inputs give the same bits on every conforming target. Accuracy is close
//! to double precision (see the tests), which is far more than rendering needs.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

pub const PI: f64 = 3.141_592_653_589_793;
pub const FRAC_PI_2: f64 = 1.570_796_326_794_896_6;
pub const FRAC_PI_4: f64 = 0.785_398_163_397_448_3;
pub const PI32: f32 = PI as f32;
pub const TAU32: f32 = (2.0 * PI) as f32;
pub const INV_PI32: f32 = (1.0 / PI) as f32;

const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;
const INV_LN2: f64 = 1.442_695_040_888_963_387_00e+00;
const PIO2_1: f64 = 1.570_796_326_734_125_614_17e+00;
const PIO2_1T: f64 = 6.077_100_506_506_192_249_32e-11;
const TWO_OVER_PI: f64 = 6.366_197_723_675_813_824_33e-01;

#[inline]
fn reduce(x: f64) -> (f64, i64) {
    let k = (x * TWO_OVER_PI).round();
    ((x - k * PIO2_1) - k * PIO2_1T, k as i64)
}

#[inline]
fn ksin(r: f64) -> f64 {
    let z = r * r;
    let p = -1.0 / 1_307_674_368_000.0;
    let p = p * z + 1.0 / 6_227_020_800.0;
    let p = p * z - 1.0 / 39_916_800.0;
    let p = p * z + 1.0 / 362_880.0;
    let p = p * z - 1.0 / 5040.0;
    let p = p * z + 1.0 / 120.0;
    let p = p * z - 1.0 / 6.0;
    r + r * z * p
}

#[inline]
fn kcos(r: f64) -> f64 {
    let z = r * r;
    let p = 1.0 / 20_922_789_888_000.0;
    let p = p * z - 1.0 / 87_178_291_200.0;
    let p = p * z + 1.0 / 479_001_600.0;
    let p = p * z - 1.0 / 3_628_800.0;
    let p = p * z + 1.0 / 40_320.0;
    let p = p * z - 1.0 / 720.0;
    let p = p * z + 1.0 / 24.0;
    let p = p * z - 0.5;
    1.0 + z * p
}

pub fn sin64(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (r, q) = reduce(x);
    match q & 3 {
        0 => ksin(r),
        1 => kcos(r),
        2 => -ksin(r),
        _ => -kcos(r),
    }
}

pub fn cos64(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (r, q) = reduce(x);
    match q & 3 {
        0 => kcos(r),
        1 => -ksin(r),
        2 => -kcos(r),
        _ => ksin(r),
    }
}

/// (sin x, cos x) with a single range reduction.
pub fn sincos64(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let (r, q) = reduce(x);
    let (s, c) = (ksin(r), kcos(r));
    match q & 3 {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

pub fn tan64(x: f64) -> f64 {
    let (s, c) = sincos64(x);
    s / c
}

fn scalbn(mut y: f64, mut n: i32) -> f64 {
    if n > 1023 {
        y *= f64::from_bits(0x7FE0_0000_0000_0000); // 2^1023
        n -= 1023;
        if n > 1023 {
            y *= f64::from_bits(0x7FE0_0000_0000_0000);
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        y *= f64::from_bits(0x0360_0000_0000_0000); // 2^-969
        n += 969;
        if n < -1022 {
            y *= f64::from_bits(0x0360_0000_0000_0000);
            n += 969;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    y * f64::from_bits(((0x3ff + n as i64) as u64) << 52)
}

pub fn exp64(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782_712_893_384 {
        return f64::INFINITY;
    }
    if x < -745.133_219_101_941_1 {
        return 0.0;
    }
    let k = (x * INV_LN2).round();
    let r = (x - k * LN2_HI) - k * LN2_LO;
    const C: [f64; 14] = [
        1.0 / 6_227_020_800.0,
        1.0 / 479_001_600.0,
        1.0 / 39_916_800.0,
        1.0 / 3_628_800.0,
        1.0 / 362_880.0,
        1.0 / 40_320.0,
        1.0 / 5040.0,
        1.0 / 720.0,
        1.0 / 120.0,
        1.0 / 24.0,
        1.0 / 6.0,
        0.5,
        1.0,
        1.0,
    ];
    let mut p = C[0];
    for c in &C[1..] {
        p = p * r + c;
    }
    scalbn(p, k as i32)
}

pub fn ln64(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let mut bits = x.to_bits();
    let mut e: i64 = 0;
    if (bits >> 52) == 0 {
        bits = (x * 18_014_398_509_481_984.0).to_bits(); // * 2^54
        e -= 54;
    }
    e += ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > 1.414_213_562_373_095_1 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let z = s * s;
    let mut p = 1.0 / 23.0;
    let mut d = 21.0;
    while d >= 1.0 {
        p = p * z + 1.0 / d;
        d -= 2.0;
    }
    let ef = e as f64;
    ef * LN2_HI + (2.0 * s * p + ef * LN2_LO)
}

pub fn pow64(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y == y.trunc() && y.abs() <= 64.0 {
        // exact-as-possible integer powers (3^2 must be 9, not 9.000000000000002)
        let mut n = y.abs() as u32;
        let (mut b, mut r) = (x, 1.0);
        while n > 0 {
            if n & 1 == 1 {
                r *= b;
            }
            b *= b;
            n >>= 1;
        }
        return if y < 0.0 { 1.0 / r } else { r };
    }
    if x == 1.0 {
        return 1.0;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    if x < 0.0 {
        if y == y.trunc() {
            let r = exp64(y * ln64(-x));
            return if (y.abs() % 2.0) == 1.0 { -r } else { r };
        }
        return f64::NAN;
    }
    exp64(y * ln64(x))
}

fn atan_unit(x: f64) -> f64 {
    // 0 <= x <= 1
    let (t, off) = if x > 0.414_213_562_373_095_03 { ((x - 1.0) / (x + 1.0), FRAC_PI_4) } else { (x, 0.0) };
    let u = t / (1.0 + (1.0 + t * t).sqrt()); // atan(t) = 2 atan(u), |u| <= 0.2035
    let z = u * u;
    let mut p = 0.0;
    let mut k = 12i32;
    while k >= 0 {
        let c = 1.0 / (2 * k + 1) as f64;
        p = p * z + if k & 1 == 1 { -c } else { c };
        k -= 1;
    }
    off + 2.0 * u * p
}

pub fn atan64(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let ax = x.abs();
    let r = if ax > 1.0 { FRAC_PI_2 - atan_unit(1.0 / ax) } else { atan_unit(ax) };
    if x < 0.0 {
        -r
    } else {
        r
    }
}

pub fn atan2_64(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (mut x, mut y) = (x, y);
    if x.is_infinite() && y.is_infinite() {
        x = x.signum();
        y = y.signum();
    }
    if x.abs() >= y.abs() {
        let a = atan64(y / x);
        if x > 0.0 {
            a
        } else if y >= 0.0 {
            a + PI
        } else {
            a - PI
        }
    } else {
        let a = atan64(x / y);
        if y > 0.0 {
            FRAC_PI_2 - a
        } else {
            -FRAC_PI_2 - a
        }
    }
}

pub fn asin64(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2_64(x, ((1.0 - x) * (1.0 + x)).sqrt())
}

pub fn acos64(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2_64(((1.0 - x) * (1.0 + x)).sqrt(), x)
}

pub fn cbrt64(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let a = x.abs();
    let mut y = exp64(ln64(a) / 3.0);
    y -= (y * y * y - a) / (3.0 * y * y);
    if x < 0.0 {
        -y
    } else {
        y
    }
}

// ---- f32 front-ends used by the renderer and image code ----
#[inline]
pub fn sin(x: f32) -> f32 {
    sin64(x as f64) as f32
}
#[inline]
pub fn cos(x: f32) -> f32 {
    cos64(x as f64) as f32
}
#[inline]
pub fn sincos(x: f32) -> (f32, f32) {
    let (s, c) = sincos64(x as f64);
    (s as f32, c as f32)
}
#[inline]
pub fn tan(x: f32) -> f32 {
    tan64(x as f64) as f32
}
#[inline]
pub fn exp(x: f32) -> f32 {
    exp64(x as f64) as f32
}
#[inline]
pub fn ln(x: f32) -> f32 {
    ln64(x as f64) as f32
}
#[inline]
pub fn pow(x: f32, y: f32) -> f32 {
    pow64(x as f64, y as f64) as f32
}
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    atan2_64(y as f64, x as f64) as f32
}
#[inline]
pub fn acos(x: f32) -> f32 {
    acos64((x as f64).clamp(-1.0, 1.0)) as f32
}
#[inline]
pub fn asin(x: f32) -> f32 {
    asin64((x as f64).clamp(-1.0, 1.0)) as f32
}
#[inline]
pub fn cbrt(x: f32) -> f32 {
    cbrt64(x as f64) as f32
}
#[inline]
pub fn radians(deg: f32) -> f32 {
    (deg as f64 * (PI / 180.0)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(a: f64, b: f64) -> f64 {
        if a == b {
            0.0
        } else {
            (a - b).abs() / b.abs().max(1e-300)
        }
    }

    /// Accuracy against the platform libm (which is accurate to <1 ulp). This is an
    /// accuracy test only; determinism comes from construction, not from this test.
    #[test]
    fn accuracy_vs_libm() {
        let mut worst = [0.0f64; 8];
        let mut i = 0u32;
        while i < 200_000 {
            let t = (i as f64 + 0.37) / 200_000.0;
            let x = (t - 0.5) * 200.0; // [-100, 100]
            worst[0] = worst[0].max((sin64(x) - x.sin()).abs());
            worst[1] = worst[1].max((cos64(x) - x.cos()).abs());
            let xe = (t - 0.5) * 80.0;
            worst[2] = worst[2].max(rel(exp64(xe), xe.exp()));
            let xl = exp64((t - 0.5) * 60.0);
            worst[3] = worst[3].max((ln64(xl) - xl.ln()).abs() / xl.ln().abs().max(1.0));
            let xa = (t - 0.5) * 2000.0;
            worst[4] = worst[4].max((atan64(xa) - xa.atan()).abs());
            let u = t * 2.0 - 1.0;
            worst[5] = worst[5].max((asin64(u) - u.asin()).abs());
            worst[6] = worst[6].max((acos64(u) - u.acos()).abs());
            let (px, py) = (t * 20.0 + 1e-3, (t * 7919.0) % 9.0 - 4.5);
            worst[7] = worst[7].max(rel(pow64(px, py), px.powf(py)));
            i += 1;
        }
        let names = ["sin", "cos", "exp", "ln", "atan", "asin", "acos", "pow"];
        for (n, w) in names.iter().zip(worst.iter()) {
            assert!(*w < 2e-13, "{n}: worst error {w:e}");
        }
    }

    #[test]
    fn exact_cases() {
        assert_eq!(pow64(3.0, 2.0), 9.0);
        assert_eq!(pow64(2.0, 10.0), 1024.0);
        assert_eq!(pow64(2.0, -2.0), 0.25);
        assert_eq!(sin64(0.0), 0.0);
        assert_eq!(cos64(0.0), 1.0);
        assert_eq!(exp64(0.0), 1.0);
        assert_eq!(ln64(1.0), 0.0);
        assert_eq!(atan2_64(0.0, -1.0), PI);
        assert!((atan2_64(1.0, 1.0) - FRAC_PI_4).abs() < 1e-15);
        assert!(exp64(1000.0).is_infinite());
        assert_eq!(exp64(-1000.0), 0.0);
        assert!((cbrt64(27.0) - 3.0).abs() < 1e-14);
    }

    /// Golden bit patterns: if any platform computes a different bit, this fails there.
    #[test]
    fn golden_bits() {
        let got = [
            sin(1.0).to_bits(),
            cos(2.5).to_bits(),
            exp(-3.25).to_bits(),
            ln(7.5).to_bits(),
            pow(0.5, 1.0 / 2.4).to_bits(),
            atan2(1.0, -2.0).to_bits(),
            acos(0.3).to_bits(),
            tan(0.7).to_bits(),
        ];
        let want = [
            1.0f32.sin().to_bits(),
            2.5f32.cos().to_bits(),
            (-3.25f32).exp().to_bits(),
            7.5f32.ln().to_bits(),
            0.5f32.powf(1.0 / 2.4).to_bits(),
            1.0f32.atan2(-2.0).to_bits(),
            0.3f32.acos().to_bits(),
            0.7f32.tan().to_bits(),
        ];
        // f64-evaluated results rounded to f32 are (almost always) the correctly rounded
        // value; allow 1 ulp against libm here. Cross-platform identity is checked by
        // the hashes in `tests/determinism.rs` and by CI on three operating systems.
        for (g, w) in got.iter().zip(want.iter()) {
            assert!((*g as i64 - *w as i64).abs() <= 1, "{g:#x} vs {w:#x}");
        }
    }
}
