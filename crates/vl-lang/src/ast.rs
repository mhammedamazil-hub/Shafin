//! Syntax tree of a VL program.
use vl_core::diag::Span;
use vl_core::math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Num(f64),
    Str(String),
    Bool(bool),
    Color(Vec3),
    Ident(String),
    List(Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call { name: String, name_span: Span, args: Vec<Expr>, kwargs: Vec<(String, Span, Expr)> },
    Index(Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String, Span),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let { name: String, span: Span, value: Expr },
    For { var: String, start: Expr, end: Expr, body: Vec<Item> },
    If { cond: Expr, then: Vec<Item>, otherwise: Vec<Item> },
    Define { name: String, span: Span, params: Vec<Param>, body: Vec<Item> },
    /// `kind name { ... }` - the name is optional.
    Block { kind: String, kind_span: Span, name: Option<(String, Span)>, body: Vec<Item>, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Prop { key: String, key_span: Span, value: Expr },
    Stmt(Stmt),
}
