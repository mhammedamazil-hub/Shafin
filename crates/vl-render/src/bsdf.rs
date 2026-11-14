//! One principled BSDF for every material.
//!
//! Lobes: Lambert diffuse, GGX specular reflection (Schlick Fresnel: tinted for
//! metals, `f0` for dielectrics), GGX reflection + refraction for transmissive
//! materials (exact dielectric Fresnel, Walter et al. 2007), and a GGX clear
//! coat. Microfacet normals are drawn from the distribution of visible normals
//! (Heitz 2018). Very smooth lobes are handled as perfect (delta) reflection or
//! refraction.
//!
//! Everything works in the local shading frame (+Z = shading normal, on the side
//! of `wo`). `wo` points away from the surface towards the viewer; `wi` points
//! away from the surface towards the light.
//!
//! Convention for refraction: the 1/eta^2 radiance scaling at an interface is
//! omitted on both entry and exit (they cancel for any path that starts and ends
//! outside the medium), which keeps path throughput steady inside glass.
use vl_core::color::luminance;
use vl_core::dmath::{self, INV_PI32};
use vl_core::math::{vec3, Vec3};

/// Roughness below this is treated as a perfect mirror / perfectly clear interface.
pub const DELTA_ROUGHNESS: f32 = 0.02;

#[derive(Clone, Copy, Debug)]
pub struct Bsdf {
    pub base: Vec3,
    pub metallic: f32,
    /// GGX alpha (= roughness^2) of the base specular and transmission lobes.
    pub alpha: f32,
    pub spec_delta: bool,
    /// Dielectric reflectance at normal incidence, already multiplied by the specular weight.
    pub f0: f32,
    /// Index of refraction on the far side divided by the index on the `wo` side.
    pub eta: f32,
    pub trans: f32,
    pub trans_tint: Vec3,
    pub coat: f32,
    pub coat_alpha: f32,
    pub coat_delta: bool,
    pub sheen: f32,
    pub thin: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BsdfSample {
    pub wi: Vec3,
    /// f * |cos| / pdf (or the lobe weight for delta events).
    pub weight: Vec3,
    /// Solid-angle density of `wi` (0 for delta events).
    pub pdf: f32,
    pub delta: bool,
    pub transmitted: bool,
}

#[inline]
fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    x2 * x2 * x
}

/// Schlick Fresnel for a dielectric layer. The grazing reflectance is 1 for any real
/// interface but fades to 0 as f0 -> 0, so a material with no specular layer is exactly
/// Lambertian instead of turning into a mirror at grazing angles.
#[inline]
fn schlick(f0: f32, c: f32) -> f32 {
    let f90 = (f0 * 50.0).min(1.0);
    f0 + (f90 - f0) * pow5((1.0 - c).clamp(0.0, 1.0))
}

#[inline]
fn schlick3(f0: Vec3, c: f32) -> Vec3 {
    f0 + (Vec3::ONE - f0) * pow5((1.0 - c).clamp(0.0, 1.0))
}

/// Unpolarised Fresnel reflectance of a dielectric interface; `eta` = n_far / n_near.
#[inline]
pub fn fresnel_dielectric(cos_i: f32, eta: f32) -> f32 {
    let c = cos_i.clamp(0.0, 1.0);
    let sin2_t = (1.0 - c * c) / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0; // total internal reflection
    }
    let ct = (1.0 - sin2_t).sqrt();
    let r_par = (eta * c - ct) / (eta * c + ct);
    let r_perp = (c - eta * ct) / (c + eta * ct);
    0.5 * (r_par * r_par + r_perp * r_perp)
}

#[inline]
fn ggx_d(alpha: f32, hz: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = hz * hz * (a2 - 1.0) + 1.0;
    a2 * INV_PI32 / (d * d)
}

#[inline]
fn ggx_lambda(alpha: f32, w: Vec3) -> f32 {
    let c2 = w.z * w.z;
    if c2 <= 0.0 {
        return f32::MAX;
    }
    let t2 = (1.0 - c2).max(0.0) / c2;
    0.5 * (-1.0 + (1.0 + alpha * alpha * t2).sqrt())
}

