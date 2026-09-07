//! Source-to-plan compilation pipeline.
//!
//! Governing rule: [`grammar.md`] — the LL(1) reference grammar. The
//! implementation target it fixes is a hand-written recursive-descent
//! parser with one token of lookahead over a hand-written lexer; Rastok
//! expression spans are delimited here and handed to the expression chain
//! untouched.
//!
//! [`grammar.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/grammar.md

pub mod check;
pub mod hir;
pub mod lexer;
pub mod lower;
pub mod parser;
mod routing;
pub mod syntax;

pub use check::Unit;
pub use hir::Hir;
pub use syntax::SyntaxTree;

use crate::contract;
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId};
use crate::node;
use crate::plan::{PlanValidationError, SemanticHash, SymbolTable, ValidatedExecutionPlan};

/// The lowering emitted a plan the validator refuses — a compiler
/// defect surfaced through the ordinary error path, never a panic.
pub const E_INTERNAL_PLAN: Code = Code::new("E-INTERNAL-PLAN");

/// The program's plan would serialize deeper than the format
/// reader's budget — a source-level limit, not a compiler defect.
pub const E_PLAN_DEPTH: Code = Code::new("E-PLAN-DEPTH");

/// The result of a successful compilation: a valid plan, its semantic
/// identity, its symbol side table, and the warnings. A plan with
/// errors is unrepresentable — errors return the whole bag instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Compilation {
    /// The executable plan — validated: the lowering's claim that it
    /// produced an executable plan is checked, not assumed.
    pub plan: ValidatedExecutionPlan,
    /// Diagnostic and trace symbols for the plan.
    pub symbols: SymbolTable,
    /// `blake3(canonical HIR)`.
    pub hash: SemanticHash,
    /// Non-fatal diagnostics that accompanied the build.
    pub warnings: Vec<crate::diagnostics::Diagnostic>,
    /// The checked lab corpus — executable by the lab runner, never
    /// identity material.
    pub labs: Vec<hir::Lab>,
    /// Source-faithful Node and Graph definitions for diagnostics and hosted
    /// inspection. This side table never enters the plan or any identity
    /// recipe.
    pub source_definitions: Vec<SourceDefinition>,
}

/// One exact source declaration retained beside a successful compilation.
///
/// The validated plan remains the execution authority. This side table only
/// lets physical tools show the code which produced an owner, including its
/// authored attributes, handlers and guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDefinition {
    /// Root-relative owner path. The App Graph uses the empty root path.
    pub owner: crate::plan::OwnerPath,
    /// Structural declaration kind.
    pub kind: node::Kind,
    /// Source table index handed to [`compile`].
    pub source: SourceId,
    /// First one-based source line covered by the declaration.
    pub first_line: usize,
    /// Last one-based source line covered by the declaration.
    pub last_line: usize,
    /// Exact declaration text, including every handler and guard.
    pub code: String,
}

/// Compiles `.rkg` sources against a contract registry.
///
/// # Errors
///
/// Returns the full diagnostic bag when parsing or checking rejects
/// the unit; no plan exists in that case, by construction.
pub fn compile(sources: &[&str], registry: &contract::ContractCatalog) -> Result<Compilation, Bag> {
    compile_with_assets(sources, registry, None)
}

/// The index of the source DECLARING the `: App` graph, by the
/// parser's own reading — the CLI's asset-root anchor
/// (D-2026-08-10-5). `None` when no source declares one.
#[must_use]
pub fn app_source_index(sources: &[&str], registry: &contract::ContractCatalog) -> Option<usize> {
    use crate::compiler::syntax::TopDecl;
    let mut scratch = Bag::new();
    for (index, source) in sources.iter().enumerate() {
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let tree = parser::parse(source_id, source, registry, &mut scratch);
        let declares_app = tree.decls.iter().any(|decl| {
            matches!(decl, TopDecl::Graph(graph) if graph
                .annotation
                .as_ref()
                .is_some_and(|annotation| annotation
                    .symbols
                    .iter()
                    .any(|symbol| symbol.text == "App")))
        });
        if declares_app {
            return Some(index);
        }
    }
    None
}

