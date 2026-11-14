//! Physical sun and sky: single-scattering Rayleigh + Mie atmosphere.
//!
//! Chosen over fitted analytic models because it needs only a handful of
//! well-known physical constants (no coefficient tables to mis-transcribe) and
//! stays sensible at sunrise and sunset. Multiple scattering and ozone are
//! ignored, so the sky is somewhat darker than a measured one, most visibly at
//! high turbidity and low sun; see docs/architecture.md for the limits.
//!
//! Units: radiance in cd/m^2 for linear-sRGB channels. The top-of-atmosphere
//! solar illuminance is 128 000 lux; everything else follows from the physics,
//! so a sunlit scene comes out near 100 000 lux and needs about EV 15.
//!
//! The model is evaluated once into a small table per scene (`SkyTable`); the
//! per-ray cost is one bilinear lookup. All math goes through `vl_core::dmath`,
//! so the table is bit-identical on every platform.
use crate::scene::SkyParams;
use vl_core::dmath;
use vl_core::math::{vec3, Vec3};

const RE: f64 = 6_360_000.0;
const RA: f64 = 6_420_000.0;
const HR: f64 = 7_994.0;
const HM: f64 = 1_200.0;
const BETA_R: [f64; 3] = [5.8e-6, 13.5e-6, 33.1e-6];
const BETA_M0: f64 = 21.0e-6;
const MIE_G: f64 = 0.76;
const OBSERVER_ALTITUDE: f64 = 100.0;
/// Solar illuminance above the atmosphere, lux.
pub const E_TOA: f64 = 128_000.0;
/// Angular radius of the real sun, radians (0.2666 degrees).
pub const SUN_ANGULAR_RADIUS: f64 = 0.004_654;

const TABLE_W: usize = 256;
const TABLE_H: usize = 96;
const VIEW_STEPS: usize = 24;
const LIGHT_STEPS: usize = 8;

type V = [f64; 3];

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn along(p: V, d: V, t: f64) -> V {
    [p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t]
}

/// Distance from `p` (inside the atmosphere) along `d` to the top of the atmosphere.
fn exit_distance(p: V, d: V) -> f64 {
    let b = dot(p, d);
    let c = dot(p, p) - RA * RA;
    -b + (b * b - c).max(0.0).sqrt()
}

fn hits_planet(p: V, d: V) -> bool {
    let b = dot(p, d);
    let c = dot(p, p) - RE * RE;
    let disc = b * b - c;
    disc > 0.0 && -b - disc.sqrt() > 0.0
}

/// Rayleigh and Mie optical depth (density-weighted path length, metres) from `p` to space along `d`.
fn optical_depth(p: V, d: V, steps: usize) -> (f64, f64) {
    let len = exit_distance(p, d);
    let ds = len / steps as f64;
    let (mut r, mut m) = (0.0, 0.0);
    for i in 0..steps {
        let q = along(p, d, (i as f64 + 0.5) * ds);
        let h = (dot(q, q).sqrt() - RE).max(0.0);
        r += dmath::exp64(-h / HR) * ds;
        m += dmath::exp64(-h / HM) * ds;
    }
    (r, m)
}

fn mie_beta(turbidity: f32) -> f64 {
    BETA_M0 * ((turbidity as f64 - 1.0).max(0.0) / 2.0)
}

fn transmittance(od_r: f64, od_m: f64, beta_m: f64) -> V {
    let mut t = [0.0; 3];
    for c in 0..3 {
        t[c] = dmath::exp64(-(BETA_R[c] * od_r + 1.1 * beta_m * od_m));
    }
    t
}

