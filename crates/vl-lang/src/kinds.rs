//! Evaluator, part 2: object kinds, relational placement, materials, lights,
//! camera, sky and render settings.
use crate::ast::*;
use crate::eval::*;
use crate::presets::{self, MatSpec, PV};
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;
use vl_core::color;
use vl_core::diag::{suggest, Span};
use vl_core::math::{vec3, Aabb, Mat4, Vec3};
use vl_ir::scene::{Shape, Tonemap};

const COMMON_OBJECT_PROPS: &[&str] = &["material", "color", "visible", "shadow"];
const MAX_DEFINE_DEPTH: u32 = 24;
const RELATIONS: [&str; 7] = ["on", "above", "below", "left_of", "right_of", "in_front_of", "behind"];

pub const MATERIAL_PROPS: &[&str] = &[
    "preset", "color", "roughness", "metallic", "specular", "ior", "transmission", "coat", "coat_roughness", "emission", "emission_strength", "sheen", "bump", "bump_strength", "absorb_color", "absorb_distance", "thin",
];
pub const LIGHT_PROPS: &[&str] = &["shape", "radius", "size", "width", "depth", "power", "brightness", "color", "temperature", "aim", "visible", "at", "on", "above", "below", "left_of", "right_of", "in_front_of", "behind", "gap", "rotate"];
pub const RENDER_PROPS: &[&str] = &["size", "width", "height", "samples", "max_depth", "seed", "exposure", "exposure_bias", "white_balance", "saturation", "contrast", "tonemap", "noise_target", "caustics"];
pub const CAMERA_PROPS: &[&str] = &["look_at", "from", "distance", "azimuth", "elevation", "fov", "focal_length", "f_stop", "focus", "blades"];
pub const SKY_PROPS: &[&str] = &["sun_elevation", "sun_azimuth", "turbidity", "sun_size", "strength", "sun_strength", "sky_strength", "ground"];
pub const ENVIRONMENT_PROPS: &[&str] = &["color", "strength", "top", "horizon", "bottom"];

pub fn shape_props(kind: &str) -> &'static [&'static str] {
    match kind {
        "sphere" => &["radius", "diameter"],
        "box" => &["size", "width", "height", "depth", "round"],
        "cylinder" => &["radius", "diameter", "height"],
        "cone" => &["radius", "top_radius", "height"],
        "torus" => &["radius", "tube"],
        "capsule" => &["radius", "height"],
        "disk" => &["radius"],
        "rect" => &["size", "width", "depth"],
        _ => &[],
    }
}

impl Ev {
    // --------------------------------------------------------------- placement

    /// Compute the local transform of something whose bounds, before any transform, are `b`.
    /// Returns the transform and the bounds in the parent's frame.
    ///
    /// Without any placement property an object stands on the floor of its frame (y = 0)
    /// at the origin - never half-buried - unless `default_pos` says otherwise.
    fn place(&mut self, props: &mut Props, b: Aabb, default_pos: Option<Vec3>) -> (Mat4, Aabb) {
        let scale = match props.take(&["scale"]) {
            None => Vec3::ONE,
            Some((v, span)) => match self.as_vec3(&v, span, "scale", true) {
                Some(s) if s.min_comp() > 0.0 => s,
                Some(_) => {
                    self.error("E205", span, "'scale' must be greater than 0 on every axis", "use a positive number, or [sx, sy, sz]");
                    Vec3::ONE
                }
                None => Vec3::ONE,
            },
        };
        let rot = match props.take(&["rotate"]) {
            None => Mat4::IDENTITY,
            Some((Value::Num(deg), _)) if deg.is_finite() => Mat4::rotate_y(deg as f32),
            Some((v, span)) => match self.as_vec3(&v, span, "rotate", false) {
                Some(r) => Mat4::rotate_xyz(r),
                None => Mat4::IDENTITY,
            },
        };
        let rs = rot.mul(&Mat4::scale(scale));
        let b0 = b.transformed(&rs);
        let gap = self.num(props, &["gap"], 0.0, -1000.0, 1000.0) as f32;

        let used: Vec<&str> = RELATIONS.iter().copied().filter(|k| props.has(&[k])).collect();
        if used.len() > 1 {
            let sp = props.list.iter().find(|p| p.key == used[1]).map(|p| p.key_span).unwrap();
            self.error("E221", sp, format!("'{}' and '{}' cannot be combined", used[0], used[1]), "use one relation, and  at: [dx, dz]  for a sideways offset");
        }
        let at = props.take(&["at"]);
        let mut pos: Option<Vec3> = None;
        if let Some(&rel) = used.first() {
            let (target, tspan) = props.take(&[rel]).unwrap();
            for other in used.iter().skip(1) {
                props.take(&[other]);
            }
            if let Some(t) = self.target_bounds(&target, tspan, rel) {
                let (tc, oc) = (t.center(), b0.center());
                // start aligned: centred on the target horizontally, standing on the target's base level
                let mut p = vec3(tc.x - oc.x, t.min.y - b0.min.y, tc.z - oc.z);
                match rel {
                    "on" | "above" => p.y = t.max.y + gap - b0.min.y,
                    "below" => p.y = t.min.y - gap - b0.max.y,
                    "right_of" => p.x = t.max.x + gap - b0.min.x,
                    "left_of" => p.x = t.min.x - gap - b0.max.x,
                    "in_front_of" => p.z = t.max.z + gap - b0.min.z,
                    _ => p.z = t.min.z - gap - b0.max.z,
                }
                match at {
                    None => {}
                    Some((Value::List(l), aspan)) if l.len() == 2 => {
                        if let (Value::Num(dx), Value::Num(dz)) = (&l[0], &l[1]) {
                            p.x += *dx as f32;
                            p.z += *dz as f32;
                        } else {
                            self.error("E204", aspan, "'at' must contain numbers", "write  at: [dx, dz]");
                        }
                    }
                    Some((_, aspan)) => {
                        self.error("E222", aspan, format!("with '{rel}', 'at' must be a two-number offset [dx, dz] from the centre of the target"), format!("write for example  {rel}: table, at: [0.2, -0.1]   (the height comes from '{rel}')"));
                    }
                }
                pos = Some(p);
            }
        } else if let Some((v, aspan)) = at {
            pos = match &v {
                Value::List(l) if l.len() == 2 => {
                    // [x, z] without a relation: stand on the floor at that spot
                    if let (Value::Num(x), Value::Num(z)) = (&l[0], &l[1]) {
                        Some(vec3(*x as f32 - b0.center().x, -b0.min.y, *z as f32 - b0.center().z))
                    } else {
                        self.error("E204", aspan, "'at' must contain numbers", "write  at: [x, y, z]");
                        None
                    }
                }
                _ => self.as_vec3(&v, aspan, "at", false),
            };
        }
        let pos = pos.unwrap_or_else(|| default_pos.unwrap_or(vec3(-b0.center().x, -b0.min.y, -b0.center().z)));
        let local = Mat4::translate(pos).mul(&rs);
        (local, b.transformed(&local))
    }

