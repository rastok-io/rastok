//! The human face of a diagnostic: rustc-shaped rendering with
//! source lines and underlines, hand-written over the span table.
//!
//! ```text
//! error[E-PORT-UNKNOWN]: this node exposes no port named `zed`
//!   --> probe.rkg:12:8
//!    |
//! 12 |     on zed {
//!    |        ^^^
//!    = port: zed
//! ```
//!
//! The renderer owns presentation only: codes, spans, and fields come
//! from the registry-governed [`Diagnostic`] untouched. Output is
//! plain text, byte-stable for a given input — golden-testable like
//! everything else.

use crate::diagnostics::{Diagnostic, SourceId};
use std::fmt::Write as _;

/// The compiled sources a rendering resolves spans against, in
/// [`SourceId`] order — the same order they were handed to the
/// compiler.
#[derive(Debug, Default)]
pub struct Sources {
    files: Vec<(String, String)>,
}

impl Sources {
    /// An empty source table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one file; ids are assigned in insertion order.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) {
        self.files.push((name.into(), text.into()));
    }

    fn get(&self, id: SourceId) -> Option<(&str, &str)> {
        self.files
            .get(id.index() as usize)
            .map(|(name, text)| (name.as_str(), text.as_str()))
    }
}

/// Renders one diagnostic in the rustc shape. A span whose source is
/// unknown degrades to the bare offset form rather than panicking —
/// rendering never fails.
#[must_use]
pub fn render(diagnostic: &Diagnostic, sources: &Sources) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}[{}]: {}",
        diagnostic.severity(),
        diagnostic.code(),
        diagnostic.message()
    );
    for span in diagnostic.spans() {
        let Some((name, text)) = sources.get(span.source()) else {
            let _ = writeln!(
                out,
                "  --> source#{} @ {}..{}",
                span.source().index(),
                span.range().start,
                span.range().end
            );
            continue;
        };
        let range = span.range();
        let (line_number, column, line_text) = locate(text, range.start);
        let _ = writeln!(out, "  --> {name}:{line_number}:{column}");
        let gutter = line_number.to_string().len();
        let _ = writeln!(out, "{:gutter$} |", "");
        let _ = writeln!(out, "{line_number} | {line_text}");
        let width = range
            .len()
            .clamp(1, line_text.len().saturating_sub(column - 1).max(1));
        let _ = writeln!(
            out,
            "{:gutter$} | {:pad$}{}",
            "",
            "",
            "^".repeat(width),
            pad = column - 1
        );
    }
    for (name, value) in diagnostic.fields() {
        let _ = writeln!(out, "  = {name}: {value}");
    }
    out
}

/// Renders a whole bag, one blank line between diagnostics.
#[must_use]
pub fn render_all(diagnostics: &[Diagnostic], sources: &Sources) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| render(diagnostic, sources))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 1-based line and column of a byte offset, with the line's text.
fn locate(text: &str, offset: usize) -> (usize, usize, &str) {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let line_number = text[..line_start].matches('\n').count() + 1;
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |at| line_start + at);
    let column = offset - line_start + 1;
    (line_number, column, &text[line_start..line_end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::contract::ContractCatalog;

    #[test]
    fn a_semantic_refusal_renders_with_its_source_line() {
        // A real semantic rule, not a parser accident: an emission
        // leaving by an input port.
        let source =
            "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  . -> o -> .\n}\n";
        let Err(bag) = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        ) else {
            panic!("the direction is violated");
        };
        let mut sources = Sources::new();
        sources.add("probe.rkg", source);
        let rendered = render_all(bag.diagnostics(), &sources);
        assert!(
            rendered.contains("error[E-PORT-DIRECTION]"),
            "the code leads: {rendered}"
        );
        assert!(
            rendered.contains("--> probe.rkg:3:"),
            "file, line, column: {rendered}"
        );
        assert!(
            rendered.contains("3 |     on in { in = . }"),
            "the offending line: {rendered}"
        );
        assert!(rendered.contains("^^"), "the underline: {rendered}");
    }

    #[test]
    fn rendering_is_total_even_without_sources() {
        let source =
            "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  . -> o -> .\n}\n";
        let Err(bag) = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        ) else {
            panic!("the direction is violated");
        };
        let rendered = render_all(bag.diagnostics(), &Sources::new());
        assert!(rendered.contains("source#0"), "degrades, never panics");
    }
}
