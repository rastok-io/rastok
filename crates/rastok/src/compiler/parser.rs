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

use crate::compiler::lexer::{Lexer, TerminatorSet, Token, TokenKind};
use crate::compiler::syntax::{
    Annotation, AspectEvent, Attribute, BaseType, CelExpr, CmpOp, ConnectorDecl, EdgeDecl,
    EdgeEndpoint, ErrorNode, ExpectStep, FieldDecl, GraphDecl, GraphItem, Guard, HandlerDecl,
    IntLit, LValue, LabDecl, LifetimeValue, MockDecl, Name, OperatorDecl, OperatorItem,
    PartitionDecl, PartitionField, PerClause, PortDecl, PortDirection, PortRef, ScalarType,
    ScenarioDecl, ScenarioItem, SimulationDecl, Statement, StructBody, StructEntry, SubBlock,
    SyntaxTree, TaggedBody, TaggedValue, TopDecl, Trigger, TypeRef, ValDecl, Value, VarDecl,
    WindowMode,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};

/// Diagnostic for a token that breaks the expected production.
pub const E_PARSE_EXPECTED: Code = Code::new("E-PARSE-EXPECTED");
/// Diagnostic for grammatical syntax outside the implemented subset.
pub const E_PARSE_DEFERRED: Code = Code::new("E-PARSE-DEFERRED");

/// Diagnostic for a `json` literal the house reader refuses — its
/// own code: JSON is not CEL (D-2026-08-12-1, review round).
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

/// Tags whose block body is raw guest text, never read by the host.
const RAW_TAGS: &[&str] = &[
    "jq", "bash", "sql", "python", "rust", "json", "text", "base64",
];

/// Tag words that open tagged-value expressions: foreign literals
/// (guests, raw literals, data formats, `json`) and the
/// expression-space constructors. Tagged INITIALIZERS left the
/// vocabulary (D-2026-08-12-1) — a plain struct with a
/// closed-choice discriminant field replaces each one, and the
/// dead spelling refuses with its migration hint.
const TAG_WORDS: &[&str] = &[
    "jq",
    "bash",
    "sql",
    "python",
    "rust",
    "json",
    "avro",
    "protobuf",
    "parquet",
    "msgpack",
    "base64",
    "csv",
    "text",
    "asset",
    "bounded",
    "unbounded",
    "regex",
];

/// The RESERVED data-format tags: words the catalogue holds with no
/// living form yet. A body after one is consumed OPAQUELY — never
/// read as an RKG struct — and defers with ONE named diagnostic
/// (D-2026-08-12-1, review round).
const RESERVED_FORMAT_TAGS: &[&str] = &["avro", "protobuf", "parquet", "msgpack", "csv"];

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

/// Parses one `.rkg` source into a syntax tree, accumulating
/// diagnostics; the tree is returned even when faulty.
pub fn parse(source_id: SourceId, source: &str, bag: &mut Bag) -> SyntaxTree {
    Parser {
        lexer: Lexer::new(source_id, source),
        peeked: None,
        source_id,
        last_end: 0,
        bag,
    }
    .file()
}

struct Parser<'src, 'bag> {
    lexer: Lexer<'src>,
    peeked: Option<Token<'src>>,
    source_id: SourceId,
    last_end: usize,
    bag: &'bag mut Bag,
}

