//! Hand-written recursive-descent parser for `.rkg` source.
//!
//! One token of lookahead over [`Lexer`], per the LL(1) contract of
//! `grammar.md`. On a fault the parser reports, resynchronizes at the
//! next declaration keyword or closing brace, leaves an
//! [`ErrorNode`] hole, and keeps parsing — healthy declarations of a
//! faulty file still reach the tree.
//!
//! The parsed subset covers declarations, handlers, statements, and
//! wiring. `lab` bodies are recognized and skipped whole until the
//! scenario stage.

use crate::compiler::lexer::{
    E_LEX_UNTERMINATED_COMMENT, E_LEX_UNTERMINATED_STRING, Lexer, StatementHead, TerminatorSet,
    Token, TokenKind, ValueHead,
};
use crate::compiler::syntax::{
    Annotation, AspectEvent, Attribute, BaseType, CmpOp, ConnectorDecl, EdgeDecl, EdgeEndpoint,
    ErrorNode, ExpectStep, ExprSource, FieldDecl, FrontendPayload, FrontendValue, GraphDecl,
    GraphItem, Guard, HandlerDecl, IntLit, LValue, LabDecl, LifetimeValue, MockDecl, Name,
    OperatorDecl, OperatorItem, PartitionDecl, PartitionField, PerClause, PortDecl, PortDirection,
    PortRef, ScalarType, ScenarioDecl, ScenarioItem, SimulationDecl, Statement, StructBody,
    StructEntry, SubBlock, SyntaxTree, TaggedCaseDecl, TopDecl, Trigger, TypeRef, ValDecl, Value,
    VarDecl, WindowMode,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};

/// Diagnostic for a token that breaks the expected production.
pub const E_PARSE_EXPECTED: Code = Code::new("E-PARSE-EXPECTED");
/// Diagnostic for grammatical syntax outside the implemented subset.
pub const E_PARSE_DEFERRED: Code = Code::new("E-PARSE-DEFERRED");

/// Diagnostic for a `json` literal the house reader refuses — its
/// own code: JSON is not Rastok expression (D-2026-08-12-1, review round).
pub const E_JSON_VALUE: Code = Code::new("E-JSON-VALUE");

/// Terminators for attribute, initializer, and emission positions.
const ATTR_POSITION: TerminatorSet = TerminatorSet {
    bytes: b";}",
    newline: true,
    arrow: false,
    keywords: &[],
};

/// Terminators for struct-body field values, where `,` separates.
const STRUCT_POSITION: TerminatorSet = TerminatorSet {
    bytes: b",;}",
    newline: true,
    arrow: false,
    keywords: &[],
};

/// Terminators for parenthesized positions; newlines are free.
const PAREN_POSITION: TerminatorSet = TerminatorSet {
    bytes: b",)",
    newline: false,
    arrow: false,
    keywords: &[],
};

/// Terminators for the `window` duration, closed by its mode word.
const WINDOW_POSITION: TerminatorSet = TerminatorSet {
    bytes: b";}",
    newline: true,
    arrow: false,
    keywords: &["tumbling", "sliding"],
};

/// Terminators for guard positions, per the delimitation rules.
const GUARD_POSITION: TerminatorSet = TerminatorSet {
    bytes: b"{",
    newline: true,
    arrow: true,
    keywords: &["where", "having", "by", "within", "last", "when"],
};

/// Declaration keywords the recovery resynchronizes on.
const SYNC_WORDS: &[&str] = &[
    "graph",
    "partition",
    "operator",
    "connector",
    "val",
    "var",
    "input",
    "output",
    "on",
    "defaults",
    "lab",
    "simulation",
    "experiment",
    "theorem",
];

/// The tagged-initializer spellings D-2026-08-12-1 killed — kept
/// only to refuse with a useful migration.
const DEAD_INITIALIZER_TAGS: &[&str] = &[
    "oauth2_client_credentials",
    "measure",
    "exponential",
    "linear",
    "constant",
    "fibonacci",
    "on_crash",
];

/// Whether a word belonged to the pre-contract structured-initializer
/// surface. The checker keeps its migration diagnostic only when a genuine
/// tagged-union target does not make the same shape meaningful.
pub(crate) fn is_dead_initializer_tag(word: &str) -> bool {
    DEAD_INITIALIZER_TAGS.contains(&word)
}

/// Parses one `.rkg` source into a syntax tree, accumulating
/// diagnostics; the tree is returned even when faulty.
pub fn parse(
    source_id: SourceId,
    source: &str,
    registry: &crate::contract::Registry,
    bag: &mut Bag,
) -> SyntaxTree {
    Parser {
        lexer: Lexer::new(source_id, source),
        registry,
        source_id,
        last_end: 0,
        bag,
    }
    .file()
}

struct Parser<'src, 'bag> {
    lexer: Lexer<'src>,
    registry: &'bag crate::contract::Registry,
    source_id: SourceId,
    last_end: usize,
    bag: &'bag mut Bag,
}

/// What line-bounded statement recovery consumed. Its primary
/// structural lexical fault is retained: suppressing ordinary garbage
/// diagnostics is useful, suppressing the reason recovery stopped is
/// not.
struct BareRecovery {
    range: core::ops::Range<usize>,
    lexical_fault: Option<Diagnostic>,
}