    // ------------------------------------------------------------------ nodes

    fn unique_name(&mut self, kind: &str, name: Option<&(String, Span)>) -> (String, String) {
        let local = match name {
            Some((n, nspan)) => {
                let clean: String = n.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
                let clean = if clean.is_empty() { kind.to_string() } else { clean };
                let taken = |n: &str| self.frames.last().unwrap().names.contains_key(n);
                if taken(&clean) {
                    let mut k = 2;
                    while taken(&format!("{clean}_{k}")) {
                        k += 1;
                    }
                    let renamed = format!("{clean}_{k}");
                    if self.quiet_renames == 0 {
                        self.warn("W301", *nspan, format!("the name '{clean}' is already used; this object is called '{renamed}'"), "give every object its own name");
                    }
                    renamed
                } else {
                    clean
                }
            }
            None => {
                let mut c = self.counters.get(kind).copied().unwrap_or(0);
                let n = loop {
                    c += 1;
                    let n = format!("{kind}_{c}");
                    if !self.frames.last().unwrap().names.contains_key(&n) {
                        break n;
                    }
                };
                self.counters.insert(kind.to_string(), c);
                n
            }
        };
        let prefix = &self.frames.last().unwrap().prefix;
        let path = if prefix.is_empty() { local.clone() } else { format!("{prefix}.{local}") };
        (local, path)
    }

    fn add_node(&mut self, mut node: SNode, declared: Option<&(String, Span)>) -> usize {
        let i = self.nodes.len();
        node.parent = self.current_frame_node();
        if let Some(p) = node.parent {
            self.nodes[p].children.push(i);
        }
        let frame = self.frames.last_mut().unwrap();
        frame.names.insert(node.name.clone(), i);
        // inside a loop the plain name refers to the most recent instance
        if let Some((n, _)) = declared {
            frame.names.insert(n.clone(), i);
        }
        self.nodes.push(node);
        i
    }

    // -------------------------------------------------------------- materials

    fn material_index(&mut self, spec: MatSpec) -> usize {
        self.mats.push(spec);
        self.mats.len() - 1
    }

    fn named_material(&mut self, name: &str) -> Option<usize> {
        if let Some(&i) = self.mat_names.get(name) {
            return Some(i);
        }
        let spec = presets::preset(name, None)?;
        let i = self.material_index(spec);
        self.mat_names.insert(name.to_string(), i);
        Some(i)
    }