impl<'src> Parser<'src, '_> {
    // ----- token plumbing -----

    fn peek(&mut self) -> Token<'src> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token(self.bag));
        }
        self.peeked.unwrap_or_else(|| unreachable!("just filled"))
    }

    fn bump(&mut self) -> Token<'src> {
        let token = self.peek();
        self.peeked = None;
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

    fn cel(&mut self, terminators: &TerminatorSet) -> CelExpr {
        if let Some(token) = &self.peeked {
            // The peeked token IS the expression's first piece: the
            // continuation re-reads the immutable prefix slice.
            let start = token.span.range().start;
            return self.cel_from(start, terminators);
        }
        // Same-line comments before a value are trivia; a bare
        // newline is not — it still reads as an empty expression.
        self.lexer.advance_within_line(self.bag);
        let token = self.lexer.cel_expr(terminators, self.bag);
        self.last_end = token.span.range().end;
        CelExpr {
            text: String::from(token.text),
            span: token.span,
        }
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
                    let expr = self.cel(&ATTR_POSITION);
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
    /// cel)?`.
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
            Some(self.cel(&ATTR_POSITION))
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

    /// One comparison operator, tokenized as its parts.
    fn cmp_op(&mut self) -> Option<CmpOp> {
        match self.peek().kind {
            TokenKind::Eq => {
                self.bump();
                self.expect(TokenKind::Eq, "`==`")?;
                Some(CmpOp::Eq)
            }
            TokenKind::Bang => {
                self.bump();
                self.expect(TokenKind::Eq, "`!=`")?;
                Some(CmpOp::Ne)
            }
            TokenKind::Lt => {
                self.bump();
                if self.peek().kind == TokenKind::Eq {
                    self.bump();
                    Some(CmpOp::Le)
                } else {
                    Some(CmpOp::Lt)
                }
            }
            TokenKind::Gt => {
                self.bump();
                if self.peek().kind == TokenKind::Eq {
                    self.bump();
                    Some(CmpOp::Ge)
                } else {
                    Some(CmpOp::Gt)
                }
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
            Some(self.cel(&GUARD_POSITION))
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
                    guards.push(Guard::Where(self.cel(&GUARD_POSITION)));
                }
                "having" => {
                    self.bump();
                    guards.push(Guard::Having(self.cel(&GUARD_POSITION)));
                }
                "by" => {
                    self.bump();
                    guards.push(Guard::By(self.cel(&GUARD_POSITION)));
                }
                "within" => {
                    self.bump();
                    guards.push(Guard::Within(self.cel(&GUARD_POSITION)));
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

    /// `"{" ( statement )* "}"`
    /// The first significant byte ahead — the dispatch probe for
    /// positions that may hold CEL text. CEL never passes through this
    /// parser's lexer: anything that does not shape like a Rastok
    /// construct is handed to the CEL chain untouched.
    fn shape(&mut self) -> Option<u8> {
        match &self.peeked {
            Some(token) if token.kind == TokenKind::Eof => None,
            Some(token) => token.text.bytes().next(),
            None => self.lexer.first_significant_byte(),
        }
    }

    fn statements(&mut self) -> Option<Vec<Statement>> {
        self.expect(TokenKind::LBrace, "`{`")?;
        let mut statements = Vec::new();
        loop {
            // Probe by shape first: a bare CEL yield (`!done`, `-x`,
            // `'txt'`) must not be tokenized — or rejected — here.
            let statement_shaped = matches!(
                self.shape(),
                None | Some(b'}' | b';' | b'(' | b'a'..=b'z' | b'A'..=b'Z' | b'_')
            );
            if !statement_shaped {
                // The separating newline is trivia; the yield's own
                // scan must start past it.
                if self.peeked.is_none() {
                    self.lexer.advance_to_significant(self.bag);
                }
                let value = self.value(&ATTR_POSITION);
                if matches!(&value, Value::Cel(cel) if cel.text.is_empty()) {
                    // Guaranteed progress: an empty scan consumed
                    // nothing, so report and step over the byte.
                    let start = self.peek().span.range().start;
                    self.error_expected("a statement");
                    self.bump();
                    statements.push(Statement::Error(self.recover(start)));
                } else {
                    statements.push(Statement::Yield(value));
                }
                continue;
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
                TokenKind::LParen => match self.multi_emit(start) {
                    Some(statement) => statements.push(statement),
                    None => statements.push(Statement::Error(self.recover(start))),
                },
                TokenKind::Ident => statements.push(self.assign_or_yield(start)),
                _ => statements.push(Statement::Yield(self.value(&ATTR_POSITION))),
            }
        }
        self.expect(TokenKind::RBrace, "`}`")?;
        Some(statements)
    }

    /// An IDENT-led statement: assignment, typed-error emission, or an
    /// implicit yield whose expression starts with a path.
    fn assign_or_yield(&mut self, start: usize) -> Statement {
        let Some(target) = self.lvalue() else {
            return Statement::Error(self.recover(start));
        };
        match self.peek().kind {
            TokenKind::Eq if target.segments.len() == 1 && target.segments[0].text == "err" => {
                self.bump();
                match self.name("an error kind") {
                    Some(kind) => {
                        let body = if self.peek().kind == TokenKind::LBrace {
                            self.struct_body()
                        } else {
                            None
                        };
                        Statement::ErrEmit { kind, body }
                    }
                    None => Statement::Error(self.recover(start)),
                }
            }
            TokenKind::Eq => {
                self.bump();
                Statement::Assign {
                    target,
                    ty: None,
                    value: self.value(&ATTR_POSITION),
                }
            }
            TokenKind::Colon => {
                self.bump();
                let Some(ty) = self.type_ref() else {
                    return Statement::Error(self.recover(start));
                };
                if self.expect(TokenKind::Eq, "`=`").is_none() {
                    return Statement::Error(self.recover(start));
                }
                Statement::Assign {
                    target,
                    ty: Some(ty),
                    value: self.value(&ATTR_POSITION),
                }
            }
            _ => {
                // Not an assignment: the consumed path is the start
                // of a bare value — the CEL continuation re-reads the
                // immutable prefix slice (D-2026-08-13-1).
                Statement::Yield(Value::Cel(self.cel_from(start, &ATTR_POSITION)))
            }
        }
    }

    /// `( a, b ) = tuple_or_broadcast` — atomic multi-emit.
    fn multi_emit(&mut self, _start: usize) -> Option<Statement> {
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
                    fields.push(PartitionField::Key(self.cel(position)));
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
                LifetimeValue::Fixed(self.cel(position))
            }
            (TokenKind::Ident, "window") => {
                self.bump();
                let duration = self.cel(&WINDOW_POSITION);
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
            _ => LifetimeValue::Expr(self.cel(position)),
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
        // that follows, while spaces and comments stay trivia.
        loop {
            let dot = self.peek();
            if dot.kind != TokenKind::Dot || dot.newline_before {
                break;
            }
            self.bump();
            segments.push(self.name("a path segment")?);
        }
        Some(LValue {
            segments,
            span: self.span_from(start),
        })
    }

    // ----- values -----

    /// A value position; `position` chooses the CEL terminators.
    fn value(&mut self, position: &TerminatorSet) -> Value {
        // A newline-terminated position must hold a value ON ITS
        // LINE: an omitted value diagnoses HERE and consumes
        // nothing — it never absorbs the next line's construct
        // (D-2026-08-12-1, review round). `;` and `}` end the
        // statement the same way a newline would.
        if position.newline {
            let missing = match &self.peeked {
                Some(token) => {
                    token.kind == TokenKind::Eof
                        || token.newline_before
                        || matches!(token.kind, TokenKind::Semi | TokenKind::RBrace)
                }
                None => matches!(
                    self.lexer.significant_byte_on_line(),
                    None | Some(b';' | b'}')
                ),
            };
            if missing {
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
        // Probe by shape before touching the token stream: CEL text
        // may open with bytes this lexer rejects (`!`, `'`, `-(`), so
        // only builder-shaped input is tokenized here.
        let builder_shaped = matches!(
            self.shape(),
            Some(b'{' | b'.' | b'a'..=b'z' | b'A'..=b'Z' | b'_')
        );
        if !builder_shaped {
            return Value::Cel(self.cel(position));
        }
        let token = self.peek();
        match (token.kind, token.text) {
            (TokenKind::LBrace, _) => self.braced_value(token.span),
            (TokenKind::Dot, _) => {
                // `.` — the exact re-emission of the current
                // envelope; grammar forbids leading dots in CEL, so
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
            _ => Value::Cel(self.cel(position)),
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
        if word == "json" {
            return self.json_tail(&ident, position);
        }
        let follower = self.lexer.significant_on_line().map(|(_, byte)| byte);
        if follower.is_none() && position.newline {
            // Nothing significant follows on the line and the line
            // ends the value: the word stands alone. No scan — a
            // comment crossing the line stays HOST trivia, and an
            // unterminated one is the trivia authority's diagnostic
            // (D-2026-08-13-1).
            return Value::Cel(CelExpr {
                text: String::from(ident.text),
                span: ident.span,
            });
        }
        let opens = matches!(follower, Some(b'(' | b'{'));
        if DEAD_INITIALIZER_TAGS.contains(&word) && opens {
            return self.dead_initializer(&ident);
        }
        if RESERVED_FORMAT_TAGS.contains(&word) && opens {
            // A reserved format's body is OPAQUE — the RAW skippers
            // consume it, one named deferral, never a struct
            // misreading.
            self.deferred("a reserved data-format body", ident.span);
            if self.lexer.significant_on_line().map(|(_, byte)| byte) == Some(b'(') {
                self.lexer.skip_balanced_raw(b'(', b')');
            }
            if self.lexer.significant_on_line().map(|(_, byte)| byte) == Some(b'{') {
                self.lexer.skip_balanced_raw(b'{', b'}');
            }
            return Value::Error(ident.span);
        }
        if TAG_WORDS.contains(&word) && opens {
            return self.tagged_value(ident);
        }
        // An ordinary word — or a tag word standing alone: the CEL
        // continuation re-reads the immutable prefix slice.
        Value::Cel(self.cel_from(ident.span.range().start, position))
    }

    /// The `json` continuation: a DOCUMENT (`{`/`[` on the line), a
    /// bare word, a refused scalar spelling, or a CEL expression
    /// beginning with the word (D-2026-08-13-1). Raw inspection
    /// only — the delegated body never meets the host lexer.
    fn json_tail(&mut self, ident: &Token<'_>, position: &TerminatorSet) -> Value {
        let probe = self.lexer.significant_on_line();
        match probe {
            Some((offset, byte @ (b'{' | b'['))) => {
                let _ = byte;
                // The document reader owns its self-delimiting
                // braces; host trivia before the opener stayed the
                // host's, and no trim exists on this path.
                match rastok_core::json::parse_value(self.lexer.source_from(offset)) {
                    Ok((value, consumed)) => {
                        let end = offset + consumed;
                        self.lexer.advance_to(end);
                        self.last_end = end;
                        Value::Json {
                            value,
                            span: Span::new(self.source_id, offset..end),
                        }
                    }
                    Err(error) => {
                        let source_len = self.lexer.source_from(0).len();
                        let at = (offset + error.offset).min(source_len);
                        // An EOF point is an EMPTY range (Span's own
                        // contract); anything else underlines one
                        // byte.
                        let end = (at + 1).min(source_len);
                        self.bag.push(
                            crate::diagnostics::Diagnostic::new(
                                E_JSON_VALUE,
                                crate::diagnostics::Phase::Parse,
                                format!("the JSON value does not read: {}", error.message),
                            )
                            .with_span(Span::new(self.source_id, at..end))
                            .with_field("detail", error.message),
                        );
                        // Recovery follows the POSITION, opaquely
                        // and forward only.
                        self.lexer.advance_to(offset);
                        let end = self.lexer.skip_json_recovery(position);
                        self.last_end = end;
                        Value::Error(Span::new(self.source_id, offset..offset))
                    }
                }
            }
            None if position.newline => {
                // Nothing significant on the line and the line ends
                // the value: the bare word stands alone, span exact —
                // a crossing comment stays HOST trivia
                // (D-2026-08-13-1).
                Value::Cel(CelExpr {
                    text: String::from(ident.text),
                    span: ident.span,
                })
            }
            // A terminator on the line, or any CEL continuation:
            // the word belongs to CEL.
            _ => Value::Cel(self.cel_from(ident.span.range().start, position)),
        }
    }

    /// The CEL continuation of an already-consumed prefix: the END
    /// is scanned forward from the cursor, the text is the immutable
    /// `source[prefix_start..end]` slice — a bounded re-read, never
    /// a rewind (D-2026-08-13-1). Any peeked token is DROPPED (its
    /// bytes sit inside the slice).
    /// Whether the region scan would terminate IMMEDIATELY on this
    /// already-peeked token — the exact mirror of the lexer's
    /// terminator test, asked of a token instead of a byte. A
    /// boundary token belongs to the HOST: the continuation is empty
    /// and the peek MUST survive (D-2026-08-13-1, review round —
    /// dropping it lost `}` / `;` / next-line statements).
    fn boundary_for(token: &Token<'_>, position: &TerminatorSet) -> bool {
        if token.kind == TokenKind::Eof {
            return true;
        }
        if token.newline_before && position.newline {
            return true;
        }
        if matches!(
            token.kind,
            TokenKind::RParen | TokenKind::RBrack | TokenKind::RBrace
        ) {
            // Any closer below depth 0 terminates, set or not.
            return true;
        }
        if let Some(byte) = token.text.as_bytes().first()
            && position.bytes.contains(byte)
        {
            return true;
        }
        if position.arrow && token.kind == TokenKind::Arrow {
            return true;
        }
        token.kind == TokenKind::Ident && position.keywords.contains(&token.text)
    }

    fn cel_from(&mut self, prefix_start: usize, position: &TerminatorSet) -> CelExpr {
        if let Some(token) = &self.peeked
            && Self::boundary_for(token, position)
        {
            // The prefix IS the whole expression; the boundary
            // token stays buffered for the host.
            let end = token.span.range().start.max(prefix_start);
            let source = self.lexer.source_from(0);
            let end = prefix_start
                + source[prefix_start..end]
                    .trim_end_matches([' ', '\t', '\r', '\n'])
                    .len();
            return CelExpr {
                text: String::from(&source[prefix_start..end]),
                span: Span::new(self.source_id, prefix_start..end),
            };
        }
        // A dropped peek is re-read through the immutable prefix
        // slice; `(`/`[`/`{` left the cursor past the opener — the
        // continuation starts one level deep.
        let depth = match self.peeked.take().map(|token| token.kind) {
            Some(TokenKind::LParen | TokenKind::LBrack | TokenKind::LBrace) => 1,
            _ => 0,
        };
        let token = self
            .lexer
            .cel_expr_from(prefix_start, depth, position, self.bag);
        self.last_end = token.span.range().end;
        CelExpr {
            text: String::from(token.text),
            span: token.span,
        }
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
        // everything through the RAW skippers: a dead form may
        // carry bytes the host lexer refuses, and the migration
        // must stay the only voice (D-2026-08-12-1, review round).
        if self.lexer.significant_on_line().map(|(_, byte)| byte) == Some(b'(') {
            self.lexer.skip_balanced_raw(b'(', b')');
        }
        if self.lexer.significant_on_line().map(|(_, byte)| byte) == Some(b'{') {
            self.lexer.skip_balanced_raw(b'{', b'}');
        }
        Value::Error(token.span)
    }

    fn tagged_value(&mut self, token: Token<'_>) -> Value {
        let tag = Name {
            text: String::from(token.text),
            span: token.span,
        };
        let mut args = Vec::new();
        let mut parens = false;
        let mut trailing_comma = false;
        if self.eat(TokenKind::LParen).is_some() {
            parens = true;
            loop {
                let next = self.peek();
                if matches!(next.kind, TokenKind::RParen | TokenKind::Eof) {
                    break;
                }
                trailing_comma = false;
                // A BARE string token followed by `,` or `)` is the
                // grammar's STRING — kept apart from CEL so wrapped
                // or concatenated spellings cannot impersonate it.
                if next.kind == TokenKind::Str {
                    let raw = String::from(next.text);
                    let span = next.span;
                    let start = span.range().start;
                    self.bump();
                    if matches!(self.peek().kind, TokenKind::Comma | TokenKind::RParen) {
                        args.push(Value::StrLit { raw, span });
                    } else {
                        // Part of a larger expression: the string is
                        // the CEL prefix, and the continuation
                        // re-reads the immutable slice — forward
                        // only, never a cursor jump back.
                        args.push(Value::Cel(self.cel_from(start, &PAREN_POSITION)));
                    }
                } else {
                    args.push(self.value(&PAREN_POSITION));
                }
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
                trailing_comma = true;
            }
            self.expect(TokenKind::RParen, "`)`");
        }
        let body = if self.peek().kind == TokenKind::LBrace {
            if RAW_TAGS.contains(&tag.text.as_str()) {
                self.bump();
                let raw = self.lexer.raw_block(self.bag);
                self.last_end = raw.span.range().end;
                Some(TaggedBody::Raw {
                    text: String::from(raw.text),
                    span: raw.span,
                })
            } else {
                self.struct_body().map(TaggedBody::Struct)
            }
        } else {
            None
        };
        if !parens && args.is_empty() && body.is_none() {
            // A WRITTEN `()` is a form — the checker owns its
            // specialized refusal; only a tag with nothing at all
            // is the parser's fault.
            self.error_expected("arguments or a block");
        }
        Value::Tagged(TaggedValue {
            tag,
            args,
            parens,
            trailing_comma,
            body,
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
                            let key = CelExpr {
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

    #[test]
    fn json_recovery_keeps_the_sibling_in_the_tree() {
        // The FAULTY source itself: the first emission is the
        // recovery form, the SECOND statement survives in the AST —
        // not merely a healed twin (D-2026-08-12-1, review round).
        let mut bag = Bag::new();
        let tree = parse(
            SourceId::new(0),
            "graph g : App {\n  operator o {\n    on in { out = json {\"a\": ???}; out = data }\n  }\n  . -> o -> .\n}\n",
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
        assert!(matches!(value, Value::Cel(expr) if expr.text == "data"));
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
        // D-2026-08-13-1 §7: value ::= … | IDENT ident_tail, decided
        // by ONE raw byte probe. Every (prefix × follower) pair lands
        // in exactly the family the grammar names — no tag greed, no
        // struct misread, no second consumer.
        for (label, value, want) in [
            // Ordinary words: always CEL, whatever follows.
            ("word alone", "data", "cel"),
            ("word + operator", "data + 1", "cel"),
            ("word + call parens", "size(data)", "cel"),
            ("word + brace", "data { }", "cel"),
            // Tag words: a tag only WITH its opener. (`duration`,
            // `decimal`, … are CEL functions, not host tags.)
            ("tag + parens", "bounded(64)", "tagged"),
            ("tag alone", "bounded", "cel"),
            ("tag + operator", "bounded + 1", "cel"),
            ("cel function call", "duration(\"5s\")", "cel"),
            // Dead and reserved tags: one refusal, forward recovery.
            ("dead tag + parens", "exponential(1, 2)", "error"),
            ("dead tag + brace", "measure { x = 1 }", "error"),
            ("reserved format + brace", "avro { x }", "error"),
            // `json`: a document, or CEL's word.
            ("json + object", "json {\"a\": 1}", "json"),
            ("json + array", "json [1]", "json"),
            // `{` and `[` are json's ONLY special followers
            // (D-2026-08-13-1, review round): every other spelling —
            // a former scalar included — is CEL beginning with the
            // word `json`, judged by CEL's own rules.
            ("json + scalar spelling", "json 1", "cel"),
            ("json alone", "json", "cel"),
            ("json + parens", "json(1)", "cel"),
            ("json + alien byte", "json @", "cel"),
        ] {
            let (value, bag) = first_value(value);
            let got = match &value {
                Value::Cel(_) => "cel",
                Value::Tagged(_) => "tagged",
                Value::Json { .. } => "json",
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
    fn a_crossing_comment_leaves_the_bare_word() {
        // D-2026-08-13-1 §6: a block comment crossing the line means
        // NO body on the line — the value is the word `json` itself,
        // span exact, and the comment stays HOST trivia. What follows
        // on the next line is a NEW statement.
        let source = "graph g : App {\n  operator o {\n    on in { out = json /* c\n*/ 1 }\n  }\n  . -> o -> .\n}\n";
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), source, &mut bag);
        assert!(bag.diagnostics().is_empty(), "{bag:?}");
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
        assert_eq!(statements.len(), 2, "the next line is a NEW statement");
        let Statement::Assign {
            value: Value::Cel(expr),
            ..
        } = &statements[0]
        else {
            panic!("first statement: {:?}", statements[0]);
        };
        assert_eq!(expr.text, "json", "the bare word alone");
        assert_eq!(
            expr.span.range(),
            source.find("json").unwrap()..source.find("json").unwrap() + 4,
            "span exact"
        );
        let Statement::Yield(Value::Cel(next)) = &statements[1] else {
            panic!("second statement: {:?}", statements[1]);
        };
        assert_eq!(next.text, "1");
    }

    #[test]
    fn an_unterminated_comment_is_the_trivia_authoritys() {
        // D-2026-08-13-1 §6: `json /* never closed` — the probe finds
        // no body, the value is the bare word, and the unterminated
        // comment is diagnosed by the ONE trivia authority, never by
        // a JSON or CEL reader.
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
    fn the_statement_factoring_keeps_the_boundary_token() {
        // D-2026-08-13-1 (review round): after `IDENT path_tail`
        // discovers a bare value, the buffered boundary token (`}`,
        // `;`, or the next line's first token) belongs to the HOST —
        // dropping it lost handler closers and absorbed siblings.
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
                assert!(
                    bag.diagnostics().is_empty(),
                    "{expression} {label}: {bag:?}"
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
                let Statement::Yield(Value::Cel(expr)) = &statements[0] else {
                    panic!("{expression} {label}: a yield: {:?}", statements[0]);
                };
                assert_eq!(expr.text, expression, "{label}: the exact expression");
            }
        }
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
        let Value::Cel(cel) = &command.value else {
            panic!("expected a CEL value");
        };
        assert_eq!(cel.text, "\"cat\"");
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
        let tree = parse_clean("val j = jq { .items | {count: length} }\n");
        let TopDecl::Val(val) = &tree.decls[0] else {
            panic!("expected a val");
        };
        let Value::Tagged(tagged) = &val.value else {
            panic!("expected a tagged value");
        };
        assert_eq!(tagged.tag.text, "jq");
        let Some(TaggedBody::Raw { text, .. }) = &tagged.body else {
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
        let source = "graph g : App {\n  operator o {\n    on in {\n      val doubled = data.n * 2\n      (out, audit) = (envelope, envelope)\n      err = PaymentDeclined { transient = false }\n      count + doubled\n      stop\n    }\n  }\n}\n";
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
        let Statement::ErrEmit { kind, body } = &statements[2] else {
            panic!("expected the typed-error emission");
        };
        assert_eq!(kind.text, "PaymentDeclined");
        assert!(body.is_some());
        let Statement::Yield(Value::Cel(cel)) = &statements[3] else {
            panic!("expected the implicit yield");
        };
        assert_eq!(cel.text, "count + doubled");
        assert!(matches!(&statements[4], Statement::Stop(None)));
    }

    #[test]
    fn cel_openings_never_meet_the_host_lexer() {
        // `!`, a single quote, and `-(` are not Rastok tokens, but they
        // open valid CEL: they must reach the CEL chain untouched.
        let source = "graph g : App {\n  operator o {\n    val a = !false\n    val b = 'hello'\n    val c = -(1)\n    on in {\n      !done\n    }\n  }\n}\n";
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
                    Value::Cel(cel) => cel.text.as_str(),
                    other => panic!("expected CEL, got {other:?}"),
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
            Statement::Yield(Value::Cel(cel)) if cel.text == "!done"
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
        assert!(matches!(&val.value, Value::Cel(cel) if cel.text == "!false"));
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
