//! Independent local proof of the recursive Operator decision algebra.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use core::fmt;

use super::{
    CasePartitionError, Dispatch, DispatchStep, EmitValue, LoopCommand, MAX_DISPATCH_DEPTH,
    MultiEmitValues, validate_case_partition,
};
use crate::datatype::DataType;
use crate::expr::{self, Expr, Signature, Tree};
use crate::schema_annotation::AnnotatedSchema;

#[cfg(test)]
mod tests;

/// A rejected fact in a checked decision tree. No source compiler is needed
/// to construct or verify this error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchErrorKind {
    /// More nested decisions than the shared resource bound permits.
    Depth,
    /// A match prefix contains something other than a pure local binding.
    ImpurePrefix,
    /// A local binding is empty, qualified, reserved or shadows a visible name.
    Binding,
    /// An expression is malformed, too deep, or contradicts its claimed type.
    Expression,
    /// A predicate has a statically non-boolean type.
    Predicate,
    /// A closed match contradicts its source domain or canonical order.
    CasePartition(CasePartitionError),
    /// Policy annotations are incompatible with the local binding's type.
    Annotations,
    /// Multi-emission needs at least two distinct targets and matching arity.
    MultiEmission,
}

/// An exact, deterministic location in the rejected decision tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchValidationError {
    /// Structural path, independent of source spans or compiler diagnostics.
    pub path: String,
    /// The violated invariant.
    pub kind: DispatchErrorKind,
}

impl DispatchValidationError {
    fn at(path: &str, kind: DispatchErrorKind) -> Self {
        Self {
            path: path.to_string(),
            kind,
        }
    }
}

impl fmt::Display for DispatchValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.path)?;
        match &self.kind {
            DispatchErrorKind::Depth => {
                write!(f, "Dispatch exceeds {MAX_DISPATCH_DEPTH} nested decisions")
            }
            DispatchErrorKind::ImpurePrefix => {
                f.write_str("only pure local bindings may precede a match")
            }
            DispatchErrorKind::Binding => {
                f.write_str("local binding is invalid or shadows a visible name")
            }
            DispatchErrorKind::Expression => {
                f.write_str("expression or claimed type cannot be re-derived")
            }
            DispatchErrorKind::Predicate => f.write_str("match predicate is not boolean"),
            DispatchErrorKind::CasePartition(error) => fmt::Display::fmt(error, f),
            DispatchErrorKind::Annotations => {
                f.write_str("annotations contradict the local binding type")
            }
            DispatchErrorKind::MultiEmission => {
                f.write_str("multi-emission needs distinct targets and matching arity")
            }
        }
    }
}

impl core::error::Error for DispatchValidationError {}

/// Re-derives local expressions, scope and decision structure against an
/// explicit environment, without trusting the source checker's arm/type claims.
///
/// This is one obligation of plan validation, not a validated execution plan.
/// The caller must independently establish the environment, resolve any
/// temporary `variant` inference bindings, and validate activation, output and
/// state targets, metadata writes, assets, annotations propagation and Loop
/// control/effects.
///
/// Predicates preserve the existing expression contract: bool or a temporary
/// `variant` which the surrounding port-resolution pass must constrain. The
/// verifier never fabricates a static bool proof for that placeholder.
///
/// # Errors
///
/// Returns the first violated invariant with its exact structural path. Closed
/// arms must already be canonical and exhaustive; no table is repaired here.
pub fn validate_dispatch(
    dispatch: &Dispatch,
    signature: &Signature,
) -> Result<(), DispatchValidationError> {
    validate_at(dispatch, signature, 0, "dispatch")
}

