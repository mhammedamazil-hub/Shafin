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
    match lowered {
        Some(l) if !failed => Compiled { source, diags, scene: Some(l.scene), objects: l.objects },
        Some(l) => Compiled { source, diags, scene: None, objects: l.objects },
        None => Compiled { source, diags, scene: None, objects: Vec::new() },
    }
}
