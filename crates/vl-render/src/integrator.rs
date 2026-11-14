//! Unidirectional path tracing with next-event estimation and multiple
//! importance sampling.
//!
//! Known, deliberate approximations (documented for users in docs/architecture.md):
//! * light reaching a surface THROUGH glass is computed with straight, attenuated
//!   shadow rays ("transparent shadows"); refractive caustics are not simulated;
//! * light reaching a diffuse surface by MIRROR reflection off another object is dropped
//!   unless `Film::reflective_caustics` is set (it would appear as speckles, not a caustic);
//! * after a path has scattered diffusely, glossy and mirror surfaces it meets later are
//!   treated as rougher (path regularisation), so reflections seen indirectly are blurrier
//!   than reality but can be lit by direct light sampling instead of by luck;
//! * indirect contributions are clamped (`Film::clamp_indirect`) to suppress fireflies,
//!   which removes a small amount of energy from very bright indirect paths;
//! * the sky dome is found by BSDF sampling only; the sun and emissive spheres,
//!   rectangles and disks are sampled directly.
use crate::rt::{ray_eps, LightShape, SceneRt, Scratch};
use vl_core::color::luminance;
use vl_core::dmath;
use vl_core::math::{onb, Vec3};
use vl_core::par;
use vl_core::rng::{hash3, Rng};
use vl_ir::scene::Shape;

pub struct LightSample {
    pub wi: Vec3,
    pub dist: f32,
    pub le: Vec3,
    /// Solid-angle density including the probability of choosing this light.
    pub pdf: f32,
    /// The emitting object (`u32::MAX` for the sun); excluded from the shadow test.
    pub obj: u32,
}

#[inline]
fn power_heuristic(a: f32, b: f32) -> f32 {
    let (a2, b2) = (a * a, b * b);
    if a2 + b2 > 0.0 {
        a2 / (a2 + b2)
    } else {
        0.0
    }
}

impl LightShape {
    fn center_and_radius2(&self) -> (Vec3, f32) {
        match *self {
            LightShape::Sphere { center, radius } => (center, radius * radius),
            LightShape::Quad { center, area, .. } | LightShape::Disk { center, area, .. } => (center, area * dmath::INV_PI32),
        }
    }

    /// Sample a direction from `p` towards the light: (wi, distance, solid-angle pdf).
    fn sample(&self, p: Vec3, u1: f32, u2: f32) -> Option<(Vec3, f32, f32)> {
        match *self {
            LightShape::Sphere { center, radius } => {
                let dc = center - p;
                let d2 = dc.length_sq();
                let r2 = radius * radius;
                if d2 <= r2 * 1.0001 {
                    return None;
                }
                let sin2_max = r2 / d2;
                let cos_max = (1.0 - sin2_max).max(0.0).sqrt();
                let omc = sin2_max / (1.0 + cos_max); // 1 - cos_max without cancellation
                let k = u1 * omc; // 1 - cos(theta)
                let cos_t = 1.0 - k;
                let sin2_t = k * (2.0 - k);
                let (sp, cp) = dmath::sincos(dmath::TAU32 * u2);
                let axis = dc * (1.0 / d2.sqrt());
                let (t1, t2) = onb(axis);
                let st = sin2_t.max(0.0).sqrt();
                let wi = (t1 * (st * cp) + t2 * (st * sp) + axis * cos_t).normalize();
                let dist = d2.sqrt() * cos_t - (r2 - d2 * sin2_t).max(0.0).sqrt();
                Some((wi, dist, 1.0 / (dmath::TAU32 * omc)))
            }
            LightShape::Quad { center, ex, ez, n, area } => {
                let q = center + ex * (2.0 * u1 - 1.0) + ez * (2.0 * u2 - 1.0);
                Self::towards(p, q, n, area)
            }
            LightShape::Disk { center, ex, ez, n, area } => {
                let r = u1.sqrt();
                let (s, c) = dmath::sincos(dmath::TAU32 * u2);
                let q = center + ex * (r * c) + ez * (r * s);
                Self::towards(p, q, n, area)
            }
        }
    }

