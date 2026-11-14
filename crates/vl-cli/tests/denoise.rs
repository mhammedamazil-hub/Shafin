//! The denoiser measured on real renders: a picture made with few samples and then
//! filtered must be closer to the converged picture than the unfiltered one, and must
//! not be brighter or darker than it.
use vl_core::color::luminance;
use vl_lang::compile;
use vl_render::denoise::denoise;
use vl_render::integrator::{self, FilmOverride, RenderOpts};
use vl_render::rt::SceneRt;

const SCENES: [(&str, &str); 2] = [
    (
        "lamp at night",
        r#"
        render { size: [180, 120] }
        plane ground { material: plaster }
        box table { size: [1.2, 0.5, 0.8], material: wood }
        cup bowl { radius: 6cm, top_radius: 14cm, height: 8cm, on: table, color: white }
        for i in 0..3 { sphere fruit { radius: 3.5cm, in: bowl, color: orange } }
        sphere ball { radius: 0.12, right_of: table, gap: 0.2, color: #3366cc, material: plastic }
        light lamp { at: [0.6, 1.4, 0.7], power: 1500, temperature: 3000 }
        camera { look_at: table, azimuth: 25, elevation: 25 }
    "#,
    ),
    (
        "sunlit, with deep shade",
        r#"
        render { size: [180, 120] }
        sky { time: afternoon, sun_azimuth: 220 }
        plane ground { material: concrete }
        box wall { size: [2, 1.2, 0.2], at: [0, -0.4] }
        box crate { size: 0.4, at: [0.3, 0.4], color: #a06030 }
        sphere ball { radius: 0.2, at: [-0.5, 0.5], color: white }
        camera { look_at: crate, azimuth: 20, elevation: 20 }
    "#,
    ),
];

#[test]
fn filtered_low_sample_renders_are_closer_to_the_converged_picture() {
    for (name, src) in SCENES {
        let c = compile("test.vl", src);
        assert!(c.scene.is_some(), "{name}: {:?}", c.diags);
        let scene = c.scene.as_ref().unwrap();
        let rt = SceneRt::build(scene);
        // the same settings as `vl render`: exposure first, then the indirect-light clamp that follows from it
        let (ev, _) = vl_render::pipeline::measure_exposure(&rt, 2);
        let clamp = scene.film.clamp_indirect / vl_render::post::exposure_scale(ev);
        let render = |spp: u32| integrator::render(&rt, &RenderOpts { threads: 2, clamp, film: FilmOverride { spp: Some(spp), ..FilmOverride::default() }, ..RenderOpts::default() });
        let reference = render(512);
        let noisy = render(16);
        let clean = denoise(&noisy, 2);
        // error relative to the local brightness, so that dark areas count as much as bright ones
        let white = reference.color.iter().map(|c| luminance(*c)).sum::<f32>() / reference.color.len() as f32;
        let err = |img: &[vl_core::math::Vec3]| -> f64 {
            img.iter().zip(&reference.color).map(|(a, r)| ((luminance(*a) - luminance(*r)) / (luminance(*r) + 0.05 * white)) as f64).map(|e| e * e).sum::<f64>() / img.len() as f64
        };
        let (e_noisy, e_clean) = (err(&noisy.color), err(&clean));
        let mean = |img: &[vl_core::math::Vec3]| img.iter().map(|c| luminance(*c) as f64).sum::<f64>() / img.len() as f64;
        let shift = mean(&clean) / mean(&reference.color) - 1.0;
        println!("{name}: relative squared error {e_noisy:.5} -> {e_clean:.5} ({:.1}x), brightness {:+.2}%", e_noisy / e_clean, shift * 100.0);
        // (what is left is mostly the one-pixel outlines of objects, where two surfaces share a pixel
        // and the filter leaves the pixel alone; on larger pictures the gain is larger)
        assert!(e_clean < e_noisy / 1.5, "{name}: error {e_noisy} -> {e_clean}");
        assert!(shift.abs() < 0.02, "{name}: the filtered picture is {:+.1}% brighter than the converged one", shift * 100.0);
    }
}
