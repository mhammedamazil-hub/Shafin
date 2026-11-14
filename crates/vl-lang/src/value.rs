//! Runtime values of the evaluator, symbolic texture expressions, and built-in functions.
//!
//! An expression that touches a shading input (`P`, `UV`, `N`, ...) evaluates to a
//! symbolic node tree (`Value::Tex`), later compiled to a `vl_ir::expr::Program`
//! that the renderer runs at every shading point. Constant sub-expressions are
//! folded by running the SAME virtual machine, so a function gives identical
//! results at compile time and at render time.
use std::collections::HashMap;
use std::rc::Rc;
use vl_core::color;
use vl_core::dmath;
use vl_core::math::{vec3, Vec3};
use vl_core::rng;
use vl_ir::expr::{Instr, Op, Program, ShadeInputs, INPUT_GRAIN, INPUT_N, INPUT_P, INPUT_RAND, INPUT_UV, INPUT_WP};

#[derive(Debug)]
pub enum Node {
    Const(Vec3),
    Input(u32),
    Op(Op, Vec<Rc<Node>>),
}

#[derive(Clone, Debug)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    /// A 3-vector; colours are linear-light RGB.
    Vec3(Vec3),
    List(Vec<Value>),
    /// Symbolic shading expression.
    Tex(Rc<Node>),
    /// Reference to a scene node (object or group).
    Object(usize),
    Material(usize),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "number",
            Value::Bool(_) => "true/false",
            Value::Str(_) => "string",
            Value::Vec3(_) => "vector",
            Value::List(_) => "list",
            Value::Tex(_) => "texture expression",
            Value::Object(_) => "object",
            Value::Material(_) => "material",
        }
    }
    pub fn vec(v: Vec3) -> Value {
        Value::Vec3(v)
    }
    /// The value as a texture node, if it is numeric.
    pub fn as_node(&self) -> Option<Rc<Node>> {
        match self {
            Value::Num(n) => Some(Rc::new(Node::Const(Vec3::splat(*n as f32)))),
            Value::Bool(b) => Some(Rc::new(Node::Const(Vec3::splat(if *b { 1.0 } else { 0.0 })))),
            Value::Vec3(v) => Some(Rc::new(Node::Const(*v))),
            Value::Tex(n) => Some(n.clone()),
            _ => None,
        }
    }
    pub fn is_tex(&self) -> bool {
        matches!(self, Value::Tex(_))
    }
}

/// Shading inputs available in texture expressions.
pub const INPUT_NAMES: &[(&str, u32, &str)] = &[
    ("P", INPUT_P, "position on the object, in metres, in the object's own frame"),
    ("WP", INPUT_WP, "position in the world, in metres"),
    ("UV", INPUT_UV, "surface coordinates (0..1 where the shape has a natural mapping)"),
    ("N", INPUT_N, "surface normal in the world"),
    ("RAND", INPUT_RAND, "three random numbers in 0..1, different for every object"),
    ("GRAIN", INPUT_GRAIN, "like P, but reordered so GRAIN.x runs along the object's longest side (wood grain, brushed metal)"),
];

/// Compiles node trees into a `Program`, sharing common sub-expressions.
#[derive(Default)]
pub struct ProgramBuilder {
    pub program: Program,
    by_ptr: HashMap<*const Node, u32>,
    by_key: HashMap<(u8, u32, u32, u32), u32>,
}

impl ProgramBuilder {
    fn push(&mut self, op: Op, a: u32, b: u32, c: u32) -> u32 {
        let key = (op as u8, a, b, c);
        if let Some(&r) = self.by_key.get(&key) {
            return r;
        }
        let r = self.program.instrs.len() as u32;
        self.program.instrs.push(Instr { op, a, b, c });
        self.program.scalar.push(false);
        self.by_key.insert(key, r);
        r
    }

