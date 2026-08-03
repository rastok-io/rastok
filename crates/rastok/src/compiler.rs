//! Source-to-plan compilation pipeline.
//!
//! Governing rule: [`grammar.md`] — the LL(1) reference grammar. The
//! implementation target it fixes is a hand-written recursive-descent
//! parser with one token of lookahead over a hand-written lexer; CEL
//! expression spans are delimited here and handed to the CEL chain
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

use crate::diagnostics::{Bag, SourceId};
use crate::plan::{ExecutionPlan, SemanticHash, SymbolTable};
use crate::stdlib;

/// The result of a successful compilation: a valid plan, its semantic
/// identity, its symbol side table, and the warnings. A plan with
/// errors is unrepresentable — errors return the whole bag instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Compilation {
    /// The executable plan.
    pub plan: ExecutionPlan,
    /// Diagnostic and trace symbols for the plan.
    pub symbols: SymbolTable,
    /// `blake3(canonical HIR)`.
    pub hash: SemanticHash,
    /// Non-fatal diagnostics that accompanied the build.
    pub warnings: Vec<crate::diagnostics::Diagnostic>,
}

/// Compiles `.rkg` sources against a contract registry.
///
/// # Errors
///
/// Returns the full diagnostic bag when parsing or checking rejects
/// the unit; no plan exists in that case, by construction.
pub fn compile(sources: &[&str], registry: &stdlib::Registry) -> Result<Compilation, Bag> {
    let mut bag = Bag::new();
    let mut unit = Unit::default();
    for (index, source) in sources.iter().enumerate() {
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let tree = parser::parse(source_id, source, &mut bag);
        unit.files.push((source_id, tree));
    }
    let hir = check::check(&unit, registry, &mut bag);
    let Some(hir) = hir else {
        return Err(bag);
    };
    if bag.has_errors() {
        return Err(bag);
    }
    let hash = lower::semantic_hash(&hir);
    let (plan, symbols) = lower::lower(&hir, registry);
    Ok(Compilation {
        plan,
        symbols,
        hash,
        warnings: bag.into_diagnostics(),
    })
}
