//! Small, deterministic certificates for finite `Loop` recursion.
//!
//! This module is deliberately narrower than the expression evaluator. A
//! proof is accepted only when every recurrent Dispatch leaf exposes one direct
//! literal decrease that this `no_std` verifier can re-derive from the shipped
//! handler table. Unsupported mathematics is a refusal, never an assumption.

use crate::envelope::WriteTarget;
use crate::expr::{self, BinaryOp, Expr};
use crate::plan::{Dispatch, DispatchStep, EmitValue, LoopCommand};
use crate::value::Value;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

mod graph;
pub use graph::derive_graph;

/// One canonical termination certificate.
///
/// `decreases` is the sorted multiset of literal strict decrements on every
/// recurrent Dispatch leaf. Sorting removes decision-table order but retains
/// repeated decrements. Validation re-derives the exact multiset; the shipped
/// list is evidence, never authority.
#[derive(Debug, Clone, PartialEq)]
pub struct Certificate {
    /// Integer-valued ranking expression over the iteration input.
    pub measure: expr::Tree,
    /// Optional invariant. The first gate accepts only the independently
    /// provable `true`; richer invariant certificates extend this field.
    pub invariant: Option<expr::Tree>,
    /// Sorted multiset of exact positive decreases on recurrent handlers.
    pub decreases: Vec<u64>,
}

/// Why a purported certificate is not a proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The measure is outside the first closed ranking fragment.
    UnsupportedMeasure,
    /// The invariant is not independently established by this gate.
    UnsupportedInvariant,
    /// A recurrent handler is not the direct input traversal proved here.
    UnsupportedActivation,
    /// One handler selects recurrence more than once.
    AmbiguousRecurrence,
    /// A recurrent path has no guard proving that the measure is positive.
    MeasureNotPositive,
    /// A recurrent path does not update the measured value by a known strict
    /// decrease.
    MeasureDoesNotDecrease,
    /// The transported witness list is not the exact result of verification.
    CertificateMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedMeasure => {
                "the Loop measure is outside the supported direct integer-path fragment"
            }
            Self::UnsupportedInvariant => {
                "the Loop invariant cannot be established at entry by this proof gate"
            }
            Self::UnsupportedActivation => "the Loop measure requires a supported entry Dispatch",
            Self::AmbiguousRecurrence => {
                "a recurrent handler does not select exactly one next state"
            }
            Self::MeasureNotPositive => {
                "a recurrent path does not prove that the Loop measure is positive"
            }
            Self::MeasureDoesNotDecrease => {
                "a recurrent path does not strictly decrease the Loop measure"
            }
            Self::CertificateMismatch => {
                "the Loop proof certificate disagrees with the shipped handlers"
            }
        })
    }
}

impl core::error::Error for Error {}

/// Independently re-derive the complete certificate from the entry Dispatch.
///
/// # Errors
/// Unsupported decisions or recurrence, or a witness multiset which differs
/// from the one derived from every recurrent leaf.
pub fn verify_dispatch(certificate: &Certificate, dispatch: &Dispatch) -> Result<(), Error> {
    let derived = derive_dispatch(
        certificate.measure.clone(),
        certificate.invariant.clone(),
        dispatch,
    )?;
    if derived.decreases != certificate.decreases {
        return Err(Error::CertificateMismatch);
    }
    Ok(())
}

/// Derives the direct integer-path certificate from a command Dispatch.
/// Branch facts include the false predicates skipped by an ordered match;
/// the fallback of `measure <= 0` can therefore establish positivity.
///
/// # Errors
/// A recurrent leaf without a positive, strictly decreasing measure, or a
/// decision outside the bounded certificate fragment.
pub fn derive_dispatch(
    measure: expr::Tree,
    invariant: Option<expr::Tree>,
    dispatch: &Dispatch,
) -> Result<Certificate, Error> {
    let path = measure_path(&measure)?;
    if invariant.as_ref().is_some_and(|tree| !is_true(tree)) {
        return Err(Error::UnsupportedInvariant);
    }
    let mut decreases = Vec::new();
    derive_decision(dispatch, path, &mut Vec::new(), &mut decreases, 0)?;
    decreases.sort_unstable();
    Ok(Certificate {
        measure,
        invariant,
        decreases,
    })
}

