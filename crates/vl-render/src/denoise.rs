//! Noise removal for path-traced frames.
//!
//! An edge-avoiding à-trous wavelet filter (Dammertz et al. 2010) guided by the per-pixel
//! variance the integrator measures (the spatial part of SVGF, Schied et al. 2017):
//!
//! 1. The surface colour of the first hit is divided out, so that texture detail is not
//!    touched; what is filtered is the light falling on the surface.
//! 2. Five passes of a 5x5 B3-spline kernel with holes (reach 1, 2, 4, 8, 16 pixels).
//!    A neighbour contributes only if it lies on the same surface (similar normal, a depth
//!    that fits the local slope) and if its brightness differs from the centre by no more
//!    than the centre's own noise explains.
//! 3. The surface colour is multiplied back.
//!
//! Where the frame has converged the measured variance is small, the brightness test
//! rejects every neighbour that differs, and the pixel is left as it is. The filter is
//! deterministic and independent of the number of threads.
//!
//! Known weaknesses: what is seen in a mirror or through glass has no features of its own
//! here, so noisy reflections are smoothed across the edges of the reflected objects; and
//! like every filter of this kind it trades noise for a little blur in soft shadows.
use crate::integrator::Frame;
use vl_core::color::luminance;
use vl_core::dmath;
use vl_core::math::Vec3;
use vl_core::par;

