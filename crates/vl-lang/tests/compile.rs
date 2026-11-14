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
fn looking_at_a_small_thing_leaves_room_around_it() {
    // Where do the corners of `name` fall in the picture (x and y from -1 to 1), and how
    // tall is it as a fraction of the picture height?
    fn in_picture(c: &vl_lang::Compiled, name: &str) -> (f32, f32) {
        let s = c.scene.as_ref().unwrap();
        let cam = &s.camera;
        let f = (cam.target - cam.pos).normalize();
        let r = f.cross(cam.up).normalize();
        let u = r.cross(f);
        let th = (cam.vfov.to_radians() * 0.5).tan();
        let aspect = s.film.width as f32 / s.film.height as f32;
        let (mn, mx) = bounds(c, name);
        let (mut worst, mut lo, mut hi) = (0.0f32, f32::MAX, f32::MIN);
        for i in 0..8 {
            let p = vec3(if i & 1 == 0 { mn.x } else { mx.x }, if i & 2 == 0 { mn.y } else { mx.y }, if i & 4 == 0 { mn.z } else { mx.z });
            let v = p - cam.pos;
            let z = v.dot(f);
            assert!(z > 0.0);
            let (x, y) = (v.dot(r) / z / (th * aspect), v.dot(u) / z / th);
            worst = worst.max(x.abs()).max(y.abs());
            lo = lo.min(y);
            hi = hi.max(y);
        }
        (worst, (hi - lo) * 0.5)
    }
    let scene = "plane ground { }\nbox table { size: [1.6, 0.75, 0.9] }\nsphere ball { radius: 3cm, on: table }\nsphere other { radius: 3cm, right_of: ball, gap: 2cm }\n";
    // a small object in a larger scene: clearly the subject, with its neighbour wholly in view
    let c = ok(&format!("{scene}camera {{ look_at: ball }}"));
    let (edge, height) = in_picture(&c, "ball");
    assert!(edge < 0.5, "the ball is near the middle: {edge}");
    assert!(height > 0.15 && height < 0.6, "the ball spans {height} of the picture height");
    let (edge, _) = in_picture(&c, "other");
    assert!(edge < 1.0, "the neighbour is cut off: {edge}");
    // without the rule the ball's own bounding box would fill the frame (about 0.9 of the height)
    // the object that is nearly the whole scene is framed whole, and large
    let c = ok(&format!("{scene}camera {{ look_at: table }}"));
    let (edge, height) = in_picture(&c, "table");
    assert!(edge < 1.0 && height > 0.45, "table: edge {edge}, height {height}");
    // an explicit distance is obeyed exactly
    let c = ok(&format!("{scene}camera {{ look_at: ball, distance: 0.5 }}"));
    let s = c.scene.as_ref().unwrap();
    assert!(close((s.camera.target - s.camera.pos).length(), 0.5));
}

