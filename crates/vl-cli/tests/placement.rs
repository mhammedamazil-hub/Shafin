//! Placement checked with the renderer's own rays.
//!
//! The language decides where things rest with formulas of its own (bounding boxes, and
//! for `in: bowl` the slope of the vessel's wall). This test does not trust them: it
//! compiles a scene, then fires rays from the middle of every placed object and measures
//! how far away the surrounding surfaces really are.
use vl_core::math::{vec3, Vec3};
use vl_core::rng::Rng;
use vl_lang::compile;
use vl_render::rt::SceneRt;

fn compiled(src: &str) -> vl_lang::Compiled {
    let c = compile("test.vl", src);
    let errors: Vec<String> = c.diags.iter().filter(|d| d.severity == vl_core::diag::Severity::Error).map(|d| format!("{} {}", d.code, d.message)).collect();
    assert!(errors.is_empty(), "unexpected errors:\n{}", errors.join("\n"));
    c
}

/// Smallest distance from `p` to any surface other than object `skip`, over many directions.
fn clearance(rt: &SceneRt, p: Vec3, skip: u32, seed: u64) -> f32 {
    let mut r = Rng::new(seed, 0);
    let mut best = f32::MAX;
    for _ in 0..40_000 {
        // uniform direction on the sphere
        let z = 1.0 - 2.0 * r.f32();
        let phi = std::f32::consts::TAU * r.f32();
        let s = (1.0 - z * z).max(0.0).sqrt();
        let d = vec3(s * phi.cos(), z, s * phi.sin());
        // walk through the object itself until something else is hit
        let mut t0 = 0.0f32;
        for _ in 0..8 {
            match rt.closest(p, d, t0 + 1e-6, 1e9, false) {
                Some(h) if h.obj == skip => t0 = h.t,
                Some(h) => {
                    best = best.min(h.t);
                    break;
                }
                None => break,
            }
        }
    }
    best
}