const PASSES: usize = 5;
const KERNEL: [f32; 5] = [1.0 / 16.0, 1.0 / 4.0, 3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
/// A brightness difference of this many standard deviations still counts as noise.
const SIGMA_L: f32 = 4.0;
const SIGMA_N: i32 = 64;
const ALBEDO_FLOOR: f32 = 0.02;

/// Returns the filtered colours (scene-referred, same layout as `frame.color`).
pub fn denoise(frame: &Frame, threads: usize) -> Vec<Vec3> {
    let (w, h) = (frame.w, frame.h);
    let n = w * h;
    if n == 0 || frame.color.len() != n {
        return frame.color.clone();
    }
    let surface = |i: usize| frame.depth[i] > 0.0 && frame.normal[i].length_sq() > 0.25;
    let floor = |a: Vec3| a.max(Vec3::splat(ALBEDO_FLOOR));
    let normal: Vec<Vec3> = (0..n).map(|i| if surface(i) { frame.normal[i].normalize() } else { Vec3::ZERO }).collect();

    // 1. light = colour / surface colour; variance of its luminance
    let mut light: Vec<Vec3> = (0..n).map(|i| if surface(i) { frame.color[i] / floor(frame.albedo[i]) } else { frame.color[i] }).collect();
    let mut var: Vec<f32> = (0..n)
        .map(|i| {
            let a = if surface(i) { luminance(floor(frame.albedo[i])) } else { 1.0 };
            frame.variance[i] / (a * a)
        })
        .collect();

    // A single sample that found a tiny bright path is not information about this pixel
    // alone. Where a pixel's own estimate is unreliable (standard error above half its
    // value) and it outshines all eight neighbours, it is limited to the brightest of them.
    let lum: Vec<f32> = light.iter().map(|c| luminance(*c)).collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !surface(i) || var[i].sqrt() < 0.5 * lum[i] {
                continue;
            }
            let mut top = 0.0f32;
            for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (qx, qy) = (x as i32 + dx, y as i32 + dy);
                if qx >= 0 && qy >= 0 && (qx as usize) < w && (qy as usize) < h {
                    top = top.max(lum[qy as usize * w + qx as usize]);
                }
            }
            if lum[i] > top && top > 0.0 {
                let k = top / lum[i];
                light[i] = light[i] * k;
                var[i] *= k * k;
            }
        }
    }

    // depth slope per pixel (the smaller of the two one-sided differences avoids edges)
    let slope: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let z = frame.depth[i];
            let d = |q: Option<usize>| q.filter(|&q| surface(q)).map(|q| frame.depth[q] - z);
            let pick = |a: Option<f32>, b: Option<f32>| match (a, b) {
                (Some(a), Some(b)) => {
                    if a.abs() < b.abs() {
                        a
                    } else {
                        -b
                    }
                }
                (Some(a), None) => a,
                (None, Some(b)) => -b,
                (None, None) => 0.0,
            };
            let dx = pick(d((x + 1 < w).then(|| i + 1)), d((x > 0).then(|| i - 1)));
            let dy = pick(d((y + 1 < h).then(|| i + w)), d((y > 0).then(|| i - w)));
            (dx, dy)
        })
        .collect();

    // 2. à-trous passes
    for pass in 0..PASSES {
        let step = 1i32 << pass;
        // the variance guiding the brightness test, smoothed over 3x3 (per-pixel estimates from a
        // few dozen samples are themselves noisy)
        let guide: Vec<f32> = (0..n)
            .map(|i| {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                let (mut s, mut ws) = (0.0f32, 0.0f32);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (qx, qy) = (x + dx, y + dy);
                        if qx >= 0 && qy >= 0 && (qx as usize) < w && (qy as usize) < h {
                            let k = [0.25f32, 0.5, 0.25][(dx + 1) as usize] * [0.25f32, 0.5, 0.25][(dy + 1) as usize];
                            s += k * var[qy as usize * w + qx as usize];
                            ws += k;
                        }
                    }
                }
                s / ws
            })
            .collect();
        let (light_in, var_in) = (&light, &var);
        let rows: Vec<Vec<(Vec3, f32)>> = par::par_map(h, threads.max(1), |y| {
            let mut out = Vec::with_capacity(w);
            for x in 0..w {
                let p = y * w + x;
                if !surface(p) {
                    out.push((light_in[p], var_in[p]));
                    continue;
                }
                let (np, zp, lp) = (normal[p], frame.depth[p], luminance(light_in[p]));
                let sd = SIGMA_L * guide[p].max(0.0).sqrt() + 1e-6 * (lp + 1e-6);
                let (mut sum, mut vsum, mut wsum) = (Vec3::ZERO, 0.0f32, 0.0f32);
                for ky in 0..5i32 {
                    let qy = y as i32 + (ky - 2) * step;
                    if qy < 0 || qy as usize >= h {
                        continue;
                    }
                    for kx in 0..5i32 {
                        let qx = x as i32 + (kx - 2) * step;
                        if qx < 0 || qx as usize >= w {
                            continue;
                        }
                        let q = qy as usize * w + qx as usize;
                        let wgt = if q == p {
                            KERNEL[2] * KERNEL[2]
                        } else {
                            if !surface(q) {
                                continue;
                            }
                            let cosn = np.dot(normal[q]);
                            if cosn < 0.9 {
                                continue; // a different facing: weight below 0.001 anyway
                            }
                            let wn = cosn.powi(SIGMA_N);
                            // depth predicted from the centre's slope
                            let expect = zp + slope[p].0 * ((kx - 2) * step) as f32 + slope[p].1 * ((ky - 2) * step) as f32;
                            let tol = 0.01 * zp + 0.25 * (slope[p].0.abs() + slope[p].1.abs()) * step as f32 + 1e-6;
                            let dz = (frame.depth[q] - expect).abs() / tol;
                            let dl = (luminance(light_in[q]) - lp).abs() / sd;
                            if dz + dl > 12.0 {
                                continue;
                            }
                            KERNEL[kx as usize] * KERNEL[ky as usize] * wn * dmath::exp(-dz - dl)
                        };
                        sum += light_in[q] * wgt;
                        vsum += var_in[q] * wgt * wgt;
                        wsum += wgt;
                    }
                }
                out.push((sum * (1.0 / wsum), vsum / (wsum * wsum)));
            }
            out
        });
        for (y, row) in rows.into_iter().enumerate() {
            for (x, (c, v)) in row.into_iter().enumerate() {
                light[y * w + x] = c;
                var[y * w + x] = v;
            }
        }
    }

    // 3. surface colour back in
    (0..n).map(|i| if surface(i) { light[i] * floor(frame.albedo[i]) } else { light[i] }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrator::Region;
    use vl_core::math::vec3;
    use vl_core::rng::Rng;

    /// A frame showing one flat surface seen head-on, to be filled in by each test.
    fn flat(w: usize, h: usize) -> Frame {
        let n = w * h;
        Frame { w, h, region: Region { x0: 0, y0: 0, x1: w, y1: h }, color: vec![Vec3::ONE; n], albedo: vec![Vec3::ONE; n], normal: vec![Vec3::Z; n], depth: vec![5.0; n], variance: vec![0.0; n], samples: vec![16; n], rays: 0, bad_samples: 0 }
    }

    /// Standard normal deviate (sum of twelve uniforms; exact enough for a test).
    fn gauss(r: &mut Rng) -> f32 {
        (0..12).map(|_| r.f32()).sum::<f32>() - 6.0
    }

    fn rms(a: &[Vec3], truth: impl Fn(usize) -> f32) -> f32 {
        (a.iter().enumerate().map(|(i, c)| (c.x - truth(i)).powi(2)).sum::<f32>() / a.len() as f32).sqrt()
    }

    #[test]
    fn a_converged_frame_is_left_alone() {
        // sharp light-and-shadow pattern, no noise, variance zero: nothing may change
        let (w, h) = (48, 32);
        let mut f = flat(w, h);
        for i in 0..w * h {
            let (x, y) = (i % w, i / w);
            f.color[i] = Vec3::splat(if (x / 5 + y / 3) % 2 == 0 { 0.2 } else { 7.0 }) * vec3(1.0, 0.8, 0.6);
        }
        let out = denoise(&f, 2);
        for i in 0..w * h {
            assert!((out[i] - f.color[i]).abs().max_comp() < 1e-4 * f.color[i].max_comp(), "pixel {i}: {:?} -> {:?}", f.color[i], out[i]);
        }
    }

    #[test]
    fn noise_goes_and_the_shadow_edge_stays() {
        // light 1 on the left, 3 on the right, 10% noise of known variance
        let (w, h) = (96, 64);
        let mut f = flat(w, h);
        let truth = |i: usize| if i % w < w / 2 { 1.0f32 } else { 3.0 };
        let mut r = Rng::new(7, 0);
        for i in 0..w * h {
            let sd = 0.1 * truth(i);
            f.color[i] = Vec3::splat(truth(i) + sd * gauss(&mut r));
            f.variance[i] = sd * sd;
        }
        let before = rms(&f.color, truth);
        let out = denoise(&f, 2);
        let after = rms(&out, truth);
        assert!(after < before / 4.0, "noise {before} -> {after}");
        // the two columns next to the edge are still on their own side
        for y in 0..h {
            for x in [w / 2 - 1, w / 2] {
                let i = y * w + x;
                assert!((out[i].x - truth(i)).abs() < 0.12 * truth(i), "edge pixel ({x}, {y}) = {} (should be {})", out[i].x, truth(i));
            }
        }
        // no energy is gained or lost
        let (m0, m1): (f32, f32) = (f.color.iter().map(|c| c.x).sum::<f32>(), out.iter().map(|c| c.x).sum::<f32>());
        assert!((m1 / m0 - 1.0).abs() < 0.005, "mean changed by {}", m1 / m0 - 1.0);
    }

    #[test]
    fn texture_is_not_blurred() {
        // a one-pixel checker in the surface colour under noisy even light
        let (w, h) = (64, 64);
        let mut f = flat(w, h);
        let mut r = Rng::new(3, 0);
        for i in 0..w * h {
            let a = if (i % w + i / w) % 2 == 0 { 0.2 } else { 0.8 };
            let light = 2.0 + 0.3 * gauss(&mut r);
            f.albedo[i] = Vec3::splat(a);
            f.color[i] = Vec3::splat(a * light);
            f.variance[i] = (a * 0.3) * (a * 0.3);
        }
        let out = denoise(&f, 2);
        let mean = |want_dark: bool| {
            let v: Vec<f32> = (0..w * h).filter(|i| ((i % w + i / w) % 2 == 0) == want_dark).map(|i| out[i].x).collect();
            v.iter().sum::<f32>() / v.len() as f32
        };
        assert!((mean(false) / mean(true) - 4.0).abs() < 0.05, "checker contrast {} (should be 4)", mean(false) / mean(true));
        // and each cell is close to its true value: the noise is gone
        let truth = |i: usize| if (i % w + i / w) % 2 == 0 { 0.4f32 } else { 1.6 };
        assert!(rms(&out, truth) < rms(&f.color, truth) / 4.0);
    }

    #[test]
    fn light_does_not_leak_between_surfaces() {
        // two faces meeting at a corner (different normals, same depth) and a nearer object
        // facing the same way (same normal, depth jump), lit differently by amounts the noise
        // could hide
        let (w, h) = (90, 40);
        let mut f = flat(w, h);
        let truth = |i: usize| [1.0f32, 1.5, 1.0][(i % w) / 30];
        let mut r = Rng::new(11, 0);
        for i in 0..w * h {
            let part = (i % w) / 30;
            f.normal[i] = [Vec3::Z, Vec3::X, Vec3::X][part];
            f.depth[i] = [5.0, 5.0, 3.0][part];
            f.color[i] = Vec3::splat(truth(i) + 0.3 * gauss(&mut r));
            f.variance[i] = 0.09;
        }
        let out = denoise(&f, 2);
        for x in [29usize, 30, 59, 60] {
            let col: f32 = (0..h).map(|y| out[y * w + x].x).sum::<f32>() / h as f32;
            assert!((col - truth(x)).abs() < 0.06, "column {x}: {col} (should be {})", truth(x));
        }
        assert!(rms(&out, truth) < 0.3 / 4.0, "noise left: {}", rms(&out, truth));
    }

    #[test]
    fn a_lone_bright_sample_is_tamed_and_a_steady_highlight_is_kept() {
        let (w, h) = (33, 33);
        let mut f = flat(w, h);
        // an unreliable spike (its variance says so) and a reliable one of the same height
        let (spike, glint) = (10 * w + 10, 22 * w + 22);
        f.color[spike] = Vec3::splat(400.0);
        f.variance[spike] = 400.0 * 400.0;
        f.color[glint] = Vec3::splat(400.0);
        f.variance[glint] = 0.0;
        let out = denoise(&f, 1);
        assert!(out[spike].x < 1.05, "the spike survived: {}", out[spike].x);
        // and it was not smeared into a bright smudge around the place
        for dy in -6i32..=6 {
            for dx in -6i32..=6 {
                let v = out[(10 + dy) as usize * w + (10 + dx) as usize].x;
                assert!((v - 1.0).abs() < 0.05, "smudge at ({dx}, {dy}): {v}");
            }
        }
        assert!((out[glint].x - 400.0).abs() < 0.01, "the highlight was changed: {}", out[glint].x);
    }

    #[test]
    fn the_result_does_not_depend_on_the_number_of_threads() {
        let (w, h) = (70, 50);
        let mut f = flat(w, h);
        let mut r = Rng::new(5, 0);
        for i in 0..w * h {
            f.color[i] = Vec3::splat(1.0 + 0.5 * r.f32()) * vec3(1.0, 0.9, 0.7);
            f.variance[i] = 0.02;
            f.depth[i] = 4.0 + (i % w) as f32 * 0.01;
        }
        let a = denoise(&f, 1);
        let b = denoise(&f, 3);
        assert!(a.iter().zip(&b).all(|(x, y)| x.x.to_bits() == y.x.to_bits() && x.y.to_bits() == y.y.to_bits() && x.z.to_bits() == y.z.to_bits()));
    }
}