    fn as_material(&mut self, v: &Value, span: Span) -> Option<usize> {
        match v {
            Value::Material(i) => Some(*i),
            Value::Str(s) => match self.named_material(s) {
                Some(i) => Some(i),
                None => {
                    let names: Vec<String> = self.mat_names.keys().cloned().chain(presets::preset_names().map(String::from)).collect();
                    let hint = match suggest(s, names.iter().map(|x| x.as_str())) {
                        Some(sg) => format!("did you mean '{sg}'?"),
                        None if color::named(s).is_some() || color::parse_hex(s).is_some() => format!("'{s}' is a colour; write  color: {s}  (optionally with  material: plastic)"),
                        None => format!("material presets: {}", presets::preset_names().collect::<Vec<_>>().join(", ")),
                    };
                    self.error("E205", span, format!("'{s}' is not a material"), hint);
                    None
                }
            },
            Value::Vec3(_) => {
                self.error("E204", span, "'material' needs a material, but a colour was given", "write  color: #rrggbb  to colour the object, or  material: plastic, color: #rrggbb");
                None
            }
            _ => {
                self.error("E204", span, format!("'material' must be a material, not a {}", v.type_name()), format!("use a preset ({} ...) or a material defined with  material name {{ ... }}", presets::preset_names().take(8).collect::<Vec<_>>().join(", ")));
                None
            }
        }
    }

    fn apply_material_props(&mut self, m: &mut MatSpec, props: &mut Props) {
        if let Some(p) = self.pv(props, &["color", "base_color"], true, 0.0, 1.0) {
            m.base = p;
        }
        if let Some(p) = self.pv(props, &["roughness"], false, 0.0, 1.0) {
            m.roughness = p;
        }
        if let Some(p) = self.pv(props, &["metallic"], false, 0.0, 1.0) {
            m.metallic = p;
        }
        if let Some(p) = self.pv(props, &["transmission"], false, 0.0, 1.0) {
            m.transmission = p;
        }
        if let Some(p) = self.pv(props, &["emission"], true, 0.0, 1.0) {
            m.emission = p;
            if m.emission_strength <= 0.0 {
                m.emission_strength = 1000.0;
            }
        }
        m.emission_strength = self.num(props, &["emission_strength"], m.emission_strength as f64, 0.0, 1.0e10) as f32;
        m.specular = self.num(props, &["specular"], m.specular as f64, 0.0, 1.0) as f32;
        m.ior = self.num(props, &["ior"], m.ior as f64, 1.0, 4.0) as f32;
        m.coat = self.num(props, &["coat"], m.coat as f64, 0.0, 1.0) as f32;
        m.coat_roughness = self.num(props, &["coat_roughness"], m.coat_roughness as f64, 0.0, 1.0) as f32;
        m.sheen = self.num(props, &["sheen"], m.sheen as f64, 0.0, 1.0) as f32;
        m.bump_strength = self.num(props, &["bump_strength"], m.bump_strength as f64, 0.0, 100.0) as f32;
        m.absorb_dist = self.num(props, &["absorb_distance"], m.absorb_dist as f64, 0.0, 1.0e6) as f32;
        if let Some(c) = self.color(props, &["absorb_color"]) {
            m.absorb_color = c;
            if m.absorb_dist <= 0.0 {
                m.absorb_dist = 0.1;
            }
        }
        m.thin = self.boolean(props, &["thin"], m.thin);
        if let Some((v, span)) = props.take(&["bump"]) {
            match &v {
                Value::Tex(n) => m.bump = Some(n.clone()),
                Value::Num(_) => m.bump = None,
                _ => self.error("E204", span, format!("'bump' must be a texture expression giving a height in metres, not a {}", v.type_name()), "for example  bump: fbm(P * 40) * 0.002"),
            }
        }
    }

    fn material_block(&mut self, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("material", body, false);
        self.scopes.pop();
        let Some((name, nspan)) = name else {
            self.error("E234", span, "a material needs a name", "write  material my_name { preset: plastic, color: #cc3333 }");
            return;
        };
        let tint = match props.list.iter().find(|p| p.key == "color" || p.key == "base_color") {
            Some(p) if !p.value.is_tex() => {
                let (v, s) = (p.value.clone(), p.span);
                self.as_color(&v, s, "color")
            }
            _ => None,
        };
        let mut m = match props.take(&["preset"]) {
            None => MatSpec::default(),
            Some((Value::Str(p), pspan)) => match presets::preset(&p, tint) {
                Some(m) => m,
                None => match self.mat_names.get(&p).copied() {
                    Some(i) => self.mats[i].clone(),
                    None => {
                        let hint = match suggest(&p, presets::preset_names()) {
                            Some(s) => format!("did you mean '{s}'?"),
                            None => format!("presets: {}", presets::preset_names().collect::<Vec<_>>().join(", ")),
                        };
                        self.error("E205", pspan, format!("unknown material preset '{p}'"), hint);
                        MatSpec::default()
                    }
                },
            },
            Some((Value::Material(i), _)) => self.mats[i].clone(),
            Some((v, pspan)) => {
                self.error("E204", pspan, format!("'preset' must be the name of a preset, not a {}", v.type_name()), format!("presets: {}", presets::preset_names().collect::<Vec<_>>().join(", ")));
                MatSpec::default()
            }
        };
        // for textured presets the tint was already applied when the preset was built
        if matches!(m.base, PV::Tex(_)) && tint.is_some() {
            props.take(&["color", "base_color"]);
        }
        self.apply_material_props(&mut m, &mut props);
        self.finish("material", props, MATERIAL_PROPS);
        m.name = name.clone();
        if self.mat_names.contains_key(name) && presets::preset(name, None).is_none() {
            self.warn("W301", *nspan, format!("material '{name}' is defined twice; the later definition is used for objects that follow"), "give every material its own name");
        }
        let i = self.material_index(m);
        self.mat_names.insert(name.clone(), i);
    }

