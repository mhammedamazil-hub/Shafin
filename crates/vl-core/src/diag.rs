//! Source positions and structured diagnostics.
//!
//! Every problem the compiler reports carries a stable code, an exact source
//! span and, where possible, a concrete hint. The same data is rendered for
//! humans (caret snippets) and as JSON for a language model's repair loop.
use crate::json::Json;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, end: end as u32 }
    }
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub hint: Option<String>,
}

impl Diag {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Diag {
        Diag { severity: Severity::Error, code, message: message.into(), span, hint: None }
    }
    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Diag {
        Diag { severity: Severity::Warning, code, message: message.into(), span, hint: None }
    }
    pub fn hint(mut self, h: impl Into<String>) -> Diag {
        self.hint = Some(h.into());
        self
    }
}

pub struct Source {
    pub name: String,
    pub text: String,
    line_starts: Vec<u32>,
}

impl Source {
    pub fn new(name: &str, text: &str) -> Source {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Source { name: name.to_string(), text: text.to_string(), line_starts }
    }
    /// 1-based (line, column) of a byte offset; columns count characters.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line] as usize;
        let end = (offset as usize).min(self.text.len());
        let col = self.text.get(start..end).map(|s| s.chars().count()).unwrap_or(0) + 1;
        (line + 1, col)
    }
    pub fn line_text(&self, line: usize) -> &str {
        let s = self.line_starts[line - 1] as usize;
        let e = self.line_starts.get(line).map(|v| *v as usize).unwrap_or(self.text.len());
        self.text[s..e].trim_end_matches(['\n', '\r'])
    }
}

/// Human-readable rendering with a caret under the offending span.
pub fn render(diags: &[Diag], src: &Source) -> String {
    let mut out = String::new();
    for d in diags {
        let (line, col) = src.line_col(d.span.start);
        let sev = if d.severity == Severity::Error { "error" } else { "warning" };
        out.push_str(&format!("{sev}[{}]: {}\n  --> {}:{}:{}\n", d.code, d.message, src.name, line, col));
        let text = src.line_text(line);
        let width = (d.span.end.saturating_sub(d.span.start) as usize).clamp(1, text.chars().count().saturating_sub(col - 1).max(1));
        out.push_str(&format!("   | {text}\n   | {}{}\n", " ".repeat(col - 1), "^".repeat(width)));
        if let Some(h) = &d.hint {
            out.push_str(&format!("   = hint: {h}\n"));
        }
    }
    out
}

pub fn to_json(diags: &[Diag], src: &Source) -> Json {
    Json::Arr(
        diags
            .iter()
            .map(|d| {
                let (line, col) = src.line_col(d.span.start);
                let mut j = Json::obj()
                    .with("severity", if d.severity == Severity::Error { "error" } else { "warning" })
                    .with("code", d.code)
                    .with("message", d.message.as_str())
                    .with("line", line)
                    .with("col", col)
                    .with("source_line", src.line_text(line).trim());
                if let Some(h) = &d.hint {
                    j.set("hint", h.as_str());
                }
                j
            })
            .collect(),
    )
}

pub fn has_errors(diags: &[Diag]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

/// Edit distance in which swapping two adjacent letters counts as ONE edit
/// (optimal string alignment). Transpositions are the commonest typing slip:
/// "tabel" must be one step from "table", not two.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// Closest candidate for a misspelt identifier ("did you mean ...").
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lname = name.to_ascii_lowercase();
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = edit_distance(&lname, &c.to_ascii_lowercase());
        let limit = (name.len().max(c.len()) / 3).max(1);
        let better = match best {
            Some((_, bd)) => d < bd,
            None => true,
        };
        if d <= limit && better && d > 0 {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        let s = Source::new("t.vl", "ab\ncdé f\n\nx");
        assert_eq!(s.line_col(0), (1, 1));
        assert_eq!(s.line_col(3), (2, 1));
        assert_eq!(s.line_col(7), (2, 4)); // after the 2-byte é
        assert_eq!(s.line_text(2), "cdé f");
        assert_eq!(s.line_col(11), (4, 1));
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("radious", ["radius", "material", "at"]), Some("radius"));
        assert_eq!(suggest("colour", ["color", "roughness"]), Some("color"));
        assert_eq!(suggest("zzz", ["radius", "material"]), None);
    }

    #[test]
    fn rendering() {
        let s = Source::new("t.vl", "sphere { radious: 1 }");
        let d = Diag::error("E100", Span::new(9, 16), "unknown property").hint("did you mean 'radius'?");
        let txt = render(&[d.clone()], &s);
        assert!(txt.contains("t.vl:1:10") && txt.contains("^^^^^^^") && txt.contains("hint"));
        let j = to_json(&[d], &s);
        assert_eq!(j.as_arr().unwrap()[0].get("col").unwrap().as_f64(), Some(10.0));
    }

    #[test]
    fn suggestions_handle_transpositions_and_never_echo_the_input() {
        assert_eq!(edit_distance("tabel", "table"), 1);
        assert_eq!(edit_distance("raduis", "radius"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(suggest("tabel", ["chair", "table", "lamp"]), Some("table"));
        assert_eq!(suggest("size", ["size", "width"]), None, "an exact match is not a suggestion");
        assert_eq!(suggest("xyzzy", ["chair", "table"]), None);
    }
}