    fn towards(p: Vec3, q: Vec3, n: Vec3, area: f32) -> Option<(Vec3, f32, f32)> {
        let v = q - p;
        let d2 = v.length_sq();
        if d2 <= 1e-12 {
            return None;
        }
        let dist = d2.sqrt();
        let wi = v * (1.0 / dist);
        let cos_l = n.dot(wi).abs();
        if cos_l < 1e-6 {
            return None;
        }
        Some((wi, dist, d2 / (area * cos_l)))
    }

    /// Solid-angle density with which `sample` would have produced a hit at distance `dist`
    /// along `wi` on a surface with normal `n_hit`.
    fn pdf(&self, p: Vec3, wi: Vec3, dist: f32, n_hit: Vec3) -> f32 {
        match *self {
            LightShape::Sphere { center, radius } => {
                let d2 = (center - p).length_sq();
                let r2 = radius * radius;
                if d2 <= r2 * 1.0001 {
                    return 0.0;
                }
                let sin2_max = r2 / d2;
                let omc = sin2_max / (1.0 + (1.0 - sin2_max).max(0.0).sqrt());
                1.0 / (dmath::TAU32 * omc)
            }
            LightShape::Quad { area, .. } | LightShape::Disk { area, .. } => {
                let cos_l = n_hit.dot(wi).abs();
                if cos_l < 1e-6 {
                    0.0
                } else {
                    dist * dist / (area * cos_l)
                }
            }
        }
    }
}

impl SceneRt<'_> {
    fn sun_weight(&self) -> f32 {
        self.sky.as_ref().map(|s| luminance(s.sun_illuminance)).unwrap_or(0.0)
    }

    #[inline]
    fn light_weight(&self, i: usize, p: Vec3) -> f32 {
        let l = &self.lights[i];
        let (c, r2) = l.shape.center_and_radius2();
        l.power / (c - p).length_sq().max(r2).max(1e-12)
    }

    fn total_light_weight(&self, p: Vec3) -> f32 {
        let mut t = self.sun_weight();
        for i in 0..self.lights.len() {
            t += self.light_weight(i, p);
        }
        t
    }

    /// Choose one light for the point `p` and sample a direction towards it.
    pub fn sample_light(&self, p: Vec3, u: [f32; 3], scratch: &mut Scratch) -> Option<LightSample> {
        let sun_w = self.sun_weight();
        let total = self.total_light_weight(p);
        if !(total > 0.0) {
            return None;
        }
        let mut x = u[0] * total;
        if x < sun_w {
            let sky = self.sky.as_ref()?;
            let omc = sky.sun_solid_angle / dmath::TAU32;
            let k = u[1] * omc;
            let st = (k * (2.0 - k)).max(0.0).sqrt();
            let (sp, cp) = dmath::sincos(dmath::TAU32 * u[2]);
            let (t1, t2) = onb(sky.sun_dir);
            let wi = (t1 * (st * cp) + t2 * (st * sp) + sky.sun_dir * (1.0 - k)).normalize();
            return Some(LightSample { wi, dist: f32::MAX, le: sky.sun_radiance, pdf: (sun_w / total) / sky.sun_solid_angle, obj: u32::MAX });
        }
        x -= sun_w;
        let n = self.lights.len();
        for i in 0..n {
            let w = self.light_weight(i, p);
            if x < w || i + 1 == n {
                let (wi, dist, pdf) = self.lights[i].shape.sample(p, u[1], u[2])?;
                let le = self.emission_at(self.lights[i].obj, p + wi * dist, scratch);
                return Some(LightSample { wi, dist, le, pdf: pdf * (w / total), obj: self.lights[i].obj });
            }
            x -= w;
        }
        None
    }

    /// Density with which `sample_light` at `p` would have produced direction `wi` hitting light `light`.
    fn light_pdf(&self, light: usize, p: Vec3, wi: Vec3, dist: f32, n_hit: Vec3) -> f32 {
        let total = self.total_light_weight(p);
        if !(total > 0.0) {
            return 0.0;
        }
        self.lights[light].shape.pdf(p, wi, dist, n_hit) * self.light_weight(light, p) / total
    }

    fn sun_pdf(&self, p: Vec3) -> f32 {
        let total = self.total_light_weight(p);
        match &self.sky {
            Some(s) if total > 0.0 => (self.sun_weight() / total) / s.sun_solid_angle,
            _ => 0.0,
        }
    }
}