    /// Material for an object from its `material:` and `color:` properties.
    fn object_material(&mut self, props: &mut Props, path: &str) -> usize {
        let mat = props.take(&["material"]);
        let tint = self.color(props, &["color"]);
        // a preset combined with a colour is rebuilt with that tint, so textured presets take it properly
        if let (Some((Value::Str(s), _)), Some(t)) = (&mat, tint) {
            if !self.mat_names.contains_key(s.as_str()) || presets::preset(s, None).is_some() {
                if let Some(mut m) = presets::preset(s, Some(t)) {
                    m.name = format!("{path}.{s}");
                    return self.material_index(m);
                }
            }
        }
        let base = match mat {
            Some((v, span)) => self.as_material(&v, span),
            None => None,
        };
        match (base, tint) {
            (Some(i), None) => i,
            (Some(i), Some(t)) => {
                let mut m = self.mats[i].clone();
                m.base = PV::Const(t);
                m.name = format!("{path}.{}", m.name);
                self.material_index(m)
            }
            (None, Some(t)) => {
                let m = MatSpec { name: format!("{path}.color"), base: PV::Const(t), ..MatSpec::default() };
                self.material_index(m)
            }
            (None, None) => match self.mat_names.get("default").copied() {
                Some(i) => i,
                None => {
                    let i = self.material_index(MatSpec { name: "default".into(), ..MatSpec::default() });
                    self.mat_names.insert("default".into(), i);
                    i
                }
            },
        }
    }

    // ----------------------------------------------------------------- blocks

    pub(crate) fn block(&mut self, kind: &str, kind_span: Span, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        if let Some(def) = self.defines.get(kind).cloned() {
            return self.instance(kind, &def, name, body, span);
        }
        let kind_l = match kind {
            "cube" => "box",
            "quad" | "rectangle" => "rect",
            "floor" | "ground" => "plane",
            "settings" | "output" | "image" => "render",
            "background" | "env" => "environment",
            "lamp" => "light",
            k => k,
        };
        match kind_l {
            "render" => self.render_block(body),
            "camera" => self.camera_block(body, span),
            "sky" => self.sky_block(body),
            "environment" => self.environment_block(body),
            "material" => self.material_block(name, body, span),
            "light" => self.light_block(name, body, span),
            "group" => self.group_block(name, body, span),
            k if OBJECT_KINDS.iter().any(|x| x.0 == k) => self.object_block(k, name, body, span),
            _ => {
                let mut all: Vec<String> = OBJECT_KINDS.iter().map(|k| k.0.to_string()).chain(OTHER_KINDS.iter().map(|s| s.to_string())).collect();
                all.extend(self.defines.keys().cloned());
                let hint = match suggest(kind, all.iter().map(|s| s.as_str())) {
                    Some(s) => format!("did you mean '{s}'?"),
                    None => format!("build it from simple shapes ({}) inside a  group name {{ ... }}, or declare a reusable kind with  define {kind}() {{ ... }}", OBJECT_KINDS.iter().map(|k| k.0).collect::<Vec<_>>().join(", ")),
                };
                self.error("E202", kind_span, format!("unknown kind of object '{kind}'"), hint);
            }
        }
    }

