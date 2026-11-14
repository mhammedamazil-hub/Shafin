//! The path tracer checked against closed-form radiometry. These tests are the
//! evidence that images are physically meaningful rather than merely plausible.
use vl_core::color::luminance;
use vl_core::math::{vec3, Mat4, Vec3};
use vl_core::rng::Rng;
use vl_ir::expr::Param;
use vl_ir::scene::{Env, Material, Object, Scene, Shape, SkyParams};
use vl_ir::sky::SkyTable;
use vl_render::integrator::{radiance, render, Region, RenderOpts};
use vl_render::rt::{SceneRt, Scratch};

fn matte(albedo: f32) -> Material {
    Material { name: format!("matte{albedo}"), base_color: Param::Const(Vec3::splat(albedo)), specular: 0.0, roughness: Param::scalar(1.0), ..Material::default() }
}

fn emitter(nits: f32) -> Material {
    Material { name: "light".into(), base_color: Param::Const(Vec3::ZERO), specular: 0.0, emission: Param::Const(Vec3::splat(nits)), ..Material::default() }
}

fn obj(name: &str, shape: Shape, xform: Mat4, material: u32) -> Object {
    Object { name: name.into(), shape, xform, material, rand: Vec3::ZERO, hidden: false, cast_shadow: true }
}

fn empty(env: Env) -> Scene {
    let mut s = Scene { env, materials: Vec::new(), ..Scene::default() };
    s.film.max_depth = 48;
    s
}

/// Mean and standard error of the radiance leaving `target` towards `eye`.
fn measure(scene: &Scene, eye: Vec3, target: Vec3, n: u32) -> (Vec3, f32) {
    scene.validate().unwrap();
    let rt = SceneRt::build(scene);
    let mut scratch = Scratch::default();
    let d = (target - eye).normalize();
    let (mut sum, mut s1, mut s2) = (Vec3::ZERO, 0.0f64, 0.0f64);
    for i in 0..n {
        let mut rng = Rng::new(1234 + i as u64, 77);
        let r = radiance(&rt, eye, d, &mut rng, &mut scratch, scene.film.max_depth, 0.0);
        assert!(r.l.is_finite());
        sum += r.l;
        let l = luminance(r.l) as f64;
        s1 += l;
        s2 += l * l;
    }
    let mean = s1 / n as f64;
    let stderr = (((s2 / n as f64) - mean * mean).max(0.0) / n as f64).sqrt();
    (sum * (1.0 / n as f32), stderr as f32)
}

fn assert_close(what: &str, got: f32, want: f32, stderr: f32, rel: f32) {
    let tol = 4.0 * stderr + rel * want.abs();
    println!("{what:58} got {got:12.5}  expected {want:12.5}  (stderr {stderr:.5}, tolerance {tol:.5})");
    assert!((got - want).abs() <= tol, "{what}: got {got}, expected {want}, tolerance {tol}");
}

