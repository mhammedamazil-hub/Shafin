//! Built-in material presets. Each is a physically plausible parameter set; the
//! textured ones are procedural (no image files), built from the same texture
//! expressions a VL program can write.
use crate::value::{make_op, Node};
use std::rc::Rc;
use vl_core::color::srgb_vec_to_linear;
use vl_core::math::{vec3, Vec3};
use vl_ir::expr::{Op, INPUT_GRAIN, INPUT_P, INPUT_RAND};

/// A material parameter before compilation: constant or texture expression.
#[derive(Clone, Debug)]
pub enum PV {
    Const(Vec3),
    Tex(Rc<Node>),
}

impl PV {
    pub fn num(v: f32) -> PV {
        PV::Const(Vec3::splat(v))
    }
}

#[derive(Clone, Debug)]
pub struct MatSpec {
    pub name: String,
    pub base: PV,
    pub metallic: PV,
    pub roughness: PV,
    pub specular: f32,
    pub ior: f32,
    pub transmission: PV,
    pub coat: f32,
    pub coat_roughness: f32,
    pub emission: PV,
    pub emission_strength: f32,
    pub sheen: f32,
    pub bump: Option<Rc<Node>>,
    pub bump_strength: f32,
    pub absorb_color: Vec3,
    pub absorb_dist: f32,
    pub thin: bool,
}

impl Default for MatSpec {
    fn default() -> MatSpec {
        MatSpec {
            name: String::new(),
            base: PV::Const(srgb(0.8, 0.8, 0.8)),
            metallic: PV::num(0.0),
            roughness: PV::num(0.5),
            specular: 1.0,
            ior: 1.5,
            transmission: PV::num(0.0),
            coat: 0.0,
            coat_roughness: 0.03,
            emission: PV::Const(Vec3::ZERO),
            emission_strength: 0.0,
            sheen: 0.0,
            bump: None,
            bump_strength: 1.0,
            absorb_color: Vec3::ONE,
            absorb_dist: 0.0,
            thin: false,
        }
    }
}

fn srgb(r: f32, g: f32, b: f32) -> Vec3 {
    srgb_vec_to_linear(vec3(r, g, b))
}

fn c(v: Vec3) -> Rc<Node> {
    Rc::new(Node::Const(v))
}
fn n(v: f32) -> Rc<Node> {
    c(Vec3::splat(v))
}
fn op(o: Op, a: Vec<Rc<Node>>) -> Rc<Node> {
    make_op(o, a)
}
fn p() -> Rc<Node> {
    Rc::new(Node::Input(INPUT_P))
}
fn mul(a: Rc<Node>, b: Rc<Node>) -> Rc<Node> {
    op(Op::Mul, vec![a, b])
}
fn add(a: Rc<Node>, b: Rc<Node>) -> Rc<Node> {
    op(Op::Add, vec![a, b])
}
fn mix(a: Rc<Node>, b: Rc<Node>, t: Rc<Node>) -> Rc<Node> {
    op(Op::Mix, vec![a, b, t])
}
fn fbm(pos: Rc<Node>, octaves: f32, seed: f32) -> Rc<Node> {
    op(Op::Fbm, vec![pos, c(vec3(octaves, 2.0, 0.5)), n(seed)])
}
fn smooth(a: f32, b: f32, x: Rc<Node>) -> Rc<Node> {
    op(Op::Smoothstep, vec![n(a), n(b), x])
}

/// (name, one-line description) of every preset, for diagnostics and the language reference.
pub const PRESETS: &[(&str, &str)] = &[
    ("matte", "flat paint, chalk, plaster: no shine"),
    ("plastic", "smooth coloured plastic with a soft highlight"),
    ("glossy", "polished lacquer or enamel"),
    ("rubber", "dull, dark, slightly rough"),
    ("ceramic", "glazed pottery or porcelain"),
    ("paper", "matte and thin"),
    ("fabric", "cloth with a soft sheen at grazing angles"),
    ("leather", "smooth leather with fine grain"),
    ("glass", "clear glass (refracts)"),
    ("frosted_glass", "translucent rough glass"),
    ("water", "clear water (ior 1.33)"),
    ("mirror", "perfect mirror"),
    ("chrome", "mirror-like metal"),
    ("gold", "polished gold"),
    ("silver", "polished silver"),
    ("copper", "polished copper"),
    ("brass", "polished brass"),
    ("steel", "brushed steel"),
    ("iron", "dark rough iron"),
    ("aluminium", "satin aluminium"),
    ("wood", "timber with grain running along each object's longest side (procedural); colour sets the species"),
    ("marble", "white marble with grey veins (procedural)"),
    ("concrete", "rough grey concrete with stains and pores (procedural)"),
    ("stone", "rough natural stone (procedural)"),
    ("light", "glows; set emission and emission_strength"),
];