fn validate_at(
    dispatch: &Dispatch,
    signature: &Signature,
    depth: usize,
    path: &str,
) -> Result<(), DispatchValidationError> {
    if depth >= MAX_DISPATCH_DEPTH && !matches!(dispatch, Dispatch::Linear(_)) {
        return Err(DispatchValidationError::at(path, DispatchErrorKind::Depth));
    }
    let mut scope = signature.clone();
    match dispatch {
        Dispatch::Linear(steps) => steps_at(steps, &mut scope, false, &format!("{path}.linear"))?,
        Dispatch::ClosedMatch {
            prefix,
            scrutinee,
            arms,
        } => {
            steps_at(prefix, &mut scope, true, &format!("{path}.prefix"))?;
            decision_at(scrutinee, &scope, &format!("{path}.scrutinee"))?;
            validate_case_partition(&scrutinee.ty, arms.iter().map(|arm| arm.cases.as_slice()))
                .map_err(|error| {
                    DispatchValidationError::at(path, DispatchErrorKind::CasePartition(error))
                })?;
            for (index, arm) in arms.iter().enumerate() {
                let branch_path = format!("{path}.arms[{index}]");
                let mut branch = scope.clone();
                // Selection narrows the closed value without unwrapping it.
                if matches!(&scrutinee.root, Expr::Path(segments)
                    if segments.len() == 1 && segments[0] == "data")
                    && let Some(narrowed) =
                        super::routing::selected_case_type(&scrutinee.ty, &arm.cases)
                {
                    branch = branch.with("data", narrowed);
                }
                validate_at(&arm.dispatch, &branch, depth + 1, &branch_path)?;
            }
        }
        Dispatch::OpenMatch {
            prefix,
            arms,
            fallback,
        } => {
            steps_at(prefix, &mut scope, true, &format!("{path}.prefix"))?;
            for (index, arm) in arms.iter().enumerate() {
                let branch_path = format!("{path}.arms[{index}]");
                let predicate_path = format!("{branch_path}.predicate");
                decision_at(&arm.predicate, &scope, &predicate_path)?;
                if !matches!(arm.predicate.ty, DataType::Bool | DataType::Variant) {
                    return Err(DispatchValidationError::at(
                        &predicate_path,
                        DispatchErrorKind::Predicate,
                    ));
                }
                let branch = scope.narrowed_by(&arm.predicate.root);
                validate_at(&arm.dispatch, &branch, depth + 1, &branch_path)?;
            }
            // No positive facts or local values from another arm may leak
            // into the next predicate or the fallback.
            validate_at(fallback, &scope, depth + 1, &format!("{path}.fallback"))?;
        }
    }
    Ok(())
}

fn steps_at(
    steps: &[DispatchStep],
    scope: &mut Signature,
    prefix: bool,
    path: &str,
) -> Result<(), DispatchValidationError> {
    for (index, step) in steps.iter().enumerate() {
        let path = format!("{path}[{index}]");
        if prefix && !matches!(step, DispatchStep::Let { .. }) {
            return Err(DispatchValidationError::at(
                &path,
                DispatchErrorKind::ImpurePrefix,
            ));
        }
        match step {
            DispatchStep::Let {
                name,
                value,
                annotations,
            } => {
                if name.is_empty()
                    || name.contains('.')
                    || matches!(name.as_str(), "data" | "meta")
                    || scope.get(name).is_some()
                {
                    return Err(DispatchValidationError::at(
                        &path,
                        DispatchErrorKind::Binding,
                    ));
                }
                tree_at(value, scope, &format!("{path}.value"))?;
                if !annotations.is_empty() {
                    AnnotatedSchema::new(value.ty.clone(), annotations.clone()).map_err(|_| {
                        DispatchValidationError::at(&path, DispatchErrorKind::Annotations)
                    })?;
                }
                *scope = scope.clone().with(name, value.ty.clone());
            }
            DispatchStep::SetVar { value, .. } => tree_at(value, scope, &format!("{path}.value"))?,
            DispatchStep::Emit { value, .. }
            | DispatchStep::LoopCommand(LoopCommand::Next(value)) => {
                emission_at(value, scope, &format!("{path}.value"))?;
            }
            DispatchStep::MultiEmit { ports, values } => {
                if ports.len() < 2
                    || ports.iter().collect::<BTreeSet<_>>().len() != ports.len()
                    || matches!(values, MultiEmitValues::Tuple(values) if values.len() != ports.len())
                {
                    return Err(DispatchValidationError::at(
                        &path,
                        DispatchErrorKind::MultiEmission,
                    ));
                }
                match values {
                    MultiEmitValues::Broadcast(value) => {
                        emission_at(value, scope, &format!("{path}.broadcast"))?
                    }
                    MultiEmitValues::Tuple(values) => {
                        for (index, value) in values.iter().enumerate() {
                            emission_at(value, scope, &format!("{path}.values[{index}]"))?;
                        }
                    }
                }
            }
            DispatchStep::Stop
            | DispatchStep::LoopCommand(LoopCommand::Again | LoopCommand::Done) => {}
        }
    }
    Ok(())
}