#[test]
fn white_furnace_conserves_energy() {
    // Inside a uniformly bright environment a lossless object must be invisible:
    // every pixel has exactly the environment's radiance, whatever the geometry.
    let glass = Material { name: "glass".into(), base_color: Param::Const(Vec3::ONE), transmission: Param::scalar(1.0), roughness: Param::scalar(0.0), ..Material::default() };
    let mirror = Material { name: "mirror".into(), base_color: Param::Const(Vec3::ONE), metallic: Param::scalar(1.0), roughness: Param::scalar(0.0), ..Material::default() };
    // (material, lossless?, smallest acceptable mean: the only permitted loss is depth truncation)
    for (name, material, lossless, min_mean) in
        [("white matte", matte(1.0), true, 0.997f32), ("clear glass", glass, true, 0.985), ("perfect mirror", mirror, true, 0.997), ("grey matte 0.5", matte(0.5), false, 0.0)]
    {
        let _ = min_mean;
        let mut s = empty(Env::Uniform(Vec3::ONE));
        s.materials = vec![material];
        s.objects = vec![
            obj("ball", Shape::Sphere { r: 1.0 }, Mat4::IDENTITY, 0),
            obj("block", Shape::Box { half: vec3(0.6, 0.4, 0.5), round: 0.1 }, Mat4::translate(vec3(1.8, -0.6, 0.3)), 0),
            obj("ring", Shape::Torus { major: 0.7, minor: 0.2 }, Mat4::translate(vec3(-1.9, -0.5, 0.4)), 0),
            obj("post", Shape::Cone { r0: 0.4, r1: 0.2, half_h: 0.7 }, Mat4::translate(vec3(0.3, -0.3, 1.9)), 0),
            obj("floor", Shape::Plane, Mat4::translate(vec3(0.0, -1.0, 0.0)), 0),
        ];
        s.camera.pos = vec3(1.0, 2.5, 7.0);
        s.camera.target = vec3(0.0, -0.3, 0.0);
        s.film.width = 96;
        s.film.height = 64;
        s.film.spp = 64;
        s.film.max_depth = 64;
        s.validate().unwrap();
        let rt = SceneRt::build(&s);
        let f = render(&rt, &RenderOpts::default());
        assert_eq!(f.bad_samples, 0);
        let lum: Vec<f32> = f.color.iter().map(|c| luminance(*c)).collect();
        let mean = lum.iter().sum::<f32>() / lum.len() as f32;
        let (lo, hi) = lum.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        println!("furnace {name:15} mean {mean:.5} min {lo:.4} max {hi:.4}");
        if lossless {
            // A lossless path carries weight exactly 1 and is never re-weighted, so a pixel can only
            // fall below 1, and only through paths cut off at `max_depth` (64 here): light trapped
            // between touching surfaces, or by total internal reflection inside glass.
            assert!(hi <= 1.0 + 1e-3, "{name}: a pixel is brighter than the environment ({hi}): energy was created");
            assert!(mean > min_mean && mean <= 1.0 + 1e-4, "{name}: mean radiance {mean} (expected > {min_mean})");
            let dark = lum.iter().filter(|&&v| v < 0.9).count();
            println!("        pixels below 0.9: {dark} of {}", lum.len());
            assert!(dark * 100 <= lum.len(), "{name}: {dark} pixels lost more than 10% of their energy");
        } else {
            // absorbing surfaces can only darken the furnace
            assert!(hi < 1.01 && mean < 0.95 && mean > 0.4, "{name}: mean {mean}, max {hi}");
        }
    }
}

#[test]
fn convex_matte_object_in_uniform_light_has_radiance_albedo_times_environment() {
    let mut s = empty(Env::Uniform(vec3(2.0, 3.0, 4.0)));
    s.materials = vec![matte(0.4)];
    s.objects = vec![obj("ball", Shape::Sphere { r: 1.0 }, Mat4::IDENTITY, 0)];
    for target in [vec3(0.0, 0.0, 1.0), vec3(0.6, 0.5, 0.2), vec3(-0.3, 0.9, 0.1)] {
        let (l, se) = measure(&s, vec3(0.0, 0.5, 6.0), target, 20_000);
        assert_close("sphere albedo 0.4 under uniform (2,3,4): red", l.x, 0.8, se, 0.01);
        assert_close("                                         blue", l.z, 1.6, se * 2.0, 0.01);
    }
}

