//! Source-faithful syntax tree for `.rkg` files.
//!
//! The tree mirrors the productions of `grammar.md` with sugar intact:
//! nothing is desugared, resolved, or typed here — that is the HIR's
//! job. Error nodes mark holes left by parse recovery; a tree holding
//! one is never lowered. Rastok expressions and guest bodies are opaque
//! spans of text: the host tree never interprets them.
//!
//! Nodes are owned plain data with public fields; spans ride along for
//! diagnostics and are never part of any canonical identity.

use crate::diagnostics::Span;

/// Maximum nested match decisions on one source/HIR path. The v16 plan gate
/// independently rechecks this bound before recursive evaluation.
pub const MAX_DISPATCH_DEPTH: usize = 32;

/// Spanned word: an identifier, keyword, or tag, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// The word, exactly as in the source.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

/// Spanned integer literal, kept as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntLit {
    /// The literal text, sign included.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

/// Opaque Rastok expression span, handed untouched to the expression chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprSource {
    /// The expression text, exactly as delimited.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

/// Hole left where parsing failed and recovery resynchronized. A tree
/// containing one never produces a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorNode {
    /// The skipped region.
    pub span: Span,
}

/// One parsed `.rkg` file: `file ::= top_decl*`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyntaxTree {
    /// Top-level declarations in source order.
    pub decls: Vec<TopDecl>,
}

/// Top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum TopDecl {
    /// `type Name = <type_ref>` — a UNIT-level structural alias
    /// (D-2026-08-10-8): sugar the checker expands; never nominal.
    Type(TypeDecl),
    /// `graph … { … }`, including the `: App` entry graph.
    Graph(GraphDecl),
    /// `val … = …`
    Val(ValDecl),
    /// `lab … { … }` — the simulation surface.
    Lab(LabDecl),
    /// Parse-recovery hole.
    Error(ErrorNode),
}

/// `lab IDENT { (simulation | experiment | theorem)* }` — the
/// logical test corpus of the App sharing its name.
#[derive(Debug, Clone, PartialEq)]
pub struct LabDecl {
    /// The lab name, binding to the App of the same name.
    pub name: Name,
    /// The simulations; experiments and theorems are refused at
    /// parse in this build.
    pub simulations: Vec<SimulationDecl>,
    /// The whole declaration.
    pub span: Span,
}

/// `simulation IDENT on graph_ref { (mock | scenario)* }`.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationDecl {
    /// The simulation name.
    pub name: Name,
    /// The `on` target segments (`app`, or a dotted sub-graph path).
    pub target: Vec<Name>,
    /// Simulation-level mocks, shared by every scenario.
    pub mocks: Vec<MockDecl>,
    /// The scenarios, in source order.
    pub scenarios: Vec<ScenarioDecl>,
    /// The whole declaration.
    pub span: Span,
}

/// `mock IDENT ("." IDENT)* { handler* }` — a scripted stand-in
/// whose body is ordinary handler grammar.
#[derive(Debug, Clone, PartialEq)]
pub struct MockDecl {
    /// The mocked connector's path segments.
    pub path: Vec<Name>,
    /// The reply branches, first match in text order.
    pub handlers: Vec<HandlerDecl>,
    /// The whole declaration.
    pub span: Span,
}

/// `scenario IDENT STRING? { … }` — one executable case.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioDecl {
    /// The scenario name.
    pub name: Name,
    /// The optional human label.
    pub label: Option<String>,
    /// The steps and scenario-level mocks, in source order.
    pub body: Vec<ScenarioItem>,
    /// The whole declaration.
    pub span: Span,
}

/// One entry of a scenario body.
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioItem {
    /// `<port_ref> = <value> (seq INT)?` — the world sends.
    Injection {
        /// The injection target (`app.in` in this build).
        target: LValue,
        /// The injected envelope value.
        value: Value,
        /// The explicit inbound sequence, when written.
        seq: Option<IntLit>,
    },
    /// `advance <expr duration>` — the discrete sweep.
    Advance {
        /// The constant duration expression.
        expr: ExprSource,
        /// The step's span.
        span: Span,
    },
    /// `expect …` — pure observation of the current window.
    Expect(ExpectStep),
    /// A scenario-level mock override.
    Mock(MockDecl),
}

