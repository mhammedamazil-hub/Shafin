//! From scene-referred radiance (nits) to a displayable image: exposure, white
//! balance, tone mapping. Deliberately simple, deterministic, per-pixel
//! operations, so re-rendering a region reproduces the rest of the image exactly.
use crate::integrator::Frame;
use vl_core::color::{kelvin, luminance};
use vl_core::dmath;
use vl_core::math::Vec3;
use vl_image::Image;
use vl_ir::scene::{Post, Tonemap};

/// Middle grey: the display value the average scene luminance is mapped to by auto exposure.
pub const KEY: f32 = 0.18;

/// Linear scale that maps radiance in nits to exposed linear values for an exposure value
/// at ISO 100 (saturation-based speed: luminance 1.2 * 2^EV nits maps to 1.0).
#[inline]
pub fn exposure_scale(ev: f32) -> f32 {
    1.0 / (1.2 * dmath::pow(2.0, ev))
}

/// Exposure value that places the geometric-mean luminance of the frame at middle grey.
/// The brightest and darkest 2% of pixels are ignored so that a visible sun, a lamp or a
/// black background does not dominate the estimate. Returns `None` for an all-black frame.
pub fn auto_ev(colors: &[Vec3]) -> Option<f32> {
    let mut logs: Vec<f32> = colors.iter().map(|c| luminance(*c)).filter(|l| *l > 0.0 && l.is_finite()).map(|l| dmath::ln(l)).collect();
    if logs.is_empty() {
        return None;
    }
    logs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let cut = logs.len() / 50;
    let kept = &logs[cut..logs.len() - cut];
    let mean = kept.iter().map(|&v| v as f64).sum::<f64>() / kept.len() as f64;
    let l_avg = dmath::exp(mean as f32);
    // scale = KEY / l_avg  and  scale = 1 / (1.2 * 2^EV)
    Some(dmath::ln(l_avg / (KEY * 1.2)) / dmath::ln(2.0))
}

/// Exposure the way an incident-light meter sets it: from the light falling on the scene,
/// not from the light coming off it.
///
/// A meter that averages what the camera sees turns a snowfield grey and a coal heap grey,
/// because it assumes every scene reflects 18%. A renderer knows each surface's own colour,
/// so it can do better: `radiance / albedo` at a surface is what a white card would look
/// like there. The exposure puts a grey card in the better-lit half of the picture at middle
/// grey, so white things come out white and dark things dark. The better-lit half is used
/// (trimmed of its top 5%) because the subject of a picture is normally in the light, and a
/// lamp-lit scene at night is mostly unlit ground.
///
/// Pixels showing sky, or surfaces with no colour of their own (black, or a light source),
/// are not metered. If fewer than 1 pixel in 20 can be metered the average-luminance
/// method is used instead.
pub fn auto_ev_incident(colors: &[Vec3], albedo: &[Vec3], depth: &[f32]) -> Option<f32> {
    let mut logs: Vec<f32> = Vec::with_capacity(colors.len());
    for ((c, a), d) in colors.iter().zip(albedo).zip(depth) {
        let (l, al) = (luminance(*c), luminance(*a));
        if *d > 0.0 && al > 0.03 && l > 0.0 && l.is_finite() {
            logs.push(dmath::ln(l / al.min(1.0)));
        }
    }
    if logs.len() * 20 < colors.len() {
        return auto_ev(colors);
    }
    logs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (lo, hi) = (logs.len() / 2, (logs.len() * 19 / 20).max(logs.len() / 2 + 1));
    let kept = &logs[lo..hi];
    let mean = kept.iter().map(|&v| v as f64).sum::<f64>() / kept.len() as f64;
    let white = dmath::exp(mean as f32);
    // a grey card there has luminance KEY * white; it should land on KEY:
    // scale = 1 / white  and  scale = 1 / (1.2 * 2^EV)
    Some(dmath::ln(white / 1.2) / dmath::ln(2.0))
}

