//! vl-lang: the VL scene language - lexer, parser, evaluator and lowering to `vl_ir::scene::Scene`.
#![forbid(unsafe_code)]

pub mod ast;
pub mod eval;
pub mod kinds;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod presets;
pub mod value;

use vl_core::diag::{self, Diag, Source};
use vl_ir::scene::Scene;

pub use lower::ObjectInfo;

/// Programs larger than this are rejected before parsing.
pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

pub struct Compiled {
    pub source: Source,
    /// Errors and warnings, in source order.
    pub diags: Vec<Diag>,
    /// The validated scene; `None` when there is any error.
    pub scene: Option<Scene>,
    /// Every named object and group, for reports and edits.
    pub objects: Vec<ObjectInfo>,
}

impl Compiled {
    pub fn ok(&self) -> bool {
        self.scene.is_some()
    }
    pub fn error_count(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == diag::Severity::Error).count()
    }
    pub fn warning_count(&self) -> usize {
        self.diags.len() - self.error_count()
    }
}

/// Compile VL source text. Never panics on any input; all problems are diagnostics.
pub fn compile(name: &str, text: &str) -> Compiled {
    let source = Source::new(name, text);
    let mut diags: Vec<Diag> = Vec::new();
    if text.len() > MAX_SOURCE_BYTES {
        diags.push(Diag::error("E001", diag::Span::new(0, 0), format!("the program is {} bytes; the limit is {MAX_SOURCE_BYTES}", text.len())).hint("use loops and 'define' instead of writing every object out"));
        return Compiled { source, diags, scene: None, objects: Vec::new() };
    }
    let toks = lexer::lex(text, &mut diags);
    let mut p = parser::Parser::new(&toks);
    let prog = p.program();
    diags.extend(p.diags);
    // evaluate even after syntax errors: the statements that did parse can still be checked
    let mut ev = eval::Ev::new();
    // where every name is introduced, for "used before it is created"
    fn names(items: &[ast::Stmt], source: &Source, out: &mut std::collections::HashMap<String, (usize, usize)>) {
        fn item_stmts(items: &[ast::Item]) -> Vec<ast::Stmt> {
            items.iter().filter_map(|i| if let ast::Item::Stmt(s) = i { Some(s.clone()) } else { None }).collect()
        }
        for st in items {
            match st {
                ast::Stmt::Let { name, span, .. } => {
                    out.entry(name.clone()).or_insert((span.start as usize, source.line_col(span.start).0));
                }
                ast::Stmt::Block { kind, kind_span, name, body, .. } => {
                    // `box table { }` introduces `table`; so does the kind-less `table { }`
                    let (n, sp) = match name {
                        Some((n, sp)) => (n.clone(), *sp),
                        None => (kind.clone(), *kind_span),
                    };
                    out.entry(n).or_insert((sp.start as usize, source.line_col(sp.start).0));
                    names(&item_stmts(body), source, out);
                }
                ast::Stmt::For { body, .. } | ast::Stmt::Define { body, .. } => names(&item_stmts(body), source, out),
                ast::Stmt::If { then, otherwise, .. } => {
                    names(&item_stmts(then), source, out);
                    names(&item_stmts(otherwise), source, out);
                }
            }
        }
    }
    names(&prog, &source, &mut ev.later);
    let prog = usable_order(prog);
    ev.run(&prog);
    let syntax_errors = diag::has_errors(&diags);
    let lowered = {
        let src = &source;
        lower::lower(&mut ev, syntax_errors, &|span| src.line_col(span.start).0)
    };
    diags.extend(ev.diags);
    diags.sort_by_key(|d| (d.span.start, d.severity != diag::Severity::Error));
    diags.dedup_by(|a, b| a.code == b.code && a.span == b.span && a.message == b.message);
    let failed = syntax_errors || diag::has_errors(&diags);
    // While there are errors, only errors are reported: a writer who is shown a warning
    // first fixes the warning and leaves the errors (observed with a 3B model).
    if failed {
        diags.retain(|d| d.severity == diag::Severity::Error);
    }
    // The same mistake on many lines is reported once, with the other lines listed.
    let diags = merge_repeats(diags, &source);
    match lowered {
        Some(l) if !failed => Compiled { source, diags, scene: Some(l.scene), objects: l.objects },
        Some(l) => Compiled { source, diags, scene: None, objects: l.objects },
        None => Compiled { source, diags, scene: None, objects: Vec::new() },
    }
}

