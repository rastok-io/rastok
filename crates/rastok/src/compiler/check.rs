//! Resolution and semantic checking: syntax trees in, HIR out.
//!
//! The checker resolves names, desugars default ports and envelope
//! sugar into semantic forms, compiles every admitted Rastok expression,
//! and refuses explicitly — never accidentally — every grammatical
//! construct outside the executable subset of this build
//! (`E-CHECK-DEFERRED`). Rules with a normative home carry their own
//! codes; `meta.sys` writes are the canonical example.

mod dispatch;
mod extensions;
mod loop_control;
mod routing;

use dispatch::*;
use loop_control::*;
use routing::*;

use crate::compiler::hir::{
    AttrValue, Attribute, ClosedArm, ClosingPolicy, Const, Direction, Dispatch, Edge, EdgeTarget,
    EmitValue, Endpoint, EndpointOwner, FieldWrite, Graph, Handler, Hir, InstanceLifetime,
    InstancePolicy, LoopCommand, LoopCommandKind, LoopCommandTarget, LoopDelay, LoopSpec,
    MultiEmitValues, Node, NodeSemantics, OpenArm, RouteArm, RouteDestination, RouteRule,
    RouteSelection, Statement, Val, Var,
};
use crate::compiler::syntax::{
    Attribute as SynAttribute, BaseType, ConnectorDecl, Dispatch as SynDispatch, EdgeCaseSet,
    EdgeDecl, EdgeDestination as SynEdgeDestination, EdgeEndpoint, EdgeMatch, ExprSource,
    GraphDecl, GraphItem, MatchSelector, Name, OperatorDecl, OperatorItem, PortRef, ScalarType,
    SchemaAnnotation as SynSchemaAnnotation, SchemaAnnotationConstant, Statement as SynStatement,
    StructBody, StructEntry, SyntaxTree, TopDecl, Trigger as SynTrigger, TypeRef, ValDecl,
    Value as SynValue, VarDecl,
};
use crate::contract;
use crate::datatype::{
    DataType, DateUnit, DecimalType, EnumType, MapType, StructType, TaggedCase, TaggedUnionType,
    TemporalType, TimeUnit, TypeError, TypeField,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};
use crate::expr;
use crate::node;
use crate::plan::closed_case_domain;
use crate::schema_annotation::{
    AnnotatedSchema, AnnotationSet, AnnotationTable, CostAnnotation, CostBasis, CurrencyCode,
    DataAnnotation, MeasureCarrier, MeasureId, MeasureSource, PersonalDataClass, SchemaPath,
    SchemaPathSegment, SensitivityLevel, is_iso_4217_currency, is_money_type, measure_spec,
};
use std::collections::{BTreeMap, BTreeSet};

/// Diagnostic for a unit without exactly one `: App` graph.
pub const E_APP_COUNT: Code = Code::new("E-APP-COUNT");
/// Diagnostic for a duplicate node name inside one graph.
pub const E_DUP_NODE: Code = Code::new("E-DUP-NODE");
/// A Graph port and child claim the same lexical name.
pub const E_NAME_CLASH: Code = Code::new("E-NAME-CLASH");
/// Diagnostic for a name that resolves to no declaration.
pub const E_RESOLVE_UNKNOWN: Code = Code::new("E-RESOLVE-UNKNOWN");
/// Diagnostic for a port that the addressed node does not expose.
pub const E_PORT_UNKNOWN: Code = Code::new("E-PORT-UNKNOWN");
/// Diagnostic for a write reaching the read-only `meta.sys` namespace.
pub const E_META_SYS_READONLY: Code = Code::new("E-META-SYS-READONLY");
/// A metadata path outside the three zones, or outside a zone's
/// declared vocabulary (D-2026-08-10-1); field `path`.
pub const E_META_ZONE: Code = Code::new("E-META-ZONE");
/// One `meta.app` field written with two incompatible types; both
/// sites carried as spans; field `field`.
pub const E_META_TYPE: Code = Code::new("E-META-TYPE");
/// Two flat keys where one prefixes the other — `audit` vs
/// `audit.origin`; field `field`.
pub const E_META_PREFIX: Code = Code::new("E-META-PREFIX");
/// A reserved word used as a node name (`meta`, `data`, `sys`,
/// `app`, `ext`); field `name`.
pub const E_NAME_RESERVED: Code = Code::new("E-NAME-RESERVED");
/// An `asset(…)` that cannot resolve at build: missing file, root
/// escape, budget breach, or a malformed call; field `path`.
pub const E_ASSET: Code = Code::new("E-ASSET");
/// A named type declared twice in the unit; field `name`.
pub const E_DUP_TYPE: Code = Code::new("E-DUP-TYPE");
/// A named type that reaches itself through its own expansion, or
/// one whose expansion exceeds the structural budgets — the
/// measured value rides the diagnostic; field `name`.
pub const E_TYPE_RECURSIVE: Code = Code::new("E-TYPE-RECURSIVE");
/// A duplicate field or symbol inside one type declaration; fields
/// `name` (the type) and `field` (the duplicate).
pub const E_TYPE_DUP: Code = Code::new("E-TYPE-DUP");
/// A type expansion beyond a structural budget — depth, nodes, or
/// width; the MEASURED value rides the diagnostic; fields `name` +
/// `depth`/`nodes`/`count` (D-2026-08-10-8).
pub const E_TYPE_BUDGET: Code = Code::new("E-TYPE-BUDGET");
/// A structural union whose branches overlap and therefore have no unique
/// selected branch.
pub const E_UNION_OVERLAP: Code = Code::new("E-UNION-OVERLAP");
/// A source type that violates a canonical constructor invariant.
pub const E_TYPE_INVALID: Code = Code::new("E-TYPE-INVALID");
/// An open compiler constraint that reached a position requiring a closed
/// effective type.
pub const E_TYPE_UNRESOLVED: Code = Code::new("E-TYPE-UNRESOLVED");
/// A data annotation is unknown, malformed, duplicated, or targets an
/// incompatible position. Fields: `annotation`, `path`, and `detail`.
pub const E_DATA_ANNOTATION: Code = Code::new("E-DATA-ANNOTATION");
/// A content-typed byte form outside whole-payload position
/// (D-2026-08-10-5); field `tag`.
pub const E_CONTENT_TYPE: Code = Code::new("E-CONTENT-TYPE");

/// A built-in frontend call whose contract and arguments have passed their
/// shape gate; placement and payload decoding follow.
enum FrontendCall {
    Json(crate::value::Value),
    Text,
    Base64 { content_type: Option<String> },
}

/// Why a type declaration refuses — one variant per rule, each with
/// its own code, span, and measured value.
enum TypeRefusal {
    UnknownName { name: String, span: Span },
    AliasBudget { measured: usize },
    Cycle { name: String, span: Span },
    DuplicateField { name: String, span: Span },
    DuplicateSymbol { name: String, span: Span },
    DuplicateCase { name: String, span: Span },
    DepthBudget { measured: usize },
    NodeBudget { measured: usize },
    WidthBudget { what: &'static str, measured: usize },
}

enum DataAnnotationTarget<'a> {
    Type(&'a DataType),
    TaggedUnionCase,
}
/// Diagnostic adapted from the expression chain, span mapped to the source.
pub const E_EXPR: Code = Code::new("E-EXPR");
/// Diagnostic for grammar admitted but outside the executable subset.
pub const E_CHECK_DEFERRED: Code = Code::new("E-CHECK-DEFERRED");
/// Diagnostic for a structural annotation or Extension that is unknown, ordered
/// incorrectly, duplicated, or incompatible with the declaration kind.
pub const E_NODE_ANNOTATION: Code = Code::new("E-NODE-ANNOTATION");
/// Diagnostic for a connector type without a Driver in this build.
pub const E_CONNECTOR_UNAVAILABLE: Code = Code::new("E-CONNECTOR-UNAVAILABLE");
/// Diagnostic for an attribute violating its contract sheet.
pub const E_CONTRACT_ATTR: Code = Code::new("E-CONTRACT-ATTR");
/// Diagnostic for a mandatory contract attribute left undeclared.
pub const E_CONTRACT_REQUIRED: Code = Code::new("E-CONTRACT-REQUIRED");
/// Diagnostic for a binding name that collides with a visible binding
/// or a reserved handler name — shadowing is refused, which is also
/// what makes compile-time constant substitution sound.
pub const E_DUP_BINDING: Code = Code::new("E-DUP-BINDING");
/// Diagnostic for a port used against its direction.
pub const E_PORT_DIRECTION: Code = Code::new("E-PORT-DIRECTION");
/// Diagnostic for a port name declared or inferred with conflicting
/// roles.
pub const E_PORT_CONFLICT: Code = Code::new("E-PORT-CONFLICT");

/// A value crossing a port whose PINNED type it cannot satisfy — an
/// emission, an injection, or the two ends of an edge disagreeing.
pub const E_PORT_TYPE: Code = Code::new("E-PORT-TYPE");

/// A Node's `instance` or `lifetime` attribute is malformed, duplicated, or
/// has no canonical instance-key type.
pub const E_INSTANCE: Code = Code::new("E-INSTANCE");
/// A Loop Extension attribute or finite-recursion contract is malformed.
pub const E_LOOP: Code = Code::new("E-LOOP");
/// A Loop body cannot prove one finite progress verdict on every traversal.
pub const E_LOOP_CONTROL: Code = Code::new("E-LOOP-CONTROL");
/// Loop is attached to a Node whose resolved execution profile cannot form one
/// finite envelope-triggered application.
pub const E_LOOP_HOST: Code = Code::new("E-LOOP-HOST");
/// Loop has no unique recurrent admission input.
pub const E_LOOP_ENTRY: Code = Code::new("E-LOOP-ENTRY");
/// Loop has no unambiguous set of ordinary outputs supplying natural Done.
pub const E_LOOP_TERMINAL: Code = Code::new("E-LOOP-TERMINAL");
/// A second Operator handler names the same activation.
pub const E_HANDLER_DUPLICATE: Code = Code::new("E-HANDLER-DUPLICATE");
/// A Graph body tries to own application code; move it into an Operator.
pub const E_GRAPH_HANDLER: Code = Code::new("E-GRAPH-HANDLER");
/// A Connector body tries to own DSL code instead of its Contract transition.
pub const E_CONNECTOR_HANDLER: Code = Code::new("E-CONNECTOR-HANDLER");
/// A Graph scope tries to declare private application state.
pub const E_GRAPH_STATE: Code = Code::new("E-GRAPH-STATE");
/// A Connector tries to declare DSL state outside its versioned Contract.
pub const E_CONNECTOR_STATE: Code = Code::new("E-CONNECTOR-STATE");
/// A closed match scrutinizes an open or unsupported type.
pub const E_MATCH_NOT_CLOSED: Code = Code::new("E-MATCH-NOT-CLOSED");
/// Two closed arms select the same case.
pub const E_MATCH_OVERLAP: Code = Code::new("E-MATCH-OVERLAP");
/// A closed match leaves a case uncovered.
pub const E_MATCH_NONEXHAUSTIVE: Code = Code::new("E-MATCH-NONEXHAUSTIVE");
/// A second closed route match tries to become another authority for the same
/// source cases.
pub const E_MATCH_DUPLICATE: Code = Code::new("E-MATCH-DUPLICATE");
/// A match appears after an effectful statement instead of at dispatch position.
pub const E_DISPATCH_POSITION: Code = Code::new("E-DISPATCH-POSITION");
/// An atomic multi-emission repeats one output target.
pub const E_MULTI_EMIT_DUPLICATE: Code = Code::new("E-MULTI-EMIT-DUPLICATE");
/// Tuple-valued multi-emission has a different arity than its targets.
pub const E_MULTI_EMIT_ARITY: Code = Code::new("E-MULTI-EMIT-ARITY");
/// A route selects the same destination twice for one source case.
pub const E_ROUTE_DUPLICATE: Code = Code::new("E-ROUTE-DUPLICATE");
/// `nothing` and a real destination are selected for the same source case.
pub const E_ROUTE_MIXED: Code = Code::new("E-ROUTE-MIXED");
/// Removed wiring syntax has one named migration diagnostic.
pub const E_WIRING_MIGRATION: Code = Code::new("E-WIRING-MIGRATION");

/// A braced value mixing ordinary data fields with reserved envelope
/// roots — the classification is one or the other, never a silent
/// reinterpretation.
pub const E_BRACE_MIXED: Code = Code::new("E-BRACE-MIXED");
/// Diagnostic for a wiring cycle — edges form a DAG.
pub const E_GRAPH_CYCLE: Code = Code::new("E-GRAPH-CYCLE");
/// Diagnostic for a static literal with two keys canonicalizing equal.
pub const E_DUP_KEY: Code = Code::new("E-DUP-KEY");

/// A node output port no edge consumes; `err` is exempt — it bubbles
/// to the root error sink.
pub const W_UNWIRED_OUTPUT: Code = Code::new("W-UNWIRED-OUTPUT");

/// A lab whose name binds to no `: App` graph in the unit.
pub const E_LAB_NO_APP: Code = Code::new("E-LAB-NO-APP");

/// A simulation targeting anything but the App root — the only
/// executable target today.
pub const E_SIM_TARGET: Code = Code::new("E-SIM-TARGET");

/// Two simulations of one lab sharing a name — fragments merge, but
/// homonyms would shadow silently.
pub const E_DUP_LAB_IDENTITY: Code = Code::new("E-DUP-LAB-IDENTITY");

/// The same source endpoint wired twice to the same target — a
/// compile error per `envelope.md`: the two deliveries would derive
/// one call ref and collide in the journal.
pub const E_DUP_EDGE: Code = Code::new("E-DUP-EDGE");

/// Merged sources of one compilation: the application files. Lab
/// fragments join at the scenario stage.
#[derive(Debug, Clone, Default)]
pub struct Unit {
    /// The parsed files, in command-line order.
    pub files: Vec<(SourceId, SyntaxTree)>,
}

/// Checks a unit into HIR against a contract catalogue. Diagnostics
/// accumulate in `bag`; `None` means the unit has no single `: App`
/// graph to check. A returned HIR is meaningful only when the bag
/// holds no errors.
#[must_use]
pub fn check(
    unit: &Unit,
    registry: &contract::ContractCatalog,
    bag: &mut Bag,
    asset_root: Option<&std::path::Path>,
) -> Option<Hir> {
    let mut apps = Vec::new();
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            if let TopDecl::Graph(graph) = decl {
                let is_app = graph.annotation.as_ref().is_some_and(|annotation| {
                    annotation.symbols.iter().any(|symbol| symbol.text == "App")
                });
                if is_app {
                    apps.push(graph);
                }
            }
        }
    }
    if apps.len() != 1 {
        let span = apps
            .last()
            .map_or_else(|| Span::new(SourceId::new(0), 0..0), |graph| graph.span);
        bag.push(
            Diagnostic::new(
                E_APP_COUNT,
                Phase::Resolve,
                format!(
                    "a deployed unit needs exactly one `: App` graph, found {}",
                    apps.len()
                ),
            )
            .with_span(span)
            .with_field("count", apps.len().to_string()),
        );
        return None;
    }
    let app = apps[0];
    let mut checker = Checker {
        bag,
        registry,
        ext_fields: std::collections::BTreeSet::new(),
        app_writes: Vec::new(),
        meta_reads: Vec::new(),
        app_schema: BTreeMap::new(),
        in_lab: false,
        asset_root: asset_root.map(std::path::Path::to_path_buf),
        assets: Vec::new(),
        type_decls: BTreeMap::new(),
        resolved_types: BTreeMap::new(),
        resolved_named_schemas: BTreeMap::new(),
        type_stack: Vec::new(),
        reported_annotation_refusals: BTreeSet::new(),
        pending_instances: Vec::new(),
        pending_connector_attributes: Vec::new(),
    };
    // No top-level declaration is silently ignored, and none is
    // silently dropped either: until unit composition and a real
    // liveness analysis exist, every additional top-level declaration
    // defers explicitly. W-DEAD-GRAPH belongs to that future analysis
    // and is deliberately not emitted before it.
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            match decl {
                TopDecl::Graph(graph) if !std::ptr::eq(app, graph) => {
                    checker.deferred("a top-level graph outside the App", graph.name.span);
                }
                TopDecl::Val(val) => {
                    checker.deferred("a top-level val", val.span);
                }
                _ => {}
            }
        }
    }
    // Named types collect FIRST — unit-level, duplicates refused,
    // every declaration cycle- and budget-checked before any use
    // (D-2026-08-10-8).
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            if let TopDecl::Type(type_decl) = decl {
                if !checker.guard_type_name(&type_decl.name) {
                    continue;
                }
                if checker.type_decls.contains_key(&type_decl.name.text) {
                    checker.bag.push(
                        Diagnostic::new(
                            E_DUP_TYPE,
                            Phase::Resolve,
                            format!("the type `{}` is already declared", type_decl.name.text),
                        )
                        .with_span(type_decl.name.span)
                        .with_field("name", type_decl.name.text.clone()),
                    );
                    continue;
                }
                checker
                    .type_decls
                    .insert(type_decl.name.text.clone(), type_decl.ty.clone());
            }
        }
    }
    let mut valid_type_names = std::collections::BTreeSet::new();
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            if let TopDecl::Type(type_decl) = decl
                && checker.validate_type_decl(type_decl)
            {
                valid_type_names.insert(type_decl.name.text.clone());
            }
        }
    }
    // ONE alias resolution, shared by every use: pure alias chains
    // collapse HERE, after validation — a use site never follows a
    // chain again, so what validation accepted, resolution serves
    // (the 256-link boundary included; longer chains and cycles
    // were refused above and collapse stops harmlessly on them).
    checker.collapse_alias_chains();
    checker.resolve_expression_types(&valid_type_names);
    let mut checked_app = checker.graph(app);
    // The production schema seals HERE — from the deployed graph's
    // writes alone. Everything the Lab compiles below is checked
    // against it and can extend nothing (D-2026-08-10-1).
    checker.seal_app_schema();
    checker.resolve_pending_instances(&mut checked_app);
    checker.resolve_pending_connector_attributes(&mut checked_app);
    checker.in_lab = true;
    // A lab is a logical corpus: fragments sharing a name merge into
    // one; a duplicate simulation name inside the merged corpus
    // refuses — it would shadow silently.
    let mut labs: Vec<crate::compiler::hir::Lab> = Vec::new();
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            if let TopDecl::Lab(lab) = decl
                && let Some(checked) = checker.lab(lab, &checked_app)
            {
                match labs.iter_mut().find(|merged| merged.name == checked.name) {
                    Some(merged) => {
                        for simulation in checked.simulations {
                            if merged
                                .simulations
                                .iter()
                                .any(|existing| existing.name == simulation.name)
                            {
                                checker.bag.push(
                                    Diagnostic::new(
                                        E_DUP_LAB_IDENTITY,
                                        Phase::Resolve,
                                        format!(
                                            "the simulation `{}` is already declared in lab `{}`",
                                            simulation.name, merged.name
                                        ),
                                    )
                                    .with_span(lab.name.span)
                                    .with_field("name", simulation.name.clone()),
                                );
                                continue;
                            }
                            merged.simulations.push(simulation);
                        }
                    }
                    None => labs.push(checked),
                }
            }
        }
    }
    checker.in_lab = false;
    checker.verify_meta_reads();
    // The asset table's CANONICAL layout (D-2026-08-10-5 as
    // amended): digest order — one content set, one byte layout,
    // whatever order the source visited it in. Indices in the
    // compiled trees remap accordingly.
    if !checker.assets.is_empty() {
        let mut order: Vec<usize> = (0..checker.assets.len()).collect();
        order.sort_by(|&a, &b| checker.assets[a].digest.cmp(&checker.assets[b].digest));
        let mut remap = vec![0u32; order.len()];
        for (new_index, &old_index) in order.iter().enumerate() {
            remap[old_index] = u32::try_from(new_index).unwrap_or_else(|_| unreachable!("bounded"));
        }
        checker.assets = order
            .iter()
            .map(|&old_index| checker.assets[old_index].clone())
            .collect();
        remap_graph_assets(&mut checked_app, &remap);
    }
    let app_schema: Vec<(String, DataType)> = checker
        .app_schema
        .iter()
        .map(|(field, ty)| (field.clone(), ty.clone()))
        .collect();
    // Standard EXT leaves are IMPLICIT — known to every consumer, never
    // serialized; the schema carries application-defined fields only.
    let ext_schema: Vec<(String, DataType)> = checker
        .ext_fields
        .iter()
        .map(|field| (field.clone(), DataType::Variant))
        .collect();
    let named_schemas = checker.resolved_named_schemas.into_iter().collect();
    Some(Hir {
        app: checked_app,
        labs,
        app_schema,
        ext_schema,
        assets: checker.assets,
        named_schemas,
    })
}

struct Checker<'bag, 'registry> {
    bag: &'bag mut Bag,
    registry: &'registry contract::ContractCatalog,
    /// The `ext` vocabulary the instantiated connectors' sheets
    /// declare — standard external-context leaves are always known.
    ext_fields: std::collections::BTreeSet<String>,
    /// The graph's `meta.app` writes — the INFERENCE source: field,
    /// written type, and site (D-2026-08-10-1). Lab writes never
    /// land here.
    app_writes: Vec<(String, DataType, Span)>,
    /// Every `meta.*` read compiled anywhere in the unit, verified
    /// against the sealed schema once inference closes.
    meta_reads: Vec<(Vec<String>, Span)>,
    /// The sealed production schema — graph writes only.
    app_schema: BTreeMap<String, DataType>,
    /// Whether the checker is inside the Lab surface: lab writes
    /// never extend the production schema.
    in_lab: bool,
    /// The asset root this compilation may read from — `None`
    /// refuses every `asset(…)` (a string-source compilation has no
    /// filesystem).
    asset_root: Option<std::path::PathBuf>,
    /// The resolved blobs, deduplicated by digest.
    assets: Vec<crate::plan::AssetBlob>,
    /// The unit's named types (D-2026-08-10-8): structural sugar,
    /// pre-validated for cycles and budgets at collection.
    type_decls: BTreeMap<String, crate::compiler::syntax::TypeRef>,
    /// Named aliases expanded to their logical type for `is`/`as`. Expression
    /// trees carry the expansion, never the source spelling.
    resolved_types: BTreeMap<String, DataType>,
    /// Alias schemas resolved through the same path-expanding annotation
    /// pass as executable type sites. A1 retains these in HIR; A2 will carry
    /// only the schemas that own transported policy.
    resolved_named_schemas: BTreeMap<String, AnnotatedSchema>,
    /// The expansion stack — a name on it twice is a cycle.
    type_stack: Vec<String>,
    /// Alias expansion may visit one faulty source annotation at several use
    /// sites. The source fault speaks once, at its own stable span.
    reported_annotation_refusals: BTreeSet<Span>,
    /// Node-owned instance keys are type-checked after the production
    /// metadata schema has been sealed. That makes `meta.app.<leaf>` precise
    /// without a second type system or a runtime-only guess.
    pending_instances: Vec<PendingInstance>,
    /// Invocation-bound Connector attributes are checked after the complete
    /// application metadata schema and owner-instance key types are known.
    /// The delay is checker-local: the expression still enters HIR identity
    /// exactly once and the Driver never receives source text.
    pending_connector_attributes: Vec<PendingConnectorAttribute>,
}

#[derive(Clone)]
struct PendingConnectorAttribute {
    node: node::Path,
    name: String,
    value: SynValue,
    domain: contract::ValueDomain,
    latest_site: crate::plan::EvaluationSite,
    signature: expr::Signature,
    consts: BTreeMap<String, crate::value::Value>,
    span: Span,
}

#[derive(Clone)]
enum InstanceOwner {
    Graph(crate::plan::OwnerPath),
    Node(node::Path),
}

#[derive(Clone)]
struct PendingInstance {
    owner: InstanceOwner,
    key: Option<SynValue>,
    lifetime: InstanceLifetimeSource,
    closing: Option<ClosingSource>,
    signature: expr::Signature,
    consts: BTreeMap<String, crate::value::Value>,
    span: Span,
}

#[derive(Clone)]
struct ClosingSource {
    drain_for: SynValue,
    drain_span: Span,
    settle_for: SynValue,
    settle_span: Span,
}

#[derive(Clone)]
enum InstanceLifetimeSource {
    Persistent,
    Idle { after: SynValue, span: Span },
    Fixed { after: SynValue, span: Span },
    SinglePass,
    Tumbling { every: SynValue, span: Span },
}

impl Checker<'_, '_> {
    fn deferred(&mut self, construct: &'static str, span: Span) {
        self.bag.push(
            Diagnostic::new(
                E_CHECK_DEFERRED,
                Phase::Check,
                format!("{construct} is outside the executable subset of this build"),
            )
            .with_span(span)
            .with_field("construct", construct),
        );
    }

    /// Compiles one Rastok expression span, adapting native diagnostics onto the
    /// source with offset-mapped spans.
    fn expr(&mut self, expr: &ExprSource, signature: &expr::Signature) -> Option<expr::Tree> {
        self.expr_with_asset_policy(expr, signature, false)
            .map(|(tree, _)| tree)
    }

    /// Compiles an expression used as the whole payload. Only this door may
    /// retain the optional content type of a root `asset(path, ct)` call.
    fn payload_expr(
        &mut self,
        expr: &ExprSource,
        signature: &expr::Signature,
    ) -> Option<(expr::Tree, Option<String>)> {
        self.expr_with_asset_policy(expr, signature, true)
    }

    fn expr_with_asset_policy(
        &mut self,
        source: &ExprSource,
        signature: &expr::Signature,
        allow_root_content_type: bool,
    ) -> Option<(expr::Tree, Option<String>)> {
        let signature = self
            .resolved_types
            .iter()
            .fold(signature.clone(), |signature, (name, ty)| {
                signature.with_type(name.clone(), ty.clone())
            });
        match expr::compile(&source.text, &signature) {
            Ok(mut tree) => {
                let (root, content_type) =
                    self.bind_asset_calls(tree.root, source.span, allow_root_content_type)?;
                tree.root = root;
                self.collect_meta_reads(&tree.root, source.span, &signature);
                Some((tree, content_type))
            }
            Err(diagnostics) => {
                let base = source.span.range().start;
                for diagnostic in diagnostics {
                    // The registry owns the codes, and the expression chain's
                    // typed kind chooses them — never the human text,
                    // which is free to change.
                    let code = match diagnostic.kind {
                        expr::DiagnosticKind::DuplicateMapKey => E_DUP_KEY,
                        expr::DiagnosticKind::AssetCall => E_ASSET,
                        // The depth gate: the same named refusal the
                        // plan validator maps at the plan phase, first
                        // surfaced here — before any recursive pass.
                        expr::DiagnosticKind::TooDeep { .. } => crate::compiler::E_PLAN_DEPTH,
                        expr::DiagnosticKind::General => E_EXPR,
                    };
                    let adapted = Diagnostic::new(code, Phase::Check, diagnostic.message.clone())
                        .with_span(Span::new(
                            source.span.source(),
                            base + diagnostic.range.start..base + diagnostic.range.end,
                        ));
                    // The STABLE payload per the registry: E-PLAN-DEPTH
                    // carries the numeric `depth` in every phase; the
                    // human sentence stays a message, never a field.
                    self.bag.push(match diagnostic.kind {
                        expr::DiagnosticKind::TooDeep { depth } => {
                            adapted.with_field("depth", depth.to_string())
                        }
                        expr::DiagnosticKind::AssetCall => adapted.with_field("path", ""),
                        _ => adapted.with_field("detail", diagnostic.message),
                    });
                }
                None
            }
        }
    }

