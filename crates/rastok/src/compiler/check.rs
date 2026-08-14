//! Resolution and semantic checking: syntax trees in, HIR out.
//!
//! The checker resolves names, desugars default ports and envelope
//! sugar into semantic forms, compiles every admitted Rastok expression,
//! and refuses explicitly — never accidentally — every grammatical
//! construct outside the executable subset of this build
//! (`E-CHECK-DEFERRED`). Rules with a normative home carry their own
//! codes; `meta.sys` writes are the canonical example.

use crate::compiler::hir::{
    AttrValue, Attribute, Const, Direction, Edge, EdgeTarget, EmitValue, Endpoint, FieldWrite,
    Graph, Handler, Hir, Node, NodeRole, Statement, Val, Var,
};
use crate::compiler::syntax::{
    BaseType, ConnectorDecl, EdgeDecl, EdgeEndpoint, ExprSource, GraphDecl, GraphItem,
    OperatorDecl, OperatorItem, PortRef, ScalarType, Statement as SynStatement, StructBody,
    StructEntry, SyntaxTree, TopDecl, Trigger as SynTrigger, TypeRef, ValDecl, Value as SynValue,
    VarDecl,
};
use crate::contract;
use crate::datatype::{
    DataType, DateUnit, DecimalType, EnumType, MapType, StructType, TemporalType, TimeUnit,
    TypeError, TypeField,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};
use crate::expr;
use crate::node;
use std::collections::BTreeMap;

/// Diagnostic for a unit without exactly one `: App` graph.
pub const E_APP_COUNT: Code = Code::new("E-APP-COUNT");
/// Diagnostic for a duplicate node name inside one graph.
pub const E_DUP_NODE: Code = Code::new("E-DUP-NODE");
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
    DepthBudget { measured: usize },
    NodeBudget { measured: usize },
    WidthBudget { what: &'static str, measured: usize },
}
/// Diagnostic adapted from the expression chain, span mapped to the source.
pub const E_EXPR: Code = Code::new("E-EXPR");
/// Diagnostic for grammar admitted but outside the executable subset.
pub const E_CHECK_DEFERRED: Code = Code::new("E-CHECK-DEFERRED");
/// Diagnostic for a connector type without an adapter in this build.
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

/// Checks a unit into HIR against a contract registry. Diagnostics
/// accumulate in `bag`; `None` means the unit has no single `: App`
/// graph to check. A returned HIR is meaningful only when the bag
/// holds no errors.
#[must_use]
pub fn check(
    unit: &Unit,
    registry: &contract::Registry,
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
        type_stack: Vec::new(),
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
    let checked_app = checker.graph(app);
    // The production schema seals HERE — from the deployed graph's
    // writes alone. Everything the Lab compiles below is checked
    // against it and can extend nothing (D-2026-08-10-1).
    checker.seal_app_schema();
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
    let mut checked_app = checked_app;
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
    // The standard `content_type` is IMPLICIT — known to every
    // consumer, never serialized (D-2026-08-10-4); the schema
    // carries the sheets' fields only.
    let ext_schema: Vec<(String, DataType)> = checker
        .ext_fields
        .iter()
        .map(|field| (field.clone(), DataType::Variant))
        .collect();
    Some(Hir {
        app: checked_app,
        labs,
        app_schema,
        ext_schema,
        assets: checker.assets,
    })
}

