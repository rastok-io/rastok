//! Hand-written recursive-descent parser for `.rkg` source.
//!
//! One token of lookahead over [`Lexer`], per the LL(1) contract of
//! `grammar.md`. On a fault the parser reports, resynchronizes at the
//! next declaration keyword or closing brace, leaves an
//! [`ErrorNode`] hole, and keeps parsing — healthy declarations of a
//! faulty file still reach the tree.
//!
//! Handlers, statements, and wiring edges are the next slice: their
//! syntax is recognized and refused explicitly for now. `lab` bodies
//! are skipped whole until the scenario stage.

use crate::compiler::lexer::{Lexer, TerminatorSet, Token, TokenKind};
use crate::compiler::syntax::{
    Annotation, Attribute, BaseType, CelExpr, ConnectorDecl, ErrorNode, FieldDecl, GraphDecl,
    GraphItem, LValue, LabStub, LifetimeValue, Name, OperatorDecl, OperatorItem, PartitionDecl,
    PartitionField, PerClause, PortDecl, PortDirection, ScalarType, StructBody, StructEntry,
    SubBlock, SyntaxTree, TaggedBody, TaggedValue, TopDecl, TypeRef, ValDecl, Value, VarDecl,
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
        self.unpeek();
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
                    "on" => {
                        self.bump();
                        self.deferred("handler", token.span);
                        items.push(GraphItem::Error(self.recover(start)));
                    }
                    _ => items.push(self.graph_attribute_or_edge(start)),
                },
                TokenKind::Dot | TokenKind::LBrace => {
                    self.bump();
                    self.deferred("edge", token.span);
                    items.push(GraphItem::Error(self.recover(start)));
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

    /// An IDENT-led graph item: attribute, or a wiring edge — the
    /// latter still deferred to the next slice.
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
                self.bump();
                self.deferred("edge", token.span);
                GraphItem::Error(self.recover(start))
            }
            _ => {
                self.error_expected("`=` or a wiring arrow");
                GraphItem::Error(self.recover(start))
            }
        }
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
                    "on" => {
                        self.bump();
                        self.deferred("handler", token.span);
                        items.push(OperatorItem::Error(self.recover(start)));
                    }
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
        while self.eat(TokenKind::Dot).is_some() {
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

    #[test]
    fn handlers_and_edges_are_refused_explicitly_for_now() {
        let mut bag = Bag::new();
        let source = "graph g : App {\n  operator o {\n    on in { }\n  }\n  a -> b.in\n}\n";
        parse(SourceId::new(0), source, &mut bag);
        let codes: Vec<_> = bag
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect();
        assert_eq!(codes, [E_PARSE_DEFERRED, E_PARSE_DEFERRED]);
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert_eq!(constructs, ["handler", "edge"]);
    }
}
