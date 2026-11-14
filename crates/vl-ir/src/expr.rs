//! Shading-expression programs.
//!
//! A VL texture such as `mix(oak_dark, oak_light, fbm(P * [1, 14, 1], 4))` is
//! compiled to a small SSA program that the renderer evaluates at each shading
//! point. Registers are `Vec3`; scalars are stored replicated in all three
//! lanes, which makes every operation component-wise and the evaluator tiny.
//! This file is the reference semantics: any backend must reproduce it (or
//! bake textures by calling it).
use crate::noise;
use vl_core::dmath;
use vl_core::json::Json;
use vl_core::math::{vec3, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// `consts[a]`
    Const,
    /// Shading input `a`: 0 = P (object-space position, metres), 1 = world position,
    /// 2 = UV, 3 = world normal, 4 = per-object random vec3 in [0,1),
    /// 5 = P with its axes reordered so the object's longest side comes first (grain frame)
    Input,
    Neg,
    Abs,
    Sign,
    Floor,
    Ceil,
    Fract,
    Sqrt,
    Sin,
    Cos,
    Exp,
    Log,
    Saturate,
    Length,
    Normalize,
    GetX,
    GetY,
    GetZ,
    Luma,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Min,
    Max,
    Step,
    Dot,
    Cross,
    Atan2,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Not,
    Mix,
    Clamp,
    Smoothstep,
    Select,
    MakeVec,
    /// noise(p, seed) -> [0,1]
    Noise,
    /// snoise(p, seed) -> [-1,1]
    SNoise,
    /// fbm(p, (octaves, lacunarity, gain), seed) -> [0,1]
    Fbm,
    /// turbulence(p, (octaves, lacunarity, gain), seed) -> [0,1]
    Turbulence,
    /// voronoi(p, seed) -> (F1, F2, cell id)
    Voronoi,
    /// cell(p, seed) -> random vec3 constant inside each unit lattice cell
    Cell,
}

pub const OP_NAMES: &[(Op, &str)] = &[
    (Op::Const, "const"),
    (Op::Input, "input"),
    (Op::Neg, "neg"),
    (Op::Abs, "abs"),
    (Op::Sign, "sign"),
    (Op::Floor, "floor"),
    (Op::Ceil, "ceil"),
    (Op::Fract, "fract"),
    (Op::Sqrt, "sqrt"),
    (Op::Sin, "sin"),
    (Op::Cos, "cos"),
    (Op::Exp, "exp"),
    (Op::Log, "log"),
    (Op::Saturate, "saturate"),
    (Op::Length, "length"),
    (Op::Normalize, "normalize"),
    (Op::GetX, "x"),
    (Op::GetY, "y"),
    (Op::GetZ, "z"),
    (Op::Luma, "luma"),
    (Op::Add, "add"),
    (Op::Sub, "sub"),
    (Op::Mul, "mul"),
    (Op::Div, "div"),
    (Op::Mod, "mod"),
    (Op::Pow, "pow"),
    (Op::Min, "min"),
    (Op::Max, "max"),
    (Op::Step, "step"),
    (Op::Dot, "dot"),
    (Op::Cross, "cross"),
    (Op::Atan2, "atan2"),
    (Op::Lt, "lt"),
    (Op::Le, "le"),
    (Op::Gt, "gt"),
    (Op::Ge, "ge"),
    (Op::Eq, "eq"),
    (Op::Ne, "ne"),
    (Op::And, "and"),
    (Op::Or, "or"),
    (Op::Not, "not"),
    (Op::Mix, "mix"),
    (Op::Clamp, "clamp"),
    (Op::Smoothstep, "smoothstep"),
    (Op::Select, "select"),
    (Op::MakeVec, "vec"),
    (Op::Noise, "noise"),
    (Op::SNoise, "snoise"),
    (Op::Fbm, "fbm"),
    (Op::Turbulence, "turbulence"),
    (Op::Voronoi, "voronoi"),
    (Op::Cell, "cell"),
];