    fn shape_from_props(&mut self, kind: &str, props: &mut Props) -> Shape {
        let big = 1.0e5;
        match kind {
            "sphere" => {
                let r = match self.opt_num(props, &["diameter"], 1e-5, big) {
                    Some(d) => d * 0.5,
                    None => self.num(props, &["radius"], 0.5, 1e-5, big),
                };
                Shape::Sphere { r: r as f32 }
            }
            "box" => {
                let mut size = Vec3::ONE;
                if let Some((v, span)) = props.take(&["size"]) {
                    match self.as_vec3(&v, span, "size", true) {
                        Some(s) if s.min_comp() > 0.0 => size = s,
                        Some(_) => self.error("E205", span, "'size' must be greater than 0 on every axis", "write  size: [width, height, depth]  in metres"),
                        None => {}
                    }
                }
                size.x = self.num(props, &["width"], size.x as f64, 1e-5, big) as f32;
                size.y = self.num(props, &["height"], size.y as f64, 1e-5, big) as f32;
                size.z = self.num(props, &["depth"], size.z as f64, 1e-5, big) as f32;
                let half = size * 0.5;
                let round = self.num(props, &["round"], 0.0, 0.0, big) as f32;
                Shape::Box { half, round: round.min(half.min_comp()) }
            }
            "cylinder" => {
                let r = match self.opt_num(props, &["diameter"], 1e-5, big) {
                    Some(d) => d * 0.5,
                    None => self.num(props, &["radius"], 0.5, 1e-5, big),
                } as f32;
                let h = self.num(props, &["height"], 1.0, 1e-5, big) as f32;
                Shape::Cone { r0: r, r1: r, half_h: h * 0.5 }
            }
            "cone" => {
                let r0 = self.num(props, &["radius"], 0.5, 0.0, big) as f32;
                let r1 = self.num(props, &["top_radius"], 0.0, 0.0, big) as f32;
                let h = self.num(props, &["height"], 1.0, 1e-5, big) as f32;
                Shape::Cone { r0: if r0 <= 0.0 && r1 <= 0.0 { 0.5 } else { r0 }, r1, half_h: h * 0.5 }
            }
            "torus" => {
                let major = self.num(props, &["radius"], 0.5, 1e-5, big) as f32;
                let minor = self.num(props, &["tube", "thickness"], 0.15, 1e-5, big) as f32;
                Shape::Torus { major, minor }
            }
            "capsule" => {
                let r = self.num(props, &["radius"], 0.25, 1e-5, big) as f32;
                let h = self.num(props, &["height"], 1.0, 1e-5, big) as f32;
                Shape::Capsule { r, half_h: (h * 0.5 - r).max(0.0) }
            }
            "disk" => Shape::Disk { r: self.num(props, &["radius"], 0.5, 1e-5, big) as f32 },
            "rect" => {
                let (mut w, mut d) = (1.0f32, 1.0f32);
                if let Some((v, span)) = props.take(&["size"]) {
                    match &v {
                        Value::List(l) if l.len() == 2 => match (&l[0], &l[1]) {
                            (Value::Num(a), Value::Num(b)) if *a > 0.0 && *b > 0.0 => {
                                w = *a as f32;
                                d = *b as f32;
                            }
                            _ => self.error("E204", span, "a rect's 'size' must be two positive numbers [width, depth]", "a rect is flat and horizontal; use  rotate: [90, 0, 0]  to stand it up"),
                        },
                        Value::Num(n) if *n > 0.0 => {
                            w = *n as f32;
                            d = *n as f32;
                        }
                        _ => self.error("E204", span, "a rect's 'size' must be [width, depth] (two positive numbers)", "a rect is flat and horizontal; use  rotate: [90, 0, 0]  to stand it up"),
                    }
                }
                w = self.num(props, &["width"], w as f64, 1e-5, big) as f32;
                d = self.num(props, &["depth"], d as f64, 1e-5, big) as f32;
                Shape::Rect { hx: w * 0.5, hz: d * 0.5 }
            }
            _ => Shape::Plane,
        }
    }

    fn object_block(&mut self, kind: &str, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        self.scopes.push(HashMap::new());
        let mut props = self.props(kind, body, false);
        self.scopes.pop();
        let (local_name, path) = self.unique_name(kind, name);
        let shape = self.shape_from_props(kind, &mut props);
        let material = self.object_material(&mut props, &path);
        let hidden = !self.boolean(&mut props, &["visible"], true);
        let cast_shadow = self.boolean(&mut props, &["shadow"], true);
        let b = shape.local_bounds(&[]);
        let (local, bounds) = self.place(&mut props, b, None);
        let known: Vec<&str> = shape_props(kind).iter().chain(COMMON_OBJECT_PROPS).chain(PLACEMENT_PROPS).copied().collect();
        self.finish(kind, props, &known);
        let is_light = matches!(self.mats[material].emission, PV::Tex(_)) || self.mats[material].emission_strength > 0.0;
        self.add_node(SNode { name: local_name, path, kind: SKind::Shape { shape, material }, kind_name: kind.to_string(), local, parent: None, children: Vec::new(), bounds, span, hidden, cast_shadow, is_light }, name);
    }

    fn open_group(&mut self, kind: &str, name: Option<&(String, Span)>, span: Span) -> usize {
        let (local_name, path) = self.unique_name(kind, name);
        let i = self.add_node(SNode { name: local_name, path: path.clone(), kind: SKind::Group, kind_name: kind.to_string(), local: Mat4::IDENTITY, parent: None, children: Vec::new(), bounds: Aabb::EMPTY, span, hidden: false, cast_shadow: true, is_light: false }, name);
        self.frames.push(Frame { names: HashMap::new(), node: Some(i), prefix: path });
        i
    }

    fn close_group(&mut self, i: usize, kind: &str, mut props: Props, extra_known: &[&str], span: Span) {
        self.frames.pop();
        let mut b = Aabb::EMPTY;
        for &c in &self.nodes[i].children.clone() {
            if !matches!(self.nodes[c].kind, SKind::Shape { shape: Shape::Plane, .. }) {
                b = b.union(&self.nodes[c].bounds);
            }
        }
        if b.is_empty() {
            let path = self.nodes[i].path.clone();
            self.warn("W303", span, format!("'{path}' contains no objects"), "add objects inside the braces");
            b = Aabb::new(Vec3::ZERO, Vec3::ZERO);
        }
        // a group keeps its own origin by default (its children were placed relative to it)
        let (local, bounds) = self.place(&mut props, b, Some(Vec3::ZERO));
        let known: Vec<&str> = extra_known.iter().chain(PLACEMENT_PROPS).copied().collect();
        self.finish(kind, props, &known);
        self.nodes[i].local = local;
        self.nodes[i].bounds = bounds;
    }

