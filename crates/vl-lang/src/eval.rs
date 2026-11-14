//! Evaluator, part 1: values, names, expressions, statements and typed property access.
//! (`kinds.rs` holds the object kinds, placement and materials.)
//!
//! Design notes that matter to whoever writes VL (usually a language model):
//! * every object has a name; reports and edits refer to objects by name;
//! * positions are normally RELATIONAL (`on: table`, `right_of: lamp`): the
//!   evaluator computes exact contact from bounding boxes, so objects neither
//!   float nor sink because of arithmetic slips;
//! * every problem becomes a diagnostic with a stable code, a location and a
//!   concrete hint, and evaluation continues so that one pass reports them all.
use crate::ast::*;
use crate::presets::{self, MatSpec, PV};
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;
use vl_core::color;
use vl_core::diag::{suggest, suggest_unique, Diag, Span};
use vl_core::math::{vec3, Aabb, Mat4, Vec3};
use vl_ir::expr::{yields_scalar, Op};
use vl_ir::scene::{Shape, Tonemap};

pub const OBJECT_KINDS: &[(&str, &str)] = &[
    ("sphere", "radius | diameter"),
    ("box", "size: [width, height, depth] | size: number, round: edge radius"),
    ("cylinder", "radius, height (stands upright)"),
    ("cone", "radius, top_radius, height"),
    ("torus", "radius (ring), tube (thickness radius); lies flat"),
    ("capsule", "radius, height (total; stands upright)"),
    ("plane", "an infinite horizontal floor"),
    ("disk", "radius; a flat horizontal circle"),
    ("rect", "size: [width, depth]; a flat horizontal rectangle"),
];
pub const OTHER_KINDS: &[&str] = &["render", "camera", "sky", "environment", "material", "light", "group"];
pub const PLACEMENT_PROPS: &[&str] = &["at", "on", "rotate", "scale", "left_of", "right_of", "in_front_of", "behind", "above", "below", "gap"];

/// Words that select an option of some property (`exposure: auto`, `tonemap: filmic`, ...).
/// Properties whose value is a word from a fixed vocabulary (material and colour names).
pub const WORD_PROPS: &[&str] = &["material", "color", "emission", "absorb_color", "time", "time_of_day", "weather", "shape", "tonemap", "exposure"];

/// The documented name of a property, given a spelling a writer may reach for instead.
/// These are pure synonyms; nothing is reported when one is used.
pub fn canonical_key<'a>(kind: &str, key: &'a str) -> &'a str {
    match (kind, key) {
        (_, "colour") => "color",
        ("camera", "target" | "lookat" | "look" | "looking_at") => "look_at",
        ("camera", "position" | "pos" | "eye" | "location") => "from",
        ("light", "intensity" | "strength" | "lumens" | "watts") => "power",
        ("light", "kelvin" | "color_temperature") => "temperature",
        ("render", "resolution") => "size",
        ("render", "spp") => "samples",
        ("render" | "camera" | "sky" | "environment" | "material", _) => key,
        (_, "position" | "pos" | "location" | "centre" | "center") => "at",
        (_, "rotation") => "rotate",
        _ => key,
    }
}

pub const OPTION_WORDS: &[&str] = &[
    "auto", "filmic", "reinhard", "linear", "none", "preview", "hd", "uhd", "fullhd",
    // times of day for  sky { time: ... }
    "noon", "midday", "day", "daytime", "sunny", "clear", "morning", "afternoon", "evening", "golden_hour", "late_afternoon",
    "sunset", "sunrise", "dawn", "dusk", "night", "midnight", "moonlight", "moonlit", "overcast", "cloudy",
];

const MAX_STEPS: u64 = 2_000_000;
const MAX_NODES: usize = 500_000;

#[derive(Clone, Debug)]
pub enum SKind {
    Shape { shape: Shape, material: usize },
    Group,
}

#[derive(Clone, Debug)]
pub struct SNode {
    pub name: String,
    /// Unique dotted path, e.g. `chair.seat`.
    pub path: String,
    pub kind: SKind,
    pub kind_name: String,
    /// Transform into the parent's frame.
    pub local: Mat4,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Bounds in the parent's frame.
    pub bounds: Aabb,
    pub span: Span,
    pub hidden: bool,
    pub cast_shadow: bool,
    pub is_light: bool,
}

pub(crate) struct Frame {
    pub(crate) names: HashMap<String, usize>,
    pub(crate) node: Option<usize>,
    pub(crate) prefix: String,
}