    pub fn compile(&mut self, n: &Rc<Node>) -> u32 {
        if let Some(&r) = self.by_ptr.get(&Rc::as_ptr(n)) {
            return r;
        }
        let r = match &**n {
            Node::Const(v) => {
                let bits = (v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                let idx = match self.program.consts.iter().position(|c| (c.x.to_bits(), c.y.to_bits(), c.z.to_bits()) == bits) {
                    Some(i) => i,
                    None => {
                        self.program.consts.push(*v);
                        self.program.consts.len() - 1
                    }
                };
                self.push(Op::Const, idx as u32, 0, 0)
            }
            Node::Input(i) => self.push(Op::Input, *i, 0, 0),
            Node::Op(op, args) => {
                let mut r = [0u32; 3];
                for (k, a) in args.iter().take(3).enumerate() {
                    r[k] = self.compile(a);
                }
                self.push(*op, r[0], r[1], r[2])
            }
        };
        self.by_ptr.insert(Rc::as_ptr(n), r);
        r
    }
}

/// Evaluate a node tree whose leaves are all constants (shading inputs read as zero).
pub fn fold(n: &Rc<Node>) -> Vec3 {
    let mut b = ProgramBuilder::default();
    let r = b.compile(n);
    let mut regs = Vec::new();
    b.program.eval(&ShadeInputs::at(Vec3::ZERO), &mut regs);
    regs[r as usize]
}

fn is_const(n: &Rc<Node>) -> bool {
    match &**n {
        Node::Const(_) => true,
        Node::Input(_) => false,
        Node::Op(_, args) => args.iter().all(is_const),
    }
}

/// Build `op(args)`; folds to a constant when every argument is constant.
pub fn make_op(op: Op, args: Vec<Rc<Node>>) -> Rc<Node> {
    let n = Rc::new(Node::Op(op, args));
    if is_const(&n) {
        Rc::new(Node::Const(fold(&n)))
    } else {
        n
    }
}

/// How a built-in maps onto the shading VM.
pub struct VmFn {
    pub name: &'static str,
    pub op: Op,
    /// Positional parameters, with defaults for the optional ones (`None` = required).
    pub params: &'static [(&'static str, Option<f32>)],
    /// Does the result have three components even for scalar inputs?
    pub vector_result: bool,
    pub doc: &'static str,
}

const R: Option<f32> = None;