#[test]
fn things_put_in_vessels_touch_the_inside_and_never_pass_through_it() {
    let c = compiled(
        r#"
        plane ground { }
        box table { size: [2, 0.74, 1], material: wood }
        // a ball smaller than the floor of the bowl: it stands on the floor
        cup bowl { radius: 6cm, top_radius: 12cm, height: 7cm, wall: 5mm, on: table, at: [-0.6, 0] }
        sphere pea { radius: 1.5cm, in: bowl }
        // a ball wider than the floor: it is held by the sloping wall
        cup funnel { radius: 1cm, top_radius: 12cm, height: 10cm, wall: 4mm, floor: 4mm, on: table, at: [-0.2, 0] }
        sphere apple { radius: 4cm, in: funnel }
        // a ball wider than the floor of its dish which still reaches the floor before the wall
        cup dish { radius: 3cm, top_radius: 12cm, height: 6cm, wall: 4mm, floor: 6mm, on: table, at: [-0.2, 0.3] }
        sphere plum { radius: 4cm, in: dish }
        // straight walls, and a vessel that is part of a group which is scaled and turned
        group mug { on: table, at: [0.2, 0], rotate: 40, scale: 1.5
            cup body { radius: 4cm, height: 9cm, wall: 4mm }
            torus handle { radius: 2.5cm, tube: 6mm, rotate: [90, 0, 0], at: [-0.055, 0.045, 0] }
        }
        sphere marble { radius: 1cm, in: mug }
        // a sloping vessel inside a group that is enlarged: the ball hangs on a wall whose size the scale changed
        group goblet { on: table, at: [0.2, 0.3], scale: 1.5, rotate: -70
            cup bowl_part { radius: 1cm, top_radius: 5cm, height: 9cm, wall: 3mm, floor: 3mm }
        }
        sphere cherry { radius: 3cm, in: goblet }
        // "on a plate" means on its inner floor, not hovering at the height of the rim
        cup plate { radius: 7cm, top_radius: 12cm, height: 2cm, wall: 4mm, on: table, at: [0.6, 0] }
        sphere grape { radius: 1.2cm, on: plate }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    for (ball, radius, vessel) in [("pea", 0.015f32, "bowl"), ("apple", 0.04, "funnel"), ("plum", 0.04, "dish"), ("marble", 0.01, "mug.body"), ("cherry", 0.03, "goblet.bowl_part"), ("grape", 0.012, "plate")] {
        let o = c.objects.iter().find(|o| o.name == ball).unwrap();
        let v = c.objects.iter().find(|o| o.name == vessel).unwrap();
        let centre = (o.bounds.min + o.bounds.max) * 0.5;
        let gap = clearance(&rt, centre, o.scene_index.unwrap() as u32, 7) - radius;
        // 40,000 directions do not hit the exact nearest point; on a 1 cm ball they come within microns
        assert!(gap > -2e-4, "{ball} sinks {:.2} mm into {vessel}", -gap * 1000.0);
        assert!(gap < 4e-4, "{ball} hovers {:.2} mm clear of {vessel}", gap * 1000.0);
        // and it is inside: below the rim, above the underside, within the footprint
        assert!(centre.y > v.bounds.min.y && centre.y - radius < v.bounds.max.y, "{ball} is not inside {vessel}");
        assert!(centre.x > v.bounds.min.x && centre.x < v.bounds.max.x && centre.z > v.bounds.min.z && centre.z < v.bounds.max.z);
    }
    // the apple really is the wall-held case: its underside is clear of the funnel's floor
    let apple = c.objects.iter().find(|o| o.name == "apple").unwrap();
    let funnel = c.objects.iter().find(|o| o.name == "funnel").unwrap();
    assert!(apple.bounds.min.y > funnel.bounds.min.y + 0.004 + 0.003, "the apple was meant to hang on the wall, above the floor");
    // none of this is reported as floating or overlapping
    let noise: Vec<&str> = c.diags.iter().filter(|d| d.code == "W320" || d.code == "W321" || d.code == "W322").map(|d| d.message.as_str()).collect();
    assert!(noise.is_empty(), "{noise:?}");
}

#[test]
fn ordinary_relations_leave_no_gap_and_no_overlap() {
    // the same ray check for plain stacking: sphere on box, box on cylinder, cone on sphere
    let c = compiled(
        r#"
        plane ground { }
        box table { size: [1.2, 0.74, 0.8] }
        sphere ball { radius: 9cm, on: table, at: [0.3, 0.1] }
        cylinder can { radius: 5cm, height: 12cm, on: table, at: [-0.3, 0] }
        box book { size: [0.2, 0.03, 0.14], on: can, rotate: 25 }
        sphere bead { radius: 2cm, on: book }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    for (name, radius) in [("ball", 0.09f32), ("bead", 0.02)] {
        let o = c.objects.iter().find(|o| o.name == name).unwrap();
        let centre = (o.bounds.min + o.bounds.max) * 0.5;
        let gap = clearance(&rt, centre, o.scene_index.unwrap() as u32, 3) - radius;
        assert!(gap.abs() < 4e-4, "{name}: {:.3} mm from its support", gap * 1000.0);
    }
}

#[test]
fn several_things_in_one_bowl_pile_up_without_passing_through_each_other() {
    // nothing here has a position: each ball goes where it comes to rest lowest
    let c = compiled(
        r#"
        plane ground { }
        cup bowl { radius: 5cm, top_radius: 14cm, height: 8cm, wall: 5mm }
        for i in 0..6 {
            sphere orange { radius: 3.4cm + i * 1mm, in: bowl, color: orange }
        }
        cup glass { radius: 3.5cm, height: 12cm, wall: 2mm, at: [0.4, 0] }
        for i in 0..3 {
            sphere ice { radius: 1.6cm, in: glass, material: ice }
        }
        cup plate { radius: 9cm, top_radius: 13cm, height: 2cm, wall: 4mm, at: [-0.4, 0] }
        sphere a { radius: 2cm, on: plate }
        sphere b { radius: 2.5cm, on: plate }
        sphere c { radius: 3cm, on: plate }
        box butter { size: [6cm, 2cm, 3cm], on: plate }
        // an egg in an egg cup: too big to go in, so it sits in the opening
        cup eggcup { radius: 1.5cm, top_radius: 2.2cm, height: 4cm, wall: 2mm, at: [0, 0.4] }
        sphere egg { radius: 2.4cm, on: eggcup }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    let mut balls = 0;
    for o in c.objects.iter().filter(|o| o.kind == "sphere") {
        let centre = (o.bounds.min + o.bounds.max) * 0.5;
        let radius = 0.5 * (o.bounds.max.y - o.bounds.min.y);
        let gap = clearance(&rt, centre, o.scene_index.unwrap() as u32, 11) - radius;
        assert!(gap > -2e-4, "{} cuts {:.2} mm into its surroundings", o.name, -gap * 1000.0);
        // each one touches something: the floor, the wall or a neighbour
        // (on the flat of the plate a ball stops at the first free spot of a 12-ring search, so it need not touch its neighbour)
        assert!(gap < 4e-4, "{} hovers {:.2} mm clear of everything", o.name, gap * 1000.0);
        balls += 1;
    }
    assert_eq!(balls, 13);
    // the pile really is a pile: the oranges do not all lie at one height, and none is above the others by more than a ball
    let ys: Vec<f32> = c.objects.iter().filter(|o| o.name.starts_with("orange")).map(|o| o.bounds.min.y).collect();
    let (lo, hi) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
    assert!(hi - lo > 0.005 && hi - lo < 0.09, "undersides range from {lo} to {hi}");
    // the egg sits in the opening: lower than the rim, higher than the inside floor
    let (egg, cup) = (c.objects.iter().find(|o| o.name == "egg").unwrap(), c.objects.iter().find(|o| o.name == "eggcup").unwrap());
    assert!(egg.bounds.min.y < cup.bounds.max.y - 0.003 && egg.bounds.min.y > cup.bounds.min.y + 0.01, "egg underside at {}", egg.bounds.min.y);
    // the block of butter stands on the plate's floor or on top of a ball, never inside one
    let butter = c.objects.iter().find(|o| o.name == "butter").unwrap();
    let bc = (butter.bounds.min + butter.bounds.max) * 0.5;
    assert!(clearance(&rt, bc, butter.scene_index.unwrap() as u32, 5) > 0.0099, "something is inside the butter");
    let noise: Vec<&str> = c.diags.iter().filter(|d| d.code == "W320" || d.code == "W321" || d.code == "W322").map(|d| d.message.as_str()).collect();
    assert!(noise.is_empty(), "{noise:?}");
}

#[test]
fn of_two_surfaces_in_the_same_place_one_is_seen_everywhere() {
    // A cloth written as a box of the table's size (two small models did exactly this), a
    // rug lying in the plane of the floor (the floor is declared after it, on purpose), a
    // sheet of paper and a round mat lying on the table top.
    let c = compiled(
        r#"
        rect rug { size: [3, 2], at: [4, 0, 0], color: red }
        plane ground { }
        box table { size: [1.4, 0.75, 0.8], material: wood }
        box cloth { size: [1.4, 0.75, 0.8], color: green }
        rect paper { size: [0.21, 0.3], on: table, at: [-0.4, 0], color: white }
        disk mat { radius: 0.1, on: cloth, at: [0.4, 0.1], color: black }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    let id = |name: &str| c.objects.iter().find(|o| o.name == name).unwrap().scene_index.unwrap() as u32;
    let mut r = Rng::new(21, 0);
    // rays from many directions onto points of each surface
    let mut check = |name: &str, cx: f32, cy: f32, cz: f32, hx: f32, hz: f32| {
        for _ in 0..4000 {
            let p = vec3(cx + (2.0 * r.f32() - 1.0) * hx, cy, cz + (2.0 * r.f32() - 1.0) * hz);
            let from = p + vec3(4.0 * r.f32() - 2.0, 0.3 + 3.0 * r.f32(), 4.0 * r.f32() - 2.0);
            let hit = rt.closest(from, p - from, 0.0, f32::MAX, true).expect("the ray is aimed at a surface");
            assert_eq!(hit.obj, id(name), "looking at '{name}' at {p:?} from {from:?}: saw '{}'", scene.objects[hit.obj as usize].name);
        }
    };
    check("cloth", 0.0, 0.75, -0.29, 0.65, 0.09); // the table top, away from the paper and the mat
    check("paper", -0.4, 0.75, 0.0, 0.1, 0.14);
    check("mat", 0.4, 0.75, 0.1, 0.06, 0.06);
    check("rug", 4.0, 0.0, 0.0, 1.45, 0.95);
}

#[test]
fn balls_given_nearly_the_same_spot_roll_apart_instead_of_balancing_in_a_tower() {
    // Written by the hosted model in experiment 3 (docs/experiments/llm-run-2.md): four
    // oranges, each 16 cm across, "at" spots 4 cm apart. Taking the spots literally stood
    // them on top of one another, half a metre high.
    let c = compiled(
        r#"
        plane ground { }
        box counter { size: [2, 0.75, 1], on: ground }
        cup bowl { radius: 0.15, top_radius: 0.18, height: 0.1, wall: 0.01, on: counter, at: [0.3, 0] }
        sphere orange1 { radius: 0.08, in: bowl, at: [0.02, 0.02] }
        sphere orange2 { radius: 0.08, in: bowl, at: [-0.02, 0.02] }
        sphere orange3 { radius: 0.08, in: bowl, at: [0.02, -0.02] }
        sphere orange4 { radius: 0.08, in: bowl, at: [-0.02, -0.02] }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    let bowl = c.objects.iter().find(|o| o.name == "bowl").unwrap();
    let mut on_floor = 0;
    for o in c.objects.iter().filter(|o| o.name.starts_with("orange")) {
        let centre = (o.bounds.min + o.bounds.max) * 0.5;
        let gap = clearance(&rt, centre, o.scene_index.unwrap() as u32, 13) - 0.08;
        assert!(gap > -3e-4 && gap < 5e-4, "{}: {:.2} mm from what it rests on", o.name, gap * 1000.0);
        // Nothing towers. Four 16 cm oranges cannot all lie in a bowl 10 cm deep and 33 cm wide:
        // the heap must rise a little above the rim, but only a little (the literal reading
        // put the fourth one 40 cm above it).
        assert!(o.bounds.min.y < bowl.bounds.max.y + 0.06, "{} sits high above the rim of the bowl: underside at {}", o.name, o.bounds.min.y);
        on_floor += (o.bounds.min.y < bowl.bounds.min.y + 0.05) as usize;
    }
    assert!(on_floor >= 2, "only {on_floor} oranges are at the bottom of the bowl");
}

#[test]
fn a_ray_leaving_glass_that_stands_on_a_table_meets_the_table() {
    // The bottom of a glass block and the table top are the same plane. If the ray met the
    // glass first it would leave the glass and continue from a hair beyond that plane, inside
    // the table: the bottom of every glass looked into a black box until this was fixed.
    let c = compiled(
        r#"
        plane ground { }
        box table { size: [1.2, 0.75, 0.8], material: wood }
        cylinder block { radius: 4cm, height: 6cm, on: table, material: glass }
        cup tumbler { radius: 3cm, height: 10cm, wall: 2mm, floor: 8mm, on: table, at: [0.3, 0], material: glass }
        cylinder can { radius: 4cm, height: 6cm, on: table, at: [-0.3, 0], color: red }
    "#,
    );
    let scene = c.scene.as_ref().unwrap();
    let rt = SceneRt::build(scene);
    let id = |name: &str| c.objects.iter().find(|o| o.name == name).unwrap().scene_index.unwrap() as u32;
    let mut r = Rng::new(31, 0);
    for (inside_x, inside_y, reach) in [(0.0f32, 0.78f32, 0.03f32), (0.3, 0.752, 0.02)] {
        // from points inside the glass (the block; the tumbler's floor), downwards at many angles
        for _ in 0..3000 {
            let from = vec3(inside_x + (2.0 * r.f32() - 1.0) * reach * 0.5, inside_y, (2.0 * r.f32() - 1.0) * reach * 0.5);
            let d = vec3((2.0 * r.f32() - 1.0) * 0.3, -1.0, (2.0 * r.f32() - 1.0) * 0.3);
            let hit = rt.closest(from, d, 0.0, f32::MAX, false).unwrap();
            assert_eq!(hit.obj, id("table"), "a ray going down inside the glass met '{}' first", scene.objects[hit.obj as usize].name);
            assert!((from.y + d.y * hit.t - 0.75).abs() < 1e-4, "... at height {}", from.y + d.y * hit.t);
        }
    }
    // an opaque can is unaffected: seen from above, it is the can
    let hit = rt.closest(vec3(-0.3, 2.0, 0.0), vec3(0.0, -1.0, 0.0), 0.0, f32::MAX, true).unwrap();
    assert_eq!(hit.obj, id("can"));
}