/// [`compile`], with an asset root the `asset(…)` literal may read
/// from at build time (D-2026-08-10-5). `None` refuses every asset.
///
/// # Errors
/// As [`compile`].
pub fn compile_with_assets(
    sources: &[&str],
    registry: &contract::ContractCatalog,
    asset_root: Option<&std::path::Path>,
) -> Result<Compilation, Bag> {
    let mut bag = Bag::new();
    let mut unit = Unit::default();
    for (index, source) in sources.iter().enumerate() {
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let tree = parser::parse(source_id, source, registry, &mut bag);
        unit.files.push((source_id, tree));
    }
    let hir = check::check(&unit, registry, &mut bag, asset_root);
    let Some(hir) = hir else {
        return Err(bag);
    };
    if bag.has_errors() {
        return Err(bag);
    }
    let source_definitions = source_definitions(&unit, sources);
    let hash = lower::semantic_hash(&hir);
    let (plan, symbols) = match lower::lower(&hir) {
        Ok(lowered) => lowered,
        Err(error) => {
            bag.push(
                Diagnostic::new(
                    check::E_CHECK_DEFERRED,
                    Phase::Plan,
                    format!(
                        "`{}` is checked but cannot ship in the current plan format: {}",
                        error.owner, error.detail
                    ),
                )
                .with_field("construct", "categorical plan transport")
                .with_field("owner", error.owner),
            );
            return Err(bag);
        }
    };
    // A lowering that emits an invalid plan is a compiler bug — but a
    // bug with a controlled error path, not a vanished process: the
    // validator exists precisely to catch this disagreement.
    let plan = match ValidatedExecutionPlan::try_from_lowered(plan) {
        Ok(plan) => plan,
        // The one validation refusal a USER causes: a program deep
        // enough that its plan would outgrow the format reader's
        // nesting budget. Every other refusal is a compiler defect.
        Err(PlanValidationError::NestingBudgetExceeded { depth, budget }) => {
            bag.push(
                Diagnostic::new(
                    E_PLAN_DEPTH,
                    Phase::Plan,
                    format!(
                        "the program nests too deeply to ship: its plan reaches \
                         {depth} levels where the format reads at most {budget}"
                    ),
                )
                .with_field("depth", depth.to_string()),
            );
            return Err(bag);
        }
        Err(
            error @ (PlanValidationError::UnresolvedPortType { .. }
            | PlanValidationError::UnwiredOutput { .. }
            | PlanValidationError::ConflictingPortTypes { .. }
            | PlanValidationError::MismatchedEdgePins { .. }
            | PlanValidationError::InvalidInstanceKey { .. }
            | PlanValidationError::ConflictingInstanceKeyTypes { .. }
            | PlanValidationError::UnresolvedInstanceAdmission { .. }
            | PlanValidationError::MissingClosingPolicy { .. }
            | PlanValidationError::AmbiguousJoinMetadataRead { .. }
            | PlanValidationError::AmbiguousInboundCallCorrelation { .. }
            | PlanValidationError::InvalidAnnotationPropagation { .. }
            | PlanValidationError::InvalidLoopControl { .. }
            | PlanValidationError::UnsafeLoopEffect { .. }
            | PlanValidationError::WiringCycle { .. }),
        ) => {
            bag.push(type_resolution_diagnostic(&error));
            return Err(bag);
        }
        Err(error) => {
            bag.push(internal_plan_diagnostic(&error));
            return Err(bag);
        }
    };
    Ok(Compilation {
        plan,
        symbols,
        hash,
        warnings: bag.into_diagnostics(),
        labs: hir.labs,
        source_definitions,
    })
}

fn source_definitions(unit: &Unit, sources: &[&str]) -> Vec<SourceDefinition> {
    let mut definitions = Vec::new();
    for (_, tree) in &unit.files {
        let Some(app) = tree.decls.iter().find_map(|decl| match decl {
            syntax::TopDecl::Graph(graph)
                if graph.annotation.as_ref().is_some_and(|annotation| {
                    annotation
                        .symbols
                        .first()
                        .is_some_and(|symbol| symbol.text == "App")
                }) =>
            {
                Some(graph)
            }
            _ => None,
        }) else {
            continue;
        };
        collect_graph_definitions(
            app,
            crate::plan::OwnerPath::root(),
            sources,
            &mut definitions,
        );
    }
    definitions
}