pub const VM_FUNCTIONS: &[VmFn] = &[
    VmFn { name: "sin", op: Op::Sin, params: &[("x", R)], vector_result: false, doc: "sine of an angle in RADIANS (use radians(deg) to convert)" },
    VmFn { name: "cos", op: Op::Cos, params: &[("x", R)], vector_result: false, doc: "cosine of an angle in RADIANS" },
    VmFn { name: "sqrt", op: Op::Sqrt, params: &[("x", R)], vector_result: false, doc: "square root" },
    VmFn { name: "abs", op: Op::Abs, params: &[("x", R)], vector_result: false, doc: "absolute value" },
    VmFn { name: "sign", op: Op::Sign, params: &[("x", R)], vector_result: false, doc: "-1, 0 or 1" },
    VmFn { name: "floor", op: Op::Floor, params: &[("x", R)], vector_result: false, doc: "round down" },
    VmFn { name: "ceil", op: Op::Ceil, params: &[("x", R)], vector_result: false, doc: "round up" },
    VmFn { name: "fract", op: Op::Fract, params: &[("x", R)], vector_result: false, doc: "fractional part, x - floor(x)" },
    VmFn { name: "exp", op: Op::Exp, params: &[("x", R)], vector_result: false, doc: "e to the power x" },
    VmFn { name: "log", op: Op::Log, params: &[("x", R)], vector_result: false, doc: "natural logarithm" },
    VmFn { name: "saturate", op: Op::Saturate, params: &[("x", R)], vector_result: false, doc: "clamp to 0..1" },
    VmFn { name: "length", op: Op::Length, params: &[("v", R)], vector_result: false, doc: "length of a vector" },
    VmFn { name: "normalize", op: Op::Normalize, params: &[("v", R)], vector_result: true, doc: "vector scaled to length 1" },
    VmFn { name: "luma", op: Op::Luma, params: &[("color", R)], vector_result: false, doc: "brightness of a colour" },
    VmFn { name: "min", op: Op::Min, params: &[("a", R), ("b", R)], vector_result: false, doc: "smaller of two values" },
    VmFn { name: "max", op: Op::Max, params: &[("a", R), ("b", R)], vector_result: false, doc: "larger of two values" },
    VmFn { name: "pow", op: Op::Pow, params: &[("x", R), ("y", R)], vector_result: false, doc: "x to the power y (x < 0 is treated as 0)" },
    VmFn { name: "mod", op: Op::Mod, params: &[("x", R), ("y", R)], vector_result: false, doc: "remainder, always in 0..y" },
    VmFn { name: "step", op: Op::Step, params: &[("edge", R), ("x", R)], vector_result: false, doc: "0 if x < edge, else 1" },
    VmFn { name: "dot", op: Op::Dot, params: &[("a", R), ("b", R)], vector_result: false, doc: "dot product" },
    VmFn { name: "cross", op: Op::Cross, params: &[("a", R), ("b", R)], vector_result: true, doc: "cross product" },
    VmFn { name: "atan2", op: Op::Atan2, params: &[("y", R), ("x", R)], vector_result: false, doc: "angle of the point (x, y) in radians" },
    VmFn { name: "mix", op: Op::Mix, params: &[("a", R), ("b", R), ("t", R)], vector_result: false, doc: "blend: a when t = 0, b when t = 1" },
    VmFn { name: "lerp", op: Op::Mix, params: &[("a", R), ("b", R), ("t", R)], vector_result: false, doc: "same as mix" },
    VmFn { name: "clamp", op: Op::Clamp, params: &[("x", R), ("lo", R), ("hi", R)], vector_result: false, doc: "limit x to lo..hi" },
    VmFn { name: "smoothstep", op: Op::Smoothstep, params: &[("edge0", R), ("edge1", R), ("x", R)], vector_result: false, doc: "smooth 0..1 ramp between two edges" },
    VmFn { name: "select", op: Op::Select, params: &[("condition", R), ("if_true", R), ("if_false", R)], vector_result: false, doc: "choose between two values" },
    VmFn { name: "vec", op: Op::MakeVec, params: &[("x", R), ("y", R), ("z", R)], vector_result: true, doc: "build a vector from three numbers" },
    VmFn { name: "noise", op: Op::Noise, params: &[("p", R), ("seed", Some(0.0))], vector_result: false, doc: "smooth random pattern in 0..1; feature size is 1 unit of p (multiply p to make it finer)" },
    VmFn { name: "snoise", op: Op::SNoise, params: &[("p", R), ("seed", Some(0.0))], vector_result: false, doc: "smooth random pattern in -1..1" },
    VmFn { name: "voronoi", op: Op::Voronoi, params: &[("p", R), ("seed", Some(0.0))], vector_result: true, doc: "cell pattern: [distance to nearest cell point, distance to second nearest, random id of the cell]" },
    VmFn { name: "cell", op: Op::Cell, params: &[("p", R), ("seed", Some(0.0))], vector_result: true, doc: "three random numbers that are constant inside each unit cube of p (tiles, bricks, flakes)" },
];

/// fbm / turbulence take (p, octaves, lacunarity, gain, seed) and pack three of them into one register.
pub const FRACTAL_FUNCTIONS: &[(&str, Op, &str)] = &[
    ("fbm", Op::Fbm, "layered noise in 0..1 (clouds, stains, wood grain): fbm(p, octaves = 4, lacunarity = 2, gain = 0.5, seed = 0)"),
    ("turbulence", Op::Turbulence, "layered creased noise in 0..1 (marble veins, fire): turbulence(p, octaves = 4, lacunarity = 2, gain = 0.5, seed = 0)"),
];
pub const FRACTAL_PARAMS: &[(&str, Option<f32>)] = &[("p", R), ("octaves", Some(4.0)), ("lacunarity", Some(2.0)), ("gain", Some(0.5)), ("seed", Some(0.0))];