#[inline]
fn ggx_g1(alpha: f32, w: Vec3) -> f32 {
    1.0 / (1.0 + ggx_lambda(alpha, w))
}

#[inline]
fn ggx_g(alpha: f32, wo: Vec3, wi: Vec3) -> f32 {
    1.0 / (1.0 + ggx_lambda(alpha, wo) + ggx_lambda(alpha, wi))
}

/// Density of visible normals: D_wo(h) = G1(wo) max(0, wo.h) D(h) / wo.z
#[inline]
fn ggx_dv(alpha: f32, wo: Vec3, h: Vec3) -> f32 {
    ggx_g1(alpha, wo) * wo.dot(h).max(0.0) * ggx_d(alpha, h.z) / wo.z.abs().max(1e-9)
}

/// Sample a visible microfacet normal (Heitz 2018, isotropic). `wo.z` must be > 0.
fn sample_vndf(alpha: f32, wo: Vec3, u1: f32, u2: f32) -> Vec3 {
    let vh = vec3(alpha * wo.x, alpha * wo.y, wo.z).normalize();
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = if lensq > 0.0 { vec3(-vh.y, vh.x, 0.0) * (1.0 / lensq.sqrt()) } else { Vec3::X };
    let t2 = vh.cross(t1);
    let r = u1.sqrt();
    let (sp, cp) = dmath::sincos(dmath::TAU32 * u2);
    let a = r * cp;
    let s = 0.5 * (1.0 + vh.z);
    let b = (1.0 - s) * (1.0 - a * a).max(0.0).sqrt() + s * (r * sp);
    let nh = t1 * a + t2 * b + vh * (1.0 - a * a - b * b).max(0.0).sqrt();
    vec3(alpha * nh.x, alpha * nh.y, nh.z.max(1e-9)).normalize()
}

#[inline]
pub fn cosine_hemisphere(u1: f32, u2: f32) -> Vec3 {
    let r = u1.sqrt();
    let (s, c) = dmath::sincos(dmath::TAU32 * u2);
    vec3(r * c, r * s, (1.0 - u1).max(0.0).sqrt())
}

struct Lobes {
    rest: f32,
    p_coat: f32,
    p_spec: f32,
    p_diff: f32,
    p_glass: f32,
    fd_o: f32,
}

impl Bsdf {
    /// A plain Lambertian surface (used by tests and as a fallback).
    pub fn diffuse(base: Vec3) -> Bsdf {
        Bsdf { base, metallic: 0.0, alpha: 1.0, spec_delta: false, f0: 0.0, eta: 1.5, trans: 0.0, trans_tint: Vec3::ONE, coat: 0.0, coat_alpha: 0.01, coat_delta: true, sheen: 0.0, thin: false }
    }

    /// Does the BSDF contain any lobe that light sampling can use?
    pub fn has_non_delta(&self) -> bool {
        let opaque_dielectric = (1.0 - self.metallic) * (1.0 - self.trans);
        opaque_dielectric > 0.0 || !self.spec_delta || (self.coat > 0.0 && !self.coat_delta)
    }

    fn lobes(&self, cos_o: f32) -> Lobes {
        let fc = self.coat * schlick(0.04, cos_o);
        let rest = 1.0 - fc;
        let fd_o = schlick(self.f0, cos_o);
        let dielectric = (1.0 - self.metallic) * (1.0 - self.trans);
        let w_coat = fc;
        let w_spec = rest * (self.metallic * luminance(schlick3(self.base, cos_o)).max(0.02) + dielectric * fd_o);
        let w_diff = rest * dielectric * (1.0 - fd_o) * (luminance(self.base) + self.sheen).clamp(0.05, 1.0);
        let w_glass = rest * (1.0 - self.metallic) * self.trans;
        let sum = w_coat + w_spec + w_diff + w_glass;
        if sum <= 0.0 {
            return Lobes { rest, p_coat: 0.0, p_spec: 0.0, p_diff: 1.0, p_glass: 0.0, fd_o };
        }
        Lobes { rest, p_coat: w_coat / sum, p_spec: w_spec / sum, p_diff: w_diff / sum, p_glass: w_glass / sum, fd_o }
    }