/// `expect STRING? nothing? port_ref (count cmp INT)? (where expr)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectStep {
    /// The optional report label.
    pub label: Option<String>,
    /// The void quantifier: `expect nothing …`.
    pub nothing: bool,
    /// The observed port reference.
    pub target: LValue,
    /// The exact-count quantifier, when written.
    pub count: Option<(CmpOp, IntLit)>,
    /// The predicate over each delivery.
    pub predicate: Option<ExprSource>,
    /// The whole step.
    pub span: Span,
}

/// A count comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

/// `graph IDENT ( ":" annotation )? removed_per? "{" body "}"`
#[derive(Debug, Clone, PartialEq)]
pub struct GraphDecl {
    /// The declared instance name.
    pub name: Name,
    /// `+`-joined stdlib symbols; classification is the checker's job.
    pub annotation: Option<Annotation>,
    /// Recovery-only capture of the unpublished `per` spelling.
    pub removed_per: Option<RemovedPerClause>,
    /// Body items in source order.
    pub body: Vec<GraphItem>,
    /// The whole declaration.
    pub span: Span,
}

/// `: IDENT ( "+" IDENT )*` — structural Connector Contract, when present,
/// followed by Extension symbols in written order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// The symbols, in written order.
    pub symbols: Vec<Name>,
}

/// One item of a graph body.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphItem {
    /// `input x` / `output x`
    Port(PortDecl),
    /// Recovery-only capture of `partition IDENT { … }`.
    RemovedPartition(RemovedPartitionDecl),
    /// `operator IDENT … { … }`
    Operator(OperatorDecl),
    /// `connector IDENT : T … { … }`
    Connector(ConnectorDecl),
    /// `val …`
    Val(ValDecl),
    /// Recovery-only capture of Graph-private state, which belongs in an
    /// Operator. Retained so the checker can name the owning rule.
    Var(VarDecl),
    /// Wiring: `a -> b.in`, chains, branches, `<->`.
    Edge(EdgeDecl),
    /// Nested `graph … { … }`.
    Graph(GraphDecl),
    /// `defaults { … }`
    Defaults(StructBody),
    /// Graph-level attribute, Extension attributes included.
    Attribute(Attribute),
    /// Historical v15 Graph handler, retained for staged migration only.
    Handler(HandlerDecl),
    /// Parse-recovery hole.
    Error(ErrorNode),
}

/// `operator IDENT ( ":" annotation )? removed_per? { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct OperatorDecl {
    /// The declared instance name.
    pub name: Name,
    /// Optional Extensions attached to the structural Operator.
    pub annotation: Option<Annotation>,
    /// Recovery-only capture of the unpublished `per` spelling.
    pub removed_per: Option<RemovedPerClause>,
    /// Body items in source order.
    pub body: Vec<OperatorItem>,
    /// The whole declaration.
    pub span: Span,
}

/// `connector IDENT ":" annotation removed_per? { body }` — the annotation is
/// syntactically mandatory: an external boundary is explicit.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorDecl {
    /// The declared instance name.
    pub name: Name,
    /// The mandatory Connector Contract, plus optional Extensions.
    pub annotation: Annotation,
    /// Recovery-only capture of the unpublished `per` spelling.
    pub removed_per: Option<RemovedPerClause>,
    /// Body items in source order.
    pub body: Vec<OperatorItem>,
    /// The whole declaration.
    pub span: Span,
}

/// One item of an operator or connector body.
#[derive(Debug, Clone, PartialEq)]
pub enum OperatorItem {
    /// `input x` / `output x`
    Port(PortDecl),
    /// `val …`
    Val(ValDecl),
    /// `var …`
    Var(VarDecl),
    /// `on … { … }`
    Handler(HandlerDecl),
    /// `name = value`, dotted keys included.
    Attribute(Attribute),
    /// `env { … }` and other named sub-blocks.
    SubBlock(SubBlock),
    /// Parse-recovery hole.
    Error(ErrorNode),
}

/// Port direction keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// `input`
    Input,
    /// `output`
    Output,
}

/// `( "input" | "output" ) IDENT ( ":" type_ref )?`
#[derive(Debug, Clone, PartialEq)]
pub struct PortDecl {
    /// Declared direction.
    pub direction: PortDirection,
    /// The port name.
    pub name: Name,
    /// Optional pinned type; absent means inferred.
    pub ty: Option<TypeRef>,
    /// The whole declaration.
    pub span: Span,
}

