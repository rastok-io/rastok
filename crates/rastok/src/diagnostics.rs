//! Structured diagnostics shared across phases.
//!
//! Governing rule: [`diagnostics.md`]. Stable forever are a diagnostic's
//! code and its structured payload fields — what tests and tools key on.
//! Never stable is the rendered message text. Machine output is
//! `{code, severity, spans, fields}`; text is rendering.
//!
//! [`diagnostics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/diagnostics.md

use std::fmt;

/// Identity of one source text within a compilation, allocated by the
/// component that loads sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u32);

impl SourceId {
    /// Wraps a source index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The wrapped source index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Byte range inside one source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    source: SourceId,
    start: usize,
    end: usize,
}

impl Span {
    /// Builds a span over `range` in `source`. An empty range marks a
    /// point, such as an unexpected end of input.
    #[must_use]
    pub fn new(source: SourceId, range: std::ops::Range<usize>) -> Self {
        debug_assert!(range.start <= range.end);
        Self {
            source,
            start: range.start,
            end: range.end,
        }
    }

    /// The source this span points into.
    #[must_use]
    pub fn source(self) -> SourceId {
        self.source
    }

    /// The byte range inside the source.
    #[must_use]
    pub fn range(self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The input is rejected.
    Error,
    /// The input is accepted with a caveat.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

/// Tool phase that emitted a diagnostic — the registry vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Source-to-syntax-tree parsing.
    Parse,
    /// Name and unit resolution.
    Resolve,
    /// Semantic checking.
    Check,
    /// Lowering and canonicalization.
    Plan,
    /// Live execution tooling.
    Run,
    /// Journal replay tooling.
    Replay,
    /// Scenario simulation tooling.
    Sim,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Parse => "parse",
            Self::Resolve => "resolve",
            Self::Check => "check",
            Self::Plan => "plan",
            Self::Run => "run",
            Self::Replay => "replay",
            Self::Sim => "sim",
        })
    }
}

/// Stable diagnostic code: `E-` or `W-` plus UPPER-KEBAB. A published
/// code is never renamed and never reused; the registry in
/// `docs/spec/diagnostics.md` is the collision authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Code(&'static str);

impl Code {
    /// Wraps a registered code literal.
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    /// The code text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Severity, carried by the code prefix — a code cannot disagree
    /// with its severity.
    #[must_use]
    pub fn severity(self) -> Severity {
        debug_assert!(self.0.starts_with("E-") || self.0.starts_with("W-"));
        if self.0.starts_with("W-") {
            Severity::Warning
        } else {
            Severity::Error
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// One structured diagnostic: stable identity plus rendering material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    code: Code,
    phase: Phase,
    spans: Vec<Span>,
    fields: Vec<(&'static str, String)>,
    message: String,
}

impl Diagnostic {
    /// Starts a diagnostic with its stable code, emitting phase, and
    /// human message. Spans and fields are attached with
    /// [`Self::with_span`] and [`Self::with_field`].
    #[must_use]
    pub fn new(code: Code, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            code,
            phase,
            spans: Vec::new(),
            fields: Vec::new(),
            message: message.into(),
        }
    }

    /// Attaches a source span. The first span is the primary location.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.spans.push(span);
        self
    }

    /// Attaches a structured payload field. Field names belong to the
    /// stability contract; attachment order is the rendering order.
    #[must_use]
    pub fn with_field(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.fields.push((name, value.into()));
        self
    }

    /// The stable code.
    #[must_use]
    pub fn code(&self) -> Code {
        self.code
    }

    /// Severity, carried by the code prefix.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.code.severity()
    }

    /// The emitting phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Attached source spans, primary first.
    #[must_use]
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Structured payload fields, in attachment order.
    #[must_use]
    pub fn fields(&self) -> &[(&'static str, String)] {
        &self.fields
    }

    /// The human message. Never part of the stability contract.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}[{}] ({}): {}",
            self.severity(),
            self.code,
            self.phase,
            self.message
        )?;
        for span in &self.spans {
            write!(
                f,
                "\n  --> source#{} @ {}..{}",
                span.source().index(),
                span.range().start,
                span.range().end
            )?;
        }
        for (name, value) in &self.fields {
            write!(f, "\n  {name} = {value}")?;
        }
        Ok(())
    }
}

/// Accumulating diagnostic destination. A bag never prints and never
/// panics; rendering and process exit decisions belong to the caller.
#[derive(Debug, Default)]
pub struct Bag {
    diagnostics: Vec<Diagnostic>,
}

impl Bag {
    /// Starts an empty bag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulates one diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Whether any accumulated diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == Severity::Error)
    }

    /// The accumulated diagnostics, in emission order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Consumes the bag and hands the diagnostics over.
    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ERROR: Code = Code::new("E-TEST-RENDER");
    const TEST_WARNING: Code = Code::new("W-TEST-RENDER");

    #[test]
    fn severity_is_carried_by_the_code_prefix() {
        assert_eq!(TEST_ERROR.severity(), Severity::Error);
        assert_eq!(TEST_WARNING.severity(), Severity::Warning);
    }

    #[test]
    fn rendering_is_stable_byte_for_byte() {
        let source = SourceId::new(0);
        let diagnostic = Diagnostic::new(TEST_ERROR, Phase::Check, "duplicate key `a`")
            .with_span(Span::new(source, 12..17))
            .with_span(Span::new(source, 30..35))
            .with_field("key", "a");
        assert_eq!(
            diagnostic.to_string(),
            "error[E-TEST-RENDER] (check): duplicate key `a`\n\
             \x20 --> source#0 @ 12..17\n\
             \x20 --> source#0 @ 30..35\n\
             \x20 key = a"
        );
    }

    #[test]
    fn stable_identity_survives_a_message_change() {
        let before =
            Diagnostic::new(TEST_ERROR, Phase::Check, "old wording").with_field("key", "a");
        let after = Diagnostic::new(TEST_ERROR, Phase::Check, "new wording").with_field("key", "a");
        assert_eq!(before.code(), after.code());
        assert_eq!(before.fields(), after.fields());
        assert_ne!(before.message(), after.message());
    }

    #[test]
    fn bag_accumulates_in_emission_order() {
        let mut bag = Bag::new();
        assert!(!bag.has_errors());
        bag.push(Diagnostic::new(TEST_WARNING, Phase::Resolve, "first"));
        assert!(!bag.has_errors());
        bag.push(Diagnostic::new(TEST_ERROR, Phase::Check, "second"));
        assert!(bag.has_errors());
        let codes: Vec<_> = bag
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect();
        assert_eq!(codes, [TEST_WARNING, TEST_ERROR]);
    }

    #[test]
    fn span_exposes_its_source_and_range() {
        let span = Span::new(SourceId::new(3), 5..9);
        assert_eq!(span.source(), SourceId::new(3));
        assert_eq!(span.range(), 5..9);
        assert_eq!(Span::new(SourceId::new(0), 4..4).range().len(), 0);
    }

    #[test]
    fn phase_spellings_match_the_registry_vocabulary() {
        let spellings: Vec<_> = [
            Phase::Parse,
            Phase::Resolve,
            Phase::Check,
            Phase::Plan,
            Phase::Run,
            Phase::Replay,
            Phase::Sim,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            spellings,
            ["parse", "resolve", "check", "plan", "run", "replay", "sim"]
        );
    }
}