pub fn preset_names() -> impl Iterator<Item = &'static str> {
    PRESETS.iter().map(|p| p.0)
}

/// Build a preset. `tint` replaces the preset's own colour where that makes sense.
pub fn preset(name: &str, tint: Option<Vec3>) -> Option<MatSpec> {
    let name = if name == "aluminum" { "aluminium" } else { name };
    let col = |d: Vec3| PV::Const(tint.unwrap_or(d));
    let metal = |f0: Vec3, rough: f32| MatSpec { base: PV::Const(tint.unwrap_or(f0)), metallic: PV::num(1.0), roughness: PV::num(rough), ..MatSpec::default() };
    let base = MatSpec { name: name.to_string(), ..MatSpec::default() };
    let m = match name {
        "matte" => MatSpec { base: col(srgb(0.8, 0.8, 0.8)), roughness: PV::num(1.0), specular: 0.0, ..base },
        "plastic" => MatSpec { base: col(srgb(0.8, 0.1, 0.1)), roughness: PV::num(0.35), ..base },
        "glossy" => MatSpec { base: col(srgb(0.7, 0.7, 0.7)), roughness: PV::num(0.6), coat: 1.0, coat_roughness: 0.03, ..base },
        "rubber" => MatSpec { base: col(srgb(0.1, 0.1, 0.1)), roughness: PV::num(0.8), specular: 0.5, ..base },
        "ceramic" => MatSpec { base: col(srgb(0.92, 0.91, 0.88)), roughness: PV::num(0.7), coat: 1.0, coat_roughness: 0.04, ..base },
        "paper" => MatSpec { base: col(srgb(0.93, 0.92, 0.88)), roughness: PV::num(0.95), specular: 0.2, ..base },
        "fabric" => MatSpec { base: col(srgb(0.3, 0.35, 0.6)), roughness: PV::num(0.95), specular: 0.2, sheen: 0.6, ..base },
        "leather" => {
            let grain = fbm(mul(p(), n(900.0)), 3.0, 5.0);
            MatSpec { base: col(srgb(0.35, 0.2, 0.12)), roughness: PV::Tex(mix(n(0.45), n(0.65), grain.clone())), bump: Some(mul(grain, n(0.0002))), ..base }
        }
        "glass" => MatSpec { base: col(Vec3::ONE), roughness: PV::num(0.0), transmission: PV::num(1.0), ior: 1.5, ..base },
        "frosted_glass" => MatSpec { base: col(Vec3::ONE), roughness: PV::num(0.35), transmission: PV::num(1.0), ior: 1.5, ..base },
        "water" => MatSpec { base: col(Vec3::ONE), roughness: PV::num(0.0), transmission: PV::num(1.0), ior: 1.33, ..base },
        "mirror" => MatSpec { base: col(Vec3::splat(0.95)), metallic: PV::num(1.0), roughness: PV::num(0.0), ..base },
        // metal colours: reflectance at normal incidence, linear RGB
        "chrome" => metal(vec3(0.55, 0.56, 0.55), 0.05),
        "gold" => metal(vec3(1.0, 0.77, 0.34), 0.15),
        "silver" => metal(vec3(0.97, 0.96, 0.92), 0.12),
        "copper" => metal(vec3(0.96, 0.64, 0.54), 0.2),
        "brass" => metal(vec3(0.89, 0.79, 0.45), 0.2),
        "steel" => metal(vec3(0.56, 0.57, 0.58), 0.35),
        "iron" => metal(vec3(0.42, 0.41, 0.40), 0.55),
        "aluminium" => metal(vec3(0.91, 0.92, 0.92), 0.3),
        "wood" => {
            // Flat-sawn timber. The grain runs along the longest side of each object (GRAIN.x).
            // Growth rings are distances from a pith axis that lies well outside the board, so
            // they show as long, slowly wandering bands rather than concentric circles; their
            // contrast varies, and fine fibre streaks and pores sit on top. Every object gets a
            // different piece of wood (offset by RAND).
            let tone = tint.unwrap_or(srgb(0.55, 0.36, 0.2));
            let (dark, light) = (tone * 0.66, (tone * 1.1).clamp(0.0, 1.0));
            let g = add(Rc::new(Node::Input(INPUT_GRAIN)), mul(Rc::new(Node::Input(INPUT_RAND)), n(7.0)));
            let u = op(Op::GetY, vec![g.clone()]);
            let v = op(Op::GetZ, vec![g.clone()]);
            // slow wander of the rings along the board, plus a finer wobble so no two lines are parallel
            let wander = mul(op(Op::Sub, vec![fbm(mul(g.clone(), c(vec3(0.9, 2.5, 2.5))), 3.0, 11.0), n(0.5)]), n(0.14));
            let wobble = mul(op(Op::Sub, vec![fbm(mul(g.clone(), c(vec3(5.0, 40.0, 40.0))), 2.0, 12.0), n(0.5)]), n(0.006));
            let du = add(u, n(0.23));
            let dv = add(mul(v, n(0.55)), n(0.61));
            let radial = add(op(Op::Sqrt, vec![add(mul(du.clone(), du), mul(dv.clone(), dv))]), add(wander, wobble));
            // growth rings a few millimetres apart: soft rise through early wood, sharp late-wood line
            let rings = op(Op::Fract, vec![mul(radial.clone(), n(150.0))]);
            let band = mul(smooth(0.0, 0.65, rings.clone()), op(Op::Sub, vec![n(1.0), smooth(0.8, 1.0, rings)]));
            // ring contrast comes and goes along the board; broad darker and lighter zones on top
            let strength = add(n(0.35), mul(fbm(mul(g.clone(), c(vec3(0.6, 7.0, 7.0))), 2.0, 5.0), n(0.65)));
            let zones = fbm(mul(radial, n(9.0)), 2.0, 19.0);
            // fibres: long thin streaks; pores: short dark dashes
            let fibre = fbm(mul(g.clone(), c(vec3(5.0, 520.0, 520.0))), 3.0, 3.0);
            let pores = smooth(0.66, 0.8, op(Op::Noise, vec![mul(g.clone(), c(vec3(45.0, 900.0, 900.0))), n(29.0)]));
            let blotch = fbm(mul(g.clone(), c(vec3(1.2, 9.0, 9.0))), 3.0, 17.0);
            let t = op(
                Op::Saturate,
                vec![op(Op::Sub, vec![add(add(add(mul(mul(band, strength), n(0.42)), mul(fibre.clone(), n(0.34))), mul(zones, n(0.22))), mul(blotch, n(0.22))), mul(pores.clone(), n(0.3))])],
            );
            let height = op(Op::Sub, vec![mul(fibre, n(0.00012)), mul(pores, n(0.00008))]);
            MatSpec { base: PV::Tex(mix(c(dark), c(light), t.clone())), roughness: PV::Tex(mix(n(0.48), n(0.4), t)), bump: Some(height), ..base }
        }
        "marble" => {
            let tone = tint.unwrap_or(srgb(0.93, 0.92, 0.9));
            let q = mul(p(), n(2.2));
            let warp = fbm(q.clone(), 5.0, 2.0);
            let x = add(op(Op::GetX, vec![q.clone()]), add(op(Op::GetY, vec![q.clone()]), mul(warp, n(2.6))));
            let wave = op(Op::Abs, vec![op(Op::Sin, vec![mul(x, n(3.6))])]);
            let vein = op(Op::Pow, vec![op(Op::Sub, vec![n(1.0), wave]), n(7.0)]);
            let cloud = fbm(mul(q, n(2.5)), 4.0, 9.0);
            let t = op(Op::Saturate, vec![add(mul(vein, n(0.85)), mul(cloud, n(0.18)))]);
            MatSpec { base: PV::Tex(mix(c(tone), c(tone * vec3(0.3, 0.31, 0.34)), t)), roughness: PV::num(0.12), coat: 0.6, coat_roughness: 0.03, ..base }
        }
        "concrete" | "stone" => {
            let stone = name == "stone";
            let tone = tint.unwrap_or(if stone { srgb(0.5, 0.47, 0.43) } else { srgb(0.62, 0.61, 0.59) });
            let stains = fbm(mul(p(), n(if stone { 2.5 } else { 1.4 })), 5.0, 21.0);
            let fine = fbm(mul(p(), n(55.0)), 4.0, 22.0);
            let pores = smooth(0.62, 0.8, op(Op::Noise, vec![mul(p(), n(140.0)), n(23.0)]));
            let shade = add(mul(stains.clone(), n(0.5)), mul(fine.clone(), n(0.3)));
            let t = op(Op::Saturate, vec![add(n(0.55), op(Op::Sub, vec![mul(shade, n(if stone { 1.0 } else { 0.6 })), mul(pores.clone(), n(0.35))]))]);
            let height = add(mul(fine, n(0.0006)), add(mul(stains, n(if stone { 0.004 } else { 0.0008 })), mul(pores, n(-0.0005))));
            MatSpec { base: PV::Tex(mul(c(tone), mul(t, n(1.25)))), roughness: PV::num(if stone { 0.85 } else { 0.9 }), specular: 0.4, bump: Some(height), ..base }
        }
        "light" => MatSpec { base: PV::Const(Vec3::ZERO), specular: 0.0, emission: PV::Const(tint.unwrap_or(Vec3::ONE)), emission_strength: 1000.0, ..base },
        _ => return None,
    };
    Some(MatSpec { name: name.to_string(), ..m })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::ProgramBuilder;
    use vl_ir::expr::ShadeInputs;

    #[test]
    fn every_listed_preset_exists_and_is_physically_plausible() {
        for (name, _) in PRESETS {
            let m = preset(name, None).unwrap_or_else(|| panic!("preset '{name}' is listed but not implemented"));
            assert_eq!(m.name, *name);
            assert!((1.0..=2.5).contains(&m.ior));
            // textured parameters stay inside their physical range everywhere
            for (what, pv, lo, hi) in [("base", &m.base, 0.0f32, 1.0f32), ("roughness", &m.roughness, 0.0, 1.0)] {
                if let PV::Tex(node) = pv {
                    let mut b = ProgramBuilder::default();
                    let r = b.compile(node);
                    b.program.validate().unwrap();
                    let mut regs = Vec::new();
                    for i in 0..2000 {
                        let q = vec3((i as f32 * 0.137).sin() * 3.0, (i as f32 * 0.071).cos() * 2.0, i as f32 * 0.0113 - 5.0);
                        b.program.eval(&ShadeInputs::at(q), &mut regs);
                        let v = regs[r as usize];
                        assert!(v.min_comp() >= lo - 1e-4 && v.max_comp() <= hi + 0.3, "{name}.{what} = {v:?} at {q:?}");
                    }
                }
            }
            if let Some(h) = &m.bump {
                let mut b = ProgramBuilder::default();
                let r = b.compile(h);
                let mut regs = Vec::new();
                b.program.eval(&ShadeInputs::at(vec3(0.3, 0.2, 0.1)), &mut regs);
                assert!(regs[r as usize].x.abs() < 0.02, "{name}: bump heights are in metres and must be small");
            }
        }
        assert!(preset("aluminum", None).is_some(), "US spelling accepted");
        assert!(preset("unobtainium", None).is_none());
        // a tint replaces the colour of plain presets
        let PV::Const(c) = preset("plastic", Some(vec3(0.0, 1.0, 0.0))).unwrap().base else { panic!() };
        assert_eq!(c, vec3(0.0, 1.0, 0.0));
    }
}
