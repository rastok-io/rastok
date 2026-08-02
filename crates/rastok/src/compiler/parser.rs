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
    Annotation, AspectEvent, Attribute, BaseType, CelExpr, ConnectorDecl, EdgeDecl, EdgeEndpoint,
    ErrorNode, FieldDecl, GraphDecl, GraphItem, Guard, HandlerDecl, IntLit, LValue, LabStub,
    LifetimeValue, Name, OperatorDecl, OperatorItem, PartitionDecl, PartitionField, PerClause,
    PortDecl, PortDirection, PortRef, ScalarType, Statement, StructBody, StructEntry, SubBlock,
    SyntaxTree, TaggedBody, TaggedValue, TopDecl, Trigger, TypeRef, ValDecl, Value, VarDecl,
    WindowMode,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};

/// Diagnostic for a token that breaks the expected production.
pub const E_PARSE_EXPECTED: Code = Code::new("E-PARSE-EXPECTED");
/// Diagnostic for grammatical syntax outside the implemented subset.
pub const E_PARSE_DEFERRED: Code = Code::new("E-PARSE-DEFERRED");

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
const RAW_TAGS: &[&str] = &["jq", "bash", "sql", "python", "rust", "json"];

/// Tag words that open tagged-value expressions.
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
    "hex",
    "csv",
    "file",
    "oauth2_client_credentials",
    "measure",
    "exponential",
    "linear",
    "constant",
    "fibonacci",
    "on_crash",
    "bounded",
    "unbounded",
    "regex",
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

    /// Hands a peeked token back to the lexer before an opaque scan.
    fn unpeek(&mut self) {
        if let Some(token) = self.peeked.take() {
            self.lexer.rewind(token.span.range().start);
        }
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
        if self.peeked.is_some() {
            self.unpeek();
        } else {
            // Same-line comments before a value are trivia; a bare
            // newline is not — it still reads as an empty expression.
            self.lexer.advance_within_line(self.bag);
        }
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
                        "lab" => decls.push(self.lab_stub(start)),
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

    /// `lab IDENT { … }` — recognized, skipped whole, refused
    /// explicitly until the scenario stage.
    fn lab_stub(&mut self, start: usize) -> TopDecl {
        self.bump();
        let Some(name) = self.name("the lab name") else {
            return TopDecl::Error(self.recover(start));
        };
        if self.expect(TokenKind::LBrace, "`{`").is_none() {
            return TopDecl::Error(self.recover(start));
        }
        let mut depth: u32 = 0;
        loop {
            match self.peek().kind {
                TokenKind::Eof => break,
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {
                    self.bump();
                }
            }
        }
        let span = self.span_from(start);
        self.deferred("lab", span);
        TopDecl::Lab(LabStub { name, span })
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
                // Not an assignment: the path was the start of a bare
                // value. Hand every consumed token back and rescan.
                self.peeked = None;
                self.lexer.rewind(start);
                self.last_end = start;
                Statement::Yield(self.value(&ATTR_POSITION))
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
        // Probe by shape before touching the token stream: CEL text
        // may open with bytes this lexer rejects (`!`, `'`, `-(`), so
        // only builder-shaped input is tokenized here.
        let builder_shaped = matches!(self.shape(), Some(b'{' | b'a'..=b'z' | b'A'..=b'Z' | b'_'));
        if !builder_shaped {
            return Value::Cel(self.cel(position));
        }
        let token = self.peek();
        match (token.kind, token.text) {
            (TokenKind::LBrace, _) => match self.struct_body() {
                Some(body) => Value::EnvelopeSugar(body),
                None => Value::Cel(CelExpr {
                    text: String::new(),
                    span: token.span,
                }),
            },
            (TokenKind::Ident, "envelope") => {
                self.bump();
                if self.peek().kind == TokenKind::LBrace {
                    match self.struct_body() {
                        Some(body) => Value::Envelope(Some(body)),
                        None => Value::Envelope(None),
                    }
                } else {
                    Value::Envelope(None)
                }
            }
            (TokenKind::Ident, "struct") => {
                self.bump();
                Value::Struct(self.struct_body().unwrap_or_default())
            }
            (TokenKind::Ident, "map") => {
                self.bump();
                Value::Map(self.struct_body().unwrap_or_default())
            }
            (TokenKind::Ident, "nothing") => {
                self.bump();
                Value::Nothing(token.span)
            }
            (TokenKind::Ident, tag) if TAG_WORDS.contains(&tag) => self.tagged_value(),
            _ => Value::Cel(self.cel(position)),
        }
    }

    fn tagged_value(&mut self) -> Value {
        let token = self.bump();
        let tag = Name {
            text: String::from(token.text),
            span: token.span,
        };
        let mut args = Vec::new();
        if self.eat(TokenKind::LParen).is_some() {
            loop {
                if matches!(self.peek().kind, TokenKind::RParen | TokenKind::Eof) {
                    break;
                }
                args.push(self.value(&PAREN_POSITION));
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "`)`");
        }
        let body = if self.peek().kind == TokenKind::LBrace {
            if RAW_TAGS.contains(&tag.text.as_str()) {
                self.bump();
                self.unpeek();
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
        if args.is_empty() && body.is_none() {
            self.error_expected("arguments or a block");
        }
        Value::Tagged(TaggedValue { tag, args, body })
    }

    /// `"{" ( entry ","? )* "}"` — shared by struct-shaped bodies.
    fn struct_body(&mut self) -> Option<StructBody> {
        self.expect(TokenKind::LBrace, "`{`")?;
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
    fn a_lab_is_skipped_with_an_explicit_refusal() {
        let mut bag = Bag::new();
        let source = "lab m0_probe { simulation smoke on app { } }\nval x = 1\n";
        let tree = parse(SourceId::new(0), source, &mut bag);
        assert_eq!(bag.diagnostics().len(), 1);
        assert_eq!(bag.diagnostics()[0].code(), E_PARSE_DEFERRED);
        assert!(matches!(&tree.decls[0], TopDecl::Lab(stub) if stub.name.text == "m0_probe"));
        assert!(matches!(tree.decls[1], TopDecl::Val(_)));
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
      out = envelope { data.text = data.text }
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
        assert!(matches!(value, Value::Envelope(Some(_))));
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
            "graph g : App {\n  output feed\n  o . out -> . feed\n  operator o { on in { out = envelope } }\n}\n",
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
            "graph g : App {\n  output feed\n  o.out -> ./* target */feed\n  operator o { on in { out = envelope } }\n}\n",
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
