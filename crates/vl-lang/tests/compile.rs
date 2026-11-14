//! End-to-end tests of the language: source text in, scene and diagnostics out.
use vl_core::math::{vec3, Vec3};
use vl_ir::expr::Param;
use vl_ir::scene::{Env, Shape};
use vl_lang::compile;

fn ok(src: &str) -> vl_lang::Compiled {
    let c = compile("test.vl", src);
    let errors: Vec<String> = c.diags.iter().filter(|d| d.severity == vl_core::diag::Severity::Error).map(|d| format!("{} {} ({:?})", d.code, d.message, d.hint)).collect();
    assert!(errors.is_empty(), "unexpected errors:\n{}", errors.join("\n"));
    assert!(c.scene.is_some());
    c
}

fn codes(src: &str) -> Vec<(String, String, String)> {
    compile("test.vl", src).diags.iter().map(|d| (d.code.to_string(), d.message.clone(), d.hint.clone().unwrap_or_default())).collect()
}

fn bounds(c: &vl_lang::Compiled, name: &str) -> (Vec3, Vec3) {
    let o = c.objects.iter().find(|o| o.name == name).unwrap_or_else(|| panic!("no object '{name}' in {:?}", c.objects.iter().map(|o| &o.name).collect::<Vec<_>>()));
    (o.bounds.min, o.bounds.max)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn relational_placement_gives_exact_contact() {
    let c = ok(r#"
        plane ground { material: concrete }
        box table { size: [1.6, 0.74, 0.9], material: wood }
        sphere ball { radius: 0.1, on: table, at: [0.3, -0.2], color: #cc2222 }
        cylinder can { radius: 4cm, height: 12cm, right_of: ball, gap: 5cm, material: aluminium }
        box crate { size: 0.4, left_of: table, gap: 0.1 }
        torus ring { radius: 0.2, tube: 0.05, on: crate }
        cone hat { radius: 0.1, height: 0.2, above: ring, gap: 0.5 }
        sphere marble { radius: 2cm, at: [1.5, 0.5] }
    "#);
    assert_eq!(c.warning_count(), 1, "only the deliberately hovering cone is flagged: {:?}", c.diags);
    let (tmin, tmax) = bounds(&c, "table");
    assert!(close(tmin.y, 0.0) && close(tmax.y, 0.74), "an object without a position stands on the floor");
    let (bmin, bmax) = bounds(&c, "ball");
    assert!(close(bmin.y, 0.74), "ball rests exactly on the table top");
    assert!(close((bmin.x + bmax.x) * 0.5, 0.3) && close((bmin.z + bmax.z) * 0.5, -0.2), "offset from the table centre");
    let (cmin, cmax) = bounds(&c, "can");
    assert!(close(cmin.x, bmax.x + 0.05), "gap of 5 cm to the right of the ball");
    assert!(close(cmin.y, bmin.y), "standing on the same level as the ball");
    assert!(close(cmax.y - cmin.y, 0.12));
    let (kmin, kmax) = bounds(&c, "crate");
    assert!(close(kmax.x, tmin.x - 0.1) && close(kmin.y, 0.0));
    let (rmin, rmax) = bounds(&c, "ring");
    assert!(close(rmin.y, kmax.y) && close(rmax.y - rmin.y, 0.1));
    let (hmin, _) = bounds(&c, "hat");
    assert!(close(hmin.y, rmax.y + 0.5));
    let (mmin, mmax) = bounds(&c, "marble");
    assert!(close(mmin.y, 0.0) && close((mmin.x + mmax.x) * 0.5, 1.5) && close((mmin.z + mmax.z) * 0.5, 0.5), "at: [x, z] stands on the floor");
    assert!(c.diags.iter().any(|d| d.code == "W320" && d.message.contains("'hat' floats 0.50 m above 'ring'")));
}

#[test]
fn loops_variables_defines_and_groups() {
    let c = ok(r#"
        let n = 5
        let spacing = 0.3
        plane ground { }
        for i in 0..n {
            sphere bead { radius: 0.05 + i * 0.01, at: [(i - 2) * spacing, 0], color: hsv(i * 60, 0.8, 0.9) }
        }
        define stool(height = 0.45, top: 0.3) {
            cylinder seat { radius: top / 2, height: 3cm, at: [0, height - 0.015, 0], material: wood }
            for k in 0..3 {
                cylinder leg { radius: 1.2cm, height: height - 0.03, at: [cos(radians(k * 120)) * top * 0.35, sin(radians(k * 120)) * top * 0.35] }
            }
        }
        stool s1 { at: [1.5, 0, 0] }
        stool s2 { at: [2.2, 0, 0.3], height: 0.7, rotate: 45 }
        group pile { at: [-2, 0, 0]
            box base { size: [0.5, 0.2, 0.5] }
            box mid { size: [0.3, 0.2, 0.3], on: base }
            sphere top { radius: 0.1, on: mid }
        }
        sphere lid { radius: 0.05, on: s2.seat }
    "#);
    let names: Vec<&str> = c.objects.iter().map(|o| o.name.as_str()).collect();
    for n in ["bead", "bead_2", "bead_5", "s1", "s1.seat", "s1.leg", "s1.leg_3", "s2.seat", "pile", "pile.base", "pile.top", "lid"] {
        assert!(names.contains(&n), "missing '{n}' in {names:?}");
    }
    let scene = c.scene.as_ref().unwrap();
    assert_eq!(scene.objects.len(), 1 + 5 + 4 + 4 + 3 + 1);
    // the taller stool
    let (smin, smax) = bounds(&c, "s2.seat");
    assert!(close(smax.y, 0.7) && close(smin.y, 0.67));
    let (lmin, _) = bounds(&c, "lid");
    assert!(close(lmin.y, 0.7), "an object can rest on a part of a defined object");
    // group contents moved with the group; stacking inside the group is exact
    let (bmin, bmax) = bounds(&c, "pile.base");
    assert!(close((bmin.x + bmax.x) * 0.5, -2.0) && close(bmin.y, 0.0));
    let (tmin, _) = bounds(&c, "pile.top");
    assert!(close(tmin.y, 0.4));
    // bead sizes follow the loop variable
    let (b5min, b5max) = bounds(&c, "bead_5");
    assert!(close(b5max.y - b5min.y, 0.18));
    assert_eq!(c.warning_count(), 0, "{:?}", c.diags);
}

#[test]
fn materials_textures_lights_camera_and_sky() {
    let c = ok(r#"
        render { size: [800, 450], samples: 32, seed: 7, exposure: 9 }
        sky { sun_elevation: 30, sun_azimuth: 180, turbidity: 4 }
        material oak { preset: wood, color: #8a5a33 }
        material stripes { color: mix(#ffffff, #1a1a8a, step(0.5, fract(P.x * 10))), roughness: mix(0.2, 0.7, noise(P * 30)) }
        material neon { emission: #ff3311, emission_strength: 4000 }
        material bumpy { preset: plastic, bump: fbm(P * 40, octaves: 3) * 0.002 }
        plane ground { material: concrete }
        box table { size: [1.2, 0.7, 0.8], material: oak }
        sphere a { radius: 0.1, on: table, at: [-0.3, 0], material: stripes }
        sphere b { radius: 0.1, on: table, material: glass }
        sphere c { radius: 0.1, on: table, at: [0.3, 0], material: gold, color: #ffd0a0 }
        box sign { size: [0.4, 0.1, 0.02], on: table, at: [0, -0.3], material: neon }
        sphere d { radius: 0.08, right_of: c, gap: 2cm, material: bumpy }
        light bulb { at: [0, 2, 0.5], power: 800lm, temperature: 2700, radius: 3cm }
        light panel { shape: rect, size: [0.6, 0.4], at: [-1, 2.2, 1], aim: table, brightness: 5000 }
        camera { look_at: table, from: [2, 1.6, 2.5], fov: 40, f_stop: 2.8, focus: b }
    "#);
    let s = c.scene.as_ref().unwrap();
    assert_eq!((s.film.width, s.film.height, s.film.spp, s.film.seed), (800, 450, 32, 7));
    assert_eq!(s.post.exposure_ev, Some(9.0));
    let Env::Sky(sky) = &s.env else { panic!() };
    // azimuth 180 = sun behind the default camera position (+Z), 30 degrees up
    assert!(sky.sun_dir.z > 0.8 && close(sky.sun_dir.y, 0.5) && sky.sun_dir.x.abs() < 1e-4, "{:?}", sky.sun_dir);
    let mat = |name: &str| s.materials.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("material {name} in {:?}", s.materials.iter().map(|m| &m.name).collect::<Vec<_>>()));
    assert!(matches!(mat("oak").base_color, Param::Expr(..)), "wood stays procedural when tinted");
    assert!(mat("oak").bump.is_some());
    assert!(matches!(mat("stripes").base_color, Param::Expr(..)) && matches!(mat("stripes").roughness, Param::Expr(..)));
    assert_eq!(mat("neon").emission.const_value().unwrap().x, 4000.0, "emission is colour times strength in nits");
    assert!(mat("bumpy").bump.is_some());
    assert_eq!(mat("glass").transmission.const_value().unwrap().x, 1.0);
    let gold = mat("c.gold");
    assert_eq!(gold.metallic.const_value().unwrap().x, 1.0);
    assert!(gold.base_color.const_value().unwrap().x > 0.9, "preset tinted by 'color'");
    // 800 lm from a 3 cm sphere: L = flux / (4 pi^2 r^2)
    let bulb = mat("bulb.light").emission.const_value().unwrap();
    let want = 800.0 / (4.0 * std::f32::consts::PI.powi(2) * 0.03 * 0.03);
    let lum = 0.2126 * bulb.x + 0.7152 * bulb.y + 0.0722 * bulb.z;
    assert!((lum - want).abs() / want < 1e-3, "{lum} vs {want}");
    assert!(bulb.x > bulb.z * 1.5, "2700 K is warm");
    assert_eq!(mat("panel.light").emission.const_value().unwrap(), Vec3::splat(5000.0));
    // the panel was turned to face the table
    let panel = s.objects.iter().find(|o| o.name == "panel").unwrap();
    assert!(matches!(panel.shape, Shape::Rect { .. }));
    let facing = panel.xform.vector(Vec3::Y).normalize();
    let to_table = (vec3(0.0, 0.35, 0.0) - vec3(-1.0, 2.2, 1.0)).normalize();
    assert!(facing.dot(to_table) > 0.999);
    // camera
    assert_eq!(s.camera.pos, vec3(2.0, 1.6, 2.5));
    assert!(close(s.camera.target.y, 0.35) && s.camera.vfov == 40.0);
    assert!(s.camera.lens_radius > 0.004 && s.camera.lens_radius < 0.008, "40 degree lens at f/2.8: {}", s.camera.lens_radius);
    let b = bounds(&c, "b");
    let bc = (b.0 + b.1) * 0.5;
    assert!(close(s.camera.focus, (bc - s.camera.pos).length()));
    for p in &s.programs {
        p.validate().unwrap();
    }
    assert!(s.programs.len() >= 6, "oak (+bump), stripes, concrete (+bump), bumpy bump");
}

#[test]
fn automatic_camera_frames_the_whole_subject() {
    let c = ok("plane ground { }\nbox a { size: [2, 1, 1] }\nsphere b { radius: 0.5, right_of: a, gap: 1 }");
    let s = c.scene.as_ref().unwrap();
    // every corner of the subject's bounds projects inside the frame
    let cam = &s.camera;
    let f = (cam.target - cam.pos).normalize();
    let r = f.cross(cam.up).normalize();
    let u = r.cross(f);
    let th = (cam.vfov.to_radians() * 0.5).tan();
    let aspect = s.film.width as f32 / s.film.height as f32;
    for name in ["a", "b"] {
        let (mn, mx) = bounds(&c, name);
        for i in 0..8 {
            let p = vec3(if i & 1 == 0 { mn.x } else { mx.x }, if i & 2 == 0 { mn.y } else { mx.y }, if i & 4 == 0 { mn.z } else { mx.z });
            let v = p - cam.pos;
            let z = v.dot(f);
            assert!(z > 0.0);
            let (x, y) = (v.dot(r) / z / (th * aspect), v.dot(u) / z / th);
            assert!(x.abs() < 1.0 && y.abs() < 1.0, "corner of '{name}' is outside the frame: ({x}, {y})");
        }
    }
    assert!(cam.pos.y > 0.0, "above the floor");
}

#[test]
fn diagnostics_for_typical_mistakes_name_the_fix() {
    let find = |d: &[(String, String, String)], code: &str| d.iter().find(|x| x.0 == code).unwrap_or_else(|| panic!("no {code} in {d:?}")).clone();

    let d = codes("sphere s { raduis: 0.5, colour: red }");
    assert_eq!(d.len(), 2);
    assert!(d[0].0 == "E203" && d[0].2.contains("did you mean 'radius'?") && d[0].2.contains("properties of sphere:"));
    assert!(d[1].2.contains("did you mean 'color'?"));

    let d = codes("sphare s { radius: 1 }\ntable t { }");
    assert!(find(&d, "E202").2.contains("did you mean 'sphere'?"));
    assert!(d.iter().any(|x| x.1.contains("'table'") && x.2.contains("group name")), "unknown kind explains how to build one: {d:?}");

    let d = codes("box t { size: [1,1,1] }\nsphere s { radius: 0.1, on: tabel }");
    let e = find(&d, "E201");
    assert!(e.1.contains("'tabel'") && e.2.contains("define it first"), "{e:?}");
    let d = codes("box table { size: [1,1,1] }\nsphere s { radius: 0.1, on: tabel }");
    assert!(find(&d, "E201").2.contains("did you mean 'table'?"));

    let d = codes("box t { }\nsphere s { on: t, at: [0, 2, 0] }");
    let e = find(&d, "E222");
    assert!(e.2.contains("on: table, at: [0.2, -0.1]"), "{e:?}");

    let d = codes("sphere s { radius: -1 }\nbox b { size: [1, 0, 1] }\nsphere q { material: 5 }\nsphere r { material: #ff0000 }");
    assert!(find(&d, "E205").1.contains("must be between"));
    assert!(d.iter().any(|x| x.2.contains("color: #rrggbb")), "colour given as a material: {d:?}");

    let d = codes("let table = box(size: [1,1,1])");
    assert!(find(&d, "E206").2.contains("objects are written as blocks"), "{d:?}");

    let d = codes("sphere s { radius: 1 / 0 }\nsphere t { radius: fbmm(P) }\nsphere u { color: noise(P, sed: 3) }");
    assert!(find(&d, "E208").1.contains("division by zero"));
    assert!(find(&d, "E206").2.contains("did you mean 'fbm'?"));
    assert!(find(&d, "E207").2.contains("did you mean 'seed'?"));

    // option words and kind words work unquoted; misuse is explained
    let d = codes("render { exposure: auto, tonemap: filmic }\nlight l { shape: disk, at: [0, 2, 0], size: [1, 1] }\nsphere s { on: box }");
    assert!(!d.iter().any(|x| x.0 == "E201"), "{d:?}");
    assert!(find(&d, "W311").1.contains("'size' has no effect on a disk light"));
    assert!(find(&d, "E204").2.contains("give the object a name"), "{d:?}");
    // a failed property does not trigger follow-on complaints
    let d = codes("light l { shape: rekt, at: [0, 2, 0], size: [1, 1] }");
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].0 == "E201" && d[0].1.contains("'rekt'") && d[0].2.contains("did you mean 'rect'?"), "{d:?}");
    let d = codes("light l { shape: \"oval\", at: [0, 2, 0] }");
    assert!(d.len() == 1 && d[0].0 == "E205" && d[0].2.contains("sphere (bulb), a rect (panel"), "{d:?}");

    let d = codes("sphere s { radius: 0.1, on: 0, above: 1 }\nsphere s { }\nmaterial { color: red }");
    assert!(d.iter().any(|x| x.0 == "E221"));
    assert!(d.iter().any(|x| x.0 == "W301" && x.1.contains("'s_2'")));
    assert!(d.iter().any(|x| x.0 == "E234"));

    let d = codes("plane ground { }\nsphere s { radius: 0.5, at: [0, 0.2, 0] }\nbox a { size: 1, at: [3, 0.5, 0] }\nbox b { size: 1, at: [3.2, 0.5, 0] }\nenvironment { strength: 0 }");
    assert!(find(&d, "W321").1.contains("'s' extends 0.30 m below the surface of 'ground'"));
    assert!(find(&d, "W322").1.contains("'a' and 'b' occupy the same space"));
    assert!(find(&d, "W330").1.contains("no light"));

    let d = codes("box room { size: [4, 3, 4] }\ncamera { from: [0, 1, 0], look_at: [0, 1, -1] }");
    assert!(find(&d, "W331").1.contains("inside 'room'"));
    let d = codes("define a() { a x { } }\na y { }");
    assert!(find(&d, "E235").1.contains("uses itself"));
    let d = codes("for i in 0..10000000 { sphere s { } }");
    assert!(find(&d, "E210").1.contains("limit"));
}

#[test]
fn errors_do_not_trigger_follow_on_warnings() {
    // Two typos leave the ball at its default size and position, inside the table. Reporting that
    // as an overlap would point the author at a consequence instead of the cause.
    let d = codes("plane ground { }\nbox table { size: [1.4, 0.75, 0.8] }\nsphere ball { raduis: 0.1, on: tabel }");
    let got: Vec<&str> = d.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(got, ["E203", "E201"], "{d:?}");
    // the same scene without the typos is clean, and a genuine overlap is still reported
    assert!(codes("plane ground { }\nbox table { size: [1.4, 0.75, 0.8] }\nsphere ball { radius: 0.1, on: table }").is_empty());
    let d = codes("plane ground { }\nbox table { size: [1.4, 0.75, 0.8] }\nsphere ball { radius: 0.5 }");
    assert!(d.len() == 1 && d[0].0 == "W322", "{d:?}");
    // a syntax error elsewhere also holds the semantic warnings back
    let d = codes("plane ground { }\nbox table { size: [1.4, 0.75, 0.8] }\nsphere ball { radius: 0.5 }\nsphere x { radius: }");
    assert!(d.iter().all(|x| x.0.starts_with('E')), "{d:?}");
}

#[test]
fn errors_never_produce_a_scene_and_warnings_do_not_block_one() {
    let c = compile("t", "sphere s { radius: 0.5 }\nbox b { sizee: 1 }");
    assert!(c.scene.is_none() && c.error_count() == 1);
    assert_eq!(c.objects.len(), 2, "objects are still reported for the parts that worked");
    let c = compile("t", "plane ground { }\nsphere s { radius: 0.5, at: [0, 3, 0] }");
    assert!(c.scene.is_some() && c.error_count() == 0 && c.warning_count() == 1);
    // an empty program is a valid (empty) scene under the default sky
    let c = compile("t", "");
    assert!(c.scene.is_some() && c.diags.is_empty());
    assert_eq!(c.scene.unwrap().objects.len(), 0);
}

#[test]
fn hostile_and_random_input_never_panics() {
    let big = "x".repeat(vl_lang::MAX_SOURCE_BYTES + 1);
    assert_eq!(compile("t", &big).diags[0].code, "E001");
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let pieces = [
        "sphere", "box", "s", "{", "}", "(", ")", "[", "]", ":", ",", "radius", "on", "at", "1", "0.5", "-", "+", "*", "/", "let", "for", "in", "..", "if", "else", "define", "group", "material", "#ff0000", "\"x\"", "P", "noise", "fbm", "mix", "camera", "light",
        "look_at", "=", "==", "<", "true", "color", "size", "3cm", ".", "top", "\n", "rotate", "scale", "0", "1e9", "1e-9", "preset", "wood", "glass", "render", "sky", "é", "\u{0}",
    ];
    for _ in 0..3000 {
        let len = (next() % 60) as usize;
        let src: String = (0..len).map(|_| pieces[(next() % pieces.len() as u64) as usize]).collect::<Vec<_>>().join(" ");
        let c = compile("fuzz", &src);
        if let Some(s) = &c.scene {
            s.validate().expect("a returned scene is always valid");
            assert_eq!(c.error_count(), 0);
        } else {
            assert!(c.error_count() > 0, "no scene and no error for: {src}");
        }
    }
}