    fn group_block(&mut self, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        let i = self.open_group("group", name, span);
        self.scopes.push(HashMap::new());
        let props = self.props("group", body, true);
        self.scopes.pop();
        self.close_group(i, "group", props, &[], span);
    }

    /// Instantiate a user-defined kind: `chair c1 { at: [1, 0], seat_height: 0.5 }`.
    fn instance(&mut self, kind: &str, def: &Rc<Define>, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        if self.define_depth >= MAX_DEFINE_DEPTH {
            self.error("E235", span, format!("'{kind}' uses itself (directly or through other definitions) more than {MAX_DEFINE_DEPTH} levels deep"), "a define must not contain itself");
            return;
        }
        // arguments and placement are evaluated in the caller's scope
        self.scopes.push(HashMap::new());
        let mut props = self.props(kind, body, false);
        self.scopes.pop();
        let mut args: HashMap<String, Value> = HashMap::new();
        for p in &def.params {
            match props.take(&[p.name.as_str()]) {
                Some((v, _)) => {
                    args.insert(p.name.clone(), v);
                }
                None => match &p.default {
                    Some(d) => {
                        if let Some(v) = self.eval(d) {
                            args.insert(p.name.clone(), v);
                        }
                    }
                    None => self.error("E207", span, format!("'{kind}' needs a value for '{}'", p.name), format!("add  {}: value  inside the braces", p.name)),
                },
            }
        }
        let i = self.open_group(kind, name, span);
        // the body sees its parameters and the program's global variables, not the caller's locals
        let globals = self.scopes[0].clone();
        let saved = std::mem::replace(&mut self.scopes, vec![globals, args]);
        self.define_depth += 1;
        self.quiet_renames += 1;
        self.items(&def.body);
        self.quiet_renames -= 1;
        self.define_depth -= 1;
        self.scopes = saved;
        let params: Vec<&str> = def.params.iter().map(|p| p.name.as_str()).collect();
        self.close_group(i, kind, props, &params, span);
    }