#[inline]
fn filmic(x: f32) -> f32 {
    // Narkowicz' fit of the ACES reference rendering transform, with the customary 0.6 pre-scale
    let x = x * 0.6;
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// Tone-map one exposed linear colour to display-linear [0, 1].
#[inline]
pub fn tonemap(c: Vec3, op: Tonemap) -> Vec3 {
    match op {
        Tonemap::Linear => c.clamp(0.0, 1.0),
        Tonemap::Filmic => c.max(Vec3::ZERO).map(filmic),
        Tonemap::Reinhard => {
            let l = luminance(c).max(0.0);
            (c * (1.0 / (1.0 + l))).clamp(0.0, 1.0)
        }
    }
}

/// Per-channel gains for a white balance given in kelvin (6500 = neutral), luminance preserving.
pub fn white_balance_gains(k: f32) -> Vec3 {
    if (k - 6500.0).abs() < 1.0 {
        return Vec3::ONE;
    }
    let (w, n) = (kelvin(k), kelvin(6500.0));
    let g = Vec3 { x: n.x / w.x.max(1e-6), y: n.y / w.y.max(1e-6), z: n.z / w.z.max(1e-6) };
    g * (1.0 / luminance(g).max(1e-6))
}

/// Apply exposure, white balance, saturation, contrast and tone mapping.
/// The result is display-linear RGBA (the PNG encoder applies the sRGB transfer curve).
pub fn develop(frame: &Frame, colors: &[Vec3], post: &Post, ev: f32) -> Image {
    let scale = exposure_scale(ev);
    let gains = white_balance_gains(post.white_balance) * scale;
    let mut img = Image::new(frame.w, frame.h);
    for (i, c) in colors.iter().enumerate() {
        let mut v = *c * gains;
        if post.saturation != 1.0 {
            let l = luminance(v);
            v = (Vec3::splat(l) + (v - Vec3::splat(l)) * post.saturation).max(Vec3::ZERO);
        }
        let mut t = tonemap(v, post.tonemap);
        if post.contrast != 1.0 {
            // contrast around middle grey in display-linear space
            t = t.map(|x| (KEY * dmath::pow((x / KEY).max(0.0), post.contrast)).clamp(0.0, 1.0));
        }
        let o = i * 4;
        img.data[o] = t.x;
        img.data[o + 1] = t.y;
        img.data[o + 2] = t.z;
        img.data[o + 3] = 1.0;
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_matches_photographic_practice() {
        // "Sunny 16": a sunlit grey card (18% reflectance under ~100 klx) is ~5700 nits and should
        // land near middle grey at EV 15.
        let card = 0.18 * 100_000.0 / std::f32::consts::PI;
        let v = card * exposure_scale(15.0);
        assert!((0.10..0.20).contains(&v), "sunlit grey card at EV 15 -> {v}");
        // auto exposure puts a uniform frame exactly on middle grey
        let frame = vec![Vec3::splat(card); 100];
        let ev = auto_ev(&frame).unwrap();
        assert!((card * exposure_scale(ev) - KEY).abs() < 1e-3);
        assert!((ev - 14.7).abs() < 0.3, "EV {ev}");
        // one stop more exposure value halves the image
        assert!((exposure_scale(10.0) / exposure_scale(11.0) - 2.0).abs() < 1e-4);
        assert!(auto_ev(&[Vec3::ZERO; 4]).is_none());
        // incident metering: the same light on snow, on a grey card and on coal gives the
        // same exposure, where the reflected-light meter differs by more than four stops
        let lit = 20_000.0f32; // luminance of a white surface in this light
        let scene = |albedo: f32| (vec![Vec3::splat(lit * albedo); 100], vec![Vec3::splat(albedo); 100], vec![1.0f32; 100]);
        let evs: Vec<f32> = [0.9, 0.18, 0.04].iter().map(|&a| { let (c, al, d) = scene(a); auto_ev_incident(&c, &al, &d).unwrap() }).collect();
        assert!((evs[0] - evs[1]).abs() < 1e-3 && (evs[1] - evs[2]).abs() < 1e-3, "{evs:?}");
        let (snow, _, _) = scene(0.9);
        let (coal, _, _) = scene(0.04);
        assert!(auto_ev(&snow).unwrap() - auto_ev(&coal).unwrap() > 4.0);
        // and a white surface then sits at scene-linear 1.0
        assert!((exposure_scale(evs[0]) * lit - 1.0).abs() < 1e-3);
        // sky-only frames fall back to the average
        let sky = vec![Vec3::splat(3000.0); 100];
        assert_eq!(auto_ev_incident(&sky, &vec![Vec3::ONE; 100], &vec![0.0; 100]), auto_ev(&sky));
        // a few extreme pixels (the sun) do not move the estimate
        let mut with_sun = frame.clone();
        with_sun[0] = Vec3::splat(1.6e9);
        assert!((auto_ev(&with_sun).unwrap() - ev).abs() < 0.01);
    }

    #[test]
    fn tone_curves_are_monotonic_and_bounded() {
        for op in [Tonemap::Linear, Tonemap::Filmic, Tonemap::Reinhard] {
            let mut prev = -1.0;
            for i in 0..2000 {
                let x = i as f32 * 0.01;
                let y = tonemap(Vec3::splat(x), op).x;
                assert!((0.0..=1.0).contains(&y) && y >= prev, "{op:?} at {x}");
                prev = y;
            }
            assert_eq!(tonemap(Vec3::ZERO, op), Vec3::ZERO);
        }
        // the filmic curve keeps middle grey in the mid-tones and rolls highlights off below 1
        let g = tonemap(Vec3::splat(KEY), Tonemap::Filmic).x;
        assert!((0.10..0.22).contains(&g), "filmic middle grey {g}");
        assert!(tonemap(Vec3::splat(4.0), Tonemap::Filmic).x < 1.0);
        assert_eq!(white_balance_gains(6500.0), Vec3::ONE);
        let warm = white_balance_gains(3200.0); // correcting tungsten light boosts blue
        assert!(warm.z > warm.x);
    }
}