    /// Resolves every ordinary `asset(…)` expression call to the canonical
    /// content table before hashing or planning. A content type is accepted
    /// only on the root call of a whole-payload expression; nested calls have no
    /// metadata namespace of their own.
    fn bind_asset_calls(
        &mut self,
        node: expr::Expr,
        at: Span,
        allow_content_type: bool,
    ) -> Option<(expr::Expr, Option<String>)> {
        let bind_child = |checker: &mut Self, child: expr::Expr| {
            checker
                .bind_asset_calls(child, at, false)
                .map(|(child, _)| child)
        };
        Some(match node {
            expr::Expr::Call {
                function: expr::Function::Asset,
                args,
            } => {
                let literal = |checker: &mut Self, value: &expr::Expr| -> Option<String> {
                    match value {
                        expr::Expr::Literal(crate::value::Value::Text(text)) => Some(text.clone()),
                        _ => {
                            checker.bag.push(
                                Diagnostic::new(
                                    E_ASSET,
                                    Phase::Check,
                                    "asset arguments are build-time string literals",
                                )
                                .with_span(at)
                                .with_field("path", ""),
                            );
                            None
                        }
                    }
                };
                let Some(path_arg) = args.first() else {
                    self.bag.push(
                        Diagnostic::new(E_ASSET, Phase::Check, "asset expects a repository path")
                            .with_span(at)
                            .with_field("path", ""),
                    );
                    return None;
                };
                let path = literal(self, path_arg)?;
                let content_type = match args.get(1) {
                    Some(value) => {
                        let written = literal(self, value)?;
                        Some(self.content_type(&written, at, "asset")?)
                    }
                    None => None,
                };
                if content_type.is_some() && !allow_content_type {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTENT_TYPE,
                            Phase::Check,
                            "a content type qualifies the WHOLE payload — a nested value has no `meta.ext` of its own",
                        )
                        .with_span(at)
                        .with_field("tag", "asset"),
                    );
                    return None;
                }
                let index = self.asset_resolve(&path, at)?;
                (expr::Expr::Asset(index), content_type)
            }
            expr::Expr::Literal(_) | expr::Expr::Path(_) | expr::Expr::Asset(_) => (node, None),
            expr::Expr::Unary { op, expr } => (
                expr::Expr::Unary {
                    op,
                    expr: Box::new(bind_child(self, *expr)?),
                },
                None,
            ),
            expr::Expr::Binary { op, left, right } => (
                expr::Expr::Binary {
                    op,
                    left: Box::new(bind_child(self, *left)?),
                    right: Box::new(bind_child(self, *right)?),
                },
                None,
            ),
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => (
                expr::Expr::Ternary {
                    cond: Box::new(bind_child(self, *cond)?),
                    then: Box::new(bind_child(self, *then)?),
                    otherwise: Box::new(bind_child(self, *otherwise)?),
                },
                None,
            ),
            expr::Expr::Index { base, index } => (
                expr::Expr::Index {
                    base: Box::new(bind_child(self, *base)?),
                    index: Box::new(bind_child(self, *index)?),
                },
                None,
            ),
            expr::Expr::Call { function, args } => (
                expr::Expr::Call {
                    function,
                    args: args
                        .into_iter()
                        .map(|arg| bind_child(self, arg))
                        .collect::<Option<Vec<_>>>()?,
                },
                None,
            ),
            expr::Expr::List(items) => (
                expr::Expr::List(
                    items
                        .into_iter()
                        .map(|item| bind_child(self, item))
                        .collect::<Option<Vec<_>>>()?,
                ),
                None,
            ),
            expr::Expr::MapLit(entries) => (
                expr::Expr::MapLit(
                    entries
                        .into_iter()
                        .map(|(key, value)| Some((key, bind_child(self, value)?)))
                        .collect::<Option<Vec<_>>>()?,
                ),
                None,
            ),
            expr::Expr::Struct(fields) => (
                expr::Expr::Struct(
                    fields
                        .into_iter()
                        .map(|(name, value)| Some((name, bind_child(self, value)?)))
                        .collect::<Option<Vec<_>>>()?,
                ),
                None,
            ),
            expr::Expr::Tagged {
                union,
                case,
                fields,
            } => (
                expr::Expr::Tagged {
                    union,
                    case,
                    fields: fields
                        .into_iter()
                        .map(|(name, value)| Some((name, bind_child(self, value)?)))
                        .collect::<Option<Vec<_>>>()?,
                },
                None,
            ),
            expr::Expr::Is { expr, target } => (
                expr::Expr::Is {
                    expr: Box::new(bind_child(self, *expr)?),
                    target,
                },
                None,
            ),
            expr::Expr::As { expr, target } => (
                expr::Expr::As {
                    expr: Box::new(bind_child(self, *expr)?),
                    target,
                },
                None,
            ),
        })
    }

    /// Compiles and reduces a constant expression, keeping both the
    /// identity-bearing tree and its reduction.
    fn constant(
        &mut self,
        expr: &ExprSource,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Const> {
        let tree = self.expr(expr, signature)?;
        self.reduce_constant(tree, expr.span, consts)
    }

    /// Reduces a tree already built by the host value grammar. Tagged and
    /// struct constructors share this door with intrinsic expressions, so a
    /// constant position has one evaluation rule and one diagnostic shape.
    fn reduce_constant(
        &mut self,
        tree: expr::Tree,
        span: Span,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Const> {
        match expr::const_eval(&tree, consts) {
            Ok(value) => Some(Const { tree, value }),
            Err(error) => {
                self.bag.push(
                    Diagnostic::new(
                        E_EXPR,
                        Phase::Check,
                        format!("this position requires a constant: {error}"),
                    )
                    .with_span(span)
                    .with_field("detail", error.message),
                );
                None
            }
        }
    }

    /// Checks one lab against the checked App. Labs never enter the
    /// semantic hash; they compile into their own executable form.
    fn lab(
        &mut self,
        decl: &crate::compiler::syntax::LabDecl,
        app: &Graph,
    ) -> Option<crate::compiler::hir::Lab> {
        if !self.guard_binding_name(&decl.name, "a lab") {
            return None;
        }
        if decl.name.text != app.name {
            self.bag.push(
                Diagnostic::new(
                    E_LAB_NO_APP,
                    Phase::Resolve,
                    format!(
                        "the lab `{}` binds to no `: App` graph in this unit",
                        decl.name.text
                    ),
                )
                .with_span(decl.name.span)
                .with_field("name", decl.name.text.clone()),
            );
            return None;
        }
        let mut simulations = Vec::new();
        for simulation in &decl.simulations {
            if decl
                .simulations
                .iter()
                .take_while(|earlier| !std::ptr::eq(*earlier, simulation))
                .any(|earlier| earlier.name.text == simulation.name.text)
            {
                self.bag.push(
                    Diagnostic::new(
                        E_DUP_LAB_IDENTITY,
                        Phase::Resolve,
                        format!(
                            "the simulation `{}` is already declared in this lab",
                            simulation.name.text
                        ),
                    )
                    .with_span(simulation.name.span)
                    .with_field("name", simulation.name.text.clone()),
                );
                continue;
            }
            let target: Vec<&str> = simulation
                .target
                .iter()
                .map(|segment| segment.text.as_str())
                .collect();
            if target != ["app"] {
                self.bag.push(
                    Diagnostic::new(
                        E_SIM_TARGET,
                        Phase::Resolve,
                        "a simulation closes `app`; sub-graph targets are a deferred capability"
                            .to_string(),
                    )
                    .with_span(simulation.name.span)
                    .with_field("target", target.join(".")),
                );
                continue;
            }
            for mock in &simulation.mocks {
                if simulation
                    .mocks
                    .iter()
                    .take_while(|earlier| !std::ptr::eq(*earlier, mock))
                    .any(|earlier| {
                        earlier.path.len() == mock.path.len()
                            && earlier
                                .path
                                .iter()
                                .zip(&mock.path)
                                .all(|(a, b)| a.text == b.text)
                    })
                {
                    self.bag.push(
                        Diagnostic::new(
                            E_DUP_LAB_IDENTITY,
                            Phase::Resolve,
                            "this connector is already mocked at this level".to_string(),
                        )
                        .with_span(mock.span)
                        .with_field("name", mock.path[0].text.clone()),
                    );
                }
            }
            let mocks = self.mocks(&simulation.mocks, app);
            let mut scenarios = Vec::new();
            for scenario in &simulation.scenarios {
                if simulation
                    .scenarios
                    .iter()
                    .take_while(|earlier| !std::ptr::eq(*earlier, scenario))
                    .any(|earlier| earlier.name.text == scenario.name.text)
                {
                    self.bag.push(
                        Diagnostic::new(
                            E_DUP_LAB_IDENTITY,
                            Phase::Resolve,
                            format!(
                                "the scenario `{}` is already declared in this simulation",
                                scenario.name.text
                            ),
                        )
                        .with_span(scenario.name.span)
                        .with_field("name", scenario.name.text.clone()),
                    );
                    continue;
                }
                scenarios.push(self.scenario(scenario, app));
            }
            simulations.push(crate::compiler::hir::Simulation {
                name: simulation.name.text.clone(),
                mocks,
                scenarios,
            });
        }
        Some(crate::compiler::hir::Lab {
            name: decl.name.text.clone(),
            simulations,
        })
    }

    fn mocks(
        &mut self,
        decls: &[crate::compiler::syntax::MockDecl],
        app: &Graph,
    ) -> Vec<crate::compiler::hir::Mock> {
        let mut mocks = Vec::new();
        for decl in decls {
            if decl.path.len() > 1 {
                self.deferred("a mock of a composed node", decl.span);
                continue;
            }
            let name = &decl.path[0];
            let Some(node) = app.nodes.iter().find(|node| {
                node.path.segments().len() == 1 && node.path.segments()[0] == name.text
            }) else {
                self.bag.push(
                    Diagnostic::new(
                        E_RESOLVE_UNKNOWN,
                        Phase::Resolve,
                        format!("`{}` names no node of the App", name.text),
                    )
                    .with_span(name.span)
                    .with_field("name", name.text.clone()),
                );
                continue;
            };
            if node.connector_contract_id().is_none() {
                self.deferred("a mock of an operator", name.span);
                continue;
            }
            // A mock is a pure function of the request: its handlers
            // check like ordinary handlers, against the connector's
            // own port table — which answers the contract's
            // EFFECTIVE out pin (`node_port_pin`): a jsonl mock is
            // judged against `array<variant>` exactly like the edge
            // audit and the machine judge it.
            let signature = expr::Signature::new();
            let mut handlers = Vec::new();
            for handler in &decl.handlers {
                if let Some(checked) = self.handler(handler, &signature, node, &BTreeMap::new()) {
                    let mut dispatch_statements = Vec::new();
                    collect_hir_statements(&checked.dispatch, &mut dispatch_statements);
                    if checked.v15_linear_parts().is_none() {
                        self.deferred(
                            "a recursive `match` Dispatch in a Lab mock before the v16 Lab executor",
                            handler.span,
                        );
                        continue;
                    }
                    let emitted: Vec<_> = dispatch_statements
                        .iter()
                        .filter_map(|statement| match statement {
                            Statement::Emit { port, value }
                                if !matches!(value, EmitValue::Nothing) =>
                            {
                                Some(port)
                            }
                            _ => None,
                        })
                        .collect();
                    // A body with no actual output is the written silence and
                    // intentionally leaves the call open. Any answer is one
                    // whole completion: exactly one exit, last.
                    if !emitted.is_empty()
                        && (emitted.iter().filter(|port| port.name() == "exit").count() != 1
                            || emitted.last().is_none_or(|port| port.name() != "exit"))
                    {
                        self.bag.push(
                            Diagnostic::new(
                                E_CONTRACT_REQUIRED,
                                Phase::Check,
                                "a mock answer requires exactly one final `exit` emission",
                            )
                            .with_span(handler.span)
                            .with_field("port", "exit"),
                        );
                        continue;
                    }
                    // A reply carries data only: its envelope is
                    // minted by the connector's own transaction —
                    // the verdict lands here, not mid-simulation.
                    let writes_meta = dispatch_statements.iter().any(|statement| {
                        let Statement::Emit { value, .. } = statement else {
                            return false;
                        };
                        let (EmitValue::Patch(writes) | EmitValue::FreshData(writes)) = value
                        else {
                            return false;
                        };
                        writes.iter().any(|write| {
                            !matches!(write.target, crate::envelope::WriteTarget::Data(_))
                        })
                    });
                    if writes_meta {
                        self.deferred("a mock reply metadata write", handler.span);
                        continue;
                    }
                    handlers.push(checked);
                }
            }
            mocks.push(crate::compiler::hir::Mock {
                connector: node.path.clone(),
                handlers,
            });
        }
        mocks
    }

    fn scenario(
        &mut self,
        decl: &crate::compiler::syntax::ScenarioDecl,
        app: &Graph,
    ) -> crate::compiler::hir::Scenario {
        use crate::compiler::hir::{Cmp, ExpectTarget, Expectation, Quantifier, ScenarioStep};
        use crate::compiler::syntax::{CmpOp, ScenarioItem};
        let mut mocks = Vec::new();
        let mut mocked: Vec<String> = Vec::new();
        let mut steps = Vec::new();
        for item in &decl.body {
            match item {
                ScenarioItem::Mock(mock) => {
                    let name = mock
                        .path
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    if mocked.contains(&name) {
                        self.bag.push(
                            Diagnostic::new(
                                E_DUP_LAB_IDENTITY,
                                Phase::Resolve,
                                "this connector is already mocked in this scenario".to_string(),
                            )
                            .with_span(mock.span)
                            .with_field("name", name),
                        );
                        continue;
                    }
                    mocked.push(name);
                    mocks.extend(self.mocks(std::slice::from_ref(mock), app));
                }
                ScenarioItem::Injection { target, value, seq } => {
                    let segments: Vec<&str> = target
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect();
                    // The injection target is a graph INPUT of the App.
                    // Source syntax predeclares conventional `in`; the
                    // checker materializes every used boundary row before
                    // HIR leaves. Source-Connector injection still joins with
                    // the source Drivers.
                    let ["app", name] = segments.as_slice() else {
                        self.deferred("an injection outside the app boundary", target.span);
                        continue;
                    };
                    let port = match *name {
                        "in" => node::Port::new("in")
                            .unwrap_or_else(|_| unreachable!("`in` is a valid port")),
                        "out" | "err" => {
                            self.wrong_direction(
                                target.span,
                                name,
                                Direction::Output,
                                "an injection enters through a graph input",
                            );
                            continue;
                        }
                        _ => match app.boundary.iter().find(|decl| decl.port.name() == *name) {
                            Some(decl) if decl.direction == Direction::Input => decl.port.clone(),
                            Some(_) => {
                                self.wrong_direction(
                                    target.span,
                                    name,
                                    Direction::Output,
                                    "an injection enters through a graph input",
                                );
                                continue;
                            }
                            None => {
                                self.bag.push(
                                    Diagnostic::new(
                                        E_PORT_UNKNOWN,
                                        Phase::Check,
                                        format!("the App boundary exposes no input named `{name}`"),
                                    )
                                    .with_span(target.span)
                                    .with_field("port", (*name).to_string()),
                                );
                                continue;
                            }
                        },
                    };
                    // A draft is world-authored: it is built from
                    // constants, with no input to read.
                    let signature = expr::Signature::new();
                    let expected = app
                        .boundary
                        .iter()
                        .find(|decl| decl.port == port)
                        .and_then(|decl| decl.ty.clone());
                    let Some(value) = self.emit_value(
                        value,
                        &signature,
                        expected.as_ref(),
                        target.span,
                        port.name(),
                    ) else {
                        continue;
                    };
                    if matches!(value, EmitValue::Identity) {
                        self.deferred("an identity injection (there is no input)", target.span);
                        continue;
                    }
                    // What a draft cannot carry refuses HERE — an
                    // accepted scenario never discovers a restriction
                    // mid-simulation.
                    if let EmitValue::Patch(writes) | EmitValue::FreshData(writes) = &value
                        && writes.iter().any(|write| {
                            matches!(write.target, crate::envelope::WriteTarget::App(_))
                        })
                    {
                        // The fixture is the WORLD's half: world
                        // context enters through `ext`; `app` is the
                        // application's own zone and a Lab never
                        // writes it (D-2026-08-10-1).
                        self.meta_zone_refusal(
                            target.span,
                            "meta.app",
                            "an injection seeds `meta.ext` (the world's context); the graph writes `meta.app` itself",
                        );
                        continue;
                    }
                    // A pinned input holds its injections to the
                    // contract — the SAME conformance notion as
                    // emissions, drafts having no input to inherit.
                    if let Some(expected) = expected
                        && !self.port_conforms(
                            &expected,
                            &value,
                            BuildOrigin::Draft,
                            &BTreeMap::new(),
                            port.name(),
                            target.span,
                        )
                    {
                        continue;
                    }
                    let seq = match seq {
                        None => None,
                        Some(int) => match int.text.parse::<u64>() {
                            Ok(seq) => Some(seq),
                            Err(_) => {
                                self.bag.push(
                                    Diagnostic::new(
                                        E_EXPR,
                                        Phase::Check,
                                        "an inbound sequence must be a non-negative integer"
                                            .to_string(),
                                    )
                                    .with_span(int.span)
                                    .with_field("detail", int.text.clone()),
                                );
                                continue;
                            }
                        },
                    };
                    steps.push(ScenarioStep::Inject { port, value, seq });
                }
                ScenarioItem::Advance { expr, span } => {
                    let signature = expr::Signature::new();
                    let Some(constant) = self.constant(expr, &signature, &BTreeMap::new()) else {
                        continue;
                    };
                    let crate::value::Value::Duration(nanos) = constant.value else {
                        self.bag.push(
                            Diagnostic::new(
                                E_EXPR,
                                Phase::Check,
                                "advance takes a constant duration".to_string(),
                            )
                            .with_span(*span)
                            .with_field("detail", format!("{:?}", constant.value)),
                        );
                        continue;
                    };
                    steps.push(ScenarioStep::Advance { nanos });
                }
                ScenarioItem::Expect(expect) => {
                    let segments: Vec<&str> = expect
                        .target
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect();
                    let (target, observed_type) = match segments.as_slice() {
                        ["app", port] => {
                            let mut root_types = app
                                .nodes
                                .iter()
                                .filter_map(|node| {
                                    node.ports
                                        .iter()
                                        .find(|decl| {
                                            decl.port.name() == *port
                                                && decl.direction == Direction::Output
                                                && decl.unwired
                                                    == crate::plan::UnwiredPolicy::RootSink
                                        })
                                        .map(|decl| decl.ty.clone().unwrap_or(DataType::Variant))
                                })
                                .collect::<Vec<_>>();
                            if !root_types.is_empty() {
                                // Root-sink and App-boundary observations share
                                // the same `app.<port>` window in the Lab. If
                                // both can speak the name, type the predicate
                                // against their common type or `variant` —
                                // never pretend one producer is invisible.
                                if let Some(boundary_type) = app
                                    .boundary
                                    .iter()
                                    .find(|decl| {
                                        decl.port.name() == *port
                                            && decl.direction == Direction::Output
                                    })
                                    .map(|decl| decl.ty.clone().unwrap_or(DataType::Variant))
                                {
                                    root_types.push(boundary_type);
                                }
                                let observed_type = root_types
                                    .first()
                                    .cloned()
                                    .filter(|first| {
                                        root_types.iter().all(|candidate| candidate == first)
                                    })
                                    .unwrap_or(DataType::Variant);
                                let Ok(port) = node::Port::new((*port).to_string()) else {
                                    continue;
                                };
                                (ExpectTarget::RootSink(port), observed_type)
                            } else {
                                // A boundary observation watches what
                                // LEAVES the App: `out` plus the declared
                                // boundary OUTPUTS. An input is an
                                // ingress, never a destination — the
                                // machine logs nothing there, so an
                                // expectation on one (like a typo) would
                                // be a false green by construction.
                                let direction = match *port {
                                    "out" => Some(Direction::Output),
                                    "in" => Some(Direction::Input),
                                    _ => app
                                        .boundary
                                        .iter()
                                        .find(|decl| decl.port.name() == *port)
                                        .map(|decl| decl.direction),
                                };
                                match direction {
                                    None => {
                                        self.bag.push(
                                        Diagnostic::new(
                                            E_PORT_UNKNOWN,
                                            Phase::Check,
                                            format!(
                                                "the App boundary exposes no port named `{port}`"
                                            ),
                                        )
                                        .with_span(expect.target.span)
                                        .with_field("port", (*port).to_string()),
                                    );
                                        continue;
                                    }
                                    Some(Direction::Input) => {
                                        self.wrong_direction(
                                            expect.target.span,
                                            port,
                                            Direction::Input,
                                            "an expectation watches what leaves the App",
                                        );
                                        continue;
                                    }
                                    Some(Direction::Output) => {}
                                }
                                let observed_type = app
                                    .boundary
                                    .iter()
                                    .find(|decl| decl.port.name() == *port)
                                    .and_then(|decl| decl.ty.clone())
                                    .unwrap_or(DataType::Variant);
                                match node::Port::new((*port).to_string()) {
                                    Ok(port) => (ExpectTarget::Boundary(port), observed_type),
                                    Err(_) => continue,
                                }
                            }
                        }
                        [name, port] => {
                            let Some(node) = app.nodes.iter().find(|node| {
                                node.path.segments().len() == 1 && node.path.segments()[0] == *name
                            }) else {
                                self.bag.push(
                                    Diagnostic::new(
                                        E_RESOLVE_UNKNOWN,
                                        Phase::Resolve,
                                        format!("`{name}` names no node of the App"),
                                    )
                                    .with_span(expect.target.span)
                                    .with_field("name", (*name).to_string()),
                                );
                                continue;
                            };
                            let observed_type = if let Some(port_decl) =
                                node.ports.iter().find(|decl| decl.port.name() == *port)
                            {
                                port_decl.ty.clone().unwrap_or(DataType::Variant)
                            } else if matches!(*port, "in" | "out") {
                                DataType::Variant
                            } else {
                                self.bag.push(
                                    Diagnostic::new(
                                        E_PORT_UNKNOWN,
                                        Phase::Check,
                                        format!("`{name}` exposes no port named `{port}`"),
                                    )
                                    .with_span(expect.target.span)
                                    .with_field("port", (*port).to_string()),
                                );
                                continue;
                            };
                            match node::Port::new((*port).to_string()) {
                                Ok(port) => (
                                    ExpectTarget::Node {
                                        path: node.path.clone(),
                                        port,
                                    },
                                    observed_type,
                                ),
                                Err(_) => continue,
                            }
                        }
                        _ => {
                            self.deferred("a nested expectation target", expect.target.span);
                            continue;
                        }
                    };
                    if expect.nothing && expect.count.is_some() {
                        // Two quantifiers on one expectation is an
                        // ambiguous form; refusing beats letting one
                        // win silently.
                        self.bag.push(
                            Diagnostic::new(
                                E_EXPR,
                                Phase::Check,
                                "`nothing` and `count` are two quantifiers; pick one",
                            )
                            .with_span(expect.span)
                            .with_field("detail", "ambiguous quantifier"),
                        );
                        continue;
                    }
                    let quantifier = if expect.nothing {
                        Quantifier::Nothing
                    } else if let Some((op, int)) = &expect.count {
                        let Ok(count) = int.text.parse::<i64>() else {
                            // A count too wide to represent is a
                            // refusal, never a dropped expectation.
                            self.bag.push(
                                Diagnostic::new(
                                    E_EXPR,
                                    Phase::Check,
                                    "this count does not fit a 64-bit integer",
                                )
                                .with_span(int.span)
                                .with_field("detail", int.text.clone()),
                            );
                            continue;
                        };
                        let cmp = match op {
                            CmpOp::Eq => Cmp::Eq,
                            CmpOp::Ne => Cmp::Ne,
                            CmpOp::Lt => Cmp::Lt,
                            CmpOp::Gt => Cmp::Gt,
                            CmpOp::Le => Cmp::Le,
                            CmpOp::Ge => Cmp::Ge,
                        };
                        Quantifier::Count(cmp, count)
                    } else {
                        Quantifier::Exists
                    };
                    let predicate = match &expect.predicate {
                        None => None,
                        Some(expr) => {
                            let signature = expr::Signature::new()
                                .with("data", observed_type.clone())
                                .with("meta", DataType::Variant);
                            match self.expr(expr, &signature) {
                                Some(tree) => {
                                    if !matches!(tree.ty, DataType::Bool | DataType::Variant) {
                                        self.bag.push(
                                            Diagnostic::new(
                                                E_EXPR,
                                                Phase::Check,
                                                "an expectation predicate must be a bool",
                                            )
                                            .with_span(expr.span)
                                            .with_field(
                                                "detail",
                                                "the predicate type is not bool".to_string(),
                                            ),
                                        );
                                        continue;
                                    }
                                    Some(tree)
                                }
                                None => continue,
                            }
                        }
                    };
                    steps.push(ScenarioStep::Expect(Expectation {
                        label: expect.label.clone(),
                        quantifier,
                        target,
                        predicate,
                    }));
                }
            }
        }
        crate::compiler::hir::Scenario {
            name: decl.name.text.clone(),
            label: decl.label.clone(),
            mocks,
            steps,
        }
    }

    fn graph(&mut self, decl: &GraphDecl) -> Graph {
        self.graph_at(
            decl,
            crate::plan::OwnerPath::root(),
            &expr::Signature::new(),
            &BTreeMap::new(),
            true,
        )
    }

    fn graph_at(
        &mut self,
        decl: &GraphDecl,
        path: crate::plan::OwnerPath,
        inherited_signature: &expr::Signature,
        inherited_consts: &BTreeMap<String, crate::value::Value>,
        is_root: bool,
    ) -> Graph {
        self.guard_binding_name(&decl.name, "a graph");
        let extensions = self.graph_extensions(decl, is_root);
        if decl.removed_per.is_some() {
            self.instance_refusal(
                decl.span,
                "`per` was the unpublished partition-binding syntax; write `instance = { key = <expr> }` inside the Graph",
            );
        }
        let mut graph = Graph {
            name: decl.name.text.clone(),
            extensions,
            loop_spec: None,
            path: path.clone(),
            boundary: Vec::new(),
            vals: Vec::new(),
            handlers: Vec::new(),
            nodes: Vec::new(),
            graphs: Vec::new(),
            edges: Vec::new(),
            routes: Vec::new(),
            instance: None,
        };
        let mut consts = inherited_consts.clone();
        let mut signature = inherited_signature.clone();
        let mut edges = Vec::new();
        let mut handlers = Vec::new();
        let mut children = Vec::new();
        let mut redeclared_defaults = std::collections::BTreeSet::new();
        self.queue_instance_attributes(
            InstanceOwner::Graph(path.clone()),
            decl.body.iter().filter_map(|item| match item {
                GraphItem::Attribute(attribute) => Some(attribute),
                _ => None,
            }),
            &signature,
            &consts,
            decl.span,
        );
        for item in &decl.body {
            match item {
                GraphItem::Port(port) => {
                    if !self.guard_binding_name(&port.name, "a boundary port") {
                        continue;
                    }
                    if graph.nodes.iter().any(|node| {
                        node.path.segments().last().map(String::as_str)
                            == Some(port.name.text.as_str())
                    }) || graph
                        .graphs
                        .iter()
                        .any(|nested| nested.name == port.name.text)
                    {
                        self.bag.push(
                            Diagnostic::new(
                                E_NAME_CLASH,
                                Phase::Resolve,
                                format!(
                                    "`{}` already names a child; Graph ports and children share one namespace",
                                    port.name.text
                                ),
                            )
                            .with_span(port.name.span)
                            .with_field("name", port.name.text.clone()),
                        );
                        continue;
                    }
                    // Boundary and state annotations resolve through the same
                    // complete logical type algebra.
                    let (pinned, annotations) = match &port.ty {
                        None => (None, AnnotationTable::empty()),
                        Some(type_ref) => match self.resolve_executable_schema(type_ref) {
                            Some(schema) => (
                                Some(schema.ty().clone()),
                                schema.annotations().clone(),
                            ),
                            None => continue,
                        },
                    };
                    let direction = match port.direction {
                        crate::compiler::syntax::PortDirection::Input => Direction::Input,
                        crate::compiler::syntax::PortDirection::Output => Direction::Output,
                    };
                    // Conventional boundary spellings are pre-bound in
                    // direction. Every declaration is materialized; an
                    // annotation additionally contributes a real source type
                    // constraint, never an implicit `variant` fallback.
                    let implicit = match port.name.text.as_str() {
                        "in" => Some(Direction::Input),
                        "out" | "err" => Some(Direction::Output),
                        _ => None,
                    };
                    if let Some(known) = implicit {
                        if known != direction {
                            self.bag.push(
                                Diagnostic::new(
                                    E_PORT_CONFLICT,
                                    Phase::Check,
                                    format!(
                                        "`{}` is already {} port of the graph; it cannot also be {}",
                                        port.name.text,
                                        direction_words(known),
                                        direction_words(direction),
                                    ),
                                )
                                .with_span(port.name.span)
                                .with_field("port", port.name.text.clone()),
                            );
                        } else if !redeclared_defaults.insert(port.name.text.clone()) {
                            self.bag.push(
                                Diagnostic::new(
                                    E_PORT_CONFLICT,
                                    Phase::Check,
                                    format!(
                                        "the boundary port `{}` is already declared",
                                        port.name.text
                                    ),
                                )
                                .with_span(port.name.span)
                                .with_field("port", port.name.text.clone()),
                            );
                        } else {
                            match node::Port::new(port.name.text.clone()) {
                                Ok(declared) => {
                                    graph.boundary.push(crate::plan::PortDecl {
                                        port: declared,
                                        direction,
                                        unwired: crate::plan::UnwiredPolicy::RequireWire,
                                        ty: pinned,
                                        annotations,
                                    });
                                }
                                Err(invalid) => self.bag.push(
                                    Diagnostic::new(
                                        E_PORT_UNKNOWN,
                                        Phase::Check,
                                        invalid.to_string(),
                                    )
                                    .with_span(port.name.span),
                                ),
                            }
                        }
                        continue;
                    }
                    if graph
                        .boundary
                        .iter()
                        .any(|decl| decl.port.name() == port.name.text)
                    {
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_CONFLICT,
                                Phase::Check,
                                format!(
                                    "the boundary port `{}` is already declared",
                                    port.name.text
                                ),
                            )
                            .with_span(port.name.span)
                            .with_field("port", port.name.text.clone()),
                        );
                        continue;
                    }
                    match node::Port::new(port.name.text.clone()) {
                        Ok(declared) => {
                            graph.boundary.push(crate::plan::PortDecl {
                                port: declared,
                                direction,
                                unwired: crate::plan::UnwiredPolicy::RequireWire,
                                ty: pinned,
                                annotations,
                            });
                        }
                        Err(invalid) => self.bag.push(
                            Diagnostic::new(E_PORT_UNKNOWN, Phase::Check, invalid.to_string())
                                .with_span(port.name.span),
                        ),
                    }
                }
                GraphItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some((value, annotations)) =
                            self.graph_val(val, &signature, &consts)
                    {
                        let Some(ty) = self.constant_binding_type(&value, val.span) else {
                            continue;
                        };
                        signature = signature.with(val.name.text.clone(), ty);
                        consts.insert(val.name.text.clone(), value.value.clone());
                        graph.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                            annotations,
                        });
                    }
                }
                GraphItem::Var(var) => self.bag.push(
                    Diagnostic::new(
                        E_GRAPH_STATE,
                        Phase::Check,
                        "a Graph is a scope and cannot declare private `var` state; move the state and its handlers into an Operator child",
                    )
                    .with_span(var.span)
                    .with_field("owner", path.to_string())
                    .with_field("name", var.name.text.clone()),
                ),
                GraphItem::Operator(_) | GraphItem::Connector(_) | GraphItem::Graph(_) => {
                    // Resolve the Graph's complete boundary and Loop entry
                    // before checking descendant handlers. Preserve each
                    // declaration's lexical constant environment.
                    children.push((item, signature.clone(), consts.clone()));
                }
                GraphItem::Edge(edge) => edges.push(edge),
                GraphItem::RemovedPartition(partition) => self.instance_refusal(
                    partition.span,
                    "`partition` was unpublished syntax; declare `instance` and `lifetime` on the Node that owns them",
                ),
                GraphItem::Defaults(_) => self.deferred("a defaults block", decl.span),
                GraphItem::Attribute(attribute) => {
                    if !is_common_node_attribute(attribute) && !is_loop_attribute(attribute) {
                        self.deferred("a graph attribute", attribute.span);
                    }
                }
                GraphItem::Handler(handler) => handlers.push(handler),
                GraphItem::Error(_) => {}
            }
        }
        let owns_loop = graph.extensions.contains(&node::Extension::Loop);
        let legacy_graph_handlers = !handlers.is_empty();
        graph.loop_spec =
            self.loop_declaration(decl, &graph, &signature, &consts, legacy_graph_handlers);
        for (item, mut child_signature, child_consts) in children {
            if !legacy_graph_handlers && let Some(spec) = &graph.loop_spec {
                let state_type = graph
                    .boundary
                    .iter()
                    .find(|port| port.port == spec.entry)
                    .and_then(|port| port.ty.clone())
                    .unwrap_or(DataType::Variant);
                child_signature = loop_state_signature(&child_signature, state_type);
            }
            match item {
                GraphItem::Operator(operator) => {
                    if let Some(node) =
                        self.operator(operator, &path, &child_signature, &child_consts)
                    {
                        self.push_node(&mut graph, node, operator.name.span);
                    }
                }
                GraphItem::Connector(connector) => {
                    if let Some(node) =
                        self.connector(connector, &path, &child_signature, &child_consts)
                    {
                        self.push_node(&mut graph, node, connector.name.span);
                    }
                }
                GraphItem::Graph(nested) => {
                    let mut segments = path.segments().to_vec();
                    segments.push(nested.name.text.clone());
                    match crate::plan::OwnerPath::new(segments) {
                        Ok(nested_path) => {
                            let checked = self.graph_at(
                                nested,
                                nested_path,
                                &child_signature,
                                &child_consts,
                                false,
                            );
                            self.push_graph(&mut graph, checked, nested.name.span);
                        }
                        Err(invalid) => self.bag.push(
                            Diagnostic::new(E_RESOLVE_UNKNOWN, Phase::Resolve, invalid.to_string())
                                .with_span(nested.name.span),
                        ),
                    }
                }
                _ => {}
            }
        }
        if owns_loop {
            if legacy_graph_handlers
                && graph.loop_spec.is_some()
                && self.contribute_loop_boundary(&mut graph, decl.span)
            {
                let state_type = graph
                    .boundary
                    .iter()
                    .find(|port| port.port.name() == "in" && port.direction == Direction::Input)
                    .and_then(|port| port.ty.clone())
                    .unwrap_or(DataType::Variant);
                let loop_signature = loop_state_signature(&signature, state_type);
                for handler in handlers {
                    if let Some(checked) =
                        self.graph_handler(handler, &graph, &loop_signature, &consts)
                    {
                        graph.handlers.push(checked);
                    }
                }
                if graph
                    .loop_spec
                    .as_ref()
                    .is_some_and(|spec| spec.proof.is_some())
                    && !self.complete_loop_proof(&mut graph, decl.span)
                {
                    graph.loop_spec = None;
                }
            }
        } else {
            for handler in handlers {
                self.bag.push(
                    Diagnostic::new(
                        E_GRAPH_HANDLER,
                        Phase::Check,
                        "a Graph cannot own application handlers; move the handler into an Operator child and wire its ports",
                    )
                    .with_span(handler.span)
                    .with_field("owner", path.to_string()),
                );
            }
        }
        for edge in edges {
            let Some(resolved) = self.edge(edge, &graph) else {
                continue;
            };
            if let Some((rule, code, detail, case)) = resolved.rules.iter().find_map(|rule| {
                graph
                    .routes
                    .iter()
                    .find_map(|existing| route_conflict(existing, rule))
                    .map(|(code, detail, case)| (rule, code, detail, case))
            }) {
                self.bag.push(
                    Diagnostic::new(code, Phase::Check, detail)
                        .with_span(edge.span)
                        .with_field("source", endpoint_display(&rule.source))
                        .with_field("case", case),
                );
                continue;
            }
            graph.routes.extend(resolved.rules);
            for candidate in resolved.edges {
                if self.reject_ack_response_edge(&graph, &candidate, edge.span) {
                    continue;
                }
                if self.reject_loop_next_edge(&graph, &candidate, edge.span) {
                    continue;
                }
                if !graph.edges.contains(&candidate) {
                    graph.edges.push(candidate);
                }
            }
        }
        self.materialize_boundary_ports(&mut graph);
        self.materialize_node_edge_ports(&mut graph);
        self.reject_cycles(&graph, decl.span);
        self.warn_unwired_outputs(&graph);
        graph
    }

    fn push_node(&mut self, graph: &mut Graph, node: Node, span: Span) {
        let name = node
            .path
            .segments()
            .last()
            .cloned()
            .unwrap_or_else(|| node.path.to_string());
        if matches!(name.as_str(), "meta" | "data" | "sys" | "app" | "ext") {
            self.bag.push(
                Diagnostic::new(
                    E_NAME_RESERVED,
                    Phase::Resolve,
                    format!(
                        "`{name}` is a reserved word — it names an envelope surface, never a node"
                    ),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else if !self.guard_binding_name(
            &crate::compiler::syntax::Name {
                text: name.clone(),
                span,
            },
            "a node",
        ) {
            // The one naming policy spoke.
        } else if graph.nodes.iter().any(|existing| {
            existing
                .path
                .segments()
                .last()
                .is_some_and(|part| part == &name)
        }) || graph.graphs.iter().any(|existing| existing.name == name)
        {
            self.bag.push(
                Diagnostic::new(
                    E_DUP_NODE,
                    Phase::Resolve,
                    format!("the node name `{name}` is already declared in this graph"),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else if matches!(name.as_str(), "in" | "out")
            || graph.boundary.iter().any(|port| port.port.name() == name)
        {
            self.bag.push(
                Diagnostic::new(
                    E_NAME_CLASH,
                    Phase::Resolve,
                    format!(
                        "`{name}` already names a Graph port; ports and children share one namespace"
                    ),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else {
            graph.nodes.push(node);
        }
    }

    fn push_graph(&mut self, graph: &mut Graph, nested: Graph, span: Span) {
        let name = nested.name.clone();
        if graph.graphs.iter().any(|existing| existing.name == name)
            || graph.nodes.iter().any(|existing| {
                existing
                    .path
                    .segments()
                    .last()
                    .is_some_and(|part| part == &name)
            })
        {
            self.bag.push(
                Diagnostic::new(
                    E_DUP_NODE,
                    Phase::Resolve,
                    format!("the node name `{name}` is already declared in this graph"),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else if matches!(name.as_str(), "in" | "out")
            || graph.boundary.iter().any(|port| port.port.name() == name)
        {
            self.bag.push(
                Diagnostic::new(
                    E_NAME_CLASH,
                    Phase::Resolve,
                    format!(
                        "`{name}` already names a Graph port; ports and children share one namespace"
                    ),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else {
            graph.graphs.push(nested);
        }
    }

    fn queue_instance_attributes<'source>(
        &mut self,
        owner: InstanceOwner,
        attributes: impl Iterator<Item = &'source SynAttribute>,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
        span: Span,
    ) {
        let mut instance = None;
        let mut lifetime = None;
        let mut closing = None;
        for attribute in attributes {
            let Some(name) = common_node_attribute_name(attribute) else {
                continue;
            };
            if attribute.target.segments.len() != 1 {
                self.instance_refusal(
                    attribute.span,
                    &format!(
                        "`{name}` is one atomic composite attribute; write `{name} = {{ ... }}`"
                    ),
                );
                continue;
            }
            match name {
                "instances" => {
                    self.instance_refusal(
                        attribute.span,
                        "`instances` was superseded; write the singular `instance = { key = <expr> }`",
                    );
                }
                "instance" if instance.replace(attribute).is_some() => {
                    self.instance_refusal(attribute.span, "`instance` is declared twice");
                }
                "lifetime" if lifetime.replace(attribute).is_some() => {
                    self.instance_refusal(attribute.span, "`lifetime` is declared twice");
                }
                "closing" if closing.replace(attribute).is_some() => {
                    self.instance_refusal(attribute.span, "`closing` is declared twice");
                }
                _ => {}
            }
        }

        let key = match instance {
            Some(attribute) => match self.instance_key_source(attribute) {
                Some(key) => Some(key),
                None => return,
            },
            None => None,
        };
        let lifetime = match lifetime {
            Some(attribute) => match self.instance_lifetime_source(attribute) {
                Some(lifetime) => lifetime,
                None => return,
            },
            None => InstanceLifetimeSource::Persistent,
        };
        let closing = match closing {
            Some(attribute) => match self.instance_closing_source(attribute) {
                Some(closing) => Some(closing),
                None => return,
            },
            None => None,
        };
        if key.is_none()
            && matches!(lifetime, InstanceLifetimeSource::Persistent)
            && closing.is_none()
        {
            return;
        }
        self.pending_instances.push(PendingInstance {
            owner,
            key,
            lifetime,
            closing,
            signature: signature.clone(),
            consts: consts.clone(),
            span,
        });
    }

    fn instance_closing_source(&mut self, attribute: &SynAttribute) -> Option<ClosingSource> {
        let SynValue::Braced(body) = &attribute.value else {
            self.instance_refusal(
                attribute.span,
                "`closing` must be `closing = { drain_for = <duration>; settle_for = <duration> }`",
            );
            return None;
        };
        let mut fields: BTreeMap<String, (&SynValue, Span)> = BTreeMap::new();
        for entry in &body.entries {
            let StructEntry::Field { key, value } = entry else {
                self.instance_refusal(
                    attribute.span,
                    "`closing` admits fields, not nested configuration blocks",
                );
                return None;
            };
            if key.segments.len() != 1 {
                self.instance_refusal(
                    key.span,
                    "closing fields are single names inside one atomic composite",
                );
                return None;
            }
            let name = key.segments[0].text.clone();
            if fields.insert(name.clone(), (value, key.span)).is_some() {
                self.instance_refusal(key.span, &format!("`closing.{name}` is declared twice"));
                return None;
            }
        }
        let Some((drain_for, drain_span)) = fields.remove("drain_for") else {
            self.instance_refusal(attribute.span, "`closing` requires `drain_for`");
            return None;
        };
        let Some((settle_for, settle_span)) = fields.remove("settle_for") else {
            self.instance_refusal(attribute.span, "`closing` requires `settle_for`");
            return None;
        };
        if let Some((unexpected, (_, span))) = fields.into_iter().next() {
            self.instance_refusal(span, &format!("`closing` has no `{unexpected}` field"));
            return None;
        }
        if matches!(drain_for, SynValue::Error(_)) || matches!(settle_for, SynValue::Error(_)) {
            return None;
        }
        Some(ClosingSource {
            drain_for: drain_for.clone(),
            drain_span,
            settle_for: settle_for.clone(),
            settle_span,
        })
    }

    fn instance_key_source(&mut self, attribute: &SynAttribute) -> Option<SynValue> {
        let SynValue::Braced(body) = &attribute.value else {
            self.instance_refusal(
                attribute.span,
                "`instance` must be a composite attribute: `instance = { key = <expr> }`",
            );
            return None;
        };
        let mut key = None;
        for entry in &body.entries {
            let StructEntry::Field { key: field, value } = entry else {
                self.instance_refusal(
                    attribute.span,
                    "`instance` admits only the field `key = <expr>`",
                );
                return None;
            };
            if field.segments.len() != 1 || field.segments[0].text != "key" {
                self.instance_refusal(
                    field.span,
                    "`instance` admits only the field `key = <expr>`",
                );
                return None;
            }
            if key.replace(value.clone()).is_some() {
                self.instance_refusal(field.span, "`instance.key` is declared twice");
                return None;
            }
        }
        match key {
            Some(SynValue::Error(_)) => None,
            Some(key) => Some(key),
            None => {
                self.instance_refusal(
                    attribute.span,
                    "`instance` requires exactly one `key` expression",
                );
                None
            }
        }
    }

    fn instance_lifetime_source(
        &mut self,
        attribute: &SynAttribute,
    ) -> Option<InstanceLifetimeSource> {
        let SynValue::Braced(body) = &attribute.value else {
            self.instance_refusal(
                attribute.span,
                "`lifetime` must be a composite attribute with a `kind` field",
            );
            return None;
        };
        let mut fields: BTreeMap<String, (&SynValue, Span)> = BTreeMap::new();
        for entry in &body.entries {
            let StructEntry::Field { key, value } = entry else {
                self.instance_refusal(
                    attribute.span,
                    "`lifetime` admits fields, not nested configuration blocks",
                );
                return None;
            };
            if key.segments.len() != 1 {
                self.instance_refusal(
                    key.span,
                    "lifetime fields are single names inside one atomic composite",
                );
                return None;
            }
            let name = key.segments[0].text.clone();
            if fields.insert(name.clone(), (value, key.span)).is_some() {
                self.instance_refusal(key.span, &format!("`lifetime.{name}` is declared twice"));
                return None;
            }
        }
        let Some((kind, kind_span)) = fields.remove("kind") else {
            self.instance_refusal(attribute.span, "`lifetime` requires exactly one `kind`");
            return None;
        };
        let SynValue::Expr(kind) = kind else {
            self.instance_refusal(kind_span, "`lifetime.kind` is a bare choice word");
            return None;
        };
        if !is_bare_word(&kind.text) {
            self.instance_refusal(kind.span, "`lifetime.kind` is a bare choice word");
            return None;
        }
        let duration = |fields: &mut BTreeMap<String, (&SynValue, Span)>, name: &str| {
            fields
                .remove(name)
                .map(|(value, span)| (value.clone(), span))
        };
        let source = match kind.text.as_str() {
            "persistent" => InstanceLifetimeSource::Persistent,
            "idle" => {
                let Some((after, span)) = duration(&mut fields, "after") else {
                    self.instance_refusal(
                        attribute.span,
                        "`lifetime(kind = idle)` requires `after`",
                    );
                    return None;
                };
                InstanceLifetimeSource::Idle { after, span }
            }
            "fixed" => {
                let Some((after, span)) = duration(&mut fields, "after") else {
                    self.instance_refusal(
                        attribute.span,
                        "`lifetime(kind = fixed)` requires `after`",
                    );
                    return None;
                };
                InstanceLifetimeSource::Fixed { after, span }
            }
            "single_pass" => InstanceLifetimeSource::SinglePass,
            "tumbling" => {
                let Some((every, span)) = duration(&mut fields, "every") else {
                    self.instance_refusal(
                        attribute.span,
                        "`lifetime(kind = tumbling)` requires `every`",
                    );
                    return None;
                };
                InstanceLifetimeSource::Tumbling { every, span }
            }
            "sliding" => {
                self.deferred("a sliding lifetime", kind.span);
                return None;
            }
            "event_time" => {
                self.deferred("an event-time lifetime", kind.span);
                return None;
            }
            other => {
                self.instance_refusal(
                    kind.span,
                    &format!(
                        "unknown lifetime kind `{other}`; expected persistent, idle, fixed, single_pass, or tumbling"
                    ),
                );
                return None;
            }
        };
        if let Some((unexpected, (_, span))) = fields.into_iter().next() {
            self.instance_refusal(
                span,
                &format!(
                    "`lifetime(kind = {})` has no `{unexpected}` field",
                    kind.text
                ),
            );
            return None;
        }
        Some(source)
    }

    /// Finishes Node-owned addressing after metadata writes have sealed
    /// `meta.app`'s exact leaf types. The expression is still compiled once
    /// and enters HIR identity; only its timing in the checker is deferred.
    fn resolve_pending_instances(&mut self, graph: &mut Graph) {
        let app_fields: Vec<(String, DataType)> = self
            .app_schema
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();
        'pending: for pending in core::mem::take(&mut self.pending_instances) {
            let input_ports: Vec<crate::plan::PortDecl> = match &pending.owner {
                InstanceOwner::Graph(path) => graph_at_path(graph, path)
                    .map(|owner| {
                        owner
                            .boundary
                            .iter()
                            .filter(|port| port.direction == Direction::Input)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                InstanceOwner::Node(path) => node_at_path(graph, path)
                    .map(|node| {
                        node.ports
                            .iter()
                            .filter(|port| port.direction == Direction::Input)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let (input_type, input_awaits_inference, known_input_types) = {
                let awaits_inference = input_ports.iter().any(|port| port.ty.is_none());
                let inputs: Vec<DataType> = input_ports
                    .iter()
                    .filter_map(|port| port.ty.clone())
                    .collect();
                let ty = if awaits_inference {
                    DataType::Variant
                } else {
                    inputs.first().cloned().unwrap_or(DataType::Variant)
                };
                (ty, awaits_inference, inputs)
            };
            let ext_fields = self.ext_schema_snapshot();
            let signature_for = |data_type: DataType| {
                let mut signature = pending
                    .signature
                    .clone()
                    .with("data", data_type)
                    .with("meta", DataType::Variant)
                    // These are the current handler-facing carriers. R2
                    // transports the exact envelope schema beside the key.
                    .with("meta.sys.id", DataType::Text)
                    .with("meta.sys.parent_id", DataType::Variant)
                    .with("meta.sys.origin_id", DataType::Text)
                    .with("meta.sys.role", DataType::Text)
                    .with("meta.sys.graph_version", DataType::INT);
                for field in crate::plan::STANDARD_EXT_FIELDS {
                    if let Some(ty) = crate::plan::standard_ext_field_type(field) {
                        signature = signature.with(format!("meta.ext.{field}"), ty);
                    }
                }
                for (field, ty) in &app_fields {
                    signature = signature.with(format!("meta.app.{field}"), ty.clone());
                }
                for (field, ty) in &ext_fields {
                    signature = signature.with(format!("meta.ext.{field}"), ty.clone());
                }
                signature
            };
            let key = match &pending.key {
                Some(source) => {
                    let signature = signature_for(input_type);
                    let Some(key_tree) = self.typed_value_tree(
                        source,
                        &signature,
                        None,
                        pending.span,
                        "instance.key",
                    ) else {
                        continue;
                    };
                    let reads_data = expression_reads_data(&key_tree.root);
                    if reads_data && !input_awaits_inference {
                        let expected = key_tree.ty.clone();
                        for input in known_input_types.iter().skip(1) {
                            let Some(candidate) = self.typed_value_tree(
                                source,
                                &signature_for(input.clone()),
                                None,
                                pending.span,
                                "instance.key",
                            ) else {
                                continue 'pending;
                            };
                            if candidate.ty != expected {
                                self.instance_refusal(
                                    pending.span,
                                    &format!(
                                        "the key resolves to both {expected} and {}; one instance expression has no single type across input ports",
                                        candidate.ty
                                    ),
                                );
                                continue 'pending;
                            }
                        }
                    }
                    let awaits_port_inference = input_awaits_inference && reads_data;
                    if !key_tree.ty.is_instance_key() && !awaits_port_inference {
                        self.instance_refusal(
                            pending.span,
                            &format!(
                                "the key has type {}; instance keys use canonical scalar/enum leaves or a closed struct of those leaves",
                                key_tree.ty
                            ),
                        );
                        continue;
                    }
                    Some(key_tree)
                }
                None => None,
            };
            let lifetime = match self.resolve_instance_lifetime(
                &pending.lifetime,
                &pending.signature,
                &pending.consts,
            ) {
                Some(lifetime) => lifetime,
                None => continue,
            };
            let closing = match pending.closing.as_ref() {
                Some(source) => {
                    match self.resolve_closing(source, &pending.signature, &pending.consts) {
                        Some(closing) => Some(closing),
                        None => continue,
                    }
                }
                None => None,
            };
            let policy = InstancePolicy {
                key,
                lifetime,
                closing,
            };
            match pending.owner {
                InstanceOwner::Graph(path) => {
                    if let Some(owner) = graph_at_path_mut(graph, &path) {
                        owner.instance = Some(policy);
                    }
                }
                InstanceOwner::Node(path) => {
                    if let Some(node) = node_at_path_mut(graph, &path) {
                        node.instance = Some(policy);
                    }
                }
            }
        }
    }

    /// Closes invocation-bound Connector attributes after the application
    /// metadata schema and instance-key types are final. Constants still
    /// reduce to `AttrValue::Const`; only an expression that actually reads
    /// invocation or instance context travels as an expression.
    fn resolve_pending_connector_attributes(&mut self, graph: &mut Graph) {
        let app_fields: Vec<(String, DataType)> = self
            .app_schema
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();
        let ext_fields = self.ext_schema_snapshot();
        for pending in core::mem::take(&mut self.pending_connector_attributes) {
            let Some(node) = node_at_path(graph, &pending.node) else {
                continue;
            };
            let contract = node.connector_contract_id().cloned();
            let Some(input_type) = node
                .ports
                .iter()
                .find(|port| port.direction == Direction::Input && port.port.name() == "in")
                .and_then(|port| port.ty.clone())
            else {
                // An invalid/deferred profile already spoke. Do not add a
                // speculative value diagnostic against a recovery port row.
                continue;
            };
            let instance_key_type = node
                .instance
                .as_ref()
                .and_then(|policy| policy.key.as_ref())
                .map(|key| key.ty.clone());
            let mut signature = pending
                .signature
                .clone()
                .with("data", input_type)
                .with("meta", DataType::Variant)
                .with("meta.sys.id", DataType::Text)
                .with("meta.sys.parent_id", DataType::Variant)
                .with("meta.sys.origin_id", DataType::Text)
                .with("meta.sys.role", DataType::Text)
                .with("meta.sys.graph_version", DataType::INT);
            for field in crate::plan::STANDARD_EXT_FIELDS {
                if let Some(ty) = crate::plan::standard_ext_field_type(field) {
                    signature = signature.with(format!("meta.ext.{field}"), ty);
                }
            }
            if let Some(key_type) = instance_key_type {
                signature = signature.with("instance.key", key_type);
            }
            for (field, ty) in &app_fields {
                signature = signature.with(format!("meta.app.{field}"), ty.clone());
            }
            for (field, ty) in &ext_fields {
                signature = signature.with(format!("meta.ext.{field}"), ty.clone());
            }
            let expected = pending.domain.expression_type();
            let boot_reference = match &pending.value {
                SynValue::Expr(source) => match expr::parse_boot_reference(&source.text) {
                    Ok(reference) => reference,
                    Err(diagnostics) => {
                        let base = source.span.range().start;
                        for diagnostic in diagnostics {
                            self.bag.push(
                                Diagnostic::new(
                                    E_CONTRACT_ATTR,
                                    Phase::Check,
                                    diagnostic.message.clone(),
                                )
                                .with_span(Span::new(
                                    source.span.source(),
                                    base + diagnostic.range.start..base + diagnostic.range.end,
                                ))
                                .with_field("attribute", pending.name.clone())
                                .with_field("detail", diagnostic.message),
                            );
                        }
                        continue;
                    }
                },
                _ => None,
            };
            let (tree, forced_site) = if let Some(reference) = boot_reference {
                let Some(contract) = contract.as_ref() else {
                    continue;
                };
                let admitted =
                    crate::boundary::connector_boot_reference_policy(contract, &pending.name)
                        .is_some_and(|policy| policy.admits(&reference));
                if !admitted {
                    let function = match reference {
                        crate::deployment::BootReference::Environment(_) => "env",
                        crate::deployment::BootReference::Secret(_) => "secret",
                    };
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "`{function}()` is not authorized for the `{}` setting of `{contract}`",
                                pending.name
                            ),
                        )
                        .with_span(pending.span)
                        .with_field("attribute", pending.name.clone())
                        .with_field("detail", "undeclared boot reference"),
                    );
                    continue;
                }
                let Some(expected) = expected.clone() else {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!("`{}` has no boot-bindable value type", pending.name),
                        )
                        .with_span(pending.span)
                        .with_field("attribute", pending.name.clone())
                        .with_field("detail", "boot reference type"),
                    );
                    continue;
                };
                (
                    expr::boot_reference_tree(&reference, expected),
                    Some(crate::plan::EvaluationSite::Boot),
                )
            } else {
                let Some(tree) = self.typed_value_tree(
                    &pending.value,
                    &signature,
                    expected.as_ref(),
                    pending.span,
                    &pending.name,
                ) else {
                    continue;
                };
                (tree, None)
            };
            if pending.domain == contract::ValueDomain::ExprPath
                && !matches!(
                    &tree.root,
                    expr::Expr::Path(path)
                        if path.first().is_some_and(|root| matches!(root.as_str(), "data" | "meta"))
                )
            {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!(
                            "`{}` must be one direct `data` or `meta` path, not a computed expression",
                            pending.name
                        ),
                    )
                    .with_span(pending.span)
                    .with_field("attribute", pending.name.clone())
                    .with_field("detail", "correlation path"),
                );
                continue;
            }
            if !pending.domain.admits_type(&tree.ty) {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!(
                            "`{}` has type {}, outside the {:?} contract domain",
                            pending.name, tree.ty, pending.domain
                        ),
                    )
                    .with_span(pending.span)
                    .with_field("attribute", pending.name.clone())
                    .with_field("detail", "attribute expression type"),
                );
                continue;
            }
            let site = forced_site.unwrap_or_else(|| expression_evaluation_site(&tree.root));
            if !pending.latest_site.admits(site) {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!(
                            "`{}` closes at {site:?}, after the sheet's {:?} ceiling",
                            pending.name, pending.latest_site
                        ),
                    )
                    .with_span(pending.span)
                    .with_field("attribute", pending.name.clone())
                    .with_field("detail", "attribute evaluation phase"),
                );
                continue;
            }
            let value = if site == crate::plan::EvaluationSite::Plan {
                let Some(constant) = self.reduce_constant(tree, pending.span, &pending.consts)
                else {
                    continue;
                };
                if !pending.domain.admits(&constant.value) {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "`{}` does not fit the {:?} contract domain",
                                pending.name, pending.domain
                            ),
                        )
                        .with_span(pending.span)
                        .with_field("attribute", pending.name.clone())
                        .with_field("detail", "attribute value domain"),
                    );
                    continue;
                }
                AttrValue::Const(constant)
            } else {
                AttrValue::Expr { site, tree }
            };
            if let Some(node) = node_at_path_mut(graph, &pending.node)
                && let NodeSemantics::Connector { attributes, .. } = &mut node.semantics
            {
                attributes.push(Attribute {
                    name: pending.name,
                    value,
                });
            }
        }
    }

    fn resolve_instance_lifetime(
        &mut self,
        source: &InstanceLifetimeSource,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<InstanceLifetime> {
        let duration = |checker: &mut Self, value: &SynValue, span: Span| {
            let tree = checker.typed_value_tree(
                value,
                signature,
                Some(&DataType::DURATION),
                span,
                "lifetime duration",
            )?;
            let constant = checker.reduce_constant(tree, span, consts)?;
            match constant.value {
                crate::value::Value::Duration(nanos) if nanos > 0 => Some(constant),
                _ => {
                    checker.instance_refusal(span, "a lifetime duration must be positive");
                    None
                }
            }
        };
        Some(match source {
            InstanceLifetimeSource::Persistent => InstanceLifetime::Persistent,
            InstanceLifetimeSource::Idle { after, span } => {
                InstanceLifetime::Idle(duration(self, after, *span)?)
            }
            InstanceLifetimeSource::Fixed { after, span } => {
                InstanceLifetime::Fixed(duration(self, after, *span)?)
            }
            InstanceLifetimeSource::SinglePass => InstanceLifetime::SinglePass,
            InstanceLifetimeSource::Tumbling { every, span } => {
                InstanceLifetime::Tumbling(duration(self, every, *span)?)
            }
        })
    }

    fn resolve_closing(
        &mut self,
        source: &ClosingSource,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<ClosingPolicy> {
        let duration = |checker: &mut Self, value: &SynValue, span: Span, name: &str| {
            let tree = checker.typed_value_tree(
                value,
                signature,
                Some(&DataType::DURATION),
                span,
                name,
            )?;
            let constant = checker.reduce_constant(tree, span, consts)?;
            match constant.value {
                crate::value::Value::Duration(nanos) if nanos > 0 => Some(constant),
                _ => {
                    checker.instance_refusal(span, &format!("`closing.{name}` must be positive"));
                    None
                }
            }
        };
        Some(ClosingPolicy {
            drain_for: duration(self, &source.drain_for, source.drain_span, "drain_for")?,
            settle_for: duration(self, &source.settle_for, source.settle_span, "settle_for")?,
        })
    }

    fn ext_schema_snapshot(&self) -> Vec<(String, DataType)> {
        self.ext_fields
            .iter()
            .map(|field| (field.clone(), DataType::Variant))
            .collect()
    }

    fn instance_refusal(&mut self, span: Span, detail: &str) {
        self.bag.push(
            Diagnostic::new(E_INSTANCE, Phase::Check, detail)
                .with_span(span)
                .with_field("detail", detail),
        );
    }

    fn graph_val(
        &mut self,
        val: &ValDecl,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<(Const, AnnotationTable)> {
        let (expected, annotations) = match &val.ty {
            Some(ty) => {
                let schema = self.resolve_executable_schema(ty)?;
                (Some(schema.ty().clone()), schema.annotations().clone())
            }
            None => (None, AnnotationTable::empty()),
        };
        let tree = self.typed_value_tree(
            &val.value,
            signature,
            expected.as_ref(),
            val.span,
            &val.name.text,
        )?;
        self.reduce_constant(tree, val.span, consts)
            .map(|value| (value, annotations))
    }

    fn operator(
        &mut self,
        decl: &OperatorDecl,
        owner: &crate::plan::OwnerPath,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Node> {
        if decl.removed_per.is_some() {
            self.instance_refusal(
                decl.span,
                "`per` was unpublished syntax; write `instance = { key = <expr> }` inside the Operator",
            );
        }
        let extensions = self.operator_annotation(decl);
        let path = self.node_path(owner, &decl.name.text, decl.name.span)?;
        let mut node = Node {
            path,
            extensions,
            loop_spec: None,
            semantics: NodeSemantics::Operator,
            vals: Vec::new(),
            vars: Vec::new(),
            ports: Vec::new(),
            handlers: Vec::new(),
            instance: None,
        };
        self.node_body(
            &decl.body,
            &mut node,
            graph_signature,
            graph_consts,
            false,
            decl.span,
        );
        self.queue_instance_attributes(
            InstanceOwner::Node(node.path.clone()),
            decl.body.iter().filter_map(|item| match item {
                OperatorItem::Attribute(attribute) => Some(attribute),
                _ => None,
            }),
            graph_signature,
            graph_consts,
            decl.span,
        );
        Some(node)
    }

    fn connector(
        &mut self,
        decl: &ConnectorDecl,
        owner: &crate::plan::OwnerPath,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Node> {
        if decl.removed_per.is_some() {
            self.instance_refusal(
                decl.span,
                "`per` was unpublished syntax; write `instance = { key = <expr> }` inside the Connector",
            );
        }
        let mut symbols = decl.annotation.symbols.iter();
        let type_symbol = symbols.next()?;
        if node::Extension::from_source_name(&type_symbol.text).is_some()
            || matches!(type_symbol.text.as_str(), "App" | "Graph" | "Operator")
        {
            self.node_annotation_refusal(
                type_symbol,
                "a Connector requires its exact Contract first, before every Extension",
            );
            return None;
        }
        if type_symbol.text == "Clock" {
            self.bag.push(
                Diagnostic::new(
                    E_CONNECTOR_UNAVAILABLE,
                    Phase::Check,
                    "`Clock` was renamed to `Timer`; use `Timer` for one-shot or periodic events",
                )
                .with_span(type_symbol.span)
                .with_field("type", type_symbol.text.clone())
                .with_field("replacement", "Timer"),
            );
            return None;
        }
        let (contract_id, roles, exchange) = match self.registry.connector(&type_symbol.text) {
            contract::Availability::Available(contract) => {
                let contract_id = match crate::connector::ContractId::parse(contract.id) {
                    Ok(contract_id) => contract_id,
                    Err(error) => {
                        self.bag.push(
                            Diagnostic::new(
                                E_CONNECTOR_UNAVAILABLE,
                                Phase::Check,
                                format!(
                                    "the `{}` contract carries an invalid semantic id: {error}",
                                    type_symbol.text
                                ),
                            )
                            .with_span(type_symbol.span)
                            .with_field("type", type_symbol.text.clone())
                            .with_field("detail", error.to_string()),
                        );
                        return None;
                    }
                };
                (contract_id, contract.roles, contract.exchange)
            }
            contract::Availability::Declared => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!(
                            "`{}` is declared in the catalogue; its Driver is not in this build",
                            type_symbol.text
                        ),
                    )
                    .with_span(type_symbol.span)
                    .with_field("type", type_symbol.text.clone()),
                );
                return None;
            }
            contract::Availability::Unknown => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!("`{}` is not a known Connector Contract", type_symbol.text),
                    )
                    .with_span(type_symbol.span)
                    .with_field("type", type_symbol.text.clone()),
                );
                return None;
            }
        };
        let extensions = self.connector_extensions(symbols.as_slice());
        let path = self.node_path(owner, &decl.name.text, decl.name.span)?;
        let mut node = Node {
            path,
            extensions,
            loop_spec: None,
            semantics: NodeSemantics::Connector {
                contract: contract_id,
                roles,
                exchange,
                attributes: Vec::new(),
            },
            vals: Vec::new(),
            vars: Vec::new(),
            ports: Vec::new(),
            handlers: Vec::new(),
            instance: None,
        };
        self.node_body(
            &decl.body,
            &mut node,
            graph_signature,
            graph_consts,
            true,
            decl.span,
        );
        self.contract(&mut node, decl);
        node.loop_spec =
            self.node_loop_declaration(&decl.body, &node, graph_signature, graph_consts, decl.span);
        self.queue_instance_attributes(
            InstanceOwner::Node(node.path.clone()),
            decl.body.iter().filter_map(|item| match item {
                OperatorItem::Attribute(attribute) => Some(attribute),
                _ => None,
            }),
            graph_signature,
            graph_consts,
            decl.span,
        );
        Some(node)
    }

    /// Validates a connector declaration against its contract sheet
    /// (D-2026-08-12-1): resolve the contract, resolve the PROFILE
    /// through the selector (written or defaulted), validate the
    /// written attributes against the composed sheet, then inject
    /// the contract's defaults into the effective HIR — BEFORE
    /// canonicalization, so omission and the exact spelling of a
    /// default are one identity.
    fn contract(&mut self, node: &mut Node, decl: &ConnectorDecl) {
        let Some(contract_id) = node.connector_contract_id() else {
            self.node_annotation_refusal(
                &decl.annotation.symbols[0],
                "a Connector declaration did not resolve to a Contract",
            );
            return;
        };
        let Some(contract) = self.registry.connector_by_id(contract_id) else {
            self.bag.push(
                Diagnostic::new(
                    E_CONNECTOR_UNAVAILABLE,
                    Phase::Check,
                    format!("the `{contract_id}` contract is absent from this compiler registry"),
                )
                .with_span(decl.annotation.symbols[0].span)
                .with_field("type", contract_id.to_string()),
            );
            return;
        };
        // Name, direction and sink policy do not depend on attribute
        // values. Materialize that part immediately so a bad selector or
        // attribute keeps its one truthful diagnostic instead of causing
        // downstream "unknown port" cascades. A successful resolution below
        // replaces these recovery rows with the effective types.
        let mut recovery_ports = Vec::with_capacity(contract.ports.len());
        for declared in contract.ports {
            let port = match node::Port::new(declared.name) {
                Ok(port) => port,
                Err(error) => {
                    self.bag.push(
                        Diagnostic::new(E_CONTRACT_ATTR, Phase::Check, error.to_string())
                            .with_span(decl.span)
                            .with_field("detail", error.to_string()),
                    );
                    return;
                }
            };
            recovery_ports.push(crate::plan::PortDecl {
                port,
                direction: declared.direction,
                unwired: declared.unwired,
                ty: None,
                annotations: AnnotationTable::empty(),
            });
        }
        node.ports = recovery_ports;
        let span_of = |name: &str| {
            decl.body
                .iter()
                .find_map(|item| match item {
                    OperatorItem::Attribute(attribute)
                        if attribute.target.segments.len() == 1
                            && attribute.target.segments[0].text == name =>
                    {
                        Some(attribute.span)
                    }
                    _ => None,
                })
                .unwrap_or(decl.span)
        };
        let refuse = |bag: &mut Bag, name: &str, span: Span, detail: String| {
            bag.push(
                Diagnostic::new(
                    E_CONTRACT_ATTR,
                    Phase::Check,
                    format!("`{name}` violates the {} sheet: {detail}", contract.name),
                )
                .with_span(span)
                .with_field("attribute", name.to_string())
                .with_field("detail", detail),
            );
        };
        // PRESENCE is three-state (D-2026-08-12-1, review round):
        // absent / present-valid / present-INVALID. The written set
        // is SYNTACTIC — an attribute whose value failed to check is
        // still WRITTEN: it is neither "missing" (no Required
        // refusal) nor defaultable (no injection into an already
        // fautive compilation).
        let written_names: std::collections::BTreeSet<String> = decl
            .body
            .iter()
            .filter_map(|item| match item {
                OperatorItem::Attribute(attribute)
                    if !is_common_node_attribute(attribute) && !is_loop_attribute(attribute) =>
                {
                    Some(
                        attribute
                            .target
                            .segments
                            .iter()
                            .map(|segment| segment.text.as_str())
                            .collect::<Vec<_>>()
                            .join("."),
                    )
                }
                _ => None,
            })
            .collect();
        // 1. The PROFILE: the selector's written word, or its
        // default. The admitted words are derived from the profiles.
        let mut profile: Option<&contract::Profile> = None;
        let mut selector_written = false;
        // The selector itself is three-state: WRITTEN-BUT-INVALID
        // (its value failed to check, so it reached no HIR) selects
        // NO implicit profile — the compiler never pretends
        // `mode = job` about a mode the author wrote and got wrong
        // (D-2026-08-12-1, review round).
        let mut selector_invalid = false;
        if let Some(selector) = &contract.selector {
            let words: Vec<&str> = selector
                .profiles
                .iter()
                .map(|profile| profile.word)
                .collect();
            let written = node
                .attributes()
                .iter()
                .find(|attribute| attribute.name == selector.attribute)
                .cloned();
            let word: Option<String> = match &written {
                Some(attribute) => {
                    selector_written = true;
                    match &attribute.value {
                        AttrValue::Word(word) => Some(word.clone()),
                        AttrValue::Const(_) | AttrValue::Expr { .. } => {
                            refuse(
                                self.bag,
                                selector.attribute,
                                span_of(selector.attribute),
                                format!(
                                    "the profile selector expects one of {words:?} as a bare \
                                     word, not a Rastok expression"
                                ),
                            );
                            None
                        }
                    }
                }
                None if written_names.contains(selector.attribute) => {
                    selector_invalid = true;
                    None
                }
                None => selector.default.map(String::from),
            };
            match word {
                Some(word) => match contract.profile(&word) {
                    Some(found) => {
                        profile = Some(found);
                        if let contract::ProfileSupport::Deferred(reason) = found.support {
                            self.deferred(reason, span_of(selector.attribute));
                        }
                    }
                    None => {
                        refuse(
                            self.bag,
                            selector.attribute,
                            span_of(selector.attribute),
                            format!("expected one of {words:?}, got `{word}`"),
                        );
                        return;
                    }
                },
                None if selector_written || selector_invalid => return,
                None => {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_REQUIRED,
                            Phase::Check,
                            format!(
                                "the {} sheet requires `{}` (no default profile)",
                                contract.name, selector.attribute
                            ),
                        )
                        .with_span(decl.span)
                        .with_field("attribute", selector.attribute),
                    );
                    return;
                }
            }
        }
        let deferred_profile = matches!(
            profile.map(|profile| profile.support),
            Some(contract::ProfileSupport::Deferred(_))
        );
        // 2. NAME-level judgement is SYNTACTIC — an attribute whose
        // value failed to check still deserves the precise refusal
        // ("belongs to another profile" / "not on the sheet").
        for item in &decl.body {
            let OperatorItem::Attribute(attribute) = item else {
                continue;
            };
            if is_common_node_attribute(attribute) || is_loop_attribute(attribute) {
                continue;
            }
            let name = attribute
                .target
                .segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>()
                .join(".");
            if contract
                .selector
                .as_ref()
                .is_some_and(|selector| selector.attribute == name)
            {
                continue;
            }
            if contract.attribute(profile, &name).is_some() {
                continue;
            }
            if let Some(owner) = contract.owning_profile(&name) {
                let selector = contract
                    .selector
                    .as_ref()
                    .map_or("mode", |selector| selector.attribute);
                let current = profile.map_or("?", |profile| profile.word);
                refuse(
                    self.bag,
                    &name,
                    attribute.span,
                    format!(
                        "`{name}` belongs to `{contract}({selector} = {owner})` and is not \
                         applicable to `{contract}({selector} = {current})`",
                        contract = contract.name,
                        owner = owner.word,
                    ),
                );
            } else {
                refuse(
                    self.bag,
                    &name,
                    attribute.span,
                    String::from("not on the contract sheet"),
                );
            }
        }
        // 3. Every checked attribute's VALUE against its domain.
        for attribute in node.attributes() {
            if contract
                .selector
                .as_ref()
                .is_some_and(|selector| selector.attribute == attribute.name)
            {
                continue;
            }
            let span = span_of(&attribute.name);
            let Some(spec) = contract.attribute(profile, &attribute.name) else {
                // The syntactic pre-pass already refused the name.
                continue;
            };
            if deferred_profile
                && contract
                    .common
                    .iter()
                    .all(|common| common.name != attribute.name)
            {
                // A deferred profile's own attributes are declared,
                // not validated — the deferral already spoke.
                continue;
            }
            let fault: Option<String> = match (&spec.domain, &attribute.value) {
                (contract::ValueDomain::Choice(words), AttrValue::Word(word)) => {
                    if words.contains(&word.as_str()) {
                        // The runtime companions — idempotency keys
                        // and `on compensate` — are not executable in
                        // this subset; admitting the words without
                        // their invariants would be a silent lie.
                        if attribute.name == "effect"
                            && !matches!(word.as_str(), "none" | "idempotent")
                        {
                            self.deferred("compensable and irreversible effects", span);
                        }
                        None
                    } else if let [only] = words {
                        let current = profile.map_or("?", |profile| profile.word);
                        let selector = contract
                            .selector
                            .as_ref()
                            .map_or("mode", |selector| selector.attribute);
                        Some(format!(
                            "`{name}` is fixed to `{only}` for `{contract}({selector} = \
                             {current})`",
                            name = attribute.name,
                            contract = contract.name,
                        ))
                    } else {
                        Some(format!("expected one of {words:?}, got `{word}`"))
                    }
                }
                (contract::ValueDomain::Choice(words), AttrValue::Const(constant)) => {
                    match &constant.value {
                        crate::value::Value::Text(text) if words.contains(&text.as_str()) => {
                            Some(format!(
                                "expected one of {words:?} as a bare word; `\"{text}\"` is Rastok expression \
                             text — write `{text}`"
                            ))
                        }
                        _ => Some(format!("expected one of {words:?} as a bare word")),
                    }
                }
                (contract::ValueDomain::Choice(words), AttrValue::Expr { .. }) => {
                    Some(format!("expected one of {words:?} as a bare word"))
                }
                (
                    contract::ValueDomain::Text
                    | contract::ValueDomain::Bytes
                    | contract::ValueDomain::TextList
                    | contract::ValueDomain::ChoiceList(_)
                    | contract::ValueDomain::Duration
                    | contract::ValueDomain::TextMap
                    | contract::ValueDomain::TypedHttpHeaders
                    | contract::ValueDomain::CustomHttpHeaders,
                    AttrValue::Word(word),
                ) => Some(format!(
                    "expected a Rastok expression fitting the {:?} domain; `{word}` is a bare word",
                    spec.domain
                )),
                (domain, AttrValue::Const(constant)) if domain.implemented() => {
                    if domain.admits(&constant.value) {
                        None
                    } else {
                        Some(format!("the value does not fit the {domain:?} domain"))
                    }
                }
                (domain, _) => Some(format!("the value does not fit the {domain:?} domain")),
            };
            if let Some(detail) = fault {
                refuse(self.bag, &attribute.name, span, detail);
            }
        }
        // 4. Required attributes — the COMMON sheet is known and
        // fully owed even under a deferred profile; only the
        // deferred profile's own attributes wait with it.
        let composed: Vec<&contract::AttributeSpec> = contract
            .common
            .iter()
            .chain(if deferred_profile {
                &[] as &[contract::AttributeSpec]
            } else {
                profile
                    .map(|profile| profile.attributes)
                    .unwrap_or_default()
            })
            .collect();
        for spec in &composed {
            if spec.presence == contract::Presence::Required && !written_names.contains(spec.name) {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_REQUIRED,
                        Phase::Check,
                        format!("the {} sheet requires `{}`", contract.name, spec.name),
                    )
                    .with_span(decl.span)
                    .with_field("attribute", spec.name),
                );
            }
        }
        if matches!(
            contract.id,
            crate::connector::HTTP_CLIENT_CONNECTOR_ID
                | crate::connector::HTTP_SERVER_CONNECTOR_ID
                | crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID
                | crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID
        ) {
            let certificate = node
                .attributes()
                .iter()
                .any(|attribute| attribute.name == "tls_certificate");
            let private_key = node
                .attributes()
                .iter()
                .any(|attribute| attribute.name == "tls_private_key");
            if certificate != private_key {
                refuse(
                    self.bag,
                    "tls_certificate/tls_private_key",
                    decl.span,
                    String::from(
                        "client/server identity requires both certificate bytes and private-key bytes",
                    ),
                );
            }
        }
        if deferred_profile {
            // No injection: the deferral already refused the
            // compilation honestly.
            return;
        }
        // 5. Defaults become EFFECTIVE HIR (D-2026-08-12-1): the
        // selector's word first, then every absent Defaulted
        // attribute — a Choice word directly, a Rastok expression default through
        // the SAME chain as author expressions (closed signature, no
        // consts: a standard contract reads nothing).
        let mut injected: Vec<Attribute> = Vec::new();
        if let Some(selector) = &contract.selector
            && !selector_written
            && !written_names.contains(selector.attribute)
            && let Some(word) = selector.default
        {
            injected.push(Attribute {
                name: selector.attribute.to_string(),
                value: AttrValue::Word(word.to_string()),
            });
        }
        for spec in &composed {
            let contract::Presence::Defaulted(default) = &spec.presence else {
                continue;
            };
            if written_names.contains(spec.name) {
                continue;
            }
            let value = match default {
                contract::DefaultValue::Choice(word) => Some(AttrValue::Word(word.to_string())),
                contract::DefaultValue::Expression(source) => self
                    .constant(
                        &ExprSource {
                            text: (*source).to_string(),
                            span: decl.span,
                        },
                        &expr::Signature::new(),
                        &BTreeMap::new(),
                    )
                    .map(AttrValue::Const),
            };
            if let Some(value) = value {
                injected.push(Attribute {
                    name: spec.name.to_string(),
                    value,
                });
            }
        }
        if let NodeSemantics::Connector { attributes, .. } = &mut node.semantics {
            attributes.extend(injected);
        }
        let effective_words: BTreeMap<String, String> = node
            .attributes()
            .iter()
            .filter_map(|attribute| match &attribute.value {
                AttrValue::Word(word) => Some((attribute.name.clone(), word.clone())),
                AttrValue::Const(_) | AttrValue::Expr { .. } => None,
            })
            .collect();
        match contract.resolve_ports(&effective_words) {
            Ok(ports) => node.ports = ports,
            Err(error) => {
                // Missing/invalid effective choices have already received
                // their source-facing contract diagnostic above. A type
                // constructor failure is a sheet defect and cannot pass the
                // ContractCatalog door; this guard prevents an incomplete HIR if a
                // future rule violates that invariant.
                if !matches!(
                    &error,
                    contract::PortResolutionError::MissingAttribute(_)
                        | contract::PortResolutionError::UnknownChoice { .. }
                ) {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "the {} port contract did not resolve: {error}",
                                contract.name
                            ),
                        )
                        .with_span(decl.span)
                        .with_field("detail", error.to_string()),
                    );
                }
            }
        }
    }

    fn node_path(
        &mut self,
        owner: &crate::plan::OwnerPath,
        name: &str,
        span: Span,
    ) -> Option<node::Path> {
        let mut segments = owner.segments().to_vec();
        segments.push(name.to_string());
        match node::Path::new(segments) {
            Ok(path) => Some(path),
            Err(invalid) => {
                self.bag.push(
                    Diagnostic::new(E_RESOLVE_UNKNOWN, Phase::Resolve, invalid.to_string())
                        .with_span(span),
                );
                None
            }
        }
    }

    /// Returns one sheet row whose value may legitimately remain open until
    /// an invocation. A profile-local name is admitted here only when every
    /// occurrence has the same domain and phase ceiling; the later contract
    /// pass still judges whether it belongs to the selected profile.
    fn invocation_attribute_spec(
        &self,
        node: &Node,
        name: &str,
    ) -> Option<(contract::ValueDomain, crate::plan::EvaluationSite)> {
        let contract_id = node.connector_contract_id()?;
        let sheet = self.registry.connector_by_id(contract_id)?;
        let mut matches = sheet
            .common
            .iter()
            .chain(
                sheet
                    .selector
                    .iter()
                    .flat_map(|selector| selector.profiles.iter())
                    .flat_map(|profile| profile.attributes.iter()),
            )
            .filter(|spec| {
                spec.name == name
                    && spec.latest_site == crate::plan::EvaluationSite::Invocation
                    && spec.domain.implemented()
                    && !matches!(spec.domain, contract::ValueDomain::Choice(_))
            });
        let first = matches.next()?;
        matches
            .all(|spec| spec.domain == first.domain && spec.latest_site == first.latest_site)
            .then(|| (first.domain.clone(), first.latest_site))
    }

    fn node_body(
        &mut self,
        body: &[OperatorItem],
        node: &mut Node,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
        is_connector: bool,
        owner_span: Span,
    ) {
        let mut signature = graph_signature.clone();
        let mut consts = graph_consts.clone();
        // Duplicate attributes are a SYNTACTIC property: a name
        // written twice refuses even when its first value failed to
        // check (D-2026-08-12-1, review round).
        let mut written_attributes = std::collections::BTreeSet::new();
        let mut declared_names: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        // Pass one: everything except handlers, so the port table and
        // the visible bindings are complete before any handler checks.
        for item in body {
            match item {
                OperatorItem::Port(port) => {
                    if is_connector {
                        // A connector's type defines its ports; the
                        // declaration itself is the fault.
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_CONFLICT,
                                Phase::Check,
                                format!(
                                    "`{}`: a connector never declares ports — its type defines them",
                                    port.name.text
                                ),
                            )
                            .with_span(port.span)
                            .with_field("port", port.name.text.clone()),
                        );
                        continue;
                    }
                    let (pinned, annotations) = match &port.ty {
                        None => (None, AnnotationTable::empty()),
                        Some(type_ref) => match self.resolve_executable_schema(type_ref) {
                            Some(schema) => {
                                (Some(schema.ty().clone()), schema.annotations().clone())
                            }
                            None => continue,
                        },
                    };
                    let direction = match port.direction {
                        crate::compiler::syntax::PortDirection::Input => Direction::Input,
                        crate::compiler::syntax::PortDirection::Output => Direction::Output,
                    };
                    if !self.guard_binding_name(&port.name, "a port") {
                        continue;
                    }
                    // An explicit redeclaration is a conflict even
                    // when the direction agrees; an inferred repeat
                    // is not.
                    if !declared_names.insert(port.name.text.clone()) {
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_CONFLICT,
                                Phase::Check,
                                format!("the port `{}` is already declared", port.name.text),
                            )
                            .with_span(port.name.span)
                            .with_field("port", port.name.text.clone()),
                        );
                        continue;
                    }
                    self.add_port(node, &port.name, direction, pinned, "declared");
                    if let Some(decl) = node
                        .ports
                        .iter_mut()
                        .find(|decl| decl.port.name() == port.name.text)
                    {
                        decl.annotations = annotations;
                    }
                }
                OperatorItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some((value, annotations)) = self.graph_val(val, &signature, &consts)
                    {
                        let Some(ty) = self.constant_binding_type(&value, val.span) else {
                            continue;
                        };
                        signature = signature.with(val.name.text.clone(), ty);
                        consts.insert(val.name.text.clone(), value.value.clone());
                        node.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                            annotations,
                        });
                    }
                }
                OperatorItem::Var(var) => {
                    if is_connector {
                        self.bag.push(
                            Diagnostic::new(
                                E_CONNECTOR_STATE,
                                Phase::Check,
                                "a Connector's semantic state is defined by its Contract; author private `var` state in an Operator",
                            )
                            .with_span(var.span)
                            .with_field("node", node.path.to_string())
                            .with_field("name", var.name.text.clone()),
                        );
                        continue;
                    }
                    if self.bindable(&var.name, &signature)
                        && let Some(cell) = self.var(var, &signature, &consts)
                    {
                        signature = signature.with(cell.name.clone(), cell.ty.clone());
                        node.vars.push(cell);
                    }
                }
                OperatorItem::Attribute(attribute) => {
                    if is_common_node_attribute(attribute) || is_loop_attribute(attribute) {
                        continue;
                    }
                    let name = attribute
                        .target
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let invocation_spec = self.invocation_attribute_spec(node, &name);
                    if node.connector_contract_id().is_none() {
                        self.deferred("an operator attribute", attribute.span);
                    } else if !written_attributes.insert(name.clone()) {
                        self.bag.push(
                            Diagnostic::new(
                                E_CONTRACT_ATTR,
                                Phase::Check,
                                format!("`{name}` is declared twice; a repeat would win silently"),
                            )
                            .with_span(attribute.span)
                            .with_field("attribute", name)
                            .with_field("detail", "repeated attribute"),
                        );
                    } else if let Some((domain, latest_site)) = invocation_spec {
                        self.pending_connector_attributes
                            .push(PendingConnectorAttribute {
                                node: node.path.clone(),
                                name,
                                value: attribute.value.clone(),
                                domain,
                                latest_site,
                                signature: signature.clone(),
                                consts: consts.clone(),
                                span: attribute.span,
                            });
                    } else if let Some(checked) =
                        self.attribute_value(&attribute.value, &signature, &consts, attribute.span)
                        && let NodeSemantics::Connector { attributes, .. } = &mut node.semantics
                    {
                        attributes.push(Attribute {
                            name,
                            value: checked,
                        });
                    }
                }
                OperatorItem::SubBlock(block) => {
                    // A configuration sub-block is a DEAD form
                    // (D-2026-08-12-1) — refused with its migration,
                    // never a generic error.
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "`{name} {{ … }}` is no longer a configuration form; write `{name} = {{ … }}`",
                                name = block.name.text
                            ),
                        )
                        .with_span(block.span)
                        .with_field("attribute", block.name.text.clone()),
                    );
                }
                OperatorItem::Handler(_) => {}
                OperatorItem::Error(_) => {}
            }
        }
        // Pass two, operators only: handler triggers and emission
        // targets declare ports implicitly — `input`/`output` exist to
        // pin a type or publish an interface, not as a prerequisite.
        if !is_connector {
            self.infer_ports(body, node);
            node.loop_spec =
                self.node_loop_declaration(body, node, graph_signature, graph_consts, owner_span);
            if let Some(spec) = &node.loop_spec {
                let state_type = node
                    .ports
                    .iter()
                    .find(|port| port.port == spec.entry)
                    .and_then(|port| port.ty.clone())
                    .unwrap_or(DataType::Variant);
                signature = loop_state_signature(&signature, state_type);
            }
        }
        // Pass three: handlers, checked against the complete table.
        for item in body {
            if let OperatorItem::Handler(handler) = item {
                if is_connector {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONNECTOR_HANDLER,
                            Phase::Check,
                            "a Connector cannot own DSL handlers; its Contract and Driver events define its transitions",
                        )
                        .with_span(handler.span)
                        .with_field("node", node.path.to_string()),
                    );
                } else if let Some(checked) = self.handler(handler, &signature, node, &consts) {
                    if node.handlers.iter().any(|existing| {
                        same_handler_trigger(&existing.activation, &checked.activation)
                    }) {
                        self.bag.push(
                            Diagnostic::new(
                                E_HANDLER_DUPLICATE,
                                Phase::Check,
                                "an Operator has exactly one handler for each trigger; put the alternatives in one `match`",
                            )
                            .with_span(handler.span)
                            .with_field("node", node.path.to_string())
                            .with_field("trigger", checked.activation.canonical_key()),
                        );
                    } else {
                        node.handlers.push(checked);
                    }
                }
            }
        }
    }

    /// Adds one port to a node's semantic table, refusing conflicting roles.
    /// Conventional names contribute their fixed direction; an explicit type
    /// annotation is a source constraint, and matching inference never
    /// overwrites that pin.
    fn add_port(
        &mut self,
        node: &mut Node,
        name: &crate::compiler::syntax::Name,
        direction: Direction,
        pinned: Option<DataType>,
        origin: &str,
    ) {
        let default_direction = match name.text.as_str() {
            "in" => Some(Direction::Input),
            "out" | "err" => Some(Direction::Output),
            _ => None,
        };
        let table_entry = node
            .ports
            .iter()
            .position(|decl| decl.port.name() == name.text);
        let existing = table_entry.map(|index| node.ports[index].direction);
        let push = |node: &mut Node, actual_direction: Direction, ty: Option<DataType>| {
            if let Ok(port) = node::Port::new(name.text.clone()) {
                node.ports.push(crate::plan::PortDecl {
                    port,
                    direction: actual_direction,
                    unwired: if actual_direction == Direction::Output && name.text == "err" {
                        crate::plan::UnwiredPolicy::RootSink
                    } else {
                        crate::plan::UnwiredPolicy::RequireWire
                    },
                    ty,
                    annotations: AnnotationTable::empty(),
                });
                true
            } else {
                false
            }
        };
        if let Some(expected) = default_direction
            && expected != direction
        {
            // Handler/emission inference only discovers the spelling. Put
            // the conventional row in its canonical direction and let the
            // real use site emit E-PORT-DIRECTION once. An explicit source
            // declaration is itself the fault and speaks here.
            if origin != "declared" {
                if existing.is_none() {
                    let _ = push(node, expected, pinned);
                }
                return;
            }
            self.bag.push(
                Diagnostic::new(
                    E_PORT_CONFLICT,
                    Phase::Check,
                    format!(
                        "`{}` is conventionally {}; it cannot be {} ({origin})",
                        name.text,
                        direction_words(expected),
                        direction_words(direction),
                    ),
                )
                .with_span(name.span)
                .with_field("port", name.text.clone()),
            );
            return;
        }
        match existing {
            Some(known) if known != direction => {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_CONFLICT,
                        Phase::Check,
                        format!(
                            "`{}` is already {} port; it cannot also be {} ({origin})",
                            name.text,
                            direction_words(known),
                            direction_words(direction),
                        ),
                    )
                    .with_span(name.span)
                    .with_field("port", name.text.clone()),
                );
            }
            Some(_) => {
                // A declaration may COMPLETE an inferred entry with
                // its pinned type; inference never overwrites one.
                if let Some(ty) = pinned
                    && let Some(index) = table_entry
                {
                    node.ports[index].ty = Some(ty);
                }
            }
            None => {
                if !push(node, direction, pinned) {
                    self.bag.push(
                        Diagnostic::new(
                            E_PORT_UNKNOWN,
                            Phase::Check,
                            format!("`{}` is not a valid port name", name.text),
                        )
                        .with_span(name.span),
                    );
                }
            }
        }
    }

    /// Infers the ports a node's handlers imply: a trigger names an
    /// input, an emission target names an output. State cells and
    /// constants are not ports.
    fn infer_ports(&mut self, body: &[OperatorItem], node: &mut Node) {
        for item in body {
            let OperatorItem::Handler(handler) = item else {
                continue;
            };
            if let crate::compiler::syntax::Trigger::Port(name) = &handler.trigger {
                self.add_port(node, name, Direction::Input, None, "fired by a handler");
            }
            let Some(dispatch) = &handler.dispatch else {
                continue;
            };
            let mut statements = Vec::new();
            collect_syntax_statements(dispatch, &mut statements);
            for statement in statements {
                let targets: Vec<&Name> = match statement {
                    SynStatement::Assign { target, .. } if target.segments.len() == 1 => {
                        vec![&target.segments[0]]
                    }
                    SynStatement::MultiEmit { targets, .. } => targets
                        .iter()
                        .filter_map(|target| {
                            (target.segments.len() == 1).then_some(&target.segments[0])
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                for name in targets {
                    let is_state = node.vars.iter().any(|var| var.name == name.text)
                        || node.vals.iter().any(|val| val.name == name.text);
                    if !is_state {
                        self.add_port(node, name, Direction::Output, None, "an emission target");
                    }
                }
            }
        }
    }

    fn var(
        &mut self,
        decl: &VarDecl,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Var> {
        if !self.guard_binding_name(&decl.name, "a var") {
            return None;
        }
        let schema = self.resolve_executable_schema(&decl.ty)?;
        let ty = schema.ty().clone();
        let annotations = schema.annotations().clone();
        let init = match &decl.value {
            None => None,
            // The parser already spoke for its recovery form.
            Some(SynValue::Error(_)) => return None,
            Some(value) => {
                let tree =
                    self.typed_value_tree(value, signature, Some(&ty), decl.span, &decl.name.text)?;
                let constant = self.reduce_constant(tree, decl.span, consts)?;
                if !ty.accepts(&constant.value) {
                    let init_ty = match value_type(&constant.value) {
                        Ok(Some(ty)) => ty.to_string(),
                        Ok(None) | Err(_) => {
                            format!("{:?}", crate::value::ValueKind::of(&constant.value))
                        }
                    };
                    self.bag.push(
                        Diagnostic::new(
                            E_EXPR,
                            Phase::Check,
                            format!(
                                "`{}` is declared {ty}, its initializer is {init_ty}",
                                decl.name.text
                            ),
                        )
                        .with_span(decl.span)
                        .with_field("detail", format!("{init_ty} does not fit {ty}")),
                    );
                    return None;
                }
                Some(constant)
            }
        };
        Some(Var {
            name: decl.name.text.clone(),
            ty,
            init,
            annotations,
        })
    }

    /// Whether `name` may introduce a binding here: reserved handler
    /// names and shadowing are refused — which is also what keeps
    /// compile-time constant substitution sound.
    /// THE naming policy, one door for every live declaration
    /// (D-2026-08-14-1): the structural value form `nothing` and the
    /// expression literals (`true`/`false`/`null`), and the handler context
    /// function `now()` refuse in every binding-introducing position.
    /// Frontend names such as `json`
    /// remain ordinary identifiers because only the adjacent `!`
    /// selects a frontend.
    /// The refusal lands AT THE DECLARATION, one voice, the name's
    /// span.
    fn guard_binding_name(&mut self, name: &crate::compiler::syntax::Name, what: &str) -> bool {
        let reason = match name.text.as_str() {
            "nothing" => "a value form of the language",
            "true" | "false" | "null" => "a literal of the expression language",
            "now" => "the transaction-time function `now()`",
            _ => return true,
        };
        self.bag.push(
            Diagnostic::new(
                E_NAME_RESERVED,
                Phase::Resolve,
                format!(
                    "`{}` is a reserved word — it names {reason}, never {what}",
                    name.text
                ),
            )
            .with_span(name.span)
            .with_field("name", name.text.clone()),
        );
        false
    }

    /// A user type name additionally may not spell a BUILTIN type:
    /// the builtin resolves first, so the declaration would be dead
    /// code shadowed in silence (review round 3 — `type text = int`
    /// compiled to the same program as its absence).
    fn guard_type_name(&mut self, name: &crate::compiler::syntax::Name) -> bool {
        if !self.guard_binding_name(name, "a type name") {
            return false;
        }
        if !crate::compiler::parser::is_builtin_type_name(&name.text) {
            return true;
        }
        self.bag.push(
            Diagnostic::new(
                E_NAME_RESERVED,
                Phase::Resolve,
                format!(
                    "`{}` is a builtin type — a user type may not shadow it",
                    name.text
                ),
            )
            .with_span(name.span)
            .with_field("name", name.text.clone()),
        );
        false
    }

    fn bindable(
        &mut self,
        name: &crate::compiler::syntax::Name,
        signature: &expr::Signature,
    ) -> bool {
        if !self.guard_binding_name(name, "a binding") {
            return false;
        }
        let collision = if matches!(name.text.as_str(), "data" | "meta") {
            Some("a reserved handler name")
        } else if signature.get(&name.text).is_some() {
            Some("an already visible binding")
        } else {
            None
        };
        match collision {
            None => true,
            Some(reason) => {
                self.bag.push(
                    Diagnostic::new(
                        E_DUP_BINDING,
                        Phase::Check,
                        format!("`{}` shadows {reason}", name.text),
                    )
                    .with_span(name.span)
                    .with_field("name", name.text.clone()),
                );
                false
            }
        }
    }

    fn inferred_value_type(&mut self, value: &crate::value::Value, span: Span) -> Option<DataType> {
        match value_type(value) {
            Ok(Some(ty)) => Some(ty),
            Ok(None) => {
                self.bag.push(
                    Diagnostic::new(
                        E_TYPE_UNRESOLVED,
                        Phase::Check,
                        "this value has no element constraint from which to infer a type",
                    )
                    .with_span(span)
                    .with_field("type", "unsolved collection element"),
                );
                None
            }
            Err(error) => {
                self.type_constructor_refusal(span, error);
                None
            }
        }
    }

    /// Host constructors already carry the exact resolved composite type in
    /// their tree. Scalar and collection literals keep value-driven inference;
    /// an empty collection still needs contextual type information.
    fn constant_binding_type(&mut self, value: &Const, span: Span) -> Option<DataType> {
        if matches!(
            value.tree.ty,
            DataType::Struct(_) | DataType::StructuralUnion(_) | DataType::TaggedUnion(_)
        ) {
            Some(value.tree.ty.clone())
        } else {
            self.inferred_value_type(&value.value, span)
        }
    }

    fn resolve_annotated_schema(&mut self, ty: &TypeRef) -> Option<AnnotatedSchema> {
        let resolved = self.resolve_plain_type_ref(ty)?;
        let mut rows = BTreeMap::new();
        self.collect_type_annotations(ty, &resolved, &SchemaPath::root(), &mut rows)?;
        let entries = rows
            .into_iter()
            .map(|(path, annotations)| {
                AnnotationSet::new(annotations)
                    .map(|annotations| (path, annotations))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>();
        let entries = match entries {
            Ok(entries) => entries,
            Err(detail) => {
                self.annotation_refusal(ty.span, "schema", &SchemaPath::root(), &detail);
                return None;
            }
        };
        let table = match AnnotationTable::new(entries) {
            Ok(table) => table,
            Err(error) => {
                self.annotation_refusal(ty.span, "schema", &SchemaPath::root(), &error.to_string());
                return None;
            }
        };
        match AnnotatedSchema::new(resolved, table) {
            Ok(schema) => Some(schema),
            Err(error) => {
                self.annotation_refusal(ty.span, "schema", &SchemaPath::root(), &error.to_string());
                None
            }
        }
    }

    /// Resolves policy metadata at a site that can reach an executable plan.
    /// Named declarations may describe future secret-bearing schemas, but no
    /// clear secret may enter today's journal until field-level
    /// externalization can replace it with a reference before admission.
    fn resolve_executable_schema(&mut self, ty: &TypeRef) -> Option<AnnotatedSchema> {
        let schema = self.resolve_annotated_schema(ty)?;
        if crate::data_policy::contains_secret(schema.annotations()) {
            self.deferred(
                "source-executable `@secret` data before field-level externalization",
                ty.span,
            );
            return None;
        }
        Some(schema)
    }

    fn resolve_plain_type_ref(&mut self, ty: &TypeRef) -> Option<DataType> {
        let mut alternatives = Vec::with_capacity(ty.alternatives.len() + usize::from(ty.optional));
        for alternative in &ty.alternatives {
            alternatives.push(self.resolve_plain_base_type(alternative, ty.span)?);
        }
        if ty.optional {
            alternatives.push(DataType::Null);
        }
        let resolved = match alternatives.as_slice() {
            [only] => only.clone(),
            _ => match DataType::structural_union(alternatives) {
                Ok(union) => union,
                Err(error) => {
                    self.type_constructor_refusal(ty.span, error);
                    return None;
                }
            },
        };
        if let Err(error) = resolved.validate() {
            self.type_constructor_refusal(ty.span, error);
            return None;
        }
        Some(resolved)
    }

    fn resolve_plain_base_type(&mut self, ty: &BaseType, span: Span) -> Option<DataType> {
        let resolved = match ty {
            BaseType::Scalar(ScalarType::Bool) => DataType::Bool,
            BaseType::Scalar(ScalarType::Int) => DataType::INT,
            BaseType::Scalar(ScalarType::Long) => DataType::LONG,
            BaseType::Scalar(ScalarType::Uint) => DataType::UINT,
            BaseType::Scalar(ScalarType::Float) => DataType::FLOAT,
            BaseType::Scalar(ScalarType::Double) => DataType::DOUBLE,
            BaseType::Scalar(ScalarType::Number) => DataType::Number,
            BaseType::Scalar(ScalarType::Bytes) => DataType::BYTES,
            BaseType::Scalar(ScalarType::Text) => DataType::Text,
            BaseType::Scalar(ScalarType::MediaType) => DataType::MediaType,
            BaseType::Scalar(ScalarType::Null) => DataType::Null,
            BaseType::Scalar(ScalarType::Decimal { precision, scale }) => {
                let scale = match i32::try_from(*scale) {
                    Ok(scale) => scale,
                    Err(_) => {
                        self.type_constructor_refusal(
                            span,
                            TypeError::DecimalScale { scale: i32::MAX },
                        );
                        return None;
                    }
                };
                match DecimalType::new(*precision, scale) {
                    Ok(decimal) => DataType::Decimal(decimal),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::Scalar(ScalarType::Date) => {
                DataType::Temporal(TemporalType::Date(DateUnit::Day))
            }
            BaseType::Scalar(ScalarType::Timestamp) => DataType::TIMESTAMP,
            BaseType::Scalar(ScalarType::TimestampNtz) => {
                DataType::Temporal(TemporalType::TimestampNtz(TimeUnit::Nanosecond))
            }
            BaseType::Scalar(ScalarType::Time) => {
                DataType::Temporal(TemporalType::Time(TimeUnit::Nanosecond))
            }
            BaseType::Scalar(ScalarType::Uuid) => DataType::Uuid,
            BaseType::Scalar(ScalarType::Duration) => DataType::DURATION,
            BaseType::Variant => DataType::Variant,
            BaseType::Struct { fields, open } => {
                if *open {
                    self.bag.push(
                        Diagnostic::new(
                            E_TYPE_UNRESOLVED,
                            Phase::Check,
                            "an open struct is an inference constraint, not an effective type",
                        )
                        .with_span(span)
                        .with_field("type", "struct { .. }"),
                    );
                    return None;
                }
                let mut resolved = Vec::with_capacity(fields.len());
                for field in fields {
                    resolved.push(TypeField::new(
                        field.name.text.clone(),
                        self.resolve_plain_type_ref(&field.ty)?,
                    ));
                }
                match StructType::new(resolved) {
                    Ok(structure) => DataType::Struct(structure),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::Map(key, value) => {
                let key = self.resolve_plain_type_ref(key)?;
                let value = self.resolve_plain_type_ref(value)?;
                match MapType::new(key, value) {
                    Ok(map) => DataType::Map(map),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::Array(element) => DataType::array(self.resolve_plain_type_ref(element)?),
            BaseType::Table(element) => {
                let element = self.resolve_plain_type_ref(element)?;
                if !matches!(element, DataType::Struct(_)) {
                    self.bag.push(
                        Diagnostic::new(
                            E_TYPE_INVALID,
                            Phase::Check,
                            "table is a relational view of array<struct>, so its row must be a struct",
                        )
                        .with_span(span)
                        .with_field("detail", "the table row is not a struct"),
                    );
                    return None;
                }
                DataType::array(element)
            }
            BaseType::Enum(symbols) => {
                match EnumType::new(symbols.iter().map(|symbol| symbol.text.clone()).collect()) {
                    Ok(enumeration) => DataType::Enum(enumeration),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::TaggedUnion(cases) => {
                let mut resolved_cases = Vec::with_capacity(cases.len());
                for case in cases {
                    let mut fields = Vec::with_capacity(case.fields.len());
                    for field in &case.fields {
                        fields.push(TypeField::new(
                            field.name.text.clone(),
                            self.resolve_plain_type_ref(&field.ty)?,
                        ));
                    }
                    let payload = match StructType::new(fields) {
                        Ok(payload) => payload,
                        Err(error) => {
                            self.type_constructor_refusal(span, error);
                            return None;
                        }
                    };
                    let case = match TaggedCase::new(case.name.text.clone(), payload) {
                        Ok(case) => case,
                        Err(error) => {
                            self.type_constructor_refusal(span, error);
                            return None;
                        }
                    };
                    resolved_cases.push(case);
                }
                match TaggedUnionType::new(resolved_cases) {
                    Ok(union) => DataType::TaggedUnion(union),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::Named(name) => {
                let Some(aliased) = self.type_decls.get(&name.text).cloned() else {
                    self.bag.push(
                        Diagnostic::new(
                            E_RESOLVE_UNKNOWN,
                            Phase::Resolve,
                            format!("`{}` names no declared type", name.text),
                        )
                        .with_span(name.span)
                        .with_field("name", name.text.clone()),
                    );
                    return None;
                };
                if self.type_stack.contains(&name.text) {
                    return None;
                }
                self.type_stack.push(name.text.clone());
                let resolved = self.resolve_plain_type_ref(&aliased);
                self.type_stack.pop();
                return resolved;
            }
        };
        Some(resolved)
    }

    fn collect_type_annotations(
        &mut self,
        source: &TypeRef,
        resolved: &DataType,
        path: &SchemaPath,
        rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    ) -> Option<()> {
        self.collect_direct_annotations(
            &source.annotations,
            DataAnnotationTarget::Type(resolved),
            path,
            rows,
        )?;

        let branch_count = source.alternatives.len() + usize::from(source.optional);
        if branch_count > 1 {
            let DataType::StructuralUnion(union) = resolved else {
                self.annotation_refusal(
                    source.span,
                    "schema",
                    path,
                    "a source union did not resolve to a structural union",
                );
                return None;
            };
            for alternative in &source.alternatives {
                let alternative_type = self.resolve_plain_base_type(alternative, source.span)?;
                if let DataType::StructuralUnion(_) = alternative_type {
                    self.collect_flattened_union_alternative(
                        alternative,
                        &alternative_type,
                        union,
                        path,
                        source.span,
                        rows,
                    )?;
                    continue;
                }
                let Some(index) = union
                    .branches()
                    .iter()
                    .position(|branch| branch == &alternative_type)
                else {
                    self.annotation_refusal(
                        source.span,
                        "schema",
                        path,
                        "a source union branch has no canonical resolved position",
                    );
                    return None;
                };
                let Ok(index) = u16::try_from(index) else {
                    self.annotation_refusal(
                        source.span,
                        "schema",
                        path,
                        "a structural-union branch index exceeds the canonical domain",
                    );
                    return None;
                };
                self.collect_base_annotations(
                    alternative,
                    &alternative_type,
                    &path.child(SchemaPathSegment::StructuralUnionBranch(index)),
                    rows,
                )?;
            }
            return Some(());
        }

        let Some(alternative) = source.alternatives.first() else {
            self.annotation_refusal(
                source.span,
                "schema",
                path,
                "a type reference has no alternative",
            );
            return None;
        };
        self.collect_base_annotations(alternative, resolved, path, rows)
    }

    fn collect_flattened_union_alternative(
        &mut self,
        source: &BaseType,
        alternative_type: &DataType,
        target: &crate::datatype::StructuralUnionType,
        path: &SchemaPath,
        at: Span,
        rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    ) -> Option<()> {
        let BaseType::Named(name) = source else {
            self.annotation_refusal(
                at,
                "schema",
                path,
                "only a named alias can contribute a nested structural union",
            );
            return None;
        };
        let aliased = self.type_decls.get(&name.text).cloned()?;
        let mut local = BTreeMap::new();
        self.collect_type_annotations(&aliased, alternative_type, &SchemaPath::root(), &mut local)?;
        let DataType::StructuralUnion(local_union) = alternative_type else {
            return None;
        };
        for (local_path, annotations) in local {
            if local_path.segments().is_empty() {
                for branch in local_union.branches() {
                    let Some(index) = target
                        .branches()
                        .iter()
                        .position(|candidate| candidate == branch)
                    else {
                        self.annotation_refusal(
                            name.span,
                            &name.text,
                            path,
                            "an alias branch has no position after union flattening",
                        );
                        return None;
                    };
                    let Ok(index) = u16::try_from(index) else {
                        return None;
                    };
                    let target_path = path.child(SchemaPathSegment::StructuralUnionBranch(index));
                    self.merge_annotation_row(
                        rows,
                        target_path,
                        annotations.clone(),
                        name.span,
                        &name.text,
                    )?;
                }
                continue;
            }
            let Some(SchemaPathSegment::StructuralUnionBranch(local_index)) =
                local_path.segments().first()
            else {
                self.annotation_refusal(
                    name.span,
                    &name.text,
                    path,
                    "an annotation inside a flattened union is not branch-addressable",
                );
                return None;
            };
            let branch = local_union.branches().get(usize::from(*local_index))?;
            let target_index = target
                .branches()
                .iter()
                .position(|candidate| candidate == branch)?;
            let Ok(target_index) = u16::try_from(target_index) else {
                return None;
            };
            let mut segments = path.segments().to_vec();
            segments.push(SchemaPathSegment::StructuralUnionBranch(target_index));
            segments.extend(local_path.segments()[1..].iter().cloned());
            self.merge_annotation_row(
                rows,
                SchemaPath::new(segments),
                annotations,
                name.span,
                &name.text,
            )?;
        }
        Some(())
    }

    fn collect_base_annotations(
        &mut self,
        source: &BaseType,
        resolved: &DataType,
        path: &SchemaPath,
        rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    ) -> Option<()> {
        match source {
            BaseType::Scalar(_) | BaseType::Variant | BaseType::Enum(_) => Some(()),
            BaseType::Named(name) => {
                let aliased = self.type_decls.get(&name.text).cloned()?;
                if self.type_stack.contains(&name.text) {
                    return None;
                }
                self.type_stack.push(name.text.clone());
                let result = self.collect_type_annotations(&aliased, resolved, path, rows);
                self.type_stack.pop();
                result
            }
            BaseType::Struct { fields, .. } => {
                let DataType::Struct(structure) = resolved else {
                    return None;
                };
                for field in fields {
                    let resolved_field = structure
                        .fields()
                        .iter()
                        .find(|candidate| candidate.name() == field.name.text)?;
                    self.collect_type_annotations(
                        &field.ty,
                        resolved_field.ty(),
                        &path.child(SchemaPathSegment::Field(field.name.text.clone())),
                        rows,
                    )?;
                }
                Some(())
            }
            BaseType::Map(key, value) => {
                let DataType::Map(map) = resolved else {
                    return None;
                };
                self.collect_type_annotations(
                    key,
                    map.key(),
                    &path.child(SchemaPathSegment::MapKey),
                    rows,
                )?;
                self.collect_type_annotations(
                    value,
                    map.value(),
                    &path.child(SchemaPathSegment::MapValue),
                    rows,
                )
            }
            BaseType::Array(element) | BaseType::Table(element) => {
                let DataType::Array(array) = resolved else {
                    return None;
                };
                self.collect_type_annotations(
                    element,
                    array.element(),
                    &path.child(SchemaPathSegment::ArrayElement),
                    rows,
                )
            }
            BaseType::TaggedUnion(cases) => {
                let DataType::TaggedUnion(union) = resolved else {
                    return None;
                };
                for source_case in cases {
                    let resolved_case = union
                        .cases()
                        .iter()
                        .find(|candidate| candidate.name() == source_case.name.text)?;
                    let case_path = path.child(SchemaPathSegment::TaggedUnionCase(
                        source_case.name.text.clone(),
                    ));
                    self.collect_direct_annotations(
                        &source_case.annotations,
                        DataAnnotationTarget::TaggedUnionCase,
                        &case_path,
                        rows,
                    )?;
                    for field in &source_case.fields {
                        let resolved_field = resolved_case
                            .payload()
                            .fields()
                            .iter()
                            .find(|candidate| candidate.name() == field.name.text)?;
                        self.collect_type_annotations(
                            &field.ty,
                            resolved_field.ty(),
                            &case_path.child(SchemaPathSegment::Field(field.name.text.clone())),
                            rows,
                        )?;
                    }
                }
                Some(())
            }
        }
    }

    fn collect_direct_annotations(
        &mut self,
        source: &[SynSchemaAnnotation],
        target: DataAnnotationTarget<'_>,
        path: &SchemaPath,
        rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
    ) -> Option<()> {
        for annotation in source {
            let resolved = self.resolve_data_annotation(annotation, &target, path)?;
            self.merge_annotation_row(
                rows,
                path.clone(),
                vec![resolved],
                annotation.span,
                &annotation.name.text,
            )?;
        }
        Some(())
    }

    fn merge_annotation_row(
        &mut self,
        rows: &mut BTreeMap<SchemaPath, Vec<DataAnnotation>>,
        path: SchemaPath,
        annotations: Vec<DataAnnotation>,
        span: Span,
        source_name: &str,
    ) -> Option<()> {
        let row = rows.entry(path.clone()).or_default();
        for annotation in annotations {
            if row
                .iter()
                .any(|existing| existing.kind() == annotation.kind())
            {
                self.annotation_refusal(
                    span,
                    source_name,
                    &path,
                    &format!(
                        "annotation kind {:?} appears twice on one resolved path",
                        annotation.kind()
                    ),
                );
                return None;
            }
            row.push(annotation);
        }
        Some(())
    }

    fn resolve_data_annotation(
        &mut self,
        annotation: &SynSchemaAnnotation,
        target: &DataAnnotationTarget<'_>,
        path: &SchemaPath,
    ) -> Option<DataAnnotation> {
        let arguments = self.annotation_arguments(annotation, path)?;
        match annotation.name.text.as_str() {
            "sensitive" => {
                self.require_annotation_arguments(annotation, path, &arguments, &["level"])?;
                let level = self.annotation_symbol(
                    annotation,
                    path,
                    arguments.get("level").copied()?,
                    "level",
                )?;
                let level = match level.as_str() {
                    "internal" => SensitivityLevel::Internal,
                    "confidential" => SensitivityLevel::Confidential,
                    "restricted" => SensitivityLevel::Restricted,
                    _ => {
                        self.annotation_refusal(
                            annotation.span,
                            &annotation.name.text,
                            path,
                            "`level` must be `internal`, `confidential`, or `restricted`",
                        );
                        return None;
                    }
                };
                Some(DataAnnotation::Sensitive(level))
            }
            "secret" => {
                self.require_annotation_arguments(annotation, path, &arguments, &[])?;
                Some(DataAnnotation::Secret)
            }
            "measure" => {
                self.require_annotation_arguments(annotation, path, &arguments, &["id"])?;
                let id =
                    self.annotation_symbol(annotation, path, arguments.get("id").copied()?, "id")?;
                let id = match MeasureId::new(id) {
                    Ok(id) => id,
                    Err(error) => {
                        self.annotation_refusal(
                            annotation.span,
                            &annotation.name.text,
                            path,
                            &error.to_string(),
                        );
                        return None;
                    }
                };
                let Some(spec) = measure_spec(&id) else {
                    self.annotation_refusal(
                        annotation.span,
                        &annotation.name.text,
                        path,
                        &format!("measure id `{}` is not in the pinned registry", id.as_str()),
                    );
                    return None;
                };
                if spec.source() != MeasureSource::Field {
                    self.annotation_refusal(
                        annotation.span,
                        &annotation.name.text,
                        path,
                        "this registry entry is a runtime instrument, not a field annotation",
                    );
                    return None;
                }
                let DataAnnotationTarget::Type(ty) = target else {
                    self.annotation_refusal(
                        annotation.span,
                        &annotation.name.text,
                        path,
                        "a semantic measure targets a typed field, not a union case",
                    );
                    return None;
                };
                let carrier_holds = match spec.carrier() {
                    MeasureCarrier::Integer => matches!(ty, DataType::Integer(_)),
                    MeasureCarrier::Duration => *ty == &DataType::DURATION,
                };
                if !carrier_holds {
                    self.annotation_refusal(
                        annotation.span,
                        &annotation.name.text,
                        path,
                        &format!("the registered measure cannot be carried by `{ty}`"),
                    );
                    return None;
                }
                Some(DataAnnotation::Measure(id))
            }
            "cost" => self.resolve_cost_annotation(annotation, target, path, &arguments),
            "personal" => {
                self.allow_annotation_arguments(
                    annotation,
                    path,
                    &arguments,
                    &["special", "criminal", "identifier"],
                )?;
                let special = match arguments.get("special") {
                    Some(value) => self.annotation_bool(annotation, path, value, "special")?,
                    None => false,
                };
                let criminal = match arguments.get("criminal") {
                    Some(value) => self.annotation_bool(annotation, path, value, "criminal")?,
                    None => false,
                };
                let identifier = match arguments.get("identifier") {
                    Some(value) => self.annotation_bool(annotation, path, value, "identifier")?,
                    None => false,
                };
                Some(DataAnnotation::Personal(PersonalDataClass::new(
                    special, criminal, identifier,
                )))
            }
            _ => {
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    "unknown data annotation",
                );
                None
            }
        }
    }

    fn resolve_cost_annotation(
        &mut self,
        annotation: &SynSchemaAnnotation,
        target: &DataAnnotationTarget<'_>,
        path: &SchemaPath,
        arguments: &BTreeMap<&str, &SchemaAnnotationConstant>,
    ) -> Option<DataAnnotation> {
        let DataAnnotationTarget::Type(ty) = target else {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                "a cost targets a decimal or canonical Money value, not a union case",
            );
            return None;
        };
        let is_money = is_money_type(ty);
        let required = if is_money {
            &["basis"][..]
        } else {
            &["basis", "currency"][..]
        };
        self.require_annotation_arguments(annotation, path, arguments, required)?;
        let basis =
            self.annotation_symbol(annotation, path, arguments.get("basis").copied()?, "basis")?;
        let basis = match basis.as_str() {
            "provider_reported" => CostBasis::ProviderReported,
            "pricebook_calculated" => CostBasis::PricebookCalculated,
            _ => {
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    "`basis` must be `provider_reported` or `pricebook_calculated`",
                );
                return None;
            }
        };
        if is_money {
            return Some(DataAnnotation::Cost(CostAnnotation::money(basis)));
        }
        if !matches!(ty, DataType::Decimal(_)) {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!("a scalar cost requires decimal, found `{ty}`"),
            );
            return None;
        }
        let currency = self.annotation_symbol(
            annotation,
            path,
            arguments.get("currency").copied()?,
            "currency",
        )?;
        let currency = match CurrencyCode::new(currency) {
            Ok(currency) => currency,
            Err(error) => {
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    &error.to_string(),
                );
                return None;
            }
        };
        if !is_iso_4217_currency(&currency) {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!(
                    "currency code `{}` is not in the pinned ISO 4217 registry",
                    currency.as_str()
                ),
            );
            return None;
        }
        Some(DataAnnotation::Cost(CostAnnotation::scalar(
            currency, basis,
        )))
    }

    fn annotation_arguments<'annotation>(
        &mut self,
        annotation: &'annotation SynSchemaAnnotation,
        path: &SchemaPath,
    ) -> Option<BTreeMap<&'annotation str, &'annotation SchemaAnnotationConstant>> {
        let mut arguments = BTreeMap::new();
        for argument in &annotation.arguments {
            if arguments
                .insert(argument.name.text.as_str(), &argument.value)
                .is_some()
            {
                self.annotation_refusal(
                    argument.name.span,
                    &annotation.name.text,
                    path,
                    &format!("argument `{}` appears twice", argument.name.text),
                );
                return None;
            }
        }
        Some(arguments)
    }

    fn require_annotation_arguments(
        &mut self,
        annotation: &SynSchemaAnnotation,
        path: &SchemaPath,
        arguments: &BTreeMap<&str, &SchemaAnnotationConstant>,
        required: &[&str],
    ) -> Option<()> {
        if let Some(unexpected) = arguments
            .keys()
            .find(|argument| !required.contains(argument))
        {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!("unexpected argument `{unexpected}`"),
            );
            return None;
        }
        if let Some(missing) = required
            .iter()
            .find(|argument| !arguments.contains_key(**argument))
        {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!("missing required argument `{missing}`"),
            );
            return None;
        }
        Some(())
    }

    fn allow_annotation_arguments(
        &mut self,
        annotation: &SynSchemaAnnotation,
        path: &SchemaPath,
        arguments: &BTreeMap<&str, &SchemaAnnotationConstant>,
        allowed: &[&str],
    ) -> Option<()> {
        if let Some(unexpected) = arguments
            .keys()
            .find(|argument| !allowed.contains(argument))
        {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!("unexpected argument `{unexpected}`"),
            );
            return None;
        }
        Some(())
    }

    fn annotation_bool(
        &mut self,
        annotation: &SynSchemaAnnotation,
        path: &SchemaPath,
        value: &SchemaAnnotationConstant,
        argument: &str,
    ) -> Option<bool> {
        match self
            .annotation_symbol(annotation, path, value, argument)?
            .as_str()
        {
            "true" => Some(true),
            "false" => Some(false),
            _ => {
                self.annotation_refusal(
                    annotation.span,
                    &annotation.name.text,
                    path,
                    &format!("argument `{argument}` must be `true` or `false`"),
                );
                None
            }
        }
    }

    fn annotation_symbol(
        &mut self,
        annotation: &SynSchemaAnnotation,
        path: &SchemaPath,
        value: &SchemaAnnotationConstant,
        argument: &str,
    ) -> Option<String> {
        let SchemaAnnotationConstant::Symbol(names) = value else {
            self.annotation_refusal(
                annotation.span,
                &annotation.name.text,
                path,
                &format!("argument `{argument}` must be a symbolic plan constant"),
            );
            return None;
        };
        Some(
            names
                .iter()
                .map(|name| name.text.as_str())
                .collect::<Vec<_>>()
                .join("."),
        )
    }

    fn annotation_refusal(
        &mut self,
        span: Span,
        annotation: &str,
        path: &SchemaPath,
        detail: &str,
    ) {
        if !self.reported_annotation_refusals.insert(span) {
            return;
        }
        self.bag.push(
            Diagnostic::new(E_DATA_ANNOTATION, Phase::Check, detail)
                .with_span(span)
                .with_field("annotation", annotation.to_string())
                .with_field("path", path.to_string())
                .with_field("detail", detail.to_string()),
        );
    }

    fn type_constructor_refusal(&mut self, span: Span, error: TypeError) {
        let message = error.to_string();
        let diagnostic = match error {
            TypeError::OverlappingUnionBranches { .. } => {
                Diagnostic::new(E_UNION_OVERLAP, Phase::Check, message.clone())
                    .with_field("detail", message)
            }
            TypeError::MemberBudget { count, .. } => {
                Diagnostic::new(E_TYPE_BUDGET, Phase::Check, message)
                    .with_field("name", "the source type")
                    .with_field("count", count.to_string())
            }
            TypeError::NodeBudget { nodes, .. } => {
                Diagnostic::new(E_TYPE_BUDGET, Phase::Check, message)
                    .with_field("name", "the source type")
                    .with_field("nodes", nodes.to_string())
            }
            TypeError::NestingBudget { depth, .. } => {
                Diagnostic::new(E_TYPE_BUDGET, Phase::Check, message)
                    .with_field("name", "the source type")
                    .with_field("depth", depth.to_string())
            }
            _ => Diagnostic::new(E_TYPE_INVALID, Phase::Check, message.clone())
                .with_field("detail", message),
        };
        self.bag.push(diagnostic.with_span(span));
    }

    fn attribute_value(
        &mut self,
        value: &SynValue,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
        span: Span,
    ) -> Option<AttrValue> {
        match value {
            // The parser already spoke for its recovery form.
            SynValue::Error(_) => None,
            SynValue::Expr(expr) if is_bare_word(&expr.text) => {
                Some(AttrValue::Word(expr.text.clone()))
            }
            SynValue::Expr(expr) => self.constant(expr, signature, consts).map(AttrValue::Const),
            SynValue::BracedMap(entries) => {
                let tree = self.map_value_tree(entries, signature, span)?;
                self.reduce_constant(tree, span, consts)
                    .map(AttrValue::Const)
            }
            SynValue::Frontend(_) => {
                let tree = self.nested_value_tree(value, signature, span)?;
                self.reduce_constant(tree, span, consts)
                    .map(AttrValue::Const)
            }
            SynValue::Tagged { case, .. }
                if crate::compiler::parser::is_dead_initializer_tag(&case.text) =>
            {
                self.dead_initializer_migration(case);
                None
            }
            _ => {
                self.deferred("this attribute value form", span);
                None
            }
        }
    }

    fn meta_sys_write(&mut self, span: Span) {
        self.bag.push(
            Diagnostic::new(
                E_META_SYS_READONLY,
                Phase::Check,
                "meta.sys is read-only and runtime-owned; it is never an lvalue",
            )
            .with_span(span),
        );
    }

    /// A metadata path outside the three-zone model, or outside a
    /// zone's vocabulary (D-2026-08-10-1).
    fn meta_zone_refusal(&mut self, span: Span, path: &str, detail: &str) {
        self.bag.push(
            Diagnostic::new(E_META_ZONE, Phase::Check, detail)
                .with_span(span)
                .with_field("path", path.to_string()),
        );
    }

    /// The verbatim `text! { … }` block: the first
    /// newline is excluded, the closing brace's column sets the
    /// margin stripped from every line, the trailing newline before
    /// the brace is excluded. No interpolation, no escapes.
    /// The shape gate for built-in frontends validates the shared
    /// `IDENT! args? payload` production before decoding its payload.
    fn frontend_data(
        &mut self,
        frontend: &crate::compiler::syntax::FrontendValue,
    ) -> Option<FrontendCall> {
        let refuse = |checker: &mut Self, span: Span, message: &str, detail: &str| {
            checker.bag.push(
                Diagnostic::new(E_EXPR, Phase::Check, message.to_string())
                    .with_span(span)
                    .with_field("detail", detail.to_string()),
            );
        };
        // One text literal after ordinary frontend-argument parsing.
        // Parentheses and a trailing comma are presentation; an
        // operator expression is not a build-time literal.
        let string_arg = |checker: &mut Self, value: &SynValue| -> Option<String> {
            match value {
                // The parser already spoke for its recovery form.
                SynValue::Error(_) => None,
                SynValue::Expr(expr) => checker.string_literal(expr),
                _ => {
                    refuse(
                        checker,
                        frontend.name.span,
                        "one string literal is required here",
                        "the argument is not a string",
                    );
                    None
                }
            }
        };
        let contract = self.registry.frontend(&frontend.name.text)?;
        match contract.reader {
            crate::contract::FrontendReader::Json => {
                if frontend.parens {
                    refuse(
                        self,
                        frontend.name.span,
                        "`json!` takes no argument list",
                        "the JSON reader owns one complete following value",
                    );
                    return None;
                }
                let crate::compiler::syntax::FrontendPayload::Json { value, .. } =
                    &frontend.payload
                else {
                    return None;
                };
                Some(FrontendCall::Json(value.clone()))
            }
            crate::contract::FrontendReader::Text => {
                if frontend.parens {
                    refuse(
                        self,
                        frontend.name.span,
                        "`text! { … }` takes no parentheses",
                        "the text reader owns one verbatim block",
                    );
                    return None;
                }
                Some(FrontendCall::Text)
            }
            crate::contract::FrontendReader::Base64 => {
                if frontend.parens && frontend.args.is_empty() {
                    refuse(
                        self,
                        frontend.name.span,
                        "empty parentheses — write `base64! { … }`",
                        "the optional argument list is one text literal or nothing",
                    );
                    return None;
                }
                match frontend.args.as_slice() {
                    [] => Some(FrontendCall::Base64 { content_type: None }),
                    [only] => {
                        let written = string_arg(self, only)?;
                        Some(FrontendCall::Base64 {
                            content_type: Some(self.content_type(
                                &written,
                                frontend.name.span,
                                "base64",
                            )?),
                        })
                    }
                    _ => {
                        refuse(
                            self,
                            frontend.name.span,
                            "base64 takes at most one content-type argument",
                            "too many arguments",
                        );
                        None
                    }
                }
            }
            crate::contract::FrontendReader::Deferred { construct, .. } => {
                self.deferred(construct, frontend.name.span);
                None
            }
        }
    }

    fn content_type(&mut self, written: &str, at: Span, construct: &str) -> Option<String> {
        match crate::datatype::canonical_media_type(written) {
            Ok(canonical) => Some(canonical),
            Err(error) => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTENT_TYPE,
                        Phase::Check,
                        format!("`{written}` is not a concrete media type: {error}"),
                    )
                    .with_span(at)
                    .with_field("tag", construct.to_string()),
                );
                None
            }
        }
    }

    /// The verbatim payload of a shape-validated `text!` call.
    fn verbatim_body(
        &mut self,
        frontend: &crate::compiler::syntax::FrontendValue,
    ) -> Option<crate::value::Value> {
        let crate::compiler::syntax::FrontendPayload::Raw { text, .. } = &frontend.payload else {
            return None;
        };
        Some(crate::value::Value::Text(strip_verbatim_margin(text)))
    }

    /// The decoded payload of a shape-validated `base64!` call —
    /// RFC 4648 standard alphabet, mandatory padding, ASCII
    /// whitespace ignored, URL-safe refused (D-2026-08-10-5/-6).
    fn base64_body(
        &mut self,
        frontend: &crate::compiler::syntax::FrontendValue,
    ) -> Option<Vec<u8>> {
        let crate::compiler::syntax::FrontendPayload::Raw { text, span } = &frontend.payload else {
            return None;
        };
        match decode_base64(text) {
            Ok(bytes) => Some(bytes),
            Err(detail) => {
                self.bag.push(
                    Diagnostic::new(E_EXPR, Phase::Check, "the base64 body does not decode")
                        .with_span(*span)
                        .with_field("detail", detail),
                );
                None
            }
        }
    }

    /// Collapses every PURE alias chain (`type A = B` where `B` is
    /// itself a single bare name) to its terminal type, iteratively
    /// — validation has already refused cycles and over-budget
    /// chains, so the guard here is a backstop, never a judge.
    fn collapse_alias_chains(&mut self) {
        let names: Vec<String> = self.type_decls.keys().cloned().collect();
        for name in names {
            let mut current = name.clone();
            let mut hops = 0usize;
            while let Some(ty) = self.type_decls.get(&current) {
                let pure_alias = !ty.optional
                    && ty.annotations.is_empty()
                    && ty.alternatives.len() == 1
                    && matches!(
                        &ty.alternatives[0],
                        crate::compiler::syntax::BaseType::Named(_)
                    );
                if !pure_alias {
                    break;
                }
                let crate::compiler::syntax::BaseType::Named(next) = &ty.alternatives[0] else {
                    unreachable!("just matched");
                };
                current = next.text.clone();
                hops += 1;
                if hops > 512 || current == name {
                    // Over-budget or cyclic — already diagnosed by
                    // validation; leave the entry as written.
                    current = name.clone();
                    break;
                }
            }
            if current != name
                && let Some(terminal) = self.type_decls.get(&current).cloned()
            {
                self.type_decls.insert(name, terminal);
            }
        }
    }

    /// Expands every validated alias once for the expression signature.
    /// Source names are ergonomics; `is`/`as` trees seal the resolved
    /// `DataType`, so renaming an alias cannot change runtime semantics.
    fn resolve_expression_types(&mut self, valid_names: &std::collections::BTreeSet<String>) {
        let declarations: Vec<(String, crate::compiler::syntax::TypeRef)> = self
            .type_decls
            .iter()
            .filter(|(name, _)| valid_names.contains(*name))
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();
        for (name, ty) in declarations {
            if let Some(schema) = self.resolve_annotated_schema(&ty) {
                self.resolved_types
                    .insert(name.clone(), schema.ty().clone());
                if !schema.annotations().is_empty() {
                    self.resolved_named_schemas.insert(name, schema);
                }
            }
        }
    }

    /// Validates one named-type declaration at collection: cycles
    /// refuse, and the EXPANDED tree answers to the structural
    /// budgets — ≤ 4096 nodes, ≤ 256 fields per struct, ≤ 256 enum
    /// symbols (D-2026-08-10-8). The 64 KiB canonical-byte budget
    /// re-verifies at the plan validator once composite pins ship.
    fn validate_type_decl(&mut self, decl: &crate::compiler::syntax::TypeDecl) -> bool {
        let mut nodes = 0usize;
        let mut stack: Vec<String> = Vec::new();
        if let Err(refusal) = self.walk_type(&decl.ty, &mut nodes, 0, &mut stack) {
            // Each refusal picks its own code, PHASE, and span — the
            // faulty reference or duplicate is underlined, never the
            // declaration name it happens to live in.
            let diagnostic = match refusal {
                TypeRefusal::UnknownName { name, span } => Diagnostic::new(
                    E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!("`{name}` names no declared type"),
                )
                .with_span(span)
                .with_field("name", name),
                TypeRefusal::Cycle { name, span } => Diagnostic::new(
                    E_TYPE_RECURSIVE,
                    Phase::Check,
                    format!(
                        "the type `{}` does not expand: `{name}` reaches itself — recursion refuses in v1",
                        decl.name.text
                    ),
                )
                .with_span(span)
                .with_field("name", decl.name.text.clone()),
                TypeRefusal::DuplicateField { name, span } => Diagnostic::new(
                    E_TYPE_DUP,
                    Phase::Check,
                    format!("the struct field `{name}` is declared twice"),
                )
                .with_span(span)
                .with_field("name", decl.name.text.clone())
                .with_field("field", name),
                TypeRefusal::DuplicateSymbol { name, span } => Diagnostic::new(
                    E_TYPE_DUP,
                    Phase::Check,
                    format!("the enum symbol `{name}` is declared twice"),
                )
                .with_span(span)
                .with_field("name", decl.name.text.clone())
                .with_field("field", name),
                TypeRefusal::DuplicateCase { name, span } => Diagnostic::new(
                    E_TYPE_DUP,
                    Phase::Check,
                    format!("the tagged-union case `{name}` is declared twice"),
                )
                .with_span(span)
                .with_field("name", decl.name.text.clone())
                .with_field("field", name),
                TypeRefusal::DepthBudget { measured } => Diagnostic::new(
                    E_TYPE_BUDGET,
                    Phase::Check,
                    format!(
                        "the type `{}` nests {measured} levels deep — the budget is 128",
                        decl.name.text
                    ),
                )
                .with_span(decl.name.span)
                .with_field("name", decl.name.text.clone())
                .with_field("depth", measured.to_string()),
                TypeRefusal::NodeBudget { measured } => Diagnostic::new(
                    E_TYPE_BUDGET,
                    Phase::Check,
                    format!(
                        "the type `{}` expands to {measured} nodes — the budget is 4096",
                        decl.name.text
                    ),
                )
                .with_span(decl.name.span)
                .with_field("name", decl.name.text.clone())
                .with_field("nodes", measured.to_string()),
                TypeRefusal::AliasBudget { measured } => Diagnostic::new(
                    E_TYPE_BUDGET,
                    Phase::Check,
                    format!(
                        "the type `{}` follows {measured} alias links — the budget is 256",
                        decl.name.text
                    ),
                )
                .with_span(decl.name.span)
                .with_field("name", decl.name.text.clone())
                .with_field("aliases", measured.to_string()),
                TypeRefusal::WidthBudget { what, measured } => Diagnostic::new(
                    E_TYPE_BUDGET,
                    Phase::Check,
                    format!(
                        "the type `{}` carries {measured} {what} — the budget is 256",
                        decl.name.text
                    ),
                )
                .with_span(decl.name.span)
                .with_field("name", decl.name.text.clone())
                .with_field("count", measured.to_string()),
            };
            self.bag.push(diagnostic);
            return false;
        }
        true
    }

    /// Walks a type reference through its named expansions — every
    /// declaration is validated WHOLE at collection, used or not.
    fn walk_type(
        &self,
        ty: &crate::compiler::syntax::TypeRef,
        nodes: &mut usize,
        depth: usize,
        stack: &mut Vec<String>,
    ) -> Result<(), TypeRefusal> {
        use crate::compiler::syntax::BaseType;
        const MAX_NODES: usize = 4096;
        const MAX_FIELDS: usize = 256;
        const MAX_DEPTH: usize = 128;
        if depth > MAX_DEPTH {
            // The depth budget of D-2026-08-10-8 counts STRUCTURAL
            // levels — `array<array<…>>` included, not just the
            // named-reference stack.
            return Err(TypeRefusal::DepthBudget { measured: depth });
        }
        for base in &ty.alternatives {
            // Every STRUCTURAL alternative is one expanded node — a
            // wide union spends its budget like a wide struct. A
            // Named reference is transparent: its EXPANSION is
            // counted, the name itself never is (sugar carries no
            // weight — D-2026-08-10-8).
            if !matches!(base, BaseType::Named(_)) {
                *nodes += 1;
                if *nodes > MAX_NODES {
                    return Err(TypeRefusal::NodeBudget { measured: *nodes });
                }
            }
            match base {
                BaseType::Named(name) => {
                    // The alias-follow budget (D-2026-08-10-8 as
                    // delivered): names are semantically transparent
                    // but FOLLOWING them is work — a 50 000-link
                    // alias chain must refuse typed, not exhaust
                    // the compiler's stack.
                    const MAX_ALIASES: usize = 256;
                    if stack.len() >= MAX_ALIASES {
                        return Err(TypeRefusal::AliasBudget {
                            measured: stack.len() + 1,
                        });
                    }
                    if stack.contains(&name.text) {
                        return Err(TypeRefusal::Cycle {
                            name: name.text.clone(),
                            span: name.span,
                        });
                    }
                    let Some(aliased) = self.type_decls.get(&name.text) else {
                        // An unknown name refuses at DECLARATION —
                        // `type Broken = Ghost` is broken whether or
                        // not anything uses it.
                        return Err(TypeRefusal::UnknownName {
                            name: name.text.clone(),
                            span: name.span,
                        });
                    };
                    stack.push(name.text.clone());
                    let aliased = aliased.clone();
                    self.walk_type(&aliased, nodes, depth, stack)?;
                    stack.pop();
                }
                BaseType::Struct { fields, .. } => {
                    if fields.len() > MAX_FIELDS {
                        return Err(TypeRefusal::WidthBudget {
                            what: "struct fields",
                            measured: fields.len(),
                        });
                    }
                    for (index, field) in fields.iter().enumerate() {
                        if fields[..index]
                            .iter()
                            .any(|earlier| earlier.name.text == field.name.text)
                        {
                            return Err(TypeRefusal::DuplicateField {
                                name: field.name.text.clone(),
                                span: field.name.span,
                            });
                        }
                        self.walk_type(&field.ty, nodes, depth + 1, stack)?;
                    }
                }
                BaseType::Enum(symbols) => {
                    if symbols.len() > MAX_FIELDS {
                        return Err(TypeRefusal::WidthBudget {
                            what: "enum symbols",
                            measured: symbols.len(),
                        });
                    }
                    for (index, symbol) in symbols.iter().enumerate() {
                        if symbols[..index]
                            .iter()
                            .any(|earlier| earlier.text == symbol.text)
                        {
                            return Err(TypeRefusal::DuplicateSymbol {
                                name: symbol.text.clone(),
                                span: symbol.span,
                            });
                        }
                    }
                }
                BaseType::TaggedUnion(cases) => {
                    if cases.len() > MAX_FIELDS {
                        return Err(TypeRefusal::WidthBudget {
                            what: "tagged-union cases",
                            measured: cases.len(),
                        });
                    }
                    for (case_index, case) in cases.iter().enumerate() {
                        if cases[..case_index]
                            .iter()
                            .any(|earlier| earlier.name.text == case.name.text)
                        {
                            return Err(TypeRefusal::DuplicateCase {
                                name: case.name.text.clone(),
                                span: case.name.span,
                            });
                        }
                        if case.fields.len() > MAX_FIELDS {
                            return Err(TypeRefusal::WidthBudget {
                                what: "tagged-case fields",
                                measured: case.fields.len(),
                            });
                        }
                        // A tagged case owns one closed-struct payload node,
                        // including a unit case's empty payload.
                        *nodes += 1;
                        if *nodes > MAX_NODES {
                            return Err(TypeRefusal::NodeBudget { measured: *nodes });
                        }
                        for (field_index, field) in case.fields.iter().enumerate() {
                            if case.fields[..field_index]
                                .iter()
                                .any(|earlier| earlier.name.text == field.name.text)
                            {
                                return Err(TypeRefusal::DuplicateField {
                                    name: field.name.text.clone(),
                                    span: field.name.span,
                                });
                            }
                            self.walk_type(&field.ty, nodes, depth + 2, stack)?;
                        }
                    }
                }
                BaseType::Map(keys, values) => {
                    self.walk_type(keys, nodes, depth + 1, stack)?;
                    self.walk_type(values, nodes, depth + 1, stack)?;
                }
                BaseType::Array(inner) | BaseType::Table(inner) => {
                    self.walk_type(inner, nodes, depth + 1, stack)?;
                }
                BaseType::Scalar(_) | BaseType::Variant => {}
            }
        }
        Ok(())
    }

    /// Resolves `asset("path", content_type?)` at BUILD time
    /// (D-2026-08-10-5): App-root-relative, escapes and missing
    /// files refuse, 8 MiB per asset and 32 MiB total, blobs
    /// deduplicated by digest.
    /// Resolves a shape-validated asset PATH at build time
    /// (D-2026-08-10-5): App-root-relative, symlink-contained,
    /// budget-checked, deduplicated by digest. One diagnostic per
    /// refusal.
    fn asset_resolve(&mut self, path: &str, at: Span) -> Option<u32> {
        const PER_ASSET: u64 = 8 * 1024 * 1024;
        const TOTAL: u64 = 32 * 1024 * 1024;
        let refuse = |checker: &mut Self, message: String| {
            checker.bag.push(
                Diagnostic::new(E_ASSET, Phase::Check, message)
                    .with_span(at)
                    .with_field("path", path.to_string()),
            );
        };
        if self.in_lab {
            refuse(
                self,
                String::from(
                    "an asset belongs to the deployed graph — a Lab cannot extend the plan",
                ),
            );
            return None;
        }
        let Some(root) = self.asset_root.clone() else {
            refuse(
                self,
                String::from("this compilation has no asset root — nothing to read from"),
            );
            return None;
        };
        // Escape refusal on the LEXICAL path — no dot-dot, no
        // absolute path; then the RESOLVED check stops symlinks.
        let relative = std::path::Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            refuse(self, String::from("the path leaves the App root"));
            return None;
        }
        let full = root.join(relative);
        match (full.canonicalize(), root.canonicalize()) {
            (Ok(resolved), Ok(resolved_root)) if resolved.starts_with(&resolved_root) => {}
            (Err(error), _) => {
                refuse(self, format!("the asset does not read: {error}"));
                return None;
            }
            _ => {
                refuse(self, String::from("the path resolves outside the App root"));
                return None;
            }
        }
        // The ONE sanctioned filesystem read of the build: asset
        // resolution is the compiler's job by decision
        // (D-2026-08-10-5); the kernel-guarding lint stands aside
        // here and only here.
        #[allow(clippy::disallowed_methods)]
        let bytes = match std::fs::read(&full) {
            Ok(bytes) => bytes,
            Err(error) => {
                refuse(self, format!("the asset does not read: {error}"));
                return None;
            }
        };
        if bytes.len() as u64 > PER_ASSET {
            refuse(
                self,
                format!(
                    "{} bytes — the per-asset budget is {PER_ASSET} (build-time bytes; runtime data goes through Connectors)",
                    bytes.len()
                ),
            );
            return None;
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        if let Some(index) = self.assets.iter().position(|blob| blob.digest == digest) {
            // Deduplicated: one content, one blob, one index.
            return Some(u32::try_from(index).unwrap_or_else(|_| unreachable!("bounded by TOTAL")));
        }
        let total: u64 = self.assets.iter().map(|blob| blob.bytes.len() as u64).sum();
        if total + bytes.len() as u64 > TOTAL {
            refuse(
                self,
                format!("the App's total asset budget ({TOTAL} bytes) is exceeded"),
            );
            return None;
        }
        self.assets.push(crate::plan::AssetBlob { digest, bytes });
        Some(
            u32::try_from(self.assets.len() - 1)
                .unwrap_or_else(|_| unreachable!("bounded by TOTAL")),
        )
    }

    /// Applies the two contextual conversions allowed by typed construction:
    /// a written text literal is sealed as a canonical `media_type`, and an
    /// unwrapped value selects the unique branch of a destination structural
    /// union. Dynamic media-type conversion remains explicit; union selection
    /// is recorded in HIR as `As`, so the VM never guesses or repairs either.
    fn contextualize_tree(
        &mut self,
        tree: expr::Tree,
        expected: &DataType,
        span: Span,
        place: &str,
    ) -> Option<expr::Tree> {
        if tree.ty == *expected {
            return Some(tree);
        }
        if expected == &DataType::MediaType
            && let expr::Expr::Literal(crate::value::Value::Text(written)) = &tree.root
        {
            return match crate::datatype::canonical_media_type(written) {
                Ok(canonical) => Some(expr::Tree {
                    root: expr::Expr::Literal(crate::value::Value::Text(canonical)),
                    ty: DataType::MediaType,
                }),
                Err(error) => {
                    self.port_type_refusal(
                        span,
                        place,
                        expected,
                        &tree.ty,
                        &format!("the literal is not a concrete media type: {error}"),
                    );
                    None
                }
            };
        }
        if let DataType::StructuralUnion(target) = expected {
            if matches!(tree.ty, DataType::StructuralUnion(_)) {
                self.port_type_refusal(
                    span,
                    place,
                    expected,
                    &tree.ty,
                    "a conversion between different structural unions requires an explicit `as`",
                );
                return None;
            }
            let matches = target
                .branches()
                .iter()
                .filter(|branch| tree.ty.is_assignable_to(branch))
                .count();
            if matches == 1 {
                return Some(expr::Tree {
                    root: expr::Expr::As {
                        expr: Box::new(tree.root),
                        target: Box::new(expr::NarrowTarget::Type(Box::new(
                            expr::NarrowType::new(expected.clone()),
                        ))),
                    },
                    ty: expected.clone(),
                });
            }
            let detail = if matches == 0 {
                "the value matches no branch of the destination union"
            } else {
                "the value matches several branches of the destination union"
            };
            self.port_type_refusal(span, place, expected, &tree.ty, detail);
            return None;
        }
        if tree.ty.is_assignable_to(expected) {
            return Some(tree);
        }
        self.port_type_refusal(
            span,
            place,
            expected,
            &tree.ty,
            "the value is not assignable to the declared type",
        );
        None
    }

    fn port_type_refusal(
        &mut self,
        span: Span,
        place: &str,
        expected: &DataType,
        found: &DataType,
        detail: &str,
    ) {
        self.bag.push(
            Diagnostic::new(
                E_PORT_TYPE,
                Phase::Check,
                format!("`{place}` requires {expected}; this value is {found}: {detail}"),
            )
            .with_span(span)
            .with_field("port", place.to_string())
            .with_field("expected", expected.to_string())
            .with_field("found", found.to_string()),
        );
    }

    /// Compiles one host value under an optional logical expectation. This is
    /// the single contextual-construction door used by ports, state and case
    /// fields; ordinary expression parsing remains context-free.
    fn typed_value_tree(
        &mut self,
        value: &SynValue,
        signature: &expr::Signature,
        expected: Option<&DataType>,
        span: Span,
        place: &str,
    ) -> Option<expr::Tree> {
        let tree = match value {
            SynValue::Tagged { case, body } => {
                return self.tagged_value_tree(case, body, signature, expected, span, place);
            }
            SynValue::Braced(body) => {
                self.struct_value_tree(body, signature, expected, span, place)?
            }
            _ => self.nested_value_tree(value, signature, span)?,
        };
        match expected {
            Some(expected) => self.contextualize_tree(tree, expected, span, place),
            None => Some(tree),
        }
    }

    fn tagged_value_tree(
        &mut self,
        name: &crate::compiler::syntax::Name,
        body: &StructBody,
        signature: &expr::Signature,
        expected: Option<&DataType>,
        span: Span,
        place: &str,
    ) -> Option<expr::Tree> {
        let mut candidates = Vec::new();
        let mut collect = |ty: &DataType| match ty {
            DataType::TaggedUnion(union)
                if union.cases().iter().any(|case| case.name() == name.text) =>
            {
                if !candidates.contains(union) {
                    candidates.push(union.clone());
                }
            }
            DataType::StructuralUnion(union) => {
                for branch in union.branches() {
                    if let DataType::TaggedUnion(tagged) = branch
                        && tagged.cases().iter().any(|case| case.name() == name.text)
                        && !candidates.contains(tagged)
                    {
                        candidates.push(tagged.clone());
                    }
                }
            }
            _ => {}
        };
        match expected {
            Some(expected) => collect(expected),
            None => {
                for ty in self.resolved_types.values() {
                    collect(ty);
                }
            }
        }
        let [union] = candidates.as_slice() else {
            if crate::compiler::parser::is_dead_initializer_tag(&name.text) {
                self.dead_initializer_migration(name);
            } else {
                let detail = if candidates.is_empty() {
                    "no compatible tagged union declares this case"
                } else {
                    "the case name belongs to more than one compatible tagged union"
                };
                let target = expected.cloned().unwrap_or(DataType::Variant);
                self.port_type_refusal(span, place, &target, &DataType::Variant, detail);
            }
            return None;
        };
        let Some(selected) = union.cases().iter().find(|case| case.name() == name.text) else {
            self.port_type_refusal(
                span,
                place,
                expected.unwrap_or(&DataType::Variant),
                &DataType::Variant,
                "the selected tagged union does not declare this case",
            );
            return None;
        };
        let fields = self.closed_struct_fields(
            body,
            selected.payload(),
            signature,
            span,
            &format!("{}::{name}", place, name = name.text),
        )?;
        let tree = expr::Tree {
            root: expr::Expr::Tagged {
                union: Box::new(union.clone()),
                case: name.text.clone(),
                fields,
            },
            ty: DataType::TaggedUnion(union.clone()),
        };
        match expected {
            Some(expected) => self.contextualize_tree(tree, expected, span, place),
            None => Some(tree),
        }
    }

    fn struct_value_tree(
        &mut self,
        body: &StructBody,
        signature: &expr::Signature,
        expected: Option<&DataType>,
        span: Span,
        place: &str,
    ) -> Option<expr::Tree> {
        let expected_struct = match expected {
            Some(DataType::Struct(structure)) => Some(structure.clone()),
            Some(DataType::StructuralUnion(union)) => {
                let Some(source_names) = source_field_names(body) else {
                    self.port_type_refusal(
                        span,
                        place,
                        expected.unwrap_or(&DataType::Variant),
                        &DataType::Variant,
                        "a union record constructor contains only unqualified `field = value` entries",
                    );
                    return None;
                };
                let matching: Vec<_> = union
                    .branches()
                    .iter()
                    .filter_map(|branch| match branch {
                        DataType::Struct(structure)
                            if structure
                                .fields()
                                .iter()
                                .map(TypeField::name)
                                .eq(source_names.iter().map(String::as_str)) =>
                        {
                            Some(structure.clone())
                        }
                        _ => None,
                    })
                    .collect();
                match matching.as_slice() {
                    [structure] => Some(structure.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        let (fields, ty) = if let Some(structure) = &expected_struct {
            (
                self.closed_struct_fields(body, structure, signature, span, place)?,
                DataType::Struct(structure.clone()),
            )
        } else {
            let mut fields = Vec::new();
            for entry in &body.entries {
                let StructEntry::Field { key, value } = entry else {
                    self.port_type_refusal(
                        span,
                        place,
                        expected.unwrap_or(&DataType::Variant),
                        &DataType::Variant,
                        "a value struct contains only `field = value` entries",
                    );
                    return None;
                };
                let [name] = key.segments.as_slice() else {
                    self.port_type_refusal(
                        key.span,
                        place,
                        expected.unwrap_or(&DataType::Variant),
                        &DataType::Variant,
                        "a closed value struct uses one field segment per entry",
                    );
                    return None;
                };
                let tree = self.typed_value_tree(value, signature, None, key.span, &name.text)?;
                fields.push((name.text.clone(), tree.root, tree.ty));
            }
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            if fields.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                self.port_type_refusal(
                    span,
                    place,
                    expected.unwrap_or(&DataType::Variant),
                    &DataType::Variant,
                    "a struct field is written more than once",
                );
                return None;
            }
            let typed = fields
                .iter()
                .map(|(name, _, ty)| TypeField::new(name.clone(), ty.clone()))
                .collect();
            let ty = match StructType::new(typed) {
                Ok(structure) => DataType::Struct(structure),
                Err(error) => {
                    self.type_constructor_refusal(span, error);
                    return None;
                }
            };
            (
                fields
                    .into_iter()
                    .map(|(name, value, _)| (name, value))
                    .collect(),
                ty,
            )
        };
        let tree = expr::Tree {
            root: expr::Expr::Struct(fields),
            ty,
        };
        match expected {
            Some(expected) => self.contextualize_tree(tree, expected, span, place),
            None => Some(tree),
        }
    }

    fn closed_struct_fields(
        &mut self,
        body: &StructBody,
        expected: &StructType,
        signature: &expr::Signature,
        span: Span,
        place: &str,
    ) -> Option<Vec<(String, expr::Expr)>> {
        let mut source = BTreeMap::new();
        for entry in &body.entries {
            let StructEntry::Field { key, value } = entry else {
                self.port_type_refusal(
                    span,
                    place,
                    &DataType::Struct(expected.clone()),
                    &DataType::Variant,
                    "a constructor payload contains only `field = value` entries",
                );
                return None;
            };
            let [name] = key.segments.as_slice() else {
                self.port_type_refusal(
                    key.span,
                    place,
                    &DataType::Struct(expected.clone()),
                    &DataType::Variant,
                    "a constructor field is one unqualified name",
                );
                return None;
            };
            if source
                .insert(name.text.clone(), (value, key.span))
                .is_some()
            {
                self.port_type_refusal(
                    key.span,
                    place,
                    &DataType::Struct(expected.clone()),
                    &DataType::Variant,
                    "a constructor field is written more than once",
                );
                return None;
            }
        }
        let expected_names: std::collections::BTreeSet<_> =
            expected.fields().iter().map(TypeField::name).collect();
        let source_names: std::collections::BTreeSet<_> =
            source.keys().map(String::as_str).collect();
        if expected_names != source_names {
            self.port_type_refusal(
                span,
                place,
                &DataType::Struct(expected.clone()),
                &DataType::Variant,
                "the constructor fields must exactly match the closed payload",
            );
            return None;
        }
        let mut fields = Vec::with_capacity(expected.fields().len());
        for field in expected.fields() {
            let Some((value, field_span)) = source.remove(field.name()) else {
                self.port_type_refusal(
                    span,
                    place,
                    &DataType::Struct(expected.clone()),
                    &DataType::Variant,
                    "the constructor fields must exactly match the closed payload",
                );
                return None;
            };
            let tree = self.typed_value_tree(
                value,
                signature,
                Some(field.ty()),
                field_span,
                field.name(),
            )?;
            fields.push((field.name().to_string(), tree.root));
        }
        Some(fields)
    }

    fn dead_initializer_migration(&mut self, name: &crate::compiler::syntax::Name) {
        self.bag.push(
            Diagnostic::new(
                E_CONTRACT_ATTR,
                Phase::Check,
                format!(
                    "`{tag} {{ … }}` is a dead tagged-initializer spelling; write a braced struct (`name = {{ … }}`) — a former discriminant becomes a field",
                    tag = name.text
                ),
            )
            .with_span(name.span)
            .with_field("attribute", name.text.clone())
            .with_field("detail", "dead tagged-initializer spelling"),
        );
    }

    /// THE nested-value lowering, shared by struct fields and map entries:
    /// expression, frontend, recursive maps and the EMPTY struct.
    /// A content-typed byte form refuses with the dedicated rule —
    /// a nested field has no `meta.ext` of its own — and the one
    /// genuinely inexpressible form, a NON-EMPTY struct inside a
    /// map, defers by name (D-2026-08-10-6 as amended; dynamic
    /// nesting spells `json`, which is always `variant`).
    fn nested_value_tree(
        &mut self,
        value: &SynValue,
        signature: &expr::Signature,
        at: Span,
    ) -> Option<expr::Tree> {
        match value {
            // The parser already spoke for its recovery form.
            SynValue::Error(_) => None,
            SynValue::Expr(expr) => self.expr(expr, signature),
            SynValue::BracedMap(entries) => self.map_value_tree(entries, signature, at),
            SynValue::Braced(body) if body.entries.is_empty() => Some(literal_tree(
                crate::value::Value::Struct(Vec::new()),
                DataType::Variant,
            )),
            SynValue::Braced(_) => {
                self.deferred("a struct inside a map entry", at);
                None
            }
            SynValue::Frontend(frontend) => {
                let call = self.frontend_data(frontend)?;
                match call {
                    FrontendCall::Json(value) => Some(literal_tree(value, DataType::Variant)),
                    FrontendCall::Text => self
                        .verbatim_body(frontend)
                        .map(|value| literal_tree(value, DataType::Text)),
                    FrontendCall::Base64 { content_type: None } => {
                        let bytes = self.base64_body(frontend)?;
                        Some(literal_tree(
                            crate::value::Value::Bytes(bytes),
                            DataType::BYTES,
                        ))
                    }
                    FrontendCall::Base64 { .. } => {
                        self.bag.push(
                            Diagnostic::new(
                                E_CONTENT_TYPE,
                                Phase::Check,
                                "a content type qualifies the WHOLE payload — a nested field has no `meta.ext` of its own",
                            )
                            .with_span(frontend.name.span)
                            .with_field("tag", frontend.name.text.clone()),
                        );
                        None
                    }
                }
            }
            _ => {
                self.deferred("this nested value form", at);
                None
            }
        }
    }

    /// Compiles a braced-map body into the MapLit expression — every
    /// entry goes through [`Self::nested_value_tree`].
    fn map_value_tree(
        &mut self,
        entries: &[crate::compiler::syntax::MapEntry],
        signature: &expr::Signature,
        at: Span,
    ) -> Option<expr::Tree> {
        let mut pairs: Vec<(String, expr::Expr, DataType)> = Vec::new();
        for entry in entries {
            // ONE string semantics: the key decodes through Rastok expression —
            // `"a\nb"` here and `{"a\nb": 1}` inside a Rastok expression span are
            // the same key, the same hash.
            let key = self.string_literal(&entry.key)?;
            if pairs.iter().any(|(known, _, _)| known == &key) {
                // A duplicate refuses HERE, at the SECOND key — the
                // registered static rule, never an internal-plan
                // surprise downstream.
                self.bag.push(
                    Diagnostic::new(
                        E_DUP_KEY,
                        Phase::Check,
                        format!("the map key `{key}` is already present"),
                    )
                    .with_span(entry.span)
                    .with_field("detail", key),
                );
                return None;
            }
            let tree = match &entry.value {
                // A named case is the B6 addition to an untyped map value.
                // Other nested forms retain their established source rules;
                // in particular a non-empty struct still names its deferral
                // rather than inferring a structurally unrelated map value.
                SynValue::Tagged { .. } => {
                    self.typed_value_tree(&entry.value, signature, None, at, "map value")?
                }
                _ => self.nested_value_tree(&entry.value, signature, at)?,
            };
            pairs.push((key, tree.root, tree.ty));
        }
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        // the Rastok expression language's own MapLit inference (join over the value types) —
        // `{ "k": 1 }` here and inside a Rastok expression span are ONE program,
        // one type, one hash.
        let mut values = None;
        for (_, _, ty) in &pairs {
            values = Some(match values {
                Some(current) => match inferred_join(&current, ty) {
                    Ok(joined) => joined,
                    Err(error) => {
                        self.type_constructor_refusal(at, error);
                        return None;
                    }
                },
                None => ty.clone(),
            });
        }
        Some(expr::Tree {
            root: expr::Expr::MapLit(
                pairs
                    .into_iter()
                    .map(|(key, root, _)| (key, root))
                    .collect(),
            ),
            // The empty map keeps a compiler-only element hole. The B2 plan
            // solver recognizes the empty AST and resolves it from context;
            // `variant` here never enters the validated table as a fallback.
            ty: DataType::text_map(values.unwrap_or(DataType::Variant)),
        })
    }

    /// Compiles an expression that MUST be one text literal — map
    /// keys and the STRING arguments of `asset`/`base64`. The Rastok expression
    /// compiler owns the escape rule; an expression, however
    /// string-shaped its spelling, refuses.
    fn string_literal(&mut self, expr: &ExprSource) -> Option<String> {
        let tree = self.expr(expr, &expr::Signature::new())?;
        match tree.root {
            expr::Expr::Literal(crate::value::Value::Text(text)) => Some(text),
            _ => {
                self.bag.push(
                    Diagnostic::new(
                        E_EXPR,
                        Phase::Check,
                        "one string literal is required here — not an expression",
                    )
                    .with_span(expr.span)
                    .with_field("detail", "the value is not a text literal"),
                );
                None
            }
        }
    }

    /// Whether an `ext` field belongs to the declared vocabulary: a standard
    /// external-context leaf or one declared by an instantiated sheet.
    fn ext_field_known(&self, field: &str) -> bool {
        crate::plan::standard_ext_field_type(field).is_some() || self.ext_fields.contains(field)
    }

    /// Records every `meta.*` path a compiled expression reads —
    /// verified against the sealed schema once inference closes.
    /// A DYNAMIC meta access (`meta["…"]`) refuses on the spot: the
    /// grid only holds if the checker sees every key.
    fn collect_meta_reads(&mut self, expr: &expr::Expr, span: Span, signature: &expr::Signature) {
        let metadata_path = |segments: &[String]| {
            (1..=segments.len()).rev().find_map(|length| {
                signature
                    .is_metadata_root(&segments[..length].join("."))
                    .then(|| {
                        core::iter::once(String::from("meta"))
                            .chain(segments[length..].iter().cloned())
                            .collect::<Vec<_>>()
                    })
            })
        };
        match expr {
            expr::Expr::Path(segments) => {
                if let Some(path) = metadata_path(segments) {
                    self.meta_reads.push((path, span));
                }
            }
            expr::Expr::Index { base, index } => {
                if matches!(&**base, expr::Expr::Path(segments)
                    if metadata_path(segments).is_some())
                {
                    self.meta_zone_refusal(
                        span,
                        "meta[…]",
                        "dynamic metadata access defeats the static grid — spell the key",
                    );
                }
                self.collect_meta_reads(base, span, signature);
                self.collect_meta_reads(index, span, signature);
            }
            expr::Expr::Unary { expr, .. }
            | expr::Expr::Is { expr, .. }
            | expr::Expr::As { expr, .. } => self.collect_meta_reads(expr, span, signature),
            expr::Expr::Binary { left, right, .. } => {
                self.collect_meta_reads(left, span, signature);
                self.collect_meta_reads(right, span, signature);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.collect_meta_reads(cond, span, signature);
                self.collect_meta_reads(then, span, signature);
                self.collect_meta_reads(otherwise, span, signature);
            }
            expr::Expr::Call { args, .. } => {
                for arg in args {
                    self.collect_meta_reads(arg, span, signature);
                }
            }
            expr::Expr::List(items) => {
                for item in items {
                    self.collect_meta_reads(item, span, signature);
                }
            }
            expr::Expr::MapLit(pairs) => {
                for (_, value) in pairs {
                    self.collect_meta_reads(value, span, signature);
                }
            }
            expr::Expr::Struct(fields) | expr::Expr::Tagged { fields, .. } => {
                for (_, value) in fields {
                    self.collect_meta_reads(value, span, signature);
                }
            }
            expr::Expr::Literal(_) | expr::Expr::Asset(_) => {}
        }
    }

    /// Seals the production `app` schema from the GRAPH's writes:
    /// types unify through the same directional relation as ports and state;
    /// explicit `variant` remains dynamic rather than impersonating an
    /// inference hole. A conflict refuses naming both sites, and flat keys
    /// must be prefix-free
    /// (D-2026-08-10-1).
    fn seal_app_schema(&mut self) {
        let mut sites: BTreeMap<String, (DataType, Span)> = BTreeMap::new();
        for (field, ty, span) in core::mem::take(&mut self.app_writes) {
            match sites.get(&field) {
                None => {
                    sites.insert(field, (ty, span));
                }
                Some((known, first)) => {
                    // Metadata leaves have one stable type. Unlike a data
                    // expression, unrelated writes do not synthesize a
                    // structural union behind the user's back.
                    let unified = if known.is_assignable_to(&ty) {
                        Some(ty.clone())
                    } else if ty.is_assignable_to(known) {
                        Some(known.clone())
                    } else {
                        None
                    };
                    match unified {
                        Some(unified) => {
                            let first = *first;
                            sites.insert(field, (unified, first));
                        }
                        None => {
                            let (known, first) = (known.clone(), *first);
                            self.bag.push(
                                Diagnostic::new(
                                    E_META_TYPE,
                                    Phase::Check,
                                    format!(
                                        "`meta.app.{field}` is written as {known:?} and as {ty:?} — one field, one type"
                                    ),
                                )
                                .with_span(span)
                                .with_span(first)
                                .with_field("field", field.clone()),
                            );
                        }
                    }
                }
            }
        }
        self.app_schema = sites
            .into_iter()
            .map(|(field, (ty, _))| (field, ty))
            .collect();
        // Prefix-freedom: `audit` and `audit.origin` cannot coexist
        // — one is a leaf, the other makes it a branch.
        let keys: Vec<String> = self.app_schema.keys().cloned().collect();
        for pair in keys.windows(2) {
            if pair[1].starts_with(&pair[0]) && pair[1].as_bytes().get(pair[0].len()) == Some(&b'.')
            {
                self.bag.push(
                    Diagnostic::new(
                        E_META_PREFIX,
                        Phase::Check,
                        format!(
                            "`meta.app.{}` is a leaf and `meta.app.{}` makes it a branch — flat keys are prefix-free",
                            pair[0], pair[1]
                        ),
                    )
                    .with_field("field", pair[0].clone()),
                );
            }
        }
    }

    /// Verifies every collected `meta.*` read against the sealed
    /// vocabulary — the Rastok expression grid, statically: three zones only, and
    /// a name written nowhere refuses.
    fn verify_meta_reads(&mut self) {
        const SYS_FIELDS: &[&str] = &[
            "id",
            "parent_id",
            "origin_id",
            "role",
            "graph_version",
            // Tier-2 birth stamps — spec'd fields, absent outside
            // their Extension scope (envelope.md §1.2).
            "saga_id",
            "saga_stack",
            "loop_id",
            "loop_stack",
            "iter_count",
        ];
        let reads = core::mem::take(&mut self.meta_reads);
        for (segments, span) in reads {
            let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
            match parts.as_slice() {
                // A WHOLE zone leaving as a value would alias the
                // grid away (`val m = meta.app; m.ghost`): fields
                // are read by their spelled path, never through a
                // zone-typed binding.
                ["meta"] | ["meta", "app"] | ["meta", "ext"] | ["meta", "sys"] => {
                    self.meta_zone_refusal(
                        span,
                        &segments.join("."),
                        "a metadata zone is not a value — read fields by their full spelled path",
                    );
                }
                ["meta", "sys", field] => {
                    if !SYS_FIELDS.contains(field) {
                        self.meta_zone_refusal(
                            span,
                            &format!("meta.sys.{field}"),
                            "the sys spine has five fields plus the Extension birth stamps — nothing else",
                        );
                    }
                }
                ["meta", "sys", ..] => {
                    // `meta.sys.id.extra` — a spine field is a LEAF.
                    self.meta_zone_refusal(
                        span,
                        &segments.join("."),
                        "a sys field is a leaf — nothing nests under it",
                    );
                }
                ["meta", "app", rest @ ..] => {
                    // A read names EXACTLY a leaf: a projected
                    // BRANCH leaving as a value would alias the grid
                    // away (`val m = meta.app.audit; m.ghost`).
                    let field = rest.join(".");
                    if !self.app_schema.contains_key(&field) {
                        self.meta_zone_refusal(
                            span,
                            &format!("meta.app.{field}"),
                            "the graph writes this exact leaf nowhere — reads name a leaf, never a branch (the Rastok expression grid)",
                        );
                    }
                }
                ["meta", "ext", rest @ ..] => {
                    let field = rest.join(".");
                    if !self.ext_field_known(&field) {
                        self.meta_zone_refusal(
                            span,
                            &format!("meta.ext.{field}"),
                            "no connector sheet declares this exact `ext` leaf",
                        );
                    }
                }
                ["meta", ..] => {
                    self.meta_zone_refusal(
                        span,
                        &segments.join("."),
                        "metadata has exactly three zones — `meta.sys`, `meta.app`, `meta.ext` (D-2026-08-10-1)",
                    );
                }
                _ => {}
            }
        }
    }

    fn emit_value(
        &mut self,
        value: &SynValue,
        signature: &expr::Signature,
        expected: Option<&DataType>,
        at: Span,
        port_name: &str,
    ) -> Option<EmitValue> {
        match value {
            // The parser already spoke for its recovery form.
            SynValue::Error(_) => None,
            SynValue::Identity(_) => Some(EmitValue::Identity),
            SynValue::Braced(body)
                if matches!(
                    expected,
                    Some(DataType::Struct(_) | DataType::StructuralUnion(_))
                ) =>
            {
                self.typed_value_tree(value, signature, expected, at, port_name)
                    .map(EmitValue::Data)
            }
            SynValue::Braced(body) => self.braced(body, signature, at),
            SynValue::Nothing(_) => Some(EmitValue::Nothing),
            SynValue::Tagged { .. } => self
                .typed_value_tree(value, signature, expected, at, port_name)
                .map(EmitValue::Data),
            SynValue::Expr(expr) => {
                let (tree, content_type) = self.payload_expr(expr, signature)?;
                let tree = match expected {
                    Some(expected) => {
                        self.contextualize_tree(tree, expected, expr.span, port_name)?
                    }
                    None => tree,
                };
                if matches!(
                    tree.ty,
                    DataType::DURATION | DataType::TIMESTAMP | DataType::Null
                ) {
                    self.deferred("a duration, timestamp, or null emission", expr.span);
                    return None;
                }
                Some(match content_type {
                    Some(content_type) => EmitValue::FreshData(vec![
                        FieldWrite {
                            target: crate::envelope::WriteTarget::Data(Vec::new()),
                            value: tree,
                        },
                        FieldWrite {
                            target: crate::envelope::WriteTarget::Ext(String::from("content_type")),
                            value: literal_tree(
                                crate::value::Value::Text(content_type),
                                DataType::MediaType,
                            ),
                        },
                    ]),
                    None => EmitValue::Data(tree),
                })
            }
            SynValue::BracedMap(entries) => {
                // `{ "k": v }` and `{:}` are executable emissions:
                // the entries compile into the existing MapLit form,
                // nested maps and json literals included.
                let tree = self.map_value_tree(entries, signature, at)?;
                let tree = match expected {
                    Some(expected) => self.contextualize_tree(tree, expected, at, port_name)?,
                    None => tree,
                };
                Some(EmitValue::Data(tree))
            }
            SynValue::Frontend(frontend) => {
                let call = self.frontend_data(frontend)?;
                let (tree, content_type) = match call {
                    FrontendCall::Json(value) => (literal_tree(value, DataType::Variant), None),
                    FrontendCall::Text => (
                        self.verbatim_body(frontend)
                            .map(|value| literal_tree(value, DataType::Text))?,
                        None,
                    ),
                    FrontendCall::Base64 { content_type } => (
                        literal_tree(
                            crate::value::Value::Bytes(self.base64_body(frontend)?),
                            DataType::BYTES,
                        ),
                        content_type,
                    ),
                };
                let tree = match expected {
                    Some(expected) => self.contextualize_tree(tree, expected, at, port_name)?,
                    None => tree,
                };
                Some(match content_type {
                    Some(content_type) => EmitValue::FreshData(vec![
                        FieldWrite {
                            target: crate::envelope::WriteTarget::Data(Vec::new()),
                            value: tree,
                        },
                        FieldWrite {
                            target: crate::envelope::WriteTarget::Ext(String::from("content_type")),
                            value: literal_tree(
                                crate::value::Value::Text(content_type),
                                DataType::MediaType,
                            ),
                        },
                    ]),
                    None => EmitValue::Data(tree),
                })
            }
        }
    }

    /// Classifies a braced body ONCE — the reform's rule (decision
    /// D-2026-08-09-1). The reserved roots `data` and `meta` form a
    /// closed, normative list (`mime_type` left it with
    /// D-2026-08-10-4):
    ///
    /// - no reserved root at the top level → a DATA RECORD: every
    ///   field, at any depth, builds the new `data` (a fresh
    ///   `Struct`); heritable metadata stays the input's.
    /// - only reserved roots → the ENVELOPE form: a whole-`data`
    ///   assignment makes the build fresh (`FreshData`-based, relative
    ///   writes); otherwise the current envelope is patched.
    /// - a mix → the dedicated diagnostic: wrap the data fields in
    ///   `data = { … }`. Never a silent reinterpretation.
    fn braced(
        &mut self,
        body: &StructBody,
        signature: &expr::Signature,
        at: Span,
    ) -> Option<EmitValue> {
        // ONE root extraction for both entry forms: a field's first
        // segment, a sub-block's name — `meta { … }` carries the
        // reserved root exactly like `meta.x = …`.
        fn root(entry: &StructEntry) -> Option<&str> {
            match entry {
                StructEntry::Field { key, .. } => {
                    key.segments.first().map(|segment| segment.text.as_str())
                }
                StructEntry::Block { name, .. } => Some(name.text.as_str()),
            }
        }
        let reserved = |entry: &StructEntry| matches!(root(entry), Some("data" | "meta"));
        let any_reserved = body.entries.iter().any(reserved);
        let all_reserved = body.entries.iter().all(reserved);
        if any_reserved && !all_reserved {
            let ordinary = body
                .entries
                .iter()
                .find(|entry| !reserved(entry))
                .and_then(|entry| root(entry).map(String::from));
            self.bag.push(
                Diagnostic::new(
                    E_BRACE_MIXED,
                    Phase::Check,
                    "this braced value mixes data fields with reserved envelope roots; wrap the data in `data = { … }`",
                )
                .with_span(at)
                .with_field("field", ordinary.unwrap_or_default()),
            );
            return None;
        }
        if !any_reserved {
            // A pure data record — the empty body included: `{}` is
            // an empty record, never an identity (identity is `.`).
            return self
                .struct_fields(body, signature)
                .map(EmitValue::FreshData);
        }
        // The envelope form. A whole-`data` assignment makes the
        // data fresh — FreshData-based, so a braced value under `data`
        // desugars to relative field writes of the fresh structure;
        // without one, the current envelope is patched. Detected on
        // the SYNTAX: after the record splice, no `Data([])` write
        // remains to look at.
        let whole_data = body.entries.iter().any(|entry| match entry {
            StructEntry::Field { key, .. } => {
                key.segments.len() == 1 && key.segments[0].text == "data"
            }
            StructEntry::Block { .. } => false,
        });
        let writes = self.field_writes(body, signature)?;
        if whole_data {
            Some(EmitValue::FreshData(writes))
        } else {
            Some(EmitValue::Patch(writes))
        }
    }

    fn field_writes(
        &mut self,
        body: &StructBody,
        signature: &expr::Signature,
    ) -> Option<Vec<FieldWrite>> {
        let mut writes = Vec::new();
        for entry in &body.entries {
            match entry {
                StructEntry::Field { key, value } => {
                    let path: Vec<String> = key
                        .segments
                        .iter()
                        .map(|segment| segment.text.clone())
                        .collect();
                    let segments: Vec<&str> = path.iter().map(String::as_str).collect();
                    // The writable surface, resolved here once: the
                    // runtime executes a verdict, never re-reads a
                    // string. Exactly three zones — no alias, no
                    // custom root (D-2026-08-10-1).
                    let target = match segments.as_slice() {
                        ["data", rest @ ..] => crate::envelope::WriteTarget::Data(
                            rest.iter().map(ToString::to_string).collect(),
                        ),
                        ["meta", "sys", ..] => {
                            self.meta_sys_write(key.span);
                            continue;
                        }
                        ["meta"] | ["meta", "app"] | ["meta", "ext"] => {
                            self.deferred("a whole-zone meta write", key.span);
                            continue;
                        }
                        ["meta", "app", rest @ ..] => {
                            crate::envelope::WriteTarget::App(rest.join("."))
                            // The write's TYPE joins the inference
                            // after the value compiles, below.
                        }
                        ["meta", "ext", rest @ ..] => {
                            let field = rest.join(".");
                            if !self.ext_field_known(&field) {
                                self.meta_zone_refusal(
                                    key.span,
                                    &format!("meta.ext.{field}"),
                                    "no connector sheet declares this `ext` field — the application's own vocabulary belongs to `meta.app`",
                                );
                                continue;
                            }
                            crate::envelope::WriteTarget::Ext(field)
                        }
                        ["meta", ..] => {
                            self.meta_zone_refusal(
                                key.span,
                                &segments.join("."),
                                "metadata has exactly three zones — `meta.sys`, `meta.app`, `meta.ext` (D-2026-08-10-1); there is no bare field and no custom root",
                            );
                            continue;
                        }
                        bare => crate::envelope::WriteTarget::Data(
                            bare.iter().map(ToString::to_string).collect(),
                        ),
                    };
                    match value {
                        SynValue::Expr(expr) => {
                            let whole_data = matches!(&target, crate::envelope::WriteTarget::Data(path) if path.is_empty());
                            let compiled = if whole_data {
                                self.payload_expr(expr, signature)
                            } else {
                                self.expr(expr, signature).map(|tree| (tree, None))
                            };
                            if let Some((tree, content_type)) = compiled {
                                let tree = if matches!(&target, crate::envelope::WriteTarget::Ext(field) if field == "content_type")
                                {
                                    let Some(tree) = self.contextualize_tree(
                                        tree,
                                        &DataType::MediaType,
                                        key.span,
                                        "meta.ext.content_type",
                                    ) else {
                                        continue;
                                    };
                                    tree
                                } else {
                                    tree
                                };
                                if matches!(
                                    &target,
                                    crate::envelope::WriteTarget::App(_)
                                        | crate::envelope::WriteTarget::Ext(_)
                                ) && let Some(why) = meta_type_refusal(&tree.ty)
                                {
                                    self.meta_zone_refusal(key.span, &key_path(key), why);
                                    continue;
                                }
                                if let crate::envelope::WriteTarget::App(field) = &target
                                    && !self.in_lab
                                {
                                    self.app_writes.push((
                                        field.clone(),
                                        tree.ty.clone(),
                                        key.span,
                                    ));
                                }
                                writes.push(FieldWrite {
                                    target,
                                    value: tree,
                                });
                                if let Some(content_type) = content_type {
                                    writes.push(FieldWrite {
                                        target: crate::envelope::WriteTarget::Ext(String::from(
                                            "content_type",
                                        )),
                                        value: literal_tree(
                                            crate::value::Value::Text(content_type),
                                            DataType::MediaType,
                                        ),
                                    });
                                }
                            }
                        }
                        // `data = { … }`: the braced value is a data
                        // RECORD — the whole-data assignment makes
                        // the build fresh, so its fields splice in as
                        // relative writes (construction, never a
                        // merge). Under a data PATH the base is the
                        // existing subtree, which the write fold
                        // cannot reset — deferred by name.
                        SynValue::Braced(nested) if matches!(&target, crate::envelope::WriteTarget::Data(inner) if inner.is_empty()) =>
                        {
                            self.record_fields(nested, signature, &[], &mut writes);
                        }
                        SynValue::Braced(_) => {
                            self.deferred("a record under a data path", key.span);
                        }
                        SynValue::Frontend(frontend)
                            if matches!(&target, crate::envelope::WriteTarget::Data(_)) =>
                        {
                            let crate::envelope::WriteTarget::Data(path) = &target else {
                                unreachable!("guarded")
                            };
                            let whole = path.is_empty();
                            let Some(call) = self.frontend_data(frontend) else {
                                continue;
                            };
                            let (tree, content_type) = match call {
                                FrontendCall::Json(value) => {
                                    (literal_tree(value, DataType::Variant), None)
                                }
                                FrontendCall::Text => (
                                    self.verbatim_body(frontend)
                                        .map(|value| literal_tree(value, DataType::Text))?,
                                    None,
                                ),
                                FrontendCall::Base64 { content_type } => (
                                    literal_tree(
                                        crate::value::Value::Bytes(self.base64_body(frontend)?),
                                        DataType::BYTES,
                                    ),
                                    content_type,
                                ),
                            };
                            if content_type.is_some() && !whole {
                                self.bag.push(
                                    Diagnostic::new(
                                        E_CONTENT_TYPE,
                                        Phase::Check,
                                        "a content type qualifies the WHOLE payload — a nested field has no `meta.ext` of its own",
                                    )
                                    .with_span(frontend.name.span)
                                    .with_field("tag", frontend.name.text.clone()),
                                );
                                continue;
                            }
                            writes.push(FieldWrite {
                                target: crate::envelope::WriteTarget::Data(path.clone()),
                                value: tree,
                            });
                            if let Some(content_type) = content_type {
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Ext(String::from(
                                        "content_type",
                                    )),
                                    value: literal_tree(
                                        crate::value::Value::Text(content_type),
                                        DataType::MediaType,
                                    ),
                                });
                            }
                        }
                        // `data = { "k": v }` — the map value is the
                        // whole payload (or a subtree).
                        SynValue::BracedMap(entries)
                            if matches!(&target, crate::envelope::WriteTarget::Data(_)) =>
                        {
                            if let Some(tree) = self.map_value_tree(entries, signature, key.span) {
                                writes.push(FieldWrite {
                                    target,
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Tagged { .. }
                            if matches!(&target, crate::envelope::WriteTarget::Data(_)) =>
                        {
                            if let Some(tree) = self.typed_value_tree(
                                value,
                                signature,
                                None,
                                key.span,
                                &key_path(key),
                            ) {
                                writes.push(FieldWrite {
                                    target,
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Error(_) => {}
                        _ => self.deferred("a nested envelope builder", key.span),
                    }
                }
                StructEntry::Block { name, .. } => {
                    // A nested block is a DEAD form (D-2026-08-12-1)
                    // — the migration speaks, never a generic
                    // deferral.
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "`{name} {{ … }}` is no longer a form; write `{name} = {{ … }}` \
                                 (or dotted `{name}.field = …` entries)",
                                name = name.text
                            ),
                        )
                        .with_span(name.span)
                        .with_field("attribute", name.text.clone()),
                    );
                }
            }
        }
        Some(writes)
    }

    /// Resolves the fields of a data RECORD — every path is a field
    /// path of the fresh structure, reserved envelope names inert. A
    /// braced VALUE nests another record: its writes splice in with
    /// the field's path as prefix (the base is fresh, so relative
    /// writes ARE construction). The counterpart of
    /// [`Self::field_writes`], which resolves the envelope form's
    /// writable surface.
    fn struct_fields(
        &mut self,
        body: &StructBody,
        signature: &expr::Signature,
    ) -> Option<Vec<FieldWrite>> {
        let mut writes = Vec::new();
        self.record_fields(body, signature, &[], &mut writes);
        Some(writes)
    }

    fn record_fields(
        &mut self,
        body: &StructBody,
        signature: &expr::Signature,
        prefix: &[String],
        writes: &mut Vec<FieldWrite>,
    ) {
        for entry in &body.entries {
            match entry {
                StructEntry::Field { key, value } => {
                    let mut path: Vec<String> = prefix.to_vec();
                    path.extend(key.segments.iter().map(|segment| segment.text.clone()));
                    match value {
                        SynValue::Expr(expr) => {
                            if let Some(tree) = self.expr(expr, signature) {
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Data(path),
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Braced(nested) => {
                            if nested.entries.is_empty() {
                                // An empty record is a VALUE, not an
                                // absence: `parent = {}` writes an
                                // empty struct where a leaf-splice
                                // would write nothing at all.
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Data(path),
                                    value: expr::Tree {
                                        root: expr::Expr::Literal(crate::value::Value::Struct(
                                            Vec::new(),
                                        )),
                                        ty: DataType::Variant,
                                    },
                                });
                            } else {
                                self.record_fields(nested, signature, &path, writes);
                            }
                        }
                        SynValue::BracedMap(entries) => {
                            // A map is an ordinary field value —
                            // `{ nested = { "a": 1 } }` composes.
                            if let Some(tree) = self.map_value_tree(entries, signature, key.span) {
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Data(path),
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Frontend(frontend) => {
                            if let Some(tree) = self.nested_value_tree(
                                &SynValue::Frontend(frontend.clone()),
                                signature,
                                key.span,
                            ) {
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Data(path),
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Tagged { .. } => {
                            if let Some(tree) = self.typed_value_tree(
                                value,
                                signature,
                                None,
                                key.span,
                                &key_path(key),
                            ) {
                                writes.push(FieldWrite {
                                    target: crate::envelope::WriteTarget::Data(path),
                                    value: tree,
                                });
                            }
                        }
                        SynValue::Error(_) => {}
                        _ => self.deferred("this record field value form", key.span),
                    }
                }
                StructEntry::Block { name, .. } => {
                    self.bag.push(
                        Diagnostic::new(
                            E_CONTRACT_ATTR,
                            Phase::Check,
                            format!(
                                "`{name} {{ … }}` is no longer a form; write `{name} = {{ … }}`",
                                name = name.text
                            ),
                        )
                        .with_span(name.span)
                        .with_field("attribute", name.text.clone()),
                    );
                }
            }
        }
    }

    /// The ONE notion of pinned-port conformance, shared by
    /// emissions and injections — every `EmitValue` form answers for
    /// the data it produces, statically, under the SAME construction
    /// model the runtime executes:
    ///
    /// - `Data` holds its logical type to the pin, and the physical
    ///   32-bit bound of `int` wherever the value is constant at
    ///   check — exactly the var-initializer rule; the runtime holds
    ///   the rest at emission construction.
    /// - `Identity` holds the INPUT's type (the activation port's
    ///   pin) to the output's pin.
    /// - `Patch` and `FreshData` FOLD their writes in order — the
    ///   order the runtime applies them: a whole-`data` write
    ///   replaces the running shape with that write's; a field write
    ///   turns it into structure; a metadata write leaves it alone.
    ///   The initial shape is the context's: the received envelope
    ///   in a handler, the draft's fresh map in an injection, a
    ///   fresh structure for `FreshData`.
    /// - `Nothing` produces nothing and always conforms.
    ///
    /// An UNKNOWN shape is a compiler inference variable and passes this
    /// early check; it is not an explicit `variant`. Pushes `E-PORT-TYPE`
    /// and answers false on a known breach.
    fn port_conforms(
        &mut self,
        pinned: &DataType,
        value: &EmitValue,
        origin: BuildOrigin<'_>,
        consts: &BTreeMap<String, crate::value::Value>,
        port_name: &str,
        span: Span,
    ) -> bool {
        let inherited_shape = || match origin {
            // A handler's Patch starts from the envelope it received,
            // whose data carries the activation port's pin.
            BuildOrigin::Handler(Some(ty)) => DataShape::Known(ty.clone(), None),
            BuildOrigin::Handler(None) => DataShape::Unknown,
            // A draft is born a fresh STRUCT (`lab::draft`) — a
            // Patch injection starts from it, so even a
            // metadata-only patch carries a struct through the
            // input.
            BuildOrigin::Draft => DataShape::Known(DataType::empty_struct(), None),
        };
        let shape = match value {
            EmitValue::Nothing => return true,
            EmitValue::Data(tree) => DataShape::Known(tree.ty.clone(), Some(tree)),
            EmitValue::Identity => inherited_shape(),
            EmitValue::Patch(writes) => match Self::folded_shape(writes, inherited_shape()) {
                Ok(shape) => shape,
                Err(error) => {
                    self.type_constructor_refusal(span, error);
                    return false;
                }
            },
            EmitValue::FreshData(writes) => {
                match Self::folded_shape(writes, DataShape::Known(DataType::empty_struct(), None)) {
                    Ok(shape) => shape,
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return false;
                    }
                }
            }
        };
        let found: Option<String> = match &shape {
            DataShape::Unknown => None,
            DataShape::Known(ty, tree) => {
                if !ty.is_assignable_to(pinned) {
                    Some(match value {
                        EmitValue::Identity => format!("{ty:?} (the input, unchanged)"),
                        _ => format!("{ty:?}"),
                    })
                } else {
                    tree.and_then(|tree| Self::constant_breach(pinned, tree, consts))
                }
            }
        };
        let Some(found) = found else {
            return true;
        };
        self.bag.push(
            Diagnostic::new(
                E_PORT_TYPE,
                Phase::Check,
                format!("`{port_name}` is pinned to {pinned:?}; this value is {found}"),
            )
            .with_span(span)
            .with_field("port", port_name.to_string())
            .with_field("expected", format!("{pinned:?}"))
            .with_field("found", found),
        );
        false
    }

    /// Folds a builder's writes IN ORDER — the runtime's order — over
    /// the running data shape.
    fn folded_shape<'w>(
        writes: &'w [FieldWrite],
        initial: DataShape<'w>,
    ) -> Result<DataShape<'w>, TypeError> {
        let mut shape = initial;
        for write in writes {
            match &write.target {
                crate::envelope::WriteTarget::Data(path) if path.is_empty() => {
                    shape = DataShape::Known(write.value.ty.clone(), Some(&write.value));
                }
                crate::envelope::WriteTarget::Data(path) => {
                    shape = match shape {
                        DataShape::Known(current, _) => DataShape::Known(
                            write_data_type_path(Some(current), path, write.value.ty.clone())?,
                            None,
                        ),
                        DataShape::Unknown => DataShape::Unknown,
                    };
                }
                crate::envelope::WriteTarget::App(_) | crate::envelope::WriteTarget::Ext(_) => {}
            }
        }
        Ok(shape)
    }

    /// The constant half of conformance: the 32-bit `int` bound,
    /// where the producing tree's value is known at check — the
    /// VISIBLE constants included, so a `val` alias of a literal
    /// answers exactly like the literal.
    fn constant_breach(
        pinned: &DataType,
        tree: &expr::Tree,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<String> {
        if *pinned == DataType::INT
            && let Ok(crate::value::Value::Int(value)) = expr::const_eval(tree, consts)
            && i32::try_from(value).is_err()
        {
            return Some(format!("the constant {value}, outside the 32-bit int"));
        }
        None
    }

    fn wrong_direction(&mut self, span: Span, port: &str, direction: Direction, required: &str) {
        let have = match direction {
            Direction::Input => "an input",
            Direction::Output => "an output",
        };
        self.bag.push(
            Diagnostic::new(
                E_PORT_DIRECTION,
                Phase::Check,
                format!("`{port}` is {have} port; {required}"),
            )
            .with_span(span)
            .with_field("port", port)
            .with_field(
                "direction",
                match direction {
                    Direction::Input => "input",
                    Direction::Output => "output",
                },
            ),
        );
    }
}

fn graph_at_path<'graph>(
    graph: &'graph Graph,
    path: &crate::plan::OwnerPath,
) -> Option<&'graph Graph> {
    if &graph.path == path {
        return Some(graph);
    }
    graph
        .graphs
        .iter()
        .find_map(|nested| graph_at_path(nested, path))
}

fn graph_at_path_mut<'graph>(
    graph: &'graph mut Graph,
    path: &crate::plan::OwnerPath,
) -> Option<&'graph mut Graph> {
    if &graph.path == path {
        return Some(graph);
    }
    graph
        .graphs
        .iter_mut()
        .find_map(|nested| graph_at_path_mut(nested, path))
}

fn node_at_path<'graph>(graph: &'graph Graph, path: &node::Path) -> Option<&'graph Node> {
    graph
        .nodes
        .iter()
        .find(|candidate| &candidate.path == path)
        .or_else(|| {
            graph
                .graphs
                .iter()
                .find_map(|nested| node_at_path(nested, path))
        })
}

fn node_at_path_mut<'graph>(
    graph: &'graph mut Graph,
    path: &node::Path,
) -> Option<&'graph mut Node> {
    if let Some(index) = graph
        .nodes
        .iter()
        .position(|candidate| &candidate.path == path)
    {
        return graph.nodes.get_mut(index);
    }
    graph
        .graphs
        .iter_mut()
        .find_map(|nested| node_at_path_mut(nested, path))
}

/// Where a builder's data construction starts — the two contexts a
/// pinned port is fed from.
#[derive(Clone, Copy)]
enum BuildOrigin<'a> {
    /// A handler emission: `Identity`/`Patch` start from the received
    /// envelope, whose data carries the activation port's pin.
    Handler(Option<&'a DataType>),
    /// A scenario injection: the draft is born as an empty map.
    Draft,
}

struct ResolvedRouting {
    edges: Vec<Edge>,
    rules: Vec<RouteRule>,
}

/// The statically known shape of the data a builder produces —
/// three DISTINCT states, never one `None` meaning all of them:
/// unknown (an open compiler constraint), or a known type (with the
/// producing tree when there is one, for the constant bound). Braced records are
/// represented by an actual closed `DataType::Struct`, never a sentinel.
enum DataShape<'a> {
    Unknown,
    Known(DataType, Option<&'a expr::Tree>),
}

/// The pin a node's port carries: its table entry when declared, or the
/// instantiated Connector contract's effective pin. The HIR and raw-plan
/// HIR and raw-plan paths call the same pure B2 resolver.
fn node_port_pin(node: &Node, port: &node::Port) -> Option<DataType> {
    node.ports
        .iter()
        .find(|decl| decl.port == *port)
        .and_then(|decl| decl.ty.clone())
}

fn common_node_attribute_name(attribute: &SynAttribute) -> Option<&str> {
    let name = attribute.target.segments.first()?.text.as_str();
    matches!(name, "instance" | "instances" | "lifetime" | "closing").then_some(name)
}

fn is_common_node_attribute(attribute: &SynAttribute) -> bool {
    common_node_attribute_name(attribute).is_some()
}

fn is_bare_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !text.as_bytes()[0].is_ascii_digit()
        && !matches!(text, "true" | "false" | "null")
}

fn direction_words(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "an input",
        Direction::Output => "an output",
    }
}