    /// Evaluate the non-delta lobes: returns (f * |cos(wi)|, pdf of sampling wi).
    pub fn eval(&self, wo: Vec3, wi: Vec3) -> (Vec3, f32) {
        if wo.z <= 0.0 || wi.z == 0.0 {
            return (Vec3::ZERO, 0.0);
        }
        let l = self.lobes(wo.z);
        let glass_w = l.rest * (1.0 - self.metallic) * self.trans;
        let mut f = Vec3::ZERO;
        let mut pdf = 0.0;
        if wi.z > 0.0 {
            let h = (wo + wi).normalize();
            let oh = wo.dot(h);
            if oh <= 0.0 {
                return (Vec3::ZERO, 0.0);
            }
            let dielectric = (1.0 - self.metallic) * (1.0 - self.trans);
            if !self.spec_delta {
                let d = ggx_d(self.alpha, h.z);
                let g = ggx_g(self.alpha, wo, wi);
                let pdf_h = ggx_dv(self.alpha, wo, h) / (4.0 * oh);
                let micro = d * g / (4.0 * wo.z * wi.z);
                let f_spec = schlick3(self.base, oh) * self.metallic + Vec3::splat(dielectric * schlick(self.f0, oh));
                f += f_spec * (l.rest * micro);
                pdf += l.p_spec * pdf_h;
                if glass_w > 0.0 {
                    let fg = fresnel_dielectric(oh, self.eta);
                    f += Vec3::splat(glass_w * fg * micro);
                    pdf += l.p_glass * fg * pdf_h;
                }
            }
            if dielectric > 0.0 {
                let fd_i = schlick(self.f0, wi.z);
                let k = l.rest * dielectric * INV_PI32;
                f += self.base * (k * (1.0 - l.fd_o) * (1.0 - fd_i));
                if self.sheen > 0.0 {
                    f += Vec3::splat(k * self.sheen * pow5(1.0 - oh));
                }
                pdf += l.p_diff * wi.z * INV_PI32;
            }
            if self.coat > 0.0 && !self.coat_delta {
                let d = ggx_d(self.coat_alpha, h.z);
                let g = ggx_g(self.coat_alpha, wo, wi);
                f += Vec3::splat(self.coat * schlick(0.04, oh) * d * g / (4.0 * wo.z * wi.z));
                pdf += l.p_coat * ggx_dv(self.coat_alpha, wo, h) / (4.0 * oh);
            }
            (f * wi.z, pdf)
        } else {
            if glass_w <= 0.0 || self.spec_delta {
                return (Vec3::ZERO, 0.0);
            }
            let ci = -wi.z;
            if self.thin {
                // thin sheet: the transmitted lobe is the reflection lobe mirrored through the surface
                let wr = vec3(wi.x, wi.y, ci);
                let h = (wo + wr).normalize();
                let oh = wo.dot(h);
                if oh <= 0.0 {
                    return (Vec3::ZERO, 0.0);
                }
                let fg = fresnel_dielectric(oh, self.eta);
                let micro = ggx_d(self.alpha, h.z) * ggx_g(self.alpha, wo, wr) / (4.0 * wo.z * ci);
                let pdf_h = ggx_dv(self.alpha, wo, h) / (4.0 * oh);
                return (self.trans_tint * (glass_w * (1.0 - fg) * micro * ci), l.p_glass * (1.0 - fg) * pdf_h);
            }
            let mut h = (wo + wi * self.eta).normalize();
            if h.z < 0.0 {
                h = -h;
            }
            let oh = wo.dot(h);
            let ih = wi.dot(h);
            if oh <= 0.0 || ih >= 0.0 {
                return (Vec3::ZERO, 0.0);
            }
            let fg = fresnel_dielectric(oh, self.eta);
            let denom = oh + self.eta * ih;
            let denom2 = denom * denom;
            if denom2 < 1e-12 {
                return (Vec3::ZERO, 0.0);
            }
            let d = ggx_d(self.alpha, h.z);
            let g = ggx_g(self.alpha, wo, wi);
            let jac = self.eta * self.eta * ih.abs() / denom2;
            let ft = (1.0 - fg) * d * g * oh * jac / (wo.z * ci);
            (self.trans_tint * (glass_w * ft * ci), l.p_glass * (1.0 - fg) * ggx_dv(self.alpha, wo, h) * jac)
        }
    }