pub struct Define {
    pub params: Vec<Param>,
    pub body: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct CameraSpec {
    pub look_at: Option<(Value, Span)>,
    pub from: Option<Vec3>,
    pub distance: Option<f64>,
    pub azimuth: f64,
    pub elevation: f64,
    pub fov: f64,
    pub f_stop: Option<f64>,
    pub focus: Option<(Value, Span)>,
    pub blades: u32,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum EnvSpec {
    /// `night`: the same sky model lit by a moon instead of the sun (see `lower`).
    Sky { elevation: f64, azimuth: f64, turbidity: f64, sun_size: f64, sun_strength: f64, sky_strength: f64, ground: Vec3, night: bool },
    Uniform(Vec3),
    Gradient { top: Vec3, horizon: Vec3, bottom: Vec3 },
}

#[derive(Clone, Debug)]
pub struct RenderSpec {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub max_depth: u32,
    pub seed: u64,
    pub noise_target: f32,
    pub exposure: Option<f32>,
    pub exposure_bias: f32,
    pub tonemap: Tonemap,
    pub white_balance: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub caustics: bool,
}

impl Default for RenderSpec {
    fn default() -> RenderSpec {
        RenderSpec { width: 1280, height: 720, samples: 64, max_depth: 12, seed: 1, noise_target: 0.0, exposure: None, exposure_bias: 0.0, tonemap: Tonemap::Filmic, white_balance: 6500.0, saturation: 1.0, contrast: 1.0, caustics: false }
    }
}

pub(crate) struct Prop {
    pub(crate) key: String,
    pub(crate) key_span: Span,
    pub(crate) value: Value,
    pub(crate) span: Span,
    pub(crate) used: bool,
}

pub(crate) struct Props {
    pub(crate) list: Vec<Prop>,
    /// Properties whose value could not be evaluated (already reported). Code that would
    /// complain about consequences of a missing property checks this to avoid piling on.
    pub(crate) failed: Vec<String>,
}

impl Props {
    /// Consume a property (under any of its aliases). A property can be taken only once:
    /// a second `take` returns `None`, so a handler that has dealt with a property can
    /// rely on later, more general code not applying it again.
    pub(crate) fn take(&mut self, keys: &[&str]) -> Option<(Value, Span)> {
        let mut found = None;
        for p in self.list.iter_mut() {
            if !p.used && keys.contains(&p.key.as_str()) {
                p.used = true;
                if found.is_none() {
                    found = Some((p.value.clone(), p.span));
                }
            }
        }
        found
    }
    /// Is the property present and not yet consumed?
    pub(crate) fn has(&self, keys: &[&str]) -> bool {
        self.list.iter().any(|p| !p.used && keys.contains(&p.key.as_str()))
    }
}

pub struct Ev {
    pub diags: Vec<Diag>,
    pub(crate) scopes: Vec<HashMap<String, Value>>,
    pub nodes: Vec<SNode>,
    pub(crate) frames: Vec<Frame>,
    pub mats: Vec<MatSpec>,
    pub(crate) mat_names: HashMap<String, usize>,
    pub(crate) defines: HashMap<String, Rc<Define>>,
    pub camera: Option<CameraSpec>,
    pub env: Option<EnvSpec>,
    pub render: RenderSpec,
    steps: u64,
    pub(crate) define_depth: u32,
    pub(crate) quiet_renames: u32,
    pub(crate) counters: HashMap<String, u32>,
    aborted: bool,
}

pub(crate) fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Num(n) => Some(*n != 0.0),
        _ => None,
    }
}

fn bin_to_op(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Add,
        BinOp::Sub => Op::Sub,
        BinOp::Mul => Op::Mul,
        BinOp::Div => Op::Div,
        BinOp::Mod => Op::Mod,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::And => Op::And,
        BinOp::Or => Op::Or,
    }
}

impl Default for Ev {
    fn default() -> Self {
        Self::new()
    }
}

impl Ev {
    pub fn new() -> Ev {
        Ev {
            diags: Vec::new(),
            scopes: vec![HashMap::new()],
            nodes: Vec::new(),
            frames: vec![Frame { names: HashMap::new(), node: None, prefix: String::new() }],
            mats: Vec::new(),
            mat_names: HashMap::new(),
            defines: HashMap::new(),
            camera: None,
            env: None,
            render: RenderSpec::default(),
            steps: 0,
            define_depth: 0,
            quiet_renames: 0,
            counters: HashMap::new(),
            aborted: false,
        }
    }