/// `val IDENT ( ":" type_ref )? "=" value` — immutable by position.
#[derive(Debug, Clone, PartialEq)]
pub struct ValDecl {
    /// The binding name.
    pub name: Name,
    /// Optional pinned type.
    pub ty: Option<TypeRef>,
    /// The bound value.
    pub value: Value,
    /// The whole declaration.
    pub span: Span,
}

/// `var IDENT ":" type_ref ( "=" value )?` — mutable operator state.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// The state cell name.
    pub name: Name,
    /// The mandatory state type.
    pub ty: TypeRef,
    /// Optional initializer.
    pub value: Option<Value>,
    /// The whole declaration.
    pub span: Span,
}

/// Recovery-only syntax for the unpublished
/// `partition IDENT { key = … ; lifetime = … }` form.
#[derive(Debug, Clone, PartialEq)]
pub struct RemovedPartitionDecl {
    /// The partition name.
    pub name: Name,
    /// Body fields in source order.
    pub fields: Vec<RemovedPartitionField>,
    /// The whole declaration.
    pub span: Span,
}

/// One field retained only to consume a removed partition body exactly once.
#[derive(Debug, Clone, PartialEq)]
pub enum RemovedPartitionField {
    /// `key = <expr_token>`
    Key(ExprSource),
    /// `lifetime = <lifetime_value>`
    Lifetime(RemovedLifetimeValue),
}

/// The removed partition body's lifetime vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub enum RemovedLifetimeValue {
    /// Bare duration value — idle-ttl.
    Expr(ExprSource),
    /// `persistent`
    Persistent(Span),
    /// `single_pass`
    SinglePass(Span),
    /// `fixed <duration>`
    Fixed(ExprSource),
    /// `window <duration> (tumbling | sliding)`
    Window {
        /// The window duration.
        duration: ExprSource,
        /// The window mode.
        mode: RemovedWindowMode,
    },
}

/// Window advancement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovedWindowMode {
    /// `tumbling`
    Tumbling,
    /// `sliding`
    Sliding,
}

/// Recovery-only capture of a named or inline unpublished `per` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum RemovedPerClause {
    /// `per a, b` — references to declared partitions; a tuple when
    /// several.
    Named(Vec<Name>),
    /// `per { … }` / `per (key = …)` — private anonymous partition.
    Inline(Vec<RemovedPartitionField>),
}

/// `on trigger guard* dispatch?` — a missing dispatch is retained only for
/// recovery so the checker can give one named refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct HandlerDecl {
    /// What fires the handler.
    pub trigger: Trigger,
    /// Guard clauses in written order.
    pub guards: Vec<Guard>,
    /// The handler's recursive dispatch, or `None` for recovery.
    pub dispatch: Option<Dispatch>,
    /// The whole handler.
    pub span: Span,
}

/// Source-faithful handler decision tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatch {
    /// A rectilinear statement block.
    Linear {
        /// Statements in authored order.
        statements: Vec<Statement>,
        /// Complete block or single-statement span.
        span: Span,
    },
    /// A closed (`match expr`) or open (`match`) decision. The checker owns
    /// the distinction because it has the scrutinee type.
    Match {
        /// Pure `val` declarations before the terminal match in a block.
        prefix: Vec<Statement>,
        /// Present for a closed match; absent for ordered predicates.
        scrutinee: Option<ExprSource>,
        /// Arms in source order. Closed arms become canonical in HIR.
        arms: Vec<MatchArm>,
        /// Complete match span, including a containing block when present.
        span: Span,
    },
}

impl Dispatch {
    /// Complete source span.
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Linear { span, .. } | Self::Match { span, .. } => *span,
        }
    }

    /// Statements of a source-linear dispatch.
    #[must_use]
    pub fn as_linear(&self) -> Option<&[Statement]> {
        match self {
            Self::Linear { statements, .. } => Some(statements),
            Self::Match { .. } => None,
        }
    }
}

/// One source match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// Closed cases, an open predicate, or the `_` fallback.
    pub selector: MatchSelector,
    /// Nested arm dispatch.
    pub dispatch: Box<Dispatch>,
    /// Complete arm span.
    pub span: Span,
}

/// Source selector of one match arm.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchSelector {
    /// One or more closed symbolic cases joined with `|`.
    Cases(Vec<Name>),
    /// An ordered open predicate.
    Predicate(ExprSource),
    /// `_`, the exact remaining domain.
    Fallback(Span),
}

