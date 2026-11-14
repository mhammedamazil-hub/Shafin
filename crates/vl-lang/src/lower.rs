//! From the evaluated scene graph to a validated `vl_ir::scene::Scene`, plus the
//! semantic checks that are possible before rendering.
use crate::eval::*;
use crate::presets::{MatSpec, PV};
use crate::value::{make_op, Node, ProgramBuilder, Value};
use std::collections::HashMap;
use std::rc::Rc;
use vl_core::diag::{Diag, Span};
use vl_core::dmath;
use vl_core::math::{vec3, Aabb, Vec3};
use vl_core::rng;
use vl_ir::expr::{Op, Param};
use vl_ir::scene::*;

/// What the compiler knows about one named thing in the scene (for reports and edits).
#[derive(Clone, Debug)]
pub struct ObjectInfo {
    /// Unique dotted name, e.g. `chair.seat`.
    pub name: String,
    pub kind: String,
    /// 1-based source line of the statement that created it.
    pub line: usize,
    /// World-space bounds.
    pub bounds: Aabb,
    pub material: Option<String>,
    pub is_group: bool,
    pub is_light: bool,
    pub parent: Option<String>,
    /// Index into `Scene::objects` for leaves.
    pub scene_index: Option<usize>,
}

fn param(pv: &PV, b: &mut Option<ProgramBuilder>, program_index: u32) -> Param {
    match pv {
        PV::Const(c) => Param::Const(*c),
        PV::Tex(n) => {
            let builder = b.get_or_insert_with(ProgramBuilder::default);
            Param::Expr(program_index, builder.compile(n))
        }
    }
}

fn lower_material(m: &MatSpec, programs: &mut Vec<vl_ir::expr::Program>) -> Material {
    let pi = programs.len() as u32;
    let mut b: Option<ProgramBuilder> = None;
    let base_color = param(&m.base, &mut b, pi);
    let metallic = param(&m.metallic, &mut b, pi);
    let roughness = param(&m.roughness, &mut b, pi);
    let transmission = param(&m.transmission, &mut b, pi);
    // emitted radiance = colour * strength (nits)
    let emission = match &m.emission {
        PV::Const(c) => Param::Const(*c * m.emission_strength),
        PV::Tex(n) => {
            let scaled = make_op(Op::Mul, vec![n.clone(), Rc::new(Node::Const(Vec3::splat(m.emission_strength)))]);
            let builder = b.get_or_insert_with(ProgramBuilder::default);
            Param::Expr(pi, builder.compile(&scaled))
        }
    };
    if let Some(builder) = b {
        programs.push(builder.program);
    }
    // the height field gets its own program: it is evaluated three times per shading point
    let bump = m.bump.as_ref().map(|n| {
        let mut bb = ProgramBuilder::default();
        let r = bb.compile(n);
        programs.push(bb.program);
        (programs.len() as u32 - 1, r)
    });
    Material {
        name: m.name.clone(),
        base_color,
        metallic,
        roughness,
        specular: m.specular,
        ior: m.ior,
        transmission,
        coat: m.coat,
        coat_roughness: m.coat_roughness,
        emission,
        sheen: m.sheen,
        bump,
        bump_strength: m.bump_strength,
        absorb_color: m.absorb_color,
        absorb_dist: m.absorb_dist,
        thin: m.thin,
    }
}

/// Unit vector towards the sun. Azimuth is a compass bearing with north = -Z and east = +X,
/// so with the default camera (which stands on the +Z side looking towards -Z) azimuth 180
/// puts the sun behind the camera, 90 to its right, 270 to its left and 0 in front of it.
pub fn sun_direction(elevation_deg: f64, azimuth_deg: f64) -> Vec3 {
    let (se, ce) = dmath::sincos(dmath::radians(elevation_deg as f32));
    let (sa, ca) = dmath::sincos(dmath::radians(azimuth_deg as f32));
    vec3(sa * ce, se, -ca * ce).normalize()
}

pub struct Lowered {
    pub scene: Scene,
    pub objects: Vec<ObjectInfo>,
}