fn derive_decision<'a>(
    dispatch: &'a Dispatch,
    measure: &[String],
    facts: &mut Vec<(&'a Expr, bool)>,
    decreases: &mut Vec<u64>,
    depth: usize,
) -> Result<(), Error> {
    if depth > crate::plan::MAX_DISPATCH_DEPTH {
        return Err(Error::UnsupportedActivation);
    }
    match dispatch {
        Dispatch::Linear(steps) => {
            let mut recurrent = false;
            for step in steps {
                let DispatchStep::LoopCommand(command) = step else {
                    continue;
                };
                let update = match command {
                    LoopCommand::Next(update) => update,
                    LoopCommand::Again => return Err(Error::MeasureDoesNotDecrease),
                    LoopCommand::Done => continue,
                };
                if recurrent {
                    return Err(Error::AmbiguousRecurrence);
                }
                recurrent = true;
                if !facts
                    .iter()
                    .any(|(guard, truth)| guard_fact_proves_positive(guard, *truth, measure))
                {
                    return Err(Error::MeasureNotPositive);
                }
                let update =
                    measured_update(update, measure).ok_or(Error::MeasureDoesNotDecrease)?;
                decreases
                    .push(strict_decrease(update, measure).ok_or(Error::MeasureDoesNotDecrease)?);
            }
        }
        Dispatch::OpenMatch { arms, fallback, .. } => {
            let original = facts.len();
            for arm in arms {
                facts.push((&arm.predicate.root, true));
                derive_decision(&arm.dispatch, measure, facts, decreases, depth + 1)?;
                facts.pop();
                facts.push((&arm.predicate.root, false));
            }
            derive_decision(fallback, measure, facts, decreases, depth + 1)?;
            facts.truncate(original);
        }
        Dispatch::ClosedMatch {
            scrutinee, arms, ..
        } => {
            for arm in arms {
                let original = facts.len();
                if scrutinee.ty == crate::datatype::DataType::Bool && arm.cases.len() == 1 {
                    match arm.cases[0].as_str() {
                        "true" => facts.push((&scrutinee.root, true)),
                        "false" => facts.push((&scrutinee.root, false)),
                        _ => return Err(Error::UnsupportedActivation),
                    }
                }
                derive_decision(&arm.dispatch, measure, facts, decreases, depth + 1)?;
                facts.truncate(original);
            }
        }
    }
    Ok(())
}

fn measure_path(measure: &expr::Tree) -> Result<&[String], Error> {
    if !matches!(
        measure.ty,
        crate::datatype::DataType::Integer(crate::datatype::IntegerType::Signed(_))
    ) {
        return Err(Error::UnsupportedMeasure);
    }
    let Expr::Path(path) = &measure.root else {
        return Err(Error::UnsupportedMeasure);
    };
    data_path(path).ok_or(Error::UnsupportedMeasure)
}

fn data_path(path: &[String]) -> Option<&[String]> {
    match path {
        [root, ..] if root == "data" => Some(path),
        [root, state, data, ..] if root == "Loop" && state == "state" && data == "data" => {
            Some(&path[2..])
        }
        _ => None,
    }
}

fn is_true(tree: &expr::Tree) -> bool {
    matches!(tree.root, Expr::Literal(Value::Bool(true)))
        && tree.ty == crate::datatype::DataType::Bool
}