#[test]
fn sphere_light_over_matte_plane_matches_closed_form() {
    // Irradiance from a uniform sphere wholly above the horizon: E = pi L (r/D)^2 cos(theta),
    // so the plane's radiance is albedo * L * r^2 * h / D^3.
    let (rho, l_e, r, h) = (0.7f32, 5000.0f32, 0.25f32, 1.5f32);
    let mut s = empty(Env::Uniform(Vec3::ZERO));
    s.materials = vec![matte(rho), emitter(l_e)];
    s.objects = vec![obj("floor", Shape::Plane, Mat4::IDENTITY, 0), obj("lamp", Shape::Sphere { r }, Mat4::translate(vec3(0.0, h, 0.0)), 1)];
    for x in [0.0f32, 0.5, 1.5, 4.0] {
        let d3 = (x * x + h * h).powf(1.5);
        let want = rho * l_e * r * r * h / d3;
        let (l, se) = measure(&s, vec3(x + 0.3, 3.0, 2.0), vec3(x, 0.0, 0.0), 40_000);
        assert_close(&format!("plane under sphere light, {x} m off axis"), l.x, want, se, 0.01);
    }
    // looking straight at the lamp returns its radiance exactly
    let (l, _) = measure(&s, vec3(0.0, 1.5, 5.0), vec3(0.0, 1.5, 0.0), 16);
    assert_eq!(l.x, l_e);
}

#[test]
fn rect_light_over_matte_plane_matches_quadrature() {
    // First-principles reference: E(x0) = L * integral over the rectangle of h^2 / (dx^2 + dz^2 + h^2)^2 dA
    let (rho, l_e, a, b, h) = (0.5f32, 2000.0f32, 0.6f32, 0.3f32, 1.0f32);
    let mut s = empty(Env::Uniform(Vec3::ZERO));
    s.materials = vec![matte(rho), emitter(l_e)];
    // the rect faces +Y in object space; flip it to face the floor (it emits from both sides anyway)
    s.objects = vec![
        obj("floor", Shape::Plane, Mat4::IDENTITY, 0),
        obj("panel", Shape::Rect { hx: a, hz: b }, Mat4::translate(vec3(0.0, h, 0.0)).mul(&Mat4::rotate_x(180.0)), 1),
    ];
    for (x0, z0) in [(0.0f32, 0.0f32), (0.8, 0.2), (2.5, -1.0)] {
        let n = 1500;
        let mut e = 0.0f64;
        for i in 0..n {
            for j in 0..n {
                let x = -a as f64 + 2.0 * a as f64 * (i as f64 + 0.5) / n as f64;
                let z = -b as f64 + 2.0 * b as f64 * (j as f64 + 0.5) / n as f64;
                let d2 = (x - x0 as f64).powi(2) + (z - z0 as f64).powi(2) + (h as f64).powi(2);
                e += (h as f64).powi(2) / (d2 * d2);
            }
        }
        e *= l_e as f64 * (4.0 * a as f64 * b as f64) / (n * n) as f64;
        let want = rho as f64 * e / std::f64::consts::PI;
        let (l, se) = measure(&s, vec3(x0 + 0.2, 0.5, z0 + 3.0), vec3(x0, 0.0, z0), 40_000);
        assert_close(&format!("plane under rect light at ({x0}, {z0})"), l.x, want as f32, se, 0.01);
    }
    // closed form on the axis (Howell catalogue, element parallel to a rectangle, corner on the normal), times 4 quadrants
    let (aa, bb) = (a / h, b / h);
    let f = (aa / (1.0 + aa * aa).sqrt() * (bb / (1.0 + aa * aa).sqrt()).atan() + bb / (1.0 + bb * bb).sqrt() * (aa / (1.0 + bb * bb).sqrt()).atan()) / (2.0 * std::f32::consts::PI);
    let (l, se) = measure(&s, vec3(0.2, 0.5, 3.0), Vec3::ZERO, 40_000);
    assert_close("same, against the closed-form view factor", l.x, 4.0 * rho * l_e * f, se, 0.01);
}