/// Sky radiance (without the solar disc) seen from the ground in direction `d` (unit, +Y up).
pub fn sky_radiance_exact(d: Vec3, sun: Vec3, turbidity: f32) -> Vec3 {
    let beta_m = mie_beta(turbidity);
    let d: V = [d.x as f64, (d.y as f64).max(0.0), d.z as f64];
    let n = dot(d, d).sqrt();
    let d = [d[0] / n, d[1] / n, d[2] / n];
    let s: V = [sun.x as f64, sun.y as f64, sun.z as f64];
    let o: V = [0.0, RE + OBSERVER_ALTITUDE, 0.0];
    let len = exit_distance(o, d);
    let ds = len / VIEW_STEPS as f64;
    let (mut od_r, mut od_m) = (0.0, 0.0);
    let (mut sum_r, mut sum_m) = ([0.0f64; 3], [0.0f64; 3]);
    for i in 0..VIEW_STEPS {
        let p = along(o, d, (i as f64 + 0.5) * ds);
        let h = (dot(p, p).sqrt() - RE).max(0.0);
        let hr = dmath::exp64(-h / HR) * ds;
        let hm = dmath::exp64(-h / HM) * ds;
        od_r += hr;
        od_m += hm;
        if hits_planet(p, s) {
            continue; // this part of the atmosphere is in the planet's shadow
        }
        let (lr, lm) = optical_depth(p, s, LIGHT_STEPS);
        let t = transmittance(od_r + lr, od_m + lm, beta_m);
        for c in 0..3 {
            sum_r[c] += t[c] * hr;
            sum_m[c] += t[c] * hm;
        }
    }
    let mu = dot(d, s);
    let phase_r = 3.0 / (16.0 * dmath::PI) * (1.0 + mu * mu);
    let g = MIE_G;
    let denom = 1.0 + g * g - 2.0 * g * mu;
    let phase_m = 3.0 / (8.0 * dmath::PI) * ((1.0 - g * g) * (1.0 + mu * mu)) / ((2.0 + g * g) * denom * denom.sqrt());
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        out[c] = (E_TOA * (sum_r[c] * BETA_R[c] * phase_r + sum_m[c] * beta_m * phase_m)) as f32;
    }
    vec3(out[0], out[1], out[2])
}

/// Fraction of sunlight reaching the ground (per channel); zero when the sun is below the horizon.
pub fn sun_transmittance(sun: Vec3, turbidity: f32) -> Vec3 {
    if sun.y <= 0.0 {
        return Vec3::ZERO;
    }
    let o: V = [0.0, RE + OBSERVER_ALTITUDE, 0.0];
    let s: V = [sun.x as f64, sun.y as f64, sun.z as f64];
    let (r, m) = optical_depth(o, s, 64);
    let t = transmittance(r, m, mie_beta(turbidity));
    vec3(t[0] as f32, t[1] as f32, t[2] as f32)
}

/// Tabulated sky for one set of parameters.
#[derive(Clone, Debug)]
pub struct SkyTable {
    data: Vec<Vec3>,
    pub sun_dir: Vec3,
    /// Radiance of the solar disc, cd/m^2.
    pub sun_radiance: Vec3,
    /// Cosine of the disc's angular radius.
    pub sun_cos: f32,
    pub sun_solid_angle: f32,
    /// Illuminance on a surface facing the sun, lux (per channel).
    pub sun_illuminance: Vec3,
    /// Illuminance from the sky dome on a horizontal surface, lux (per channel).
    pub sky_illuminance: Vec3,
    /// Radiance of the ground below the horizon.
    pub ground: Vec3,
}

const HALF_PI: f32 = dmath::FRAC_PI_2 as f32;

