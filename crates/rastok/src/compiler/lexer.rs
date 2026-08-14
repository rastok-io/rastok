//! Hand-written lexer for `.rkg` source — the ONE owner of the byte
//! cursor and the token lookahead (D-2026-08-13-1, review round 5).
//!
//! The parser reads complete lexemes through [`Lexer::peek`] and
//! [`Lexer::next`] (keywords are not distinguished: every word is an
//! [`TokenKind::Ident`], the parser deciding by position), consumes
//! sub-language regions as COMPLETE specialized lexemes
//! ([`Lexer::expr_region`], [`Lexer::json_tail`], [`Lexer::verbatim_region`],
//! [`Lexer::guest_region`],
//! [`Lexer::swallow_opaque_on_line`]), and asks the named
//! finite-byte-lookahead questions ([`Lexer::statement_head`],
//! [`Lexer::value_head`], [`Lexer::tag_opener_on_line`], the line
//! questions). It never touches bytes.
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
    /// `==`
    EqEq,
    /// `!=`
    BangEq,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,

    /// Opaque Rastok expression span, produced only by [`Lexer::expr_region`].
    ExprSource,
    /// Opaque raw region text, produced only by the region framers
    /// ([`Lexer::verbatim_region`], [`Lexer::guest_region`]).
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

/// Terminators ending a Rastok expression span at bracket depth 0, chosen
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

/// A tag argument's head (review round 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringArgument<'a> {
    /// The next significant input is not a double-quoted string —
    /// nothing was consumed.
    NotAString,
    /// A bare STRING at the argument boundary: the grammar's
    /// StrLit, consumed.
    Alone(Token<'a>),
    /// A STRING continuing into a larger expression: the Rastok expression
    /// prefix, consumed — stitch from its start.
    Prefix(Token<'a>),
}

/// A tag's same-line opener — a CLOSED type, never a raw byte at
/// the parser's surface (review round 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagOpener {
    /// `(` — a call's arguments.
    Paren,
    /// `{` — a raw or (dead) struct body.
    Brace,
}

/// How an assignment continues after its consumed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentHead {
    /// `= value`.
    Plain,
    /// `: type = value`.
    Typed,
}

/// How the next significant input opens a VALUE position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueHead {
    /// The host tokenizes a builder form.
    Structural,
    /// Delegate one expression region to Rastok expression.
    Expr,
}

/// How the next significant input opens a STATEMENT position.
///
/// This is deliberately distinct from [`ValueHead`]: a Rastok expression-shaped
/// value is valid in a value position, while the same spelling is an
/// error in a statement position (there is no implicit yield).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementHead {
    /// A host statement opener (`IDENT`, `(`, `;`, `}`, or EOF).
    Structural,
    /// A recognizable non-identifier value whose real value parser
    /// must consume it before the statement-position refusal.
    BareValue,
    /// Other invalid input, recovered one host-lexeme line at a time.
    Recovery,
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
        value: rastok_core::value::Value,
        /// Where its bytes sit in the source.
        range: core::ops::Range<usize>,
    },
    /// A follower that is neither a document nor the boundary — the
    /// reserved word admits no Rastok expression continuation.
    BadForm {
        /// The diagnostic anchor: the follower's byte (an EMPTY
        /// range at EOF, per Span's own contract).
        anchor: Span,
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
    /// The recovery form's span (the opener, an empty range).
    pub opener: Span,
    /// The diagnostic anchor: the faulty byte (an EMPTY range at
    /// EOF, per Span's own contract).
    pub anchor: Span,
    /// The reader's message.
    pub message: String,
    /// One past the recovery's last consumed byte.
    pub recovered_to: usize,
}

/// Result of the single read-only Rastok expression framer. A complete region ends
/// at the host position's FOLLOW. An incomplete region carries the
/// earlier boundary normal value parsing may commit without consuming
/// a following construct; statement recovery commits nothing.
enum ExprExtent {
    Complete { end: usize },
    Incomplete { recovery_end: usize },
}

impl ExprExtent {
    fn commit_end(&self) -> usize {
        match self {
            Self::Complete { end } => *end,
            Self::Incomplete { recovery_end } => *recovery_end,
        }
    }

    fn complete_end(&self) -> Option<usize> {
        match self {
            Self::Complete { end } => Some(*end),
            Self::Incomplete { .. } => None,
        }
    }
}

