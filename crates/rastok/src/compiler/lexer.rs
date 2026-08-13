//! Hand-written lexer for `.rkg` source.
//!
//! Two scanning modes, per the delimitation rules of `grammar.md`:
//!
//! - [`Lexer::next_token`] — ordinary tokens; whitespace, newlines, and
//!   comments are skipped. Keywords are not distinguished here: every
//!   word is an [`TokenKind::Ident`], and the parser decides by
//!   position (field names may reuse type words).
//! - [`Lexer::cel_expr`] — an opaque CEL expression span, scanned at
//!   bracket depth 0 until the position's terminator set. The
//!   terminator is never consumed; the parser resumes on it.
//!
//! Tokens borrow their text from the source; the lexer copies nothing.

use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};

/// Diagnostic for a character outside the lexical grammar.
pub const E_LEX_CHAR: Code = Code::new("E-LEX-CHAR");
/// Diagnostic for a string literal that never closes.
pub const E_LEX_UNTERMINATED_STRING: Code = Code::new("E-LEX-UNTERMINATED-STRING");
/// Diagnostic for a block comment that never closes.
pub const E_LEX_UNTERMINATED_COMMENT: Code = Code::new("E-LEX-UNTERMINATED-COMMENT");
/// Diagnostic for a raw guest block whose braces never balance.
pub const E_LEX_UNTERMINATED_BLOCK: Code = Code::new("E-LEX-UNTERMINATED-BLOCK");

/// Kind of one lexical token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Word: identifier or keyword — the parser decides by position.
    Ident,
    /// Integer literal, optionally negative.
    Int,
    /// Float literal, optionally negative.
    Float,
    /// String literal, quotes included in the text.
    Str,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBrack,
    /// `]`
    RBrack,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `=`
    Eq,
    /// `:`
    Colon,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `,`
    Comma,
    /// `;`
    Semi,
    /// `+`
    Plus,
    /// `?`
    Question,
    /// `|`
    Pipe,
    /// `!`
    Bang,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `->`
    Arrow,
    /// `<->`
    BiArrow,
    /// Opaque CEL expression span, produced only by [`Lexer::cel_expr`].
    CelExpr,
    /// Opaque raw guest text, produced only by [`Lexer::raw_block`].
    RawText,
    /// Lexically invalid input; a diagnostic was pushed.
    Error,
    /// End of input, returned indefinitely once reached.
    Eof,
}

/// One lexical token: kind, borrowed text, and source span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    /// The token kind.
    pub kind: TokenKind,
    /// The exact source slice, zero-copy.
    pub text: &'a str,
    /// Where the slice sits in the source.
    pub span: Span,
    /// Whether the trivia before this token crossed a line break —
    /// the separation signal the parser uses where the grammar makes
    /// the newline meaningful, so spaces and comments stay trivia.
    pub newline_before: bool,
}

/// Terminators ending a CEL expression span at bracket depth 0, chosen
/// by the parser per grammar position. The terminator is never
/// consumed.
#[derive(Debug, Clone, Copy)]
pub struct TerminatorSet {
    /// Single-byte terminators (`;`, `}`, `,`, `)`, `{`, …).
    pub bytes: &'static [u8],
    /// Whether a newline at depth 0 terminates the span.
    pub newline: bool,
    /// Whether `->` at depth 0 terminates the span.
    pub arrow: bool,
    /// Whole-word terminators (`having`, `by`, `within`, …), matched at
    /// word boundaries only.
    pub keywords: &'static [&'static str],
}

/// The `json` continuation, classified totally by ONE probe
/// (D-2026-08-13-1, review round 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonTail {
    /// Nothing on the line, or the position's own boundary: the
    /// bare word stands alone (a contract may judge it as a Word).
    Bare,
    /// A document read whole; the cursor sits past it.
    Document {
        /// The parsed document.
        value: rastok_core::cel::Value,
        /// Where its bytes sit in the source.
        range: core::ops::Range<usize>,
    },
    /// A follower that is neither a document nor the boundary — the
    /// reserved word admits no CEL continuation.
    BadForm {
        /// The follower's offset (diagnostic anchor).
        at: usize,
        /// One past the recovery's last consumed byte.
        recovered_to: usize,
    },
    /// A document that opened but does not read.
    Unreadable(JsonRefusal),
}