impl<'src> Parser<'src, '_> {
    // ----- token plumbing (the lexer owns the lookahead) -----

    fn peek(&mut self) -> Token<'src> {
        self.lexer.peek(self.bag)
    }

    fn bump(&mut self) -> Token<'src> {
        let token = self.lexer.next(self.bag);
        if token.kind != TokenKind::Eof {
            self.last_end = token.span.range().end;
        }
        token
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token<'src>> {
        (self.peek().kind == kind).then(|| self.bump())
    }

    fn at_word(&mut self, word: &str) -> bool {
        let token = self.peek();
        token.kind == TokenKind::Ident && token.text == word
    }

    fn expect(&mut self, kind: TokenKind, expected: &str) -> Option<Token<'src>> {
        if self.peek().kind == kind {
            Some(self.bump())
        } else {
            self.error_expected(expected);
            None
        }
    }

    fn error_expected(&mut self, expected: &str) {
        let token = self.peek();
        let found = match token.kind {
            TokenKind::Eof => String::from("end of input"),
            _ => format!("`{}`", token.text),
        };
        self.bag.push(
            Diagnostic::new(
                E_PARSE_EXPECTED,
                Phase::Parse,
                format!("expected {expected}, found {found}"),
            )
            .with_span(token.span)
            .with_field("expected", expected)
            .with_field("found", found),
        );
    }

    fn deferred(&mut self, construct: &'static str, span: Span) {
        self.bag.push(
            Diagnostic::new(
                E_PARSE_DEFERRED,
                Phase::Parse,
                format!("{construct} syntax is not implemented by this build yet"),
            )
            .with_span(span)
            .with_field("construct", construct),
        );
    }

    /// Consumes a refused bare statement to its STRONG boundary —
    /// host lexemes into a scratch bag. Ordinary garbage diagnostics
    /// stay silent; an unterminated string/comment is retained as the
    /// one truthful voice. Strong boundaries: `;`, a depth-zero `}`,
    /// the next line's first lexeme, EOF. PROGRESSION IS STRUCTURAL: the
    /// first lexeme is consumed unconditionally unless it is `;`,
    /// `}` or EOF — which the statement loop itself consumes or
    /// breaks on (review round 8: a lone `)` looped forever).
    fn skip_bare_statement(&mut self, tail_of_consumed: bool) -> BareRecovery {
        let mut scratch = Bag::new();
        let mut depth: u32 = 0;
        let mut consumed_any = tail_of_consumed;
        let start = self.lexer.peek(&mut scratch).span.range().start;
        let mut end = start;
        loop {
            let token = self.lexer.peek(&mut scratch);
            let boundary = match token.kind {
                TokenKind::Eof | TokenKind::Semi => true,
                TokenKind::RBrace if depth == 0 => true,
                _ => consumed_any && token.newline_before,
            };
            if boundary {
                break;
            }
            let token = self.lexer.next(&mut scratch);
            consumed_any = true;
            end = token.span.range().end;
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        let lexical_fault = scratch.into_diagnostics().into_iter().find(|diagnostic| {
            matches!(
                diagnostic.code(),
                E_LEX_UNTERMINATED_STRING | E_LEX_UNTERMINATED_COMMENT
            )
        });
        if let Some(fault_end) = lexical_fault
            .as_ref()
            .and_then(|diagnostic| diagnostic.spans().first())
            .map(|span| span.range().end)
        {
            end = end.max(fault_end);
        }
        self.last_end = self.last_end.max(end);
        BareRecovery {
            range: start..end,
            lexical_fault,
        }
    }

    fn bare_statement_refusal(&mut self, range: core::ops::Range<usize>) -> Statement {
        let span = Span::new(self.source_id, range.start..range.end.max(range.start));
        self.bag.push(
            Diagnostic::new(
                E_PARSE_EXPECTED,
                Phase::Parse,
                "a bare expression is not a statement; write `out = <value>` \
                 (or `err = Kind { … }`)",
            )
            .with_span(span)
            .with_field("expected", "a statement")
            .with_field("found", "a bare expression"),
        );
        Statement::Error(ErrorNode { span })
    }

    fn recovered_bare_statement(&mut self, start: usize, recovery: BareRecovery) -> Statement {
        let end = recovery.range.end.max(start);
        let span = Span::new(self.source_id, start..end);
        if let Some(diagnostic) = recovery.lexical_fault {
            self.bag.push(diagnostic);
            Statement::Error(ErrorNode { span })
        } else {
            self.bare_statement_refusal(start..end)
        }
    }

    /// Finishes recovery after a host statement production has
    /// already emitted its own diagnostic. Unlike the generic
    /// declaration recovery, this stops at the statement boundary so
    /// a malformed multi-emit cannot consume the next healthy line.
    fn recover_statement_tail(&mut self, start: usize) -> ErrorNode {
        let recovery = self.skip_bare_statement(true);
        let end = self.last_end.max(recovery.range.end).max(start);
        if let Some(diagnostic) = recovery.lexical_fault {
            self.bag.push(diagnostic);
        }
        ErrorNode {
            span: Span::new(self.source_id, start..end),
        }
    }

    /// Parses a recognizable value with its real owner, then rejects
    /// only its placement. This is an error production, never an
    /// implicit yield: the parsed value is deliberately discarded and
    /// the syntax tree receives one [`Statement::Error`] hole.
    fn bare_value_statement(&mut self) -> Statement {
        let diagnostics_before = self.bag.diagnostics().len();
        let (start, value) = match self.lexer.value_head() {
            ValueHead::Structural => {
                let start = self.peek().span.range().start;
                (start, self.present_value(&ATTR_POSITION))
            }
            ValueHead::Expr => {
                // This value starts a NEW statement line. Cross the
                // preceding statement trivia before probing/framing;
                // otherwise that leading newline would be the
                // ATTR_POSITION terminator and yield an empty lexeme.
                self.lexer.advance_to_significant(self.bag);
                let Some(expr) = self.try_complete_expr_lexeme(None, &ATTR_POSITION) else {
                    let recovery = self.skip_bare_statement(false);
                    let start = recovery.range.start;
                    return self.recovered_bare_statement(start, recovery);
                };
                (expr.span.range().start, Value::Expr(expr))
            }
        };
        let end = self.last_end.max(start);
        if self.bag.diagnostics().len() != diagnostics_before || matches!(value, Value::Error(_)) {
            Statement::Error(ErrorNode {
                span: Span::new(self.source_id, start..end),
            })
        } else {
            self.bare_statement_refusal(start..end)
        }
    }

    /// Skips tokens until a top-of-declaration keyword or the closing
    /// brace of the current block, balancing nested braces.
    fn recover(&mut self, start: usize) -> ErrorNode {
        let mut depth: u32 = 0;
        loop {
            let token = self.peek();
            match token.kind {
                TokenKind::Eof => break,
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.bump();
                }
                TokenKind::Ident if depth == 0 && SYNC_WORDS.contains(&token.text) => break,
                _ => {
                    self.bump();
                }
            }
        }
        ErrorNode {
            span: self.span_from(start),
        }
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(self.source_id, start..self.last_end.max(start))
    }

    fn name(&mut self, expected: &str) -> Option<Name> {
        let token = self.expect(TokenKind::Ident, expected)?;
        Some(Name {
            text: String::from(token.text),
            span: token.span,
        })
    }

    // ----- productions -----

    fn file(mut self) -> SyntaxTree {
        let mut decls = Vec::new();
        loop {
            let token = self.peek();
            match token.kind {
                TokenKind::Eof => break,
                TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Ident => {
                    let start = token.span.range().start;
                    match token.text {
                        "graph" => decls.push(match self.graph_decl() {
                            Some(graph) => TopDecl::Graph(graph),
                            None => TopDecl::Error(self.recover(start)),
                        }),
                        "val" => decls.push(match self.val_decl() {
                            Some(val) => TopDecl::Val(val),
                            None => TopDecl::Error(self.recover(start)),
                        }),
                        "lab" => decls.push(self.lab_decl(start)),
                        "type" => {
                            self.bump();
                            let decl = (|| {
                                let name = self.name("a type name")?;
                                self.expect(TokenKind::Eq, "`=`")?;
                                let ty = self.type_ref()?;
                                Some(crate::compiler::syntax::TypeDecl {
                                    name,
                                    ty,
                                    span: self.span_from(start),
                                })
                            })();
                            decls.push(match decl {
                                Some(decl) => TopDecl::Type(decl),
                                None => TopDecl::Error(self.recover(start)),
                            });
                        }
                        _ => {
                            self.error_expected("a top-level declaration");
                            self.bump();
                            decls.push(TopDecl::Error(self.recover(start)));
                        }
                    }
                }
                _ => {
                    let start = token.span.range().start;
                    self.error_expected("a top-level declaration");
                    self.bump();
                    decls.push(TopDecl::Error(self.recover(start)));
                }
            }
        }
        SyntaxTree { decls }
    }

    /// `lab IDENT { (simulation | experiment | theorem)* }`.
    fn lab_decl(&mut self, start: usize) -> TopDecl {
        self.bump();
        let Some(name) = self.name("the lab name") else {
            return TopDecl::Error(self.recover(start));
        };
        if self.expect(TokenKind::LBrace, "`{`").is_none() {
            return TopDecl::Error(self.recover(start));
        }
        let mut simulations = Vec::new();
        loop {
            let token = self.peek();
            match (token.kind, token.text) {
                (TokenKind::RBrace, _) => {
                    self.bump();
                    break;
                }
                (TokenKind::Eof, _) => {
                    self.error_expected("`}`");
                    break;
                }
                (TokenKind::Ident, "simulation") => {
                    if let Some(simulation) = self.simulation_decl() {
                        simulations.push(simulation);
                    }
                }
                (TokenKind::Ident, word @ ("experiment" | "theorem")) => {
                    // Parsed for balance, refused whole: the
                    // experiment runner and the theorem layer are not
                    // in this build.
                    let construct = if word == "experiment" {
                        "an experiment"
                    } else {
                        "a theorem"
                    };
                    let block_start = token.span.range().start;
                    self.skip_balanced_block();
                    self.deferred(construct, self.span_from(block_start));
                }
                _ => {
                    self.error_expected("`simulation`, `experiment`, or `theorem`");
                    self.bump();
                }
            }
        }
        TopDecl::Lab(LabDecl {
            name,
            simulations,
            span: self.span_from(start),
        })
    }

    /// Skips forward through the first `{` to its matching `}` —
    /// how a refused-but-balanced declaration is passed over.
    fn skip_balanced_block(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::LBrace => {
                    self.bump();
                    break;
                }
                _ => {
                    self.bump();
                }
            }
        }
        let mut depth: u32 = 0;
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    self.bump();
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    /// `simulation IDENT on graph_ref { (mock | scenario)* }`.
    fn simulation_decl(&mut self) -> Option<SimulationDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the simulation name")?;
        let on = self.peek();
        if !(on.kind == TokenKind::Ident && on.text == "on") {
            self.error_expected("`on`");
            return None;
        }
        self.bump();
        let target = self.dotted_names("the simulation target")?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut mocks = Vec::new();
        let mut scenarios = Vec::new();
        loop {
            let token = self.peek();
            match (token.kind, token.text) {
                (TokenKind::RBrace, _) => {
                    self.bump();
                    break;
                }
                (TokenKind::Eof, _) => {
                    self.error_expected("`}`");
                    break;
                }
                (TokenKind::Ident, "mock") => {
                    if let Some(mock) = self.mock_decl() {
                        mocks.push(mock);
                    }
                }
                (TokenKind::Ident, "scenario") => {
                    if let Some(scenario) = self.scenario_decl() {
                        scenarios.push(scenario);
                    }
                }
                _ => {
                    self.error_expected("`mock` or `scenario`");
                    self.bump();
                }
            }
        }
        Some(SimulationDecl {
            name,
            target,
            mocks,
            scenarios,
            span: self.span_from(start),
        })
    }

    /// `mock IDENT ("." IDENT)* { handler* }` — the body is ordinary
    /// handler grammar.
    fn mock_decl(&mut self) -> Option<MockDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let path = self.dotted_names("the mocked connector")?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut handlers = Vec::new();
        loop {
            let token = self.peek();
            match (token.kind, token.text) {
                (TokenKind::RBrace, _) => {
                    self.bump();
                    break;
                }
                (TokenKind::Eof, _) => {
                    self.error_expected("`}`");
                    break;
                }
                (TokenKind::Ident, "on") => {
                    let handler_start = token.span.range().start;
                    match self.handler_decl() {
                        Some(handler) => handlers.push(handler),
                        None => {
                            self.recover(handler_start);
                        }
                    }
                }
                _ => {
                    self.error_expected("`on` or `}`");
                    self.bump();
                }
            }
        }
        Some(MockDecl {
            path,
            handlers,
            span: self.span_from(start),
        })
    }

    /// `scenario IDENT STRING? { (injection | advance | expect |
    /// mock | …)* }`.
    fn scenario_decl(&mut self) -> Option<ScenarioDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the scenario name")?;
        let label = self.string_literal();
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut body = Vec::new();
        loop {
            let token = self.peek();
            match (token.kind, token.text) {
                (TokenKind::RBrace, _) => {
                    self.bump();
                    break;
                }
                (TokenKind::Eof, _) => {
                    self.error_expected("`}`");
                    break;
                }
                (TokenKind::Ident, "advance") => {
                    let step_start = token.span.range().start;
                    self.bump();
                    let expr = self.expr(&ATTR_POSITION);
                    body.push(ScenarioItem::Advance {
                        expr,
                        span: self.span_from(step_start),
                    });
                }
                (TokenKind::Ident, "expect") => {
                    if let Some(step) = self.expect_step() {
                        body.push(ScenarioItem::Expect(step));
                    }
                }
                (TokenKind::Ident, "mock") => {
                    if let Some(mock) = self.mock_decl() {
                        body.push(ScenarioItem::Mock(mock));
                    }
                }
                (TokenKind::Ident, word @ ("crash" | "restart" | "origin")) => {
                    // Fault injection and the civil-time anchor join
                    // with the recovery fold; parsed to the end of
                    // the line, refused whole.
                    let construct = match word {
                        "crash" => "a crash step",
                        "restart" => "a restart step",
                        _ => "a scenario origin",
                    };
                    let step_start = token.span.range().start;
                    self.bump();
                    loop {
                        let next = self.peek();
                        if next.newline_before
                            || matches!(next.kind, TokenKind::RBrace | TokenKind::Eof)
                        {
                            break;
                        }
                        self.bump();
                    }
                    self.deferred(construct, self.span_from(step_start));
                }
                (TokenKind::Ident, _) => {
                    let target = self.lvalue()?;
                    self.expect(TokenKind::Eq, "`=`")?;
                    let value = self.value(&ATTR_POSITION);
                    let seq = if self.peek().kind == TokenKind::Ident && self.peek().text == "seq" {
                        self.bump();
                        let int = self.expect(TokenKind::Int, "an inbound sequence")?;
                        Some(IntLit {
                            text: String::from(int.text),
                            span: int.span,
                        })
                    } else {
                        None
                    };
                    body.push(ScenarioItem::Injection { target, value, seq });
                }
                _ => {
                    self.error_expected("a scenario step");
                    self.bump();
                }
            }
        }
        Some(ScenarioDecl {
            name,
            label,
            body,
            span: self.span_from(start),
        })
    }

    /// `expect STRING? nothing? port_ref (count cmp INT)? (where
    /// expr)?`.
    fn expect_step(&mut self) -> Option<ExpectStep> {
        let start = self.peek().span.range().start;
        self.bump();
        let label = self.string_literal();
        let nothing = if self.peek().kind == TokenKind::Ident && self.peek().text == "nothing" {
            self.bump();
            true
        } else {
            false
        };
        let target = self.lvalue()?;
        let count = if self.peek().kind == TokenKind::Ident && self.peek().text == "count" {
            self.bump();
            let op = self.cmp_op()?;
            let int = self.expect(TokenKind::Int, "a count")?;
            Some((
                op,
                IntLit {
                    text: String::from(int.text),
                    span: int.span,
                },
            ))
        } else {
            None
        };
        let predicate = if self.peek().kind == TokenKind::Ident && self.peek().text == "where" {
            self.bump();
            Some(self.expr(&ATTR_POSITION))
        } else {
            None
        };
        Some(ExpectStep {
            label,
            nothing,
            target,
            count,
            predicate,
            span: self.span_from(start),
        })
    }

    /// One comparison operator — single maximal-munch lexemes
    /// (D-2026-08-13-1, review round 6).
    fn cmp_op(&mut self) -> Option<CmpOp> {
        match self.peek().kind {
            TokenKind::EqEq => {
                self.bump();
                Some(CmpOp::Eq)
            }
            TokenKind::BangEq => {
                self.bump();
                Some(CmpOp::Ne)
            }
            TokenKind::LtEq => {
                self.bump();
                Some(CmpOp::Le)
            }
            TokenKind::Lt => {
                self.bump();
                Some(CmpOp::Lt)
            }
            TokenKind::GtEq => {
                self.bump();
                Some(CmpOp::Ge)
            }
            TokenKind::Gt => {
                self.bump();
                Some(CmpOp::Gt)
            }
            _ => {
                self.error_expected("a comparison operator");
                None
            }
        }
    }

    /// An optional string literal, quotes stripped.
    fn string_literal(&mut self) -> Option<String> {
        let token = self.peek();
        if token.kind != TokenKind::Str {
            return None;
        }
        let text = token.text;
        self.bump();
        Some(text[1..text.len().saturating_sub(1)].to_string())
    }

    /// Dotted same-line names: `app`, `sessions.codex`.
    fn dotted_names(&mut self, expected: &str) -> Option<Vec<Name>> {
        let first = self.name(expected)?;
        let mut segments = vec![first];
        loop {
            let dot = self.peek();
            if dot.kind != TokenKind::Dot || dot.newline_before {
                break;
            }
            self.bump();
            segments.push(self.name("a path segment")?);
        }
        Some(segments)
    }

    fn graph_decl(&mut self) -> Option<GraphDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the graph name")?;
        let annotation = self.optional_annotation()?;
        let per = self.optional_per()?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let body = self.graph_body();
        self.expect(TokenKind::RBrace, "`}`");
        Some(GraphDecl {
            name,
            annotation,
            per,
            body,
            span: self.span_from(start),
        })
    }

    fn graph_body(&mut self) -> Vec<GraphItem> {
        let mut items = Vec::new();
        loop {
            let token = self.peek();
            let start = token.span.range().start;
            match token.kind {
                TokenKind::Eof | TokenKind::RBrace => break,
                TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Ident => match token.text {
                    "input" | "output" => items.push(match self.port_decl() {
                        Some(port) => GraphItem::Port(port),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "partition" => items.push(match self.partition_decl() {
                        Some(partition) => GraphItem::Partition(partition),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "operator" => items.push(match self.operator_decl() {
                        Some(operator) => GraphItem::Operator(operator),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "connector" => items.push(match self.connector_decl() {
                        Some(connector) => GraphItem::Connector(connector),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "graph" => items.push(match self.graph_decl() {
                        Some(graph) => GraphItem::Graph(graph),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "val" => items.push(match self.val_decl() {
                        Some(val) => GraphItem::Val(val),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    "defaults" => {
                        self.bump();
                        items.push(match self.struct_body() {
                            Some(body) => GraphItem::Defaults(body),
                            None => GraphItem::Error(self.recover(start)),
                        });
                    }
                    "on" => items.push(match self.handler_decl() {
                        Some(handler) => GraphItem::Handler(handler),
                        None => GraphItem::Error(self.recover(start)),
                    }),
                    _ => items.push(self.graph_attribute_or_edge(start)),
                },
                TokenKind::Dot | TokenKind::LBrace => {
                    items.push(
                        match self
                            .leading_endpoint()
                            .and_then(|first| self.edge_decl(start, first))
                        {
                            Some(edge) => GraphItem::Edge(edge),
                            None => GraphItem::Error(self.recover(start)),
                        },
                    );
                }
                _ => {
                    self.error_expected("a graph body item");
                    self.bump();
                    items.push(GraphItem::Error(self.recover(start)));
                }
            }
        }
        items
    }

    /// An IDENT-led graph item: attribute, or a wiring edge.
    fn graph_attribute_or_edge(&mut self, start: usize) -> GraphItem {
        let Some(target) = self.lvalue() else {
            return GraphItem::Error(self.recover(start));
        };
        let token = self.peek();
        match token.kind {
            TokenKind::Eq => {
                self.bump();
                let value = self.value(&ATTR_POSITION);
                GraphItem::Attribute(Attribute {
                    target,
                    value,
                    span: self.span_from(start),
                })
            }
            TokenKind::Arrow | TokenKind::BiArrow => {
                let span = target.span;
                let first = EdgeEndpoint::Ref(PortRef {
                    boundary: false,
                    segments: target.segments,
                    span,
                });
                match self.edge_decl(start, first) {
                    Some(edge) => GraphItem::Edge(edge),
                    None => GraphItem::Error(self.recover(start)),
                }
            }
            _ => {
                self.error_expected("`=` or a wiring arrow");
                GraphItem::Error(self.recover(start))
            }
        }
    }

    // ----- wiring edges -----

    /// A `.`- or `{`-led first endpoint of an edge.
    fn leading_endpoint(&mut self) -> Option<EdgeEndpoint> {
        match self.peek().kind {
            TokenKind::Dot => Some(EdgeEndpoint::Ref(self.boundary_ref())),
            _ => Some(EdgeEndpoint::Branch(self.branch_refs()?)),
        }
    }

    /// The rest of an edge after its first endpoint: arrows, guard,
    /// and configuration.
    fn edge_decl(&mut self, start: usize, first: EdgeEndpoint) -> Option<EdgeDecl> {
        let mut endpoints = vec![first];
        let bidirectional = self.peek().kind == TokenKind::BiArrow;
        if bidirectional {
            self.bump();
            endpoints.push(self.edge_endpoint()?);
        } else {
            self.expect(TokenKind::Arrow, "`->`")?;
            endpoints.push(self.edge_endpoint()?);
            while self.eat(TokenKind::Arrow).is_some() {
                endpoints.push(self.edge_endpoint()?);
            }
        }
        let guard = if self.at_word("when") {
            self.bump();
            Some(self.expr(&GUARD_POSITION))
        } else {
            None
        };
        let config = if self.peek().kind == TokenKind::LBrace {
            self.struct_body()
        } else {
            None
        };
        Some(EdgeDecl {
            endpoints,
            bidirectional,
            guard,
            config,
            span: self.span_from(start),
        })
    }

    fn edge_endpoint(&mut self) -> Option<EdgeEndpoint> {
        let token = self.peek();
        match token.kind {
            TokenKind::Dot => Some(EdgeEndpoint::Ref(self.boundary_ref())),
            TokenKind::LBrace => Some(EdgeEndpoint::Branch(self.branch_refs()?)),
            TokenKind::Ident if token.text == "nothing" => {
                self.bump();
                Some(EdgeEndpoint::Nothing(token.span))
            }
            TokenKind::Ident => {
                let path = self.lvalue()?;
                Some(EdgeEndpoint::Ref(PortRef {
                    boundary: false,
                    segments: path.segments,
                    span: path.span,
                }))
            }
            _ => {
                self.error_expected("an edge endpoint");
                None
            }
        }
    }

    /// `.` or `.foo` — the current graph's boundary.
    fn boundary_ref(&mut self) -> PortRef {
        let dot = self.bump();
        let start = dot.span.range().start;
        // A boundary name belongs to its dot only on the same line:
        // the token's newline flag is the separation signal, so `b ->
        // .` followed by `a -> …` on the next line never reads as
        // `.a`, while spaces and comments stay ordinary trivia.
        let next = self.peek();
        let same_line = next.kind == TokenKind::Ident && !next.newline_before;
        let segments = if same_line {
            match self.name("a boundary port name") {
                Some(name) => vec![name],
                None => vec![],
            }
        } else {
            vec![]
        };
        PortRef {
            boundary: true,
            segments,
            span: self.span_from(start),
        }
    }

    /// `{ a ; b }` — a fan-out or fan-in branch.
    fn branch_refs(&mut self) -> Option<Vec<PortRef>> {
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut refs = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Dot => refs.push(self.boundary_ref()),
                _ => {
                    let path = self.lvalue()?;
                    refs.push(PortRef {
                        boundary: false,
                        segments: path.segments,
                        span: path.span,
                    });
                }
            }
        }
        self.expect(TokenKind::RBrace, "`}`")?;
        Some(refs)
    }

    fn operator_decl(&mut self) -> Option<OperatorDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the operator name")?;
        let annotation = self.optional_annotation()?;
        let per = self.optional_per()?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let body = self.operator_body();
        self.expect(TokenKind::RBrace, "`}`");
        Some(OperatorDecl {
            name,
            annotation,
            per,
            body,
            span: self.span_from(start),
        })
    }

    fn connector_decl(&mut self) -> Option<ConnectorDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the connector name")?;
        self.expect(TokenKind::Colon, "`:` — a connector declares its type")?;
        let annotation = self.annotation()?;
        let per = self.optional_per()?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let body = self.operator_body();
        self.expect(TokenKind::RBrace, "`}`");
        Some(ConnectorDecl {
            name,
            annotation,
            per,
            body,
            span: self.span_from(start),
        })
    }

    fn operator_body(&mut self) -> Vec<OperatorItem> {
        let mut items = Vec::new();
        loop {
            let token = self.peek();
            let start = token.span.range().start;
            match token.kind {
                TokenKind::Eof | TokenKind::RBrace => break,
                TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Ident => match token.text {
                    "input" | "output" => items.push(match self.port_decl() {
                        Some(port) => OperatorItem::Port(port),
                        None => OperatorItem::Error(self.recover(start)),
                    }),
                    "val" => items.push(match self.val_decl() {
                        Some(val) => OperatorItem::Val(val),
                        None => OperatorItem::Error(self.recover(start)),
                    }),
                    "var" => items.push(match self.var_decl() {
                        Some(var) => OperatorItem::Var(var),
                        None => OperatorItem::Error(self.recover(start)),
                    }),
                    "on" => items.push(match self.handler_decl() {
                        Some(handler) => OperatorItem::Handler(handler),
                        None => OperatorItem::Error(self.recover(start)),
                    }),
                    _ => items.push(self.attribute_or_sub_block(start)),
                },
                _ => {
                    self.error_expected("a node body item");
                    self.bump();
                    items.push(OperatorItem::Error(self.recover(start)));
                }
            }
        }
        items
    }

    /// An IDENT-led node body item: `lvalue = value` or `IDENT { … }`.
    fn attribute_or_sub_block(&mut self, start: usize) -> OperatorItem {
        let Some(target) = self.lvalue() else {
            return OperatorItem::Error(self.recover(start));
        };
        match self.peek().kind {
            TokenKind::Eq => {
                self.bump();
                let value = self.value(&ATTR_POSITION);
                OperatorItem::Attribute(Attribute {
                    target,
                    value,
                    span: self.span_from(start),
                })
            }
            TokenKind::LBrace if target.segments.len() == 1 => {
                let name = target
                    .segments
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| unreachable!("length checked"));
                match self.struct_body() {
                    Some(body) => OperatorItem::SubBlock(SubBlock {
                        name,
                        body,
                        span: self.span_from(start),
                    }),
                    None => OperatorItem::Error(self.recover(start)),
                }
            }
            _ => {
                self.error_expected("`=` or `{`");
                OperatorItem::Error(self.recover(start))
            }
        }
    }

    // ----- handlers -----

    /// `on trigger guard* body?` — a body-less handler is the Loop
    /// base-case declaration; its legality is the checker's judgment.
    fn handler_decl(&mut self) -> Option<HandlerDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let trigger = self.trigger()?;
        let mut guards = Vec::new();
        loop {
            let token = self.peek();
            if token.kind != TokenKind::Ident {
                break;
            }
            match token.text {
                "where" => {
                    self.bump();
                    guards.push(Guard::Where(self.expr(&GUARD_POSITION)));
                }
                "having" => {
                    self.bump();
                    guards.push(Guard::Having(self.expr(&GUARD_POSITION)));
                }
                "by" => {
                    self.bump();
                    guards.push(Guard::By(self.expr(&GUARD_POSITION)));
                }
                "within" => {
                    self.bump();
                    guards.push(Guard::Within(self.expr(&GUARD_POSITION)));
                }
                "last" => {
                    self.bump();
                    let token = self.expect(TokenKind::Int, "a count")?;
                    guards.push(Guard::Last(IntLit {
                        text: String::from(token.text),
                        span: token.span,
                    }));
                }
                _ => break,
            }
        }
        let body = if self.peek().kind == TokenKind::LBrace {
            Some(self.statements()?)
        } else {
            None
        };
        Some(HandlerDecl {
            trigger,
            guards,
            body,
            span: self.span_from(start),
        })
    }

    fn trigger(&mut self) -> Option<Trigger> {
        let token = self.peek();
        match token.kind {
            TokenKind::LParen => {
                self.bump();
                let mut ports = vec![self.name("a port name")?];
                while self.eat(TokenKind::Comma).is_some() {
                    ports.push(self.name("a port name")?);
                }
                self.expect(TokenKind::RParen, "`)`")?;
                Some(Trigger::Join(ports))
            }
            TokenKind::Ident => {
                let first = self.bump();
                if self.eat(TokenKind::Dot).is_some() {
                    let aspect = Name {
                        text: String::from(first.text),
                        span: first.span,
                    };
                    let event = self.name("`start` or `stop`")?;
                    let event = match event.text.as_str() {
                        "start" => AspectEvent::Start,
                        "stop" => AspectEvent::Stop,
                        _ => {
                            self.error_expected("`start` or `stop`");
                            return None;
                        }
                    };
                    return Some(Trigger::Aspect { aspect, event });
                }
                Some(match first.text {
                    "start" => Trigger::Start(first.span),
                    "stop" => Trigger::Stop(first.span),
                    "compensate" => Trigger::Compensate(first.span),
                    _ => Trigger::Port(Name {
                        text: String::from(first.text),
                        span: first.span,
                    }),
                })
            }
            _ => {
                self.error_expected("a handler trigger");
                None
            }
        }
    }

    /// `"{" ( statement )* "}"` — one-token host dispatch. A
    /// recognizable value in this position is parsed only so its own
    /// delimiters remain authoritative, then replaced by an error
    /// hole; other invalid input takes line-bounded token recovery.
    fn statements(&mut self) -> Option<Vec<Statement>> {
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut statements = Vec::new();
        loop {
            match self.lexer.statement_head() {
                StatementHead::Structural => {}
                StatementHead::BareValue => {
                    statements.push(self.bare_value_statement());
                    continue;
                }
                StatementHead::Recovery => {
                    let recovery = self.skip_bare_statement(false);
                    let start = recovery.range.start;
                    statements.push(self.recovered_bare_statement(start, recovery));
                    continue;
                }
            }
            let token = self.peek();
            let start = token.span.range().start;
            match token.kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Ident if token.text == "val" => match self.val_decl() {
                    Some(val) => statements.push(Statement::Val(val)),
                    None => statements.push(Statement::Error(self.recover(start))),
                },
                TokenKind::Ident if token.text == "stop" => {
                    self.bump();
                    let target = match self.peek().kind {
                        TokenKind::Ident => self.name("a node name"),
                        _ => None,
                    };
                    statements.push(Statement::Stop(target));
                }
                // In statement position `(` selects multi-emit. A
                // parenthesized expression value is valid only after `=`;
                // malformed multi-emits recover forward from the point
                // of failure, never by probing or rewinding.
                TokenKind::LParen => match self.multi_emit() {
                    Some(statement) => statements.push(statement),
                    None => statements.push(Statement::Error(self.recover_statement_tail(start))),
                },
                TokenKind::Ident => statements.push(self.assign_statement(start)),
                _ => {
                    // Unreachable by the head classification —
                    // defensive refusal, never a silent value.
                    self.error_expected("a statement");
                    self.bump();
                    statements.push(Statement::Error(self.recover(start)));
                }
            }
        }
        self.expect(TokenKind::RBrace, "`}`")?;
        Some(statements)
    }

    /// An IDENT-led assignment. Every output, including the conventional
    /// `err`, uses this one production; a bare expression refuses (no
    /// implicit yield, review rounds 7–8).
    fn assign_statement(&mut self, start: usize) -> Statement {
        let first = self.bump();
        let mut segments = vec![Name {
            text: String::from(first.text),
            span: first.span,
        }];
        while self.lexer.dot_follows() {
            self.bump();
            let Some(segment) = self.name("a path segment") else {
                return Statement::Error(self.recover(start));
            };
            segments.push(segment);
        }
        let target_span = self.span_from(start);
        // ONE typed lookahead DRIVES the production (review round 8):
        // the probe decides without tokenizing, and the arms consume
        // what the probe proved.
        match self.lexer.assignment_follows() {
            Some(crate::compiler::lexer::AssignmentHead::Plain) => {
                self.bump();
                Statement::Assign {
                    target: LValue {
                        span: target_span,
                        segments,
                    },
                    ty: None,
                    value: self.value(&ATTR_POSITION),
                }
            }
            Some(crate::compiler::lexer::AssignmentHead::Typed) => {
                self.bump();
                let Some(ty) = self.type_ref() else {
                    return Statement::Error(self.recover(start));
                };
                if self.expect(TokenKind::Eq, "`=`").is_none() {
                    return Statement::Error(self.recover(start));
                }
                Statement::Assign {
                    target: LValue {
                        span: target_span,
                        segments,
                    },
                    ty: Some(ty),
                    value: self.value(&ATTR_POSITION),
                }
            }
            None => {
                // A bare expression. A single FORM word is consumed
                // by its real dispatch (the verbatim reader, the
                // json reader, a guest framer, the dead refusals) —
                // then the statement-position voice; anything else
                // takes the classic recovery.
                if segments.len() == 1
                    && (first.text == "nothing"
                        || self.lexer.frontend_bang_follows(first.span.range().end)
                        || (DEAD_INITIALIZER_TAGS.contains(&first.text)
                            && self.lexer.tag_opener_on_line().is_some()))
                {
                    let value = self.ident_tail(first, &ATTR_POSITION);
                    let end = self.last_end.max(start);
                    if matches!(value, Value::Error(_)) {
                        // The form's own voice spoke; the recovery
                        // form carries the statement.
                        return Statement::Error(ErrorNode {
                            span: Span::new(self.source_id, start..end),
                        });
                    }
                    return self.bare_statement_refusal(start..end);
                }
                // The grammar has already rejected this position. If
                // the Rastok expression continuation is structurally complete, let
                // its real region framer consume the whole misplaced
                // value (including balanced multiline calls/lists) so
                // its interior cannot become phantom host statements.
                // If it is incomplete, line recovery preserves the
                // next healthy statement instead.
                if let Some(expr) = self.try_complete_expr_lexeme(Some(start), &ATTR_POSITION) {
                    return self.bare_statement_refusal(start..expr.span.range().end.max(start));
                }
                let recovery = self.skip_bare_statement(true);
                self.recovered_bare_statement(start, recovery)
            }
        }
    }

    /// `( a, b ) = tuple_or_broadcast` — atomic multi-emit.
    fn multi_emit(&mut self) -> Option<Statement> {
        self.bump();
        let mut targets = vec![self.lvalue()?];
        while self.eat(TokenKind::Comma).is_some() {
            targets.push(self.lvalue()?);
        }
        self.expect(TokenKind::RParen, "`)`")?;
        self.expect(TokenKind::Eq, "`=`")?;
        let values = if self.eat(TokenKind::LParen).is_some() {
            let mut values = vec![self.value(&PAREN_POSITION)];
            while self.eat(TokenKind::Comma).is_some() {
                values.push(self.value(&PAREN_POSITION));
            }
            self.expect(TokenKind::RParen, "`)`")?;
            values
        } else {
            vec![self.value(&ATTR_POSITION)]
        };
        Some(Statement::MultiEmit { targets, values })
    }

    fn port_decl(&mut self) -> Option<PortDecl> {
        let token = self.peek();
        let start = token.span.range().start;
        let direction = match token.text {
            "input" => PortDirection::Input,
            _ => PortDirection::Output,
        };
        self.bump();
        let name = self.name("the port name")?;
        let ty = match self.eat(TokenKind::Colon) {
            Some(_) => Some(self.type_ref()?),
            None => None,
        };
        Some(PortDecl {
            direction,
            name,
            ty,
            span: self.span_from(start),
        })
    }

    fn val_decl(&mut self) -> Option<ValDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the val name")?;
        let ty = match self.eat(TokenKind::Colon) {
            Some(_) => Some(self.type_ref()?),
            None => None,
        };
        self.expect(TokenKind::Eq, "`=` — a val is always initialized")?;
        let value = self.value(&ATTR_POSITION);
        Some(ValDecl {
            name,
            ty,
            value,
            span: self.span_from(start),
        })
    }

    fn var_decl(&mut self) -> Option<VarDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the var name")?;
        self.expect(TokenKind::Colon, "`:` — a var declares its type")?;
        let ty = self.type_ref()?;
        let value = self.eat(TokenKind::Eq).map(|_| self.value(&ATTR_POSITION));
        Some(VarDecl {
            name,
            ty,
            value,
            span: self.span_from(start),
        })
    }

    fn partition_decl(&mut self) -> Option<PartitionDecl> {
        let start = self.peek().span.range().start;
        self.bump();
        let name = self.name("the partition name")?;
        self.expect(TokenKind::LBrace, "`{`")?;
        let fields = self.partition_fields(TokenKind::RBrace, &ATTR_POSITION);
        self.expect(TokenKind::RBrace, "`}`");
        Some(PartitionDecl {
            name,
            fields,
            span: self.span_from(start),
        })
    }

    /// `key = …` / `lifetime = …` fields, up to `close`.
    fn partition_fields(
        &mut self,
        close: TokenKind,
        position: &TerminatorSet,
    ) -> Vec<PartitionField> {
        let mut fields = Vec::new();
        loop {
            let token = self.peek();
            if token.kind == close || token.kind == TokenKind::Eof {
                break;
            }
            match token.kind {
                TokenKind::Semi | TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::Ident if token.text == "key" => {
                    self.bump();
                    if self.expect(TokenKind::Eq, "`=`").is_none() {
                        break;
                    }
                    fields.push(PartitionField::Key(self.expr(position)));
                }
                TokenKind::Ident if token.text == "lifetime" => {
                    self.bump();
                    if self.expect(TokenKind::Eq, "`=`").is_none() {
                        break;
                    }
                    fields.push(PartitionField::Lifetime(self.lifetime_value(position)));
                }
                _ => {
                    self.error_expected("`key` or `lifetime`");
                    break;
                }
            }
        }
        fields
    }

    fn lifetime_value(&mut self, position: &TerminatorSet) -> LifetimeValue {
        let token = self.peek();
        match (token.kind, token.text) {
            (TokenKind::Ident, "persistent") => {
                self.bump();
                LifetimeValue::Persistent(token.span)
            }
            (TokenKind::Ident, "single_pass") => {
                self.bump();
                LifetimeValue::SinglePass(token.span)
            }
            (TokenKind::Ident, "fixed") => {
                self.bump();
                LifetimeValue::Fixed(self.expr(position))
            }
            (TokenKind::Ident, "window") => {
                self.bump();
                let duration = self.expr(&WINDOW_POSITION);
                let mode = if self.at_word("sliding") {
                    self.bump();
                    WindowMode::Sliding
                } else if self.at_word("tumbling") {
                    self.bump();
                    WindowMode::Tumbling
                } else {
                    self.error_expected("`tumbling` or `sliding`");
                    WindowMode::Tumbling
                };
                LifetimeValue::Window { duration, mode }
            }
            _ => LifetimeValue::Expr(self.expr(position)),
        }
    }

    fn optional_annotation(&mut self) -> Option<Option<Annotation>> {
        match self.eat(TokenKind::Colon) {
            Some(_) => Some(Some(self.annotation()?)),
            None => Some(None),
        }
    }

    fn annotation(&mut self) -> Option<Annotation> {
        let mut symbols = vec![self.name("a type symbol")?];
        while self.eat(TokenKind::Plus).is_some() {
            symbols.push(self.name("a type symbol")?);
        }
        Some(Annotation { symbols })
    }

    fn optional_per(&mut self) -> Option<Option<PerClause>> {
        if !self.at_word("per") {
            return Some(None);
        }
        self.bump();
        let token = self.peek();
        let clause = match token.kind {
            TokenKind::LParen => {
                self.bump();
                let fields = self.partition_fields(TokenKind::RParen, &PAREN_POSITION);
                self.expect(TokenKind::RParen, "`)`")?;
                PerClause::Inline(fields)
            }
            TokenKind::LBrace => {
                self.bump();
                let fields = self.partition_fields(TokenKind::RBrace, &ATTR_POSITION);
                self.expect(TokenKind::RBrace, "`}`")?;
                PerClause::Inline(fields)
            }
            _ => {
                let mut names = vec![self.name("a partition name")?];
                while self.eat(TokenKind::Comma).is_some() {
                    names.push(self.name("a partition name")?);
                }
                PerClause::Named(names)
            }
        };
        Some(Some(clause))
    }

    fn lvalue(&mut self) -> Option<LValue> {
        let first = self.name("a name")?;
        let start = first.span.range().start;
        let mut segments = vec![first];
        // A continuation dot belongs to its path only on the same
        // line: the newline flag separates `a -> b` from a `. -> c`
        // that follows, while spaces and comments stay trivia. The
        // walk PROBES (review round 7): the follower after a path
        // may be Rastok expression bytes outside the host vocabulary, and only a
        // proven dot is tokenized.
        while self.lexer.dot_follows() {
            self.bump();
            segments.push(self.name("a path segment")?);
        }
        Some(LValue {
            segments,
            span: self.span_from(start),
        })
    }

    // ----- values -----

    /// A value position; `position` chooses the Rastok expression terminators.
    fn value(&mut self, position: &TerminatorSet) -> Value {
        // A newline-terminated position must hold a value ON ITS
        // LINE: an omitted value diagnoses HERE and consumes
        // nothing — it never absorbs the next line's construct
        // (D-2026-08-12-1, review round). `;` and `}` end the
        // statement the same way a newline would.
        if position.newline && self.lexer.value_missing_on_line() {
            {
                // An empty range: the fault is a POINT (possibly
                // EOF), per Span's own contract.
                let at = Span::new(self.source_id, self.last_end..self.last_end);
                self.bag.push(
                    Diagnostic::new(
                        E_PARSE_EXPECTED,
                        Phase::Parse,
                        "expected a value before the end of the line",
                    )
                    .with_span(at)
                    .with_field("expected", "a value")
                    .with_field("found", "end of line"),
                );
                return Value::Error(at);
            }
        }
        self.present_value(position)
    }

    /// A value whose first significant token is already known to be
    /// present. Error productions use this entry after classifying a
    /// bare value at the start of its own line; the ordinary
    /// [`Self::value`]
    /// entry remains responsible for detecting `out =` followed by a
    /// newline.
    fn present_value(&mut self, position: &TerminatorSet) -> Value {
        // Classified by the lexer before touching the token stream:
        // Rastok expression text may open with bytes outside the host vocabulary,
        // so only builder-shaped input is tokenized here.
        if self.lexer.value_head() == ValueHead::Expr {
            return Value::Expr(self.expr(position));
        }
        let token = self.peek();
        match (token.kind, token.text) {
            (TokenKind::LBrace, _) => self.braced_value(token.span),
            (TokenKind::Dot, _) => {
                // `.` — the exact re-emission of the current
                // envelope; grammar forbids leading dots in Rastok expression, so
                // the form is unambiguous here.
                self.bump();
                Value::Identity(token.span)
            }
            (TokenKind::Ident, _) => {
                // The left-factored identifier prefix
                // (D-2026-08-13-1): consumed ONCE, never handed
                // back — every continuation moves forward.
                let ident = self.bump();
                self.ident_tail(ident, position)
            }
            _ => Value::Expr(self.expr(position)),
        }
    }

    /// The consumed identifier's continuation — one raw probe (the
    /// next byte may be host-alien, so no token is peeked until the
    /// form proves host-safe), every branch strictly forward
    /// (D-2026-08-13-1).
    fn ident_tail(&mut self, ident: Token<'_>, position: &TerminatorSet) -> Value {
        let word = ident.text;
        if word == "nothing" {
            return Value::Nothing(ident.span);
        }
        if self.lexer.frontend_bang_follows(ident.span.range().end) {
            return self.frontend_value(ident, position);
        }
        if self.lexer.line_exhausted() && position.newline {
            // Nothing significant follows on the line and the line
            // ends the value: the word stands alone. No scan — a
            // comment crossing the line stays HOST trivia, and an
            // unterminated one is the trivia authority's diagnostic
            // (D-2026-08-13-1).
            return Value::Expr(ExprSource {
                text: String::from(ident.text),
                span: ident.span,
            });
        }
        if let Some(opener) = self.lexer.tag_opener_on_line() {
            // `IDENT {` is the one predictive tagged-constructor shape. The
            // checker decides whether context names a genuine union case;
            // obsolete initializer words retain their migration only when it
            // does not. Parenthesized old initializers remain dead here.
            if opener == crate::compiler::lexer::TagOpener::Brace {
                return match self.struct_body() {
                    Some(body) => Value::Tagged {
                        case: Name {
                            text: String::from(ident.text),
                            span: ident.span,
                        },
                        body,
                    },
                    None => Value::Error(ident.span),
                };
            }
            if is_dead_initializer_tag(word) {
                return self.dead_initializer(&ident);
            }
        }
        // An ordinary word: the expression continuation re-reads the
        // immutable prefix slice. Frontend NAMES have no special status here.
        Value::Expr(self.expr_from(ident.span.range().start, position))
    }

    /// The Rastok expression continuation of an already-consumed prefix: the END
    /// is scanned forward from the cursor, the text is the immutable
    /// `source[prefix_start..end]` slice — a bounded re-read, never
    /// a rewind (D-2026-08-13-1). Any peeked token is DROPPED (its
    /// bytes sit inside the slice).
    fn expr(&mut self, terminators: &TerminatorSet) -> ExprSource {
        self.expr_lexeme(None, terminators)
    }

    fn expr_from(&mut self, prefix_start: usize, position: &TerminatorSet) -> ExprSource {
        self.expr_lexeme(Some(prefix_start), position)
    }

    /// One Rastok expression region lexeme from the lexer — complete, or bounded at
    /// an incomplete extent's safe recovery point. The parser consumes
    /// it as a grammatical unit and never touches bytes; Rastok expression remains
    /// the sole syntax-diagnostic authority for value regions.
    fn expr_lexeme(&mut self, prefix_start: Option<usize>, position: &TerminatorSet) -> ExprSource {
        let token = self.lexer.expr_region(prefix_start, position);
        self.last_end = self.last_end.max(token.span.range().end);
        ExprSource {
            text: String::from(token.text),
            span: token.span,
        }
    }

    fn try_complete_expr_lexeme(
        &mut self,
        prefix_start: Option<usize>,
        position: &TerminatorSet,
    ) -> Option<ExprSource> {
        let token = self
            .lexer
            .try_complete_expr_region(prefix_start, position)?;
        self.last_end = self.last_end.max(token.span.range().end);
        Some(ExprSource {
            text: String::from(token.text),
            span: token.span,
        })
    }

    /// The dead-initializer refusal (D-2026-08-12-1): one migration
    /// diagnostic, the whole spelling swallowed so one fault speaks
    /// once. The caller verified the tag is followed by `(` or `{`.
    fn dead_initializer(&mut self, token: &Token<'_>) -> Value {
        let tag = String::from(token.text);
        self.bag.push(
            Diagnostic::new(
                E_PARSE_EXPECTED,
                Phase::Parse,
                format!(
                    "`{tag} {{ … }}` is a dead tagged-initializer spelling; write a \
                     braced struct (`name = {{ … }}`) — a former discriminant becomes \
                     a field (D-2026-08-12-1)"
                ),
            )
            .with_span(token.span)
            .with_field("expected", "a braced struct value (`name = { … }`)")
            .with_field("found", format!("the dead initializer tag `{tag}`"))
            .with_field("tag", tag),
        );
        // Swallow `(args)` and `{ body }` so one fault speaks once —
        // through the OPAQUE region API: a dead form may carry bytes
        // the host lexer refuses, and the migration must stay the
        // only voice (D-2026-08-12-1, review round).
        self.lexer
            .swallow_opaque_on_line(crate::compiler::lexer::TagOpener::Paren);
        self.lexer
            .swallow_opaque_on_line(crate::compiler::lexer::TagOpener::Brace);
        Value::Error(token.span)
    }

    /// A tagged frontend after the grammar has selected `IDENT!`.  The
    /// selection is name-agnostic; the registry is consulted only now for the
    /// framing contract and reader.
    fn frontend_value(&mut self, token: Token<'_>, position: &TerminatorSet) -> Value {
        let name = Name {
            text: String::from(token.text),
            span: token.span,
        };
        // The adjacency probe proved this exact token; consuming it is a
        // normal one-token LL(1) step, never a rewind.
        if self.expect(TokenKind::Bang, "`!`").is_none() {
            return Value::Error(name.span);
        }
        let mut args = Vec::new();
        let mut parens = false;
        if self.lexer.frontend_args_follow() {
            let _ = self.expect(TokenKind::LParen, "`(`");
            parens = true;
            loop {
                if self.lexer.arguments_close() {
                    break;
                }
                args.push(self.value(&PAREN_POSITION));
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "`)`");
        }
        let Some(reader) = self
            .registry
            .frontend(&name.text)
            .map(|contract| contract.reader)
        else {
            self.bag.push(
                Diagnostic::new(
                    crate::compiler::check::E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!("`{}` names no registered tagged frontend", name.text),
                )
                .with_span(name.span)
                .with_field("name", name.text.clone()),
            );
            // No contract means no truthful framer. Recovery is bounded by the
            // host value position and is never used for a valid frontend.
            let skipped = self.expr(position);
            self.last_end = self.last_end.max(skipped.span.range().end);
            return Value::Error(name.span);
        };

        let payload = match reader.framing() {
            crate::contract::FrontendFraming::JsonValue => {
                match self.lexer.json_region(position, self.bag) {
                    crate::compiler::lexer::JsonRegion::Value { value, range } => {
                        self.last_end = range.end;
                        FrontendPayload::Json {
                            value,
                            span: Span::new(self.source_id, range),
                        }
                    }
                    crate::compiler::lexer::JsonRegion::Refused {
                        opener,
                        anchor,
                        message,
                        recovered_to,
                    } => {
                        self.bag.push(
                            Diagnostic::new(
                                E_JSON_VALUE,
                                Phase::Parse,
                                format!("the JSON value does not read: {message}"),
                            )
                            .with_span(anchor)
                            .with_field("detail", message),
                        );
                        self.last_end = recovered_to;
                        return Value::Error(opener);
                    }
                    crate::compiler::lexer::JsonRegion::HostError { at, recovered_to } => {
                        self.last_end = recovered_to;
                        return Value::Error(at);
                    }
                }
            }
            crate::contract::FrontendFraming::VerbatimBraced => {
                let diagnostic_count = self.bag.diagnostics().len();
                let Some(raw) = self.lexer.verbatim_region(self.bag) else {
                    if self.bag.diagnostics().len() == diagnostic_count {
                        self.error_expected("a braced frontend payload");
                    }
                    return Value::Error(name.span);
                };
                self.last_end = raw.span.range().end;
                FrontendPayload::Raw {
                    text: String::from(raw.text),
                    span: raw.span,
                }
            }
            crate::contract::FrontendFraming::QuotedBraced => {
                let diagnostic_count = self.bag.diagnostics().len();
                let Some(raw) = self.lexer.guest_region(self.bag) else {
                    if self.bag.diagnostics().len() == diagnostic_count {
                        self.error_expected("a braced frontend payload");
                    }
                    return Value::Error(name.span);
                };
                self.last_end = raw.span.range().end;
                FrontendPayload::Raw {
                    text: String::from(raw.text),
                    span: raw.span,
                }
            }
        };
        Value::Frontend(FrontendValue {
            name,
            args,
            parens,
            payload,
        })
    }

    /// One brace, punctuation decides (D-2026-08-10-6): `}` is the
    /// empty struct, `:` alone the empty map, a STRING key opens map
    /// entries, an IDENT opens struct entries. Mixing refuses.
    fn braced_value(&mut self, open: Span) -> Value {
        let _ = open;
        // The opening brace is still unconsumed (`struct_body`
        // expects it); peek PAST it to dispatch.
        self.bump();
        match self.peek().kind {
            TokenKind::Colon => {
                // `{:}` — the empty map.
                self.bump();
                self.expect(TokenKind::RBrace, "`}`");
                Value::BracedMap(Vec::new())
            }
            TokenKind::Str => {
                let mut entries = Vec::new();
                loop {
                    let token = self.peek();
                    match token.kind {
                        TokenKind::RBrace | TokenKind::Eof => break,
                        TokenKind::Comma | TokenKind::Semi => {
                            self.bump();
                        }
                        TokenKind::Str => {
                            let key = ExprSource {
                                text: String::from(token.text),
                                span: token.span,
                            };
                            let span = token.span;
                            self.bump();
                            if self.expect(TokenKind::Colon, "`:` (a map entry)").is_none() {
                                break;
                            }
                            entries.push(crate::compiler::syntax::MapEntry {
                                key,
                                span,
                                value: self.value(&STRUCT_POSITION),
                            });
                        }
                        TokenKind::Ident => {
                            self.error_expected(
                                "a quoted map key — `=` entries build a struct, `:` entries a map; the two never mix",
                            );
                            break;
                        }
                        _ => {
                            self.error_expected("a map entry or `}`");
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBrace, "`}`");
                Value::BracedMap(entries)
            }
            _ => {
                // Struct entries (or `{}` — the empty struct). A
                // stray `"k": v` inside refuses in the entry loop.
                match self.struct_entries() {
                    Some(body) => Value::Braced(body),
                    // The entry loop already spoke.
                    None => Value::Error(self.peek().span),
                }
            }
        }
    }

    /// `"{" ( entry ","? )* "}"` — shared by struct-shaped bodies.
    fn struct_body(&mut self) -> Option<StructBody> {
        self.expect(TokenKind::LBrace, "`{`")?;
        self.struct_entries()
    }

    /// The entry loop, opening brace already consumed.
    fn struct_entries(&mut self) -> Option<StructBody> {
        let mut entries = Vec::new();
        loop {
            let token = self.peek();
            match token.kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Comma | TokenKind::Semi => {
                    self.bump();
                }
                TokenKind::Ident => {
                    let Some(key) = self.lvalue() else { break };
                    match self.peek().kind {
                        TokenKind::Eq => {
                            self.bump();
                            entries.push(StructEntry::Field {
                                key,
                                value: self.value(&STRUCT_POSITION),
                            });
                        }
                        TokenKind::LBrace if key.segments.len() == 1 => {
                            let name = key
                                .segments
                                .into_iter()
                                .next()
                                .unwrap_or_else(|| unreachable!("length checked"));
                            let Some(body) = self.struct_body() else {
                                break;
                            };
                            entries.push(StructEntry::Block { name, body });
                        }
                        _ => {
                            self.error_expected("`=` or `{`");
                            break;
                        }
                    }
                }
                TokenKind::Str => {
                    self.error_expected(
                        "a field name — `=` entries build a struct, `:` entries a map; the two never mix",
                    );
                    break;
                }
                _ => {
                    self.error_expected("a field or `}`");
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace, "`}`")?;
        Some(StructBody { entries })
    }

    // ----- types -----

    fn type_ref(&mut self) -> Option<TypeRef> {
        let start = self.peek().span.range().start;
        let mut alternatives = vec![self.base_type()?];
        while self.eat(TokenKind::Pipe).is_some() {
            alternatives.push(self.base_type()?);
        }
        let optional = self.eat(TokenKind::Question).is_some();
        Some(TypeRef {
            alternatives,
            optional,
            span: self.span_from(start),
        })
    }

    fn base_type(&mut self) -> Option<BaseType> {
        let token = self.peek();
        if token.kind != TokenKind::Ident {
            self.error_expected("a type");
            return None;
        }
        if let Some(scalar) = simple_scalar(token.text) {
            self.bump();
            return Some(BaseType::Scalar(scalar));
        }
        match token.text {
            "decimal" => {
                self.bump();
                self.expect(TokenKind::LParen, "`(`")?;
                let precision = self.int_literal("the precision")?;
                self.expect(TokenKind::Comma, "`,`")?;
                let scale = self.int_literal("the scale")?;
                self.expect(TokenKind::RParen, "`)`")?;
                Some(BaseType::Scalar(ScalarType::Decimal { precision, scale }))
            }
            "variant" => {
                self.bump();
                Some(BaseType::Variant)
            }
            "struct" => {
                self.bump();
                self.expect(TokenKind::LBrace, "`{`")?;
                let mut fields = Vec::new();
                let mut open = false;
                loop {
                    let token = self.peek();
                    match token.kind {
                        TokenKind::RBrace | TokenKind::Eof => break,
                        TokenKind::Comma => {
                            self.bump();
                        }
                        TokenKind::DotDot => {
                            self.bump();
                            open = true;
                        }
                        _ => {
                            let name = self.name("a field name")?;
                            self.expect(TokenKind::Colon, "`:`")?;
                            let ty = self.type_ref()?;
                            fields.push(FieldDecl { name, ty });
                        }
                    }
                }
                self.expect(TokenKind::RBrace, "`}`")?;
                Some(BaseType::Struct { fields, open })
            }
            "map" => {
                self.bump();
                self.expect(TokenKind::Lt, "`<`")?;
                let keys = self.type_ref()?;
                self.expect(TokenKind::Comma, "`,`")?;
                let values = self.type_ref()?;
                self.expect(TokenKind::Gt, "`>`")?;
                Some(BaseType::Map(Box::new(keys), Box::new(values)))
            }
            "array" | "table" => {
                let table = token.text == "table";
                self.bump();
                self.expect(TokenKind::Lt, "`<`")?;
                let inner = Box::new(self.type_ref()?);
                self.expect(TokenKind::Gt, "`>`")?;
                Some(if table {
                    BaseType::Table(inner)
                } else {
                    BaseType::Array(inner)
                })
            }
            "enum" => {
                self.bump();
                self.expect(TokenKind::LBrace, "`{`")?;
                let mut names = vec![self.name("an enum member")?];
                while self.eat(TokenKind::Comma).is_some() {
                    names.push(self.name("an enum member")?);
                }
                self.expect(TokenKind::RBrace, "`}`")?;
                Some(BaseType::Enum(names))
            }
            "union" => {
                self.bump();
                self.expect(TokenKind::LBrace, "`{`")?;
                let mut cases = Vec::new();
                loop {
                    match self.peek().kind {
                        TokenKind::RBrace | TokenKind::Eof => break,
                        TokenKind::Comma => {
                            self.bump();
                        }
                        _ => {
                            let name = self.name("a tagged-union case")?;
                            let mut fields = Vec::new();
                            if self.eat(TokenKind::LBrace).is_some() {
                                loop {
                                    match self.peek().kind {
                                        TokenKind::RBrace | TokenKind::Eof => break,
                                        TokenKind::Comma => {
                                            self.bump();
                                        }
                                        _ => {
                                            let field = self.name("a case payload field")?;
                                            self.expect(TokenKind::Colon, "`:`")?;
                                            fields.push(crate::compiler::syntax::FieldDecl {
                                                name: field,
                                                ty: self.type_ref()?,
                                            });
                                        }
                                    }
                                }
                                self.expect(TokenKind::RBrace, "`}`")?;
                            }
                            cases.push(TaggedCaseDecl { name, fields });
                        }
                    }
                }
                self.expect(TokenKind::RBrace, "`}`")?;
                Some(BaseType::TaggedUnion(cases))
            }
            _ => {
                self.bump();
                Some(BaseType::Named(Name {
                    text: String::from(token.text),
                    span: token.span,
                }))
            }
        }
    }

    fn int_literal(&mut self, expected: &str) -> Option<u32> {
        let token = self.expect(TokenKind::Int, expected)?;
        match token.text.parse() {
            Ok(value) => Some(value),
            Err(_) => {
                self.bag.push(
                    Diagnostic::new(
                        E_PARSE_EXPECTED,
                        Phase::Parse,
                        format!("expected a small positive integer, found `{}`", token.text),
                    )
                    .with_span(token.span)
                    .with_field("expected", "a small positive integer")
                    .with_field("found", token.text),
                );
                None
            }
        }
    }
}

/// Whether `word` spells ANY builtin type — scalar or composite.
/// The ONE authority the type-name guard asks (review round 4): a
/// vocabulary extension lands here, next to the grammar that reads
/// it, so a user `type` can never silently shadow a new builtin.
#[must_use]
pub fn is_builtin_type_name(word: &str) -> bool {
    simple_scalar(word).is_some()
        || matches!(
            word,
            "variant" | "struct" | "map" | "array" | "table" | "enum" | "union" | "decimal"
        )
}

fn simple_scalar(word: &str) -> Option<ScalarType> {
    Some(match word {
        "bool" => ScalarType::Bool,
        "int" => ScalarType::Int,
        "long" => ScalarType::Long,
        "float" => ScalarType::Float,
        "double" => ScalarType::Double,
        "number" => ScalarType::Number,
        "bytes" => ScalarType::Bytes,
        "text" => ScalarType::Text,
        "null" => ScalarType::Null,
        "date" => ScalarType::Date,
        "timestamp" => ScalarType::Timestamp,
        "timestamp_ntz" => ScalarType::TimestampNtz,
        "time" => ScalarType::Time,
        "uuid" => ScalarType::Uuid,
        "duration" => ScalarType::Duration,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source_id: SourceId, source: &str, bag: &mut Bag) -> SyntaxTree {
        let registry =
            crate::contract::Registry::try_m0().expect("the standard frontend catalogue is valid");
        super::parse(source_id, source, &registry, bag)
    }

    #[test]
    fn json_recovery_keeps_the_sibling_in_the_tree() {
        // The FAULTY source itself: the first emission is the
        // recovery form, the SECOND statement survives in the AST —
        // not merely a healed twin (D-2026-08-12-1, review round).
        let mut bag = Bag::new();
        let tree = parse(
            SourceId::new(0),
            "graph g : App {\n  operator o {\n    on in { out = json! {\"a\": ???}; out = data }\n  }\n  . -> o -> .\n}\n",
            &mut bag,
        );
        assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("an operator: {:?}", graph.body[0]);
        };
        let OperatorItem::Handler(handler) = &operator.body[0] else {
            panic!("a handler");
        };
        let statements = handler.body.as_ref().expect("a body");
        assert_eq!(statements.len(), 2, "both statements live");
        let Statement::Assign { value, .. } = &statements[0] else {
            panic!("first statement: {:?}", statements[0]);
        };
        assert!(matches!(value, Value::Error(_)), "the recovery form");
        let Statement::Assign { value, .. } = &statements[1] else {
            panic!("second statement: {:?}", statements[1]);
        };
        assert!(matches!(value, Value::Expr(expr) if expr.text == "data"));
    }

    /// The first emitted value of a one-operator graph, with its
    /// parse bag — the §6/§7 dispatch probes read the AST directly.
    fn first_value(value: &str) -> (Value, Bag) {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {value} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), &source, &mut bag);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("an operator: {:?}", graph.body[0]);
        };
        let OperatorItem::Handler(handler) = &operator.body[0] else {
            panic!("a handler");
        };
        let statements = handler.body.as_ref().expect("a body");
        let Statement::Assign { value, .. } = &statements[0] else {
            panic!("an assignment: {:?}", statements[0]);
        };
        (value.clone(), bag)
    }

    #[test]
    fn the_identifier_dispatch_matrix_holds() {
        // B4: IDENT followed immediately by `!` is the sole frontend
        // production. The NAME is irrelevant to the grammar; the registry
        // judges it only after the shape has selected the production.
        for (label, value, want) in [
            ("word alone", "data", "expr"),
            ("word + operator", "data + 1", "expr"),
            ("word + call parens", "size(data)", "expr"),
            ("word + brace", "data { }", "tagged"),
            ("duration expression", "duration(\"5s\")", "expr"),
            ("asset expression", "asset(\"README.md\")", "expr"),
            ("decimal expression", "decimal(\"1.50\")", "expr"),
            ("dead tag + parens", "exponential(1, 2)", "error"),
            // The parser cannot know whether a case is genuine. The checker
            // resolves it against a destination type, or keeps the old
            // migration diagnostic when no such case exists.
            ("former tag + brace", "measure { x = 1 }", "tagged"),
            ("json object", "json! {\"a\": 1}", "frontend"),
            ("json array", "json! [1]", "frontend"),
            ("json scalar", "json! 1", "frontend"),
            ("text", "text! {hello}", "frontend"),
            ("base64", "base64! {AAEC}", "frontend"),
            ("registered guest", "jq! {.x}", "frontend"),
            ("reserved format", "avro! {x}", "frontend"),
            // Frontend names remain ordinary expression identifiers without
            // the adjacent bang. Maximal munch keeps `!=` an operator.
            ("json alone", "json", "expr"),
            ("json inequality", "json!=data", "expr"),
            ("spaced bang", "json ! 1", "expr"),
            ("unknown frontend", "not_registered! {x}", "error"),
        ] {
            let (value, bag) = first_value(value);
            let got = match &value {
                Value::Expr(_) => "expr",
                Value::Frontend(_) => "frontend",
                Value::Tagged { .. } => "tagged",
                Value::Error(_) => "error",
                other => panic!("{label}: unexpected family {other:?}"),
            };
            assert_eq!(got, want, "{label}: {value:?}");
            if want == "error" {
                assert_eq!(bag.diagnostics().len(), 1, "{label}: one voice — {bag:?}");
            } else {
                assert!(bag.diagnostics().is_empty(), "{label}: {bag:?}");
            }
        }
    }

    #[test]
    fn the_bang_is_syntax_not_a_reserved_name() {
        let (ordinary, bag) = first_value("json");
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
        assert!(matches!(ordinary, Value::Expr(expr) if expr.text == "json"));

        let (spaced, bag) = first_value("json ! 1");
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
        assert!(matches!(spaced, Value::Expr(expr) if expr.text == "json ! 1"));

        let (frontend, bag) = first_value("json! 1");
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
        assert!(matches!(frontend, Value::Frontend(_)));

        let (inequality, bag) = first_value("json!=data");
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
        assert!(matches!(inequality, Value::Expr(expr) if expr.text == "json!=data"));
    }

    #[test]
    fn an_unterminated_comment_is_the_trivia_authoritys() {
        // D-2026-08-13-1 §6: `json /* never closed` — the probe finds
        // no body, the value is the bare word, and the unterminated
        // comment is diagnosed by the ONE trivia authority, never by
        // a JSON or Rastok expression reader.
        let source = "graph g : App {\n  operator o {\n    on in { out = json /* never closed\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let _ = parse(SourceId::new(0), source, &mut bag);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code() == crate::compiler::lexer::E_LEX_UNTERMINATED_COMMENT),
            "the trivia authority speaks: {bag:?}"
        );
    }

    #[test]
    fn an_unterminated_comment_in_an_expression_value_returns_to_trivia() {
        // The Rastok expression extent stops BEFORE an unterminated comment. Parsing
        // therefore reaches the one trivia authority and reports the
        // cause before the inevitable missing-closer cascades; CHECK is
        // not required to make the primary fault visible.
        let registry = crate::contract::Registry::try_m0().expect("the standard catalogue holds");
        for (label, source) in [
            (
                "following source is commented out",
                "graph g : App {\n  operator o {\n    on in {\n      out = 1 /* never\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
            ),
            (
                "physical end of file",
                "graph g : App {\n  operator o {\n    on in {\n      out = 1 /* never",
            ),
        ] {
            let rejected = crate::compiler::compile(&[source], &registry)
                .expect_err("an unterminated comment refuses the compilation");
            assert_eq!(
                rejected.diagnostics()[0].code(),
                crate::compiler::lexer::E_LEX_UNTERMINATED_COMMENT,
                "{label}: the primary cause speaks first — {rejected:?}"
            );
            assert!(
                rejected
                    .diagnostics()
                    .iter()
                    .all(|diagnostic| diagnostic.code() != crate::compiler::check::E_EXPR),
                "{label}: Rastok expression never duplicates the trivia authority — {rejected:?}"
            );
        }

        let closed = "graph g : App {\n  operator o {\n    on in {\n      out = 1 /* closed */\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
        crate::compiler::compile(&[closed], &registry)
            .expect("a closed trailing comment remains ordinary trivia");
    }

    #[test]
    fn a_refused_bare_expression_keeps_the_boundary_token() {
        // Review round 7: the implicit yield is REMOVED — a bare
        // expression refuses with ONE voice carrying the migration
        // hint. Its complete Rastok expression continuation is framed as one region,
        // and the boundary (`}`, `;`, the next line) still belongs to
        // the HOST: closers and siblings survive.
        let expressions = ["data", "data.member", "data + 1", "size(data)"];
        type Shape = fn(&str) -> String;
        let contexts: [(&str, Shape, usize); 4] = [
            ("before `}`", |e: &str| format!("on in {{ {e} }}"), 1),
            (
                "before `;` + sibling",
                |e: &str| format!("on in {{ {e}; out = 1 }}"),
                2,
            ),
            (
                "before newline + sibling",
                |e: &str| format!("on in {{\n      {e}\n      out = 1\n    }}"),
                2,
            ),
            (
                "before newline + `}`",
                |e: &str| format!("on in {{\n      {e}\n    }}"),
                1,
            ),
        ];
        for expression in expressions {
            for (label, shape, statements_wanted) in &contexts {
                let source = format!(
                    "graph g : App {{\n  operator o {{\n    {}\n  }}\n  . -> o -> .\n}}\n",
                    shape(expression)
                );
                let mut bag = Bag::new();
                let tree = parse(SourceId::new(0), &source, &mut bag);
                assert_eq!(
                    bag.diagnostics().len(),
                    1,
                    "{expression} {label}: one voice — {bag:?}"
                );
                assert!(
                    bag.diagnostics()[0].message().contains("out = <value>"),
                    "{expression} {label}: the migration hint — {bag:?}"
                );
                let TopDecl::Graph(graph) = &tree.decls[0] else {
                    panic!("a graph");
                };
                let GraphItem::Operator(operator) = &graph.body[0] else {
                    panic!("{expression} {label}: an operator: {:?}", graph.body[0]);
                };
                let OperatorItem::Handler(handler) = &operator.body[0] else {
                    panic!("{expression} {label}: a handler");
                };
                let statements = handler.body.as_ref().expect("a body");
                assert_eq!(
                    statements.len(),
                    *statements_wanted,
                    "{expression} {label}: {statements:?}"
                );
                assert!(
                    matches!(&statements[0], Statement::Error(_)),
                    "{expression} {label}: the refusal form: {:?}",
                    statements[0]
                );
                if *statements_wanted == 2 {
                    let Statement::Assign { value, .. } = &statements[1] else {
                        panic!("{expression} {label}: the sibling: {:?}", statements[1]);
                    };
                    assert!(matches!(value, Value::Expr(expr) if expr.text == "1"));
                }
            }
        }
    }

    /// The handler statements of a one-operator graph, with the
    /// parse bag — the recovery proofs read the AST directly.
    fn handler_statements(source: &str) -> (Vec<Statement>, Bag) {
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), source, &mut bag);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("an operator: {:?}", graph.body[0]);
        };
        let OperatorItem::Handler(handler) = &operator.body[0] else {
            panic!("a handler");
        };
        (handler.body.clone().expect("a body"), bag)
    }

    #[test]
    fn a_dead_struct_body_keeps_the_sibling_and_the_closers() {
        // `IDENT {` is now the predictive tagged-constructor production. A
        // former initializer without arguments therefore reaches the checker
        // intact; its parenthesized predecessor remains a parse migration.
        for (label, source, diagnostics, tagged) in [
            (
                "no args",
                "graph g : App {\n  operator o {\n    on in {\n      out = measure { x = 1 }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
                0,
                true,
            ),
            (
                "with args",
                "graph g : App {\n  operator o {\n    on in {\n      out = on_crash(\"x\") { y = 2 }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
                1,
                false,
            ),
        ] {
            let (statements, bag) = handler_statements(source);
            assert_eq!(bag.diagnostics().len(), diagnostics, "{label}: {bag:?}");
            if diagnostics == 1 {
                assert_eq!(bag.diagnostics()[0].code(), E_PARSE_EXPECTED, "{label}");
            }
            assert_eq!(statements.len(), 2, "{label}: the sibling lives");
            let Statement::Assign { value, .. } = &statements[0] else {
                panic!("{label}: first statement: {:?}", statements[0]);
            };
            assert_eq!(matches!(value, Value::Tagged { .. }), tagged, "{label}");
            assert_eq!(matches!(value, Value::Error(_)), !tagged, "{label}");
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: second statement: {:?}", statements[1]);
            };
            assert!(
                matches!(value, Value::Expr(expr) if expr.text == "data"),
                "{label}"
            );
        }
    }

    #[test]
    fn a_bad_json_form_stays_inside_its_position() {
        // The frontend reader owns the malformed document but never the
        // following host statement or its delimiters.
        for (label, source) in [
            (
                "object",
                "graph g : App {\n  operator o {\n    on in { out = json! {\"a\": ???}; out = data }\n  }\n  . -> o -> .\n}\n",
            ),
            (
                "array",
                "graph g : App {\n  operator o {\n    on in { out = json! [1, ???]; out = data }\n  }\n  . -> o -> .\n}\n",
            ),
        ] {
            let (statements, bag) = handler_statements(source);
            assert_eq!(bag.diagnostics().len(), 1, "{label}: one voice — {bag:?}");
            assert_eq!(bag.diagnostics()[0].code(), E_JSON_VALUE, "{label}");
            assert_eq!(statements.len(), 2, "{label}: {statements:?}");
            let Statement::Assign { value, .. } = &statements[0] else {
                panic!("{label}: first statement: {:?}", statements[0]);
            };
            assert!(matches!(value, Value::Error(_)), "{label}: {value:?}");
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: second statement: {:?}", statements[1]);
            };
            assert!(
                matches!(value, Value::Expr(expr) if expr.text == "data"),
                "{label}"
            );
        }
    }

    #[test]
    fn a_comment_never_leaks_a_boundary_into_a_recovery() {
        // Review round 5's proofs: comments are trivia to EVERY
        // recovery — a `;`/`}`/`)` inside one can never bound or
        // close a region, whatever the faulty form. One voice,
        // sibling kept, closers kept, span on the initial fault.
        for (label, source, code) in [
            (
                "bad json form + block comment hiding `;` and `}`",
                "graph g : App {\n  operator o {\n    on in {\n      out = json! foo /* ; } */\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
                E_JSON_VALUE,
            ),
            (
                "bad json form + line comment hiding `;` and `}`",
                "graph g : App {\n  operator o {\n    on in {\n      out = json! foo // ; }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
                E_JSON_VALUE,
            ),
            (
                "dead initializer + comment before its opener",
                "graph g : App {\n  operator o {\n    on in {\n      out = on_crash /* ) */ (foo)\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
                E_PARSE_EXPECTED,
            ),
        ] {
            let (statements, bag) = handler_statements(source);
            assert_eq!(bag.diagnostics().len(), 1, "{label}: one voice — {bag:?}");
            assert_eq!(bag.diagnostics()[0].code(), code, "{label}");
            let fault = source.find("out = ").expect("the faulty line") + "out = ".len();
            assert!(
                bag.diagnostics()[0].spans()[0].range().start >= fault,
                "{label}: the span sits on the fault"
            );
            assert_eq!(
                statements.len(),
                2,
                "{label}: the sibling lives — {statements:?}"
            );
            let Statement::Assign { value, .. } = &statements[0] else {
                panic!("{label}: first statement: {:?}", statements[0]);
            };
            assert!(
                matches!(value, Value::Error(_)),
                "{label}: the recovery form"
            );
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: second statement: {:?}", statements[1]);
            };
            assert!(
                matches!(value, Value::Expr(expr) if expr.text == "data"),
                "{label}"
            );
        }
    }

    #[test]
    fn a_complete_bare_expression_is_one_refused_value_region() {
        // The implicit yield is REMOVED. Once `=`/`:` have failed, a
        // structurally complete continuation is framed as ONE Rastok expression
        // value — with and without spaces, every operator row is ONE
        // refusal voice with the migration hint; no lexical fault, no
        // phantom assignment.
        for op in ["==", "!=", "<=", ">=", "&&", "||", "*", "/", "%", "-"] {
            for spelled in [format!("data {op} data"), format!("data{op}data")] {
                let source = format!(
                    "graph g : App {{\n  operator o {{\n    on in {{\n      {spelled}\n    }}\n  }}\n  . -> o -> .\n}}\n"
                );
                let mut bag = Bag::new();
                let tree = parse(SourceId::new(0), &source, &mut bag);
                assert_eq!(bag.diagnostics().len(), 1, "{spelled}: {bag:?}");
                assert!(
                    bag.diagnostics()[0].message().contains("out = <value>"),
                    "{spelled}: the migration hint — {bag:?}"
                );
                let TopDecl::Graph(graph) = &tree.decls[0] else {
                    panic!("a graph");
                };
                let GraphItem::Operator(operator) = &graph.body[0] else {
                    panic!("{spelled}: an operator: {:?}", graph.body[0]);
                };
                let OperatorItem::Handler(handler) = &operator.body[0] else {
                    panic!("{spelled}: a handler");
                };
                let statements = handler.body.as_ref().expect("a body");
                assert_eq!(statements.len(), 1, "{spelled}: {statements:?}");
                assert!(
                    matches!(&statements[0], Statement::Error(_)),
                    "{spelled}: the refusal form: {:?}",
                    statements[0]
                );
            }
        }
    }

    #[test]
    fn a_verbatim_body_owns_its_quotes_and_a_dead_form_its_comments() {
        // Round 6's remaining frontier proofs, committed: `text`
        // content is verbatim (apostrophes are data), and a closer
        // inside a comment never ends an opaque dead region.
        let text = "graph g : App {\n  operator o {\n    on in { out = text! {\n  don't panic\n} }\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), text, &mut bag);
        assert!(bag.diagnostics().is_empty(), "verbatim: {bag:?}");
        let _ = tree;
        let dead = "graph g : App {\n  operator o {\n    on in {\n      out = on_crash(foo /* ) */ bar)\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
        let (statements, bag) = handler_statements(dead);
        assert_eq!(bag.diagnostics().len(), 1, "one migration voice: {bag:?}");
        assert_eq!(statements.len(), 2, "the sibling lives: {statements:?}");
        let Statement::Assign { value, .. } = &statements[1] else {
            panic!("second statement: {:?}", statements[1]);
        };
        assert!(matches!(value, Value::Expr(expr) if expr.text == "data"));
    }

    #[test]
    fn every_bare_form_refuses_with_the_one_hint() {
        // Review round 7: with the implicit yield removed, EVERY
        // value form refuses identically as a bare statement — one
        // voice, the migration hint, the sibling alive. The same
        // forms behind `out =` stay whole programs (their own
        // tests).
        for (label, bare) in [
            ("expr", "data + 1"),
            ("json document", "json! [1]"),
            ("verbatim text", "text! { don't panic }"),
            ("base64", "base64! { AAEC }"),
            ("guest", "jq! { .x }"),
            ("nothing", "nothing"),
        ] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{\n      {bare}\n      out = data\n    }}\n  }}\n  . -> o -> .\n}}\n"
            );
            let (statements, bag) = handler_statements(&source);
            assert_eq!(bag.diagnostics().len(), 1, "{label}: one voice — {bag:?}");
            assert!(
                bag.diagnostics()[0].message().contains("out = <value>"),
                "{label}: the hint — {bag:?}"
            );
            assert_eq!(statements.len(), 2, "{label}: {statements:?}");
            assert!(
                matches!(&statements[0], Statement::Error(_)),
                "{label}: the refusal form"
            );
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: the sibling: {:?}", statements[1]);
            };
            assert!(
                matches!(value, Value::Expr(expr) if expr.text == "data"),
                "{label}"
            );
        }
    }

    #[test]
    fn the_balanced_text_block_counts_every_brace() {
        // The text contract, stated normatively (review round 7):
        // the block is BALANCED TEXT — braces count wherever they
        // sit, quotes included, because verbatim text has no string
        // lexicon. Balanced twins compile; a quoted lone closer ends
        // the block early (the named limitation: hostile text
        // belongs to a Rastok expression string).
        let balanced = "graph g : App {\n  operator o {\n    on in { out = text! { a \"{x}\" b } }\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let _ = parse(SourceId::new(0), balanced, &mut bag);
        assert!(bag.diagnostics().is_empty(), "balanced: {bag:?}");
        // The balanced twin also keeps its sibling whole.
        let with_sibling = "graph g : App {\n  operator o {\n    on in {\n      out = text! { a \"{x}\" b }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
        let (statements, bag) = handler_statements(with_sibling);
        assert!(bag.diagnostics().is_empty(), "balanced + sibling: {bag:?}");
        assert_eq!(statements.len(), 2, "{statements:?}");
        // A quoted LONE closer ends the block early — the documented
        // limitation. The FIRST voice is pinned: the host meets the
        // remainder (`\" b }`) as an unterminated string anchored
        // AFTER the early close; the cascade that follows belongs to
        // the mis-framed remainder, not to the framer.
        let lone_closer = "graph g : App {\n  operator o {\n    on in { out = text! { a \"}\" b } }\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let _ = parse(SourceId::new(0), lone_closer, &mut bag);
        assert!(!bag.diagnostics().is_empty(), "the limitation refuses");
        let first = &bag.diagnostics()[0];
        let early_close = lone_closer.find("\"}\"").unwrap() + 1;
        assert!(
            first.spans()[0].range().start > early_close,
            "the first voice is anchored past the early close: {first:?}"
        );
    }

    #[test]
    fn every_recovery_terminates_and_keeps_the_structure() {
        // Review round 8's blockers: a lone closer looped the
        // compiler forever, and the recovery scanner absorbed
        // siblings and closers. Every row must TERMINATE (the test
        // completing is the proof), refuse with voices that never
        // include a cascade over the sibling, and keep the sibling
        // and the handler's closer in the tree.
        for (label, garbage) in [
            ("lone close paren", ")"),
            ("lone close bracket", "]"),
            ("unclosed call", "data("),
            ("unclosed array", "["),
            ("single-quoted brace", "data + 'a}b'"),
            ("braced garbage", "{ a = 1 }"),
            ("parenthesized non-multi-emit", "(data)"),
            ("alien bytes", "@ # $"),
        ] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{\n      {garbage}\n      out = data\n    }}\n  }}\n  . -> o -> .\n}}\n"
            );
            let (statements, bag) = handler_statements(&source);
            assert!(!bag.diagnostics().is_empty(), "{label}: the refusal speaks");
            assert!(
                bag.diagnostics()
                    .iter()
                    .all(|d| d.code() == E_PARSE_EXPECTED),
                "{label}: no foreign voice — {bag:?}"
            );
            let last = statements.len() - 1;
            let Statement::Assign { value, .. } = &statements[last] else {
                panic!("{label}: the sibling lives: {statements:?}");
            };
            assert!(
                matches!(value, Value::Expr(expr) if expr.text == "data"),
                "{label}"
            );
        }
    }

    #[test]
    fn a_multiline_bare_value_is_one_hole_not_live_statements() {
        // A value in statement position is invalid, but its internal
        // fields are never statements of the enclosing handler. The
        // real value parser owns every structurally recognizable form
        // before the position diagnostic is emitted.
        let registry = crate::contract::Registry::try_m0().expect("the standard catalogue holds");
        for (label, bare) in [
            ("struct", "{\n        a = 1\n      }"),
            ("map", "{\n        \"a\": 1\n      }"),
            (
                "Rastok expression array",
                "[\n        1,\n        2\n      ]",
            ),
            ("Rastok expression call", "size(\n        data\n      )"),
            ("Rastok expression unary", "!size(\n        data\n      )"),
        ] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{\n      {bare}\n      out = data\n    }}\n  }}\n  . -> o -> .\n}}\n"
            );
            let (statements, bag) = handler_statements(&source);
            assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
            assert_eq!(
                bag.diagnostics()[0].code(),
                E_PARSE_EXPECTED,
                "{label}: one position voice"
            );
            assert_eq!(statements.len(), 2, "{label}: {statements:?}");
            assert!(
                matches!(&statements[0], Statement::Error(_)),
                "{label}: the whole bare value is one hole"
            );
            let Statement::Assign { target, value, .. } = &statements[1] else {
                panic!("{label}: the sibling lives: {:?}", statements[1]);
            };
            assert_eq!(target.segments[0].text, "out", "{label}");
            assert!(matches!(value, Value::Expr(expr) if expr.text == "data"));

            let rejected = crate::compiler::compile(&[&source], &registry)
                .expect_err("a misplaced value refuses the compilation");
            assert_eq!(
                rejected.diagnostics().len(),
                1,
                "{label}: no checker warning from phantom syntax — {rejected:?}"
            );
            assert_eq!(
                rejected.diagnostics()[0].code(),
                E_PARSE_EXPECTED,
                "{label}"
            );
        }
    }

    #[test]
    fn a_structural_lexical_fault_is_not_hidden_by_bare_recovery() {
        // Host recovery is line-bounded. An unterminated ordinary
        // string therefore speaks with its lexical code and leaves
        // the next statement intact instead of consuming the file.
        for (label, garbage) in [
            ("single quote", "data + 'unterminated"),
            ("double quote", "data + \"unterminated"),
            ("leading single quote", "'unterminated"),
            ("leading double quote", "\"unterminated"),
            ("escaped single quote newline", "'unterminated\\"),
            ("escaped double quote newline", "\"unterminated\\"),
        ] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{\n      {garbage}\n      out = data\n    }}\n  }}\n  . -> o -> .\n}}\n"
            );
            let (statements, bag) = handler_statements(&source);
            assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
            assert_eq!(
                bag.diagnostics()[0].code(),
                crate::compiler::lexer::E_LEX_UNTERMINATED_STRING,
                "{label}: the cause, not a generic position refusal"
            );
            assert_eq!(statements.len(), 2, "{label}: {statements:?}");
            assert!(matches!(&statements[0], Statement::Error(_)), "{label}");
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: the sibling lives: {:?}", statements[1]);
            };
            assert!(matches!(value, Value::Expr(expr) if expr.text == "data"));
        }

        // A block comment really extends to EOF, so no sibling can
        // be recovered. Its structural lexical fault must still be
        // the first and truthful voice; a scratch bag cannot erase it.
        let source = "graph g : App {\n  operator o {\n    on in {\n      data + /* unterminated\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let _ = parse(SourceId::new(0), source, &mut bag);
        assert_eq!(
            bag.diagnostics()[0].code(),
            crate::compiler::lexer::E_LEX_UNTERMINATED_COMMENT,
            "the terminal lexical cause survives recovery: {bag:?}"
        );
    }

    #[test]
    fn an_incomplete_expression_value_never_consumes_the_next_statement() {
        // The Rastok expression framer may look beyond a newline to prove a balanced
        // multiline expression, but an INCOMPLETE extent must recover
        // at the value position's first safe line boundary. The Rastok expression
        // compiler owns the one syntax diagnostic; the host parser
        // only preserves the surrounding RKG structure.
        let registry = crate::contract::Registry::try_m0().expect("the standard catalogue holds");
        for (label, malformed) in [
            ("double quote", "\"oops"),
            ("single quote", "'oops"),
            ("unclosed call", "size(data"),
            ("unclosed array", "[1,"),
        ] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{\n      out = {malformed}\n      out = data\n    }}\n  }}\n  . -> o -> .\n}}\n"
            );
            let (statements, parse_bag) = handler_statements(&source);
            assert!(
                parse_bag.diagnostics().is_empty(),
                "{label}: Rastok expression, not the host parser, owns the malformed expression — {parse_bag:?}"
            );
            assert_eq!(statements.len(), 2, "{label}: {statements:?}");
            let Statement::Assign {
                value: Value::Expr(expr),
                ..
            } = &statements[0]
            else {
                panic!(
                    "{label}: malformed Rastok expression remains one value: {:?}",
                    statements[0]
                );
            };
            assert_eq!(
                expr.text, malformed,
                "{label}: the region stops on its line"
            );
            let Statement::Assign { value, .. } = &statements[1] else {
                panic!("{label}: the sibling lives: {:?}", statements[1]);
            };
            assert!(matches!(value, Value::Expr(expr) if expr.text == "data"));

            let rejected = crate::compiler::compile(&[source.as_str()], &registry)
                .expect_err("malformed Rastok expression refuses the compilation");
            assert_eq!(
                rejected.diagnostics().len(),
                1,
                "{label}: one fault, one Rastok expression voice — {rejected:?}"
            );
            assert_eq!(
                rejected.diagnostics()[0].code(),
                crate::compiler::check::E_EXPR,
                "{label}"
            );
        }
    }

    #[test]
    fn incomplete_expression_has_one_voice_in_attributes_and_guards() {
        let registry = crate::contract::Registry::try_m0().expect("the standard catalogue holds");
        for (label, source) in [
            (
                "attribute",
                "graph g : App {\n  connector shell : Process {\n    command = \"oops\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> shell -> .\n}\n",
            ),
            (
                "guard",
                "graph g : App {\n  operator o {\n    on in where true == \"oops\n    { out = data }\n  }\n  . -> o -> .\n}\n",
            ),
        ] {
            let rejected = crate::compiler::compile(&[source], &registry)
                .expect_err("malformed Rastok expression refuses the compilation");
            assert_eq!(
                rejected.diagnostics().len(),
                1,
                "{label}: the following construct survives and the fault speaks once — {rejected:?}"
            );
            assert_eq!(
                rejected.diagnostics()[0].code(),
                crate::compiler::check::E_EXPR,
                "{label}"
            );
        }
    }

    #[test]
    fn expression_calls_are_framed_as_one_value() {
        // Asset is an ordinary expression call. The host owns only the complete
        // expression extent; comments and line breaks inside its parens do not
        // create a second host argument grammar.
        let multiline_string = "graph g : App {\n  operator o {\n    on in { out = asset(\n  \"README.md\"\n) }\n  }\n  . -> o -> .\n}\n";
        let (statements, bag) = handler_statements(multiline_string);
        assert!(bag.diagnostics().is_empty(), "multiline string: {bag:?}");
        let Statement::Assign {
            value: Value::Expr(call),
            ..
        } = &statements[0]
        else {
            panic!("the asset call: {:?}", statements[0]);
        };
        assert!(
            call.text.contains("\"README.md\""),
            "the expression owns the multiline call: {call:?}"
        );
        let commented = "graph g : App {\n  operator o {\n    on in { out = asset( /* path\n      continues */ \"a.bin\" ) }\n  }\n  . -> o -> .\n}\n";
        let (statements, bag) = handler_statements(commented);
        assert!(
            bag.diagnostics().is_empty(),
            "comment before string: {bag:?}"
        );
        let Statement::Assign {
            value: Value::Expr(call),
            ..
        } = &statements[0]
        else {
            panic!("the asset call: {:?}", statements[0]);
        };
        assert!(call.text.contains("/* path"), "{call:?}");

        // An arbitrary expression call obeys exactly the same framing.
        let continued = "graph g : App {\n  operator o {\n    on in { out = bounded(\"x\"\n  + \"y\") }\n  }\n  . -> o -> .\n}\n";
        let (statements, bag) = handler_statements(continued);
        assert!(bag.diagnostics().is_empty(), "continuation: {bag:?}");
        let Statement::Assign {
            value: Value::Expr(call),
            ..
        } = &statements[0]
        else {
            panic!("the bounded call: {:?}", statements[0]);
        };
        assert!(
            call.text.contains('+'),
            "the continuation is one expression region: {call:?}"
        );
    }

    fn parse_clean(source: &str) -> SyntaxTree {
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), source, &mut bag);
        assert!(
            bag.diagnostics().is_empty(),
            "unexpected diagnostics: {bag:?}"
        );
        tree
    }

    const DECLS_ONLY: &str = r#"