impl SkyTable {
    pub fn build(p: &SkyParams) -> SkyTable {
        let sun = p.sun_dir.normalize();
        let rows: Vec<Vec<Vec3>> = vl_core::par::par_map(TABLE_H, vl_core::par::available_threads(), |j| {
            let t = j as f32 / (TABLE_H - 1) as f32;
            let elev = HALF_PI * t * t;
            let (se, ce) = dmath::sincos(elev);
            (0..TABLE_W)
                .map(|i| {
                    let phi = dmath::TAU32 * i as f32 / TABLE_W as f32;
                    let (sp, cp) = dmath::sincos(phi);
                    sky_radiance_exact(vec3(ce * cp, se, ce * sp), sun, p.turbidity) * p.sky_strength
                })
                .collect()
        });
        let data: Vec<Vec3> = rows.into_iter().flatten().collect();

        // horizontal illuminance of the dome: E = integral of L cos(zenith) d(omega)
        let mut sky_e = Vec3::ZERO;
        for j in 0..TABLE_H - 1 {
            let t0 = j as f32 / (TABLE_H - 1) as f32;
            let t1 = (j + 1) as f32 / (TABLE_H - 1) as f32;
            let (e0, e1) = (HALF_PI * t0 * t0, HALF_PI * t1 * t1);
            let em = 0.5 * (e0 + e1);
            let (se, ce) = dmath::sincos(em);
            let dw = ce * (e1 - e0) * (dmath::TAU32 / TABLE_W as f32);
            for i in 0..TABLE_W {
                let l = (data[j * TABLE_W + i] + data[(j + 1) * TABLE_W + i]) * 0.5;
                sky_e += l * (se * dw);
            }
        }

        let radius = (SUN_ANGULAR_RADIUS as f32 * p.sun_size.max(0.05)).min(0.5);
        let sun_cos = dmath::cos(radius);
        let solid = dmath::TAU32 * (1.0 - sun_cos);
        let sun_e = sun_transmittance(sun, p.turbidity) * (E_TOA as f32 * p.sun_strength);
        let sun_radiance = sun_e * (1.0 / solid);
        let horizontal = sun_e * sun.y.max(0.0) + sky_e;
        let ground = p.ground_albedo * horizontal * dmath::INV_PI32;
        SkyTable { data, sun_dir: sun, sun_radiance, sun_cos, sun_solid_angle: solid, sun_illuminance: sun_e, sky_illuminance: sky_e, ground }
    }

    /// Sky radiance without the solar disc.
    pub fn sky(&self, d: Vec3) -> Vec3 {
        let y = d.y.clamp(-1.0, 1.0);
        let elev = dmath::asin(y.max(0.0));
        let t = (elev / HALF_PI).max(0.0).sqrt().min(1.0);
        let mut phi = dmath::atan2(d.z, d.x);
        if phi < 0.0 {
            phi += dmath::TAU32;
        }
        let fx = phi / dmath::TAU32 * TABLE_W as f32;
        let fy = t * (TABLE_H - 1) as f32;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let xi0 = (x0 as usize) % TABLE_W;
        let xi1 = (xi0 + 1) % TABLE_W;
        let yi0 = (y0 as usize).min(TABLE_H - 1);
        let yi1 = (yi0 + 1).min(TABLE_H - 1);
        let at = |x: usize, y: usize| self.data[y * TABLE_W + x];
        let top = at(xi0, yi0) * (1.0 - ax) + at(xi1, yi0) * ax;
        let bot = at(xi0, yi1) * (1.0 - ax) + at(xi1, yi1) * ax;
        let sky = top * (1.0 - ay) + bot * ay;
        if y >= 0.0 {
            sky
        } else {
            // blend the horizon into the ground over the first ~3 degrees below it
            let k = vl_core::math::smoothstep(0.0, 0.05, -y);
            sky * (1.0 - k) + self.ground * k
        }
    }

