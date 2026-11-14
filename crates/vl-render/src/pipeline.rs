//! The complete path from a validated scene to an image.
use crate::integrator::{self, FilmOverride, Frame, RenderOpts};
use crate::post;
use crate::rt::SceneRt;
use vl_core::par;
use vl_image::Image;
use vl_ir::scene::Scene;

pub struct Options<'a> {
    pub threads: usize,
    pub progress: Option<&'a (dyn Fn(f32) + Sync)>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options { threads: par::available_threads(), progress: None }
    }
}

pub struct Output {
    /// Display-linear image, ready for the PNG encoder.
    pub image: Image,
    /// Scene-referred radiance and feature buffers.
    pub frame: Frame,
    /// Exposure value used, and whether it was chosen automatically.
    pub ev: f32,
    pub auto_exposure: bool,
    pub seconds: f64,
    /// Things the renderer could not honour; never silently ignored.
    pub warnings: Vec<String>,
}

/// Longest side of the low-resolution pre-pass used for automatic exposure.
const PREPASS_SIDE: u32 = 160;

/// Exposure for a scene: explicit, or measured with a fast low-resolution pre-pass.
/// The pre-pass depends only on the scene, so the exposure is the same for a full
/// render and for a later region re-render.
pub fn measure_exposure(rt: &SceneRt, threads: usize) -> (f32, bool) {
    let post = &rt.scene.post;
    if let Some(ev) = post.exposure_ev {
        return (ev + post.exposure_bias, false);
    }
    let f = &rt.scene.film;
    let k = (PREPASS_SIDE as f32 / f.width.max(f.height) as f32).min(1.0);
    let (w, h) = (((f.width as f32 * k).round() as u32).max(1), ((f.height as f32 * k).round() as u32).max(1));
    let pre = integrator::render(rt, &RenderOpts { threads, film: FilmOverride { width: Some(w), height: Some(h), spp: Some(16), max_depth: Some(f.max_depth.min(6)) }, ..RenderOpts::default() });
    // an all-black scene has no meaningful exposure; EV 0 keeps the arithmetic finite
    (post::auto_ev(&pre.color).unwrap_or(0.0) + post.exposure_bias, true)
}

pub fn render_scene(scene: &Scene, opts: &Options) -> Output {
    let t0 = std::time::Instant::now();
    let rt = SceneRt::build(scene);
    let mut warnings = Vec::new();
    if scene.fog.is_some() {
        warnings.push("fog is part of the scene but this renderer version does not simulate participating media; the image has no fog".to_string());
    }
    if !rt.unsampled_emitters.is_empty() {
        warnings.push(format!(
            "emissive objects that are not spheres, rects or disks are only found by chance and will be noisy: {}",
            rt.unsampled_emitters.join(", ")
        ));
    }
    for (name, on) in [("bloom", scene.post.bloom > 0.0), ("vignette", scene.post.vignette > 0.0), ("grain", scene.post.grain > 0.0)] {
        if on {
            warnings.push(format!("post-processing '{name}' is not implemented yet and was not applied"));
        }
    }
    let (ev, auto_exposure) = measure_exposure(&rt, opts.threads);
    let clamp = if scene.film.clamp_indirect > 0.0 { scene.film.clamp_indirect / post::exposure_scale(ev) } else { 0.0 };
    let frame = integrator::render(&rt, &RenderOpts { threads: opts.threads, clamp, progress: opts.progress, ..RenderOpts::default() });
    if frame.bad_samples > 0 {
        warnings.push(format!("{} samples were not finite and were discarded", frame.bad_samples));
    }
    let image = post::develop(&frame, &frame.color, &scene.post, ev);
    Output { image, frame, ev, auto_exposure, seconds: t0.elapsed().as_secs_f64(), warnings }
}