/// Rewrites every `Expr::Asset` index in a checked graph after the
/// table's canonical digest sort.
fn remap_graph_assets(graph: &mut Graph, remap: &[u32]) {
    fn expr(node: &mut expr::Expr, remap: &[u32]) {
        match node {
            expr::Expr::Asset(index) => {
                *index = remap[*index as usize];
            }
            expr::Expr::Literal(_) | expr::Expr::Path(_) => {}
            expr::Expr::Unary { expr: inner, .. }
            | expr::Expr::Is { expr: inner, .. }
            | expr::Expr::As { expr: inner, .. } => expr(inner, remap),
            expr::Expr::Binary { left, right, .. } => {
                expr(left, remap);
                expr(right, remap);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                expr(cond, remap);
                expr(then, remap);
                expr(otherwise, remap);
            }
            expr::Expr::Index { base, index } => {
                expr(base, remap);
                expr(index, remap);
            }
            expr::Expr::Call { args, .. } => {
                for arg in args {
                    expr(arg, remap);
                }
            }
            expr::Expr::List(items) => {
                for item in items {
                    expr(item, remap);
                }
            }
            expr::Expr::MapLit(pairs) => {
                for (_, value) in pairs {
                    expr(value, remap);
                }
            }
            expr::Expr::Struct(fields) | expr::Expr::Tagged { fields, .. } => {
                for (_, value) in fields {
                    expr(value, remap);
                }
            }
        }
    }
    fn emit(value: &mut EmitValue, remap: &[u32]) {
        match value {
            EmitValue::Data(tree) => expr(&mut tree.root, remap),
            EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
                for write in writes {
                    expr(&mut write.value.root, remap);
                }
            }
            EmitValue::Identity | EmitValue::Nothing => {}
        }
    }
    fn instance(policy: &mut InstancePolicy, remap: &[u32]) {
        if let Some(key) = &mut policy.key {
            expr(&mut key.root, remap);
        }
        match &mut policy.lifetime {
            InstanceLifetime::Idle(value)
            | InstanceLifetime::Fixed(value)
            | InstanceLifetime::Tumbling(value) => expr(&mut value.tree.root, remap),
            InstanceLifetime::Persistent | InstanceLifetime::SinglePass => {}
        }
        if let Some(closing) = &mut policy.closing {
            expr(&mut closing.drain_for.tree.root, remap);
            expr(&mut closing.settle_for.tree.root, remap);
        }
    }
    fn remap_handler(handler: &mut Handler, remap: &[u32]) {
        if let Some(within) = &mut handler.join_within {
            expr(&mut within.tree.root, remap);
        }
        if let Some(guard) = &mut handler.admission_guard {
            expr(&mut guard.root, remap);
        }
        fn statement(statement: &mut Statement, remap: &[u32]) {
            match statement {
                Statement::Let { value, .. } | Statement::SetVar { value, .. } => {
                    expr(&mut value.root, remap);
                }
                Statement::Emit { value, .. } => emit(value, remap),
                Statement::MultiEmit { values, .. } => match values {
                    MultiEmitValues::Broadcast(value) => emit(value, remap),
                    MultiEmitValues::Tuple(values) => {
                        for value in values {
                            emit(value, remap);
                        }
                    }
                },
                Statement::LoopCommand(LoopCommand::Next(value)) => emit(value, remap),
                Statement::Stop
                | Statement::LoopCommand(LoopCommand::Again | LoopCommand::Done) => {}
            }
        }
        fn remap_dispatch(tree: &mut Dispatch, remap: &[u32]) {
            match tree {
                Dispatch::Linear(statements) => {
                    for item in statements {
                        statement(item, remap);
                    }
                }
                Dispatch::ClosedMatch {
                    prefix,
                    scrutinee,
                    arms,
                } => {
                    for item in prefix {
                        statement(item, remap);
                    }
                    expr(&mut scrutinee.root, remap);
                    for arm in arms {
                        remap_dispatch(&mut arm.dispatch, remap);
                    }
                }
                Dispatch::OpenMatch {
                    prefix,
                    arms,
                    fallback,
                } => {
                    for item in prefix {
                        statement(item, remap);
                    }
                    for arm in arms {
                        expr(&mut arm.predicate.root, remap);
                        remap_dispatch(&mut arm.dispatch, remap);
                    }
                    remap_dispatch(fallback, remap);
                }
            }
        }
        remap_dispatch(&mut handler.dispatch, remap);
    }
    if let Some(policy) = &mut graph.instance {
        instance(policy, remap);
    }
    if let Some(spec) = &mut graph.loop_spec {
        if let Some(fuel) = &mut spec.fuel {
            expr(&mut fuel.tree.root, remap);
        }
        match &mut spec.delay {
            None => {}
            Some(LoopDelay::Fixed { after }) => expr(&mut after.tree.root, remap),
            Some(LoopDelay::Exponential { first, ceiling }) => {
                expr(&mut first.tree.root, remap);
                expr(&mut ceiling.tree.root, remap);
            }
        }
        if let Some(proof) = &mut spec.proof {
            expr(&mut proof.measure.root, remap);
            if let Some(invariant) = &mut proof.invariant {
                expr(&mut invariant.root, remap);
            }
        }
    }
    for handler in &mut graph.handlers {
        remap_handler(handler, remap);
    }
    for node in &mut graph.nodes {
        if let Some(policy) = &mut node.instance {
            instance(policy, remap);
        }
        if let NodeSemantics::Connector { attributes, .. } = &mut node.semantics {
            for attribute in attributes {
                match &mut attribute.value {
                    AttrValue::Const(constant) => expr(&mut constant.tree.root, remap),
                    AttrValue::Expr { tree, .. } => expr(&mut tree.root, remap),
                    AttrValue::Word(_) => {}
                }
            }
        }
        for val in &mut node.vals {
            expr(&mut val.value.tree.root, remap);
        }
        for handler in &mut node.handlers {
            remap_handler(handler, remap);
        }
    }
    for val in &mut graph.vals {
        expr(&mut val.value.tree.root, remap);
    }
    for nested in &mut graph.graphs {
        remap_graph_assets(nested, remap);
    }
}