pub fn lower(ev: &mut Ev, line_of: &dyn Fn(Span) -> usize) -> Option<Lowered> {
    let mut scene = Scene { materials: Vec::new(), ..Scene::default() };
    let r = &ev.render;
    scene.film = Film { width: r.width, height: r.height, spp: r.samples, min_spp: (r.samples / 4).max(8).min(r.samples), noise_target: r.noise_target, max_depth: r.max_depth, seed: r.seed, reflective_caustics: r.caustics, ..Film::default() };
    scene.post = Post { exposure_ev: r.exposure, exposure_bias: r.exposure_bias, tonemap: r.tonemap, white_balance: r.white_balance, saturation: r.saturation, contrast: r.contrast, ..Post::default() };
    scene.env = match ev.env.clone() {
        None => Env::Sky(SkyParams { sun_dir: sun_direction(40.0, 135.0), ..SkyParams::default() }),
        Some(EnvSpec::Sky { elevation, azimuth, turbidity, sun_size, sun_strength, sky_strength, ground }) => Env::Sky(SkyParams {
            sun_dir: sun_direction(elevation, azimuth),
            turbidity: turbidity as f32,
            ground_albedo: ground,
            sun_size: sun_size as f32,
            sun_strength: sun_strength as f32,
            sky_strength: sky_strength as f32,
        }),
        Some(EnvSpec::Uniform(c)) => Env::Uniform(c),
        Some(EnvSpec::Gradient { top, horizon, bottom }) => Env::Gradient { zenith: top, horizon, ground: bottom },
    };

    // ---- objects and the materials they use
    let mut mat_map: HashMap<usize, u32> = HashMap::new();
    let mut infos: Vec<ObjectInfo> = Vec::new();
    for i in 0..ev.nodes.len() {
        let n = &ev.nodes[i];
        let bounds = ev.world_bounds(i);
        let parent = n.parent.map(|p| ev.nodes[p].path.clone());
        match &n.kind {
            SKind::Group => infos.push(ObjectInfo { name: n.path.clone(), kind: n.kind_name.clone(), line: line_of(n.span), bounds, material: None, is_group: true, is_light: false, parent, scene_index: None }),
            SKind::Shape { shape, material } => {
                let mi = match mat_map.get(material) {
                    Some(&m) => m,
                    None => {
                        let lowered = lower_material(&ev.mats[*material], &mut scene.programs);
                        scene.materials.push(lowered);
                        let m = scene.materials.len() as u32 - 1;
                        mat_map.insert(*material, m);
                        m
                    }
                };
                let h = rng::hash2(n.path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |a, b| (a ^ b as u64).wrapping_mul(0x100_0000_01b3)), 17);
                let rand = vec3(rng::unit_f32(h), rng::unit_f32(rng::mix64(h)), rng::unit_f32(rng::mix64(rng::mix64(h))));
                infos.push(ObjectInfo {
                    name: n.path.clone(),
                    kind: n.kind_name.clone(),
                    line: line_of(n.span),
                    bounds,
                    material: Some(ev.mats[*material].name.clone()),
                    is_group: false,
                    is_light: n.is_light,
                    parent,
                    scene_index: Some(scene.objects.len()),
                });
                scene.objects.push(Object { name: n.path.clone(), shape: shape.clone(), xform: ev.world_xform(i), material: mi, rand, hidden: n.hidden, cast_shadow: n.cast_shadow });
            }
        }
    }
    if scene.materials.is_empty() {
        scene.materials.push(Material { name: "default".into(), ..Material::default() });
    }

    // ---- camera
    let is_plane = |i: usize| matches!(ev.nodes[i].kind, SKind::Shape { shape: Shape::Plane, .. });
    let mut subject = Aabb::EMPTY;
    for i in 0..ev.nodes.len() {
        if ev.nodes[i].parent.is_none() && !is_plane(i) && !ev.nodes[i].is_light {
            subject = subject.union(&ev.world_bounds(i));
        }
    }
    if subject.is_empty() {
        // only lights or planes: frame the lights, or a unit region at the origin
        for i in 0..ev.nodes.len() {
            if ev.nodes[i].parent.is_none() && !is_plane(i) {
                subject = subject.union(&ev.world_bounds(i));
            }
        }
    }
    if subject.is_empty() {
        subject = Aabb::new(vec3(-0.5, 0.0, -0.5), vec3(0.5, 1.0, 0.5));
    }
    let spec = ev.camera.clone().unwrap_or(CameraSpec { look_at: None, from: None, distance: None, azimuth: 30.0, elevation: 18.0, fov: 35.0, f_stop: None, focus: None, blades: 0, span: Span::new(0, 0) });
    let mut framed = subject;
    let mut target = subject.center();
    if let Some((v, vspan)) = &spec.look_at {
        match v {
            Value::Object(i) => {
                framed = ev.world_bounds(*i);
                if is_plane(*i) {
                    framed = subject;
                }
                target = framed.center();
            }
            Value::Vec3(p) if p.is_finite() => target = *p,
            _ => ev.diags.push(Diag::error("E204", *vspan, format!("'look_at' must be an object or a point [x, y, z], not a {}", v.type_name())).hint("for example  look_at: table")),
        }
    }
    let aspect = r.width as f32 / r.height as f32;
    let vfov = spec.fov as f32;
    let half_v = dmath::radians(vfov) * 0.5;
    let half_h = dmath::atan2(dmath::tan(half_v) * aspect, 1.0);
    let pos = match spec.from {
        Some(p) => p,
        None => {
            let radius = (framed.size().length() * 0.5).max(0.05);
            let auto = radius / dmath::sin(half_v.min(half_h)) * 1.08;
            let d = spec.distance.map(|d| d as f32).unwrap_or(auto);
            let (se, ce) = dmath::sincos(dmath::radians(spec.elevation as f32));
            let (sa, ca) = dmath::sincos(dmath::radians(spec.azimuth as f32));
            target + vec3(sa * ce, se, ca * ce) * d
        }
    };
    let to_target = (target - pos).length();
    if to_target < 1e-5 {
        ev.diags.push(Diag::error("E240", spec.span, "the camera is at the very point it looks at").hint("move 'from' away from 'look_at', or omit 'from' to have the camera placed automatically"));
        return None;
    }
    let mut up = Vec3::Y;
    if ((target - pos) * (1.0 / to_target)).cross(up).length() < 1e-3 {
        up = Vec3::Z; // looking straight up or down
    }
    let focus = match &spec.focus {
        None => to_target,
        Some((Value::Num(d), _)) if *d > 0.0 => *d as f32,
        Some((Value::Object(i), _)) => (ev.world_bounds(*i).center() - pos).length().max(1e-3),
        Some((Value::Vec3(p), _)) => (*p - pos).length().max(1e-3),
        Some((v, fspan)) => {
            ev.diags.push(Diag::error("E204", *fspan, format!("'focus' must be an object, a point or a distance in metres, not a {}", v.type_name())).hint("for example  focus: vase"));
            to_target
        }
    };
    // full-frame camera: sensor 24 mm tall; lens radius = focal length / (2 * f-number)
    let lens_radius = spec.f_stop.map(|n| (0.012 / dmath::tan(half_v)) / (2.0 * n as f32)).unwrap_or(0.0);
    scene.camera = Camera { pos, target, up, vfov, lens_radius, focus, blades: spec.blades };

    semantic_checks(ev, &scene, &infos);

    if let Err(errs) = scene.validate() {
        for e in errs {
            ev.diags.push(Diag::error("E900", Span::new(0, 0), format!("the scene is not valid: {e}")).hint("this usually follows from an earlier error; fix those first"));
        }
        return None;
    }
    Some(Lowered { scene, objects: infos })
}