    /// Draw a direction. `u` = (lobe choice, two sampling numbers, reflect/refract choice).
    pub fn sample(&self, wo: Vec3, u: [f32; 4]) -> Option<BsdfSample> {
        if wo.z <= 0.0 {
            return None;
        }
        let l = self.lobes(wo.z);
        let glass_w = l.rest * (1.0 - self.metallic) * self.trans;
        let mirror = vec3(-wo.x, -wo.y, wo.z);
        let delta = |wi: Vec3, weight: Vec3, transmitted: bool| Some(BsdfSample { wi, weight, pdf: 0.0, delta: true, transmitted });
        let wi;
        let mut expect_below = false;
        let c = u[0];
        if c < l.p_coat {
            if self.coat_delta {
                return delta(mirror, Vec3::splat(self.coat * schlick(0.04, wo.z) / l.p_coat), false);
            }
            let h = sample_vndf(self.coat_alpha, wo, u[1], u[2]);
            wi = h * (2.0 * wo.dot(h)) - wo;
        } else if c < l.p_coat + l.p_spec {
            if self.spec_delta {
                let dielectric = (1.0 - self.metallic) * (1.0 - self.trans);
                let f_spec = schlick3(self.base, wo.z) * self.metallic + Vec3::splat(dielectric * l.fd_o);
                return delta(mirror, f_spec * (l.rest / l.p_spec), false);
            }
            let h = sample_vndf(self.alpha, wo, u[1], u[2]);
            wi = h * (2.0 * wo.dot(h)) - wo;
        } else if c < l.p_coat + l.p_spec + l.p_glass {
            let (h, is_delta) = if self.spec_delta { (Vec3::Z, true) } else { (sample_vndf(self.alpha, wo, u[1], u[2]), false) };
            let oh = wo.dot(h);
            let fg = fresnel_dielectric(oh, self.eta);
            let reflect = u[3] < fg;
            let dir = if reflect {
                h * (2.0 * oh) - wo
            } else if self.thin {
                let r = h * (2.0 * oh) - wo;
                vec3(r.x, r.y, -r.z)
            } else {
                let sin2_t = (1.0 - oh * oh).max(0.0) / (self.eta * self.eta);
                let ct = (1.0 - sin2_t).max(0.0).sqrt();
                (-wo * (1.0 / self.eta) + h * (oh / self.eta - ct)).normalize()
            };
            if is_delta {
                // the Fresnel term cancels against the probability of the branch taken
                let w = glass_w / l.p_glass;
                return if reflect { delta(dir, Vec3::splat(w), false) } else { delta(dir, self.trans_tint * w, true) };
            }
            expect_below = !reflect;
            wi = dir;
        } else {
            wi = cosine_hemisphere(u[1], u[2]);
        }
        // A microfacet reflection can point below the surface (and a refraction above it).
        // Such directions are not part of the lobe that produced them; accepting them and
        // re-evaluating with eval() would weight them with a density they were not drawn from.
        if (wi.z < 0.0) != expect_below {
            return None;
        }
        let (f_cos, pdf) = self.eval(wo, wi);
        if !(pdf > 0.0) || !f_cos.is_finite() {
            return None;
        }
        Some(BsdfSample { wi, weight: f_cos * (1.0 / pdf), pdf, delta: false, transmitted: expect_below })
    }