fn emission_at(
    value: &EmitValue,
    signature: &Signature,
    path: &str,
) -> Result<(), DispatchValidationError> {
    match value {
        EmitValue::Identity | EmitValue::Nothing => Ok(()),
        EmitValue::Data(tree) => tree_at(tree, signature, path),
        EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
            for (index, write) in writes.iter().enumerate() {
                tree_at(&write.value, signature, &format!("{path}.writes[{index}]"))?;
            }
            Ok(())
        }
    }
}

fn decision_at(
    tree: &Tree,
    signature: &Signature,
    path: &str,
) -> Result<(), DispatchValidationError> {
    tree_at(tree, signature, path)?;
    // Decisions have no contextual construction target. In particular, an int
    // literal relabelled `variant` must not pass as a dynamic boolean guard.
    if expr::rederive_type(&tree.root, signature).as_ref() != Some(&tree.ty) {
        return Err(DispatchValidationError::at(
            path,
            DispatchErrorKind::Expression,
        ));
    }
    Ok(())
}

fn tree_at(tree: &Tree, signature: &Signature, path: &str) -> Result<(), DispatchValidationError> {
    // Check the iterative depth bound before either recursive expression walk.
    if expr::structural_depth(&tree.root) > crate::canonical_cbor::MAX_NESTING
        || tree.ty.check_bounds().is_err()
        || super::validation::expr_canonical(&tree.root).is_err()
        || !expression_has_type(&tree.root, &tree.ty, signature, false)
    {
        return Err(DispatchValidationError::at(
            path,
            DispatchErrorKind::Expression,
        ));
    }
    Ok(())
}

fn expression_has_type(
    expression: &Expr,
    ty: &DataType,
    signature: &Signature,
    field: bool,
) -> bool {
    // Contextual constructors carry their destination shape; a narrower field
    // (e.g. an int literal in a long field) is legal. Paths and decision results
    // cannot invent a different type merely by changing Tree::ty.
    match (expression, ty) {
        (Expr::Literal(value), _) => {
            ty.accepts(value)
                || (field
                    && expr::rederive_type(expression, signature)
                        .is_some_and(|found| found.is_assignable_to(ty)))
        }
        (Expr::List(items), DataType::Array(array)) => {
            array
                .length()
                .is_none_or(|length| length.get() as usize == items.len())
                && items
                    .iter()
                    .all(|item| expression_has_type(item, array.element(), signature, true))
        }
        (Expr::MapLit(entries), DataType::Map(map)) if map.key() == &DataType::Text => entries
            .iter()
            .all(|(_, value)| expression_has_type(value, map.value(), signature, true)),
        (Expr::Struct(fields), DataType::Struct(structure)) => {
            fields.len() == structure.fields().len()
                && fields
                    .iter()
                    .zip(structure.fields())
                    .all(|((name, value), expected)| {
                        name == expected.name()
                            && expression_has_type(value, expected.ty(), signature, true)
                    })
        }
        (
            Expr::Tagged {
                union,
                case,
                fields,
            },
            DataType::TaggedUnion(expected),
        ) if union.as_ref() == expected => expected
            .cases()
            .iter()
            .find(|candidate| candidate.name() == case)
            .is_some_and(|case| {
                fields.len() == case.payload().fields().len()
                    && fields.iter().zip(case.payload().fields()).all(
                        |((name, value), expected)| {
                            name == expected.name()
                                && expression_has_type(value, expected.ty(), signature, true)
                        },
                    )
            }),
        _ => expr::rederive_type(expression, signature)
            .is_some_and(|found| found == *ty || (field && found.is_assignable_to(ty))),
    }
}