/// Functions evaluated only at compile time (their arguments must be plain numbers).
pub const CONST_FUNCTIONS: &[(&str, &str)] = &[
    ("rgb", "colour from red, green, blue in 0..1 (or 0..255) as written in sRGB: rgb(0.8, 0.2, 0.1)"),
    ("hsv", "colour from hue (degrees), saturation and value in 0..1"),
    ("kelvin", "colour of a black body at a temperature: kelvin(2700) is warm lamp light"),
    ("gray", "neutral colour of the given brightness in 0..1 (sRGB)"),
    ("grey", "same as gray"),
    ("radians", "degrees to radians"),
    ("degrees", "radians to degrees"),
    ("tan", "tangent of an angle in radians"),
    ("asin", "inverse sine, result in radians"),
    ("acos", "inverse cosine, result in radians"),
    ("round", "round to the nearest whole number"),
    ("int", "drop the fractional part"),
    ("rand", "repeatable random number in 0..1 from up to three seed numbers: rand(i), rand(i, 7)"),
    ("len", "number of items in a list"),
];

pub fn all_function_names() -> Vec<&'static str> {
    VM_FUNCTIONS.iter().map(|f| f.name).chain(FRACTAL_FUNCTIONS.iter().map(|f| f.0)).chain(CONST_FUNCTIONS.iter().map(|f| f.0)).collect()
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Call a compile-time-only function. `Err` carries a message for the diagnostic.
pub fn call_const(name: &str, args: &[Value]) -> Result<Value, String> {
    let nums: Option<Vec<f64>> = args.iter().map(num).collect();
    let need = |n: usize| -> Result<Vec<f64>, String> {
        match &nums {
            Some(v) if v.len() == n => Ok(v.clone()),
            Some(v) => Err(format!("{name}() takes {n} number{}, but {} were given", if n == 1 { "" } else { "s" }, v.len())),
            None => Err(format!("{name}() needs plain numbers (it cannot be used on a texture expression or a vector)")),
        }
    };
    Ok(match name {
        "rgb" => {
            let v = need(3)?;
            let scale = if v.iter().any(|x| *x > 1.0) { 255.0 } else { 1.0 };
            Value::Vec3(color::srgb_vec_to_linear(vec3((v[0] / scale) as f32, (v[1] / scale) as f32, (v[2] / scale) as f32).clamp(0.0, 1.0)))
        }
        "hsv" => {
            let v = need(3)?;
            let h = (v[0] / 360.0).rem_euclid(1.0) as f32;
            Value::Vec3(color::srgb_vec_to_linear(color::hsv_to_rgb(h, v[1].clamp(0.0, 1.0) as f32, v[2].clamp(0.0, 1.0) as f32)))
        }
        "kelvin" => {
            let v = need(1)?;
            if !(1000.0..=40000.0).contains(&v[0]) {
                return Err(format!("kelvin({}) is outside 1000..40000", v[0]));
            }
            let c = color::kelvin(v[0] as f32);
            Value::Vec3(c * (1.0 / color::luminance(c).max(1e-6)))
        }
        "gray" | "grey" => {
            let v = need(1)?;
            Value::Vec3(Vec3::splat(color::srgb_to_linear(v[0].clamp(0.0, 1.0) as f32)))
        }
        "radians" => Value::Num(need(1)?[0].to_radians()),
        "degrees" => Value::Num(need(1)?[0].to_degrees()),
        "tan" => Value::Num(dmath::tan64(need(1)?[0])),
        "asin" => Value::Num(dmath::asin64(need(1)?[0].clamp(-1.0, 1.0))),
        "acos" => Value::Num(dmath::acos64(need(1)?[0].clamp(-1.0, 1.0))),
        "round" => Value::Num(need(1)?[0].round()),
        "int" => Value::Num(need(1)?[0].trunc()),
        "rand" => {
            let v = nums.clone().ok_or_else(|| "rand() needs plain numbers".to_string())?;
            if v.is_empty() || v.len() > 3 {
                return Err(format!("rand() takes 1 to 3 seed numbers, but {} were given", v.len()));
            }
            let b = |i: usize| v.get(i).copied().unwrap_or(0.0).to_bits();
            Value::Num(rng::unit_f32(rng::hash3(b(0), b(1), b(2))) as f64)
        }
        "len" => match args {
            [Value::List(l)] => Value::Num(l.len() as f64),
            [Value::Vec3(_)] => Value::Num(3.0),
            _ => return Err("len() takes one list".to_string()),
        },
        _ => return Err(format!("unknown function '{name}'")),
    })
}