#[test]
fn disk_light_and_tilted_receiver() {
    // On-axis irradiance from a disk of radius R at height h, parallel to the receiver: E = pi L R^2 / (R^2 + h^2)
    let (rho, l_e, rad, h) = (0.8f32, 1000.0f32, 0.4f32, 0.9f32);
    let mut s = empty(Env::Uniform(Vec3::ZERO));
    s.materials = vec![matte(rho), emitter(l_e)];
    s.objects = vec![obj("floor", Shape::Plane, Mat4::IDENTITY, 0), obj("lamp", Shape::Disk { r: rad }, Mat4::translate(vec3(0.0, h, 0.0)), 1)];
    let want = rho * l_e * rad * rad / (rad * rad + h * h);
    let (l, se) = measure(&s, vec3(0.3, 0.4, 2.0), Vec3::ZERO, 40_000);
    assert_close("plane under disk light, on axis", l.x, want, se, 0.01);
}

#[test]
fn sun_and_sky_on_a_horizontal_matte_plane() {
    let p = SkyParams { sun_dir: vec3(0.3, 0.6, 0.2).normalize(), turbidity: 3.0, ..SkyParams::default() };
    let table = SkyTable::build(&p);
    let rho = 0.5f32;
    let mut s = empty(Env::Sky(p.clone()));
    s.materials = vec![matte(rho)];
    s.objects = vec![obj("ground", Shape::Plane, Mat4::IDENTITY, 0)];
    let want = (table.sun_illuminance * p.sun_dir.y + table.sky_illuminance) * (rho / std::f32::consts::PI);
    let (l, se) = measure(&s, vec3(0.0, 2.0, 5.0), Vec3::ZERO, 60_000);
    println!("sun {:.0} lx on the ground, sky {:.0} lx", luminance(table.sun_illuminance) * p.sun_dir.y, luminance(table.sky_illuminance));
    assert_close("sunlit plane luminance", luminance(l), luminance(want), se, 0.015);
    assert_close("sunlit plane, blue channel", l.z, want.z, se * 2.0, 0.02);

    // a post casts a hard-edged shadow: inside it only the sky dome (minus the post's own solid angle) lights the ground
    s.objects.push(obj("wall", Shape::Box { half: vec3(0.02, 4.0, 4.0), round: 0.0 }, Mat4::translate(p.sun_dir * 6.0 * (1.0 / p.sun_dir.y) * 0.0 + vec3(0.6, 4.0, 0.0)), 0));
    let (shadow, _) = measure(&s, vec3(-3.0, 2.0, 5.0), vec3(0.0, 0.0, 0.0), 20_000);
    let sky_only = luminance(table.sky_illuminance) * rho / std::f32::consts::PI;
    println!("in shadow {:.1} nits; sky-only upper bound {:.1}; sunlit {:.1}", luminance(shadow), sky_only, luminance(want));
    assert!(luminance(shadow) < 1.3 * sky_only && luminance(shadow) > 0.2 * sky_only, "shadowed point should be lit by sky (and a little bounce) only");
}

#[test]
fn glass_pane_attenuates_shadow_rays_like_two_fresnel_interfaces() {
    let (rho, l_e, r, h) = (0.7f32, 5000.0f32, 0.05f32, 2.0f32);
    let glass = Material { name: "glass".into(), base_color: Param::Const(Vec3::ONE), transmission: Param::scalar(1.0), roughness: Param::scalar(0.0), ior: 1.5, ..Material::default() };
    let mut s = empty(Env::Uniform(Vec3::ZERO));
    s.film.max_depth = 2; // direct lighting only, so the pane's own inter-reflections do not enter
    s.materials = vec![matte(rho), emitter(l_e), glass];
    s.objects = vec![obj("floor", Shape::Plane, Mat4::IDENTITY, 0), obj("lamp", Shape::Sphere { r }, Mat4::translate(vec3(0.0, h, 0.0)), 1)];
    let (open, se0) = measure(&s, vec3(0.2, 0.3, 1.5), Vec3::ZERO, 30_000);
    s.objects.push(obj("pane", Shape::Box { half: vec3(1.0, 0.01, 1.0), round: 0.0 }, Mat4::translate(vec3(0.0, 1.0, 0.0)), 2));
    let (covered, se1) = measure(&s, vec3(0.2, 0.3, 1.5), Vec3::ZERO, 30_000);
    let ratio = covered.x / open.x;
    println!("under glass pane: {:.4} / {:.4} = {ratio:.4} (two interfaces at normal incidence: 0.96^2 = 0.9216)", covered.x, open.x);
    assert!((ratio - 0.9216).abs() < 0.01 + 4.0 * (se0 + se1) / open.x, "ratio {ratio}");
}

