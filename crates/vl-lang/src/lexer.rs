//! Tokens. The lexer never fails: problems become diagnostics and lexing continues,
//! so one typo does not hide every other error in the program.
use vl_core::color;
use vl_core::diag::{suggest, Diag, Span};
use vl_core::math::Vec3;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Color(Vec3),
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Assign,
    Dot,
    DotDot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("'{s}'"),
            Tok::Num(n) => format!("number {n}"),
            Tok::Str(s) => format!("string \"{s}\""),
            Tok::Color(_) => "a colour".into(),
            Tok::LBrace => "'{'".into(),
            Tok::RBrace => "'}'".into(),
            Tok::LParen => "'('".into(),
            Tok::RParen => "')'".into(),
            Tok::LBracket => "'['".into(),
            Tok::RBracket => "']'".into(),
            Tok::Comma => "','".into(),
            Tok::Semi => "';'".into(),
            Tok::Colon => "':'".into(),
            Tok::Assign => "'='".into(),
            Tok::Dot => "'.'".into(),
            Tok::DotDot => "'..'".into(),
            Tok::Plus => "'+'".into(),
            Tok::Minus => "'-'".into(),
            Tok::Star => "'*'".into(),
            Tok::Slash => "'/'".into(),
            Tok::Percent => "'%'".into(),
            Tok::Lt => "'<'".into(),
            Tok::Le => "'<='".into(),
            Tok::Gt => "'>'".into(),
            Tok::Ge => "'>='".into(),
            Tok::EqEq => "'=='".into(),
            Tok::Ne => "'!='".into(),
            Tok::Eof => "end of file".into(),
        }
    }
}

/// Unit suffixes and the factor that converts them to the language's base units
/// (metres, degrees; photometric and temperature units are already base units).
pub const UNITS: &[(&str, f64)] = &[
    ("mm", 0.001),
    ("cm", 0.01),
    ("m", 1.0),
    ("km", 1000.0),
    ("in", 0.0254),
    ("ft", 0.3048),
    ("deg", 1.0),
    ("rad", 57.295_779_513_082_32),
    ("lm", 1.0),
    ("lx", 1.0),
    ("nit", 1.0),
    ("nits", 1.0),
    ("K", 1.0),
    ("k", 1.0),
    ("ev", 1.0),
    ("EV", 1.0),
];