#[test]
fn diagnostics_for_typical_mistakes_name_the_fix() {
    let find = |d: &[(String, String, String)], code: &str| d.iter().find(|x| x.0 == code).unwrap_or_else(|| panic!("no {code} in {d:?}")).clone();

    // an unknown property is never fatal: a clear misspelling is read as intended, anything
    // else is ignored, and either way the message says what was done
    let c = ok("sphere s { raduis: 0.2, colour: red, bounce: 3 }");
    let d = codes("sphere s { raduis: 0.2, colour: red, bounce: 3 }");
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d[0].0 == "W345" && d[0].1.contains("'raduis'; it was read as 'radius'"), "{d:?}");
    assert!(d[1].0 == "W345" && d[1].1.contains("'bounce'; it was ignored") && d[1].2.contains("properties of sphere:"), "{d:?}");
    assert!(close(bounds(&c, "s").1.y, 0.4), "the misspelt radius was applied");

    let d = codes("sphare s { radius: 1 }\ntable t { }");
    assert!(find(&d, "E202").2.contains("did you mean 'sphere'?"));
    assert!(d.iter().any(|x| x.1.contains("'table'") && x.2.contains("group name")), "unknown kind explains how to build one: {d:?}");

    // a name nothing resembles is an error, and the hint offers no code to copy
    let d = codes("box t { size: [1,1,1] }\nsphere s { radius: 0.1, on: tabel }");
    let e = find(&d, "E201");
    assert!(e.1.contains("'tabel'") && e.2.contains("nothing called 'tabel' exists above this line") && !e.2.contains("let "), "{e:?}");
    // a misspelling of exactly one name is read as that name
    let c = ok("box table { size: [1,1,1] }\nsphere s { radius: 0.1, on: tabel }");
    assert!(close(bounds(&c, "s").0.y, 1.0), "the ball stands on the table");
    let d = codes("box table { size: [1,1,1] }\nsphere s { radius: 0.1, on: tabel }");
    assert!(find(&d, "W346").1.contains("read as 'table'"));
    // but not when two names are equally close
    let d = codes("sphere ball1 { radius: 0.1 }\nsphere ball2 { radius: 0.1, right_of: ball1 }\nsphere s { radius: 0.1, on: ball3 }");
    assert!(d.iter().any(|x| x.0 == "E201"), "{d:?}");

    // three numbers for 'at' together with a relation: the relation decides the height
    let c = ok("box t { }\nsphere s { radius: 0.1, on: t, at: [0.2, 9, -0.1] }");
    let (mn, mx) = bounds(&c, "s");
    assert!(close(mn.y, 1.0) && close((mn.x + mx.x) * 0.5, 0.2) && close((mn.z + mx.z) * 0.5, -0.1));
    let d = codes("box t { }\nsphere s { radius: 0.1, on: t, at: [0.2, 9, -0.1] }");
    assert!(find(&d, "W343").2.contains("at: [0.2, -0.1]"), "{d:?}");
    // something that is not a position at all is still an error
    let d = codes("box t { }\nsphere s { on: t, at: 3 }");
    assert!(find(&d, "E222").2.contains("on: table, at: [0.2, -0.1]"), "{d:?}");

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
    let d = codes("render { exposure: auto, tonemap: filmic }\nlight l { shape: disk, at: [0, 2, 0], size: [1, 1] }");
    assert!(d.len() == 1 && find(&d, "W311").1.contains("'size' has no effect on a disk light"), "{d:?}");
    // while there is an error only errors are shown: the warning above waits its turn
    let d = codes("render { exposure: auto, tonemap: filmic }\nlight l { shape: disk, at: [0, 2, 0], size: [1, 1] }\nsphere s { on: box }");
    assert!(d.len() == 1 && find(&d, "E204").2.contains("give the object a name"), "{d:?}");
    // a failed property does not trigger follow-on complaints
    let d = codes("light l { shape: rekt, at: [0, 2, 0], size: [1, 1] }");
    assert!(d.len() == 1 && d[0].0 == "W346" && d[0].1.contains("read as 'rect'"), "a near miss is read as intended: {d:?}");
    let d = codes("light l { shape: blob, at: [0, 2, 0], size: [1, 1] }");
    assert!(d.len() == 1 && d[0].0 == "E205" && d[0].2.contains("sphere (bulb), a rect (panel"), "the property explains its own vocabulary: {d:?}");
    let d = codes("render { tonemap: vivid }\nsphere s { }");
    assert!(d.len() == 1 && d[0].0.starts_with('E') && !d[0].2.contains("object created earlier"), "{d:?}");
    let d = codes("light l { shape: \"oval\", at: [0, 2, 0] }");
    assert!(d.len() == 1 && d[0].0 == "E205" && d[0].2.contains("sphere (bulb), a rect (panel"), "{d:?}");

    let d = codes("sphere s { radius: 0.1, on: 0, above: 1 }\nsphere s { }\nmaterial { color: red }");
    assert!(d.iter().any(|x| x.0 == "E221"));
    assert!(d.iter().any(|x| x.0 == "E234"));
    let d = codes("sphere s { radius: 0.1 }\nsphere s { radius: 0.1, at: [1, 0] }");
    assert!(d.iter().any(|x| x.0 == "W301" && x.1.contains("'s_2'")), "{d:?}");

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
    // Two mistakes leave the ball at its default size and position, inside the table. Reporting
    // that as an overlap would point the author at a consequence instead of the cause.
    let d = codes("plane ground { }\nbox table { size: [1.4, 0.75, 0.8] }\nsphere ball { radius: \"small\", on: shelf }");
    let got: Vec<&str> = d.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(got, ["E204", "E201"], "{d:?}");
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
    let c = compile("t", "sphere s { radius: 0.5 }\nbox b { size: \"big\" }");
    assert!(c.scene.is_none() && c.error_count() == 1, "{:?}", c.diags);
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