/// The ONE static admissibility rule for metadata leaves
/// (D-2026-08-10-1): `scalar | array<scalar>` — a statically known
/// type outside it refuses at CHECK; `variant` passes and the runtime's
/// `to_meta_value` guards it (defense in depth, same rule).
fn meta_type_refusal(ty: &DataType) -> Option<&'static str> {
    let leaf_scalar = |candidate: &DataType| {
        matches!(
            candidate,
            DataType::Bool | DataType::Text | DataType::MediaType | DataType::Variant
        ) || candidate == &DataType::INT
            || candidate == &DataType::DOUBLE
            || candidate == &DataType::BYTES
    };
    if leaf_scalar(ty) {
        None
    } else if ty == &DataType::UINT {
        Some("the uint domain stays in expressions — a metadata leaf takes `int`")
    } else if let DataType::Array(array) = ty {
        if leaf_scalar(array.element()) {
            None
        } else {
            Some(
                "a metadata leaf is `scalar | array<scalar>` — uint and nested composites stay out",
            )
        }
    } else if ty == &DataType::DURATION || ty == &DataType::TIMESTAMP {
        Some(
            "a duration or timestamp metadata leaf is outside this build's physical form — carry it in `data` for now",
        )
    } else {
        Some("a metadata leaf is `scalar | array<scalar>` — composites belong in `data`")
    }
}