pub struct PathResult {
    pub l: Vec3,
    pub albedo: Vec3,
    pub normal: Vec3,
    pub depth: f32,
    pub rays: u32,
}

/// Radiance arriving at `o` from direction `d` (unit vector pointing away from `o`).
/// Largest GGX alpha that path regularisation will impose (roughness 0.5). Measured on the
/// development scene at 64 spp: a floor of 0.09 left 701 speckle pixels, 0.25 leaves 64, with
/// the mean image energy unchanged to 0.05%.
const REGULARISE_MAX_ALPHA: f32 = 0.25;

pub fn radiance(rt: &SceneRt, mut o: Vec3, mut d: Vec3, rng: &mut Rng, scratch: &mut Scratch, max_depth: u32, clamp: f32) -> PathResult {
    let mut out = PathResult { l: Vec3::ZERO, albedo: Vec3::ONE, normal: Vec3::ZERO, depth: 0.0, rays: 0 };
    let caustics = rt.scene.film.reflective_caustics;
    let mut beta = Vec3::ONE;
    let mut prev_delta = true;
    let mut prev_pdf = 0.0f32;
    let mut prev_p = o;
    // Has the path scattered non-specularly yet? Since then, has it passed through clear glass?
    let mut diffuse_seen = false;
    let mut via_transmission = false;
    // Angular spread (radians, capped at 1) of the widest non-specular bounce so far.
    let mut blur = 0.0f32;

    for depth in 0..max_depth {
        out.rays += 1;
        let hit = rt.closest(o, d, 0.0, f32::MAX, depth == 0);
        let add = |l: &mut Vec3, c: Vec3| {
            if !c.is_finite() {
                return;
            }
            if clamp > 0.0 && diffuse_seen {
                let lum = luminance(c);
                if lum > clamp {
                    *l += c * (clamp / lum);
                    return;
                }
            }
            *l += c;
        };
        let hit = match hit {
            Some(h) => h,
            None => {
                add(&mut out.l, beta * rt.env(d));
                if let Some(sky) = &rt.sky {
                    // the sun is sampled as a light; a path that reached it through glass after a
                    // diffuse bounce is already covered by the transparent-shadow estimate
                    if sky.in_sun(d) && !(diffuse_seen && (via_transmission || (prev_delta && !caustics))) {
                        let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, rt.sun_pdf(prev_p)) };
                        add(&mut out.l, beta * sky.sun_radiance * w);
                    }
                }
                break;
            }
        };
        let mut s = rt.shade(o, d, &hit, scratch);
        if depth == 0 {
            out.albedo = s.bsdf.base;
            out.normal = s.ns;
            out.depth = hit.t;
        }
        if blur > 0.0 && s.bsdf.trans < 0.5 {
            // path regularisation: a surface reached after a blurry bounce is made at least
            // proportionally rough, so a mirror seen from a diffuse surface can be lit by
            // direct light sampling (a blurred caustic) instead of by chance (fireflies)
            let floor = (0.3 * blur).min(REGULARISE_MAX_ALPHA);
            if s.bsdf.spec_delta || s.bsdf.alpha < floor {
                s.bsdf.alpha = s.bsdf.alpha.max(floor);
                s.bsdf.spec_delta = false;
            }
            if s.bsdf.coat > 0.0 && (s.bsdf.coat_delta || s.bsdf.coat_alpha < floor) {
                s.bsdf.coat_alpha = s.bsdf.coat_alpha.max(floor);
                s.bsdf.coat_delta = false;
            }
        }
        if !s.front {
            beta = beta * rt.absorption(s.material, hit.t);
        }
        if s.emission.max_comp() > 0.0 {
            let inst = &rt.insts[hit.obj as usize];
            let flat = matches!(inst.shape, Shape::Rect { .. } | Shape::Disk { .. } | Shape::Plane);
            if s.front || flat {
                if inst.light >= 0 {
                    if !(diffuse_seen && (via_transmission || (prev_delta && !caustics))) {
                        let w = if prev_delta { 1.0 } else { power_heuristic(prev_pdf, rt.light_pdf(inst.light as usize, prev_p, d, hit.t, s.ng)) };
                        add(&mut out.l, beta * s.emission * w);
                    }
                } else {
                    add(&mut out.l, beta * s.emission);
                }
            }
        }

        let wo = s.to_local(-d);
        if wo.z <= 0.0 {
            break;
        }
        let eps = ray_eps(s.p);

        if s.bsdf.has_non_delta() {
            if let Some(ls) = rt.sample_light(s.p, [rng.f32(), rng.f32(), rng.f32()], scratch) {
                let wi_l = s.to_local(ls.wi);
                let (f, pdf_b) = s.bsdf.eval(wo, wi_l);
                // shading and geometric normals must agree about the side, otherwise light leaks
                let geo_side = ls.wi.dot(s.ng) > 0.0;
                if f.max_comp() > 0.0 && ls.pdf > 0.0 && geo_side == (wi_l.z > 0.0) {
                    let so = s.p + s.ng * if geo_side { eps } else { -eps };
                    // aim the shadow segment from the offset origin at the sampled point itself
                    let (sd, tmax) = if ls.dist == f32::MAX {
                        (ls.wi, f32::MAX)
                    } else {
                        let v = s.p + ls.wi * ls.dist - so;
                        let len = v.length();
                        (v * (1.0 / len.max(1e-20)), len)
                    };
                    out.rays += 1;
                    let tr = rt.transmittance(so, sd, 0.0, tmax, ls.obj, scratch);
                    if tr.max_comp() > 0.0 {
                        let w = power_heuristic(ls.pdf, pdf_b);
                        // direct light on the first diffuse surface is never clamped
                        let c = beta * f * ls.le * tr * (w / ls.pdf);
                        if diffuse_seen {
                            add(&mut out.l, c);
                        } else if c.is_finite() {
                            out.l += c;
                        }
                    }
                }
            }
        }

        let smp = match s.bsdf.sample(wo, [rng.f32(), rng.f32(), rng.f32(), rng.f32()]) {
            Some(x) => x,
            None => break,
        };
        let wi = s.to_world(smp.wi).normalize();
        let geo_out = wi.dot(s.ng) > 0.0;
        if geo_out == smp.transmitted {
            break; // sampled through the shading normal but not the real surface (or vice versa)
        }
        beta = beta * smp.weight;
        if !(beta.max_comp() > 0.0) || !beta.is_finite() {
            break;
        }
        if smp.delta {
            if smp.transmitted {
                via_transmission = true;
            }
        } else {
            diffuse_seen = true;
            via_transmission = false;
            // a lobe of density p covers about 1/p steradians: angular radius ~ sqrt(1 / (pi p))
            blur = blur.max((dmath::INV_PI32 / smp.pdf.max(1e-6)).sqrt().min(1.0));
        }
        prev_delta = smp.delta;
        prev_pdf = smp.pdf;
        prev_p = s.p;
        o = s.p + s.ng * if geo_out { eps } else { -eps };
        d = wi;

        // Russian roulette only once the path has lost energy. Paths at full throughput
        // (mirror and glass chains) are never killed at random - that would add noise exactly
        // where many bounces are needed - and are ended by `max_depth` instead.
        if depth >= 3 {
            let q = beta.max_comp();
            if q < 1.0 {
                let q = q.max(0.05);
                if rng.f32() >= q {
                    break;
                }
                beta = beta * (1.0 / q);
            }
        }
    }
    out
}