graph m0_probe : App {
  partition session {
    key = meta.app.session_key
    lifetime = duration("2h")
  }

  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input loud : text
    val greeting = "hi"
    var count : int = 0
  }

  val retries = 3
}
"#;

    #[test]
    fn a_declarations_only_file_parses() {
        let tree = parse_clean(DECLS_ONLY);
        assert_eq!(tree.decls.len(), 1);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        assert_eq!(graph.name.text, "m0_probe");
        assert_eq!(graph.annotation.as_ref().unwrap().symbols[0].text, "App");
        assert_eq!(graph.body.len(), 4);
        assert!(matches!(graph.body[0], GraphItem::Partition(_)));
        assert!(matches!(graph.body[1], GraphItem::Connector(_)));
        assert!(matches!(graph.body[2], GraphItem::Operator(_)));
        assert!(matches!(graph.body[3], GraphItem::Val(_)));
    }

    #[test]
    fn the_connector_body_is_five_attributes() {
        let tree = parse_clean(DECLS_ONLY);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Connector(echo) = &graph.body[1] else {
            panic!("expected the connector");
        };
        assert_eq!(echo.name.text, "echo");
        assert_eq!(echo.annotation.symbols[0].text, "Process");
        let keys: Vec<&str> = echo
            .body
            .iter()
            .map(|item| match item {
                OperatorItem::Attribute(attribute) => attribute.target.segments[0].text.as_str(),
                other => panic!("expected an attribute, got {other:?}"),
            })
            .collect();
        assert_eq!(keys, ["mode", "command", "codec", "effect", "behavior"]);
        let OperatorItem::Attribute(command) = &echo.body[1] else {
            panic!("expected an attribute");
        };
        let Value::Expr(expr) = &command.value else {
            panic!("expected an expression value");
        };
        assert_eq!(expr.text, "\"cat\"");
    }

    #[test]
    fn the_partition_holds_key_and_lifetime() {
        let tree = parse_clean(DECLS_ONLY);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Partition(session) = &graph.body[0] else {
            panic!("expected the partition");
        };
        assert_eq!(session.name.text, "session");
        let PartitionField::Key(key) = &session.fields[0] else {
            panic!("expected the key");
        };
        assert_eq!(key.text, "meta.app.session_key");
        let PartitionField::Lifetime(LifetimeValue::Expr(ttl)) = &session.fields[1] else {
            panic!("expected the idle-ttl lifetime");
        };
        assert_eq!(ttl.text, "duration(\"2h\")");
    }

    #[test]
    fn the_operator_holds_port_val_and_var() {
        let tree = parse_clean(DECLS_ONLY);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Operator(shout) = &graph.body[2] else {
            panic!("expected the operator");
        };
        let OperatorItem::Port(port) = &shout.body[0] else {
            panic!("expected the input port");
        };
        assert_eq!(port.direction, PortDirection::Input);
        assert!(matches!(
            port.ty.as_ref().unwrap().alternatives[0],
            BaseType::Scalar(ScalarType::Text)
        ));
        assert!(matches!(&shout.body[1], OperatorItem::Val(_)));
        let OperatorItem::Var(count) = &shout.body[2] else {
            panic!("expected the var");
        };
        assert!(matches!(
            count.ty.alternatives[0],
            BaseType::Scalar(ScalarType::Int)
        ));
    }

    #[test]
    fn nested_generic_types_parse() {
        let tree = parse_clean("val x : map<text, array<int>>? = y\n");
        let TopDecl::Val(val) = &tree.decls[0] else {
            panic!("expected a val");
        };
        let ty = val.ty.as_ref().unwrap();
        assert!(ty.optional);
        let BaseType::Map(keys, values) = &ty.alternatives[0] else {
            panic!("expected a map");
        };
        assert!(matches!(
            keys.alternatives[0],
            BaseType::Scalar(ScalarType::Text)
        ));
        assert!(matches!(values.alternatives[0], BaseType::Array(_)));
    }

    #[test]
    fn open_struct_types_parse() {
        let tree = parse_clean("val x : struct { kind: text, .. } = y\n");
        let TopDecl::Val(val) = &tree.decls[0] else {
            panic!("expected a val");
        };
        let BaseType::Struct { fields, open } = &val.ty.as_ref().unwrap().alternatives[0] else {
            panic!("expected a struct type");
        };
        assert!(open);
        assert_eq!(fields[0].name.text, "kind");
    }

    #[test]
    fn a_raw_guest_body_stays_opaque() {
        let tree = parse_clean("val j = jq! { .items | {count: length} }\n");
        let TopDecl::Val(val) = &tree.decls[0] else {
            panic!("expected a val");
        };
        let Value::Frontend(frontend) = &val.value else {
            panic!("expected a frontend value");
        };
        assert_eq!(frontend.name.text, "jq");
        let FrontendPayload::Raw { text, .. } = &frontend.payload else {
            panic!("expected a raw body");
        };
        assert_eq!(text.trim(), ".items | {count: length}");
    }

    #[test]
    fn recovery_leaves_a_hole_and_keeps_parsing() {
        let mut bag = Bag::new();
        let source = "graph g : App {\n  connector c {\n    mode = job\n  }\n  val x = 1\n}\n";
        let tree = parse(SourceId::new(0), source, &mut bag);
        assert_eq!(bag.diagnostics().len(), 1);
        assert_eq!(bag.diagnostics()[0].code(), E_PARSE_EXPECTED);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        assert!(matches!(graph.body[0], GraphItem::Error(_)));
        let GraphItem::Val(val) = &graph.body[1] else {
            panic!("expected the val to survive recovery");
        };
        assert_eq!(val.name.text, "x");
    }

    #[test]
    fn a_lab_parses_into_its_simulation_surface() {
        let mut bag = Bag::new();
        let source = r#"
lab m0_probe {
  simulation smoke on app {
    mock echo {
      on in { out = { text = "salut" } }
    }
    scenario roundtrip "one message through" {
      app.in = { text = "salut" }
      expect app.out count == 1 where data.text == "salut"
      advance duration("5s")
      expect nothing app.err
    }
  }
}
val x = 1
"#;
        let tree = parse(SourceId::new(0), source, &mut bag);
        assert!(bag.diagnostics().is_empty(), "{:#?}", bag.diagnostics());
        let TopDecl::Lab(lab) = &tree.decls[0] else {
            panic!("expected the lab");
        };
        assert_eq!(lab.name.text, "m0_probe");
        let simulation = &lab.simulations[0];
        assert_eq!(simulation.name.text, "smoke");
        assert_eq!(simulation.target[0].text, "app");
        assert_eq!(simulation.mocks[0].path[0].text, "echo");
        assert_eq!(simulation.mocks[0].handlers.len(), 1);
        let scenario = &simulation.scenarios[0];
        assert_eq!(scenario.label.as_deref(), Some("one message through"));
        assert!(
            matches!(&scenario.body[0], ScenarioItem::Injection { target, .. }
            if target.segments[0].text == "app" && target.segments[1].text == "in")
        );
        let ScenarioItem::Expect(first) = &scenario.body[1] else {
            panic!("expected an expect step");
        };
        assert!(!first.nothing);
        assert_eq!(
            first.count,
            Some((
                CmpOp::Eq,
                IntLit {
                    text: String::from("1"),
                    span: first.count.as_ref().unwrap().1.span
                }
            ))
        );
        assert!(first.predicate.is_some());
        assert!(matches!(&scenario.body[2], ScenarioItem::Advance { .. }));
        let ScenarioItem::Expect(last) = &scenario.body[3] else {
            panic!("expected the void expect");
        };
        assert!(last.nothing);
        assert!(matches!(tree.decls[1], TopDecl::Val(_)));
    }

    #[test]
    fn experiments_and_theorems_are_refused_whole() {
        let mut bag = Bag::new();
        let source = "lab m0_probe {\n  experiment probe on app { profile = \"staging\" }\n  theorem safety over smoke { anything { nested } }\n}\n";
        let tree = parse(SourceId::new(0), source, &mut bag);
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_PARSE_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert_eq!(constructs, ["an experiment", "a theorem"]);
        assert!(matches!(&tree.decls[0], TopDecl::Lab(lab) if lab.simulations.is_empty()));
    }

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    on in where data.text != "" {
      out = { data.text = data.text }
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_fixture_parses_end_to_end() {
        let tree = parse_clean(M0_PROBE);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected the app graph");
        };
        let GraphItem::Operator(shout) = &graph.body[1] else {
            panic!("expected the operator");
        };
        let OperatorItem::Handler(handler) = &shout.body[0] else {
            panic!("expected the handler");
        };
        assert!(matches!(&handler.trigger, Trigger::Port(port) if port.text == "in"));
        let Guard::Where(guard) = &handler.guards[0] else {
            panic!("expected the where guard");
        };
        assert_eq!(guard.text, "data.text != \"\"");
        let statements = handler.body.as_ref().unwrap();
        let Statement::Assign { target, value, .. } = &statements[0] else {
            panic!("expected the emission");
        };
        assert_eq!(target.segments[0].text, "out");
        assert!(matches!(value, Value::Braced(_)));
        let GraphItem::Edge(edge) = &graph.body[2] else {
            panic!("expected the wiring chain");
        };
        assert_eq!(edge.endpoints.len(), 4);
        assert!(matches!(
            &edge.endpoints[0],
            EdgeEndpoint::Ref(port) if port.boundary && port.segments.is_empty()
        ));
        assert!(matches!(
            &edge.endpoints[2],
            EdgeEndpoint::Ref(port) if !port.boundary && port.segments[0].text == "echo"
        ));
    }

    #[test]
    fn ring_two_handler_forms_parse_faithfully() {
        let source = "graph g : App {\n  operator o {\n    on (a, b) where a.x > 0 having count(a) >= 2 by a.key within duration(\"5m\") last 3 { }\n    on start { }\n    on Loop.stop\n  }\n}\n";
        let tree = parse_clean(source);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("expected the operator");
        };
        let OperatorItem::Handler(join) = &operator.body[0] else {
            panic!("expected the join handler");
        };
        assert!(matches!(&join.trigger, Trigger::Join(ports) if ports.len() == 2));
        assert_eq!(join.guards.len(), 5);
        assert!(matches!(&join.guards[4], Guard::Last(count) if count.text == "3"));
        let OperatorItem::Handler(start) = &operator.body[1] else {
            panic!("expected the start handler");
        };
        assert!(matches!(start.trigger, Trigger::Start(_)));
        let OperatorItem::Handler(base_case) = &operator.body[2] else {
            panic!("expected the base-case handler");
        };
        assert!(matches!(
            &base_case.trigger,
            Trigger::Aspect { aspect, event: AspectEvent::Stop } if aspect.text == "Loop"
        ));
        assert!(base_case.body.is_none());
    }

    #[test]
    fn statements_cover_the_grammar_forms() {
        let source = "graph g : App {\n  operator o {\n    on in {\n      val doubled = data.n * 2\n      (out, audit) = (envelope, envelope)\n      err = PaymentDeclined { transient = false }\n      out = count + doubled\n      stop\n    }\n  }\n}\n";
        let tree = parse_clean(source);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("expected the operator");
        };
        let OperatorItem::Handler(handler) = &operator.body[0] else {
            panic!("expected the handler");
        };
        let statements = handler.body.as_ref().unwrap();
        assert!(matches!(&statements[0], Statement::Val(val) if val.name.text == "doubled"));
        let Statement::MultiEmit { targets, values } = &statements[1] else {
            panic!("expected the multi-emit");
        };
        assert_eq!(targets.len(), 2);
        assert_eq!(values.len(), 2);
        let Statement::Assign { target, value, .. } = &statements[2] else {
            panic!("expected an ordinary port assignment");
        };
        assert_eq!(target.segments[0].text, "err");
        assert!(matches!(value, Value::Tagged { case, .. } if case.text == "PaymentDeclined"));
        let Statement::Assign { value, .. } = &statements[3] else {
            panic!("expected the assignment: {:?}", statements[3]);
        };
        assert!(matches!(value, Value::Expr(expr) if expr.text == "count + doubled"));
        assert!(matches!(&statements[4], Statement::Stop(None)));
    }

    #[test]
    fn expression_openings_never_meet_the_host_lexer() {
        // `!`, a single quote, and `-(` are not Rastok tokens, but they
        // open valid Rastok expression: they must reach the expression chain untouched.
        let source = "graph g : App {\n  operator o {\n    val a = !false\n    val b = 'hello'\n    val c = -(1)\n    on in {\n      out = !done\n    }\n  }\n}\n";
        let tree = parse_clean(source);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("expected the operator");
        };
        let texts: Vec<&str> = operator
            .body
            .iter()
            .take(3)
            .map(|item| match item {
                OperatorItem::Val(val) => match &val.value {
                    Value::Expr(expr) => expr.text.as_str(),
                    other => panic!("expected Rastok expression, got {other:?}"),
                },
                other => panic!("expected a val, got {other:?}"),
            })
            .collect();
        assert_eq!(texts, ["!false", "'hello'", "-(1)"]);
        let OperatorItem::Handler(handler) = &operator.body[3] else {
            panic!("expected the handler");
        };
        assert!(matches!(
            &handler.body.as_ref().unwrap()[0],
            Statement::Assign { value: Value::Expr(expr), .. } if expr.text == "!done"
        ));
    }

    #[test]
    fn boundary_and_path_dots_respect_line_separation() {
        // `b -> .` then `a -> .` on the next line: the dots must not
        // swallow the identifiers across the dropped newlines.
        let source = "graph g : App {\n  . -> a -> .\n  b -> .\n}\n";
        let tree = parse_clean(source);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        assert_eq!(graph.body.len(), 2, "two separate edges: {graph:#?}");
        let GraphItem::Edge(first) = &graph.body[0] else {
            panic!("expected the first edge");
        };
        assert!(matches!(
            &first.endpoints[2],
            EdgeEndpoint::Ref(port) if port.boundary && port.segments.is_empty()
        ));
        let GraphItem::Edge(second) = &graph.body[1] else {
            panic!("expected the second edge");
        };
        assert!(matches!(
            &second.endpoints[0],
            EdgeEndpoint::Ref(port) if !port.boundary && port.segments[0].text == "b"
        ));
        // Adjacency still admits tight forms on one line.
        let tight = parse_clean("graph g : App {\n  .feed -> x.in\n  output feed\n}\n");
        let TopDecl::Graph(graph) = &tight.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Edge(edge) = &graph.body[0] else {
            panic!("expected the edge");
        };
        assert!(matches!(
            &edge.endpoints[0],
            EdgeEndpoint::Ref(port) if port.boundary && port.segments[0].text == "feed"
        ));
    }

    #[test]
    fn spaces_and_comments_around_dots_stay_trivia() {
        // Same line: spacing and comments must not change the parse.
        let spaced = parse_clean(
            "graph g : App {\n  output feed\n  o . out -> . feed\n  operator o { on in { out = . } }\n}\n",
        );
        let TopDecl::Graph(graph) = &spaced.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Edge(edge) = &graph.body[1] else {
            panic!("expected the edge");
        };
        assert!(matches!(
            &edge.endpoints[0],
            EdgeEndpoint::Ref(port) if port.segments.len() == 2 && port.segments[1].text == "out"
        ));
        assert!(matches!(
            &edge.endpoints[1],
            EdgeEndpoint::Ref(port) if port.boundary && port.segments[0].text == "feed"
        ));
        let commented = parse_clean(
            "graph g : App {\n  output feed\n  o.out -> ./* target */feed\n  operator o { on in { out = . } }\n}\n",
        );
        let TopDecl::Graph(graph) = &commented.decls[0] else {
            panic!("expected a graph");
        };
        assert!(matches!(&graph.body[1], GraphItem::Edge(_)));
    }

    #[test]
    fn a_same_line_comment_before_a_value_is_trivia() {
        let tree = parse_clean(
            "graph g : App {\n  operator o {\n    val enabled = /* toggled */ !false\n  }\n}\n",
        );
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Operator(operator) = &graph.body[0] else {
            panic!("expected the operator");
        };
        let OperatorItem::Val(val) = &operator.body[0] else {
            panic!("expected the val");
        };
        assert!(matches!(&val.value, Value::Expr(expr) if expr.text == "!false"));
    }

    #[test]
    fn wiring_covers_branches_guards_and_discard() {
        let source = "graph g : App {\n  route -> { audit.in ; billing.in }\n  route -> nothing\n  a.out -> b.in when data.kind == \"approval\"\n  c <-> d\n}\n";
        let tree = parse_clean(source);
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        let GraphItem::Edge(fan_out) = &graph.body[0] else {
            panic!("expected the fan-out edge");
        };
        assert!(matches!(
            &fan_out.endpoints[1],
            EdgeEndpoint::Branch(refs) if refs.len() == 2
        ));
        let GraphItem::Edge(discard) = &graph.body[1] else {
            panic!("expected the discard edge");
        };
        assert!(matches!(&discard.endpoints[1], EdgeEndpoint::Nothing(_)));
        let GraphItem::Edge(guarded) = &graph.body[2] else {
            panic!("expected the guarded edge");
        };
        assert_eq!(
            guarded.guard.as_ref().unwrap().text,
            "data.kind == \"approval\""
        );
        let GraphItem::Edge(duplex) = &graph.body[3] else {
            panic!("expected the duplex edge");
        };
        assert!(duplex.bidirectional);
    }
}