fn measured_update<'a>(emission: &'a EmitValue, measure: &[String]) -> Option<&'a Expr> {
    match emission {
        EmitValue::Data(tree) => {
            let (root, field_path) = measure.split_first()?;
            (root == "data")
                .then(|| expression_field(&tree.root, field_path))
                .flatten()
        }
        EmitValue::Patch(fields) | EmitValue::FreshData(fields) => {
            let (root, field_path) = measure.split_first()?;
            if root != "data" || field_path.is_empty() {
                return None;
            }
            let mut selected = None;
            for field in fields {
                let WriteTarget::Data(path) = &field.target else {
                    continue;
                };
                if path == field_path {
                    selected = Some(&field.value.root);
                } else if path.starts_with(field_path) || field_path.starts_with(path) {
                    // An ancestor replacement or descendant mutation changes
                    // the measured value through a form this fragment does
                    // not model. A later exact write may establish it again.
                    selected = None;
                }
            }
            selected
        }
        EmitValue::Identity | EmitValue::Nothing => None,
    }
}

fn expression_field<'a>(expression: &'a Expr, path: &[String]) -> Option<&'a Expr> {
    let Some((field, rest)) = path.split_first() else {
        return Some(expression);
    };
    let Expr::Struct(fields) = expression else {
        return None;
    };
    let (_, value) = fields.iter().find(|(name, _)| name == field)?;
    expression_field(value, rest)
}