    /// Is `d` inside the solar disc?
    #[inline]
    pub fn in_sun(&self, d: Vec3) -> bool {
        self.sun_radiance.max_comp() > 0.0 && d.dot(self.sun_dir) >= self.sun_cos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::color::luminance;

    fn params(elev_deg: f32, turbidity: f32) -> SkyParams {
        let e = dmath::radians(elev_deg);
        SkyParams { sun_dir: vec3(dmath::cos(e), dmath::sin(e), 0.0), turbidity, ..SkyParams::default() }
    }

    #[test]
    fn midday_sky_has_physical_magnitudes_and_colours() {
        let t = SkyTable::build(&params(60.0, 3.0));
        let sun_lux = luminance(t.sun_illuminance);
        let sky_lux = luminance(t.sky_illuminance);
        let zenith = t.sky(Vec3::Y);
        println!("midday: sun {sun_lux:.0} lx, sky {sky_lux:.0} lx, zenith {:.0} nits rgb {:?}, sun disc {:.3e} nits", luminance(zenith), zenith, luminance(t.sun_radiance));
        // measured clear-sky daylight: direct sun 80-120 klx, clear sky dome 8-25 klx, zenith 1-8 knits
        assert!((70_000.0..125_000.0).contains(&sun_lux), "sun illuminance {sun_lux}");
        assert!((4_000.0..30_000.0).contains(&sky_lux), "sky illuminance {sky_lux}");
        assert!((500.0..10_000.0).contains(&luminance(zenith)), "zenith luminance");
        assert!(zenith.z > zenith.y && zenith.y > zenith.x, "zenith must be blue: {zenith:?}");
        // the sky brightens towards the horizon away from the sun
        assert!(luminance(t.sky(vec3(-0.999, 0.04, 0.0).normalize())) > luminance(zenith));
        // disc radiance is of the order of 1e9 nits
        assert!((5.0e8..3.0e9).contains(&luminance(t.sun_radiance)));
    }

    #[test]
    fn sunset_is_warm_and_night_has_no_sun() {
        let t = SkyTable::build(&params(2.0, 3.0));
        let s = t.sun_illuminance;
        println!("sunset: sun rgb {s:?}, sky {:.0} lx", luminance(t.sky_illuminance));
        assert!(s.x > 1.5 * s.y && s.y > 1.5 * s.z, "low sun must be red-orange: {s:?}");
        assert!(luminance(s) < 40_000.0);
        // sky next to the sun is warmer (red/blue ratio) than the sky opposite
        let near = t.sky(vec3(0.99, 0.05, 0.1).normalize());
        let far = t.sky(vec3(-0.5, 0.8, 0.0).normalize());
        assert!(near.x / near.z > far.x / far.z);

        let night = SkyTable::build(&params(-10.0, 3.0));
        assert_eq!(night.sun_radiance, Vec3::ZERO);
        assert!(!night.in_sun(night.sun_dir));
        assert!(luminance(night.sky_illuminance) < 0.05 * luminance(t.sky_illuminance) + 50.0);
    }

    #[test]
    fn table_matches_exact_evaluation_and_is_continuous() {
        let p = params(35.0, 4.0);
        let t = SkyTable::build(&p);
        let mut worst = 0.0f32;
        for k in 0..200 {
            let a = k as f32 * 0.731;
            let e = 0.02 + 1.5 * ((k * 37 % 100) as f32 / 100.0);
            let d = vec3(dmath::cos(e) * dmath::cos(a), dmath::sin(e), dmath::cos(e) * dmath::sin(a));
            // stay away from the Mie peak around the sun, where a 256x96 table is knowingly coarse
            if d.dot(p.sun_dir) > 0.97 {
                continue;
            }
            let exact = sky_radiance_exact(d, p.sun_dir, p.turbidity);
            let got = t.sky(d);
            worst = worst.max((luminance(got) - luminance(exact)).abs() / luminance(exact));
        }
        println!("table vs exact, worst relative luminance error {worst:.4}");
        assert!(worst < 0.03, "table interpolation error {worst}");
        // no seam at azimuth 0 / 2 pi
        let a = t.sky(vec3(1.0, 0.3, 1e-5).normalize());
        let b = t.sky(vec3(1.0, 0.3, -1e-5).normalize());
        assert!((luminance(a) - luminance(b)).abs() / luminance(a) < 1e-3);
        // ground below the horizon is finite and positive
        assert!(t.sky(vec3(0.0, -1.0, 0.0)).min_comp() > 0.0);
    }
}