fn key_path(key: &crate::compiler::syntax::LValue) -> String {
    key.segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Canonical flat field-name set when a braced value can be judged as one
/// closed struct. Detailed refusals stay at the construction door.
fn source_field_names(body: &StructBody) -> Option<Vec<String>> {
    let mut names = Vec::with_capacity(body.entries.len());
    for entry in &body.entries {
        let StructEntry::Field { key, .. } = entry else {
            return None;
        };
        let [name] = key.segments.as_slice() else {
            return None;
        };
        names.push(name.text.clone());
    }
    names.sort();
    names.dedup();
    Some(names)
}

/// A literal expression tree — the compiler's own constants.
fn literal_tree(value: crate::value::Value, ty: DataType) -> expr::Tree {
    expr::Tree {
        root: expr::Expr::Literal(value),
        ty,
    }
}

/// The verbatim margin rule (D-2026-08-10-6): drop one leading
/// newline; the segment after the LAST newline (the closing brace's
/// indentation) is the margin, stripped from every line; the final
/// newline is dropped with it. A single-line body is itself.
fn strip_verbatim_margin(raw: &str) -> String {
    let Some(body) = raw.strip_prefix('\n') else {
        return String::from(raw);
    };
    let Some(last_newline) = body.rfind('\n') else {
        return String::from(body);
    };
    let margin = &body[last_newline + 1..];
    if !margin.chars().all(|c| c == ' ') {
        return String::from(body);
    }
    let content = &body[..last_newline];
    content
        .split('\n')
        .map(|line| line.strip_prefix(margin).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// RFC 4648 standard base64: mandatory padding, ASCII whitespace
/// ignored, URL-safe alphabet refused (D-2026-08-10-6).
fn decode_base64(raw: &str) -> Result<Vec<u8>, String> {
    let mut symbols: Vec<u8> = Vec::new();
    for ch in raw.bytes() {
        match ch {
            b' ' | b'\n' | b'\r' | b'\t' => {}
            b'A'..=b'Z' => symbols.push(ch - b'A'),
            b'a'..=b'z' => symbols.push(ch - b'a' + 26),
            b'0'..=b'9' => symbols.push(ch - b'0' + 52),
            b'+' => symbols.push(62),
            b'/' => symbols.push(63),
            b'=' => symbols.push(64),
            b'-' | b'_' => {
                return Err(String::from(
                    "the URL-safe alphabet is refused — RFC 4648 standard only",
                ));
            }
            other => {
                return Err(format!(
                    "`{}` is outside the base64 alphabet",
                    other as char
                ));
            }
        }
    }
    if !symbols.len().is_multiple_of(4) {
        return Err(String::from(
            "the length is not a multiple of four — padding is mandatory",
        ));
    }
    let mut bytes = Vec::with_capacity(symbols.len() / 4 * 3);
    let chunk_count = symbols.len() / 4;
    for (index, chunk) in symbols.chunks(4).enumerate() {
        let pads = chunk.iter().filter(|&&s| s == 64).count();
        if pads > 0 && index + 1 != chunk_count {
            return Err(String::from("padding may only end the body"));
        }
        if chunk[..4 - pads].contains(&64) || pads > 2 {
            return Err(String::from("padding may only end the body"));
        }
        let [a, b, c, d] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        if a == 64 || b == 64 {
            return Err(String::from("padding may only end the body"));
        }
        bytes.push((a << 2) | (b >> 4));
        if c != 64 {
            bytes.push((b << 4) | (c >> 2));
        }
        if d != 64 {
            bytes.push((c << 6) | d);
        }
    }
    Ok(bytes)
}

fn write_data_type_path(
    current: Option<DataType>,
    path: &[String],
    leaf: DataType,
) -> Result<DataType, TypeError> {
    let Some((head, rest)) = path.split_first() else {
        return Ok(leaf);
    };
    let mut fields: BTreeMap<String, DataType> = match current {
        Some(DataType::Struct(structure)) => structure
            .fields()
            .iter()
            .map(|field| (field.name().to_string(), field.ty().clone()))
            .collect(),
        _ => BTreeMap::new(),
    };
    let nested = write_data_type_path(fields.remove(head), rest, leaf)?;
    fields.insert(head.clone(), nested);
    StructType::new(
        fields
            .into_iter()
            .map(|(name, ty)| TypeField::new(name, ty))
            .collect(),
    )
    .map(DataType::Struct)
}

fn inferred_join(left: &DataType, right: &DataType) -> Result<DataType, TypeError> {
    if left.is_assignable_to(right) {
        Ok(right.clone())
    } else if right.is_assignable_to(left) {
        Ok(left.clone())
    } else {
        DataType::structural_union(vec![left.clone(), right.clone()])
    }
}

fn expression_reads_data(expr: &expr::Expr) -> bool {
    let mut stack = vec![expr];
    while let Some(expr) = stack.pop() {
        match expr {
            expr::Expr::Path(path) if path.first().is_some_and(|root| root == "data") => {
                return true;
            }
            expr::Expr::Path(_) | expr::Expr::Literal(_) | expr::Expr::Asset(_) => {}
            expr::Expr::Unary { expr, .. }
            | expr::Expr::Is { expr, .. }
            | expr::Expr::As { expr, .. } => stack.push(expr),
            expr::Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push(cond);
                stack.push(then);
                stack.push(otherwise);
            }
            expr::Expr::Index { base, index } => {
                stack.push(base);
                stack.push(index);
            }
            expr::Expr::Call { args, .. } | expr::Expr::List(args) => stack.extend(args),
            expr::Expr::MapLit(entries) => {
                stack.extend(entries.iter().map(|(_, value)| value));
            }
            expr::Expr::Struct(fields) | expr::Expr::Tagged { fields, .. } => {
                stack.extend(fields.iter().map(|(_, value)| value));
            }
        }
    }
    false
}

/// Earliest runtime phase required by one checked expression. Source-visible
/// immutable vals are plan constants in the current frontend; the two runtime
/// roots and `instance.key` are therefore the only later bindings here. The
/// package validator independently re-derives this claim from the transported
/// owner environment.
fn expression_evaluation_site(expr: &expr::Expr) -> crate::plan::EvaluationSite {
    use crate::plan::EvaluationSite;

    let mut site = EvaluationSite::Plan;
    let mut stack = vec![expr];
    while let Some(expr) = stack.pop() {
        match expr {
            expr::Expr::Path(path)
                if path
                    .first()
                    .is_some_and(|root| matches!(root.as_str(), "data" | "meta")) =>
            {
                return EvaluationSite::Invocation;
            }
            expr::Expr::Path(path)
                if matches!(path.as_slice(), [instance, key, ..]
                    if instance == "instance" && key == "key") =>
            {
                site = site.max(EvaluationSite::Instance);
            }
            expr::Expr::Path(_) | expr::Expr::Literal(_) | expr::Expr::Asset(_) => {}
            expr::Expr::Unary { expr, .. }
            | expr::Expr::Is { expr, .. }
            | expr::Expr::As { expr, .. } => stack.push(expr),
            expr::Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push(cond);
                stack.push(then);
                stack.push(otherwise);
            }
            expr::Expr::Index { base, index } => {
                stack.push(base);
                stack.push(index);
            }
            expr::Expr::Call { args, .. } | expr::Expr::List(args) => stack.extend(args),
            expr::Expr::MapLit(entries) => {
                stack.extend(entries.iter().map(|(_, value)| value));
            }
            expr::Expr::Struct(fields) | expr::Expr::Tagged { fields, .. } => {
                stack.extend(fields.iter().map(|(_, value)| value));
            }
        }
    }
    site
}

fn value_type(value: &crate::value::Value) -> Result<Option<DataType>, TypeError> {
    Ok(Some(match value {
        crate::value::Value::Struct(fields) => {
            let mut resolved = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                let Some(ty) = value_type(value)? else {
                    return Ok(None);
                };
                resolved.push(TypeField::new(name.clone(), ty));
            }
            DataType::Struct(StructType::new(resolved)?)
        }
        crate::value::Value::Decimal { .. } => DataType::Number,
        crate::value::Value::Bool(_) => DataType::Bool,
        crate::value::Value::Int(value) => {
            if i32::try_from(*value).is_ok() {
                DataType::INT
            } else {
                DataType::LONG
            }
        }
        crate::value::Value::Uint(_) => DataType::UINT,
        crate::value::Value::Double(_) => DataType::DOUBLE,
        crate::value::Value::Text(_) => DataType::Text,
        crate::value::Value::Bytes(_) => DataType::BYTES,
        crate::value::Value::Duration(_) => DataType::DURATION,
        crate::value::Value::Timestamp(_) => DataType::TIMESTAMP,
        // The materialized carrier does not repeat temporal parameters or a
        // keyed map's schema. These cases acquire their precise type from a
        // port/schema constraint; none is a context-free source literal.
        crate::value::Value::Uuid(_) => DataType::Uuid,
        crate::value::Value::Date(_)
        | crate::value::Value::Time(_)
        | crate::value::Value::TimestampNtz(_)
        | crate::value::Value::Interval { .. }
        | crate::value::Value::KeyedMap(_) => DataType::Variant,
        crate::value::Value::Null => DataType::Null,
        crate::value::Value::Array(items) => {
            let mut element = None;
            for item in items {
                let Some(item) = value_type(item)? else {
                    return Ok(None);
                };
                element = Some(match element {
                    Some(current) => inferred_join(&current, &item)?,
                    None => item,
                });
            }
            let Some(element) = element else {
                return Ok(None);
            };
            DataType::array(element)
        }
        crate::value::Value::Map(entries) => {
            let mut element = None;
            for item in entries.values() {
                let Some(item) = value_type(item)? else {
                    return Ok(None);
                };
                element = Some(match element {
                    Some(current) => inferred_join(&current, &item)?,
                    None => item,
                });
            }
            let Some(element) = element else {
                return Ok(None);
            };
            DataType::text_map(element)
        }
        crate::value::Value::Enum(_)
        | crate::value::Value::StructuralUnion { .. }
        | crate::value::Value::TaggedUnion(_) => DataType::Variant,
    }))
}

#[cfg(test)]
mod tests;