struct Checker<'bag, 'registry> {
    bag: &'bag mut Bag,
    registry: &'registry contract::Registry,
    /// The `ext` vocabulary the instantiated connectors' sheets
    /// declare — the standard `content_type` is always known.
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
    /// The expansion stack — a name on it twice is a cycle.
    type_stack: Vec<String>,
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
                self.collect_meta_reads(&tree.root, source.span);
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
                    Some(value) => Some(literal(self, value)?),
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
        match expr::const_eval(&tree, consts) {
            Ok(value) => Some(Const { tree, value }),
            Err(error) => {
                self.bag.push(
                    Diagnostic::new(
                        E_EXPR,
                        Phase::Check,
                        format!("this position requires a constant: {error}"),
                    )
                    .with_span(expr.span)
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
            if node.connector_trait().is_none() {
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
                    if checked
                        .body
                        .iter()
                        .any(|statement| matches!(statement, Statement::Emit { port, .. } if port.name() == "err"))
                    {
                        self.deferred("a mock err reply", handler.span);
                        continue;
                    }
                    // A reply carries data only: its envelope is
                    // minted by the connector's own transaction —
                    // the verdict lands here, not mid-simulation.
                    let writes_meta = checked.body.iter().any(|statement| {
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
                    // The injection target is a graph INPUT of the
                    // App — the implicit `in` or a declared boundary
                    // input; the boundary convention makes a boundary
                    // source an input of the graph. Source-Connector
                    // injection still joins with the source adapters.
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
                    let Some(value) = self.emit_value(value, &signature, target.span) else {
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
                    if let Some(expected) = app
                        .boundary
                        .iter()
                        .find(|decl| decl.port == port)
                        .and_then(|decl| decl.ty.clone())
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
                    let target = match segments.as_slice() {
                        ["app", "err"] => ExpectTarget::RootErr,
                        ["app", port] => {
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
                            match node::Port::new((*port).to_string()) {
                                Ok(port) => ExpectTarget::Boundary(port),
                                Err(_) => continue,
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
                            let known = matches!(*port, "in" | "out" | "err")
                                || node.ports.iter().any(|decl| decl.port.name() == *port);
                            if !known {
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
                            }
                            match node::Port::new((*port).to_string()) {
                                Ok(port) => ExpectTarget::Node {
                                    path: node.path.clone(),
                                    port,
                                },
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
                                .with("data", DataType::Variant)
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
        self.guard_binding_name(&decl.name, "a graph");
        if let Some(annotation) = &decl.annotation {
            for symbol in &annotation.symbols {
                if symbol.text != "App" {
                    self.deferred("a graph annotation other than App", symbol.span);
                }
            }
        }
        if decl.per.is_some() {
            self.deferred("a keyed graph (per)", decl.span);
        }
        let mut graph = Graph {
            name: decl.name.text.clone(),
            boundary: Vec::new(),
            vals: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let mut consts = BTreeMap::new();
        let mut signature = expr::Signature::new();
        let mut edges = Vec::new();
        let mut redeclared_defaults = std::collections::BTreeSet::new();
        for item in &decl.body {
            match item {
                GraphItem::Port(port) => {
                    if !self.guard_binding_name(&port.name, "a boundary port") {
                        continue;
                    }
                    // The annotation resolves through the same type
                    // subset as state cells — its deferrals (long,
                    // float, unions) refuse identically.
                    let pinned = match &port.ty {
                        None => None,
                        Some(type_ref) => match self.state_type(type_ref) {
                            Some((mapped, _)) => Some(mapped),
                            None => continue,
                        },
                    };
                    let direction = match port.direction {
                        crate::compiler::syntax::PortDirection::Input => Direction::Input,
                        crate::compiler::syntax::PortDirection::Output => Direction::Output,
                    };
                    // The conventional ports are pre-bound in direction,
                    // but an explicit annotation is still a real source
                    // constraint. An untyped restatement remains absent
                    // from the raw v6 table; a typed one travels so B2 can
                    // resolve it without treating `variant` as a fallback.
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
                        } else if let Some(ty) = pinned {
                            match node::Port::new(port.name.text.clone()) {
                                Ok(declared) => graph.boundary.push(crate::plan::PortDecl {
                                    port: declared,
                                    direction,
                                    ty: Some(ty),
                                }),
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
                        Ok(declared) => graph.boundary.push(crate::plan::PortDecl {
                            port: declared,
                            direction,
                            ty: pinned,
                        }),
                        Err(invalid) => self.bag.push(
                            Diagnostic::new(E_PORT_UNKNOWN, Phase::Check, invalid.to_string())
                                .with_span(port.name.span),
                        ),
                    }
                }
                GraphItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some(value) = self.graph_val(val, &signature, &consts)
                    {
                        let Some(ty) = self.inferred_value_type(&value.value, val.span) else {
                            continue;
                        };
                        signature = signature.with(val.name.text.clone(), ty);
                        consts.insert(val.name.text.clone(), value.value.clone());
                        graph.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                        });
                    }
                }
                GraphItem::Operator(operator) => {
                    if let Some(node) = self.operator(operator, &signature, &consts) {
                        self.push_node(&mut graph, node, operator.name.span);
                    }
                }
                GraphItem::Connector(connector) => {
                    if let Some(node) = self.connector(connector, &signature, &consts) {
                        self.push_node(&mut graph, node, connector.name.span);
                    }
                }
                GraphItem::Edge(edge) => edges.push(edge),
                GraphItem::Partition(partition) => {
                    self.deferred("a partition", partition.span);
                }
                GraphItem::Graph(nested) => self.deferred("a nested graph", nested.span),
                GraphItem::Defaults(_) => self.deferred("a defaults block", decl.span),
                GraphItem::Attribute(attribute) => {
                    self.deferred("a graph attribute", attribute.span);
                }
                GraphItem::Handler(handler) => self.deferred("a graph handler", handler.span),
                GraphItem::Error(_) => {}
            }
        }
        for edge in edges {
            let Some(resolved) = self.edge(edge, &graph) else {
                continue;
            };
            for candidate in resolved {
                // Duplicates are compared AFTER desugaring and
                // resolution: `relay -> sink` and
                // `relay.out -> sink.in` are one pair of endpoints.
                if graph.edges.contains(&candidate) {
                    self.bag.push(
                        Diagnostic::new(E_DUP_EDGE, Phase::Check, "this edge is already declared")
                            .with_span(edge.span)
                            .with_field("edge", edge_display(&candidate)),
                    );
                    continue;
                }
                graph.edges.push(candidate);
            }
        }
        self.reject_cycles(&graph, decl.span);
        self.warn_unwired_outputs(&graph);
        graph
    }

    /// Every non-`err` output either reaches an edge or is discarded
    /// in writing (`-> nothing`); a port that just leaks gets flagged.
    /// `err` is exempt: unwired errors bubble to the root sink.
    fn warn_unwired_outputs(&mut self, graph: &Graph) {
        for node in &graph.nodes {
            // Untyped conventional ports do not enter the raw table, but an
            // emission on them leaks all the same. A typed restatement may
            // already be present, so sweep emitted ports beside the rows.
            let mut outputs: Vec<node::Port> = node
                .ports
                .iter()
                .filter(|decl| decl.direction == Direction::Output)
                .map(|decl| decl.port.clone())
                .collect();
            for handler in &node.handlers {
                for statement in &handler.body {
                    if let Statement::Emit { port, .. } = statement
                        && !outputs.contains(port)
                    {
                        outputs.push(port.clone());
                    }
                }
            }
            for port in &outputs {
                if port.name() == "err" {
                    continue;
                }
                let wired = graph.edges.iter().any(|edge| {
                    edge.from.node.as_ref() == Some(&node.path) && edge.from.port == *port
                });
                if !wired {
                    self.bag.push(
                        Diagnostic::new(
                            W_UNWIRED_OUTPUT,
                            Phase::Check,
                            format!(
                                "`{}.{port}` is emitted but wired nowhere; \
                                 route it or discard it with `-> nothing`",
                                node.path
                            ),
                        )
                        .with_field("node", node.path.to_string())
                        .with_field("port", port.name().to_string()),
                    );
                }
            }
        }
    }

    /// Wiring is a DAG (`grammar.md`): a cycle among nodes refuses.
    fn reject_cycles(&mut self, graph: &Graph, span: Span) {
        let index_of = |path: &node::Path| {
            graph
                .nodes
                .iter()
                .position(|candidate| &candidate.path == path)
        };
        let mut successors = vec![Vec::new(); graph.nodes.len()];
        for edge in &graph.edges {
            if let (Some(from), EdgeTarget::Port(to)) = (&edge.from.node, &edge.to)
                && let (Some(from), Some(to)) = (
                    from.segments().first().and_then(|_| index_of(from)),
                    to.node.as_ref().and_then(&index_of),
                )
            {
                successors[from].push(to);
            }
        }
        // Iterative three-state DFS: 0 unvisited, 1 on the stack, 2 done.
        let mut state = vec![0u8; graph.nodes.len()];
        for start in 0..graph.nodes.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            state[start] = 1;
            while let Some((current, next_child)) = stack.pop() {
                if let Some(&child) = successors[current].get(next_child) {
                    stack.push((current, next_child + 1));
                    match state[child] {
                        0 => {
                            state[child] = 1;
                            stack.push((child, 0));
                        }
                        1 => {
                            let name = graph.nodes[child].path.to_string();
                            self.bag.push(
                                Diagnostic::new(
                                    E_GRAPH_CYCLE,
                                    Phase::Check,
                                    format!("the wiring cycles through `{name}`; edges form a DAG"),
                                )
                                .with_span(span)
                                .with_field("node", name),
                            );
                            return;
                        }
                        _ => {}
                    }
                } else {
                    state[current] = 2;
                }
            }
        }
    }

    fn push_node(&mut self, graph: &mut Graph, node: Node, span: Span) {
        let name = node.path.to_string();
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
        } else if graph
            .nodes
            .iter()
            .any(|existing| existing.path == node.path)
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
        } else {
            graph.nodes.push(node);
        }
    }

    fn graph_val(
        &mut self,
        val: &ValDecl,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Const> {
        if val.ty.is_some() {
            self.deferred("a typed val", val.span);
        }
        match &val.value {
            // The parser already spoke for its recovery form.
            SynValue::Error(_) => None,
            SynValue::Expr(expr) => self.constant(expr, signature, consts),
            _ => {
                self.deferred("a non-expression val initializer", val.span);
                None
            }
        }
    }

    fn operator(
        &mut self,
        decl: &OperatorDecl,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Node> {
        if let Some(annotation) = &decl.annotation {
            for symbol in &annotation.symbols {
                self.deferred("an operator alias or aspect", symbol.span);
            }
        }
        if decl.per.is_some() {
            self.deferred("a keyed node (per)", decl.span);
        }
        let path = self.node_path(&decl.name.text, decl.name.span)?;
        let mut node = Node {
            path,
            role: NodeRole::Operator,
            vals: Vec::new(),
            vars: Vec::new(),
            ports: Vec::new(),
            handlers: Vec::new(),
        };
        self.node_body(&decl.body, &mut node, graph_signature, graph_consts, false);
        Some(node)
    }

    fn connector(
        &mut self,
        decl: &ConnectorDecl,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Node> {
        let mut symbols = decl.annotation.symbols.iter();
        let trait_name = symbols
            .next()
            .map(|symbol| symbol.text.clone())
            .unwrap_or_else(|| unreachable!("the parser mandates a connector annotation"));
        for aspect in symbols {
            self.deferred("a connector aspect", aspect.span);
        }
        if decl.per.is_some() {
            self.deferred("a keyed node (per)", decl.span);
        }
        let path = self.node_path(&decl.name.text, decl.name.span)?;
        // The RESOLVED contract materializes into the node — error
        // set and the non-implicit port table alike: the plan must
        // carry everything its embedded validator will check, because
        // that validator has no Registry to consult (m1-slice's
        // transport rule). Types join when the sheets carry them.
        let (error_set, contract_ports) = match self.registry.connector(&trait_name) {
            contract::Availability::Available(contract) => {
                let mut ports = Vec::new();
                for (name, direction) in contract
                    .input_ports
                    .iter()
                    .map(|name| (*name, Direction::Input))
                    .chain(
                        contract
                            .output_ports
                            .iter()
                            .map(|name| (*name, Direction::Output)),
                    )
                {
                    if !matches!(name, "in" | "out" | "err") {
                        // Sheets are validated at the registry door
                        // (`Registry::try_with`), so the name IS a
                        // port name.
                        let port = node::Port::new(name.to_string())
                            .unwrap_or_else(|_| unreachable!("a validated sheet"));
                        ports.push(crate::plan::PortDecl::untyped(port, direction));
                    }
                }
                (
                    contract.error_set.iter().map(ToString::to_string).collect(),
                    ports,
                )
            }
            _ => (Vec::new(), Vec::new()),
        };
        let mut node = Node {
            path,
            role: NodeRole::Connector {
                error_set,
                trait_name,
                attributes: Vec::new(),
            },
            vals: Vec::new(),
            vars: Vec::new(),
            ports: contract_ports,
            handlers: Vec::new(),
        };
        self.node_body(&decl.body, &mut node, graph_signature, graph_consts, true);
        self.contract(&mut node, decl);
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
        let symbol = &decl.annotation.symbols[0];
        let contract = match self.registry.connector(&symbol.text) {
            contract::Availability::Available(contract) => contract,
            contract::Availability::Declared => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!(
                            "`{}` is declared in the catalogue; its adapter is not in this build",
                            symbol.text
                        ),
                    )
                    .with_span(symbol.span)
                    .with_field("type", symbol.text.clone()),
                );
                return;
            }
            contract::Availability::Unknown => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!("`{}` is not a known connector type", symbol.text),
                    )
                    .with_span(symbol.span)
                    .with_field("type", symbol.text.clone()),
                );
                return;
            }
        };
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
                OperatorItem::Attribute(attribute) => Some(
                    attribute
                        .target
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                ),
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
                        AttrValue::Const(_) => {
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
                        if attribute.name == "effect" && word != "idempotent" {
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
                (
                    contract::ValueDomain::Text
                    | contract::ValueDomain::TextList
                    | contract::ValueDomain::Duration,
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
        if let NodeRole::Connector { attributes, .. } = &mut node.role {
            attributes.extend(injected);
        }
    }

    fn node_path(&mut self, name: &str, span: Span) -> Option<node::Path> {
        match node::Path::new([name]) {
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

    fn node_body(
        &mut self,
        body: &[OperatorItem],
        node: &mut Node,
        graph_signature: &expr::Signature,
        graph_consts: &BTreeMap<String, crate::value::Value>,
        is_connector: bool,
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
                    let pinned = match &port.ty {
                        None => None,
                        Some(type_ref) => match self.state_type(type_ref) {
                            Some((mapped, _)) => Some(mapped),
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
                }
                OperatorItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some(value) = self.graph_val(val, &signature, &consts)
                    {
                        let Some(ty) = self.inferred_value_type(&value.value, val.span) else {
                            continue;
                        };
                        signature = signature.with(val.name.text.clone(), ty);
                        consts.insert(val.name.text.clone(), value.value.clone());
                        node.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                        });
                    }
                }
                OperatorItem::Var(var) => {
                    if self.bindable(&var.name, &signature)
                        && let Some(cell) = self.var(var, &signature, &consts)
                    {
                        signature = signature.with(cell.name.clone(), cell.ty.clone());
                        node.vars.push(cell);
                    }
                }
                OperatorItem::Attribute(attribute) => {
                    let name = attribute
                        .target
                        .segments
                        .iter()
                        .map(|segment| segment.text.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    if node.connector_trait().is_none() {
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
                    } else if let Some(checked) =
                        self.attribute_value(&attribute.value, &signature, &consts, attribute.span)
                        && let NodeRole::Connector { attributes, .. } = &mut node.role
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
        }
        // Pass three: handlers, checked against the complete table.
        for item in body {
            if let OperatorItem::Handler(handler) = item {
                if is_connector {
                    self.deferred("a connector handler", handler.span);
                } else if let Some(checked) = self.handler(handler, &signature, node, &consts) {
                    // Multi-handler arbitration (which fires? does a
                    // false guard fall through?) has no specified rule
                    // yet — and the emission-id recipe carries no
                    // handler identity, so two firings could collide.
                    if node
                        .handlers
                        .iter()
                        .any(|existing| existing.activation == checked.activation)
                    {
                        self.deferred("a second handler for one trigger", handler.span);
                    } else {
                        node.handlers.push(checked);
                    }
                }
            }
        }
    }

    /// Adds one port to a node's semantic table, refusing conflicting
    /// roles. Conventional ports remain implicit when merely used, but
    /// an explicit type annotation enters the table like any other source
    /// constraint. A matching inferred use never overwrites that pin.
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
        let existing = table_entry
            .map(|index| node.ports[index].direction)
            .or(default_direction);
        let is_conventional = default_direction.is_some();
        let push = |node: &mut Node, ty: Option<DataType>| {
            if let Ok(port) = node::Port::new(name.text.clone()) {
                node.ports.push(crate::plan::PortDecl {
                    port,
                    direction,
                    ty,
                });
                true
            } else {
                false
            }
        };
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
                if let Some(ty) = pinned {
                    if let Some(index) = table_entry {
                        node.ports[index].ty = Some(ty);
                    } else if is_conventional && !push(node, Some(ty)) {
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
            None => {
                if !push(node, pinned) {
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
            let Some(statements) = &handler.body else {
                continue;
            };
            for statement in statements {
                let SynStatement::Assign { target, .. } = statement else {
                    continue;
                };
                if target.segments.len() != 1 {
                    continue;
                }
                let name = &target.segments[0];
                let is_state = node.vars.iter().any(|var| var.name == name.text)
                    || node.vals.iter().any(|val| val.name == name.text)
                    || name.text == "err";
                if !is_state {
                    self.add_port(node, name, Direction::Output, None, "an emission target");
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
        let (ty, int_bounded) = self.state_type(&decl.ty)?;
        let init = match &decl.value {
            None => None,
            Some(SynValue::Expr(expr)) => {
                let constant = self.constant(expr, signature, consts)?;
                if int_bounded
                    && let crate::value::Value::Int(value) = constant.value
                    && i32::try_from(value).is_err()
                {
                    self.bag.push(
                        Diagnostic::new(
                            E_EXPR,
                            Phase::Check,
                            format!("`{value}` does not fit `int` (32-bit)"),
                        )
                        .with_span(expr.span)
                        .with_field("detail", "out of the 32-bit int range"),
                    );
                    return None;
                }
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
                        .with_span(expr.span)
                        .with_field("detail", format!("{init_ty} does not fit {ty}")),
                    );
                    return None;
                }
                Some(constant)
            }
            // The parser already spoke for its recovery form.
            Some(SynValue::Error(_)) => return None,
            Some(_) => {
                self.deferred("a non-expression var initializer", decl.span);
                return None;
            }
        };
        Some(Var {
            name: decl.name.text.clone(),
            ty,
            init,
        })
    }

    /// Whether `name` may introduce a binding here: reserved handler
    /// names and shadowing are refused — which is also what keeps
    /// compile-time constant substitution sound.
    /// THE naming policy, one door for every live declaration
    /// (D-2026-08-14-1): the structural value form `nothing` and the
    /// expression literals (`true`/`false`/`null`) refuse in every
    /// binding-introducing position. Frontend names such as `json`
    /// remain ordinary identifiers because only the adjacent `!`
    /// selects a frontend.
    /// The refusal lands AT THE DECLARATION, one voice, the name's
    /// span.
    fn guard_binding_name(&mut self, name: &crate::compiler::syntax::Name, what: &str) -> bool {
        let reason = match name.text.as_str() {
            "nothing" => "a value form of the language",
            "true" | "false" | "null" => "a literal of the expression language",
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

    /// Resolves a source type structurally into the canonical core algebra.
    /// Named declarations are aliases only; no source spelling survives in
    /// the result. The boolean keeps the existing focused diagnostic for the
    /// friendly 32-bit `int` alias.
    fn state_type(&mut self, ty: &TypeRef) -> Option<(DataType, bool)> {
        let mapped = self.resolve_type_ref(ty)?;
        let int_bounded = mapped == DataType::INT;
        Some((mapped, int_bounded))
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

    fn resolve_type_ref(&mut self, ty: &TypeRef) -> Option<DataType> {
        let mut alternatives = Vec::with_capacity(ty.alternatives.len() + usize::from(ty.optional));
        for alternative in &ty.alternatives {
            alternatives.push(self.resolve_base_type(alternative, ty.span)?);
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

    fn resolve_base_type(&mut self, ty: &BaseType, span: Span) -> Option<DataType> {
        let resolved = match ty {
            BaseType::Scalar(ScalarType::Bool) => DataType::Bool,
            BaseType::Scalar(ScalarType::Int) => DataType::INT,
            BaseType::Scalar(ScalarType::Long) => DataType::LONG,
            BaseType::Scalar(ScalarType::Float) => DataType::FLOAT,
            BaseType::Scalar(ScalarType::Double) => DataType::DOUBLE,
            BaseType::Scalar(ScalarType::Number) => DataType::Number,
            BaseType::Scalar(ScalarType::Bytes) => DataType::BYTES,
            BaseType::Scalar(ScalarType::Text) => DataType::Text,
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
                        self.resolve_type_ref(&field.ty)?,
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
                let key = self.resolve_type_ref(key)?;
                let value = self.resolve_type_ref(value)?;
                match MapType::new(key, value) {
                    Ok(map) => DataType::Map(map),
                    Err(error) => {
                        self.type_constructor_refusal(span, error);
                        return None;
                    }
                }
            }
            BaseType::Array(element) => DataType::array(self.resolve_type_ref(element)?),
            BaseType::Table(element) => {
                let element = self.resolve_type_ref(element)?;
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
                let resolved = self.resolve_type_ref(&aliased);
                self.type_stack.pop();
                return resolved;
            }
        };
        Some(resolved)
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
            _ => {
                self.deferred("this attribute value form", span);
                None
            }
        }
    }

    fn handler(
        &mut self,
        decl: &crate::compiler::syntax::HandlerDecl,
        node_signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Handler> {
        let activation = match &decl.trigger {
            SynTrigger::Port(name) => {
                let Some(direction) = self.node_port_direction(node, &name.text) else {
                    self.bag.push(
                        Diagnostic::new(
                            E_PORT_UNKNOWN,
                            Phase::Check,
                            format!("this node exposes no port named `{}`", name.text),
                        )
                        .with_span(name.span)
                        .with_field("port", name.text.clone()),
                    );
                    return None;
                };
                if direction != Direction::Input {
                    self.wrong_direction(
                        name.span,
                        &name.text,
                        direction,
                        "a handler listens on an input port",
                    );
                    return None;
                }
                match node::Port::new(name.text.clone()) {
                    Ok(port) => node::HandlerActivation::Port(port),
                    Err(_) => return None,
                }
            }
            SynTrigger::Start(span) => {
                self.deferred("a start lifecycle handler", *span);
                return None;
            }
            SynTrigger::Stop(span) => {
                self.deferred("a stop lifecycle handler", *span);
                return None;
            }
            SynTrigger::Compensate(span) => {
                self.deferred("a compensate handler", *span);
                return None;
            }
            SynTrigger::Aspect { aspect, .. } => {
                self.deferred("an aspect-qualified handler", aspect.span);
                return None;
            }
            SynTrigger::Join(_) => {
                self.deferred("a join handler", decl.span);
                return None;
            }
        };
        // A source annotation or instantiated Connector contract on the
        // activation port TYPES the handler's `data`. An unpinned operator
        // port remains a compiler inference variable; `variant` below is only
        // the expression checker's temporary signature, never the port's
        // inferred effective type.
        let declared_data_type = match &activation {
            node::HandlerActivation::Port(port) => node_port_pin(node, port),
        };
        // The expression engine still needs a concrete signature during B2;
        // its `variant` here is parser/checker scaffolding only. The separate
        // `declared_data_type` is what prevents that placeholder from being
        // mistaken for an explicit dynamic port constraint.
        let data_type = declared_data_type.clone().unwrap_or(DataType::Variant);
        let mut signature = node_signature
            .clone()
            .with("data", data_type.clone())
            .with("meta", DataType::Variant);
        let mut guard = None;
        for clause in &decl.guards {
            match clause {
                crate::compiler::syntax::Guard::Where(expr) if guard.is_none() => {
                    let tree = self.expr(expr, &signature)?;
                    if !matches!(tree.ty, DataType::Bool | DataType::Variant) {
                        self.bag.push(
                            Diagnostic::new(E_EXPR, Phase::Check, "a where guard must be a bool")
                                .with_span(expr.span)
                                .with_field("detail", "the guard type is not bool"),
                        );
                        return None;
                    }
                    signature = signature.narrowed_by(&tree.root);
                    guard = Some(tree);
                }
                crate::compiler::syntax::Guard::Where(expr) => {
                    self.deferred("a second where guard", expr.span);
                }
                _ => {
                    self.deferred("a join guard clause", decl.span);
                }
            }
        }
        let Some(body) = &decl.body else {
            self.deferred("a body-less handler outside Loop", decl.span);
            return None;
        };
        let mut statements = Vec::new();
        for statement in body {
            match statement {
                SynStatement::Val(val) => match &val.value {
                    // The parser already spoke for its recovery form.
                    SynValue::Error(_) => {}
                    SynValue::Expr(expr) => {
                        if self.bindable(&val.name, &signature)
                            && let Some(tree) = self.expr(expr, &signature)
                        {
                            signature = signature.with(val.name.text.clone(), tree.ty.clone());
                            statements.push(Statement::Let {
                                name: val.name.text.clone(),
                                value: tree,
                            });
                        }
                    }
                    _ => self.deferred("a non-expression local val", val.span),
                },
                SynStatement::Assign { target, ty, value } => {
                    if ty.is_some() {
                        self.deferred("a typed assignment", target.span);
                    }
                    if target.segments.len() > 1 {
                        if target.segments[0].text == "meta" && target.segments[1].text == "sys" {
                            self.meta_sys_write(target.span);
                        } else {
                            self.deferred("a dotted statement target", target.span);
                        }
                        continue;
                    }
                    let name = &target.segments[0];
                    if let Some(var) = node.vars.iter().find(|var| var.name == name.text) {
                        match value {
                            // The parser already spoke for its
                            // recovery form.
                            SynValue::Error(_) => {}
                            SynValue::Expr(expr) => {
                                if let Some(tree) = self.expr(expr, &signature) {
                                    let unresolved_input = declared_data_type.is_none()
                                        && tree.ty == DataType::Variant
                                        && expression_reads_data(&tree.root);
                                    if !unresolved_input && !tree.ty.is_assignable_to(&var.ty) {
                                        self.bag.push(
                                            Diagnostic::new(
                                                E_EXPR,
                                                Phase::Check,
                                                format!(
                                                    "`{}` is a {:?}, this value is a {:?}",
                                                    name.text, var.ty, tree.ty
                                                ),
                                            )
                                            .with_span(expr.span)
                                            .with_field(
                                                "detail",
                                                format!("{:?} does not fit {:?}", tree.ty, var.ty),
                                            ),
                                        );
                                        continue;
                                    }
                                    statements.push(Statement::SetVar {
                                        name: name.text.clone(),
                                        value: tree,
                                    });
                                }
                            }
                            _ => self.deferred("a non-expression state write", target.span),
                        }
                        continue;
                    }
                    let Some(direction) = self.node_port_direction(node, &name.text) else {
                        self.bag.push(
                            Diagnostic::new(
                                E_RESOLVE_UNKNOWN,
                                Phase::Resolve,
                                format!(
                                    "`{}` names neither a port nor a state cell here",
                                    name.text
                                ),
                            )
                            .with_span(name.span)
                            .with_field("name", name.text.clone()),
                        );
                        continue;
                    };
                    if direction != Direction::Output {
                        self.wrong_direction(
                            name.span,
                            &name.text,
                            direction,
                            "an emission targets an output port",
                        );
                        continue;
                    }
                    let Ok(port) = node::Port::new(name.text.clone()) else {
                        continue;
                    };
                    if let Some(emit) = self.emit_value(value, &signature, name.span) {
                        // A pinned output holds its emissions to the
                        // contract — ONE conformance notion, every
                        // EmitValue form.
                        if let Some(expected) = node_port_pin(node, &port)
                            && !self.port_conforms(
                                &expected,
                                &emit,
                                BuildOrigin::Handler(declared_data_type.as_ref()),
                                consts,
                                port.name(),
                                name.span,
                            )
                        {
                            continue;
                        }
                        statements.push(Statement::Emit { port, value: emit });
                    }
                }
                SynStatement::ErrEmit { kind, .. } => {
                    self.deferred("a typed-error emission", kind.span);
                }
                SynStatement::MultiEmit { targets, .. } => {
                    let span = targets.first().map_or(decl.span, |target| target.span);
                    self.deferred("an atomic multi-emit", span);
                }
                SynStatement::Stop(_) => self.deferred("a stop action", decl.span),
                SynStatement::Error(_) => {}
            }
        }
        Some(Handler {
            activation,
            guard,
            body: statements,
        })
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
                    [only] => Some(FrontendCall::Base64 {
                        content_type: Some(string_arg(self, only)?),
                    }),
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
            if let Some(resolved) = self.resolve_type_ref(&ty) {
                self.resolved_types.insert(name, resolved);
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
            let tree = self.nested_value_tree(&entry.value, signature, at)?;
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

    /// Whether an `ext` field belongs to the declared vocabulary:
    /// the standard `content_type` plus whatever the instantiated
    /// connectors' sheets declare (none in the M0 catalogue yet).
    fn ext_field_known(&self, field: &str) -> bool {
        field == "content_type" || self.ext_fields.contains(field)
    }

    /// Records every `meta.*` path a compiled expression reads —
    /// verified against the sealed schema once inference closes.
    /// A DYNAMIC meta access (`meta["…"]`) refuses on the spot: the
    /// grid only holds if the checker sees every key.
    fn collect_meta_reads(&mut self, expr: &expr::Expr, span: Span) {
        match expr {
            expr::Expr::Path(segments) => {
                if segments.first().is_some_and(|root| root == "meta") {
                    self.meta_reads.push((segments.clone(), span));
                }
            }
            expr::Expr::Index { base, index } => {
                if matches!(&**base, expr::Expr::Path(segments)
                    if segments.first().is_some_and(|root| root == "meta"))
                {
                    self.meta_zone_refusal(
                        span,
                        "meta[…]",
                        "dynamic metadata access defeats the static grid — spell the key",
                    );
                }
                self.collect_meta_reads(base, span);
                self.collect_meta_reads(index, span);
            }
            expr::Expr::Unary { expr, .. }
            | expr::Expr::Is { expr, .. }
            | expr::Expr::As { expr, .. } => self.collect_meta_reads(expr, span),
            expr::Expr::Binary { left, right, .. } => {
                self.collect_meta_reads(left, span);
                self.collect_meta_reads(right, span);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.collect_meta_reads(cond, span);
                self.collect_meta_reads(then, span);
                self.collect_meta_reads(otherwise, span);
            }
            expr::Expr::Call { args, .. } => {
                for arg in args {
                    self.collect_meta_reads(arg, span);
                }
            }
            expr::Expr::List(items) => {
                for item in items {
                    self.collect_meta_reads(item, span);
                }
            }
            expr::Expr::MapLit(pairs) => {
                for (_, value) in pairs {
                    self.collect_meta_reads(value, span);
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
            // their aspect scope (envelope.md §1.2).
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
                            "the sys spine has five fields plus the aspect birth stamps — nothing else",
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
        at: Span,
    ) -> Option<EmitValue> {
        match value {
            // The parser already spoke for its recovery form.
            SynValue::Error(_) => None,
            SynValue::Identity(_) => Some(EmitValue::Identity),
            SynValue::Braced(body) => self.braced(body, signature, at),
            SynValue::Nothing(_) => Some(EmitValue::Nothing),
            SynValue::Expr(expr) => {
                let (tree, content_type) = self.payload_expr(expr, signature)?;
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
                                DataType::Text,
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
                self.map_value_tree(entries, signature, at)
                    .map(EmitValue::Data)
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
                                DataType::Text,
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
                                if matches!(&target, crate::envelope::WriteTarget::Ext(field) if field == "content_type")
                                    && !tree.ty.is_assignable_to(&DataType::Text)
                                {
                                    self.bag.push(
                                        Diagnostic::new(
                                            E_EXPR,
                                            Phase::Check,
                                            "content_type takes text",
                                        )
                                        .with_span(key.span)
                                        .with_field("detail", "the value type is not text"),
                                    );
                                    continue;
                                }
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
                                            DataType::Text,
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
                                        DataType::Text,
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

    fn edge(&mut self, decl: &EdgeDecl, graph: &Graph) -> Option<Vec<Edge>> {
        if decl.bidirectional {
            self.deferred("a bidirectional edge", decl.span);
            return None;
        }
        if decl.guard.is_some() {
            self.deferred("an edge guard (when)", decl.span);
            return None;
        }
        if decl.config.is_some() {
            self.deferred("an edge configuration block", decl.span);
            return None;
        }
        let mut resolved = Vec::new();
        for pair in decl.endpoints.windows(2) {
            let from = self.endpoint(&pair[0], graph, true)?;
            let to = match &pair[1] {
                EdgeEndpoint::Nothing(_) => EdgeTarget::Discard,
                other => EdgeTarget::Port(self.endpoint(other, graph, false)?),
            };
            // Two pinned ends must be directionally compatible at wiring
            // time. An unpinned side remains open here and is closed by the
            // validated-plan resolver.
            if let EdgeTarget::Port(target) = &to
                && let Some(source_ty) = endpoint_pinned_type(&from, graph)
                && let Some(target_ty) = endpoint_pinned_type(target, graph)
                && !source_ty.is_assignable_to(&target_ty)
            {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_TYPE,
                        Phase::Check,
                        format!(
                            "this edge connects {source_ty:?} to {target_ty:?}; pinned ends must agree"
                        ),
                    )
                    .with_span(decl.span)
                    .with_field("port", target.port.name().to_string())
                    .with_field("expected", format!("{target_ty:?}"))
                    .with_field("found", format!("{source_ty:?}")),
                );
                return None;
            }
            resolved.push(Edge { from, to });
        }
        Some(resolved)
    }

    fn endpoint(
        &mut self,
        endpoint: &EdgeEndpoint,
        graph: &Graph,
        is_source: bool,
    ) -> Option<Endpoint> {
        let reference = match endpoint {
            EdgeEndpoint::Ref(reference) => reference,
            EdgeEndpoint::Branch(refs) => {
                let span = refs.first().map_or_else(
                    || Span::new(SourceId::new(0), 0..0),
                    |reference| reference.span,
                );
                self.deferred("a branch edge", span);
                return None;
            }
            EdgeEndpoint::Nothing(span) => {
                self.deferred("nothing as an edge source", *span);
                return None;
            }
        };
        self.port_ref(reference, graph, is_source)
    }

    /// The direction a port has on a node: declared ports first, then
    /// the connector contract's port sheet, then the pre-bound
    /// defaults. `None` means the node exposes no such port.
    fn node_port_direction(&self, target: &Node, port_name: &str) -> Option<Direction> {
        // ONE source of truth: the node's resolved table (contract
        // ports materialize into it at resolution) plus the implicit
        // trio. The Registry is never consulted again — a sheet
        // defect cannot make the checker and the plan's validator
        // disagree.
        if let Some(decl) = target
            .ports
            .iter()
            .find(|decl| decl.port.name() == port_name)
        {
            return Some(decl.direction);
        }
        match port_name {
            "in" => Some(Direction::Input),
            "out" | "err" => Some(Direction::Output),
            _ => None,
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

    fn port_ref(
        &mut self,
        reference: &PortRef,
        graph: &Graph,
        is_source: bool,
    ) -> Option<Endpoint> {
        if reference.boundary {
            // As a source the boundary feeds the graph (an input port
            // of the graph); as a target it drains it (an output).
            let expected = if is_source {
                Direction::Input
            } else {
                Direction::Output
            };
            let port_name = match reference.segments.first() {
                None => {
                    if is_source {
                        "in"
                    } else {
                        "out"
                    }
                }
                Some(name) => {
                    let Some(decl) = graph
                        .boundary
                        .iter()
                        .find(|decl| decl.port.name() == name.text)
                    else {
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_UNKNOWN,
                                Phase::Check,
                                format!("`.{}` names no declared boundary port", name.text),
                            )
                            .with_span(name.span)
                            .with_field("port", name.text.clone()),
                        );
                        return None;
                    };
                    if decl.direction != expected {
                        let required = if is_source {
                            "a boundary edge source must be an input of the graph"
                        } else {
                            "a boundary edge target must be an output of the graph"
                        };
                        self.wrong_direction(name.span, &name.text, decl.direction, required);
                        return None;
                    }
                    name.text.as_str()
                }
            };
            return node::Port::new(port_name)
                .ok()
                .map(|port| Endpoint { node: None, port });
        }
        let node_name = &reference.segments[0];
        let Some(target) = graph
            .nodes
            .iter()
            .find(|candidate| candidate.path.segments()[0] == node_name.text)
        else {
            self.bag.push(
                Diagnostic::new(
                    E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!("`{}` names no node in this graph", node_name.text),
                )
                .with_span(node_name.span)
                .with_field("name", node_name.text.clone()),
            );
            return None;
        };
        let port_name = match reference.segments.len() {
            1 => {
                if is_source {
                    "out"
                } else {
                    "in"
                }
            }
            2 => reference.segments[1].text.as_str(),
            _ => {
                self.deferred("a composed edge path", reference.span);
                return None;
            }
        };
        // A connector's non-default port is transported (the resolved
        // table travels in the plan) but not yet EXECUTABLE: the
        // Partner seam, the mocks, and the dispatcher still speak
        // in/out/err. Accept-then-execute-differently is worse than
        // refusal — it defers by name until the connector items land.
        if target.connector_trait().is_some() && !matches!(port_name, "in" | "out" | "err") {
            self.deferred("a connector edge beyond the default trio", reference.span);
            return None;
        }
        let Some(direction) = self.node_port_direction(target, port_name) else {
            self.bag.push(
                Diagnostic::new(
                    E_PORT_UNKNOWN,
                    Phase::Check,
                    format!("`{}` exposes no port named `{}`", node_name.text, port_name),
                )
                .with_span(reference.span)
                .with_field("port", port_name),
            );
            return None;
        };
        let expected = if is_source {
            Direction::Output
        } else {
            Direction::Input
        };
        if direction != expected {
            let required = if is_source {
                "an edge source must be an output of its node"
            } else {
                "an edge target must be an input of its node"
            };
            self.wrong_direction(reference.span, port_name, direction, required);
            return None;
        }
        node::Port::new(port_name).ok().map(|port| Endpoint {
            node: Some(target.path.clone()),
            port,
        })
    }
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
/// adapters call the same pure B2 resolver.
fn node_port_pin(node: &Node, port: &node::Port) -> Option<DataType> {
    node.ports
        .iter()
        .find(|decl| decl.port == *port)
        .and_then(|decl| decl.ty.clone())
        .or_else(|| {
            // The derivation itself lives in one boundary helper; B5 moves
            // it into the contract metamodel without changing callers.
            let NodeRole::Connector {
                trait_name,
                attributes,
                ..
            } = &node.role
            else {
                return None;
            };
            let word = attributes.iter().find_map(|attribute| {
                if attribute.name != "codec" {
                    return None;
                }
                match &attribute.value {
                    crate::compiler::hir::AttrValue::Word(word) => Some(word.as_str()),
                    crate::compiler::hir::AttrValue::Const(_) => None,
                }
            });
            rastok_core::boundary::resolved_contract_port_type(trait_name, port.name(), word)
        })
}

/// The pinned type of an edge endpoint, when its port declares one:
/// a node's table entry (the connector's effective out pin
/// included), or a declared boundary port. An unannotated conventional or
/// named port answers `None` here and is closed by validated-plan inference.
fn endpoint_pinned_type(endpoint: &Endpoint, graph: &Graph) -> Option<DataType> {
    match &endpoint.node {
        None => graph
            .boundary
            .iter()
            .find(|decl| decl.port == endpoint.port)
            .and_then(|decl| decl.ty.clone()),
        Some(path) => graph
            .nodes
            .iter()
            .find(|node| &node.path == path)
            .and_then(|node| node_port_pin(node, &endpoint.port)),
    }
}

/// The canonical text of one checked edge — the human face of
/// E-DUP-EDGE, mirroring the plan's rendering.
fn edge_display(edge: &Edge) -> String {
    let end = |endpoint: &Endpoint| match &endpoint.node {
        Some(path) => format!("{path}.{}", endpoint.port),
        None => format!(".{}", endpoint.port),
    };
    let to = match &edge.to {
        EdgeTarget::Port(endpoint) => end(endpoint),
        EdgeTarget::Discard => String::from("!discard"),
    };
    format!("{}>{to}", end(&edge.from))
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
    for node in &mut graph.nodes {
        for val in &mut node.vals {
            expr(&mut val.value.tree.root, remap);
        }
        for handler in &mut node.handlers {
            if let Some(guard) = &mut handler.guard {
                expr(&mut guard.root, remap);
            }
            for statement in &mut handler.body {
                match statement {
                    Statement::Let { value, .. } | Statement::SetVar { value, .. } => {
                        expr(&mut value.root, remap);
                    }
                    Statement::Emit { value, .. } => emit(value, remap),
                }
            }
        }
    }
    for val in &mut graph.vals {
        expr(&mut val.value.tree.root, remap);
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
            DataType::Bool | DataType::Text | DataType::Variant
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
        }
    }
    false
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
        // The exact value is not an inference hole. v6 can only transport
        // the historical `variant` tree type; B8 switches this bridge to the
        // canonical `number` type together with the single v7 registry.
        crate::value::Value::Decimal { .. } => DataType::Variant,
        crate::value::Value::Bool(_) => DataType::Bool,
        crate::value::Value::Int(_) => DataType::INT,
        crate::value::Value::Uint(_) => DataType::UINT,
        crate::value::Value::Double(_) => DataType::DOUBLE,
        crate::value::Value::Text(_) => DataType::Text,
        crate::value::Value::Bytes(_) => DataType::BYTES,
        crate::value::Value::Duration(_) => DataType::DURATION,
        crate::value::Value::Timestamp(_) => DataType::TIMESTAMP,
        // The materialized carrier does not repeat temporal parameters or a
        // keyed map's schema. These cases acquire their precise type from a
        // port/schema constraint; none is a source literal in v6.
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
mod tests {
    use super::*;
    use crate::compiler::parser::parse;

    fn check_source(source: &str) -> (Option<Hir>, Bag) {
        let mut bag = Bag::new();
        let registry = contract::Registry::try_m0().expect("the standard catalogue holds");
        let tree = parse(SourceId::new(0), source, &registry, &mut bag);
        let unit = Unit {
            files: vec![(SourceId::new(0), tree)],
        };
        let hir = check(&unit, &registry, &mut bag, None);
        (hir, bag)
    }

    fn diagnostic_codes(bag: &Bag) -> Vec<Code> {
        bag.diagnostics()
            .iter()
            .map(crate::diagnostics::Diagnostic::code)
            .collect()
    }

    fn check_clean(source: &str) -> Hir {
        let (hir, bag) = check_source(source);
        assert!(
            !bag.has_errors(),
            "unexpected diagnostics: {:#?}",
            bag.diagnostics()
        );
        hir.expect("an app graph")
    }

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  val greeting = "hello"

  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_checks_into_hir() {
        let hir = check_clean(M0_PROBE);
        assert_eq!(hir.app.name, "m0_probe");
        assert_eq!(hir.app.vals[0].name, "greeting");
        assert_eq!(hir.app.nodes.len(), 2);
        let echo = &hir.app.nodes[0];
        assert_eq!(echo.category(), node::Kind::Connector);
        assert_eq!(echo.connector_trait(), Some("Process"));
        // The EFFECTIVE sheet (D-2026-08-12-1): the five written
        // attributes plus the injected defaults — `args = []` and,
        // on the job profile, `max_output = "8MiB"` and
        // `on_timeout = kill`. `mode` was written here.
        assert_eq!(echo.attributes().len(), 8);
        assert_eq!(
            echo.attributes()[0].value,
            AttrValue::Word(String::from("job"))
        );
        let AttrValue::Const(command) = &echo.attributes()[1].value else {
            panic!("expected a constant attribute");
        };
        assert_eq!(
            command.value,
            crate::value::Value::Text(String::from("cat"))
        );
        let shout = &hir.app.nodes[1];
        assert_eq!(shout.category(), node::Kind::Operator);
        assert_eq!(shout.vars[0].ty, DataType::INT);
        let handler = &shout.handlers[0];
        assert_eq!(
            handler.activation,
            node::HandlerActivation::Port(node::Port::new("in").unwrap())
        );
        assert!(handler.guard.is_some());
        assert!(matches!(&handler.body[0], Statement::SetVar { name, .. } if name == "count"));
        let Statement::Emit { port, value } = &handler.body[1] else {
            panic!("expected the emission");
        };
        assert_eq!(port.name(), "out");
        let EmitValue::Data(tree) = value else {
            panic!("expected a whole-data expression");
        };
        assert!(matches!(&tree.root, expr::Expr::Path(path) if path == &["data", "text"]));
    }

    #[test]
    fn edges_desugar_to_explicit_pairs() {
        let hir = check_clean(M0_PROBE);
        let edges = &hir.app.edges;
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[0].from.node, None);
        assert_eq!(edges[0].from.port.name(), "in");
        let EdgeTarget::Port(to) = &edges[0].to else {
            panic!("expected a port target");
        };
        assert_eq!(to.node.as_ref().unwrap().to_string(), "shout");
        assert_eq!(to.port.name(), "in");
        let EdgeTarget::Port(last) = &edges[2].to else {
            panic!("expected a port target");
        };
        assert_eq!(last.node, None);
        assert_eq!(last.port.name(), "out");
    }

    #[test]
    fn a_meta_sys_write_carries_its_normative_code() {
        let source = "graph g : App {\n  operator o {\n    on in {\n      out = { meta.sys.id = data.x }\n    }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_META_SYS_READONLY)
            .expect("the meta.sys refusal");
        assert!(diagnostic.message().contains("read-only"));
    }

    #[test]
    fn a_unit_needs_exactly_one_app() {
        let (hir, bag) = check_source("graph a : App { }\ngraph b : App { }\n");
        assert!(hir.is_none());
        assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
        let (hir, bag) = check_source("graph a { }\n");
        assert!(hir.is_none());
        assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
    }

    #[test]
    fn unknown_names_and_ports_are_refused() {
        let source = "graph g : App {\n  operator o { }\n  o -> ghost\n  o.mystery -> o\n}\n";
        let (_, bag) = check_source(source);
        let codes: Vec<_> = bag
            .diagnostics()
            .iter()
            .map(crate::diagnostics::Diagnostic::code)
            .collect();
        assert!(codes.contains(&E_RESOLVE_UNKNOWN));
        assert!(codes.contains(&E_PORT_UNKNOWN));
    }

    #[test]
    fn duplicate_nodes_are_refused() {
        let (_, bag) = check_source("graph g : App {\n  operator o { }\n  operator o { }\n}\n");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_DUP_NODE)
        );
    }

    #[test]
    fn expression_faults_map_back_to_source_spans() {
        let source = "graph g : App {\n  operator o {\n    on in where mystery > 0 { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_EXPR)
            .expect("a Rastok expression diagnostic");
        let span = diagnostic.spans()[0];
        let range = span.range();
        assert_eq!(&source[range], "mystery > 0");
    }

    #[test]
    fn a_missing_mandatory_axis_violates_the_sheet() {
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n    behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_REQUIRED)
            .expect("the missing-effect refusal");
        assert_eq!(diagnostic.fields()[0].1, "effect");
    }

    #[test]
    fn a_domain_violation_names_the_sheet() {
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = utf16\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
            .expect("the codec refusal");
        assert!(diagnostic.message().contains("Process sheet"));
    }

    #[test]
    fn correlate_on_a_job_names_the_owning_profile() {
        // D-2026-08-12-1: a field of another profile is ABSENT here,
        // and the diagnostic names its owner — never "forbidden".
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n    correlate = data.id\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
            .expect("the correlate refusal");
        assert!(
            diagnostic
                .message()
                .contains("belongs to `Process(mode = service)`"),
            "{}",
            diagnostic.message()
        );
        assert!(
            diagnostic
                .message()
                .contains("not applicable to `Process(mode = job)`"),
            "{}",
            diagnostic.message()
        );
    }

    #[test]
    fn unavailable_and_unknown_connector_types_split() {
        let declared = "graph g : App {\n  connector h : HttpServer { }\n  . -> h -> .\n}\n";
        let (_, bag) = check_source(declared);
        assert!(diagnostic_codes(&bag).contains(&E_CONNECTOR_UNAVAILABLE));
        assert!(bag.diagnostics()[0].message().contains("catalogue"));
        let unknown = "graph g : App {\n  connector d : Postgres { }\n  . -> d -> .\n}\n";
        let (_, bag) = check_source(unknown);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("not a known connector type"))
        );
    }

    #[test]
    fn a_map_emission_compiles_to_the_map_literal_form() {
        // `{ "n": 1 }` is an executable emission now — the entries
        // compile into the existing MapLit expression.
        let source = "graph g : App {\n  operator o {\n    on in { out = { \"n\": 1 } }\n  }\n  . -> o -> .\n}\n";
        let (hir, bag) = check_source(source);
        assert!(!bag.has_errors(), "{:#?}", bag.diagnostics());
        let hir = hir.expect("a checked unit");
        let Statement::Emit { value, .. } = &hir.app.nodes[0].handlers[0].body[0] else {
            panic!("an emission");
        };
        let EmitValue::Data(tree) = value else {
            panic!("a data emission");
        };
        assert!(
            matches!(&tree.root, expr::Expr::MapLit(pairs) if pairs.len() == 1),
            "the MapLit form: {tree:?}"
        );
    }

    #[test]
    fn a_named_type_guard_narrows_the_handler_body() {
        let source = r#"
type Customer = struct { id: int }

graph g : App {
  operator inspect {
    on in where data is Customer {
      out = data.id
    }
  }

  . -> inspect -> .
}
"#;
        let hir = check_clean(source);
        let handler = &hir.app.nodes[0].handlers[0];
        let guard = handler.guard.as_ref().expect("the compiled guard");
        let customer = DataType::Struct(
            StructType::new(vec![TypeField::new("id", DataType::INT)]).expect("one closed field"),
        );
        let expr::Expr::Is { target, .. } = &guard.root else {
            panic!("the narrowing guard");
        };
        assert!(matches!(
            target.as_ref(),
            expr::NarrowTarget::Type(target) if target.ty() == &customer
        ));
        let Statement::Emit { value, .. } = &handler.body[0] else {
            panic!("the guarded emission");
        };
        let EmitValue::Data(tree) = value else {
            panic!("the expression emission");
        };
        assert_eq!(tree.ty, DataType::INT);
        assert!(matches!(&tree.root, expr::Expr::Path(path) if path == &["data", "id"]));
    }

    #[test]
    fn the_removed_struct_spelling_is_gone_from_the_grammar() {
        // D-2026-08-09-1 removes `struct { … }` from value positions
        // cleanly — a braced record IS the struct. The old spelling
        // now falls outside the grammar and refuses; it never parses
        // as anything else.
        let source = "graph g : App {\n  operator o {\n    on in { out = struct { n = 1 } }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(
            !bag.diagnostics().is_empty(),
            "the removed form must refuse, not resolve"
        );
    }

    #[test]
    fn non_idempotent_effects_are_deferred_until_their_invariants_run() {
        // `compensable` demands an executable `on compensate` and
        // `irreversible` an idempotency key; neither companion exists
        // in this subset, so the words refuse instead of lying.
        for word in ["compensable", "irreversible"] {
            let source = format!(
                "graph g : App {{\n  connector c : Process {{\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = {word} ; behavior = deterministic\n  }}\n  . -> c -> .\n}}\n"
            );
            let (_, bag) = check_source(&source);
            assert!(
                bag.diagnostics().iter().any(|diagnostic| {
                    diagnostic.code() == E_CHECK_DEFERRED
                        && diagnostic.fields()[0].1 == "compensable and irreversible effects"
                }),
                "`effect = {word}` must refuse: {:#?}",
                bag.diagnostics()
            );
        }
    }

    #[test]
    fn an_unwired_output_warns_without_refusing() {
        let source = "graph g : App {\n  operator o {\n    on in {\n      out = .\n      side = .\n    }\n  }\n  . -> o -> .\n}\n";
        let (hir, bag) = check_source(source);
        assert!(hir.is_some(), "a leak is a warning, not a refusal");
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == W_UNWIRED_OUTPUT && diagnostic.fields()[1].1 == "side"
        }));
        // Discarding in writing silences it.
        let discarded = "graph g : App {\n  operator o {\n    on in {\n      out = .\n      side = .\n    }\n  }\n  . -> o -> .\n  o.side -> nothing\n}\n";
        let (_, bag) = check_source(discarded);
        assert!(
            !bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == W_UNWIRED_OUTPUT)
        );
    }

    #[test]
    fn a_second_handler_for_one_trigger_is_refused() {
        let source = "graph g : App {\n  operator o {\n    on in { out = . }\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_CHECK_DEFERRED
                && diagnostic.fields()[0].1 == "a second handler for one trigger"
        }));
    }

    #[test]
    fn lifecycle_handlers_this_machine_never_fires_are_refused() {
        let source = "graph g : App {\n  operator o {\n    on start { out = . }\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_CHECK_DEFERRED
                && diagnostic.fields()[0].1 == "a start lifecycle handler"
        }));
    }

    #[test]
    fn unrepresentable_emissions_refuse_at_check_not_at_runtime() {
        let source = "graph g : App {\n  operator o {\n    on in { out = duration(\"3s\") }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == E_CHECK_DEFERRED
                    && diagnostic.fields()[0].1 == "a duration, timestamp, or null emission"
            }),
            "an accepted plan must not fault on a normal emission: {:#?}",
            bag.diagnostics()
        );
    }