fn strict_decrease(after: &Expr, measure: &[String]) -> Option<u64> {
    let Expr::Binary {
        op: BinaryOp::Sub,
        left,
        right,
    } = after
    else {
        return None;
    };
    if !is_path(left, measure) {
        return None;
    }
    match right.as_ref() {
        Expr::Literal(Value::Int(value)) if *value > 0 => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn guard_fact_proves_positive(guard: &Expr, truth: bool, measure: &[String]) -> bool {
    match guard {
        Expr::Unary {
            op: expr::UnaryOp::Not,
            expr,
        } => guard_fact_proves_positive(expr, !truth, measure),
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            if truth {
                guard_fact_proves_positive(left, true, measure)
                    || guard_fact_proves_positive(right, true, measure)
            } else {
                guard_fact_proves_positive(left, false, measure)
                    && guard_fact_proves_positive(right, false, measure)
            }
        }
        Expr::Binary {
            op: BinaryOp::Or,
            left,
            right,
        } => {
            if truth {
                guard_fact_proves_positive(left, true, measure)
                    && guard_fact_proves_positive(right, true, measure)
            } else {
                guard_fact_proves_positive(left, false, measure)
                    || guard_fact_proves_positive(right, false, measure)
            }
        }
        Expr::Binary { op, left, right } if is_path(left, measure) => {
            let Some(bound) = signed_literal(right) else {
                return false;
            };
            if truth {
                matches!(op, BinaryOp::Gt if bound >= 0) || matches!(op, BinaryOp::Ge if bound >= 1)
            } else {
                matches!(op, BinaryOp::Le if bound >= 0) || matches!(op, BinaryOp::Lt if bound >= 1)
            }
        }
        Expr::Binary { op, left, right } if is_path(right, measure) => {
            let Some(bound) = signed_literal(left) else {
                return false;
            };
            if truth {
                matches!(op, BinaryOp::Lt if bound >= 0) || matches!(op, BinaryOp::Le if bound >= 1)
            } else {
                matches!(op, BinaryOp::Ge if bound >= 0) || matches!(op, BinaryOp::Gt if bound >= 1)
            }
        }
        _ => false,
    }
}

fn is_path(expr: &Expr, expected: &[String]) -> bool {
    matches!(expr, Expr::Path(path) if data_path(path) == Some(expected))
}

fn signed_literal(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Literal(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::DataType;
    use crate::plan::{FieldWrite, OpenArm};
    use alloc::boxed::Box;
    use alloc::vec;

    fn path() -> Vec<String> {
        vec![String::from("data"), String::from("n")]
    }

    fn tree(root: Expr, ty: DataType) -> expr::Tree {
        expr::Tree { root, ty }
    }

    fn guarded(guard: Expr, update: EmitValue) -> Dispatch {
        Dispatch::OpenMatch {
            prefix: vec![],
            arms: vec![OpenArm {
                predicate: tree(guard, DataType::Bool),
                dispatch: Box::new(Dispatch::Linear(vec![DispatchStep::LoopCommand(
                    LoopCommand::Next(update),
                )])),
            }],
            fallback: Box::new(Dispatch::Linear(vec![DispatchStep::LoopCommand(
                LoopCommand::Done,
            )])),
        }
    }

    fn recurrent(guard: Expr, after: Expr) -> Dispatch {
        guarded(
            guard,
            EmitValue::FreshData(vec![FieldWrite {
                target: WriteTarget::Data(vec![String::from("n")]),
                value: tree(after, DataType::INT),
            }]),
        )
    }

    fn positive_guard() -> Expr {
        Expr::Binary {
            op: BinaryOp::Gt,
            left: Box::new(Expr::Path(path())),
            right: Box::new(Expr::Literal(Value::Int(0))),
        }
    }

    fn decrease(by: i64) -> Expr {
        Expr::Binary {
            op: BinaryOp::Sub,
            left: Box::new(Expr::Path(path())),
            right: Box::new(Expr::Literal(Value::Int(by))),
        }
    }

    fn certificate(decreases: Vec<u64>) -> Certificate {
        Certificate {
            measure: tree(Expr::Path(path()), DataType::INT),
            invariant: None,
            decreases,
        }
    }

    #[test]
    fn a_direct_countdown_is_a_small_checkable_certificate() {
        verify_dispatch(
            &certificate(vec![1]),
            &recurrent(positive_guard(), decrease(1)),
        )
        .unwrap();
    }

    #[test]
    fn a_non_decrease_or_unbounded_recurrence_never_becomes_a_proof() {
        assert_eq!(
            verify_dispatch(
                &certificate(vec![1]),
                &recurrent(positive_guard(), Expr::Path(path()))
            ),
            Err(Error::MeasureDoesNotDecrease)
        );
        assert_eq!(
            verify_dispatch(
                &certificate(vec![1]),
                &recurrent(Expr::Literal(Value::Bool(true)), decrease(1))
            ),
            Err(Error::MeasureNotPositive)
        );
    }

    #[test]
    fn a_certificate_cannot_lie_about_the_verified_decrease() {
        assert_eq!(
            verify_dispatch(
                &certificate(vec![2]),
                &recurrent(positive_guard(), decrease(1))
            ),
            Err(Error::CertificateMismatch)
        );
    }

    #[test]
    fn the_last_effective_write_owns_the_measured_field() {
        let exact = |root| FieldWrite {
            target: WriteTarget::Data(vec![String::from("n")]),
            value: tree(root, DataType::INT),
        };
        let dispatch = |writes| guarded(positive_guard(), EmitValue::FreshData(writes));
        assert_eq!(
            derive_dispatch(
                certificate(vec![]).measure,
                None,
                &dispatch(vec![exact(decrease(1)), exact(Expr::Path(path()))]),
            ),
            Err(Error::MeasureDoesNotDecrease),
            "an earlier decrease cannot hide a later overwrite"
        );
        assert_eq!(
            derive_dispatch(
                certificate(vec![]).measure,
                None,
                &dispatch(vec![exact(Expr::Path(path())), exact(decrease(1))]),
            )
            .unwrap()
            .decreases,
            [1],
            "the final exact write is the recurrence"
        );
    }

    #[test]
    fn an_unproved_invariant_never_rides_as_a_runtime_assertion() {
        let invariant = tree(Expr::Literal(Value::Bool(false)), DataType::Bool);
        assert_eq!(
            derive_dispatch(
                certificate(vec![]).measure,
                Some(invariant),
                &recurrent(positive_guard(), decrease(1)),
            ),
            Err(Error::UnsupportedInvariant)
        );
    }
}