/// Syntactic handler trigger, as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// `on start`
    Start(Span),
    /// `on stop`
    Stop(Span),
    /// `on compensate`
    Compensate(Span),
    /// `on <Extension>.start` / `on <Extension>.stop`
    Extension {
        /// The qualifying historical Extension name.
        extension: Name,
        /// The qualified lifecycle event.
        event: ExtensionEvent,
    },
    /// `on <child>.<output>` — an output emitted by a direct child of the
    /// lexical Graph. This form is meaningful only for Graph-owned handlers;
    /// ordinary Nodes still activate on their own input ports.
    ChildPort {
        /// Direct child declaration.
        child: Name,
        /// Output port on that child.
        port: Name,
    },
    /// `on (a, b)` — presence join.
    Join(Vec<Name>),
    /// `on <port>`
    Port(Name),
}

/// Recovery-only capture of a removed Extension-qualified lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionEvent {
    /// `.start`
    Start,
    /// `.stop`
    Stop,
}

/// One guard clause of a handler.
#[derive(Debug, Clone, PartialEq)]
pub enum Guard {
    /// `where <expr_token>` — content predicate.
    Where(ExprSource),
    /// `having <expr_token>` — aggregate predicate.
    Having(ExprSource),
    /// `by <expr_token>` — join-key override.
    By(ExprSource),
    /// `within <expr_token>` — join time window.
    Within(ExprSource),
    /// `last INT` — join count bound.
    Last(IntLit),
}

/// One statement of a handler body.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `val …` — handler-local binding.
    Val(ValDecl),
    /// `lvalue ( ":" type_ref )? "=" value`
    Assign {
        /// The assignment target.
        target: LValue,
        /// Optional pinned type.
        ty: Option<TypeRef>,
        /// The assigned value.
        value: Value,
    },
    /// `(a, b) = …` — atomic multi-emit, tuple or broadcast.
    MultiEmit {
        /// The targets, in written order.
        targets: Vec<LValue>,
        /// One value per target, or one broadcast value.
        values: Vec<Value>,
    },
    /// `Loop.next = value` — a temporal-manager command carrying new state.
    LoopNext {
        /// The next entry data.
        value: Value,
        /// Complete statement span.
        span: Span,
    },
    /// `Loop.again` — retain the current Loop state.
    LoopAgain(Span),
    /// `Loop.done` — close the current Loop run without a business envelope.
    LoopDone(Span),
    /// Explicit no-op dispatch leaf.
    Nothing(Span),
    /// `stop IDENT?` — lifecycle action.
    Stop(Option<Name>),
    /// Parse-recovery hole.
    Error(ErrorNode),
}

/// Dotted assignment target or reference path, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LValue {
    /// The path segments; reserved words may appear after the first.
    pub segments: Vec<Name>,
    /// The whole path.
    pub span: Span,
}

/// A value position: builders by prefix, Rastok expression for everything scalar.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Opaque Rastok expression.
    Expr(ExprSource),
    /// `.` — exact re-emission of the current envelope (identity).
    Identity(Span),
    /// `{ … }` — a braced body, the record literal in every value
    /// position. In ENVELOPE positions the CHECKER classifies it
    /// once: no reserved root at the top level builds a data record;
    /// only reserved roots (`data`, `meta`) build or
    /// patch the envelope; a mix refuses with a dedicated
    /// diagnostic. Elsewhere the body is simply a record.
    Braced(StructBody),
    /// `{ "k": v, … }` — the map literal, decided by its `:`
    /// punctuation; `{:}` is the empty map (D-2026-08-10-6). Keys
    /// keep their SOURCE spelling and span: the checker decodes them
    /// through the ONE string rule (the Rastok expression language's) and owns duplicates.
    BracedMap(Vec<MapEntry>),
    /// `nothing` — emit nothing.
    Nothing(Span),
    /// The parser's RECOVERY form: a diagnostic was already pushed
    /// for this span, and the checker stays silent — one fault, one
    /// voice. Never a user state: a genuinely empty expression
    /// still diagnoses (D-2026-08-12-1, review round).
    Error(Span),
    /// `IDENT!` followed by the region owned by the registered frontend.
    Frontend(FrontendValue),
    /// `Case { field = value, … }` — a contextually typed tagged-union
    /// constructor. The parser records only the case spelling and payload;
    /// the checker resolves the one compatible union and seals it into HIR.
    Tagged {
        /// Semantic case name.
        case: Name,
        /// Source payload fields.
        body: StructBody,
    },
}

