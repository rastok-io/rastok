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
pub mod syntax;

pub use check::Unit;
pub use hir::Hir;
pub use syntax::SyntaxTree;

use crate::contract;
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId};
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
}

/// Compiles `.rkg` sources against a contract registry.
///
/// # Errors
///
/// Returns the full diagnostic bag when parsing or checking rejects
/// the unit; no plan exists in that case, by construction.
pub fn compile(sources: &[&str], registry: &contract::Registry) -> Result<Compilation, Bag> {
    compile_with_assets(sources, registry, None)
}

/// The index of the source DECLARING the `: App` graph, by the
/// parser's own reading — the CLI's asset-root anchor
/// (D-2026-08-10-5). `None` when no source declares one.
#[must_use]
pub fn app_source_index(sources: &[&str]) -> Option<usize> {
    use crate::compiler::syntax::TopDecl;
    let mut scratch = Bag::new();
    for (index, source) in sources.iter().enumerate() {
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let tree = parser::parse(source_id, source, &mut scratch);
        let declares_app = tree.decls.iter().any(|decl| {
            matches!(decl, TopDecl::Graph(graph) if graph
                .annotation
                .as_ref()
                .is_some_and(|annotation| annotation.symbols.iter().any(|symbol| symbol.text == "App")))
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
    registry: &contract::Registry,
    asset_root: Option<&std::path::Path>,
) -> Result<Compilation, Bag> {
    let mut bag = Bag::new();
    let mut unit = Unit::default();
    for (index, source) in sources.iter().enumerate() {
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let tree = parser::parse(source_id, source, &mut bag);
        unit.files.push((source_id, tree));
    }
    let hir = check::check(&unit, registry, &mut bag, asset_root);
    let Some(hir) = hir else {
        return Err(bag);
    };
    if bag.has_errors() {
        return Err(bag);
    }
    let hash = lower::semantic_hash(&hir);
    let (plan, symbols) = lower::lower(&hir);
    // A lowering that emits an invalid plan is a compiler bug — but a
    // bug with a controlled error path, not a vanished process: the
    // validator exists precisely to catch this disagreement.
    let plan = match ValidatedExecutionPlan::try_new(plan) {
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
            | PlanValidationError::ConflictingPortTypes { .. }
            | PlanValidationError::MismatchedEdgePins { .. }),
        ) => {
            bag.push(type_resolution_diagnostic(&error));
            return Err(bag);
        }
        Err(PlanValidationError::DataTypeNotInFormat {
            location,
            ty,
            format,
        }) => {
            bag.push(
                Diagnostic::new(
                    check::E_CHECK_DEFERRED,
                    Phase::Check,
                    format!(
                        "`{location}` resolves to {ty}, which this build cannot yet transport in plan-package v{format}"
                    ),
                )
                .with_field("construct", "plan-package v7 type transport"),
            );
            return Err(bag);
        }
        Err(PlanValidationError::ExpressionNotInFormat { form, format }) => {
            bag.push(
                Diagnostic::new(
                    check::E_CHECK_DEFERRED,
                    Phase::Check,
                    format!(
                        "the `{form}` expression is valid but plan-package v{format} cannot transport it"
                    ),
                )
                .with_field("construct", "plan-package v7 expression transport"),
            );
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
    })
}

fn type_resolution_diagnostic(error: &PlanValidationError) -> Diagnostic {
    match error {
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
}