fn collect_graph_definitions(
    graph: &syntax::GraphDecl,
    path: crate::plan::OwnerPath,
    sources: &[&str],
    definitions: &mut Vec<SourceDefinition>,
) {
    push_source_definition(
        definitions,
        path.clone(),
        node::Kind::Graph,
        graph.span,
        sources,
    );
    for item in &graph.body {
        let (name, kind, span) = match item {
            syntax::GraphItem::Operator(decl) => (&decl.name.text, node::Kind::Operator, decl.span),
            syntax::GraphItem::Connector(decl) => {
                (&decl.name.text, node::Kind::Connector, decl.span)
            }
            syntax::GraphItem::Graph(decl) => {
                let Some(child) = child_owner_path(&path, &decl.name.text) else {
                    continue;
                };
                collect_graph_definitions(decl, child, sources, definitions);
                continue;
            }
            syntax::GraphItem::Port(_)
            | syntax::GraphItem::RemovedPartition(_)
            | syntax::GraphItem::Val(_)
            | syntax::GraphItem::Var(_)
            | syntax::GraphItem::Edge(_)
            | syntax::GraphItem::Defaults(_)
            | syntax::GraphItem::Attribute(_)
            | syntax::GraphItem::Handler(_)
            | syntax::GraphItem::Error(_) => continue,
        };
        if let Some(child) = child_owner_path(&path, name) {
            push_source_definition(definitions, child, kind, span, sources);
        }
    }
}

fn child_owner_path(parent: &crate::plan::OwnerPath, name: &str) -> Option<crate::plan::OwnerPath> {
    let mut segments = parent.segments().to_vec();
    segments.push(name.to_owned());
    crate::plan::OwnerPath::new(segments).ok()
}