/// Programs written by small language models in the first experiment (run 43587377319),
/// kept byte for byte. Each failed to compile at the time; the comments say why, and what
/// the language now does instead. These are the most valuable test data in the project:
/// they are what the intended user of the language actually writes.
#[test]
fn programs_written_by_small_models_compile() {
    let load = |name: &str| std::fs::read_to_string(format!("{}/tests/data/llm/{name}.vl", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let warn_codes = |c: &vl_lang::Compiled| c.diags.iter().map(|d| d.code.to_string()).collect::<Vec<_>>();

    // "light sky { sun_elevation: 40, sun_azimuth: 135 }": the sun written as a lamp
    let c = ok(&load("qwen3b_desk_round0"));
    assert_eq!(warn_codes(&c), ["W347"]);
    let Env::Sky(sky) = &c.scene.as_ref().unwrap().env else { panic!("the block was read as the sky") };
    assert!(sky.sun_strength == 1.0 && (sky.sun_dir.y - 40f32.to_radians().sin()).abs() < 1e-4, "daylight at 40 degrees, not night");

    // "material: snow" and "color: dark" were unknown words
    let c = ok(&load("qwen3b_snowman_round0"));
    assert!(!warn_codes(&c).iter().any(|w| w.starts_with("W34")), "nothing had to be guessed: {:?}", c.diags);
    let (b1, b2, b3) = (bounds(&c, "ball1"), bounds(&c, "ball2"), bounds(&c, "ball3"));
    assert!(close(b1.0.y, 0.0) && close(b2.0.y, b1.1.y) && close(b3.0.y, b2.1.y), "three balls stacked on the ground");

    // "on: ground, at: [-0.5, 0.4, 0.3]" and "light environment { strength: 500 }"
    let c = ok(&load("qwen3b_night-room_round0"));
    assert!(warn_codes(&c).contains(&"W343".to_string()));
    let sofa = bounds(&c, "sofa");
    assert!(close(sofa.0.y, 0.0) && close((sofa.0.x + sofa.1.x) * 0.5, -0.5) && close((sofa.0.z + sofa.1.z) * 0.5, 0.3));
    // it has a lamp and says nothing about the sky, so the lamp lights it
    let Env::Sky(sky) = &c.scene.as_ref().unwrap().env else { panic!() };
    assert!(sky.sun_strength < 1e-4, "night");

    // "material: white", "material: black" and "eye1 { at: ... }" for an object that exists
    let c = ok(&load("qwen1_5b_snowman_round0"));
    let w = warn_codes(&c);
    assert!(w.contains(&"W342".to_string()) && w.contains(&"W344".to_string()), "{w:?}");

    // What is still beyond repair: a reply cut off in the middle of a statement, and
    // invented syntax ("let snow = material { ... }"). Both must stay errors.
    for name in ["gptoss20b_desk_round0", "qwen3b_snowman_round1", "qwen1_5b_snowman_round1"] {
        let c = compile(name, &load(name));
        assert!(c.scene.is_none() && c.error_count() >= 1, "{name}");
        assert!(c.diags.iter().all(|d| d.severity == vl_core::diag::Severity::Error), "{name}: only errors are shown while there are errors");
    }
    // nine copies of one mistake are one message
    let c = compile("t", "sphere a { on: zzz }\nsphere b { on: zzz }\nsphere c { on: zzz }");
    assert_eq!(c.diags.len(), 1);
    assert!(c.diags[0].message.ends_with("(also on lines 2, 3)"), "{}", c.diags[0].message);
}

#[test]
fn words_for_light_and_time_of_day() {
    let env = |src: &str| ok(src).scene.unwrap().env;
    let sun_up = |e: &Env| match e {
        Env::Sky(s) => (s.sun_dir.y, s.sun_strength),
        _ => panic!("not a sky"),
    };
    // no lamps, nothing said: daylight
    let (y, k) = sun_up(&env("sphere s { }"));
    assert!(y > 0.5 && k == 1.0);
    // a lamp and nothing said about the sky: the lamp lights the scene
    let (_, k) = sun_up(&env("sphere s { }\nlight l { at: [0, 2, 0] }"));
    assert!(k == vl_lang::lower::NIGHT_SCALE);
    // a lamp and a sky statement: daylight, as asked
    let (_, k) = sun_up(&env("sky { }\nsphere s { }\nlight l { at: [0, 2, 0] }"));
    assert!(k == 1.0);
    // one word for the time of day; explicit numbers win over it
    assert!(sun_up(&env("sky { time: sunset }")).0 < 0.06);
    assert!(sun_up(&env("sky { time: noon }")).0 > 0.85);
    assert!((sun_up(&env("sky { time: sunset, sun_elevation: 30 }")).0 - 0.5).abs() < 1e-4);
    let (_, k) = sun_up(&env("sky { time: night }"));
    assert!(k == vl_lang::lower::NIGHT_SCALE);
    assert!(matches!(env("sky { time: overcast }"), Env::Gradient { .. }));
    // moonlight alone is held two stops under so that night looks like night
    assert_eq!(ok("sky { time: night }").scene.unwrap().post.exposure_bias, -2.0);
    assert_eq!(ok("sky { time: night }\nlight l { at: [0, 2, 0] }").scene.unwrap().post.exposure_bias, 0.0);
    // the sun as its own block, and an unknown word
    assert!(sun_up(&env("sun { elevation: 10 }")).0 < 0.2);
    let d = codes("sky { time: teatime }");
    assert!(d.len() == 1 && d[0].0 == "W348", "{d:?}");
}

#[test]
fn every_css_colour_and_material_word_is_understood() {
    for (name, _) in vl_core::color::NAMED {
        let d = codes(&format!("sphere s {{ color: {name} }}"));
        assert!(d.is_empty(), "colour '{name}': {d:?}");
    }
    assert!(codes("plane ground { }\nsphere s { color: dark_red }\nsphere t { color: LightGrey, at: [2, 0] }").is_empty(), "case and underscores are ignored");
    for (alias, _) in vl_lang::presets::ALIASES {
        let d = codes(&format!("sphere s {{ material: {alias} }}"));
        assert!(d.is_empty(), "material word '{alias}': {d:?}");
    }
    // unknown words are warnings that say what was used instead
    let d = codes("sphere s { color: blurple }");
    assert!(d.len() == 1 && d[0].0 == "W340" && d[0].1.contains("read as 'purple'"), "{d:?}");
    let d = codes("sphere s { color: zzqx }");
    assert!(d.len() == 1 && d[0].0 == "W340" && d[0].1.contains("grey was used"), "{d:?}");
    let d = codes("sphere s { material: wod }");
    assert!(d.len() == 1 && d[0].0 == "W341" && d[0].1.contains("read as 'wood'"), "{d:?}");
    let d = codes("sphere s { material: unobtainium }");
    assert!(d.len() == 1 && d[0].0 == "W341" && d[0].1.contains("plain surface") && d[0].2.contains("snow"), "{d:?}");
}

#[test]
fn hollow_vessels_and_things_put_in_them() {
    let c = ok(r#"
        plane ground { }
        box table { size: [1.2, 0.75, 0.8], material: wood }
        cup bowl { radius: 5cm, top_radius: 12cm, height: 7cm, wall: 5mm, floor: 1cm, on: table }
        // `in` is a relation here and a keyword in the loop: both must work in one program
        for i in 0..3 {
            sphere orange { radius: 3cm, in: bowl, color: orange, material: plastic }
        }
        tube pipe { radius: 10cm, height: 0.5, wall: 1cm, at: [1.5, 0] }
        cup mug { radius: 4cm, height: 9cm, on: table, at: [0.4, 0] }
        sphere sugar { radius: 5mm, inside: mug }
    "#);
    assert_eq!(c.warning_count(), 0, "{:?}", c.diags);
    let scene = c.scene.as_ref().unwrap();
    let shape = |name: &str| {
        let o = c.objects.iter().find(|o| o.name == name).unwrap();
        scene.objects[o.scene_index.unwrap()].shape.clone()
    };
    assert!(matches!(shape("bowl"), Shape::Cup { r0, r1, half_h, wall, floor } if close(r0, 0.05) && close(r1, 0.12) && close(half_h, 0.035) && close(wall, 0.005) && close(floor, 0.01)));
    assert!(matches!(shape("pipe"), Shape::Cup { floor, wall, .. } if floor == 0.0 && close(wall, 0.01)), "a tube is a cup without a floor");
    // defaults: a wall of 8% of the radius and a floor one and a half walls thick
    assert!(matches!(shape("mug"), Shape::Cup { wall, floor, .. } if close(wall, 0.0032) && close(floor, 0.0048)), "{:?}", shape("mug"));
    // the first orange stands on the bowl's inner floor, in the middle
    let (bmin, _) = bounds(&c, "bowl");
    let (omin, omax) = bounds(&c, "orange");
    assert!(close(omin.y, bmin.y + 0.01) && close((omin.x + omax.x) * 0.5, 0.0) && close((omin.z + omax.z) * 0.5, 0.0));
    // the others found room beside it: no two share a centre, and all are inside the bowl's outline
    let centres: Vec<Vec3> = ["orange", "orange_2", "orange_3"].iter().map(|n| (bounds(&c, n).0 + bounds(&c, n).1) * 0.5).collect();
    for i in 0..3 {
        for j in i + 1..3 {
            assert!((centres[i] - centres[j]).length() > 0.0599, "oranges {i} and {j} are {} m apart", (centres[i] - centres[j]).length());
        }
        assert!(vec3(centres[i].x, 0.0, centres[i].z).length() < 0.12 - 0.03);
    }
    // the same colour for all three: the object called `orange` did not hide the colour word
    let albedo = |name: &str| {
        let o = c.objects.iter().find(|o| o.name == name).unwrap();
        match &scene.materials[scene.objects[o.scene_index.unwrap()].material as usize].base_color {
            Param::Const(c) => *c,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(albedo("orange"), albedo("orange_3"));
    assert!(albedo("orange").x > albedo("orange").z * 2.0, "orange, not grey: {:?}", albedo("orange"));
    let (smin, _) = bounds(&c, "sugar");
    assert!(close(smin.y, bounds(&c, "mug").0.y + 0.0048), "'inside' is 'in'");
}

#[test]
fn words_that_are_also_object_names_keep_their_meaning() {
    // An object named after a material or a colour must not capture the word.
    let c = ok(r#"
        plane snow { material: snow }
        box brick { size: [0.2, 0.1, 0.1], material: brick }
        box brick { size: [0.2, 0.1, 0.1], on: brick, material: brick }
        sphere gold { radius: 0.1, material: gold, at: [1, 0] }
        sphere red { radius: 0.1, color: red, at: [-1, 0] }
        sphere red { radius: 0.1, color: red, left_of: red }
    "#);
    // the only remarks are about the repeated names
    let codes: Vec<&str> = c.diags.iter().map(|d| d.code).filter(|c| *c != "W301").collect();
    assert!(codes.is_empty(), "{:?}", c.diags);
    let scene = c.scene.as_ref().unwrap();
    let mat = |name: &str| {
        let o = c.objects.iter().find(|o| o.name == name).unwrap();
        &scene.materials[scene.objects[o.scene_index.unwrap()].material as usize]
    };
    assert_eq!(mat("brick").name, mat("brick_2").name, "both bricks are brick");
    assert!(mat("brick").name.contains("brick") && mat("gold").name.contains("gold"), "{} {}", mat("brick").name, mat("gold").name);
    for n in ["red", "red_2"] {
        assert!(matches!(&mat(n).base_color, Param::Const(c) if c.x > 0.5 && c.y < 0.1), "{n}: {:?}", mat(n).base_color);
    }
    // a variable still wins: the writer defined it
    let c = ok("let red = #00ff00\nsphere s { radius: 0.1, color: red }\nplane g { }");
    let scene = c.scene.as_ref().unwrap();
    let o = c.objects.iter().find(|o| o.name == "s").unwrap();
    match &scene.materials[scene.objects[o.scene_index.unwrap()].material as usize].base_color {
        Param::Const(col) => assert!(col.y > 0.9 && col.x < 0.01, "{col:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn things_too_wide_for_a_vessel_and_vague_relations_are_explained() {
    let d = codes(r#"
        plane ground { }
        cup glass { radius: 3cm, height: 10cm }
        box book { size: [0.2, 0.03, 0.15], in: glass }
        sphere ball { radius: 5cm, next_to: glass }
    "#);
    let w349 = d.iter().find(|x| x.0 == "W349").unwrap_or_else(|| panic!("{d:?}"));
    assert!(w349.1.contains("'glass'") && w349.1.contains("on top instead"), "{w349:?}");
    let w350 = d.iter().find(|x| x.0 == "W350").unwrap_or_else(|| panic!("{d:?}"));
    assert!(w350.1.contains("next_to") && w350.2.contains("left_of"), "{w350:?}");
    assert!(d.iter().all(|x| x.0.starts_with('W')), "no errors: {d:?}");
    // the article fits the noun
    let d = codes("plane g { }\nsphere a { radius: 1 }\nsphere b { radius: a }");
    assert!(d.iter().any(|x| x.1.contains("not an object")), "{d:?}");
}