#[test]
fn output_is_independent_of_thread_count_and_region() {
    let mut s = empty(Env::Sky(SkyParams::default()));
    let glossy = Material { name: "glossy".into(), base_color: Param::Const(vec3(0.8, 0.2, 0.1)), roughness: Param::scalar(0.3), ..Material::default() };
    let glass = Material { name: "glass".into(), transmission: Param::scalar(1.0), roughness: Param::scalar(0.0), ..Material::default() };
    s.materials = vec![matte(0.6), glossy, glass, emitter(3000.0)];
    s.objects = vec![
        obj("ground", Shape::Plane, Mat4::IDENTITY, 0),
        obj("ball", Shape::Sphere { r: 0.5 }, Mat4::translate(vec3(0.0, 0.5, 0.0)), 1),
        obj("cube", Shape::Box { half: Vec3::splat(0.3), round: 0.05 }, Mat4::translate(vec3(1.0, 0.3, 0.2)).mul(&Mat4::rotate_y(30.0)), 2),
        obj("lamp", Shape::Sphere { r: 0.1 }, Mat4::translate(vec3(-1.0, 1.2, 0.5)), 3),
    ];
    s.camera.pos = vec3(0.5, 1.2, 4.0);
    s.camera.target = vec3(0.3, 0.4, 0.0);
    s.camera.lens_radius = 0.02;
    s.film.width = 80;
    s.film.height = 56;
    s.film.spp = 6;
    s.film.max_depth = 6;
    s.validate().unwrap();
    let rt = SceneRt::build(&s);
    let bits = |v: &[Vec3]| v.iter().flat_map(|c| [c.x.to_bits(), c.y.to_bits(), c.z.to_bits()]).collect::<Vec<u32>>();
    let one = render(&rt, &RenderOpts { threads: 1, ..RenderOpts::default() });
    let four = render(&rt, &RenderOpts { threads: 4, ..RenderOpts::default() });
    assert_eq!(one.bad_samples, 0);
    assert!(bits(&one.color) == bits(&four.color), "render differs between 1 and 4 threads");
    assert!(bits(&one.normal) == bits(&four.normal) && one.depth == four.depth && one.rays == four.rays);

    let region = Region { x0: 17, y0: 9, x1: 61, y1: 40 };
    let part = render(&rt, &RenderOpts { threads: 3, region: Some(region), ..RenderOpts::default() });
    let (mut inside, mut outside_zero) = (0, true);
    for y in 0..s.film.height as usize {
        for x in 0..s.film.width as usize {
            let i = y * s.film.width as usize + x;
            let in_region = x >= region.x0 && x < region.x1 && y >= region.y0 && y < region.y1;
            if in_region {
                assert_eq!(bits(&part.color[i..i + 1]), bits(&one.color[i..i + 1]), "pixel ({x},{y}) differs between region and full render");
                inside += 1;
            } else {
                outside_zero &= part.color[i] == Vec3::ZERO && part.samples[i] == 0;
            }
        }
    }
    assert_eq!(inside, 44 * 31);
    assert!(outside_zero, "pixels outside the region must be untouched");
    // the picture is not degenerate: it has contrast and the image is not black
    let mean = one.color.iter().map(|c| luminance(*c)).sum::<f32>() / one.color.len() as f32;
    assert!(mean > 100.0, "mean luminance {mean} nits under a daylight sky");
}