/// Turn a node back into a plain value when it is constant.
pub fn node_to_value(n: Rc<Node>, vector: bool) -> Value {
    match &*n {
        Node::Const(v) => {
            if vector || v.x != v.y || v.y != v.z {
                Value::Vec3(*v)
            } else {
                Value::Num(v.x as f64)
            }
        }
        _ => Value::Tex(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_and_sharing() {
        let two = Value::Num(2.0).as_node().unwrap();
        let p = Rc::new(Node::Input(INPUT_P));
        // constants fold through the VM
        let n = make_op(Op::Sqrt, vec![make_op(Op::Add, vec![two.clone(), two.clone()])]);
        assert!(matches!(&*n, Node::Const(v) if v.x == 2.0));
        // an expression on P stays symbolic and shares common sub-expressions when compiled
        let scaled = make_op(Op::Mul, vec![p.clone(), two.clone()]);
        let a = make_op(Op::Noise, vec![scaled.clone(), Value::Num(0.0).as_node().unwrap()]);
        let b = make_op(Op::Noise, vec![make_op(Op::Mul, vec![p.clone(), two.clone()]), Value::Num(0.0).as_node().unwrap()]);
        let sum = make_op(Op::Add, vec![a, b]);
        let mut pb = ProgramBuilder::default();
        let r = pb.compile(&sum);
        pb.program.validate().unwrap();
        // P, 2, P*2, 0, noise, add  -> 6 instructions although noise(P*2) was written twice
        assert_eq!(pb.program.len(), 6, "{:?}", pb.program.instrs);
        let mut regs = Vec::new();
        pb.program.eval(&ShadeInputs::at(vec3(0.3, 0.7, 0.1)), &mut regs);
        let direct = vl_ir::noise::noise(vec3(0.6, 1.4, 0.2), 0);
        assert_eq!(regs[r as usize].x, 2.0 * direct);
    }

    #[test]
    fn const_functions() {
        let v = |x: f64| Value::Num(x);
        let Value::Vec3(c) = call_const("rgb", &[v(255.0), v(128.0), v(0.0)]).unwrap() else { panic!() };
        assert!((c.x - 1.0).abs() < 1e-6 && (c.y - 0.2158).abs() < 1e-3 && c.z == 0.0);
        let Value::Vec3(c2) = call_const("rgb", &[v(1.0), v(0.5), v(0.0)]).unwrap() else { panic!() };
        assert!((c2.y - 0.2140).abs() < 1e-3, "0..1 form");
        let Value::Num(r1) = call_const("rand", &[v(3.0)]).unwrap() else { panic!() };
        let Value::Num(r2) = call_const("rand", &[v(3.0)]).unwrap() else { panic!() };
        let Value::Num(r3) = call_const("rand", &[v(4.0)]).unwrap() else { panic!() };
        assert!(r1 == r2 && r1 != r3 && (0.0..1.0).contains(&r1));
        let Value::Vec3(k) = call_const("kelvin", &[v(2700.0)]).unwrap() else { panic!() };
        assert!(k.x > k.y && k.y > k.z, "2700 K is warm: {k:?}");
        assert!((color::luminance(k) - 1.0).abs() < 1e-4, "kelvin() preserves brightness");
        assert!(call_const("rgb", &[v(1.0)]).unwrap_err().contains("takes 3 numbers"));
        assert!(call_const("kelvin", &[v(100.0)]).is_err());
    }
}
