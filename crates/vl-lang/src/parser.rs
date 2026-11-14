//! Recursive-descent parser with error recovery: after a syntax error it skips
//! to the next statement, so a program with several mistakes reports them all
//! in one pass (fewer repair round trips for the language model).
use crate::ast::*;
use crate::lexer::Tok;
use vl_core::diag::{Diag, Span};

pub struct Parser<'a> {
    toks: &'a [(Tok, Span)],
    pos: usize,
    pub diags: Vec<Diag>,
    depth: u32,
}

const MAX_NESTING: u32 = 64;
const KEYWORDS: &[&str] = &["let", "for", "in", "if", "else", "define", "true", "false", "and", "or", "not"];

type PResult<T> = Result<T, ()>;

impl<'a> Parser<'a> {
    pub fn new(toks: &'a [(Tok, Span)]) -> Parser<'a> {
        Parser { toks, pos: 0, diags: Vec::new(), depth: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].0
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].0
    }
    fn span(&self) -> Span {
        self.toks[self.pos.min(self.toks.len() - 1)].1
    }
    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1).min(self.toks.len() - 1)].1
    }
    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }
    fn err<T>(&mut self, code: &'static str, span: Span, msg: String, hint: &str) -> PResult<T> {
        self.diags.push(Diag::error(code, span, msg).hint(hint));
        Err(())
    }
    fn expect(&mut self, t: &Tok, context: &str) -> PResult<Span> {
        if self.at(t) {
            let s = self.span();
            self.bump();
            Ok(s)
        } else {
            let (sp, found) = (self.span(), self.peek().describe());
            self.err("E110", sp, format!("expected {} {context}, found {found}", t.describe()), &format!("insert {} here", t.describe()))
        }
    }
    fn ident(&mut self, context: &str) -> PResult<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {
                let sp = self.span();
                self.bump();
                Ok((s, sp))
            }
            other => {
                let sp = self.span();
                self.err("E111", sp, format!("expected a name {context}, found {}", other.describe()), "names start with a letter and contain letters, digits or '_'")
            }
        }
    }

    pub fn program(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        while !self.at(&Tok::Eof) {
            if self.eat(&Tok::Semi) || self.eat(&Tok::Comma) {
                continue;
            }
            let before = self.pos;
            match self.stmt() {
                Ok(s) => out.push(s),
                Err(()) => self.recover(before),
            }
        }
        out
    }

    /// Skip to something that can start a new statement at the current nesting level.
    fn recover(&mut self, before: usize) {
        if self.pos == before {
            self.bump();
        }
        let mut depth = 0i32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        return; // let the enclosing block see its closing brace
                    }
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                Tok::Ident(_) if depth == 0 => {
                    // `name :` (a property), `kind {`, `kind name {` or a keyword start a new item
                    let starts = matches!(self.peek_at(1), Tok::Colon | Tok::LBrace) || (matches!(self.peek_at(1), Tok::Ident(_) | Tok::Str(_)) && matches!(self.peek_at(2), Tok::LBrace)) || self.is_kw("let") || self.is_kw("for") || self.is_kw("if") || self.is_kw("define");
                    if starts {
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        if self.is_kw("let") {
            self.bump();
            let (name, span) = self.ident("after 'let'")?;
            self.expect(&Tok::Assign, "after the variable name")?;
            let value = self.expr()?;
            return Ok(Stmt::Let { name, span, value });
        }
        if self.is_kw("for") {
            self.bump();
            let (var, _) = self.ident("after 'for'")?;
            if !self.is_kw("in") {
                let sp = self.span();
                return self.err("E112", sp, format!("expected 'in' after the loop variable, found {}", self.peek().describe()), "write: for i in 0..5 { ... }");
            }
            self.bump();
            let start = self.expr()?;
            self.expect(&Tok::DotDot, "in the loop range")?;
            let end = self.expr()?;
            let body = self.block()?;
            return Ok(Stmt::For { var, start, end, body });
        }
        if self.is_kw("if") {
            self.bump();
            let cond = self.expr()?;
            let then = self.block()?;
            let mut otherwise = Vec::new();
            if self.is_kw("else") {
                self.bump();
                if self.is_kw("if") {
                    otherwise.push(Item::Stmt(self.stmt()?));
                } else {
                    otherwise = self.block()?;
                }
            }
            return Ok(Stmt::If { cond, then, otherwise });
        }
        if self.is_kw("define") {
            self.bump();
            let (name, span) = self.ident("after 'define'")?;
            let mut params = Vec::new();
            if self.eat(&Tok::LParen) {
                while !self.at(&Tok::RParen) && !self.at(&Tok::Eof) {
                    let (pn, ps) = self.ident("for a parameter")?;
                    let default = if self.eat(&Tok::Assign) || self.eat(&Tok::Colon) { Some(self.expr()?) } else { None };
                    params.push(Param { name: pn, span: ps, default });
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RParen, "to close the parameter list")?;
            }
            let body = self.block()?;
            return Ok(Stmt::Define { name, span, params, body });
        }
        // kind [name] { ... }
        let start = self.span();
        let (kind, kind_span) = self.ident("to start a statement")?;
        let name = match self.peek().clone() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {
                let sp = self.span();
                self.bump();
                Some((s, sp))
            }
            Tok::Str(s) => {
                let sp = self.span();
                self.bump();
                Some((s, sp))
            }
            _ => None,
        };
        if !self.at(&Tok::LBrace) {
            let (sp, found) = (self.span(), self.peek().describe());
            let hint = if matches!(self.peek(), Tok::Assign) {
                "to define an object write:  kind name { property: value }   - to define a variable write:  let name = value"
            } else if matches!(self.peek(), Tok::LParen) {
                "objects are written as blocks, not calls:  kind name { property: value }"
            } else {
                "a statement has the form:  kind name { property: value }"
            };
            return self.err("E113", sp, format!("expected '{{' to start the body of '{kind}', found {found}"), hint);
        }
        let body = self.block()?;
        Ok(Stmt::Block { kind, kind_span, name, body, span: start.to(self.prev_span()) })
    }

    fn block(&mut self) -> PResult<Vec<Item>> {
        let open = self.expect(&Tok::LBrace, "to open a block")?;
        self.depth += 1;
        if self.depth > MAX_NESTING {
            self.depth -= 1;
            return self.err("E114", open, format!("blocks are nested more than {MAX_NESTING} levels deep"), "flatten the program");
        }
        let mut items = Vec::new();
        loop {
            if self.eat(&Tok::Comma) || self.eat(&Tok::Semi) {
                continue;
            }
            if self.eat(&Tok::RBrace) {
                break;
            }
            if self.at(&Tok::Eof) {
                self.depth -= 1;
                return self.err("E115", open, "this '{' is never closed".to_string(), "add a matching '}'");
            }
            let before = self.pos;
            // "in" is a keyword in  for i in a..b  and also a relation:  in: bowl
            let item = if matches!(self.peek(), Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) || s == "in") && matches!(self.peek_at(1), Tok::Colon | Tok::Assign) {
                // property:  key: value   (key = value is accepted too)
                let (key, key_span) = match self.peek().clone() {
                    Tok::Ident(s) => (s, self.span()),
                    _ => unreachable!("guarded by the test above"),
                };
                self.bump();
                self.bump();
                self.expr().map(|value| Item::Prop { key, key_span, value })
            } else {
                self.stmt().map(Item::Stmt)
            };
            match item {
                Ok(it) => items.push(it),
                Err(()) => self.recover(before),
            }
        }
        self.depth -= 1;
        Ok(items)
    }

    pub fn expr(&mut self) -> PResult<Expr> {
        self.depth += 1;
        if self.depth > MAX_NESTING * 4 {
            self.depth -= 1;
            let sp = self.span();
            return self.err("E114", sp, "expression is nested too deeply".to_string(), "split it using 'let'");
        }
        let r = self.binary(0);
        self.depth -= 1;
        r
    }

    fn binop(&self) -> Option<(BinOp, u8)> {
        Some(match self.peek() {
            Tok::Ident(s) if s == "or" => (BinOp::Or, 1),
            Tok::Ident(s) if s == "and" => (BinOp::And, 2),
            Tok::EqEq => (BinOp::Eq, 3),
            Tok::Ne => (BinOp::Ne, 3),
            Tok::Lt => (BinOp::Lt, 4),
            Tok::Le => (BinOp::Le, 4),
            Tok::Gt => (BinOp::Gt, 4),
            Tok::Ge => (BinOp::Ge, 4),
            Tok::Plus => (BinOp::Add, 5),
            Tok::Minus => (BinOp::Sub, 5),
            Tok::Star => (BinOp::Mul, 6),
            Tok::Slash => (BinOp::Div, 6),
            Tok::Percent => (BinOp::Mod, 6),
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = self.binop() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        if self.eat(&Tok::Minus) {
            let e = self.unary()?;
            let span = start.to(e.span);
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Neg, Box::new(e)), span });
        }
        if self.eat(&Tok::Plus) {
            return self.unary();
        }
        if self.is_kw("not") {
            self.bump();
            let e = self.unary()?;
            let span = start.to(e.span);
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(e)), span });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.at(&Tok::LBracket) && self.prev_span().end == self.span().start {
                // indexing must be written without a space: v[0]. (With a space, `a [1,2]` would
                // swallow the list that starts the next property value.)
                self.bump();
                let idx = self.expr()?;
                let close = self.expect(&Tok::RBracket, "to close the index")?;
                let span = e.span.to(close);
                e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(idx)), span };
            } else if self.at(&Tok::Dot) {
                self.bump();
                let (name, sp) = self.ident("after '.'")?;
                let span = e.span.to(sp);
                e = Expr { kind: ExprKind::Member(Box::new(e), name, sp), span };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Num(n) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Num(n), span })
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Str(s), span })
            }
            Tok::Color(c) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Color(c), span })
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.expect(&Tok::RParen, "to close the parenthesis")?;
                Ok(e)
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                while !self.at(&Tok::RBracket) && !self.at(&Tok::Eof) {
                    items.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                let close = self.expect(&Tok::RBracket, "to close the list")?;
                Ok(Expr { kind: ExprKind::List(items), span: span.to(close) })
            }
            Tok::Ident(s) => {
                if s == "true" || s == "false" {
                    self.bump();
                    return Ok(Expr { kind: ExprKind::Bool(s == "true"), span });
                }
                if KEYWORDS.contains(&s.as_str()) {
                    return self.err("E116", span, format!("'{s}' is a reserved word and cannot be used as a value"), "use a different name");
                }
                self.bump();
                if self.at(&Tok::LParen) && self.prev_span().end == self.span().start {
                    self.bump();
                    let (mut args, mut kwargs) = (Vec::new(), Vec::new());
                    while !self.at(&Tok::RParen) && !self.at(&Tok::Eof) {
                        if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Colon | Tok::Assign) {
                            let (k, ks) = self.ident("for a named argument")?;
                            self.bump();
                            kwargs.push((k, ks, self.expr()?));
                        } else {
                            args.push(self.expr()?);
                        }
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    let close = self.expect(&Tok::RParen, "to close the argument list")?;
                    return Ok(Expr { kind: ExprKind::Call { name: s, name_span: span, args, kwargs }, span: span.to(close) });
                }
                Ok(Expr { kind: ExprKind::Ident(s), span })
            }
            other => self.err("E117", span, format!("expected a value, found {}", other.describe()), "a value is a number, a [list], a \"string\", a #rrggbb colour, a name, or a function call"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse(src: &str) -> (Vec<Stmt>, Vec<Diag>) {
        let mut d = Vec::new();
        let toks = lex(src, &mut d);
        let mut p = Parser::new(&toks);
        let prog = p.program();
        d.extend(p.diags);
        (prog, d)
    }

    #[test]
    fn parses_blocks_properties_and_control_flow() {
        let src = r#"
            let n = 3
            material oak { color: #8a5a33, roughness: 0.5 }
            box table { size: [1.6, 6cm, 0.9]  at: [0, 0.75, 0]; material: oak }
            for i in 0..n {
                sphere ball { radius: 0.1 + i * 0.02, on: table, at: [i * 0.3 - 0.3, 0] }
            }
            if n > 2 { camera { look_at: table } } else { camera { look_at: [0, 0, 0] } }
            define lamp(height = 1.2, power: 800lm) { sphere bulb { radius: 5cm, at: [0, height, 0] } }
            lamp "desk lamp" { at: [1, 0, 0], height: 0.5 }
        "#;
        let (prog, d) = parse(src);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(prog.len(), 7);
        match &prog[2] {
            Stmt::Block { kind, name, body, .. } => {
                assert_eq!(kind, "box");
                assert_eq!(name.as_ref().unwrap().0, "table");
                assert_eq!(body.len(), 3, "separators between properties are optional");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&prog[3], Stmt::For { var, .. } if var == "i"));
        assert!(matches!(&prog[5], Stmt::Define { params, .. } if params.len() == 2 && params[1].default.is_some()));
        assert!(matches!(&prog[6], Stmt::Block { name: Some((n, _)), .. } if n == "desk lamp"));
    }

    #[test]
    fn expression_precedence_and_calls() {
        let (prog, d) = parse("x { a: 1 + 2 * 3 - -4, b: mix(#ff0000, [0,0,1], fbm(P * 4, octaves: 5)), c: not a < 2 and true, d: v[1] + p.x }");
        assert!(d.is_empty(), "{d:?}");
        let Stmt::Block { body, .. } = &prog[0] else { panic!() };
        let Item::Prop { value, .. } = &body[0] else { panic!() };
        // ((1 + (2*3)) - (-4))
        let ExprKind::Binary(BinOp::Sub, l, r) = &value.kind else { panic!("{value:?}") };
        assert!(matches!(&l.kind, ExprKind::Binary(BinOp::Add, _, m) if matches!(&m.kind, ExprKind::Binary(BinOp::Mul, _, _))));
        assert!(matches!(&r.kind, ExprKind::Unary(UnOp::Neg, _)));
        let Item::Prop { value, .. } = &body[1] else { panic!() };
        let ExprKind::Call { name, args, .. } = &value.kind else { panic!() };
        assert_eq!((name.as_str(), args.len()), ("mix", 3));
        assert!(matches!(&args[2].kind, ExprKind::Call { kwargs, .. } if kwargs.len() == 1 && kwargs[0].0 == "octaves"));
    }

    #[test]
    fn a_list_after_a_name_is_a_new_value_not_an_index() {
        // `material: oak` followed by a property whose value starts with '[' on the next line
        let (prog, d) = parse("box b { material: oak\n size: [1, 2, 3] }\nbox c { on: b at: [0.1, 0.2] }");
        assert!(d.is_empty(), "{d:?}");
        let Stmt::Block { body, .. } = &prog[0] else { panic!() };
        assert_eq!(body.len(), 2);
        let Stmt::Block { body, .. } = &prog[1] else { panic!() };
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn reports_several_errors_in_one_pass_and_keeps_good_statements() {
        let src = "sphere a { radius: }\nbox b { size: [1,2,3] }\ntable = box(1,2)\nsphere c { radius: 0.2 }\ncone d { height: 1";
        let (prog, d) = parse(src);
        let codes: Vec<&str> = d.iter().map(|x| x.code).collect();
        assert_eq!(codes, ["E117", "E113", "E115"], "{d:?}");
        assert!(d[1].hint.as_ref().unwrap().contains("kind name {"));
        let names: Vec<String> = prog.iter().filter_map(|s| if let Stmt::Block { name: Some((n, _)), .. } = s { Some(n.clone()) } else { None }).collect();
        assert_eq!(names, ["a", "b", "c"], "statements around the errors survive");
    }

    #[test]
    fn hostile_input_does_not_overflow_the_stack() {
        let deep = "a{".repeat(5000);
        let (_, d) = parse(&deep);
        assert!(d.iter().any(|x| x.code == "E114"));
        let deep_expr = format!("a {{ x: {}1{} }}", "(".repeat(5000), ")".repeat(5000));
        let (_, d) = parse(&deep_expr);
        assert!(d.iter().any(|x| x.code == "E114"));
    }
}