/// Top-level statements in an order in which everything exists before it is referred to.
///
/// "A bowl on the table. The table is ..." is a natural way to describe a scene, and a
/// camera is often written first. A statement that refers to something created further
/// down is therefore read after the statement that creates it. A program that is already
/// in a usable order is not changed, and neither is one whose references go in a circle
/// (that one gets the usual error).
fn usable_order(prog: Vec<ast::Stmt>) -> Vec<ast::Stmt> {
    use ast::{Expr, ExprKind, Item, Stmt};
    use std::collections::{HashMap, HashSet};
    fn idents(e: &Expr, out: &mut HashSet<String>) {
        match &e.kind {
            ExprKind::Ident(n) => {
                out.insert(n.clone());
            }
            ExprKind::List(v) => v.iter().for_each(|x| idents(x, out)),
            ExprKind::Unary(_, a) => idents(a, out),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                idents(a, out);
                idents(b, out);
            }
            ExprKind::Call { args, kwargs, .. } => {
                args.iter().for_each(|x| idents(x, out));
                kwargs.iter().for_each(|x| idents(&x.2, out));
            }
            ExprKind::Member(a, _, _) => idents(a, out),
            _ => {}
        }
    }
    // names a statement introduces at the top level, and names it refers to
    fn scan(st: &Stmt, top: bool, intro: &mut Vec<String>, uses: &mut HashSet<String>) {
        fn items(body: &[Item], top: bool, intro: &mut Vec<String>, uses: &mut HashSet<String>) {
            for it in body {
                match it {
                    // `color: orange`, `material: brick`: a word of the language first, not a reference
                    Item::Prop { key, value, .. } if eval::WORD_PROPS.contains(&key.as_str()) && matches!(value.kind, ExprKind::Ident(_)) => {}
                    Item::Prop { value, .. } => idents(value, uses),
                    Item::Stmt(s) => scan(s, top, intro, uses),
                }
            }
        }
        match st {
            Stmt::Let { name, value, .. } => {
                idents(value, uses);
                if top {
                    intro.push(name.clone());
                }
            }
            Stmt::Block { kind, name, body, .. } => {
                if top {
                    intro.push(name.as_ref().map(|n| n.0.clone()).unwrap_or_else(|| kind.clone()));
                }
                // what is inside a group or an object is named within it, not at the top level
                items(body, false, intro, uses);
            }
            Stmt::For { start, end, body, .. } => {
                idents(start, uses);
                idents(end, uses);
                items(body, top, intro, uses);
            }
            Stmt::If { cond, then, otherwise } => {
                idents(cond, uses);
                items(then, top, intro, uses);
                items(otherwise, top, intro, uses);
            }
            Stmt::Define { body, .. } => items(body, false, intro, uses),
        }
    }
    let n = prog.len();
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut uses: Vec<HashSet<String>> = Vec::with_capacity(n);
    for (i, st) in prog.iter().enumerate() {
        let (mut intro, mut u) = (Vec::new(), HashSet::new());
        scan(st, true, &mut intro, &mut u);
        for name in &intro {
            u.remove(name); // inside a loop a name means the one made the time before
        }
        for name in intro {
            first.entry(name).or_insert(i);
        }
        uses.push(u);
    }
    // statement i waits for the statements that first make what it refers to
    let waits: Vec<Vec<usize>> = (0..n).map(|i| uses[i].iter().filter_map(|name| first.get(name).copied()).filter(|&j| j != i).collect()).collect();
    if (0..n).all(|i| waits[i].iter().all(|&j| j < i)) {
        return prog; // already usable as written
    }
    let mut done = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    while order.len() < n {
        // the earliest statement all of whose needs are met
        match (0..n).find(|&i| !done[i] && waits[i].iter().all(|&j| done[j])) {
            Some(i) => {
                done[i] = true;
                order.push(i);
            }
            None => return prog, // references in a circle: leave the program as written
        }
    }
    let mut slots: Vec<Option<ast::Stmt>> = prog.into_iter().map(Some).collect();
    order.into_iter().map(|i| slots[i].take().unwrap()).collect()
}

/// Collapse diagnostics that have the same code and message into the first one, naming the
/// other lines. Nine copies of "unknown name 'snow'" cost a small model a minute to read
/// and bury the other problems.
fn merge_repeats(diags: Vec<Diag>, source: &Source) -> Vec<Diag> {
    let mut out: Vec<Diag> = Vec::new();
    let mut extra: Vec<Vec<usize>> = Vec::new();
    for d in diags {
        match out.iter().position(|o| o.code == d.code && o.message == d.message && o.severity == d.severity) {
            Some(i) => extra[i].push(source.line_col(d.span.start).0),
            None => {
                out.push(d);
                extra.push(Vec::new());
            }
        }
    }
    for (d, lines) in out.iter_mut().zip(extra) {
        if !lines.is_empty() {
            let mut lines = lines;
            lines.dedup();
            let shown: Vec<String> = lines.iter().take(8).map(|l| l.to_string()).collect();
            let more = if lines.len() > 8 { ", ..." } else { "" };
            d.message = format!("{} (also on line{} {}{more})", d.message, if lines.len() == 1 { "" } else { "s" }, shown.join(", "));
        }
    }
    limit_per_code(out, source)
}

/// At most three warnings of one kind are spelt out; the rest become one line. A scene in
/// which twenty things overlap needs one piece of advice, not twenty.
fn limit_per_code(diags: Vec<Diag>, source: &Source) -> Vec<Diag> {
    const SHOWN: usize = 3;
    let mut seen: Vec<(&'static str, usize)> = Vec::new();
    let mut rest: Vec<(&'static str, Vec<usize>, Diag)> = Vec::new();
    let mut out: Vec<Diag> = Vec::new();
    for d in diags {
        if d.severity != vl_core::diag::Severity::Warning {
            out.push(d);
            continue;
        }
        let n = match seen.iter_mut().find(|s| s.0 == d.code) {
            Some(s) => {
                s.1 += 1;
                s.1
            }
            None => {
                seen.push((d.code, 1));
                1
            }
        };
        if n <= SHOWN {
            out.push(d);
        } else {
            let line = source.line_col(d.span.start).0;
            match rest.iter_mut().find(|r| r.0 == d.code) {
                Some(r) => r.1.push(line),
                None => rest.push((d.code, vec![line], d)),
            }
        }
    }
    for (_, mut lines, mut d) in rest {
        let n = lines.len();
        if n == 1 {
            out.push(d); // a fourth one is simply shown
            continue;
        }
        lines.sort_unstable();
        lines.dedup();
        let shown: Vec<String> = lines.iter().take(10).map(|l| l.to_string()).collect();
        d.message = format!("{n} more remarks of this kind, on line{} {}{}", if lines.len() == 1 { "" } else { "s" }, shown.join(", "), if lines.len() > 10 { ", ..." } else { "" });
        d.hint = Some("the advice above applies to each of them".to_string());
        out.push(d);
    }
    out
}