/// Region of the film to render: x0, y0 (inclusive) to x1, y1 (exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

/// Overrides for the film settings stored in the scene (used for previews and pre-passes).
#[derive(Clone, Copy, Debug, Default)]
pub struct FilmOverride {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub spp: Option<u32>,
    pub max_depth: Option<u32>,
}

pub struct RenderOpts<'a> {
    pub threads: usize,
    pub region: Option<Region>,
    /// Luminance limit (nits) for indirect contributions; 0 disables clamping.
    pub clamp: f32,
    pub film: FilmOverride,
    pub progress: Option<&'a (dyn Fn(f32) + Sync)>,
}

impl Default for RenderOpts<'_> {
    fn default() -> Self {
        RenderOpts { threads: par::available_threads(), region: None, clamp: 0.0, film: FilmOverride::default(), progress: None }
    }
}

/// Film settings in effect for one render call.
#[derive(Clone, Copy, Debug)]
struct FilmRt {
    w: usize,
    h: usize,
    spp: u32,
    min_spp: u32,
    noise_target: f32,
    max_depth: u32,
    seed: u64,
}

/// Scene-referred render result (radiance in nits) plus the feature buffers used
/// for denoising and inspection.
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub region: Region,
    pub color: Vec<Vec3>,
    pub albedo: Vec<Vec3>,
    pub normal: Vec<Vec3>,
    pub depth: Vec<f32>,
    /// Variance of each pixel's mean luminance (0 where unknown).
    pub variance: Vec<f32>,
    pub samples: Vec<u16>,
    pub rays: u64,
    /// Samples discarded because they were not finite (should be 0).
    pub bad_samples: u64,
}