pub fn lex(text: &str, diags: &mut Vec<Diag>) -> Vec<(Tok, Span)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let is_ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_';
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        // comments: // ...   and   # ... (unless the '#' starts a 6- or 8-digit hex colour)
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        if c == b'#' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            let digits = j - i - 1;
            let ends = j >= b.len() || !is_ident(b[j]);
            if (digits == 6 || digits == 8) && ends {
                if let Some((rgb, _)) = color::parse_hex(&text[i..j]) {
                    out.push((Tok::Color(rgb), Span::new(i, j)));
                    i = j;
                    continue;
                }
            }
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if is_ident_start(c) {
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            out.push((Tok::Ident(text[start..i].to_string()), Span::new(start, i)));
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            // a '.' starts a fraction unless it is the range operator '..'
            if i < b.len() && b[i] == b'.' && b.get(i + 1) != Some(&b'.') {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let mut value: f64 = text[start..i].parse().unwrap_or(0.0);
            // unit suffix written directly after the number: 12cm, 45deg, 800lm
            if i < b.len() && is_ident_start(b[i]) {
                let us = i;
                while i < b.len() && is_ident(b[i]) {
                    i += 1;
                }
                let unit = &text[us..i];
                match UNITS.iter().find(|(u, _)| *u == unit) {
                    Some((_, f)) => value *= f,
                    None => {
                        let mut d = Diag::error("E101", Span::new(us, i), format!("unknown unit '{unit}'"));
                        d = match suggest(unit, UNITS.iter().map(|(u, _)| *u)) {
                            Some(s) => d.hint(format!("did you mean '{s}'? Lengths are in metres by default (also mm, cm, km, in, ft); angles are in degrees")),
                            None => d.hint("write the number without a unit (metres, degrees) or use one of: mm, cm, m, km, in, ft, deg, rad, lm, lx, K"),
                        };
                        diags.push(d);
                    }
                }
            }
            out.push((Tok::Num(value), Span::new(start, i)));
            continue;
        }
        if c == b'"' || c == b'\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < b.len() {
                if b[i] == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                if b[i] == b'\n' {
                    break;
                }
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                // copy one UTF-8 character
                let ch_len = text[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
                s.push_str(&text[i..i + ch_len]);
                i += ch_len;
            }
            if !closed {
                diags.push(Diag::error("E102", Span::new(start, i), "string is missing its closing quote").hint("add the closing quote on the same line"));
            }
            out.push((Tok::Str(s), Span::new(start, i)));
            continue;
        }
        let two = |a: u8| b.get(i + 1) == Some(&a);
        let (tok, len) = match c {
            b'{' => (Tok::LBrace, 1),
            b'}' => (Tok::RBrace, 1),
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b'[' => (Tok::LBracket, 1),
            b']' => (Tok::RBracket, 1),
            b',' => (Tok::Comma, 1),
            b';' => (Tok::Semi, 1),
            b':' => (Tok::Colon, 1),
            b'.' if two(b'.') => (Tok::DotDot, 2),
            b'.' => (Tok::Dot, 1),
            b'+' => (Tok::Plus, 1),
            b'-' => (Tok::Minus, 1),
            b'*' => (Tok::Star, 1),
            b'/' => (Tok::Slash, 1),
            b'%' => (Tok::Percent, 1),
            b'<' if two(b'=') => (Tok::Le, 2),
            b'<' => (Tok::Lt, 1),
            b'>' if two(b'=') => (Tok::Ge, 2),
            b'>' => (Tok::Gt, 1),
            b'=' if two(b'=') => (Tok::EqEq, 2),
            b'=' => (Tok::Assign, 1),
            b'!' if two(b'=') => (Tok::Ne, 2),
            _ => {
                let ch_len = text[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
                diags.push(Diag::error("E100", Span::new(i, i + ch_len), format!("unexpected character '{}'", &text[i..i + ch_len])).hint("remove this character"));
                i += ch_len;
                continue;
            }
        };
        i += len;
        out.push((tok, Span::new(start, i)));
    }
    out.push((Tok::Eof, Span::new(b.len(), b.len())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> (Vec<Tok>, Vec<Diag>) {
        let mut d = Vec::new();
        let t = lex(s, &mut d).into_iter().map(|(t, _)| t).collect();
        (t, d)
    }

    #[test]
    fn numbers_units_ranges() {
        let (t, d) = toks("12cm 1.5 .25 2e3 45deg 0..5 3.0mm 800lm 1.5e-2");
        assert!(d.is_empty());
        let nums: Vec<f64> = t.iter().filter_map(|t| if let Tok::Num(n) = t { Some(*n) } else { None }).collect();
        let want = [0.12, 1.5, 0.25, 2000.0, 45.0, 0.0, 5.0, 0.003, 800.0, 0.015];
        assert_eq!(nums.len(), want.len());
        for (a, b) in nums.iter().zip(want) {
            assert!((a - b).abs() < 1e-12, "{a} vs {b}");
        }
        assert!(t.contains(&Tok::DotDot));
        // "cmm" is one edit from both "cm" and "mm": either suggestion is fair, and the hint lists the units
        let (t, d) = toks("12cmm 3");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E101");
        let hint = d[0].hint.as_ref().unwrap();
        assert!(hint.contains("did you mean") && hint.contains("cm") && hint.contains("mm"), "{hint}");
        assert_eq!(t.iter().filter(|t| matches!(t, Tok::Num(_))).count(), 2, "lexing continues after the bad unit");
        let (_, d) = toks("5metres");
        assert!(d[0].hint.as_ref().unwrap().contains("without a unit"), "no close match: explain the defaults");
    }

    #[test]
    fn comments_and_colours() {
        let (t, d) = toks("a # a comment\nb // another\nc /* block\n comment */ d #ff8000 #bed is a comment\n#12345678");
        assert!(d.is_empty());
        let idents: Vec<&str> = t.iter().filter_map(|t| if let Tok::Ident(s) = t { Some(s.as_str()) } else { None }).collect();
        assert_eq!(idents, ["a", "b", "c", "d"]);
        assert_eq!(t.iter().filter(|t| matches!(t, Tok::Color(_))).count(), 2);
        if let Some(Tok::Color(c)) = t.iter().find(|t| matches!(t, Tok::Color(_))) {
            assert!((c.x - 1.0).abs() < 1e-6 && c.z == 0.0 && c.y > 0.2 && c.y < 0.23, "{c:?} (linear-light)");
        }
    }

    #[test]
    fn strings_operators_and_recovery() {
        let (t, d) = toks(r#"x = "hi there" <= >= == != 'single' @ ok"#);
        assert_eq!(d.len(), 1, "one diagnostic for '@'");
        assert_eq!(d[0].code, "E100");
        assert!(t.contains(&Tok::Str("hi there".into())) && t.contains(&Tok::Str("single".into())));
        assert!(t.contains(&Tok::Le) && t.contains(&Tok::Ge) && t.contains(&Tok::EqEq) && t.contains(&Tok::Ne));
        assert!(t.contains(&Tok::Ident("ok".into())), "lexing continues after an error");
        let (_, d) = toks("\"unterminated\nnext");
        assert_eq!(d[0].code, "E102");
    }
}
