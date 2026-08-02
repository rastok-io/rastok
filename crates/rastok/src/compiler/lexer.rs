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

/// Hand-written scanner over one UTF-8 source.
#[derive(Debug)]
pub struct Lexer<'a> {
    source: &'a str,
    source_id: SourceId,
    pos: usize,
}

impl<'a> Lexer<'a> {
    /// Starts scanning `source`, attributing spans to `source_id`.
    #[must_use]
    pub fn new(source_id: SourceId, source: &'a str) -> Self {
        Self {
            source,
            source_id,
            pos: 0,
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
        let mut depth: u32 = 0;
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
                _ => self.pos += 1,
            }
        }
        let end = self
            .source
            .as_bytes()
            .get(start..self.pos)
            .map_or(self.pos, |scanned| start + scanned.trim_ascii_end().len());
        Token {
            kind: TokenKind::CelExpr,
            text: &self.source[start..end],
            span: Span::new(self.source_id, start..end),
        }
    }

    /// Raw guest text after an already-consumed `{`, scanned to the
    /// matching `}` under the frozen balanced-brace delimitation; the
    /// closing brace is consumed, the text excludes it. The host never
    /// reads the content — quoted strings are skipped opaquely.
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
        };
        if self.peek_byte() == Some(b'}') {
            self.pos += 1;
        }
        token
    }

    /// Repositions the scanner at an absolute byte offset. Used by the
    /// parser to hand a peeked token back before an opaque-span scan.
    pub fn rewind(&mut self, pos: usize) {
        debug_assert!(pos <= self.source.len());
        self.pos = pos;
    }

    fn at_terminator(&self, byte: u8, terminators: &TerminatorSet) -> bool {
        if terminators.bytes.contains(&byte) {
            return true;
        }
        if terminators.newline && byte == b'\n' {
            return true;
        }
        if terminators.arrow && self.source.as_bytes()[self.pos..].starts_with(b"->") {
            return true;
        }
        if !terminators.keywords.is_empty() && self.at_word_boundary() {
            let rest = &self.source[self.pos..];
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

    fn at_word_boundary(&self) -> bool {
        self.pos == 0
            || !self
                .source
                .as_bytes()
                .get(self.pos - 1)
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
        loop {
            match self.peek_byte() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/') if self.peek_byte_at(1) == Some(b'/') => {
                    self.eat_while(|byte| byte != b'\n');
                }
                Some(b'/') if self.peek_byte_at(1) == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    match self.source[self.pos..].find("*/") {
                        Some(offset) => self.pos += offset + 2,
                        None => {
                            self.pos = self.source.len();
                            bag.push(
                                Diagnostic::new(
                                    E_LEX_UNTERMINATED_COMMENT,
                                    Phase::Parse,
                                    "this block comment never closes",
                                )
                                .with_span(Span::new(self.source_id, start..self.pos)),
                            );
                        }
                    }
                }
                _ => return,
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
        b'>' => TokenKind::Gt,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