/// Checks that need no rendering: things a person would call obviously wrong.
/// They are warnings, because floating or intersecting objects are sometimes intended.
fn semantic_checks(ev: &mut Ev, scene: &Scene, infos: &[ObjectInfo]) {
    let top: Vec<usize> = (0..ev.nodes.len()).filter(|&i| ev.nodes[i].parent.is_none()).collect();
    let is_plane = |ev: &Ev, i: usize| matches!(ev.nodes[i].kind, SKind::Shape { shape: Shape::Plane, .. });
    let bounds: HashMap<usize, Aabb> = top.iter().map(|&i| (i, ev.world_bounds(i))).collect();
    let xz_overlap = |a: &Aabb, b: &Aabb| a.min.x < b.max.x && b.min.x < a.max.x && a.min.z < b.max.z && b.min.z < a.max.z;
    let mut out: Vec<Diag> = Vec::new();

    if top.len() <= 400 {
        for &i in &top {
            if is_plane(ev, i) || ev.nodes[i].is_light {
                continue;
            }
            let b = bounds[&i];
            let tol = 0.01f32.max(0.01 * b.size().y);
            // highest surface at or below this object's base
            let mut support: Option<(usize, f32)> = None;
            let mut sunk: Option<(usize, f32)> = None;
            for &j in &top {
                if j == i || ev.nodes[j].is_light {
                    continue;
                }
                let o = bounds[&j];
                if is_plane(ev, j) {
                    let y = o.max.y;
                    if b.min.y < y - tol && b.max.y > y {
                        sunk = Some((j, y - b.min.y));
                    }
                    if y <= b.min.y + tol && support.map(|s| y > s.1).unwrap_or(true) {
                        support = Some((j, y));
                    }
                } else if xz_overlap(&b, &o) && o.max.y <= b.min.y + tol && support.map(|s| o.max.y > s.1).unwrap_or(true) {
                    support = Some((j, o.max.y));
                }
            }
            let (name, span) = (ev.nodes[i].path.clone(), ev.nodes[i].span);
            if let Some((j, depth)) = sunk {
                out.push(Diag::warning("W321", span, format!("'{name}' extends {depth:.2} m below the surface of '{}'", ev.nodes[j].path)).hint(format!("to stand it on the surface write  on: {}", ev.nodes[j].path)));
                continue;
            }
            match support {
                Some((j, y)) if b.min.y - y > tol => {
                    out.push(Diag::warning("W320", span, format!("'{name}' floats {:.2} m above '{}' (nothing touches its underside)", b.min.y - y, ev.nodes[j].path)).hint(format!("to rest it there write  on: {}  - ignore this if it is meant to hang in the air", ev.nodes[j].path)));
                }
                None if top.iter().any(|&j| j != i && !ev.nodes[j].is_light) => {
                    out.push(Diag::warning("W320", span, format!("'{name}' has nothing underneath it")).hint("add a floor (plane ground { }) or place it with  on: some_object  - ignore this if it is meant to hang in the air"));
                }
                _ => {}
            }
        }
        // strongly overlapping bounding boxes
        for (a, &i) in top.iter().enumerate() {
            for &j in &top[a + 1..] {
                if is_plane(ev, i) || is_plane(ev, j) || ev.nodes[i].is_light || ev.nodes[j].is_light {
                    continue;
                }
                let (bi, bj) = (bounds[&i], bounds[&j]);
                let ov = bi.overlap(&bj);
                let small = bi.size().min(bj.size());
                if ov.x > 0.25 * small.x.max(1e-4) && ov.y > 0.25 * small.y.max(1e-4) && ov.z > 0.25 * small.z.max(1e-4) {
                    out.push(
                        Diag::warning("W322", ev.nodes[j].span, format!("'{}' and '{}' occupy the same space (their bounding boxes overlap by {:.2} x {:.2} x {:.2} m)", ev.nodes[i].path, ev.nodes[j].path, ov.x, ov.y, ov.z))
                            .hint(format!("move one of them, for example  right_of: {}  or  on: {}  - ignore this if one is meant to be inside the other", ev.nodes[i].path, ev.nodes[i].path)),
                    );
                }
            }
        }
    }

    // lighting and camera sanity
    let has_light = infos.iter().any(|o| o.is_light);
    let dark_env = match &scene.env {
        Env::Uniform(c) => c.max_comp() <= 0.0,
        Env::Gradient { zenith, horizon, ground } => zenith.max_comp() + horizon.max_comp() + ground.max_comp() <= 0.0,
        Env::Sky(s) => s.sun_dir.y < -0.1 || (s.sun_strength <= 0.0 && s.sky_strength <= 0.0),
    };
    if dark_env && !has_light {
        out.push(Diag::warning("W330", Span::new(0, 0), "the scene has no light: the image will be black").hint("add  sky { }  for daylight, or a  light name { at: [x, y, z], power: 1500 }"));
    }
    let cam = scene.camera.pos;
    for &i in &top {
        if is_plane(ev, i) {
            continue;
        }
        let b = bounds[&i];
        let inside = cam.x > b.min.x && cam.x < b.max.x && cam.y > b.min.y && cam.y < b.max.y && cam.z > b.min.z && cam.z < b.max.z;
        if inside && matches!(ev.nodes[i].kind, SKind::Shape { .. }) {
            let span = ev.camera.as_ref().map(|c| c.span).unwrap_or(Span::new(0, 0));
            out.push(Diag::warning("W331", span, format!("the camera is inside '{}'", ev.nodes[i].path)).hint("move the camera with 'from', or omit 'from' to have it placed automatically"));
        }
    }
    for &i in &top {
        if is_plane(ev, i) && cam.y < bounds[&i].max.y {
            let span = ev.camera.as_ref().map(|c| c.span).unwrap_or(Span::new(0, 0));
            out.push(Diag::warning("W331", span, format!("the camera is below the floor '{}'", ev.nodes[i].path)).hint("raise the camera: its 'from' height must be above the floor"));
        }
    }
    ev.diags.extend(out);
}