fn push_source_definition(
    definitions: &mut Vec<SourceDefinition>,
    owner: crate::plan::OwnerPath,
    kind: node::Kind,
    span: crate::diagnostics::Span,
    sources: &[&str],
) {
    let Some(source) = sources.get(span.source().index() as usize) else {
        return;
    };
    let range = span.range();
    let Some(code) = source.get(range.clone()) else {
        return;
    };
    let first_line = source.as_bytes()[..range.start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    let final_offset = range.end.saturating_sub(1).max(range.start);
    let last_line = source.as_bytes()[..final_offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    definitions.push(SourceDefinition {
        owner,
        kind,
        source: span.source(),
        first_line,
        last_line,
        code: code.to_owned(),
    });
}

fn type_resolution_diagnostic(error: &PlanValidationError) -> Diagnostic {
    match error {
        PlanValidationError::UnwiredOutput { node, port } => Diagnostic::new(
            check::E_UNWIRED_OUTPUT,
            Phase::Check,
            format!("`{node}.{port}` can be produced but has no destination; route it or write `{node}.{port} -> nothing`"),
        )
        .with_field("node", node.to_string())
        .with_field("port", port.name()),
        PlanValidationError::UnresolvedPortType { node, port } => Diagnostic::new(
            check::E_PORT_TYPE,
            Phase::Check,
            match node {
                Some(node) => format!(
                    "`{node}.{port}` has no effective payload type; add a type constraint"
                ),
                None => format!(
                    "the boundary port `{port}` has no effective payload type; add a type constraint"
                ),
            },
        )
        .with_field("port", port.name())
        .with_field("expected", "a resolved logical type")
        .with_field("found", "an unsolved inference variable"),
        PlanValidationError::ConflictingPortTypes {
            node,
            port,
            expected,
            found,
        } => Diagnostic::new(
            check::E_PORT_TYPE,
            Phase::Check,
            match node {
                Some(node) => format!(
                    "`{node}.{port}` requires {expected}, but a producer supplies {found}"
                ),
                None => format!(
                    "the boundary port `{port}` requires {expected}, but a producer supplies {found}"
                ),
            },
        )
        .with_field("port", port.name())
        .with_field("expected", expected.to_string())
        .with_field("found", found.to_string()),
        PlanValidationError::MismatchedEdgePins {
            port,
            expected,
            found,
        } => Diagnostic::new(
            check::E_PORT_TYPE,
            Phase::Check,
            format!(
                "`{port}` requires {expected}, but its producer supplies {found}"
            ),
        )
        .with_field("port", port.name())
        .with_field("expected", expected.to_string())
        .with_field("found", found.to_string()),
        PlanValidationError::InvalidInstanceKey { owner, ty } => Diagnostic::new(
            check::E_INSTANCE,
            Phase::Check,
            format!(
                "the instance key on `{owner}` resolves to {ty}; instance keys use canonical scalar/enum leaves or a closed struct of those leaves"
            ),
        )
        .with_field("name", owner)
        .with_field("detail", format!("non-canonical key type {ty}")),
        PlanValidationError::ConflictingInstanceKeyTypes {
            owner,
            expected,
            found,
        } => Diagnostic::new(
            check::E_INSTANCE,
            Phase::Check,
            format!(
                "the instance key on `{owner}` has no single type: it resolves to {expected} and {found} across its inputs"
            ),
        )
        .with_field("name", owner)
        .with_field(
            "detail",
            format!("conflicting key types {expected} and {found}"),
        ),
        PlanValidationError::UnresolvedInstanceAdmission { owner } => Diagnostic::new(
            check::E_INSTANCE,
            Phase::Check,
            format!(
                "the data-dependent instance key on `{owner}` has no resolved admission signature"
            ),
        )
        .with_field("name", owner)
        .with_field("detail", "missing resolved admission signature"),
        PlanValidationError::MissingClosingPolicy { connector, owner } => Diagnostic::new(
            check::E_INSTANCE,
            Phase::Check,
            format!(
                "closable owner `{owner}` contains outbound Connector `{connector}` and therefore requires `closing = {{ drain_for = <duration>; settle_for = <duration> }}`"
            ),
        )
        .with_field("name", owner)
        .with_field(
            "detail",
            format!("outbound Connector `{connector}` can remain open during closure"),
        ),
        PlanValidationError::AmbiguousJoinMetadataRead { owner, field } => Diagnostic::new(
            check::E_META_ZONE,
            Phase::Check,
            format!(
                "`{field}` may be absent on `{owner}` after a join; choose one named input with an explicit metadata write before reading it"
            ),
        )
        .with_field("field", field)
        .with_field("detail", "required metadata read after CommonOrAbsent"),
        PlanValidationError::AmbiguousInboundCallCorrelation { node } => Diagnostic::new(
            check::E_CONTRACT_ATTR,
            Phase::Check,
            format!(
                "`{node}` receives a response through a root-mixing join; add `correlate = <direct data or meta path>`"
            ),
        )
        .with_field("node", node.to_string())
        .with_field("attribute", "correlate"),
        PlanValidationError::InvalidAnnotationPropagation { location, reason } => {
            Diagnostic::new(
                check::E_DATA_ANNOTATION,
                Phase::Check,
                format!(
                    "data policy at {location} cannot be derived from every producer: {reason}"
                ),
            )
            .with_field("annotation", "propagation")
            .with_field("path", location)
            .with_field("detail", reason.to_string())
        }
        PlanValidationError::UnsafeLoopEffect { owner } => Diagnostic::new(
            check::E_LOOP_EFFECT,
            Phase::Check,
            format!("Loop `{owner}` can repeat a visible or ambiguous effect; use effect = none or idempotent"),
        )
        .with_field("owner", owner)
        .with_field("path", "application"),
        PlanValidationError::InvalidLoopControl {
            owner,
            path,
            detail,
        } => Diagnostic::new(
            check::E_LOOP_CONTROL,
            Phase::Check,
            format!("cannot prove that Loop `{owner}` always terminates at `{path}`: {detail}"),
        )
        .with_field("owner", owner)
        .with_field("path", path)
        .with_field("detail", *detail),
        PlanValidationError::WiringCycle { node } => Diagnostic::new(
            check::E_GRAPH_CYCLE,
            Phase::Check,
            format!("the wiring cycles through `{node}`; executable edges form a DAG"),
        )
        .with_field("node", node.to_string()),
        _ => internal_plan_diagnostic(error),
    }
}

/// The diagnostic for a lowering/validator disagreement. The payload
/// contract, stable per the registry: code `E-INTERNAL-PLAN`, phase
/// `plan`, field `detail` carrying the violated invariant.
fn internal_plan_diagnostic(error: &PlanValidationError) -> Diagnostic {
    Diagnostic::new(
        E_INTERNAL_PLAN,
        Phase::Plan,
        "the lowering emitted an invalid plan — a compiler bug, please report it",
    )
    .with_field("detail", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registered payload: code, phase, and the `detail` field —
    /// what tools and tests key on, per `diagnostics.md` rule 2.
    #[test]
    fn the_internal_plan_diagnostic_carries_its_registered_payload() {
        let error = PlanValidationError::NonCanonicalOrder {
            collection: "nodes",
        };
        let diagnostic = internal_plan_diagnostic(&error);
        assert_eq!(diagnostic.code(), E_INTERNAL_PLAN);
        assert_eq!(diagnostic.code().as_str(), "E-INTERNAL-PLAN");
        assert_eq!(diagnostic.phase(), Phase::Plan);
        assert_eq!(
            diagnostic.fields(),
            [(
                "detail",
                String::from("invalid plan: `nodes` is not in canonical order"),
            )]
        );
    }

    #[test]
    fn successful_compilation_retains_source_faithful_owner_definitions() {
        let source = r#"
graph inspected : App {
  input in: text
  output out: text
  operator relay {
    on in where size(data) > 0 { out = . }
  }

  graph nested {
    input in: text
    output out: text
    operator inner {
      on in { out = . }
    }
    . -> inner -> .
  }

  . -> relay -> nested -> .
}
"#;
        let compilation = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the inspection fixture compiles");

        assert_eq!(compilation.source_definitions.len(), 4);
        let app = &compilation.source_definitions[0];
        assert!(app.owner.is_root());
        assert_eq!(app.kind, node::Kind::Graph);
        assert_eq!(app.first_line, 2);
        assert_eq!(app.last_line, 19);
        assert!(app.code.starts_with("graph inspected : App"));

        let relay = &compilation.source_definitions[1];
        assert_eq!(relay.owner.to_string(), "relay");
        assert_eq!(relay.kind, node::Kind::Operator);
        assert_eq!(relay.first_line, 5);
        assert_eq!(relay.last_line, 7);
        assert_eq!(
            relay.code,
            "operator relay {\n    on in where size(data) > 0 { out = . }\n  }"
        );

        let nested = &compilation.source_definitions[2];
        assert_eq!(nested.owner.to_string(), "nested");
        assert_eq!(nested.kind, node::Kind::Graph);
        let inner = &compilation.source_definitions[3];
        assert_eq!(inner.owner.to_string(), "nested.inner");
        assert_eq!(inner.kind, node::Kind::Operator);
        assert!(inner.code.contains("on in { out = . }"));
    }

    #[test]
    fn an_instance_key_uses_the_graphs_resolved_input_type() {
        let source = r#"
graph inferred_instance : App {
  input in: int
  output out: int
  operator relay {
    instance = { key = data }
    on in { out = . }
  }
  . -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the graph supplies the omitted node-port type");
        assert_eq!(
            compilation
                .plan
                .node_instance(crate::plan::Slot::new(0))
                .expect("the node has an owner")
                .key
                .as_ref()
                .expect("the node is keyed")
                .ty,
            crate::datatype::DataType::INT
        );
    }

    #[test]
    fn an_idle_process_without_explicit_closing_refuses_at_check() {
        let source = r#"
graph g : App {
  input in: text
  connector shell : Process {
    instance = { key = data }
    lifetime = { kind = idle ; after = duration("1s") }
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }
  . -> shell -> nothing
  shell.err -> nothing
  shell.exit -> nothing
}
"#;
        let bag = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("idle cannot invent a drain or cancellation policy");
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == check::E_INSTANCE
                && diagnostic.message().contains("requires `closing")
        }));
    }

    #[test]
    fn an_inferred_composite_key_closes_after_port_resolution() {
        let source = r#"
graph inferred_composite : App {
  input in: struct { tenant: text, order: long }
  output out: struct { tenant: text, order: long }
  operator relay {
    instance = { key = { payload = data } }
    on in { out = . }
  }
  . -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the resolved input closes every composite-key leaf");
        let key = compilation
            .plan
            .node_instance(crate::plan::Slot::new(0))
            .expect("the node has an owner")
            .key
            .as_ref()
            .expect("the node is keyed");
        assert!(key.ty.is_instance_key());
        assert!(matches!(key.ty, crate::datatype::DataType::Struct(_)));
    }

    #[test]
    fn graph_routing_and_finite_lifetimes_share_the_r4_plan() {
        let keyed_graph = r#"
graph keyed_graph : App {
  input in: text
  instance = { key = data }
  operator relay { on in { out = . } }
  . -> relay -> nothing
}
"#;
        compile(
            &[keyed_graph],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("R3 routes through the Graph-owned key before producer commit");

        let finite = r#"
graph finite_singleton : App {
  input in: text
  operator relay {
    lifetime = { kind = fixed ; after = duration("30s") }
    on in { out = . }
  }
  . -> relay -> nothing
}
"#;
        let compilation = compile(
            &[finite],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("R4 executes a fixed owner lifetime");
        assert_eq!(
            compilation
                .plan
                .node_instance(crate::plan::Slot::new(0))
                .expect("relay has one owner")
                .lifetime,
            crate::plan::InstanceLifetime::Fixed(30_000_000_000)
        );
    }

    #[test]
    fn a_resolved_variant_cannot_become_an_instance_key() {
        let source = r#"
graph dynamic_instance : App {
  input in: variant
  output out: variant
  operator relay {
    instance = { key = data }
    on in { out = . }
  }
  . -> relay -> .
}
"#;
        let bag = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("a resolved dynamic value has no canonical instance identity");
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == check::E_INSTANCE
                && diagnostic
                    .message()
                    .contains("canonical scalar/enum leaves")
        }));
    }

    #[test]
    fn one_instance_expression_requires_one_type_across_input_ports() {
        let source = r#"
graph conflicting_instance : App {
  input alpha: int
  input beta: text
  output out: int
  operator relay {
    instance = { key = data }
    input a: int
    input b: text
    output out: int
    on a { out = 1 }
    on b { out = 1 }
  }
  .alpha -> relay.a
  .beta -> relay.b
  relay -> .
}
"#;
        let bag = compile(
            &[source],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("one key expression cannot change logical type by input port");
        assert!(bag.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == check::E_INSTANCE
                && diagnostic.message().contains("has no single type")
        }));
    }

    #[test]
    fn loop_fuel_reenters_without_a_spatial_cycle_and_expires_exactly() {
        use crate::boundary::{
            Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence,
        };
        use crate::envelope::{Data, Namespace};
        use crate::journal::{MemJournal, Record, loop_protocol::Fact};
        use crate::runtime::vm::Vm;
        use crate::value::Value;

        fn run(witness: &str) -> (Vm<'static>, MemJournal) {
            let source = format!(
                r#"
graph app : App {{
  input in: int
  output out: int
  output expired: union {{ LoopExpired {{ last_state: int }} }}
  operator countdown : Loop {{
    input in: int
    output out: int
    {witness}
    on in match {{
      data <= 0 => out = Loop.state.data
      _ => Loop.next = data - 1
    }}
  }}
  in -> countdown -> out
  countdown.Loop.expired -> expired
}}
"#
            );
            let compilation = compile(
                &[&source],
                &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
            )
            .expect("the finite countdown is a valid Loop");
            let plan = Box::leak(Box::new(compilation.plan));
            let mut journal = MemJournal::new(crate::plan::GraphVersion(1));
            let mut vm = Vm::boot(plan, crate::plan::GraphVersion(1));
            vm.inject(
                &mut journal,
                InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        crate::node::Path::new(["loop-test"])
                            .expect("the provenance path is valid"),
                    ),
                    port: crate::node::Port::new("in").expect("the input port is valid"),
                    draft: Draft {
                        data: Data::materialized(Value::Int(3)),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
            )
            .expect("the root is admitted before Loop work");
            vm.drain(&mut journal)
                .expect("the immediate Loop reaches one terminal fact");
            (vm, journal)
        }

        let (completed, completed_journal) = run("Loop.fuel = 4");
        assert_eq!(completed.outputs().len(), 1);
        assert_eq!(completed.outputs()[0].port.name(), "out");
        assert_eq!(
            crate::runtime::data_value(completed.outputs()[0].envelope.data())
                .expect("the result is materialized"),
            Value::Int(0)
        );
        let completed_facts: Vec<_> = completed_journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(transaction) => transaction.loop_protocol(),
                _ => None,
            })
            .flat_map(|packet| &packet.facts)
            .collect();
        assert_eq!(
            completed_facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Advance { .. }))
                .count(),
            3
        );
        assert_eq!(
            completed_facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Admit { iteration: 1.., .. }))
                .count(),
            3,
            "every selected state crosses the temporal boundary exactly once"
        );
        assert!(matches!(completed_facts.first(), Some(Fact::Open { .. })));
        assert!(matches!(completed_facts.last(), Some(Fact::Close { .. })));

        let (expired, expired_journal) = run("Loop.fuel = 2");
        assert_eq!(expired.outputs().len(), 1);
        assert_eq!(expired.outputs()[0].port.name(), "expired");
        let Value::TaggedUnion(expiry) =
            crate::runtime::data_value(expired.outputs()[0].envelope.data())
                .expect("LoopExpired is materialized")
        else {
            panic!("expiry is a tagged union");
        };
        assert_eq!(expiry.case, "LoopExpired");
        assert_eq!(
            expiry.payload,
            vec![(String::from("last_state"), Value::Int(1))]
        );
        assert!(expired_journal.records().iter().any(|record| matches!(
            record,
            Record::Transaction(transaction)
                if transaction.loop_protocol().is_some_and(|packet|
                    packet.facts.iter().any(|fact| matches!(fact, Fact::Expired { .. })))
        )));

        let (proved, proved_journal) = run("Loop.measure = Loop.state.data");
        assert_eq!(proved.outputs().len(), 1);
        assert_eq!(proved.outputs()[0].port.name(), "out");
        assert_eq!(
            crate::runtime::data_value(proved.outputs()[0].envelope.data())
                .expect("the proved result is materialized"),
            Value::Int(0)
        );
        assert_eq!(
            proved_journal
                .records()
                .iter()
                .filter_map(|record| match record {
                    Record::Transaction(transaction) => transaction.loop_protocol(),
                    _ => None,
                })
                .flat_map(|packet| &packet.facts)
                .filter(|fact| matches!(fact, Fact::Admit { iteration: 1.., .. }))
                .count(),
            3,
            "measure 3 derives exactly four body applications, three of them recurrent"
        );

        let (capped, capped_journal) = run("Loop.fuel = 2\n  Loop.measure = Loop.state.data");
        assert_eq!(
            capped.outputs()[0].port.name(),
            "expired",
            "explicit fuel remains the operational cap when a proof also exists"
        );
        assert!(capped_journal.records().iter().any(|record| matches!(
            record,
            Record::Transaction(transaction)
                if transaction.loop_protocol().is_some_and(|packet|
                    packet.facts.iter().any(|fact| matches!(fact, Fact::Expired { .. })))
        )));
    }

    #[test]
    fn an_open_loop_resumes_from_the_journal_without_regaining_fuel() {
        use crate::boundary::{
            Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence,
        };
        use crate::envelope::{Data, Namespace};
        use crate::journal::{MemJournal, Record, loop_protocol::Fact};
        use crate::runtime::vm::Vm;
        use crate::value::Value;

        let compilation = compile(
            &[r#"
graph app : App {
  input in: int
  output out: int
  operator countdown : Loop {
    input in: int
    output out: int
    instance = { key = data }
    Loop.fuel = 4
    on in match {
      data <= 0 => out = Loop.state.data
      _ => Loop.next = data - 1
    }
  }
  in -> countdown -> out
  countdown.Loop.expired -> nothing
}
"#],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the finite countdown is a valid Loop");
        let plan = Box::leak(Box::new(compilation.plan));
        let mut journal = MemJournal::new(crate::plan::GraphVersion(1));
        let mut before_crash = Vm::boot(plan, crate::plan::GraphVersion(1));
        before_crash
            .inject(
                &mut journal,
                InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        crate::node::Path::new(["loop-recovery"])
                            .expect("the provenance path is valid"),
                    ),
                    port: crate::node::Port::new("in").expect("the input port is valid"),
                    draft: Draft {
                        data: Data::materialized(Value::Int(3)),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
            )
            .expect("the root is admitted");
        while !journal.records().iter().any(|record| {
            matches!(record,
            Record::Transaction(transaction) if transaction.loop_protocol()
                .is_some_and(|packet| !packet.produced.is_empty()))
        }) {
            assert!(
                before_crash
                    .step(&mut journal)
                    .expect("the first command commits")
            );
        }
        drop(before_crash);

        let mut recovered = Vm::boot(plan, crate::plan::GraphVersion(1));
        recovered
            .recover_from_journal(&journal)
            .expect("the open run is reconstructed from journal facts");
        recovered
            .drain(&mut journal)
            .expect("the recovered run reaches its original terminal");
        assert_eq!(recovered.outputs().len(), 1);
        assert_eq!(
            crate::runtime::data_value(recovered.outputs()[0].envelope.data())
                .expect("the terminal payload is materialized"),
            Value::Int(0)
        );
        let facts: Vec<_> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(transaction) => transaction.loop_protocol(),
                _ => None,
            })
            .flat_map(|packet| &packet.facts)
            .collect();
        assert_eq!(
            facts
                .iter()
                .filter(|fact| matches!(fact, Fact::Open { .. }))
                .count(),
            1
        );
        let ordinals: Vec<_> = facts
            .iter()
            .filter_map(|fact| match fact {
                Fact::Advance { to_iteration, .. } => Some(*to_iteration),
                _ => None,
            })
            .collect();
        assert_eq!(ordinals, [1, 2, 3]);
        assert!(matches!(
            facts.last(),
            Some(Fact::Close { iteration: 3, .. })
        ));
    }

    #[test]
    fn two_roots_keep_independent_loop_runs_in_one_instance() {
        use crate::boundary::{
            Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence,
        };
        use crate::envelope::{Data, Namespace};
        use crate::journal::{
            MemJournal, Record,
            loop_protocol::{Fact, Fold, Phase},
        };
        use crate::runtime::vm::Vm;
        use crate::value::Value;

        let compilation = compile(
            &[r#"
graph app : App {
  input in: int
  output out: int
  operator countdown : Loop {
    input in: int
    output out: int
    Loop.fuel = 3
    on in match {
      data <= 0 => out = Loop.state.data
      _ => Loop.next = data - 1
    }
  }
  in -> countdown -> out
  countdown.Loop.expired -> nothing
}
"#],
            &crate::contract::ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the finite countdown is a valid Loop");
        let plan = Box::leak(Box::new(compilation.plan));
        let mut journal = MemJournal::new(crate::plan::GraphVersion(1));
        let mut vm = Vm::boot(plan, crate::plan::GraphVersion(1));
        for value in [2, 1] {
            vm.inject(
                &mut journal,
                InboundSubmission {
                    provenance: InboundEnvelopeProvenance::single(
                        crate::node::Path::new(["concurrent-loops"])
                            .expect("the provenance path is valid"),
                    ),
                    port: crate::node::Port::new("in").expect("the input port is valid"),
                    draft: Draft {
                        data: Data::materialized(Value::Int(value)),
                        ext: Namespace::new(),
                    },
                    sequence: SubmissionSequence::Next,
                },
            )
            .expect("both roots fit the host mailbox");
        }
        vm.drain(&mut journal)
            .expect("both runs terminate independently");
        assert_eq!(vm.outputs().len(), 2);
        assert!(vm.outputs().iter().all(|output| {
            crate::runtime::data_value(output.envelope.data()) == Ok(Value::Int(0))
        }));
        let opens: Vec<_> = journal
            .records()
            .iter()
            .filter_map(|record| match record {
                Record::Transaction(transaction) => transaction.loop_protocol(),
                _ => None,
            })
            .flat_map(|packet| &packet.facts)
            .filter_map(|fact| match fact {
                Fact::Open { run, .. } => Some(*run),
                _ => None,
            })
            .collect();
        assert_eq!(opens.len(), 2);
        assert_ne!(opens[0], opens[1]);
        let fold = Fold::from_records(journal.records()).expect("both folds are valid");
        assert_eq!(fold.runs().count(), 2);
        for run in opens {
            assert_eq!(
                fold.run(run)
                    .expect("the terminal fold remains queryable")
                    .phase,
                Phase::Closed
            );
        }
    }
}
