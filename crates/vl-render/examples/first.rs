//! Renders a small hard-coded scene. Used while developing the renderer; real scenes come from VL programs.
use vl_core::math::{vec3, Mat4, Vec3};
use vl_ir::expr::Param;
use vl_ir::scene::{Env, Material, Object, Scene, Shape, SkyParams};
use vl_render::pipeline::{render_scene, Options};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or_else(|| "out/first.png".into());
    let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(640);
    let spp: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(64);

    let c = |r: f32, g: f32, b: f32| Param::Const(vec3(r, g, b));
    let mut s = Scene { env: Env::Sky(SkyParams { sun_dir: vec3(-0.45, 0.55, 0.5).normalize(), turbidity: 3.0, ..SkyParams::default() }), materials: Vec::new(), ..Scene::default() };
    s.materials = vec![
        Material { name: "ground".into(), base_color: c(0.55, 0.52, 0.47), roughness: Param::scalar(0.9), specular: 0.3, ..Material::default() },
        Material { name: "red".into(), base_color: c(0.75, 0.05, 0.04), roughness: Param::scalar(0.25), ..Material::default() },
        Material { name: "glass".into(), base_color: c(1.0, 1.0, 1.0), transmission: Param::scalar(1.0), roughness: Param::scalar(0.0), ior: 1.5, ..Material::default() },
        Material { name: "gold".into(), base_color: c(1.0, 0.77, 0.34), metallic: Param::scalar(1.0), roughness: Param::scalar(0.2), ..Material::default() },
        Material { name: "ceramic".into(), base_color: c(0.9, 0.9, 0.88), roughness: Param::scalar(0.6), coat: 1.0, coat_roughness: 0.05, ..Material::default() },
    ];
    let obj = |name: &str, shape: Shape, xform: Mat4, material: u32| Object { name: name.into(), shape, xform, material, rand: Vec3::ZERO, hidden: false, cast_shadow: true };
    s.objects = vec![
        obj("ground", Shape::Plane, Mat4::IDENTITY, 0),
        obj("red_ball", Shape::Sphere { r: 0.5 }, Mat4::translate(vec3(-1.1, 0.5, 0.0)), 1),
        obj("glass_ball", Shape::Sphere { r: 0.45 }, Mat4::translate(vec3(0.0, 0.45, 0.6)), 2),
        obj("gold_block", Shape::Box { half: vec3(0.4, 0.4, 0.4), round: 0.06 }, Mat4::translate(vec3(1.1, 0.4, -0.1)).mul(&Mat4::rotate_y(28.0)), 3),
        obj("ring", Shape::Torus { major: 0.45, minor: 0.14 }, Mat4::translate(vec3(0.1, 0.14, -1.2)), 4),
    ];
    s.camera.pos = vec3(0.4, 1.5, 4.6);
    s.camera.target = vec3(0.0, 0.4, 0.0);
    s.camera.vfov = 32.0;
    s.camera.lens_radius = 0.012;
    s.camera.focus = (s.camera.target - s.camera.pos).length();
    s.film.width = width;
    s.film.height = width * 9 / 16;
    s.film.spp = spp;
    s.validate().expect("valid scene");

    let last = std::sync::atomic::AtomicU32::new(0);
    let progress = |f: f32| {
        let pct = (f * 10.0) as u32;
        if last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
            eprint!("{}% ", pct * 10);
        }
    };
    let o = render_scene(&s, &Options { progress: Some(&progress), ..Options::default() });
    eprintln!();
    let png = vl_image::png::encode_png(&o.image, false, &vl_image::png::PngOptions::default());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&out, &png).expect("write png");
    let px = (s.film.width * s.film.height) as f64;
    println!(
        "{}x{} @ {} spp: {:.2} s, {:.2} M rays ({:.2} M rays/s), EV {:.2} ({}), {} bytes -> {}",
        s.film.width,
        s.film.height,
        spp,
        o.seconds,
        o.frame.rays as f64 / 1e6,
        o.frame.rays as f64 / 1e6 / o.seconds,
        o.ev,
        if o.auto_exposure { "auto" } else { "manual" },
        png.len(),
        out
    );
    println!("rays per pixel-sample: {:.2}; warnings: {:?}", o.frame.rays as f64 / (px * spp as f64), o.warnings);
    // Speckle metric: pixels much brighter than the median of their 3x3 neighbourhood (display-linear luminance).
    let (w, h) = (o.image.w, o.image.h);
    let lum = |x: usize, y: usize| {
        let p = o.image.get(x, y);
        0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
    };
    let (mut fireflies, mut total) = (0usize, 0.0f64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut n = [0.0f32; 8];
            let mut k = 0;
            for dy in 0..3 {
                for dx in 0..3 {
                    if dx != 1 || dy != 1 {
                        n[k] = lum(x + dx - 1, y + dy - 1);
                        k += 1;
                    }
                }
            }
            n.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = 0.5 * (n[3] + n[4]);
            let v = lum(x, y);
            total += v as f64;
            if v > 1.4 * median + 0.04 {
                fireflies += 1;
            }
        }
    }
    println!("speckle pixels: {fireflies} of {} ({:.3}%); mean display luminance {:.5}", w * h, 100.0 * fireflies as f64 / (w * h) as f64, total / ((w - 2) * (h - 2)) as f64);
}