#[inline]
fn tent(u: f32) -> f32 {
    if u < 0.5 {
        (2.0 * u).sqrt() - 1.0
    } else {
        1.0 - (2.0 - 2.0 * u).sqrt()
    }
}

struct Px {
    color: Vec3,
    albedo: Vec3,
    normal: Vec3,
    depth: f32,
    variance: f32,
    samples: u16,
    rays: u32,
    bad: u32,
}

fn render_pixel(rt: &SceneRt, film: &FilmRt, x: usize, y: usize, scratch: &mut Scratch, clamp: f32) -> Px {
    let (w, h) = (film.w, film.h);
    let pix = (y * w + x) as u64;
    let spp = film.spp.max(1);
    let min_spp = film.min_spp.clamp(1, spp);
    let (mut sum, mut alb, mut nrm) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
    let (mut dep, mut s1, mut s2) = (0.0f32, 0.0f64, 0.0f64);
    let (mut n, mut rays, mut bad) = (0u32, 0u32, 0u32);
    while n < spp {
        // one independent, reproducible random stream per (seed, pixel, sample)
        let mut rng = Rng::new(hash3(film.seed, pix, n as u64), pix);
        let px = x as f32 + 0.5 + tent(rng.f32());
        let py = y as f32 + 0.5 + tent(rng.f32());
        let (o, d) = rt.cam.ray(px, py, w as f32, h as f32, rng.f32(), rng.f32());
        let r = radiance(rt, o, d, &mut rng, scratch, film.max_depth, clamp);
        rays += r.rays;
        n += 1;
        if !r.l.is_finite() {
            bad += 1;
            continue;
        }
        sum += r.l;
        alb += r.albedo;
        nrm += r.normal;
        dep += r.depth;
        let lum = luminance(r.l) as f64;
        s1 += lum;
        s2 += lum * lum;
        if film.noise_target > 0.0 && n >= min_spp && n % 8 == 0 {
            let mean = s1 / n as f64;
            let var_mean = ((s2 / n as f64 - mean * mean).max(0.0)) / n as f64;
            if var_mean.sqrt() <= film.noise_target as f64 * (mean + 1e-3 * mean.max(1.0)) {
                break;
            }
        }
    }
    let inv = 1.0 / n as f32;
    let mean = s1 / n as f64;
    let variance = (((s2 / n as f64 - mean * mean).max(0.0)) / n as f64) as f32;
    Px { color: sum * inv, albedo: alb * inv, normal: nrm * inv, depth: dep * inv, variance, samples: n.min(u16::MAX as u32) as u16, rays, bad }
}