/// One `"key": value` entry of a braced map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEntry {
    /// The key as WRITTEN — quotes and escapes included; decoded at
    /// check by the Rastok expression string rule.
    pub key: ExprSource,
    /// The key's location.
    pub span: Span,
    /// The entry's value.
    pub value: Value,
}

/// `IDENT "!" ( "(" args ")" )? FRONTEND_REGION`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontendValue {
    /// The ordinary identifier selecting the registered frontend.
    pub name: Name,
    /// Parenthesized arguments, possibly empty.
    pub args: Vec<Value>,
    /// Whether a parenthesis list was WRITTEN — `base64!()` and
    /// `base64!` differ, and the frontend contract knows which forms admit
    /// which (D-2026-08-10-6).
    pub parens: bool,
    /// The complete region returned by the selected frontend reader/framer.
    pub payload: FrontendPayload,
}

/// One complete region owned by a tagged frontend.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontendPayload {
    /// One complete JSON value, already read by the canonical house reader.
    Json {
        /// The parsed value — exact numbers, duplicate keys refused.
        value: rastok_core::value::Value,
        /// Where the JSON bytes were written.
        span: Span,
    },
    /// Raw frontend text, never tokenized by the host parser.
    Raw {
        /// The region bytes, braces excluded.
        text: String,
        /// Where the region bytes were written.
        span: Span,
    },
}

/// `{ attr_or_block ( ","? attr_or_block )* }`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructBody {
    /// Entries in source order.
    pub entries: Vec<StructEntry>,
}

/// One entry of a struct-shaped body.
#[derive(Debug, Clone, PartialEq)]
pub enum StructEntry {
    /// `key = value`, dotted keys allowed.
    Field {
        /// The dotted key.
        key: LValue,
        /// The value.
        value: Value,
    },
    /// `name { … }` nested sub-block.
    Block {
        /// The block name.
        name: Name,
        /// The nested body.
        body: StructBody,
    },
}

/// `name = value` at declaration level.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The dotted attribute key.
    pub target: LValue,
    /// The attribute value.
    pub value: Value,
    /// The whole attribute.
    pub span: Span,
}

/// `IDENT { … }` named sub-block inside a node body.
#[derive(Debug, Clone, PartialEq)]
pub struct SubBlock {
    /// The block name.
    pub name: Name,
    /// The block body.
    pub body: StructBody,
    /// The whole block.
    pub span: Span,
}

/// One wiring declaration; chains generalize the single edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeDecl {
    /// Two or more endpoints joined by arrows, in written order.
    pub endpoints: Vec<EdgeEndpoint>,
    /// `<->` instead of `->`; syntactically a two-endpoint form.
    pub bidirectional: bool,
    /// `when <expr_token>` boolean edge guard.
    pub guard: Option<ExprSource>,
    /// `{ … }` trailing edge configuration.
    pub config: Option<StructBody>,
    /// The whole declaration.
    pub span: Span,
}

/// One endpoint of an edge.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeEndpoint {
    /// A port reference.
    Ref(PortRef),
    /// `{ a ; b }` in source position: independent mpsc producers.
    SourceSet(Vec<PortRef>),
    /// `{ a ; b }` in destination position: one atomic delivery set.
    DestinationSet(Vec<EdgeDestination>),
    /// Closed case routing after an arrow.
    Match(EdgeMatch),
    /// `nothing` — explicit discard sink.
    Nothing(Span),
}

/// One non-routing destination inside a destination set or route arm.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeDestination {
    /// Ordinary endpoint or a qualified manager command, resolved later.
    Ref(PortRef),
    /// Explicit discard. A set mixing it with another destination is rejected
    /// by the checker with `E-ROUTE-MIXED`.
    Nothing(Span),
}

/// Source-faithful closed routing table.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeMatch {
    /// Arms in source order; the checker canonicalizes them by case.
    pub arms: Vec<EdgeMatchArm>,
    /// Complete `match { ... }` span.
    pub span: Span,
}

/// One case route.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeMatchArm {
    /// Explicit cases or the `_` remainder.
    pub cases: EdgeCaseSet,
    /// One simple or grouped destination. Nested route matches are not part of
    /// the grammar.
    pub destination: Box<EdgeEndpoint>,
    /// Complete arm span.
    pub span: Span,
}