pub fn op_name(op: Op) -> &'static str {
    OP_NAMES.iter().find(|(o, _)| *o == op).map(|(_, n)| *n).unwrap_or("?")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Instr {
    pub op: Op,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

pub const INPUT_P: u32 = 0;
pub const INPUT_WP: u32 = 1;
pub const INPUT_UV: u32 = 2;
pub const INPUT_N: u32 = 3;
pub const INPUT_RAND: u32 = 4;
pub const INPUT_GRAIN: u32 = 5;

/// Values available at a shading point.
#[derive(Clone, Copy, Debug)]
pub struct ShadeInputs {
    pub p: Vec3,
    pub wp: Vec3,
    pub uv: Vec3,
    pub n: Vec3,
    pub rand: Vec3,
    /// `p` with its components reordered: (along the object's longest side, the other two).
    pub grain: Vec3,
}

impl ShadeInputs {
    pub fn at(p: Vec3) -> ShadeInputs {
        ShadeInputs { p, wp: p, uv: Vec3::ZERO, n: Vec3::Y, rand: Vec3::ZERO, grain: p }
    }
}

/// Axis order that puts the longest side of a box of the given size first (ties keep x, y, z order).
pub fn grain_axes(size: Vec3) -> [usize; 3] {
    let mut axes = [0usize, 1, 2];
    axes.sort_by(|&a, &b| size[b].partial_cmp(&size[a]).unwrap_or(std::cmp::Ordering::Equal));
    axes
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub instrs: Vec<Instr>,
    pub consts: Vec<Vec3>,
    /// `true` when the register holds a scalar (replicated lanes).
    pub scalar: Vec<bool>,
}

pub const MAX_PROGRAM_LEN: usize = 4096;

#[inline]
fn b2f(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn fmod(a: f32, b: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else {
        a - b * (a / b).floor()
    }
}

#[inline]
fn seed_of(v: Vec3) -> u32 {
    v.x.clamp(0.0, 4.0e9) as u32
}

impl Program {
    pub fn len(&self) -> usize {
        self.instrs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.instrs.is_empty()
    }
    /// Structural validity: operands must refer to earlier registers.
    pub fn validate(&self) -> Result<(), String> {
        if self.instrs.len() > MAX_PROGRAM_LEN {
            return Err(format!("expression program has {} instructions (limit {MAX_PROGRAM_LEN})", self.instrs.len()));
        }
        for (i, ins) in self.instrs.iter().enumerate() {
            match ins.op {
                Op::Const => {
                    if ins.a as usize >= self.consts.len() {
                        return Err(format!("instruction {i}: constant index out of range"));
                    }
                }
                Op::Input => {
                    if ins.a > INPUT_GRAIN {
                        return Err(format!("instruction {i}: unknown input {}", ins.a));
                    }
                }
                _ => {
                    for (k, r) in [ins.a, ins.b, ins.c].iter().enumerate() {
                        if k < arity(ins.op) && *r as usize >= i {
                            return Err(format!("instruction {i}: operand refers to a later register"));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Evaluate all registers into `regs` (resized as needed).
    pub fn eval(&self, inp: &ShadeInputs, regs: &mut Vec<Vec3>) {
        regs.clear();
        regs.reserve(self.instrs.len());
        for ins in &self.instrs {
            let a = || regs[ins.a as usize];
            let b = || regs[ins.b as usize];
            let c = || regs[ins.c as usize];
            let v = match ins.op {
                Op::Const => self.consts[ins.a as usize],
                Op::Input => match ins.a {
                    INPUT_P => inp.p,
                    INPUT_WP => inp.wp,
                    INPUT_UV => inp.uv,
                    INPUT_N => inp.n,
                    INPUT_RAND => inp.rand,
                    _ => inp.grain,
                },
                Op::Neg => -a(),
                Op::Abs => a().abs(),
                Op::Sign => a().map(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }),
                Op::Floor => a().map(f32::floor),
                Op::Ceil => a().map(f32::ceil),
                Op::Fract => a().map(|x| x - x.floor()),
                Op::Sqrt => a().map(|x| x.max(0.0).sqrt()),
                Op::Sin => a().map(dmath::sin),
                Op::Cos => a().map(dmath::cos),
                Op::Exp => a().map(|x| dmath::exp(x.min(80.0))),
                Op::Log => a().map(|x| dmath::ln(x.max(1e-30))),
                Op::Saturate => a().clamp(0.0, 1.0),
                Op::Length => Vec3::splat(a().length()),
                Op::Normalize => a().normalize(),
                Op::GetX => Vec3::splat(a().x),
                Op::GetY => Vec3::splat(a().y),
                Op::GetZ => Vec3::splat(a().z),
                Op::Luma => Vec3::splat(vl_core::color::luminance(a())),
                Op::Add => a() + b(),
                Op::Sub => a() - b(),
                Op::Mul => a() * b(),
                Op::Div => {
                    let (x, y) = (a(), b());
                    vec3(if y.x != 0.0 { x.x / y.x } else { 0.0 }, if y.y != 0.0 { x.y / y.y } else { 0.0 }, if y.z != 0.0 { x.z / y.z } else { 0.0 })
                }
                Op::Mod => {
                    let (x, y) = (a(), b());
                    vec3(fmod(x.x, y.x), fmod(x.y, y.y), fmod(x.z, y.z))
                }
                Op::Pow => {
                    let (x, y) = (a(), b());
                    vec3(dmath::pow(x.x.max(0.0), y.x), dmath::pow(x.y.max(0.0), y.y), dmath::pow(x.z.max(0.0), y.z))
                }
                Op::Min => a().min(b()),
                Op::Max => a().max(b()),
                Op::Step => {
                    let (e, x) = (a(), b());
                    vec3(b2f(x.x >= e.x), b2f(x.y >= e.y), b2f(x.z >= e.z))
                }
                Op::Dot => Vec3::splat(a().dot(b())),
                Op::Cross => a().cross(b()),
                Op::Atan2 => {
                    let (y, x) = (a(), b());
                    vec3(dmath::atan2(y.x, x.x), dmath::atan2(y.y, x.y), dmath::atan2(y.z, x.z))
                }
                Op::Lt => Vec3::splat(b2f(a().x < b().x)),
                Op::Le => Vec3::splat(b2f(a().x <= b().x)),
                Op::Gt => Vec3::splat(b2f(a().x > b().x)),
                Op::Ge => Vec3::splat(b2f(a().x >= b().x)),
                Op::Eq => Vec3::splat(b2f(a().x == b().x)),
                Op::Ne => Vec3::splat(b2f(a().x != b().x)),
                Op::And => Vec3::splat(b2f(a().x != 0.0 && b().x != 0.0)),
                Op::Or => Vec3::splat(b2f(a().x != 0.0 || b().x != 0.0)),
                Op::Not => Vec3::splat(b2f(a().x == 0.0)),
                Op::Mix => {
                    let (x, y, t) = (a(), b(), c());
                    x + (y - x) * t
                }
                Op::Clamp => a().max(b()).min(c()),
                Op::Smoothstep => {
                    let (e0, e1, x) = (a(), b(), c());
                    let f = |e0: f32, e1: f32, x: f32| vl_core::math::smoothstep(e0, e1, x);
                    vec3(f(e0.x, e1.x, x.x), f(e0.y, e1.y, x.y), f(e0.z, e1.z, x.z))
                }
                Op::Select => {
                    if a().x != 0.0 {
                        b()
                    } else {
                        c()
                    }
                }
                Op::MakeVec => vec3(a().x, b().x, c().x),
                Op::Noise => Vec3::splat(noise::noise(a(), seed_of(b()))),
                Op::SNoise => Vec3::splat(noise::snoise(a(), seed_of(b()))),
                Op::Fbm => {
                    let k = b();
                    Vec3::splat(noise::fbm(a(), k.x.clamp(1.0, 12.0) as u32, k.y, k.z, seed_of(c())))
                }
                Op::Turbulence => {
                    let k = b();
                    Vec3::splat(noise::turbulence(a(), k.x.clamp(1.0, 12.0) as u32, k.y, k.z, seed_of(c())))
                }
                Op::Voronoi => noise::voronoi(a(), seed_of(b())),
                Op::Cell => {
                    let p = a();
                    noise::cell_random(p.x.clamp(-1e6, 1e6).floor() as i32, p.y.clamp(-1e6, 1e6).floor() as i32, p.z.clamp(-1e6, 1e6).floor() as i32, seed_of(b()))
                }
            };
            // never let NaN/inf escape into the renderer
            regs.push(if v.is_finite() { v } else { Vec3::ZERO });
        }
    }

    pub fn to_json(&self) -> Json {
        let mut instrs = Json::arr();
        for (i, ins) in self.instrs.iter().enumerate() {
            let mut row = Json::arr();
            row.push(op_name(ins.op));
            match ins.op {
                Op::Const => {
                    let c = self.consts[ins.a as usize];
                    if self.scalar.get(i).copied().unwrap_or(false) {
                        row.push(Json::f32(c.x));
                    } else {
                        row.push(Json::Arr(vec![Json::f32(c.x), Json::f32(c.y), Json::f32(c.z)]));
                    }
                }
                Op::Input => row.push(["P", "WP", "UV", "N", "RAND", "GRAIN"][ins.a.min(5) as usize]),
                _ => {
                    for r in [ins.a, ins.b, ins.c].iter().take(arity(ins.op)) {
                        row.push(*r as f64);
                    }
                }
            }
            instrs.push(row);
        }
        Json::obj().with("instructions", instrs)
    }
}

pub fn arity(op: Op) -> usize {
    match op {
        Op::Const | Op::Input => 0,
        Op::Neg
        | Op::Abs
        | Op::Sign
        | Op::Floor
        | Op::Ceil
        | Op::Fract
        | Op::Sqrt
        | Op::Sin
        | Op::Cos
        | Op::Exp
        | Op::Log
        | Op::Saturate
        | Op::Length
        | Op::Normalize
        | Op::GetX
        | Op::GetY
        | Op::GetZ
        | Op::Luma
        | Op::Not => 1,
        Op::Mix | Op::Clamp | Op::Smoothstep | Op::Select | Op::MakeVec | Op::Fbm | Op::Turbulence => 3,
        _ => 2,
    }
}

/// Does the op always produce a scalar (replicated) register?
pub fn yields_scalar(op: Op) -> Option<bool> {
    match op {
        Op::Length | Op::GetX | Op::GetY | Op::GetZ | Op::Luma | Op::Dot | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne | Op::And | Op::Or | Op::Not | Op::Noise | Op::SNoise | Op::Fbm | Op::Turbulence => {
            Some(true)
        }
        Op::Normalize | Op::Cross | Op::MakeVec | Op::Voronoi | Op::Cell | Op::Input => Some(false),
        _ => None,
    }
}

/// A material/volume parameter: a constant or one register of a program.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    Const(Vec3),
    /// (program index in `Scene::programs`, register index)
    Expr(u32, u32),
}

impl Param {
    pub fn scalar(v: f32) -> Param {
        Param::Const(Vec3::splat(v))
    }
    pub fn is_const(&self) -> bool {
        matches!(self, Param::Const(_))
    }
    pub fn const_value(&self) -> Option<Vec3> {
        match self {
            Param::Const(v) => Some(*v),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(instrs: Vec<(Op, u32, u32, u32)>, consts: Vec<Vec3>) -> Program {
        let n = instrs.len();
        Program { instrs: instrs.into_iter().map(|(op, a, b, c)| Instr { op, a, b, c }).collect(), consts, scalar: vec![false; n] }
    }

    #[test]
    fn evaluates_checker_like_program() {
        // floor(P.x) + floor(P.z) mod 2 -> select(white, black)
        let p = prog(
            vec![
                (Op::Input, INPUT_P, 0, 0),
                (Op::Floor, 0, 0, 0),
                (Op::GetX, 1, 0, 0),
                (Op::GetZ, 1, 0, 0),
                (Op::Add, 2, 3, 0),
                (Op::Const, 0, 0, 0),
                (Op::Mod, 4, 5, 0),
                (Op::Const, 1, 0, 0),
                (Op::Const, 2, 0, 0),
                (Op::Select, 6, 7, 8),
            ],
            vec![Vec3::splat(2.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 0.0, 0.0)],
        );
        p.validate().unwrap();
        let mut regs = Vec::new();
        p.eval(&ShadeInputs::at(vec3(0.5, 0.0, 0.5)), &mut regs);
        assert_eq!(regs[9], Vec3::ZERO);
        p.eval(&ShadeInputs::at(vec3(1.5, 0.0, 0.5)), &mut regs);
        assert_eq!(regs[9], Vec3::ONE);
        p.eval(&ShadeInputs::at(vec3(-0.5, 0.0, 0.5)), &mut regs);
        assert_eq!(regs[9], Vec3::ONE, "negative coordinates keep the pattern (floor-mod)");
    }

    #[test]
    fn rejects_forward_references_and_handles_nan() {
        let bad = prog(vec![(Op::Neg, 0, 0, 0)], vec![]);
        assert!(bad.validate().is_err());
        let p = prog(vec![(Op::Const, 0, 0, 0), (Op::Const, 1, 0, 0), (Op::Div, 0, 1, 0), (Op::Log, 1, 0, 0), (Op::Sqrt, 3, 0, 0)], vec![Vec3::ONE, Vec3::ZERO]);
        p.validate().unwrap();
        let mut regs = Vec::new();
        p.eval(&ShadeInputs::at(Vec3::ZERO), &mut regs);
        assert!(regs.iter().all(|v| v.is_finite()));
        assert_eq!(regs[2], Vec3::ZERO, "division by zero is defined as 0");
    }
}