    fn light_block(&mut self, name: Option<&(String, Span)>, body: &[Item], span: Span) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("light", body, false);
        self.scopes.pop();
        let (local_name, path) = self.unique_name("light", name);
        let shape_name = match props.take(&["shape"]) {
            None => "sphere".to_string(),
            Some((Value::Str(s), sspan)) => {
                if ["sphere", "rect", "disk"].contains(&s.as_str()) {
                    s
                } else {
                    let hint = match suggest(&s, ["sphere", "rect", "disk"]) {
                        Some(x) => format!("did you mean '{x}'?"),
                        None => "a light is a sphere (bulb), a rect (panel, window) or a disk (spot)".to_string(),
                    };
                    self.error("E205", sspan, format!("a light cannot have the shape '{s}'"), hint);
                    "sphere".to_string()
                }
            }
            Some((v, sspan)) => {
                self.error("E204", sspan, format!("'shape' must be sphere, rect or disk, not a {}", v.type_name()), "write  shape: rect");
                "sphere".to_string()
            }
        };
        let pi = std::f32::consts::PI;
        let (shape, flux_per_nit) = match shape_name.as_str() {
            "rect" => {
                let s = self.shape_from_props("rect", &mut props);
                let area = if let Shape::Rect { hx, hz } = s { 4.0 * hx * hz } else { 1.0 };
                // emits from both faces: flux = 2 * pi * L * area
                (s, 2.0 * pi * area)
            }
            "disk" => {
                let r = self.num(&mut props, &["radius"], 0.15, 1e-4, 1e4) as f32;
                (Shape::Disk { r }, 2.0 * pi * (pi * r * r))
            }
            _ => {
                let r = self.num(&mut props, &["radius"], 0.05, 1e-4, 1e4) as f32;
                // a Lambertian sphere: flux = pi * L * (4 pi r^2)
                (Shape::Sphere { r }, 4.0 * pi * pi * r * r)
            }
        };
        if !props.failed.iter().any(|k| k == "shape") {
            let unused: &[&str] = if shape_name == "rect" { &["radius"] } else { &["size", "width", "depth"] };
            for key in unused {
                if let Some((_, uspan)) = props.take(&[key]) {
                    let hint = if shape_name == "rect" { "a rect light is sized with  size: [width, depth]" } else { "for a panel write  shape: rect, size: [width, depth];  a sphere or disk light is sized with  radius" };
                    self.warn("W311", uspan, format!("'{key}' has no effect on a {shape_name} light"), hint);
                }
            }
        } else {
            props.take(&["size", "width", "depth", "radius"]);
        }
        let mut tint = self.color(&mut props, &["color"]);
        if let Some(k) = self.opt_num(&mut props, &["temperature"], 1000.0, 40000.0) {
            let c = color::kelvin(k as f32);
            tint = Some(c * (1.0 / color::luminance(c).max(1e-6)));
        }
        let tint = tint.unwrap_or(Vec3::ONE);
        // `power` is luminous flux in lumens (a 60 W incandescent bulb is about 800 lm);
        // `brightness` sets the surface luminance in nits directly
        let nits = match self.opt_num(&mut props, &["brightness"], 0.0, 1.0e12) {
            Some(n) => n as f32,
            None => self.num(&mut props, &["power"], 1500.0, 0.0, 1.0e12) as f32 / flux_per_nit,
        };
        let m = MatSpec { name: format!("{path}.light"), base: PV::Const(Vec3::ZERO), specular: 0.0, emission: PV::Const(tint), emission_strength: nits, ..MatSpec::default() };
        let material = self.material_index(m);
        let hidden = !self.boolean(&mut props, &["visible"], true);
        let aim = props.take(&["aim", "look_at"]);
        let b = shape.local_bounds(&[]);
        let had_position = props.has(&["at"]) || RELATIONS.iter().any(|r| props.has(&[r]));
        let (mut local, mut bounds) = self.place(&mut props, b, Some(vec3(0.0, 2.5, 0.0)));
        if !had_position {
            self.warn("W310", span, format!("light '{path}' has no position; it was placed at [0, 2.5, 0]"), "add  at: [x, y, z]");
        }
        if let Some((v, aspan)) = aim {
            if let Some(target) = self.as_vec3(&v, aspan, "aim", false) {
                let pos = local.translation();
                let dir = target - pos;
                if dir.length() > 1e-6 {
                    // flat lights face along their +Y axis
                    local = Mat4::translate(pos).mul(&Mat4::align_y(dir));
                    bounds = b.transformed(&local);
                }
            }
        }
        self.finish("light", props, LIGHT_PROPS);
        self.add_node(SNode { name: local_name, path, kind: SKind::Shape { shape, material }, kind_name: "light".into(), local, parent: None, children: Vec::new(), bounds, span, hidden, cast_shadow: false, is_light: true }, name);
    }

    fn render_block(&mut self, body: &[Item]) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("render", body, false);
        self.scopes.pop();
        let r = self.render.clone();
        let mut out = r.clone();
        if let Some((v, span)) = props.take(&["size", "resolution"]) {
            match &v {
                Value::List(l) if l.len() == 2 => match (&l[0], &l[1]) {
                    (Value::Num(w), Value::Num(h)) if (1.0..=16384.0).contains(w) && (1.0..=16384.0).contains(h) => {
                        out.width = w.round() as u32;
                        out.height = h.round() as u32;
                    }
                    _ => self.error("E205", span, "image size must be two numbers between 1 and 16384", "4K is  size: [3840, 2160]"),
                },
                Value::Str(s) => match s.as_str() {
                    "4k" | "4K" | "uhd" => (out.width, out.height) = (3840, 2160),
                    "1080p" | "hd" | "fullhd" => (out.width, out.height) = (1920, 1080),
                    "720p" => (out.width, out.height) = (1280, 720),
                    "preview" => (out.width, out.height) = (640, 360),
                    _ => self.error("E205", span, format!("unknown image size '{s}'"), "use  size: [width, height]  or one of \"4k\", \"1080p\", \"720p\", \"preview\""),
                },
                _ => self.error("E204", span, "'size' must be [width, height] in pixels", "4K is  size: [3840, 2160]"),
            }
        }
        out.width = self.num(&mut props, &["width"], out.width as f64, 1.0, 16384.0) as u32;
        out.height = self.num(&mut props, &["height"], out.height as f64, 1.0, 16384.0) as u32;
        out.samples = self.num(&mut props, &["samples", "spp"], r.samples as f64, 1.0, 65536.0) as u32;
        out.max_depth = self.num(&mut props, &["max_depth", "bounces"], r.max_depth as f64, 1.0, 64.0) as u32;
        out.seed = self.num(&mut props, &["seed"], r.seed as f64, 0.0, 9.0e15) as u64;
        out.noise_target = self.num(&mut props, &["noise_target"], r.noise_target as f64, 0.0, 1.0) as f32;
        match props.take(&["exposure"]) {
            None => {}
            Some((Value::Num(ev), _)) if (-10.0..=30.0).contains(&ev) => out.exposure = Some(ev as f32),
            Some((Value::Str(s), _)) if s == "auto" => out.exposure = None,
            Some((_, span)) => self.error("E205", span, "'exposure' must be \"auto\" or an exposure value between -10 and 30", "EV 15 suits a sunny day, 12 an overcast day, 7 a lit room at night"),
        }
        out.exposure_bias = self.num(&mut props, &["exposure_bias"], r.exposure_bias as f64, -10.0, 10.0) as f32;
        out.white_balance = self.num(&mut props, &["white_balance"], r.white_balance as f64, 1500.0, 20000.0) as f32;
        out.saturation = self.num(&mut props, &["saturation"], r.saturation as f64, 0.0, 4.0) as f32;
        out.contrast = self.num(&mut props, &["contrast"], r.contrast as f64, 0.2, 4.0) as f32;
        out.caustics = self.boolean(&mut props, &["caustics"], r.caustics);
        if let Some((v, span)) = props.take(&["tonemap"]) {
            match &v {
                Value::Str(s) if s == "filmic" => out.tonemap = Tonemap::Filmic,
                Value::Str(s) if s == "linear" || s == "none" => out.tonemap = Tonemap::Linear,
                Value::Str(s) if s == "reinhard" => out.tonemap = Tonemap::Reinhard,
                _ => self.error("E205", span, "'tonemap' must be \"filmic\", \"reinhard\" or \"linear\"", "filmic is the photographic default"),
            }
        }
        self.finish("render", props, RENDER_PROPS);
        self.render = out;
    }

    fn camera_block(&mut self, body: &[Item], span: Span) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("camera", body, false);
        self.scopes.pop();
        let look_at = props.take(&["look_at", "target"]);
        let from = match props.take(&["from", "position", "at"]) {
            Some((v, s)) => self.as_vec3(&v, s, "from", false),
            None => None,
        };
        let distance = self.opt_num(&mut props, &["distance"], 1e-3, 1.0e6);
        let azimuth = self.num(&mut props, &["azimuth"], 30.0, -3600.0, 3600.0);
        let elevation = self.num(&mut props, &["elevation"], 18.0, -89.0, 89.0);
        let mut fov = self.num(&mut props, &["fov"], 35.0, 1.0, 150.0);
        if let Some(f) = self.opt_num(&mut props, &["focal_length"], 4.0, 2000.0) {
            // full-frame sensor, 24 mm tall
            fov = 2.0 * (12.0 / f).atan().to_degrees();
        }
        let f_stop = self.opt_num(&mut props, &["f_stop", "aperture"], 0.5, 64.0);
        let focus = props.take(&["focus"]);
        let blades = self.num(&mut props, &["blades"], 0.0, 0.0, 16.0) as u32;
        self.finish("camera", props, CAMERA_PROPS);
        if self.camera.is_some() {
            self.warn("W304", span, "there is more than one camera; the last one is used", "keep a single camera block");
        }
        self.camera = Some(CameraSpec { look_at, from, distance, azimuth, elevation, fov, f_stop, focus, blades, span });
    }

    fn sky_block(&mut self, body: &[Item]) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("sky", body, false);
        self.scopes.pop();
        let elevation = self.num(&mut props, &["sun_elevation", "elevation"], 40.0, -90.0, 90.0);
        let azimuth = self.num(&mut props, &["sun_azimuth", "azimuth"], 135.0, -3600.0, 3600.0);
        let turbidity = self.num(&mut props, &["turbidity", "haze"], 3.0, 1.0, 10.0);
        let sun_size = self.num(&mut props, &["sun_size"], 1.0, 0.1, 100.0);
        let strength = self.num(&mut props, &["strength"], 1.0, 0.0, 1000.0);
        let sun_strength = self.num(&mut props, &["sun_strength"], 1.0, 0.0, 1000.0) * strength;
        let sky_strength = self.num(&mut props, &["sky_strength"], 1.0, 0.0, 1000.0) * strength;
        let ground = self.color(&mut props, &["ground", "ground_color"]).unwrap_or(Vec3::splat(0.25));
        self.finish("sky", props, SKY_PROPS);
        self.env = Some(EnvSpec::Sky { elevation, azimuth, turbidity, sun_size, sun_strength, sky_strength, ground });
    }

    fn environment_block(&mut self, body: &[Item]) {
        self.scopes.push(HashMap::new());
        let mut props = self.props("environment", body, false);
        self.scopes.pop();
        // `strength` is the luminance in nits of a white environment (an overcast sky is about 2000)
        let strength = self.num(&mut props, &["strength", "brightness"], 1000.0, 0.0, 1.0e9) as f32;
        let color = self.color(&mut props, &["color"]);
        let top = self.color(&mut props, &["top", "zenith"]);
        let horizon = self.color(&mut props, &["horizon"]);
        let bottom = self.color(&mut props, &["bottom", "ground"]);
        self.finish("environment", props, ENVIRONMENT_PROPS);
        self.env = Some(if top.is_some() || horizon.is_some() || bottom.is_some() {
            let base = color.unwrap_or(Vec3::ONE);
            let h = horizon.unwrap_or(base);
            EnvSpec::Gradient { top: top.unwrap_or(h) * strength, horizon: h * strength, bottom: bottom.unwrap_or(h * 0.5) * strength }
        } else {
            EnvSpec::Uniform(color.unwrap_or(Vec3::ONE) * strength)
        });
    }
}