    /// Fraction of light a straight shadow ray keeps when crossing this surface
    /// (the "transparent shadow" approximation used instead of refractive caustics).
    pub fn shadow_transmittance(&self, cos: f32) -> Vec3 {
        let w = (1.0 - self.metallic) * self.trans * (1.0 - self.coat * schlick(0.04, cos));
        if w <= 0.0 {
            return Vec3::ZERO;
        }
        // Fresnel as seen from the air side (reflectance is symmetric for the reverse direction)
        let n = if self.eta >= 1.0 { self.eta } else { 1.0 / self.eta };
        self.trans_tint * (w * (1.0 - fresnel_dielectric(cos, n)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vl_core::rng::Rng;

    fn mat(base: f32, metallic: f32, rough: f32, trans: f32, coat: f32) -> Bsdf {
        Bsdf {
            base: Vec3::splat(base),
            metallic,
            alpha: rough * rough,
            spec_delta: rough < DELTA_ROUGHNESS,
            f0: 0.04,
            eta: 1.5,
            trans,
            trans_tint: Vec3::ONE,
            coat,
            coat_alpha: 0.3 * 0.3,
            coat_delta: false,
            sheen: 0.0,
            thin: false,
        }
    }

    fn uniform_sphere(u1: f32, u2: f32) -> Vec3 {
        let z = 1.0 - 2.0 * u1;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let (s, c) = dmath::sincos(dmath::TAU32 * u2);
        vec3(r * c, r * s, z)
    }

    fn cases() -> Vec<(&'static str, Bsdf)> {
        vec![
            ("white diffuse", Bsdf::diffuse(Vec3::ONE)),
            ("plastic r0.5", mat(1.0, 0.0, 0.5, 0.0, 0.0)),
            ("plastic r0.3", mat(1.0, 0.0, 0.3, 0.0, 0.0)),
            ("metal r0.6", mat(1.0, 1.0, 0.6, 0.0, 0.0)),
            ("metal r0.35", mat(1.0, 1.0, 0.35, 0.0, 0.0)),
            ("glass r0.5", mat(1.0, 0.0, 0.5, 1.0, 0.0)),
            ("glass r0.35", mat(1.0, 0.0, 0.35, 1.0, 0.0)),
            ("glass inside r0.5", Bsdf { eta: 1.0 / 1.5, ..mat(1.0, 0.0, 0.5, 1.0, 0.0) }),
            ("thin glass r0.5", Bsdf { thin: true, ..mat(1.0, 0.0, 0.5, 1.0, 0.0) }),
            ("coated plastic", mat(1.0, 0.0, 0.5, 0.0, 1.0)),
            ("half metal half glass", mat(1.0, 0.5, 0.5, 0.5, 0.5)),
        ]
    }

    /// Reference integrals by deterministic midpoint quadrature over the whole sphere
    /// (uniform in z and azimuth, so every cell has the same solid angle). Returns
    /// (integral of f cos, integral of the sampling density).
    fn quadrature(b: &Bsdf, wo: Vec3) -> (f64, f64) {
        let (nz, np) = (1400usize, 1400usize);
        let (mut albedo, mut pdf_int) = (0.0f64, 0.0f64);
        for iz in 0..nz {
            let z = -1.0 + 2.0 * (iz as f32 + 0.5) / nz as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            for ip in 0..np {
                let (sp, cp) = dmath::sincos(dmath::TAU32 * (ip as f32 + 0.5) / np as f32);
                let (f, pdf) = b.eval(wo, vec3(r * cp, r * sp, z));
                albedo += f.x as f64;
                pdf_int += pdf as f64;
            }
        }
        let cell = 4.0 * std::f64::consts::PI / (nz * np) as f64;
        (albedo * cell, pdf_int * cell)
    }

    /// sample() must draw directions with exactly the density eval() reports, that density must
    /// integrate to at most 1, and no material may reflect more energy than it receives.
    #[test]
    fn sampling_matches_evaluation_and_conserves_energy() {
        let mut rng = Rng::new(42, 7);
        let _ = uniform_sphere(0.5, 0.5);
        for (name, b) in cases() {
            for &cos_o in &[0.95f32, 0.6, 0.25] {
                let wo = vec3((1.0 - cos_o * cos_o).sqrt(), 0.0, cos_o);
                let (albedo_q, pdf_int) = quadrature(&b, wo);
                let n = 400_000;
                let (mut sum, mut sum2, mut bad) = (0.0f64, 0.0f64, 0);
                for _ in 0..n {
                    if let Some(s) = b.sample(wo, [rng.f32(), rng.f32(), rng.f32(), rng.f32()]) {
                        assert!(!s.delta);
                        sum += s.weight.x as f64;
                        sum2 += (s.weight.x as f64).powi(2);
                        let (f, pdf) = b.eval(wo, s.wi);
                        if (pdf - s.pdf).abs() > 1e-4 * pdf.max(1.0) || (f.x / pdf - s.weight.x).abs() > 1e-3 * s.weight.x.max(1.0) {
                            bad += 1;
                        }
                        assert_eq!(s.transmitted, s.wi.z < 0.0);
                    }
                }
                let albedo_s = sum / n as f64;
                let stderr = ((sum2 / n as f64 - albedo_s * albedo_s).max(0.0) / n as f64).sqrt();
                println!("{name:22} cos_o {cos_o:4}: albedo quadrature {albedo_q:.4}  sampled {albedo_s:.4} +- {stderr:.4}  integral of pdf {pdf_int:.4}");
                assert_eq!(bad, 0, "{name}: sample() and eval() disagree");
                assert!(pdf_int < 1.004, "{name}: pdf integrates to {pdf_int} > 1");
                assert!(pdf_int > 0.55, "{name}: pdf integrates to only {pdf_int}");
                // agreement within 5 standard errors of the sampled estimate plus quadrature error
                assert!((albedo_q - albedo_s).abs() < 5.0 * stderr + 0.004, "{name} cos_o {cos_o}: quadrature {albedo_q} vs sampled {albedo_s} (stderr {stderr})");
                assert!(albedo_q < 1.004, "{name}: reflects more energy than it receives ({albedo_q})");
                if name == "white diffuse" {
                    assert!((albedo_q - 1.0).abs() < 2e-3 && (pdf_int - 1.0).abs() < 2e-3, "a white Lambertian surface must have albedo 1 at every angle");
                }
            }
        }
    }

    #[test]
    fn delta_lobes_conserve_energy() {
        let mut rng = Rng::new(3, 3);
        let glass = Bsdf { coat_delta: true, ..mat(1.0, 0.0, 0.0, 1.0, 0.0) };
        let mirror = Bsdf { coat_delta: true, ..mat(1.0, 1.0, 0.0, 0.0, 0.0) };
        let glossy = Bsdf { coat_delta: true, ..mat(1.0, 0.0, 0.0, 0.0, 0.0) }; // mirror-like coat over diffuse
        for (name, b, expect_one) in [("glass", glass, true), ("mirror", mirror, true), ("smooth plastic", glossy, false)] {
            for &cos_o in &[0.9f32, 0.3] {
                let wo = vec3((1.0 - cos_o * cos_o).sqrt(), 0.0, cos_o);
                let (mut sum, mut refl, mut tran) = (0.0f64, 0, 0);
                let n = 200_000;
                for _ in 0..n {
                    if let Some(s) = b.sample(wo, [rng.f32(), rng.f32(), rng.f32(), rng.f32()]) {
                        sum += s.weight.x as f64;
                        if s.delta && s.transmitted {
                            tran += 1;
                            // Snell's law
                            let sin_o = (1.0 - wo.z * wo.z).sqrt();
                            let sin_i = (1.0 - s.wi.z * s.wi.z).sqrt();
                            assert!((sin_o - 1.5 * sin_i).abs() < 1e-4 && s.wi.z < 0.0 && s.wi.x < 0.0);
                        } else if s.delta {
                            refl += 1;
                            assert!((s.wi - vec3(-wo.x, -wo.y, wo.z)).length() < 1e-6);
                        }
                    }
                }
                let albedo = sum / n as f64;
                println!("{name:15} cos_o {cos_o}: albedo {albedo:.4}, reflected {refl}, refracted {tran}");
                if expect_one {
                    assert!((albedo - 1.0).abs() < 0.01, "{name}: lossless material has albedo {albedo}");
                } else {
                    assert!(albedo > 0.85 && albedo < 1.005, "{name}: albedo {albedo}");
                }
                if name == "glass" {
                    let f = fresnel_dielectric(cos_o, 1.5) as f64;
                    assert!((refl as f64 / n as f64 - f).abs() < 0.01, "reflection frequency must equal Fresnel");
                }
            }
        }
        // Fresnel sanity: 4% at normal incidence for n = 1.5, total internal reflection from inside
        assert!((fresnel_dielectric(1.0, 1.5) - 0.04).abs() < 1e-4);
        assert_eq!(fresnel_dielectric(0.3, 1.0 / 1.5), 1.0);
        assert!((fresnel_dielectric(1.0, 1.0 / 1.5) - 0.04).abs() < 1e-4);
    }
}