/// Closed route selector.
#[derive(Debug, Clone, PartialEq)]
pub enum EdgeCaseSet {
    /// One or more symbolic cases joined by `|`.
    Cases(Vec<Name>),
    /// Exact remainder of the source's productible cases.
    Fallback(Span),
}

/// `node`, `node.port`, a composed path, or the graph boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRef {
    /// Whether the reference starts at the graph's own boundary
    /// (leading `.`): bare `.` has no segments, `.foo` has one.
    pub boundary: bool,
    /// The path segments after the anchor.
    pub segments: Vec<Name>,
    /// The whole reference.
    pub span: Span,
}

/// `type_ref ::= union_type "?"? data_annotation*`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRef {
    /// The `|`-joined alternatives, one when not a union.
    pub alternatives: Vec<BaseType>,
    /// Trailing `?` — shorthand for `| null`.
    pub optional: bool,
    /// Data-policy annotations written on this exact type position.
    pub annotations: Vec<SchemaAnnotation>,
    /// The whole type expression.
    pub span: Span,
}

/// One `@name(...)` annotation on a schema position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaAnnotation {
    /// Source annotation name, without the `@` sigil.
    pub name: Name,
    /// Named constant arguments in written order.
    pub arguments: Vec<SchemaAnnotationArgument>,
    /// The complete annotation span.
    pub span: Span,
}

/// One named constant argument of a schema annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaAnnotationArgument {
    /// Argument name.
    pub name: Name,
    /// Constant source value.
    pub value: SchemaAnnotationConstant,
}

/// Closed constant vocabulary accepted by schema annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaAnnotationConstant {
    /// Dotted symbolic constant such as `gen_ai.usage.input_tokens`.
    Symbol(Vec<Name>),
    /// String constant, with quotes stripped and source escapes retained.
    Text {
        /// Source content between the quotes.
        text: String,
        /// Source span including quotes.
        span: Span,
    },
    /// Integer constant, preserved as written.
    Integer(IntLit),
    /// A syntactically complete expression retained so the checker can reject
    /// dynamic annotation arguments with the annotation diagnostic family,
    /// rather than cascading from the host parser.
    Dynamic(ExprSource),
}

/// One base type of a union.
#[derive(Debug, Clone, PartialEq)]
pub enum BaseType {
    /// A scalar physical or logical type.
    Scalar(ScalarType),
    /// `variant` — the dynamic top type.
    Variant,
    /// `struct { a: T, .. }` — typed record, possibly open.
    Struct {
        /// The declared fields, in written order.
        fields: Vec<FieldDecl>,
        /// Whether `..` closes the record as open.
        open: bool,
    },
    /// `map<K, V>`
    Map(Box<TypeRef>, Box<TypeRef>),
    /// `array<T>`
    Array(Box<TypeRef>),
    /// `table<T>` — alias for `array<struct>`.
    Table(Box<TypeRef>),
    /// `enum { A, B }`
    Enum(Vec<Name>),
    /// `union { Case { field: T }, UnitCase }`
    TaggedUnion(Vec<TaggedCaseDecl>),
    /// A named type, schema, or declared enum/union.
    Named(Name),
}

/// One named case of a source tagged union.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedCaseDecl {
    /// Semantic case name.
    pub name: Name,
    /// Data-policy annotations on the tagged-union case itself.
    pub annotations: Vec<SchemaAnnotation>,
    /// Closed payload fields; an empty list is the unit case.
    pub fields: Vec<FieldDecl>,
}

/// `type Name = <type_ref>` (D-2026-08-10-8).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    /// The alias name.
    pub name: Name,
    /// The aliased structural type.
    pub ty: TypeRef,
    /// The whole declaration.
    pub span: Span,
}

/// One field of a struct type.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    /// The field name; type words are allowed here.
    pub name: Name,
    /// The field type.
    pub ty: TypeRef,
}