const BAND: usize = 8;

/// Render the scene's film (or a region of it). The result does not depend on the
/// number of threads, and a region render reproduces the same pixels as a full render.
pub fn render(rt: &SceneRt, opts: &RenderOpts) -> Frame {
    let sf = &rt.scene.film;
    let film = FilmRt {
        w: opts.film.width.unwrap_or(sf.width).max(1) as usize,
        h: opts.film.height.unwrap_or(sf.height).max(1) as usize,
        spp: opts.film.spp.unwrap_or(sf.spp),
        min_spp: sf.min_spp,
        noise_target: sf.noise_target,
        max_depth: opts.film.max_depth.unwrap_or(sf.max_depth),
        seed: sf.seed,
    };
    let film = &film;
    let (w, h) = (film.w, film.h);
    let region = opts.region.unwrap_or(Region { x0: 0, y0: 0, x1: w, y1: h });
    let region = Region { x0: region.x0.min(w), y0: region.y0.min(h), x1: region.x1.min(w), y1: region.y1.min(h) };
    let n = w * h;
    let mut frame = Frame {
        w,
        h,
        region,
        color: vec![Vec3::ZERO; n],
        albedo: vec![Vec3::ZERO; n],
        normal: vec![Vec3::ZERO; n],
        depth: vec![0.0; n],
        variance: vec![0.0; n],
        samples: vec![0; n],
        rays: 0,
        bad_samples: 0,
    };
    if region.x1 <= region.x0 || region.y1 <= region.y0 {
        return frame;
    }
    let bands = (region.y1 - region.y0).div_ceil(BAND);
    let threads = opts.threads.max(1);
    let batch = (threads * 4).max(1);
    let mut done = 0;
    while done < bands {
        let count = batch.min(bands - done);
        let first = done;
        let rows: Vec<Vec<Px>> = par::par_map(count, threads, |i| {
            let y0 = region.y0 + (first + i) * BAND;
            let y1 = (y0 + BAND).min(region.y1);
            let mut scratch = Scratch::default();
            let mut out = Vec::with_capacity((y1 - y0) * (region.x1 - region.x0));
            for y in y0..y1 {
                for x in region.x0..region.x1 {
                    out.push(render_pixel(rt, film, x, y, &mut scratch, opts.clamp));
                }
            }
            out
        });
        for (i, band) in rows.into_iter().enumerate() {
            let y0 = region.y0 + (first + i) * BAND;
            let rw = region.x1 - region.x0;
            for (k, px) in band.into_iter().enumerate() {
                let (x, y) = (region.x0 + k % rw, y0 + k / rw);
                let idx = y * w + x;
                frame.color[idx] = px.color;
                frame.albedo[idx] = px.albedo;
                frame.normal[idx] = px.normal;
                frame.depth[idx] = px.depth;
                frame.variance[idx] = px.variance;
                frame.samples[idx] = px.samples;
                frame.rays += px.rays as u64;
                frame.bad_samples += px.bad as u64;
            }
        }
        done += count;
        if let Some(p) = opts.progress {
            p(done as f32 / bands as f32);
        }
    }
    frame
}

/// Object visible through the centre of each pixel (pinhole, no depth of field);
/// `u32::MAX` where the camera sees the environment. Crisp and independent of sampling,
/// so it is used for inspection, masks and region edits.
pub fn id_pass(rt: &SceneRt, threads: usize) -> Vec<u32> {
    let film = &rt.scene.film;
    let (w, h) = (film.width as usize, film.height as usize);
    let rows: Vec<Vec<u32>> = par::par_map(h, threads.max(1), |y| {
        (0..w)
            .map(|x| {
                let (o, d) = rt.cam.pinhole_ray(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32);
                rt.closest(o, d, 0.0, f32::MAX, true).map(|h| h.obj).unwrap_or(u32::MAX)
            })
            .collect()
    });
    rows.into_iter().flatten().collect()
}