/// A refused json-document delegation: where the document opened,
/// where the reader stopped, its message, and how far the FORWARD
/// recovery consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRefusal {
    /// The opener's offset (the span the recovery form carries).
    pub opener: usize,
    /// The faulty byte's offset (diagnostic anchor).
    pub at: usize,
    /// The reader's message.
    pub message: String,
    /// One past the recovery's last consumed byte.
    pub recovered_to: usize,
}

/// Hand-written scanner over one UTF-8 source.
#[derive(Debug)]
pub struct Lexer<'a> {
    source: &'a str,
    source_id: SourceId,
    pos: usize,
    newline_before: bool,
}

impl<'a> Lexer<'a> {
    /// Starts scanning `source`, attributing spans to `source_id`.
    #[must_use]
    pub fn new(source_id: SourceId, source: &'a str) -> Self {
        Self {
            source,
            source_id,
            pos: 0,
            newline_before: false,
        }
    }

    /// The next ordinary token; [`TokenKind::Eof`] at the end, forever.
    pub fn next_token(&mut self, bag: &mut Bag) -> Token<'a> {
        self.skip_trivia(bag);
        let start = self.pos;
        let Some(byte) = self.peek_byte() else {
            return self.token(TokenKind::Eof, start);
        };
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                self.eat_while(is_ident_continue);
                self.token(TokenKind::Ident, start)
            }
            b'0'..=b'9' => self.number(start),
            b'-' => match self.peek_byte_at(1) {
                Some(b'>') => {
                    self.pos += 2;
                    self.token(TokenKind::Arrow, start)
                }
                Some(b'0'..=b'9') => {
                    self.pos += 1;
                    self.number(start)
                }
                _ => self.unknown_character(bag, start),
            },
            b'<' => {
                if self.source.as_bytes()[self.pos..].starts_with(b"<->") {
                    self.pos += 3;
                    self.token(TokenKind::BiArrow, start)
                } else {
                    self.pos += 1;
                    self.token(TokenKind::Lt, start)
                }
            }
            b'.' => {
                if self.peek_byte_at(1) == Some(b'.') {
                    self.pos += 2;
                    self.token(TokenKind::DotDot, start)
                } else {
                    self.pos += 1;
                    self.token(TokenKind::Dot, start)
                }
            }
            b'"' => self.string(bag, start),
            _ => {
                if let Some(kind) = punctuation(byte) {
                    self.pos += 1;
                    self.token(kind, start)
                } else {
                    self.unknown_character(bag, start)
                }
            }
        }
    }

    /// An opaque CEL expression span, scanned at bracket depth 0 until
    /// `terminators` — the delimitation rules of `grammar.md`. Quoted
    /// strings (both CEL quote styles) are skipped opaquely; any
    /// closing bracket below depth 0 also terminates.
    pub fn cel_expr(&mut self, terminators: &TerminatorSet, bag: &mut Bag) -> Token<'a> {
        while matches!(self.peek_byte(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
        let start = self.pos;
        self.cel_scan(0, terminators, bag);
        let end = self
            .source
            .as_bytes()
            .get(start..self.pos)
            .map_or(self.pos, |scanned| start + scanned.trim_ascii_end().len());
        Token {
            kind: TokenKind::CelExpr,
            text: &self.source[start..end],
            span: Span::new(self.source_id, start..end),
            newline_before: false,
        }
    }

    /// The unlexed source from `offset` — a read-only window for
    /// diagnostics and prefix slices; cursor re-syncs happen only
    /// inside the lexer's own delegation methods.
    #[must_use]
    pub fn source_from(&self, offset: usize) -> &'a str {
        &self.source[offset..]
    }

    /// The WHOLE `json` continuation, one probe and one atomic
    /// forward walk (D-2026-08-13-1, review rounds 2–3): the word is
    /// RESERVED, so its follower decides totally — nothing (or the
    /// position's own boundary) leaves the bare word; `{`/`[` on the
    /// line hands the window to the document reader; anything else
    /// is an invalid json form, consumed forward to the position's
    /// recovery point. Every cursor write is a locally-derived
    /// forward offset — no repositioning primitive exists.
    pub fn json_tail(&mut self, terminators: &TerminatorSet) -> JsonTail {
        let Some((offset, byte)) = self.significant_on_line() else {
            return JsonTail::Bare;
        };
        if matches!(byte, b'{' | b'[') {
            return match rastok_core::json::parse_value(&self.source[offset..]) {
                Ok((value, consumed)) => JsonTail::Document {
                    value,
                    range: {
                        let end = offset + consumed;
                        self.pos = end;
                        offset..end
                    },
                },
                Err(error) => {
                    let at = (offset + error.offset).min(self.source.len());
                    self.pos = offset;
                    let recovered_to = self.skip_json_recovery(terminators);
                    JsonTail::Unreadable(JsonRefusal {
                        opener: offset,
                        at,
                        message: error.message,
                        recovered_to,
                    })
                }
            };
        }
        if matches!(byte, b')' | b']' | b'}') || self.terminates_at(offset, byte, terminators) {
            // The position's own boundary: the word stands alone.
            return JsonTail::Bare;
        }
        // A follower that is neither a document nor the boundary:
        // the json form is invalid — `json` cannot begin a CEL
        // expression, being reserved (review round 3).
        self.pos = offset;
        let recovered_to = self.skip_json_recovery(terminators);
        JsonTail::BadForm {
            at: offset,
            recovered_to,
        }
    }

    /// Scans a CEL continuation from the CURRENT position to the
    /// position's terminator, but reports the token from
    /// `text_start` — the left-factored prefix the parser already
    /// consumed (D-2026-08-13-1): re-reading the immutable
    /// `source[text_start..end]` slice is a bounded re-read, never
    /// a parser rewind.
    pub fn cel_expr_from(
        &mut self,
        text_start: usize,
        depth: u32,
        terminators: &TerminatorSet,
        bag: &mut Bag,
    ) -> Token<'a> {
        // `depth` counts the openers the caller already consumed:
        // a dropped `(`/`[` peek left the cursor INSIDE its region,
        // and the scan must not break on the matching closer.
        self.cel_scan(depth, terminators, bag);
        // The stitched slice trims its OWN end: an empty continuation
        // leaves the whitespace skipped before the terminator inside
        // `text_start..end`, and the inner trim never saw it.
        let end = self.pos.max(text_start);
        let end = self
            .source
            .as_bytes()
            .get(text_start..end)
            .map_or(end, |stitched| text_start + stitched.trim_ascii_end().len());
        Token {
            kind: TokenKind::CelExpr,
            text: &self.source[text_start..end],
            span: Span::new(self.source_id, text_start..end),
            newline_before: false,
        }
    }

    /// The CEL region scan both entries share: consumes to the
    /// position's terminator at depth zero, tracking balanced
    /// delimiters and skipping quoted content opaquely.
    fn cel_scan(&mut self, mut depth: u32, terminators: &TerminatorSet, bag: &mut Bag) {
        while let Some(byte) = self.peek_byte() {
            if depth == 0 && self.at_terminator(byte, terminators) {
                break;
            }
            match byte {
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' | b'}' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                b'"' | b'\'' => self.skip_quoted(bag, byte),
                // Comments are trivia INSIDE the region too: a
                // terminator inside one never terminates. The depth
                // rule mirrors the newline's exactly — a line's end
                // can only end the value at depth ZERO, so a comment
                // reaching (or crossing) the line's end breaks the
                // scan only there; at positive depth it is consumed
                // and the expression continues (D-2026-08-13-1,
                // review round 2).
                b'/' if let Some(comment) =
                    rastok_core::trivia::comment_at(self.source, self.pos) =>
                {
                    if !comment.terminated {
                        // The value ends here; the trivia authority
                        // diagnoses the unterminated comment.
                        break;
                    }
                    let reaches_line_end = comment.multiline
                        || self.source.as_bytes().get(comment.end) == Some(&b'\n')
                        || comment.end == self.source.len();
                    if depth == 0 && terminators.newline && reaches_line_end {
                        break;
                    }
                    self.pos = comment.end;
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Captures a raw brace-balanced block (guest text, verbatim
    /// text, inline bytes) — braces excluded, no host lexing.
    pub fn raw_block(&mut self, bag: &mut Bag) -> Token<'a> {
        let start = self.pos;
        let mut depth: u32 = 0;
        loop {
            match self.peek_byte() {
                None => {
                    bag.push(
                        Diagnostic::new(
                            E_LEX_UNTERMINATED_BLOCK,
                            Phase::Parse,
                            "this raw block never closes",
                        )
                        .with_span(Span::new(self.source_id, start..self.pos)),
                    );
                    break;
                }
                Some(b'{') => {
                    depth += 1;
                    self.pos += 1;
                }
                Some(b'}') => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                Some(quote @ (b'"' | b'\'')) => self.skip_quoted(bag, quote),
                Some(_) => self.pos += 1,
            }
        }
        let token = Token {
            kind: TokenKind::RawText,
            text: &self.source[start..self.pos],
            span: Span::new(self.source_id, start..self.pos),
            newline_before: false,
        };
        if self.peek_byte() == Some(b'}') {
            self.pos += 1;
        }
        token
    }

    /// Consumes trivia so an opaque scan starts at significant input.
    /// Needed before a shape-dispatched CEL scan: the scan itself
    /// treats a leading newline as an empty expression, but the
    /// newline separating two statements is not part of either.
    pub fn advance_to_significant(&mut self, bag: &mut Bag) {
        self.skip_trivia(bag);
    }

    /// Consumes horizontal trivia — spaces and comments — up to the
    /// next newline outside a comment. A value scan starts past its
    /// same-line trivia, while a bare newline still reads as an empty
    /// expression, per the delimitation rules.
    pub fn advance_within_line(&mut self) {
        loop {
            match self.peek_byte() {
                Some(b' ' | b'\t' | b'\r') => self.pos += 1,
                Some(b'/')
                    if let Some(comment) =
                        rastok_core::trivia::comment_at(self.source, self.pos) =>
                {
                    // Line-scoped policy: a comment crossing the
                    // line (or never closing) is NOT consumed — the
                    // line ends here, and the trivia authority will
                    // meet the comment itself.
                    if !comment.terminated || comment.multiline {
                        return;
                    }
                    self.pos = comment.end;
                }
                _ => return,
            }
        }
    }

    /// The first significant byte ahead, past trivia, without consuming
    /// anything and without diagnosing. The parser's dispatch probe for
    /// value positions: CEL text must reach the CEL chain untouched,
    /// never partially interpreted — or rejected — by this lexer.
    #[must_use]
    pub fn first_significant_byte(&self) -> Option<u8> {
        let bytes = self.source.as_bytes();
        let mut pos = self.pos;
        loop {
            match *bytes.get(pos)? {
                b' ' | b'\t' | b'\r' | b'\n' => pos += 1,
                b'/' if let Some(comment) = rastok_core::trivia::comment_at(self.source, pos) => {
                    if !comment.terminated {
                        return None;
                    }
                    pos = comment.end;
                }
                byte => return Some(byte),
            }
        }
    }

    /// The first significant byte ON THE CURRENT LINE, or `None`
    /// when only trivia remains before the newline (or the end of
    /// input). A newline-terminated value position asks this BEFORE
    /// any dispatch: an omitted value must diagnose where it is
    /// missing, never absorb the next line's construct
    /// (D-2026-08-12-1, review round).
    #[must_use]
    pub fn significant_byte_on_line(&self) -> Option<u8> {
        self.significant_on_line().map(|(_, byte)| byte)
    }

    /// The line-local probe WITH its offset: the position of the
    /// first significant byte on the current line, and that byte.
    /// The trivia rule is the HOST's exactly — ASCII spacing and
    /// comments that stay on the line — so a caller handing the
    /// remainder to a foreign reader starts AT this offset: the
    /// reader never sees host trivia, and never applies its own
    /// (Unicode) idea of whitespace (D-2026-08-12-1, review round).
    #[must_use]
    pub fn significant_on_line(&self) -> Option<(usize, u8)> {
        let bytes = self.source.as_bytes();
        let mut pos = self.pos;
        loop {
            match *bytes.get(pos)? {
                b'\n' => return None,
                b' ' | b'\t' | b'\r' => pos += 1,
                b'/' if let Some(comment) = rastok_core::trivia::comment_at(self.source, pos) => {
                    if !comment.terminated || comment.multiline {
                        // The comment swallows the rest of the line.
                        return None;
                    }
                    pos = comment.end;
                }
                byte => return Some((pos, byte)),
            }
        }
    }

    /// Consumes one raw balanced-delimiter region — the OPEN byte is
    /// expected at the current position — with BOTH quote styles
    /// protected (a delimiter inside either kind of string is
    /// content, never structure). ABSOLUTELY SILENT: this is error
    /// recovery for dead/reserved forms, the one diagnostic already
    /// spoke, and even an unterminated string in the swallowed text
    /// adds no second voice — the quotes are skipped locally, not
    /// through `skip_quoted` (which diagnoses)
    /// (D-2026-08-12-1, review round).
    pub fn skip_balanced_raw(&mut self, open: u8, close: u8) {
        let bytes = self.source.as_bytes();
        let mut depth: u32 = 0;
        let mut quote: Option<u8> = None;
        while let Some(&byte) = bytes.get(self.pos) {
            if let Some(active) = quote {
                match byte {
                    b'\\' => {
                        self.pos = (self.pos + 2).min(bytes.len());
                        continue;
                    }
                    _ if byte == active => quote = None,
                    _ => {}
                }
                self.pos += 1;
                continue;
            }
            match byte {
                b'"' | b'\'' => {
                    quote = Some(byte);
                    self.pos += 1;
                }
                _ if byte == open => {
                    depth += 1;
                    self.pos += 1;
                }
                _ if byte == close => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Skips OPAQUELY over a FAULTY JSON value to the position's
    /// recovery point — never consuming the terminator, never moving
    /// backward — and returns the final offset. JSON's own grammar
    /// drives the walk (D-2026-08-12-1, review round): a STACK of
    /// expected closers (`{`→`}`, `[`→`]` — a closer of another type
    /// never pops), double-quote strings with escapes, and STRONG
    /// boundaries — `;` and `)` cannot appear in JSON outside a
    /// string, so they stop the walk at ANY depth; `,`, `}` and the
    /// newline stop only at depth zero when the position knows them.
    /// A mismatched closer stops too: under-consuming garbage beats
    /// absorbing a sibling. Error recovery only — nothing speaks.
    pub fn skip_json_recovery(&mut self, terminators: &TerminatorSet) -> usize {
        let bytes = self.source.as_bytes();
        let mut expected: Vec<u8> = Vec::new();
        let mut in_string = false;
        while let Some(&byte) = bytes.get(self.pos) {
            if in_string {
                match byte {
                    b'\\' => {
                        self.pos = (self.pos + 2).min(bytes.len());
                        continue;
                    }
                    b'"' => in_string = false,
                    _ => {}
                }
                self.pos += 1;
                continue;
            }
            // Strong boundaries: impossible in JSON outside strings.
            // A `)` matching a paren the GARBAGE opened (`json(1)`,
            // review round 3) pops instead — the CALLER's `)` always
            // arrives with an empty stack and still stops the walk.
            if byte == b';' || (byte == b')' && expected.last() != Some(&b')')) {
                return self.pos;
            }
            if expected.is_empty()
                && (terminators.bytes.contains(&byte) || (terminators.newline && byte == b'\n'))
            {
                return self.pos;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => expected.push(b'}'),
                b'[' => expected.push(b']'),
                b'(' => expected.push(b')'),
                b'}' | b']' | b')' => {
                    if expected.last() == Some(&byte) {
                        expected.pop();
                    } else {
                        // A closer that matches nothing — a
                        // boundary, not JSON.
                        return self.pos;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        self.pos
    }

    fn at_terminator(&self, byte: u8, terminators: &TerminatorSet) -> bool {
        self.terminates_at(self.pos, byte, terminators)
    }

    fn terminates_at(&self, pos: usize, byte: u8, terminators: &TerminatorSet) -> bool {
        if terminators.bytes.contains(&byte) {
            return true;
        }
        if terminators.newline && byte == b'\n' {
            return true;
        }
        if terminators.arrow && self.source.as_bytes()[pos..].starts_with(b"->") {
            return true;
        }
        if !terminators.keywords.is_empty() && self.word_boundary_at(pos) {
            let rest = &self.source[pos..];
            return terminators.keywords.iter().any(|keyword| {
                rest.starts_with(keyword)
                    && !rest[keyword.len()..]
                        .bytes()
                        .next()
                        .is_some_and(is_ident_continue)
            });
        }
        false
    }

    fn word_boundary_at(&self, pos: usize) -> bool {
        pos == 0
            || !self
                .source
                .as_bytes()
                .get(pos - 1)
                .copied()
                .is_some_and(is_ident_continue)
    }

    fn skip_quoted(&mut self, bag: &mut Bag, quote: u8) {
        let start = self.pos;
        self.pos += 1;
        while let Some(byte) = self.peek_byte() {
            self.pos += 1;
            match byte {
                b'\\' => self.pos = (self.pos + 1).min(self.source.len()),
                _ if byte == quote => return,
                _ => {}
            }
        }
        bag.push(
            Diagnostic::new(
                E_LEX_UNTERMINATED_STRING,
                Phase::Parse,
                "this string never closes",
            )
            .with_span(Span::new(self.source_id, start..self.pos)),
        );
    }

    fn number(&mut self, start: usize) -> Token<'a> {
        self.eat_while(|byte| byte.is_ascii_digit());
        if self.peek_byte() == Some(b'.')
            && self
                .peek_byte_at(1)
                .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.pos += 1;
            self.eat_while(|byte| byte.is_ascii_digit());
            self.token(TokenKind::Float, start)
        } else {
            self.token(TokenKind::Int, start)
        }
    }

    fn string(&mut self, bag: &mut Bag, start: usize) -> Token<'a> {
        self.pos += 1;
        while let Some(byte) = self.peek_byte() {
            self.pos += 1;
            match byte {
                b'\\' => self.pos = (self.pos + 1).min(self.source.len()),
                b'"' => return self.token(TokenKind::Str, start),
                _ => {}
            }
        }
        bag.push(
            Diagnostic::new(
                E_LEX_UNTERMINATED_STRING,
                Phase::Parse,
                "this string never closes",
            )
            .with_span(Span::new(self.source_id, start..self.pos)),
        );
        self.token(TokenKind::Error, start)
    }

    fn skip_trivia(&mut self, bag: &mut Bag) {
        let run_start = self.pos;
        loop {
            match self.peek_byte() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/')
                    if let Some(comment) =
                        rastok_core::trivia::comment_at(self.source, self.pos) =>
                {
                    // Host policy: CONSUME, diagnosing the
                    // unterminated block — the one voice for it.
                    if !comment.terminated {
                        bag.push(
                            Diagnostic::new(
                                E_LEX_UNTERMINATED_COMMENT,
                                Phase::Parse,
                                "this block comment never closes",
                            )
                            .with_span(Span::new(self.source_id, self.pos..comment.end)),
                        );
                    }
                    self.pos = comment.end;
                }
                _ => {
                    self.newline_before = self.source[run_start..self.pos].contains('\n');
                    return;
                }
            }
        }
    }

    fn unknown_character(&mut self, bag: &mut Bag, start: usize) -> Token<'a> {
        let character = self.source[start..]
            .chars()
            .next()
            .unwrap_or_else(|| unreachable!("caller saw a byte at this position"));
        self.pos += character.len_utf8();
        let token = self.token(TokenKind::Error, start);
        bag.push(
            Diagnostic::new(
                E_LEX_CHAR,
                Phase::Parse,
                format!("the character `{character}` is not part of the grammar"),
            )
            .with_span(token.span)
            .with_field("character", character.to_string()),
        );
        token
    }

    fn token(&self, kind: TokenKind, start: usize) -> Token<'a> {
        Token {
            kind,
            text: &self.source[start..self.pos],
            span: Span::new(self.source_id, start..self.pos),
            newline_before: self.newline_before,
        }
    }

    fn eat_while(&mut self, keep: impl Fn(u8) -> bool) {
        while self.peek_byte().is_some_and(&keep) {
            self.pos += 1;
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.source.as_bytes().get(self.pos + offset).copied()
    }
}

fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn punctuation(byte: u8) -> Option<TokenKind> {
    Some(match byte {
        b'{' => TokenKind::LBrace,
        b'}' => TokenKind::RBrace,
        b'[' => TokenKind::LBrack,
        b']' => TokenKind::RBrack,
        b'(' => TokenKind::LParen,
        b')' => TokenKind::RParen,
        b'=' => TokenKind::Eq,
        b':' => TokenKind::Colon,
        b',' => TokenKind::Comma,
        b';' => TokenKind::Semi,
        b'+' => TokenKind::Plus,
        b'?' => TokenKind::Question,
        b'|' => TokenKind::Pipe,
        b'!' => TokenKind::Bang,
        b'>' => TokenKind::Gt,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_recovery_never_pops_a_mismatched_closer() {
        // The typed stack (D-2026-08-12-1, review round): a closer
        // of another type STOPS the walk — it never pops — and the
        // returned offset points exactly at it.
        let terminators = TerminatorSet {
            bytes: b"}",
            newline: true,
            arrow: false,
            keywords: &[],
        };
        // `[` opened, `}` arrives: stop BEFORE `}`, stack unpopped.
        let mut lexer = Lexer::new(SourceId::new(0), "[1, 2 } tail");
        let end = lexer.skip_json_recovery(&terminators);
        assert_eq!(end, 6, "stops at the mismatched `}}`");
        // `{` opened, `]` arrives: stop BEFORE `]`.
        let mut lexer = Lexer::new(SourceId::new(0), "{\"a\": 1 ] tail");
        let end = lexer.skip_json_recovery(&terminators);
        assert_eq!(end, 8, "stops at the mismatched `]`");
        // Strong boundaries hold at ANY depth.
        let mut lexer = Lexer::new(SourceId::new(0), "{\"a\": ???; tail");
        let end = lexer.skip_json_recovery(&terminators);
        assert_eq!(end, 9, "`;` stops inside the open brace");
        // A matched closer pops and the walk continues to the
        // position's own terminator.
        let mut lexer = Lexer::new(SourceId::new(0), "{\"a\": [1]} }");
        let end = lexer.skip_json_recovery(&terminators);
        assert_eq!(end, 11, "balanced json consumed, position `}}` kept");
    }

    fn lex(source: &str) -> (Vec<(TokenKind, &str)>, Bag) {
        let mut bag = Bag::new();
        let mut lexer = Lexer::new(SourceId::new(0), source);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next_token(&mut bag);
            if token.kind == TokenKind::Eof {
                return (tokens, bag);
            }
            tokens.push((token.kind, token.text));
        }
    }

    fn kinds(source: &str) -> Vec<(TokenKind, &str)> {
        let (tokens, bag) = lex(source);
        assert!(bag.diagnostics().is_empty(), "unexpected: {bag:?}");
        tokens
    }

    #[test]
    fn a_declaration_line_tokenizes() {
        use TokenKind::{Colon, Ident, LBrace};
        assert_eq!(
            kinds("graph m0_probe : App {"),
            [
                (Ident, "graph"),
                (Ident, "m0_probe"),
                (Colon, ":"),
                (Ident, "App"),
                (LBrace, "{"),
            ]
        );
    }

    #[test]
    fn arrows_take_maximal_munch() {
        use TokenKind::{Arrow, BiArrow, Dot, Ident};
        assert_eq!(
            kinds("a -> b.out <-> c"),
            [
                (Ident, "a"),
                (Arrow, "->"),
                (Ident, "b"),
                (Dot, "."),
                (Ident, "out"),
                (BiArrow, "<->"),
                (Ident, "c"),
            ]
        );
    }

    #[test]
    fn numbers_and_dots_disambiguate() {
        use TokenKind::{Dot, DotDot, Float, Ident, Int};
        assert_eq!(
            kinds("1.5 -3 1..2 a.b .."),
            [
                (Float, "1.5"),
                (Int, "-3"),
                (Int, "1"),
                (DotDot, ".."),
                (Int, "2"),
                (Ident, "a"),
                (Dot, "."),
                (Ident, "b"),
                (DotDot, ".."),
            ]
        );
    }

    #[test]
    fn strings_keep_their_quotes_and_escapes() {
        assert_eq!(
            kinds(r#"codec = "js\"onl""#),
            [
                (TokenKind::Ident, "codec"),
                (TokenKind::Eq, "="),
                (TokenKind::Str, r#""js\"onl""#),
            ]
        );
    }

    #[test]
    fn comments_and_newlines_are_trivia() {
        assert_eq!(
            kinds("a // line\n/* block\nstill */ b"),
            [(TokenKind::Ident, "a"), (TokenKind::Ident, "b")]
        );
    }

    #[test]
    fn an_unknown_character_is_reported_and_skipped() {
        let (tokens, bag) = lex("a @ b");
        assert_eq!(
            tokens,
            [
                (TokenKind::Ident, "a"),
                (TokenKind::Error, "@"),
                (TokenKind::Ident, "b"),
            ]
        );
        let diagnostic = &bag.diagnostics()[0];
        assert_eq!(diagnostic.code(), E_LEX_CHAR);
        assert_eq!(diagnostic.spans()[0].range(), 2..3);
        assert_eq!(diagnostic.fields(), [("character", String::from("@"))]);
    }

    #[test]
    fn an_unterminated_string_is_reported() {
        let (tokens, bag) = lex("val x = \"oops");
        assert_eq!(tokens.last(), Some(&(TokenKind::Error, "\"oops")));
        assert_eq!(bag.diagnostics()[0].code(), E_LEX_UNTERMINATED_STRING);
    }

    #[test]
    fn an_unterminated_comment_is_reported() {
        let (_, bag) = lex("a /* never closed");
        assert_eq!(bag.diagnostics()[0].code(), E_LEX_UNTERMINATED_COMMENT);
    }

    const ATTRIBUTE_POSITION: TerminatorSet = TerminatorSet {
        bytes: b";}",
        newline: true,
        arrow: false,
        keywords: &[],
    };

    fn cel(source: &str, terminators: &TerminatorSet) -> (String, String) {
        let mut bag = Bag::new();
        let mut lexer = Lexer::new(SourceId::new(0), source);
        let token = lexer.cel_expr(terminators, &mut bag);
        assert_eq!(token.kind, TokenKind::CelExpr);
        let mut rest = String::new();
        loop {
            let token = lexer.next_token(&mut bag);
            if token.kind == TokenKind::Eof {
                break;
            }
            rest.push_str(token.text);
        }
        (token.text.to_string(), rest)
    }

    #[test]
    fn a_cel_span_stops_at_the_unconsumed_terminator() {
        let (expr, rest) = cel("meta.app.user_id ; x = 1", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "meta.app.user_id");
        assert_eq!(rest, ";x=1");
    }

    #[test]
    fn a_cel_span_tracks_bracket_depth() {
        let (expr, _) = cel("{'a': 1}.size() > 0 ; tail", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "{'a': 1}.size() > 0");
    }

    #[test]
    fn a_cel_span_skips_terminators_inside_strings() {
        let (expr, _) = cel("\"a;b\" + 'c}d' ; tail", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "\"a;b\" + 'c}d'");
    }

    #[test]
    fn a_newline_terminates_at_depth_zero_only() {
        let (expr, _) = cel("(a &&\n b)\n next = 1", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "(a &&\n b)");
    }

    #[test]
    fn keyword_terminators_match_word_boundaries() {
        const GUARD_POSITION: TerminatorSet = TerminatorSet {
            bytes: b"{",
            newline: true,
            arrow: true,
            keywords: &["having", "within"],
        };
        let (expr, _) = cel(
            "count(p) + shaving >= k having size(p) > 0",
            &GUARD_POSITION,
        );
        assert_eq!(expr, "count(p) + shaving >= k");
        let (arrowed, _) = cel("data.kind == \"x\" -> audit", &GUARD_POSITION);
        assert_eq!(arrowed, "data.kind == \"x\"");
    }

    #[test]
    fn an_unbalanced_closer_terminates_defensively() {
        let (expr, _) = cel("a + b) leftovers", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "a + b");
    }
}