/// The scalar type vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    /// `bool`
    Bool,
    /// `int`
    Int,
    /// `long`
    Long,
    /// `uint`
    Uint,
    /// `float`
    Float,
    /// `double`
    Double,
    /// `number`
    Number,
    /// `bytes`
    Bytes,
    /// `text`
    Text,
    /// `media_type`
    MediaType,
    /// `null`
    Null,
    /// `decimal(p, s)`
    Decimal {
        /// Total digits.
        precision: u32,
        /// Fractional digits.
        scale: u32,
    },
    /// `date`
    Date,
    /// `timestamp`
    Timestamp,
    /// `timestamp_ntz`
    TimestampNtz,
    /// `time`
    Time,
    /// `uuid`
    Uuid,
    /// `duration`
    Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::SourceId;

    fn span(range: std::ops::Range<usize>) -> Span {
        Span::new(SourceId::new(0), range)
    }

    fn name(text: &str, range: std::ops::Range<usize>) -> Name {
        Name {
            text: String::from(text),
            span: span(range),
        }
    }

    #[test]
    fn a_minimal_app_graph_is_representable() {
        // graph m0 : App { . -> shout -> . }
        let tree = SyntaxTree {
            decls: vec![TopDecl::Graph(GraphDecl {
                name: name("m0", 6..8),
                annotation: Some(Annotation {
                    symbols: vec![name("App", 11..14)],
                }),
                removed_per: None,
                body: vec![GraphItem::Edge(EdgeDecl {
                    endpoints: vec![
                        EdgeEndpoint::Ref(PortRef {
                            boundary: true,
                            segments: vec![],
                            span: span(17..18),
                        }),
                        EdgeEndpoint::Ref(PortRef {
                            boundary: false,
                            segments: vec![name("shout", 22..27)],
                            span: span(22..27),
                        }),
                        EdgeEndpoint::Ref(PortRef {
                            boundary: true,
                            segments: vec![],
                            span: span(31..32),
                        }),
                    ],
                    bidirectional: false,
                    guard: None,
                    config: None,
                    span: span(17..32),
                })],
                span: span(0..34),
            })],
        };
        let TopDecl::Graph(graph) = &tree.decls[0] else {
            panic!("expected a graph");
        };
        assert_eq!(graph.annotation.as_ref().unwrap().symbols[0].text, "App");
        assert_eq!(graph.body.len(), 1);
    }

    #[test]
    fn the_braced_body_is_carried_unclassified() {
        // `{ n = 1 }` reaches the checker as its body, uninterpreted
        // — classification happens once, there (D-2026-08-09-1).
        let body = StructBody {
            entries: vec![StructEntry::Field {
                key: LValue {
                    segments: vec![name("n", 2..3)],
                    span: span(2..3),
                },
                value: Value::Expr(ExprSource {
                    text: String::from("1"),
                    span: span(6..7),
                }),
            }],
        };
        // One braced form exists now; the CHECKER classifies it —
        // the syntax only records the body.
        assert_eq!(Value::Braced(body.clone()), Value::Braced(body));
    }

    #[test]
    fn recursive_types_nest_through_boxes() {
        // map<text, array<int>>?
        let ty = TypeRef {
            alternatives: vec![BaseType::Map(
                Box::new(TypeRef {
                    alternatives: vec![BaseType::Scalar(ScalarType::Text)],
                    optional: false,
                    annotations: Vec::new(),
                    span: span(4..8),
                }),
                Box::new(TypeRef {
                    alternatives: vec![BaseType::Array(Box::new(TypeRef {
                        alternatives: vec![BaseType::Scalar(ScalarType::Int)],
                        optional: false,
                        annotations: Vec::new(),
                        span: span(16..19),
                    }))],
                    optional: false,
                    annotations: Vec::new(),
                    span: span(10..20),
                }),
            )],
            optional: true,
            annotations: Vec::new(),
            span: span(0..22),
        };
        assert!(ty.optional);
        let BaseType::Map(_, values) = &ty.alternatives[0] else {
            panic!("expected a map");
        };
        assert!(matches!(values.alternatives[0], BaseType::Array(_)));
    }

    #[test]
    fn error_nodes_are_first_class_holes() {
        let tree = SyntaxTree {
            decls: vec![
                TopDecl::Error(ErrorNode { span: span(0..9) }),
                TopDecl::Val(ValDecl {
                    name: name("x", 14..15),
                    ty: None,
                    value: Value::Expr(ExprSource {
                        text: String::from("1"),
                        span: span(18..19),
                    }),
                    span: span(10..19),
                }),
            ],
        };
        assert!(matches!(tree.decls[0], TopDecl::Error(_)));
        assert!(matches!(tree.decls[1], TopDecl::Val(_)));
    }
}