/// Hand-written scanner over one UTF-8 source — and the ONE owner
/// of both the byte cursor and the token lookahead (D-2026-08-13-1,
/// review round 5): the parser reads complete lexemes through
/// [`Self::peek`]/[`Self::next`] and asks semantic questions; it
/// never touches bytes, and no path can combine a buffered lexeme
/// with a second read of the same bytes outside this impl.
#[derive(Debug)]
pub struct Lexer<'a> {
    source: &'a str,
    source_id: SourceId,
    pos: usize,
    newline_before: bool,
    lookahead: Option<Token<'a>>,
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
            lookahead: None,
        }
    }

    /// The buffered next lexeme — lexing it if none is buffered.
    pub fn peek(&mut self, bag: &mut Bag) -> Token<'a> {
        if self.lookahead.is_none() {
            self.lookahead = Some(self.next_token(bag));
        }
        self.lookahead
            .unwrap_or_else(|| unreachable!("just filled"))
    }

    /// Consumes and returns the next lexeme.
    pub fn next(&mut self, bag: &mut Bag) -> Token<'a> {
        match self.lookahead.take() {
            Some(token) => token,
            None => self.next_token(bag),
        }
    }

    /// The next ordinary token; [`TokenKind::Eof`] at the end, forever.
    fn next_token(&mut self, bag: &mut Bag) -> Token<'a> {
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
                } else if self.peek_byte_at(1) == Some(b'=') {
                    self.pos += 2;
                    self.token(TokenKind::LtEq, start)
                } else {
                    self.pos += 1;
                    self.token(TokenKind::Lt, start)
                }
            }
            // The COMPARISON lexemes, maximal munch — the HOST's
            // own vocabulary (the simulation expect grammar), not a
            // copied Rastok expression operator set. Error recovery may observe
            // them, but no valid Rastok expression production depends on them.
            b'=' if self.peek_byte_at(1) == Some(b'=') => {
                self.pos += 2;
                self.token(TokenKind::EqEq, start)
            }
            b'!' if self.peek_byte_at(1) == Some(b'=') => {
                self.pos += 2;
                self.token(TokenKind::BangEq, start)
            }
            b'>' if self.peek_byte_at(1) == Some(b'=') => {
                self.pos += 2;
                self.token(TokenKind::GtEq, start)
            }
            // A single-quoted string is the Rastok expression language's spelling, never the
            // host's — lexed as ONE opaque Error token (its content
            // never re-speaks, and recoveries stay quote-immune).
            // An unescaped line ending bounds an unterminated token:
            // otherwise one bad quote would consume every following
            // statement before line-bounded recovery can see it.
            b'\'' => {
                self.pos += 1;
                let mut closed = false;
                loop {
                    match self.peek_byte() {
                        None | Some(b'\r' | b'\n') => break,
                        Some(b'\\') => match self.peek_byte_at(1) {
                            Some(b'\r' | b'\n') => {
                                self.pos += 1;
                                break;
                            }
                            _ => self.pos = (self.pos + 2).min(self.source.len()),
                        },
                        Some(b'\'') => {
                            self.pos += 1;
                            closed = true;
                            break;
                        }
                        Some(_) => self.pos += 1,
                    }
                }
                let diagnostic = if closed {
                    Diagnostic::new(
                        E_LEX_CHAR,
                        Phase::Parse,
                        "a single-quoted string is the Rastok expression language's spelling; host strings are \
                         double-quoted",
                    )
                } else {
                    Diagnostic::new(
                        E_LEX_UNTERMINATED_STRING,
                        Phase::Parse,
                        "this string never closes before the end of the line",
                    )
                };
                bag.push(diagnostic.with_span(Span::new(self.source_id, start..self.pos)));
                self.token(TokenKind::Error, start)
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

    /// An opaque Rastok expression span, scanned at bracket depth 0 until
    /// `terminators` — the delimitation rules of `grammar.md`. Quoted
    /// strings (both Rastok expression quote styles) are opaque and line-bounded; any
    /// closing bracket below depth 0 also terminates.
    fn expr_token(&mut self, terminators: &TerminatorSet) -> Token<'a> {
        while matches!(self.peek_byte(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
        let start = self.pos;
        let extent = self.expr_extent(self.pos, Vec::new(), terminators);
        self.commit_expr_extent(&extent);
        let end = self
            .source
            .as_bytes()
            .get(start..self.pos)
            .map_or(self.pos, |scanned| start + scanned.trim_ascii_end().len());
        Token {
            kind: TokenKind::ExprSource,
            text: &self.source[start..end],
            span: Span::new(self.source_id, start..end),
            newline_before: false,
        }
    }

    /// The unlexed source from `offset` — a read-only window for
    /// diagnostics and prefix slices; cursor re-syncs happen only
    /// inside the lexer's own delegation methods.
    #[must_use]
    /// The dispatch shape — INTERNAL: the first byte of the next
    /// significant input, from the buffered lexeme when one exists.
    /// The parser receives the CLASSIFICATIONS below, never a byte
    /// (D-2026-08-13-1, review round 6).
    fn shape(&mut self) -> Option<u8> {
        match &self.lookahead {
            Some(token) if token.kind == TokenKind::Eof => None,
            Some(token) => token.text.bytes().next(),
            None => self.first_significant_byte(),
        }
    }

    /// How a STATEMENT opens: as a host construct, a recognizable
    /// bare value, or input for line-bounded recovery. With
    /// [`Self::tag_opener_on_line`] and the line questions, this is
    /// one of THE named finite-byte-lookahead points of the grammar:
    /// Rastok expression regions may open with bytes the host refuses, so the
    /// choice reads one byte — inside the lexer.
    pub fn statement_head(&mut self) -> StatementHead {
        match self.shape() {
            None | Some(b'}' | b';' | b'(' | b'a'..=b'z' | b'A'..=b'Z' | b'_') => {
                StatementHead::Structural
            }
            Some(b'{' | b'[' | b'.' | b'"' | b'\'' | b'!' | b'-' | b'0'..=b'9') => {
                StatementHead::BareValue
            }
            _ => StatementHead::Recovery,
        }
    }

    /// How a VALUE opens: with a builder form (`{`, `.`, a word), or
    /// as a Rastok expression — same contract as
    /// [`Self::statement_head`].
    pub fn value_head(&mut self) -> ValueHead {
        match self.shape() {
            Some(b'{' | b'.' | b'a'..=b'z' | b'A'..=b'Z' | b'_') => ValueHead::Structural,
            _ => ValueHead::Expr,
        }
    }

    /// Whether a newline-terminated position is MISSING its value:
    /// nothing significant before the line's end, or the statement
    /// enders `;`/`}` — asked of the buffered lexeme when one
    /// exists.
    pub fn value_missing_on_line(&mut self) -> bool {
        match &self.lookahead {
            Some(token) => {
                token.kind == TokenKind::Eof
                    || token.newline_before
                    || matches!(token.kind, TokenKind::Semi | TokenKind::RBrace)
            }
            None => matches!(self.significant_byte_on_line(), None | Some(b';' | b'}')),
        }
    }

    /// Whether NOTHING significant remains on the current line.
    pub fn line_exhausted(&mut self) -> bool {
        match &self.lookahead {
            Some(token) => token.kind == TokenKind::Eof || token.newline_before,
            None => self.significant_on_line().is_none(),
        }
    }

    /// Whether the argument list CLOSES here (`)` or the end of
    /// input) — asked without tokenizing, across lines (a paren
    /// position is newline-free).
    pub fn arguments_close(&mut self) -> bool {
        match &self.lookahead {
            Some(token) => matches!(token.kind, TokenKind::RParen | TokenKind::Eof),
            None => matches!(self.first_significant_byte(), None | Some(b')')),
        }
    }

    /// A tag argument's head, decided WITHOUT tokenizing anything
    /// but a proven double-quoted string (D-2026-08-13-1, review
    /// round 7): a bare STRING followed by the argument boundary is
    /// the grammar's StrLit; a string continuing into an expression
    /// is the Rastok expression prefix; anything else — a single-quoted Rastok expression
    /// string included — is untouched for the value dispatch.
    pub fn string_argument(&mut self, bag: &mut Bag) -> StringArgument<'a> {
        // ONE trivia policy: a paren position is newline-free, so
        // BOTH probes cross lines — the opening string may sit on
        // the next line, and so may its boundary or continuation
        // (review round 8).
        let opens_string = match &self.lookahead {
            Some(token) => token.kind == TokenKind::Str,
            None => self.first_significant_byte() == Some(b'"'),
        };
        if !opens_string {
            return StringArgument::NotAString;
        }
        let token = self.next(bag);
        let alone = match &self.lookahead {
            Some(follower) => matches!(
                follower.kind,
                TokenKind::Comma | TokenKind::RParen | TokenKind::Eof
            ),
            None => matches!(self.first_significant_byte(), None | Some(b',' | b')')),
        };
        if alone {
            StringArgument::Alone(token)
        } else {
            StringArgument::Prefix(token)
        }
    }

    /// Whether a SAME-LINE `.` follows — the path walk's question,
    /// asked WITHOUT tokenizing the follower (a path may be chased
    /// by Rastok expression bytes outside the host vocabulary, review round 7).
    pub fn dot_follows(&mut self) -> bool {
        if let Some(token) = &self.lookahead {
            return token.kind == TokenKind::Dot && !token.newline_before;
        }
        matches!(self.significant_on_line(), Some((_, b'.')))
    }

    /// Whether an ASSIGNMENT follows the consumed path: `=` (alone —
    /// `==` belongs to expressions), or `:` (a typed assignment). Answered without
    /// tokenizing, so a bare-expression statement can refuse with
    /// ONE positional voice without first manufacturing a host token
    /// for a Rastok expression-only follower (review round 7 — the implicit yield is
    /// REMOVED).
    pub fn assignment_follows(&mut self) -> Option<AssignmentHead> {
        if let Some(token) = &self.lookahead {
            return match token.kind {
                TokenKind::Eq => Some(AssignmentHead::Plain),
                TokenKind::Colon => Some(AssignmentHead::Typed),
                _ => None,
            };
        }
        let (offset, byte) = self.significant_on_line()?;
        match byte {
            b'=' if self.source.as_bytes().get(offset + 1) != Some(&b'=') => {
                Some(AssignmentHead::Plain)
            }
            b':' => Some(AssignmentHead::Typed),
            _ => None,
        }
    }

    /// The tag-opener question: does a `(` or `{` follow ON THE
    /// SAME LINE? Answered from the buffered lexeme when one exists
    /// (its `newline_before` decides), from a read-only byte probe
    /// otherwise — a tag is a tag only WITH its same-line opener.
    pub fn tag_opener_on_line(&mut self) -> Option<TagOpener> {
        if let Some(token) = &self.lookahead {
            if token.newline_before {
                return None;
            }
            return match token.kind {
                TokenKind::LParen => Some(TagOpener::Paren),
                TokenKind::LBrace => Some(TagOpener::Brace),
                _ => None,
            };
        }
        match self.significant_on_line() {
            Some((_, b'(')) => Some(TagOpener::Paren),
            Some((_, b'{')) => Some(TagOpener::Brace),
            _ => None,
        }
    }

    /// Swallows one balanced region opening with `open` on the
    /// current line — the dead-initializer and reserved-format
    /// recovery, walked on HOST LEXEMES into a scratch bag: a
    /// closer inside a comment is trivia and never closes, a
    /// single-quoted string is ONE opaque lexeme, and every
    /// diagnostic the garbage would raise stays silent (the one
    /// migration voice already spoke — D-2026-08-13-1, review
    /// round 6). Returns whether a region was swallowed.
    pub fn swallow_opaque_on_line(&mut self, open: TagOpener) -> bool {
        let (open_kind, close_kind) = match open {
            TagOpener::Paren => (TokenKind::LParen, TokenKind::RParen),
            TagOpener::Brace => (TokenKind::LBrace, TokenKind::RBrace),
        };
        let mut scratch = Bag::new();
        let opener = self.peek(&mut scratch);
        if opener.newline_before || opener.kind != open_kind {
            return false;
        }
        let _ = self.next(&mut scratch);
        let mut depth: u32 = 1;
        loop {
            let token = self.next(&mut scratch);
            match token.kind {
                TokenKind::Eof => break,
                kind if kind == open_kind => depth += 1,
                kind if kind == close_kind => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// The mirror of the terminator test, asked of a buffered
    /// lexeme: would the region scan terminate IMMEDIATELY on it?
    /// A boundary lexeme belongs to the HOST and must survive.
    fn token_terminates(token: &Token<'_>, terminators: &TerminatorSet) -> bool {
        if token.kind == TokenKind::Eof {
            return true;
        }
        if token.newline_before && terminators.newline {
            return true;
        }
        if matches!(
            token.kind,
            TokenKind::RParen | TokenKind::RBrack | TokenKind::RBrace
        ) {
            return true;
        }
        if let Some(byte) = token.text.as_bytes().first()
            && terminators.bytes.contains(byte)
        {
            return true;
        }
        if terminators.arrow && token.kind == TokenKind::Arrow {
            return true;
        }
        token.kind == TokenKind::Ident && terminators.keywords.contains(&token.text)
    }

    /// One Rastok expression region lexeme (D-2026-08-13-1) — complete to the
    /// position's FOLLOW, or bounded at the safe recovery point of an
    /// incomplete extent. `prefix_start` names
    /// the already-consumed lexeme(s) beginning the expression; the
    /// stitching re-read of that immutable slice happens HERE, under
    /// the same owner as the buffer and the cursor. A buffered
    /// boundary lexeme ends the region before itself and SURVIVES;
    /// a buffered opener deepens the scan it already entered.
    pub fn expr_region(
        &mut self,
        prefix_start: Option<usize>,
        terminators: &TerminatorSet,
    ) -> Token<'a> {
        if let Some(token) = &self.lookahead {
            let start = prefix_start.unwrap_or_else(|| token.span.range().start);
            if Self::token_terminates(token, terminators) {
                let end = token.span.range().start.max(start);
                return self.stitched(start, end);
            }
            let expected = match self.lookahead.take().map(|token| token.kind) {
                Some(TokenKind::LParen) => vec![b')'],
                Some(TokenKind::LBrack) => vec![b']'],
                Some(TokenKind::LBrace) => vec![b'}'],
                _ => Vec::new(),
            };
            let extent = self.expr_extent(self.pos, expected, terminators);
            self.commit_expr_extent(&extent);
            return self.stitched(start, self.pos);
        }
        match prefix_start {
            Some(start) => {
                let extent = self.expr_extent(self.pos, Vec::new(), terminators);
                self.commit_expr_extent(&extent);
                self.stitched(start, self.pos)
            }
            None => {
                // Same-line comments before a value are trivia; a
                // bare newline is not — it still reads as an empty
                // expression.
                self.advance_within_line();
                self.expr_token(terminators)
            }
        }
    }

    /// Commits and returns one Rastok expression region only when the single Rastok expression
    /// framer reaches this position's boundary with every delimiter
    /// and quote closed. `None` leaves both cursor and lookahead
    /// untouched, so line-bounded host recovery can preserve the next
    /// statement. This chooses only ERROR-RECOVERY extent after the
    /// parser has already rejected the statement production; it never
    /// chooses between valid grammar productions.
    pub fn try_complete_expr_region(
        &mut self,
        prefix_start: Option<usize>,
        terminators: &TerminatorSet,
    ) -> Option<Token<'a>> {
        if let Some(token) = &self.lookahead {
            let start = prefix_start.unwrap_or_else(|| token.span.range().start);
            // When this buffered token STARTS the misplaced region,
            // its `newline_before` describes trivia before the value,
            // not a boundary after it. Continuation tokens retain the
            // ordinary terminator rule.
            let mut first = *token;
            if prefix_start.is_none() {
                first.newline_before = false;
            }
            if Self::token_terminates(&first, terminators) {
                let end = token.span.range().start.max(start);
                return Some(self.stitched(start, end));
            }
            let expected = match token.kind {
                TokenKind::LParen => vec![b')'],
                TokenKind::LBrack => vec![b']'],
                TokenKind::LBrace => vec![b'}'],
                _ => Vec::new(),
            };
            let extent = self.expr_extent(self.pos, expected, terminators);
            let end = extent.complete_end()?;
            let _ = self.lookahead.take();
            self.pos = end;
            return Some(self.stitched(start, end));
        }

        let start = prefix_start.unwrap_or(self.pos);
        let extent = self.expr_extent(self.pos, Vec::new(), terminators);
        let end = extent.complete_end()?;
        self.pos = end;
        Some(self.stitched(start, end))
    }

    /// The stitched region token: `source[start..end]`, its OWN end
    /// trimmed (ASCII, newline included — never a Unicode trim).
    fn stitched(&self, start: usize, end: usize) -> Token<'a> {
        let end = start
            + self.source[start..end]
                .trim_end_matches([' ', '\t', '\r', '\n'])
                .len();
        Token {
            kind: TokenKind::ExprSource,
            text: &self.source[start..end],
            span: Span::new(self.source_id, start..end),
            newline_before: false,
        }
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
                    let end = (at + 1).min(self.source.len());
                    self.pos = offset;
                    let recovered_to = self.skip_json_recovery(terminators);
                    JsonTail::Unreadable(JsonRefusal {
                        opener: Span::new(self.source_id, offset..offset),
                        anchor: Span::new(self.source_id, at..end),
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
        // the json form is invalid — `json` cannot begin a Rastok expression
        // expression, being reserved (review round 3). The walk is
        // the HOST-bounded one: garbage never crosses the line of a
        // newline-ended position (review round 4).
        self.pos = offset;
        let recovered_to = self.skip_bad_form(terminators);
        let end = (offset + 1).min(self.source.len());
        JsonTail::BadForm {
            anchor: Span::new(self.source_id, offset..end),
            recovered_to,
        }
    }

    /// The ONE Rastok expression framer. It is read-only so the normal value path
    /// and the misplaced-value recovery consult exactly the same
    /// delimiter, quote, comment, and terminator rules. A mismatched
    /// closer never pays down an unrelated opener. It may scan past a
    /// newline to prove a valid balanced expression, but an incomplete
    /// newline-ended value commits only its first safe recovery line;
    /// the Rastok expression compiler, not this host framer, diagnoses its syntax.
    fn expr_extent(
        &self,
        start: usize,
        mut expected: Vec<u8>,
        terminators: &TerminatorSet,
    ) -> ExprExtent {
        let bytes = self.source.as_bytes();
        let mut pos = start;
        let mut recovery_end = None;
        while let Some(&byte) = bytes.get(pos) {
            if expected.is_empty() && self.terminates_at(pos, byte, terminators) {
                return ExprExtent::Complete { end: pos };
            }
            if byte == b'/'
                && let Some(comment) = rastok_core::trivia::comment_at(self.source, pos)
            {
                if !comment.terminated {
                    // Stop BEFORE the comment. The host trivia reader is
                    // the one authority that diagnoses an unterminated
                    // comment, and it runs during parsing — unlike the Rastok expression
                    // checker, which cannot run after missing-closer parse
                    // cascades have already rejected the source.
                    return ExprExtent::Incomplete { recovery_end: pos };
                }
                let reaches_line_end = comment.multiline
                    || bytes.get(comment.end) == Some(&b'\n')
                    || comment.end == bytes.len();
                if expected.is_empty() && terminators.newline && reaches_line_end {
                    return ExprExtent::Complete { end: pos };
                }
                if !expected.is_empty() && terminators.newline && reaches_line_end {
                    recovery_end.get_or_insert(comment.end);
                }
                pos = comment.end;
                continue;
            }
            match byte {
                b'(' => expected.push(b')'),
                b'[' => expected.push(b']'),
                b'{' => expected.push(b'}'),
                quote @ (b'"' | b'\'') => {
                    pos += 1;
                    let mut closed = false;
                    while let Some(&quoted) = bytes.get(pos) {
                        match quoted {
                            b'\r' | b'\n' => {
                                // Ordinary Rastok expression strings are line-bounded.
                                // Include the malformed prefix, keep the
                                // following line for the host, and let the
                                // Rastok expression compiler own the syntax diagnostic.
                                return ExprExtent::Incomplete { recovery_end: pos };
                            }
                            b'\\' => {
                                if matches!(bytes.get(pos + 1), Some(b'\r' | b'\n')) {
                                    return ExprExtent::Incomplete {
                                        recovery_end: pos + 1,
                                    };
                                }
                                pos = (pos + 2).min(bytes.len());
                            }
                            _ if quoted == quote => {
                                pos += 1;
                                closed = true;
                                break;
                            }
                            _ => pos += 1,
                        }
                    }
                    if !closed {
                        return ExprExtent::Incomplete { recovery_end: pos };
                    }
                    continue;
                }
                b')' | b']' | b'}' => match expected.last() {
                    Some(wanted) if *wanted == byte => {
                        expected.pop();
                    }
                    None => {
                        return ExprExtent::Complete { end: pos };
                    }
                    Some(_) => {
                        return ExprExtent::Incomplete {
                            recovery_end: recovery_end.unwrap_or(pos),
                        };
                    }
                },
                b'\r' | b'\n' if !expected.is_empty() && terminators.newline => {
                    recovery_end.get_or_insert(pos);
                }
                _ => {}
            }
            pos += 1;
        }
        if expected.is_empty() {
            ExprExtent::Complete { end: pos }
        } else {
            ExprExtent::Incomplete {
                recovery_end: recovery_end.unwrap_or(pos),
            }
        }
    }

    fn commit_expr_extent(&mut self, extent: &ExprExtent) {
        self.pos = extent.commit_end();
    }

    /// A VERBATIM region (`text { … }`, `base64 { … }`): the OPENER
    /// is consumed HERE, so the region is selected before any
    /// content byte can meet the host vocabulary (D-2026-08-13-1,
    /// review round 6). Content is verbatim under the text
    /// contract: braces balance, EVERY other byte is ordinary data —
    /// quotes and apostrophes included (`text { don't }` is text),
    /// no escapes, no interpolation. Exact bytes, exact span, one
    /// forward commit.
    pub fn verbatim_region(&mut self, bag: &mut Bag) -> Option<Token<'a>> {
        self.region_after_opener(bag, false)
    }

    /// A GUEST region (`jq { … }`, `bash { … }`, `sql { … }`, …):
    /// the opener consumed HERE, the ENTIRE region belonging to the
    /// registered guest compiler — exact bytes, exact span, no
    /// normalization; the host never tokenizes the content and
    /// resumes exactly past the closer; guest errors come back as
    /// typed results remapped by span.
    ///
    /// DELIMITATION CONTRACT (frozen, and NAMED as an open
    /// question): brace balance with both quote styles skipped
    /// opaquely (backslash escapes). The host cannot know a guest's
    /// comment or string lexicon — grammar.md exposes the failure
    /// modes and the arbitration options (authoring contract /
    /// fenced delimiters / guest-registered framers) rather than
    /// growing per-guest lexicons here.
    pub fn guest_region(&mut self, bag: &mut Bag) -> Option<Token<'a>> {
        self.region_after_opener(bag, true)
    }

    fn region_after_opener(&mut self, bag: &mut Bag, quotes_opaque: bool) -> Option<Token<'a>> {
        // TOTAL, no panic: `None` when the next lexeme is not the
        // opener (nothing consumed beyond the buffered peek) — the
        // caller speaks its own expected-diagnostic.
        if self.peek(bag).kind != TokenKind::LBrace {
            return None;
        }
        let _ = self.next(bag);
        let start = self.pos;
        let bytes = self.source.as_bytes();
        let mut pos = start;
        let mut depth: u32 = 0;
        loop {
            match bytes.get(pos).copied() {
                None => {
                    bag.push(
                        Diagnostic::new(
                            E_LEX_UNTERMINATED_BLOCK,
                            Phase::Parse,
                            "this raw block never closes",
                        )
                        .with_span(Span::new(self.source_id, start..pos)),
                    );
                    break;
                }
                Some(b'{') => {
                    depth += 1;
                    pos += 1;
                }
                Some(b'}') => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    pos += 1;
                }
                Some(quote @ (b'"' | b'\'')) if quotes_opaque => {
                    let quote_start = pos;
                    pos += 1;
                    loop {
                        match bytes.get(pos).copied() {
                            None => {
                                bag.push(
                                    Diagnostic::new(
                                        E_LEX_UNTERMINATED_STRING,
                                        Phase::Parse,
                                        "this string never closes",
                                    )
                                    .with_span(Span::new(self.source_id, quote_start..pos)),
                                );
                                break;
                            }
                            Some(b'\\') => pos = (pos + 2).min(bytes.len()),
                            Some(byte) => {
                                pos += 1;
                                if byte == quote {
                                    break;
                                }
                            }
                        }
                    }
                }
                Some(_) => pos += 1,
            }
        }
        let token = Token {
            kind: TokenKind::RawText,
            text: &self.source[start..pos],
            span: Span::new(self.source_id, start..pos),
            newline_before: false,
        };
        if bytes.get(pos) == Some(&b'}') {
            pos += 1;
        }
        // Forward by arithmetic, no assert: the local cursor started
        // AT self.pos and only ever incremented.
        self.pos = pos;
        Some(token)
    }

    /// Consumes trivia so an opaque scan starts at significant input.
    /// Needed before a shape-dispatched Rastok expression scan: the scan itself
    /// treats a leading newline as an empty expression, but the
    /// newline separating two statements is not part of either. A
    /// no-op when a lexeme is buffered — its peek consumed the
    /// trivia already.
    pub fn advance_to_significant(&mut self, bag: &mut Bag) {
        if self.lookahead.is_none() {
            self.skip_trivia(bag);
        }
    }

    /// Consumes horizontal trivia — spaces and comments — up to the
    /// next newline outside a comment. A value scan starts past its
    /// same-line trivia, while a bare newline still reads as an empty
    /// expression, per the delimitation rules.
    fn advance_within_line(&mut self) {
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
    /// value positions: Rastok expression text must reach the expression chain untouched,
    /// never partially interpreted — or rejected — by this lexer.
    #[must_use]
    fn first_significant_byte(&self) -> Option<u8> {
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
    fn significant_byte_on_line(&self) -> Option<u8> {
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
    fn significant_on_line(&self) -> Option<(usize, u8)> {
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

    /// Skips a NON-JSON garbage form (a `json` follower that opens
    /// no document, D-2026-08-13-1 review round 4) to the HOST
    /// position's boundary. Unlike the JSON-document recovery, the
    /// bound is the position itself: at a newline-ended position the
    /// newline is a STRONG boundary even inside a paren the garbage
    /// opened — a balanced same-line form (`json(1)`) is swallowed
    /// whole, an unbalanced one (`json(1` …) ends at its line.
    /// Quoted content (both styles) is opaque; `;` always stops; a
    /// closer the garbage did not open stops (it is the CALLER's).
    fn skip_bad_form(&mut self, terminators: &TerminatorSet) -> usize {
        let bytes = self.source.as_bytes();
        let mut expected: Vec<u8> = Vec::new();
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
            if byte == b'/'
                && let Some(comment) = rastok_core::trivia::comment_at(self.source, self.pos)
            {
                // Trivia in the garbage too: a boundary byte inside
                // a comment never bounds. An unterminated or
                // line-crossing comment ends the walk at a
                // newline-ended position — the comment stays the
                // HOST's.
                if !comment.terminated || (terminators.newline && comment.multiline) {
                    return self.pos;
                }
                self.pos = comment.end;
                continue;
            }
            if byte == b';' {
                return self.pos;
            }
            if terminators.newline && byte == b'\n' {
                return self.pos;
            }
            if expected.is_empty() && terminators.bytes.contains(&byte) {
                return self.pos;
            }
            match byte {
                b'"' | b'\'' => {
                    quote = Some(byte);
                }
                b'(' => expected.push(b')'),
                b'{' => expected.push(b'}'),
                b'[' => expected.push(b']'),
                b')' | b'}' | b']' => {
                    if expected.last() == Some(&byte) {
                        expected.pop();
                    } else {
                        // A closer the garbage never opened — the
                        // caller's, a boundary.
                        return self.pos;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        self.pos
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
    fn skip_json_recovery(&mut self, terminators: &TerminatorSet) -> usize {
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
            // Strong boundaries: impossible in JSON outside strings
            // (a non-json BadForm walks `skip_bad_form` instead —
            // review round 4 split the two invariants).
            if byte == b';' || byte == b')' {
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
                b'}' | b']' => {
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
            match byte {
                b'\r' | b'\n' => break,
                b'\\' => match self.peek_byte_at(1) {
                    Some(b'\r' | b'\n') => {
                        self.pos += 1;
                        break;
                    }
                    _ => self.pos = (self.pos + 2).min(self.source.len()),
                },
                b'"' => {
                    self.pos += 1;
                    return self.token(TokenKind::Str, start);
                }
                _ => self.pos += 1,
            }
        }
        bag.push(
            Diagnostic::new(
                E_LEX_UNTERMINATED_STRING,
                Phase::Parse,
                "this string never closes before the end of the line",
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

    #[test]
    fn a_region_is_selected_at_its_opener_before_any_content_lexing() {
        // The atomic-frontier guarantee (review round 6): the region
        // method CONSUMES the opener itself, so no content byte can
        // meet the host vocabulary — a buffered opener composes, and
        // the leading trivia of the region stays region bytes.
        let mut bag = Bag::new();
        let mut lexer = Lexer::new(SourceId::new(0), "{  don't \"quote\" {ok} } tail");
        let opener = lexer.peek(&mut bag);
        assert_eq!(opener.kind, TokenKind::LBrace, "the opener buffers safely");
        let region = lexer.verbatim_region(&mut bag).expect("the opener is next");
        assert_eq!(
            region.text, "  don't \"quote\" {ok} ",
            "verbatim: quotes and apostrophes are ORDINARY bytes, braces balance"
        );
        let next = lexer.next(&mut bag);
        assert_eq!(next.text, "tail", "the host resumes past the closer");
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
    }
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

        for (label, source, first) in [
            ("double quote", "\"oops\nnext", "\"oops"),
            ("single quote", "'oops\nnext", "'oops"),
            ("escaped double quote newline", "\"oops\\\nnext", "\"oops\\"),
            ("escaped single quote newline", "'oops\\\nnext", "'oops\\"),
        ] {
            let (tokens, bag) = lex(source);
            assert_eq!(
                tokens,
                [(TokenKind::Error, first), (TokenKind::Ident, "next"),],
                "{label}: the next line remains tokenizable"
            );
            assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
            assert_eq!(
                bag.diagnostics()[0].code(),
                E_LEX_UNTERMINATED_STRING,
                "{label}"
            );
        }
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

    fn lex_expression(source: &str, terminators: &TerminatorSet) -> (String, String) {
        let mut bag = Bag::new();
        let mut lexer = Lexer::new(SourceId::new(0), source);
        let token = lexer.expr_token(terminators);
        assert_eq!(token.kind, TokenKind::ExprSource);
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
    fn an_expression_span_stops_at_the_unconsumed_terminator() {
        let (expr, rest) = lex_expression("meta.app.user_id ; x = 1", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "meta.app.user_id");
        assert_eq!(rest, ";x=1");
    }

    #[test]
    fn an_expression_span_tracks_bracket_depth() {
        let (expr, _) = lex_expression("{'a': 1}.size() > 0 ; tail", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "{'a': 1}.size() > 0");
    }

    #[test]
    fn an_expression_span_skips_terminators_inside_strings() {
        let (expr, _) = lex_expression("\"a;b\" + 'c}d' ; tail", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "\"a;b\" + 'c}d'");
    }

    #[test]
    fn a_newline_terminates_at_depth_zero_only() {
        let (expr, _) = lex_expression("(a &&\n b)\n next = 1", &ATTRIBUTE_POSITION);
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
        let (expr, _) = lex_expression(
            "count(p) + shaving >= k having size(p) > 0",
            &GUARD_POSITION,
        );
        assert_eq!(expr, "count(p) + shaving >= k");
        let (arrowed, _) = lex_expression("data.kind == \"x\" -> audit", &GUARD_POSITION);
        assert_eq!(arrowed, "data.kind == \"x\"");
    }

    #[test]
    fn an_unbalanced_closer_terminates_defensively() {
        let (expr, _) = lex_expression("a + b) leftovers", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "a + b");

        let (expr, rest) = lex_expression("[a + b)\nnext", &ATTRIBUTE_POSITION);
        assert_eq!(expr, "[a + b", "a `)` never closes a `[` opener");
        assert!(
            rest.starts_with(')'),
            "the mismatched closer survives for host recovery: {rest:?}"
        );
    }
}