    pub(crate) fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>, hint: impl Into<String>) {
        if self.diags.len() < 200 {
            self.diags.push(Diag::error(code, span, msg).hint(hint));
        }
    }
    pub(crate) fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>, hint: impl Into<String>) {
        if self.diags.len() < 200 {
            self.diags.push(Diag::warning(code, span, msg).hint(hint));
        }
    }

    // ------------------------------------------------------------------ names

    fn all_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in &self.scopes {
            v.extend(s.keys().cloned());
        }
        for f in &self.frames {
            v.extend(f.names.keys().cloned());
        }
        v.extend(self.mat_names.keys().cloned());
        v.extend(presets::preset_names().map(String::from));
        v.extend(color::NAMED.iter().map(|c| c.0.to_string()));
        v.extend(INPUT_NAMES.iter().map(|c| c.0.to_string()));
        v.extend(["pi", "tau"].map(String::from));
        v.extend(OPTION_WORDS.iter().map(|s| s.to_string()));
        v.extend(OBJECT_KINDS.iter().map(|k| k.0.to_string()));
        v
    }

    /// What a name refers to, or `None`; reports nothing.
    pub(crate) fn try_lookup(&self, name: &str) -> Option<Value> {
        for s in self.scopes.iter().rev() {
            if let Some(v) = s.get(name) {
                return Some(v.clone());
            }
        }
        for f in self.frames.iter().rev() {
            if let Some(&i) = f.names.get(name) {
                return Some(Value::Object(i));
            }
        }
        if let Some(&i) = self.mat_names.get(name) {
            return Some(Value::Material(i));
        }
        if let Some((_, i, _)) = INPUT_NAMES.iter().find(|x| x.0 == name) {
            return Some(Value::Tex(Rc::new(Node::Input(*i))));
        }
        match name {
            "pi" => return Some(Value::Num(std::f64::consts::PI)),
            "tau" => return Some(Value::Num(std::f64::consts::TAU)),
            _ => {}
        }
        // Preset names, colour names, kind names and option words are passed on as words; the
        // property that receives them decides what they mean. (So  shape: rect  and
        // exposure: auto  work without quotes, which is how they are naturally written.)
        if presets::canonical(name).is_some() || color::named(name).is_some() || OPTION_WORDS.contains(&name) || OBJECT_KINDS.iter().any(|k| k.0 == name) {
            return Some(Value::Str(name.to_string()));
        }
        None
    }

    fn lookup(&mut self, name: &str, span: Span) -> Option<Value> {
        if let Some(v) = self.try_lookup(name) {
            return Some(v);
        }
        let names = self.all_names();
        // a misspelling of exactly one existing name is read as that name
        if let Some(sg) = suggest_unique(name, names.iter().map(|s| s.as_str()), 2).map(String::from) {
            if let Some(v) = self.try_lookup(&sg) {
                self.warn("W346", span, format!("unknown name '{name}'; it was read as '{sg}'"), format!("write  {sg}"));
                return Some(v);
            }
        }
        // The hint must not suggest code to paste: in the first experiment both small models
        // copied "let name = value" literally and produced new syntax errors.
        let hint = match suggest(name, names.iter().map(|s| s.as_str())) {
            Some(s) => format!("did you mean '{s}'?"),
            None => format!("nothing called '{name}' exists above this line. Use the name of an object created earlier, or a number"),
        };
        self.error("E201", span, format!("unknown name '{name}'"), hint);
        None
    }

    // ------------------------------------------------------------ expressions

    pub fn eval(&mut self, e: &Expr) -> Option<Value> {
        match &e.kind {
            ExprKind::Num(n) => Some(Value::Num(*n)),
            ExprKind::Str(s) => Some(Value::Str(s.clone())),
            ExprKind::Bool(b) => Some(Value::Bool(*b)),
            ExprKind::Color(c) => Some(Value::Vec3(*c)),
            ExprKind::Ident(name) => self.lookup(name, e.span),
            ExprKind::List(items) => {
                let vals: Option<Vec<Value>> = items.iter().map(|i| self.eval(i)).collect();
                let vals = vals?;
                if vals.len() == 3 {
                    if let [Value::Num(a), Value::Num(b), Value::Num(c)] = vals.as_slice() {
                        return Some(Value::Vec3(vec3(*a as f32, *b as f32, *c as f32)));
                    }
                    if vals.iter().any(|v| v.is_tex()) && vals.iter().all(|v| matches!(v, Value::Num(_) | Value::Tex(_))) {
                        let nodes: Vec<Rc<Node>> = vals.iter().map(|v| v.as_node().unwrap()).collect();
                        return Some(Value::Tex(make_op(Op::MakeVec, nodes)));
                    }
                }
                Some(Value::List(vals))
            }
            ExprKind::Unary(op, inner) => {
                let v = self.eval(inner)?;
                match (op, &v) {
                    (UnOp::Neg, Value::Num(n)) => Some(Value::Num(-n)),
                    (UnOp::Neg, Value::Vec3(a)) => Some(Value::Vec3(-*a)),
                    (UnOp::Neg, Value::Tex(n)) => Some(Value::Tex(make_op(Op::Neg, vec![n.clone()]))),
                    (UnOp::Not, Value::Tex(n)) => Some(Value::Tex(make_op(Op::Not, vec![n.clone()]))),
                    (UnOp::Not, _) if truthy(&v).is_some() => Some(Value::Bool(!truthy(&v).unwrap())),
                    _ => {
                        let what = if *op == UnOp::Neg { "negate" } else { "apply 'not' to" };
                        self.error("E204", e.span, format!("cannot {what} a {}", v.type_name()), "this operator needs a number, a vector or a texture expression");
                        None
                    }
                }
            }
            ExprKind::Binary(op, l, r) => {
                let (lv, rv) = (self.eval(l), self.eval(r));
                self.binary(*op, lv?, rv?, e.span)
            }
            ExprKind::Call { name, name_span, args, kwargs } => self.call(name, *name_span, args, kwargs, e.span),
            ExprKind::Index(base, idx) => {
                let (b, i) = (self.eval(base), self.eval(idx));
                let (b, i) = (b?, i?);
                let Value::Num(n) = i else {
                    self.error("E204", idx.span, format!("an index must be a number, not a {}", i.type_name()), "use 0, 1 or 2");
                    return None;
                };
                let k = n as i64;
                match &b {
                    Value::Vec3(v) if (0..3).contains(&k) => Some(Value::Num(v[k as usize] as f64)),
                    Value::List(l) if k >= 0 && (k as usize) < l.len() => Some(l[k as usize].clone()),
                    Value::Tex(t) if (0..3).contains(&k) => Some(Value::Tex(make_op([Op::GetX, Op::GetY, Op::GetZ][k as usize], vec![t.clone()]))),
                    Value::Vec3(_) | Value::List(_) | Value::Tex(_) => {
                        self.error("E205", idx.span, format!("index {k} is out of range"), "indices start at 0");
                        None
                    }
                    _ => {
                        self.error("E204", base.span, format!("a {} cannot be indexed", b.type_name()), "only lists and vectors have elements");
                        None
                    }
                }
            }
            ExprKind::Member(base, field, fspan) => {
                let b = self.eval(base)?;
                self.member(&b, field, *fspan)
            }
        }
    }

    fn member(&mut self, b: &Value, field: &str, span: Span) -> Option<Value> {
        let comp = match field {
            "x" | "r" => Some(0),
            "y" | "g" => Some(1),
            "z" | "b" => Some(2),
            _ => None,
        };
        match (b, comp) {
            (Value::Vec3(v), Some(k)) => return Some(Value::Num(v[k] as f64)),
            (Value::Tex(t), Some(k)) => return Some(Value::Tex(make_op([Op::GetX, Op::GetY, Op::GetZ][k], vec![t.clone()]))),
            _ => {}
        }
        if let Value::Object(i) = b {
            // a child of a group: chair.seat
            if let Some(&c) = self.nodes[*i].children.iter().find(|&&c| self.nodes[c].name == field) {
                return Some(Value::Object(c));
            }
            let bb = self.bounds_of(*i, span)?;
            let (c, s) = (bb.center(), bb.size());
            let fields = ["top", "bottom", "left", "right", "front", "back", "center", "size", "min", "max", "x", "y", "z", "width", "height", "depth"];
            return Some(match field {
                "top" => Value::Num(bb.max.y as f64),
                "bottom" => Value::Num(bb.min.y as f64),
                "left" => Value::Num(bb.min.x as f64),
                "right" => Value::Num(bb.max.x as f64),
                "front" => Value::Num(bb.max.z as f64),
                "back" => Value::Num(bb.min.z as f64),
                "center" => Value::Vec3(c),
                "size" => Value::Vec3(s),
                "min" => Value::Vec3(bb.min),
                "max" => Value::Vec3(bb.max),
                "x" => Value::Num(c.x as f64),
                "y" => Value::Num(c.y as f64),
                "z" => Value::Num(c.z as f64),
                "width" => Value::Num(s.x as f64),
                "height" => Value::Num(s.y as f64),
                "depth" => Value::Num(s.z as f64),
                _ => {
                    let children: Vec<String> = self.nodes[*i].children.iter().map(|&c| self.nodes[c].name.clone()).collect();
                    let hint = match suggest(field, fields.iter().copied().chain(children.iter().map(|s| s.as_str()))) {
                        Some(s) => format!("did you mean '.{s}'?"),
                        None => format!("an object has: {}", fields.join(", ")),
                    };
                    let path = self.nodes[*i].path.clone();
                    self.error("E209", span, format!("'{path}' has no part or measurement called '{field}'"), hint);
                    return None;
                }
            });
        }
        self.error("E209", span, format!("a {} has no field '{field}'", b.type_name()), "vectors have .x .y .z; objects have .top .bottom .center .size and more");
        None
    }

    fn binary(&mut self, op: BinOp, l: Value, r: Value, span: Span) -> Option<Value> {
        use BinOp::*;
        if let (Value::Num(a), Value::Num(b)) = (&l, &r) {
            let (a, b) = (*a, *b);
            return Some(match op {
                Add => Value::Num(a + b),
                Sub => Value::Num(a - b),
                Mul => Value::Num(a * b),
                Div | Mod if b == 0.0 => {
                    self.error("E208", span, "division by zero", "check the divisor; inside a loop from 0 the first value of the loop variable is 0");
                    return None;
                }
                Div => Value::Num(a / b),
                Mod => Value::Num(a - b * (a / b).floor()),
                Lt => Value::Bool(a < b),
                Le => Value::Bool(a <= b),
                Gt => Value::Bool(a > b),
                Ge => Value::Bool(a >= b),
                Eq => Value::Bool(a == b),
                Ne => Value::Bool(a != b),
                And => Value::Bool(a != 0.0 && b != 0.0),
                Or => Value::Bool(a != 0.0 || b != 0.0),
            });
        }
        if matches!(op, And | Or | Eq | Ne) {
            if let (Some(a), Some(b)) = (truthy(&l), truthy(&r)) {
                return Some(Value::Bool(match op {
                    And => a && b,
                    Or => a || b,
                    Eq => a == b,
                    _ => a != b,
                }));
            }
        }
        if let (Value::Str(a), Value::Str(b)) = (&l, &r) {
            match op {
                Eq => return Some(Value::Bool(a == b)),
                Ne => return Some(Value::Bool(a != b)),
                Add => return Some(Value::Str(format!("{a}{b}"))),
                _ => {}
            }
        }
        // a two-number list (used with `on:`) supports adding and scaling, for convenience
        if let (Value::List(a), Value::List(b)) = (&l, &r) {
            if a.len() == b.len() && matches!(op, Add | Sub) {
                let out: Option<Vec<Value>> = a.iter().zip(b).map(|(x, y)| self.binary(op, x.clone(), y.clone(), span)).collect();
                return out.map(Value::List);
            }
        }
        if let (Value::List(a), Value::Num(_)) = (&l, &r) {
            if matches!(op, Mul | Div) {
                let out: Option<Vec<Value>> = a.iter().map(|x| self.binary(op, x.clone(), r.clone(), span)).collect();
                return out.map(Value::List);
            }
        }
        if let (Some(a), Some(b)) = (l.as_node(), r.as_node()) {
            let vector = matches!(l, Value::Vec3(_)) || matches!(r, Value::Vec3(_));
            if vector && !l.is_tex() && !r.is_tex() && matches!(op, Lt | Le | Gt | Ge | And | Or) {
                self.error("E204", span, "vectors cannot be compared with < or >", "compare one component, for example  v.y > 0");
                return None;
            }
            let scalar = yields_scalar(bin_to_op(op)) == Some(true);
            return Some(node_to_value(make_op(bin_to_op(op), vec![a, b]), vector && !scalar));
        }
        let sym = match op {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Mod => "%",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Eq => "==",
            Ne => "!=",
            And => "and",
            Or => "or",
        };
        let hint = if matches!(l, Value::Object(_)) || matches!(r, Value::Object(_)) {
            "an object is not a number; use one of its measurements, for example  table.top  or  table.center"
        } else {
            "both sides must be numbers, vectors or texture expressions"
        };
        self.error("E204", span, format!("cannot apply '{sym}' to a {} and a {}", l.type_name(), r.type_name()), hint);
        None
    }

    fn call(&mut self, name: &str, name_span: Span, args: &[Expr], kwargs: &[(String, Span, Expr)], span: Span) -> Option<Value> {
        let vals: Vec<Option<Value>> = args.iter().map(|a| self.eval(a)).collect();
        let kvals: Vec<(String, Span, Option<Value>)> = kwargs.iter().map(|(k, s, e)| (k.clone(), *s, self.eval(e))).collect();
        let fractal = FRACTAL_FUNCTIONS.iter().find(|f| f.0 == name);
        let vm = VM_FUNCTIONS.iter().find(|f| f.name == name);
        let params: Option<&[(&str, Option<f32>)]> = fractal.map(|_| FRACTAL_PARAMS).or(vm.map(|f| f.params));
        let Some(params) = params else {
            if CONST_FUNCTIONS.iter().any(|f| f.0 == name) {
                let v: Option<Vec<Value>> = vals.into_iter().collect();
                if let Some((k, s, _)) = kvals.first() {
                    self.error("E207", *s, format!("{name}() does not take named arguments ('{k}')"), "pass the values in order");
                    return None;
                }
                return match call_const(name, &v?) {
                    Ok(v) => Some(v),
                    Err(m) => {
                        let doc = CONST_FUNCTIONS.iter().find(|f| f.0 == name).map(|f| f.1).unwrap_or("");
                        self.error("E207", span, m, format!("{name}: {doc}"));
                        None
                    }
                };
            }
            let names = all_function_names();
            let hint = if OBJECT_KINDS.iter().any(|k| k.0 == name) || OTHER_KINDS.contains(&name) || self.defines.contains_key(name) {
                format!("'{name}' is a kind of object, and objects are written as blocks:  {name} my_name {{ property: value }}")
            } else if presets::preset(name, None).is_some() {
                format!("'{name}' is a material preset, not a function; write  material: {name}  or define  material my_{name} {{ preset: {name}, color: ... }}")
            } else {
                match suggest(name, names.iter().copied()) {
                    Some(s) => format!("did you mean '{s}'?"),
                    None => format!("available functions: {}", names.join(", ")),
                }
            };
            self.error("E206", name_span, format!("unknown function '{name}'"), hint);
            return None;
        };
        let sig = || params.iter().map(|p| p.0).collect::<Vec<_>>().join(", ");
        if vals.len() > params.len() {
            self.error("E207", span, format!("{name}() takes at most {} arguments, but {} were given", params.len(), vals.len()), format!("signature: {name}({})", sig()));
            return None;
        }
        let mut bound: Vec<Option<Value>> = vec![None; params.len()];
        let mut ok = true;
        for (i, v) in vals.into_iter().enumerate() {
            ok &= v.is_some();
            bound[i] = v;
        }
        for (k, s, v) in kvals {
            match params.iter().position(|p| p.0 == k) {
                Some(i) => {
                    ok &= v.is_some();
                    bound[i] = v;
                }
                None => {
                    let hint = match suggest(&k, params.iter().map(|p| p.0)) {
                        Some(sg) => format!("did you mean '{sg}'? signature: {name}({})", sig()),
                        None => format!("signature: {name}({})", sig()),
                    };
                    self.error("E207", s, format!("{name}() has no argument named '{k}'"), hint);
                    ok = false;
                }
            }
        }
        if !ok {
            return None;
        }
        let mut nodes: Vec<Rc<Node>> = Vec::new();
        let mut any_vec = false;
        for (i, (pname, default)) in params.iter().enumerate() {
            let v = match bound[i].take() {
                Some(v) => v,
                None => match default {
                    Some(d) => Value::Num(*d as f64),
                    None => {
                        self.error("E207", span, format!("{name}() is missing its argument '{pname}'"), format!("signature: {name}({})", sig()));
                        return None;
                    }
                },
            };
            any_vec |= matches!(v, Value::Vec3(_));
            match v.as_node() {
                Some(n) => nodes.push(n),
                None => {
                    let hint = if matches!(v, Value::Str(_)) { "colours can be written as #rrggbb or with rgb(r, g, b)".to_string() } else { format!("'{pname}' must be a number, a vector or a texture expression") };
                    self.error("E204", span, format!("{name}(): argument '{pname}' is a {}", v.type_name()), hint);
                    return None;
                }
            }
        }
        if let Some((_, op, _)) = fractal {
            // (p, octaves, lacunarity, gain, seed): the three shape numbers must be constants
            let mut k = [0.0f32; 3];
            for (j, slot) in k.iter_mut().enumerate() {
                match &*nodes[j + 1] {
                    Node::Const(c) => *slot = c.x,
                    _ => {
                        self.error("E204", span, format!("{name}(): '{}' must be a plain number, not a texture expression", params[j + 1].0), "only the position argument may vary across the surface");
                        return None;
                    }
                }
            }
            let packed = Rc::new(Node::Const(vec3(k[0].clamp(1.0, 12.0), k[1], k[2])));
            return Some(node_to_value(make_op(*op, vec![nodes[0].clone(), packed, nodes[4].clone()]), false));
        }
        let f = vm.unwrap();
        let vector = match yields_scalar(f.op) {
            Some(true) => false,
            Some(false) => true,
            None => any_vec || f.vector_result,
        };
        Some(node_to_value(make_op(f.op, nodes), vector))
    }

    // ------------------------------------------------------------- statements

    pub fn run(&mut self, prog: &[Stmt]) {
        for s in prog {
            if self.aborted {
                break;
            }
            self.stmt(s);
        }
    }

    fn tick(&mut self, span: Span) -> bool {
        self.steps += 1;
        if self.steps > MAX_STEPS || self.nodes.len() > MAX_NODES {
            if !self.aborted {
                self.aborted = true;
                self.error("E210", span, format!("the program is too large to evaluate (more than {MAX_STEPS} statements or {MAX_NODES} objects)"), "reduce loop counts");
            }
            return false;
        }
        true
    }

    pub(crate) fn items(&mut self, items: &[Item]) {
        for it in items {
            if self.aborted {
                return;
            }
            match it {
                Item::Stmt(s) => self.stmt(s),
                Item::Prop { key, key_span, .. } => {
                    self.error("E232", *key_span, format!("the property '{key}' is not inside an object"), "properties belong inside a block:  kind name { property: value }");
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, span, value } => {
                if !self.tick(*span) {
                    return;
                }
                if let Some(v) = self.eval(value) {
                    self.scopes.last_mut().unwrap().insert(name.clone(), v);
                }
            }
            Stmt::For { var, start, end, body } => {
                let (a, b) = (self.eval(start), self.eval(end));
                let (Some(Value::Num(a)), Some(Value::Num(b))) = (&a, &b) else {
                    if a.is_some() && b.is_some() {
                        self.error("E204", start.span.to(end.span), "the bounds of a 'for' range must be numbers", "write: for i in 0..5 { ... }");
                    }
                    return;
                };
                let (a, b) = (a.round() as i64, b.round() as i64);
                if b - a > 100_000 {
                    self.error("E210", start.span.to(end.span), format!("this loop would run {} times (limit 100000)", b - a), "reduce the range");
                    return;
                }
                self.quiet_renames += 1;
                for i in a..b {
                    if self.aborted || !self.tick(start.span) {
                        break;
                    }
                    self.scopes.push(HashMap::from([(var.clone(), Value::Num(i as f64))]));
                    self.items(body);
                    self.scopes.pop();
                }
                self.quiet_renames -= 1;
            }
            Stmt::If { cond, then, otherwise } => {
                let Some(c) = self.eval(cond) else { return };
                match truthy(&c) {
                    Some(t) => {
                        self.scopes.push(HashMap::new());
                        self.items(if t { then } else { otherwise });
                        self.scopes.pop();
                    }
                    None => self.error("E204", cond.span, format!("an 'if' condition must be true/false or a number, not a {}", c.type_name()), "compare two numbers, for example  if i % 2 == 0 { ... }"),
                }
            }
            Stmt::Define { name, span, params, body } => {
                if OBJECT_KINDS.iter().any(|k| k.0 == name) || OTHER_KINDS.contains(&name.as_str()) {
                    self.error("E233", *span, format!("'{name}' is a built-in kind and cannot be redefined"), format!("choose another name, for example my_{name}"));
                    return;
                }
                self.defines.insert(name.clone(), Rc::new(Define { params: params.clone(), body: body.clone() }));
            }
            Stmt::Block { kind, kind_span, name, body, span } => {
                if !self.tick(*span) {
                    return;
                }
                self.block(kind, *kind_span, name.as_ref(), body, *span);
            }
        }
    }

    /// Evaluate the properties of a block. Nested statements create children when
    /// `children` is true; otherwise only `let` is allowed among the properties.
    pub(crate) fn props(&mut self, kind: &str, body: &[Item], children: bool) -> Props {
        let mut list: Vec<Prop> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for it in body {
            match it {
                Item::Prop { key, key_span, value } => {
                    let mut key = canonical_key(kind, key).to_string();
                    // a misspelt property that can only mean one thing is read as that thing
                    let known = crate::kinds::known_props(kind);
                    if !known.is_empty() && !known.contains(&key.as_str()) && !crate::kinds::ALIAS_PROPS.contains(&key.as_str()) {
                        if let Some(sg) = suggest_unique(&key, known.iter().copied(), 2) {
                            let already = body.iter().any(|b| matches!(b, Item::Prop { key: k, .. } if canonical_key(kind, k) == sg));
                            if !already {
                                self.warn("W345", *key_span, format!("'{kind}' has no property '{key}'; it was read as '{sg}'"), format!("write  {sg}"));
                                key = sg.to_string();
                            }
                        }
                    }
                    let key = &key;
                    if let Some(prev) = list.iter().position(|p| p.key == *key) {
                        self.warn("W302", *key_span, format!("property '{key}' is set twice; the last value is used"), "remove one of them");
                        list.remove(prev);
                    }
                    // A bare word given as a material or a colour is passed on as a word even
                    // when nothing has that name: the property knows the vocabulary and can
                    // say "there is no material 'snow'" instead of "unknown name".
                    let v = match &value.kind {
                        ExprKind::Ident(word) if WORD_PROPS.contains(&key.as_str()) && self.try_lookup(word).is_none() => Some(Value::Str(word.clone())),
                        _ => self.eval(value),
                    };
                    match v {
                        Some(v) => list.push(Prop { key: key.clone(), key_span: *key_span, value: v, span: value.span, used: false }),
                        // the value failed to evaluate (already reported): the default applies
                        None => failed.push(key.clone()),
                    }
                }
                Item::Stmt(s @ Stmt::Let { .. }) => self.stmt(s),
                Item::Stmt(s) => {
                    if children {
                        self.stmt(s);
                    } else {
                        let sp = match s {
                            Stmt::Block { span, .. } => *span,
                            Stmt::Define { span, .. } => *span,
                            Stmt::For { start, .. } => start.span,
                            Stmt::If { cond, .. } => cond.span,
                            Stmt::Let { span, .. } => *span,
                        };
                        self.error("E231", sp, format!("a '{kind}' cannot contain other objects or statements"), "put the objects in a  group name { ... }  or write them after this block");
                    }
                }
            }
        }
        Props { list, failed }
    }

    pub(crate) fn finish(&mut self, kind: &str, props: Props, known: &[&str]) {
        for p in props.list.iter().filter(|p| !p.used) {
            let mut hint = match suggest(&p.key, known.iter().copied()) {
                Some(s) => format!("did you mean '{s}'? "),
                None => String::new(),
            };
            hint.push_str(&format!("properties of {kind}: {}", known.join(", ")));
            self.warn("W345", p.key_span, format!("'{kind}' has no property '{}'; it was ignored", p.key), hint);
        }
    }

    // ---------------------------------------------------------- typed getters

    pub(crate) fn num(&mut self, props: &mut Props, keys: &[&str], default: f64, lo: f64, hi: f64) -> f64 {
        match props.take(keys) {
            None => default,
            Some((Value::Num(n), span)) => {
                if n.is_finite() && n >= lo && n <= hi {
                    n
                } else {
                    self.error("E205", span, format!("'{}' is {n}, but it must be between {lo} and {hi}", keys[0]), format!("use a value between {lo} and {hi}"));
                    default
                }
            }
            Some((v, span)) => {
                self.error("E204", span, format!("'{}' must be a number, not a {}", keys[0], v.type_name()), "write a plain number, for example 0.5");
                default
            }
        }
    }

    pub(crate) fn opt_num(&mut self, props: &mut Props, keys: &[&str], lo: f64, hi: f64) -> Option<f64> {
        if props.has(keys) {
            let n = self.num(props, keys, f64::NAN, lo, hi);
            if n.is_nan() {
                None
            } else {
                Some(n)
            }
        } else {
            None
        }
    }

    pub(crate) fn boolean(&mut self, props: &mut Props, keys: &[&str], default: bool) -> bool {
        match props.take(keys) {
            None => default,
            Some((v, span)) => match truthy(&v) {
                Some(b) => b,
                None => {
                    self.error("E204", span, format!("'{}' must be true or false, not a {}", keys[0], v.type_name()), "write true or false");
                    default
                }
            },
        }
    }

    pub(crate) fn as_color(&mut self, v: &Value, span: Span, what: &str) -> Option<Vec3> {
        match v {
            Value::Vec3(c) => {
                if c.min_comp() < 0.0 || !c.is_finite() {
                    self.error("E205", span, format!("'{what}' has a negative or invalid component"), "colour components are between 0 and 1");
                    return None;
                }
                Some(*c)
            }
            Value::Str(s) => {
                if let Some(c) = color::named(s) {
                    return Some(c);
                }
                if let Some((c, _)) = color::parse_hex(s) {
                    return Some(c);
                }
                // a near miss is read as the colour that was meant; anything else becomes grey
                match suggest(s, color::NAMED.iter().map(|c| c.0)) {
                    Some(sg) => {
                        self.warn("W340", span, format!("there is no colour '{s}'; it was read as '{sg}'"), format!("write  {what}: {sg}"));
                        color::named(sg)
                    }
                    None => {
                        self.warn("W340", span, format!("there is no colour '{s}'; grey was used instead"), format!("write a colour name (red, white, navy, darkgreen ...) or  {what}: #rrggbb"));
                        Some(Vec3::splat(0.2))
                    }
                }
            }
            Value::Material(i) => match &self.mats[*i].base {
                PV::Const(c) => Some(*c),
                PV::Tex(_) => {
                    let name = self.mats[*i].name.clone();
                    self.error("E204", span, format!("'{what}' needs a colour, but '{name}' is a textured material"), "write a colour such as #8a5a33");
                    None
                }
            },
            Value::Num(n) if (0.0..=1.0).contains(n) => Some(Vec3::splat(*n as f32)),
            _ => {
                self.error("E204", span, format!("'{what}' must be a colour, not a {}", v.type_name()), "write a colour as #rrggbb, rgb(r, g, b), or a colour name");
                None
            }
        }
    }

    pub(crate) fn color(&mut self, props: &mut Props, keys: &[&str]) -> Option<Vec3> {
        let (v, span) = props.take(keys)?;
        self.as_color(&v, span, keys[0])
    }

    /// A material parameter: number, colour or texture expression.
    pub(crate) fn pv(&mut self, props: &mut Props, keys: &[&str], colour: bool, lo: f32, hi: f32) -> Option<PV> {
        let (v, span) = props.take(keys)?;
        match &v {
            Value::Tex(n) => Some(PV::Tex(n.clone())),
            Value::Num(n) if !colour => {
                if (*n as f32) < lo || (*n as f32) > hi || !n.is_finite() {
                    self.error("E205", span, format!("'{}' is {n}, but it must be between {lo} and {hi}", keys[0]), format!("use a value between {lo} and {hi}"));
                    return None;
                }
                Some(PV::num(*n as f32))
            }
            _ if colour => self.as_color(&v, span, keys[0]).map(PV::Const),
            _ => {
                self.error("E204", span, format!("'{}' must be a number or a texture expression, not a {}", keys[0], v.type_name()), "for example 0.4, or  mix(0.2, 0.8, noise(P * 10))");
                None
            }
        }
    }

    pub(crate) fn as_vec3(&mut self, v: &Value, span: Span, what: &str, allow_scalar: bool) -> Option<Vec3> {
        match v {
            Value::Vec3(a) if a.is_finite() => Some(*a),
            Value::Num(n) if allow_scalar && n.is_finite() => Some(Vec3::splat(*n as f32)),
            Value::Object(i) => {
                let b = self.bounds_of(*i, span)?;
                Some(b.center())
            }
            Value::List(l) => {
                self.error("E204", span, format!("'{what}' needs three numbers [x, y, z], but the list has {}", l.len()), "y is up; distances are in metres");
                None
            }
            _ => {
                self.error("E204", span, format!("'{what}' must be a vector [x, y, z], not a {}", v.type_name()), "y is up; distances are in metres");
                None
            }
        }
    }

    // ----------------------------------------------------------------- bounds

    pub(crate) fn current_frame_node(&self) -> Option<usize> {
        self.frames.last().unwrap().node
    }

    /// Bounds of node `i` expressed in the frame currently being built.
    pub(crate) fn bounds_of(&mut self, i: usize, span: Span) -> Option<Aabb> {
        let here = self.current_frame_node();
        if self.nodes[i].parent == here {
            return Some(self.nodes[i].bounds);
        }
        // walk up from the node until we reach the current frame (or the top level)
        let mut b = self.nodes[i].bounds;
        let mut p = self.nodes[i].parent;
        while let Some(pi) = p {
            if Some(pi) == here {
                return Some(b);
            }
            b = b.transformed(&self.nodes[pi].local);
            p = self.nodes[pi].parent;
        }
        if here.is_none() {
            return Some(b);
        }
        let name = self.nodes[i].path.clone();
        self.error("E220", span, format!("'{name}' is outside this group, so its position cannot be used here"), "inside a group or a define, refer only to objects of the same group; place the whole group relative to outside objects instead");
        None
    }

    pub(crate) fn target_bounds(&mut self, v: &Value, span: Span, what: &str) -> Option<Aabb> {
        match v {
            Value::Object(i) => self.bounds_of(*i, span),
            Value::Num(y) if y.is_finite() => Some(Aabb::new(vec3(0.0, *y as f32, 0.0), vec3(0.0, *y as f32, 0.0))),
            Value::Str(s) => {
                let hint = if OBJECT_KINDS.iter().any(|k| k.0 == s) {
                    format!("'{s}' is a kind of object; give the object a name ( {s} my_{s} {{ ... }} ) and refer to it by that name")
                } else {
                    "use the name of an object defined earlier, or a height in metres (for example  on: 0  for the floor)".to_string()
                };
                self.error("E204", span, format!("'{what}' needs an object, but '{s}' is not the name of an object"), hint);
                None
            }
            _ => {
                self.error("E204", span, format!("'{what}' needs the name of an object (or a height in metres), not a {}", v.type_name()), "for example  on: table");
                None
            }
        }
    }

    /// World-space bounds of a node (used once the whole program has run).
    pub fn world_bounds(&self, i: usize) -> Aabb {
        let mut b = self.nodes[i].bounds;
        let mut p = self.nodes[i].parent;
        while let Some(pi) = p {
            b = b.transformed(&self.nodes[pi].local);
            p = self.nodes[pi].parent;
        }
        b
    }

    pub fn world_xform(&self, i: usize) -> Mat4 {
        let mut m = self.nodes[i].local;
        let mut p = self.nodes[i].parent;
        while let Some(pi) = p {
            m = self.nodes[pi].local.mul(&m);
            p = self.nodes[pi].parent;
        }
        m
    }
}