    #[test]
    fn the_jsonl_codec_compiles() {
        // M1 item 2 (D-2026-08-11-1): the JSON chain exists — the
        // deferral lifted with it.
        let source = "graph g : App {\n  connector c : Process {\n    mode = job ; command = \"cat\"\n    codec = jsonl\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(
            !bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED),
            "{:#?}",
            bag.diagnostics()
        );
    }

    #[test]
    fn a_misspelled_expectation_target_refuses() {
        // The exact false-green shapes of the fifth review: a typo'd
        // node port, a typo'd boundary port, a non-boolean predicate.
        let base = "graph g : App {\n  operator relay {\n    on in { out = . }\n  }\n  . -> relay -> .\n}\n\nlab g {\n  simulation s on app {\n    scenario sc {\n      app.in = { text = \"x\" }\n      EXPECT\n    }\n  }\n}\n";
        let cases = [
            ("expect nothing relay.typo", E_PORT_UNKNOWN),
            ("expect nothing app.typo", E_PORT_UNKNOWN),
            ("expect nothing app.out where 42", E_EXPR),
        ];
        for (expect, code) in cases {
            let source = base.replace("EXPECT", expect);
            let (_, bag) = check_source(&source);
            assert!(
                bag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code() == code),
                "`{expect}` must refuse with {code:?}: {:#?}",
                bag.diagnostics()
            );
        }
    }

    #[test]
    fn lab_fragments_merge_and_homonym_simulations_refuse() {
        let source = "graph g : App {\n  operator o {\n    on in { out = . }\n  }\n  . -> o -> .\n}\n\nlab g {\n  simulation a on app { }\n}\n\nlab g {\n  simulation b on app { }\n}\n\nlab g {\n  simulation a on app { }\n}\n";
        let (hir, bag) = check_source(source);
        // Fragments merged into one corpus of two simulations…
        let hir = hir.expect("the unit checks");
        assert_eq!(hir.labs.len(), 1);
        assert_eq!(hir.labs[0].simulations.len(), 2);
        // …and the homonym refused by name.
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == E_DUP_LAB_IDENTITY && diagnostic.fields()[0].1 == "a"
        }));
    }

    #[test]
    fn an_implicit_unwired_output_still_warns() {
        // `out` never enters the declared port table; the leak must
        // surface anyway — the errata's interim floor is visibility.
        let source = "graph g : App {\n  operator o {\n    on in { out = . }\n  }\n  . -> o\n}\n";
        let (_, bag) = check_source(source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == W_UNWIRED_OUTPUT && diagnostic.fields()[1].1 == "out"
            }),
            "{:#?}",
            bag.diagnostics()
        );
    }

    #[test]
    fn a_duplicated_edge_refuses_whatever_its_spelling() {
        // Textually identical, and sugar-vs-explicit: both resolve to
        // one endpoint pair and the second refuses with its own span.
        for second in ["o -> p", "o.out -> p.in"] {
            let source = format!(
                "graph g : App {{\n  operator o {{\n    on in {{ out = . }}\n  }}\n  operator p {{\n    on in {{ out = . }}\n  }}\n  . -> o -> p -> .\n  {second}\n}}\n"
            );
            let (_, bag) = check_source(&source);
            assert!(
                bag.diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code() == E_DUP_EDGE),
                "`{second}` must refuse: {:#?}",
                bag.diagnostics()
            );
        }
    }

    #[test]
    fn duplicate_mocks_refuse_at_both_levels() {
        let base = "graph g : App {\n  connector shell : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  operator o {\n    on in { out = data.text }\n  }\n  . -> o -> shell -> .\n}\n\nlab g {\n  simulation s on app {\nBODY  }\n}\n";
        let simulation_level = base.replace(
            "BODY",
            "    mock shell { on in { out = . } }\n    mock shell { on in { out = . } }\n",
        );
        let scenario_level = base.replace(
            "BODY",
            "    scenario x {\n      mock shell { on in { out = . } }\n      mock shell { on in { out = . } }\n      app.in = \"y\"\n    }\n",
        );
        for source in [simulation_level, scenario_level] {
            let (_, bag) = check_source(&source);
            assert!(
                bag.diagnostics().iter().any(|diagnostic| {
                    diagnostic.code() == E_DUP_LAB_IDENTITY && diagnostic.fields()[0].1 == "shell"
                }),
                "the duplicate mock must refuse: {:#?}",
                bag.diagnostics()
            );
        }
    }

    #[test]
    fn no_top_level_declaration_is_silently_ignored() {
        // Until unit composition and real liveness exist, an extra
        // top-level graph is an explicit deferral — an ERROR, so its
        // unchecked content can never ride along silently.
        let source = "graph main : App { }\ngraph orphan { }\nval loose = 1\n";
        let (_, bag) = check_source(source);
        assert!(bag.has_errors());
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert!(constructs.contains(&"a top-level graph outside the App"));
        assert!(constructs.contains(&"a top-level val"));
    }

    #[test]
    fn a_var_initializer_must_fit_its_declared_type() {
        let source = "graph g : App {\n  operator o {\n    var count : int = \"oops\"\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_EXPR)
            .expect("the type mismatch");
        assert!(diagnostic.message().contains("declared int32"));
    }

    #[test]
    fn shadowing_and_reserved_names_are_refused() {
        let source = "graph g : App {\n  val limit = 1\n  operator o {\n    var limit : int = 0\n    var data : int = 0\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let shadows: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_DUP_BINDING)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert_eq!(shadows, ["limit", "data"]);
    }

    #[test]
    fn handler_and_emission_ports_are_inferred() {
        // The reviewer's reproduction: `on event` implies an input,
        // `result = .` implies an output — no declaration
        // needed. This program is conforming and must check cleanly.
        let source = "graph g : App {\n  input feed\n  output sink\n\n  operator o {\n    on event {\n      result = .\n    }\n  }\n\n  .feed -> o.event\n  o.result -> .sink\n}\n";
        let hir = check_clean(source);
        let ports = &hir.app.nodes[0].ports;
        assert!(
            ports
                .iter()
                .any(|decl| decl.port.name() == "event" && decl.direction == Direction::Input)
        );
        assert!(
            ports
                .iter()
                .any(|decl| decl.port.name() == "result" && decl.direction == Direction::Output)
        );
    }

    #[test]
    fn conflicting_port_roles_are_refused() {
        // The same name inferred as input and output.
        let source = "graph g : App {\n  operator o {\n    on chan {\n      chan = .\n    }\n  }\n  . -> o.chan -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
        // Explicit double declaration at the boundary.
        let source = "graph g : App {\n  input feed\n  output feed\n  operator o { on in { out = . } }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
    }

    #[test]
    fn a_connector_never_declares_ports() {
        let source = "graph g : App {\n  connector c : Process {\n    input extra\n    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_PORT_CONFLICT)
            .expect("the connector port refusal");
        assert!(diagnostic.message().contains("its type defines them"));
    }

    #[test]
    fn physical_integer_types_are_not_silently_merged() {
        let long = "graph g : App {\n  operator o {\n    var n : long = 0\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (hir, bag) = check_source(long);
        assert!(
            !bag.has_errors(),
            "long is now canonical: {:#?}",
            bag.diagnostics()
        );
        assert_eq!(
            hir.expect("one App").app.nodes[0].vars[0].ty,
            DataType::LONG
        );
        let overflow = "graph g : App {\n  operator o {\n    var n : int = 2147483648\n    on in { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(overflow);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("does not fit `int`"))
        );
    }

    #[test]
    fn ports_are_used_with_their_direction() {
        // `o.in` as an edge source, a handler on `out`, and an
        // emission into `in` all refuse.
        let source =
            "graph g : App {\n  operator o {\n    on out { in = . }\n  }\n  o.in -> o\n}\n";
        let (_, bag) = check_source(source);
        let directions = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
            .count();
        assert_eq!(
            directions,
            2,
            "handler + edge source: {:#?}",
            bag.diagnostics()
        );
        let emission =
            "graph g : App {\n  operator o {\n    on in { in = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(emission);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_DIRECTION));
    }

    #[test]
    fn boundary_ports_carry_direction() {
        let source = "graph g : App {\n  input feed\n  operator o { on in { out = . } }\n  o.out -> .feed\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
            .expect("an input boundary port cannot drain the graph");
        assert!(diagnostic.message().contains("boundary edge target"));
    }

    #[test]
    fn wiring_cycles_are_refused() {
        let source = "graph g : App {\n  operator a { on in { out = . } }\n  operator b { on in { out = . } }\n  a -> b\n  b -> a\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_GRAPH_CYCLE)
            .expect("the cycle must refuse");
        assert!(diagnostic.message().contains("DAG"));
        let chain = "graph g : App {\n  operator a { on in { out = . } }\n  operator b { on in { out = . } }\n  . -> a -> b -> .\n}\n";
        let (_, bag) = check_source(chain);
        assert!(!diagnostic_codes(&bag).contains(&E_GRAPH_CYCLE));
    }

    #[test]
    fn repeated_attributes_and_port_declarations_are_refused() {
        let repeated_attr = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; command = \"false\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(repeated_attr);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR
                    && diagnostic.message().contains("twice"))
        );
        let repeated_port = "graph g : App {\n  operator o {\n    input event\n    input event\n    on event { out = . }\n  }\n  .feed -> o.event\n  input feed\n  o -> .\n}\n";
        let (_, bag) = check_source(repeated_port);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_PORT_CONFLICT
                    && diagnostic.message().contains("already declared"))
        );
    }

    #[test]
    fn guarded_constructs_defer_explicitly() {
        let source = "graph g : App {\n  partition user { key = meta.app.id }\n  operator o {\n    on (a, b) { out = . }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert!(constructs.contains(&"a partition"));
        assert!(constructs.contains(&"a join handler"));
    }
}
