//! Small, deterministic certificates for finite `Loop` recursion.
//!
//! This module is deliberately narrower than the expression evaluator. A
//! proof is accepted only when every recurrent handler exposes one direct
//! affine decrease that this `no_std` verifier can re-derive from the shipped
//! handler table. Unsupported mathematics is a refusal, never an assumption.

use crate::envelope::WriteTarget;
use crate::expr::{self, BinaryOp, Expr};
use crate::node;
use crate::plan::{DispatchStep, Emission, Handler, HandlerOwner, OwnerSlot};
use crate::value::Value;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// One canonical termination certificate.
///
/// `decreases` is the sorted multiset of exact positive decrements from every
/// handler owned by the Loop Graph which emits `next`. Sorting makes the
/// witness independent of source and table order while retaining duplicate
/// paths. The verifier derives the same multiset from the handler expressions
/// and requires byte-for-byte agreement; the list is evidence, never
/// authority.
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
            Self::UnsupportedActivation => "a recurrent path is not a direct Graph input handler",
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

/// Verifies a certificate against the canonical handler table.
///
/// The initial fragment proves a direct signed-integer ranking path such as
/// `data.n`, guarded by `data.n > 0` (or an equivalent stronger bound), and
/// updated as `data.n - k` for a positive integer literal `k`. This is enough
/// to establish that no infinite sequence of `next` decisions exists. The
/// ordinary Loop control proof remains responsible for exhaustive terminal
/// versus recurrent decisions.
///
/// # Errors
///
/// Any expression, activation, invariant, guard, update, or witness outside
/// that closed fragment is refused with an [`Error`].
pub fn verify(
    owner: OwnerSlot,
    certificate: &Certificate,
    handlers: &[Handler],
) -> Result<(), Error> {
    let derived = derive(
        owner,
        certificate.measure.clone(),
        certificate.invariant.clone(),
        handlers,
    )?;
    if derived.decreases != certificate.decreases {
        return Err(Error::CertificateMismatch);
    }
    Ok(())
}

/// Derives the canonical certificate for the first supported proof fragment.
///
/// The hosted compiler uses this constructor to ship evidence. Validation
/// calls [`verify`] after decode and derives the same evidence again; a
/// transported list is therefore never trusted.
///
/// # Errors
///
/// The same closed-fragment refusals as [`verify`].
pub fn derive(
    owner: OwnerSlot,
    measure: expr::Tree,
    invariant: Option<expr::Tree>,
    handlers: &[Handler],
) -> Result<Certificate, Error> {
    let measure_path = measure_path(&measure)?;
    if invariant.as_ref().is_some_and(|tree| !is_true(tree)) {
        return Err(Error::UnsupportedInvariant);
    }

    let mut derived = Vec::new();
    for handler in handlers
        .iter()
        .filter(|handler| handler.owner == HandlerOwner::Graph(owner))
    {
        let Some(next) = next_emission(handler)? else {
            continue;
        };
        if !matches!(
            &handler.activation,
            node::HandlerActivation::Port(port) if port.name() == "in"
        ) {
            return Err(Error::UnsupportedActivation);
        }
        let guard = handler.guard.as_ref().ok_or(Error::MeasureNotPositive)?;
        if !guard_proves_positive(&guard.root, measure_path) {
            return Err(Error::MeasureNotPositive);
        }
        let after = measured_update(next, measure_path).ok_or(Error::MeasureDoesNotDecrease)?;
        let decrease = strict_decrease(after, measure_path).ok_or(Error::MeasureDoesNotDecrease)?;
        derived.push(decrease);
    }
    derived.sort_unstable();
    Ok(Certificate {
        measure,
        invariant,
        decreases: derived,
    })
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
    if path.first().is_none_or(|root| root != "data") {
        return Err(Error::UnsupportedMeasure);
    }
    Ok(path)
}

fn is_true(tree: &expr::Tree) -> bool {
    matches!(tree.root, Expr::Literal(Value::Bool(true)))
        && tree.ty == crate::datatype::DataType::Bool
}

fn next_emission(handler: &Handler) -> Result<Option<&Emission>, Error> {
    let mut next = handler
        .body
        .iter()
        .flat_map(DispatchStep::emissions)
        .filter_map(|(port, value)| (port.name() == "next").then_some(value));
    let Some(selected) = next.next() else {
        return Ok(None);
    };
    if next.next().is_some() {
        return Err(Error::AmbiguousRecurrence);
    }
    Ok(Some(selected))
}

fn measured_update<'a>(emission: &'a Emission, measure: &[String]) -> Option<&'a Expr> {
    match emission {
        Emission::Data(tree) => {
            let (root, field_path) = measure.split_first()?;
            (root == "data")
                .then(|| expression_field(&tree.root, field_path))
                .flatten()
        }
        Emission::Patch(fields) | Emission::FreshData(fields) => {
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
        Emission::Identity | Emission::Nothing => None,
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

fn guard_proves_positive(guard: &Expr, measure: &[String]) -> bool {
    match guard {
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => guard_proves_positive(left, measure) || guard_proves_positive(right, measure),
        Expr::Binary { op, left, right } if is_path(left, measure) => {
            let Some(bound) = signed_literal(right) else {
                return false;
            };
            matches!(op, BinaryOp::Gt if bound >= 0) || matches!(op, BinaryOp::Ge if bound >= 1)
        }
        Expr::Binary { op, left, right } if is_path(right, measure) => {
            let Some(bound) = signed_literal(left) else {
                return false;
            };
            matches!(op, BinaryOp::Lt if bound >= 0) || matches!(op, BinaryOp::Le if bound >= 1)
        }
        _ => false,
    }
}

fn is_path(expr: &Expr, expected: &[String]) -> bool {
    matches!(expr, Expr::Path(path) if path == expected)
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
    use crate::plan::FieldWrite;
    use alloc::boxed::Box;
    use alloc::vec;

    fn path() -> Vec<String> {
        vec![String::from("data"), String::from("n")]
    }

    fn tree(root: Expr, ty: DataType) -> expr::Tree {
        expr::Tree { root, ty }
    }

    fn recurrent(guard: Expr, after: Expr) -> Handler {
        Handler {
            owner: HandlerOwner::Graph(OwnerSlot::new(0)),
            activation: node::HandlerActivation::Port(node::Port::new("in").unwrap()),
            guard: Some(tree(guard, DataType::Bool)),
            body: vec![DispatchStep::Emit {
                port: node::Port::new("next").unwrap(),
                value: Emission::FreshData(vec![FieldWrite {
                    target: WriteTarget::Data(vec![String::from("n")]),
                    value: tree(after, DataType::INT),
                }]),
            }],
        }
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
    fn a_direct_affine_countdown_is_a_small_checkable_certificate() {
        verify(
            OwnerSlot::new(0),
            &certificate(vec![1]),
            &[recurrent(positive_guard(), decrease(1))],
        )
        .unwrap();
    }

    #[test]
    fn a_non_decrease_or_unbounded_recurrence_never_becomes_a_proof() {
        assert_eq!(
            verify(
                OwnerSlot::new(0),
                &certificate(vec![1]),
                &[recurrent(positive_guard(), Expr::Path(path()))],
            ),
            Err(Error::MeasureDoesNotDecrease)
        );
        assert_eq!(
            verify(
                OwnerSlot::new(0),
                &certificate(vec![1]),
                &[recurrent(Expr::Literal(Value::Bool(true)), decrease(1))],
            ),
            Err(Error::MeasureNotPositive)
        );
    }

    #[test]
    fn a_certificate_cannot_lie_about_the_verified_decrease() {
        assert_eq!(
            verify(
                OwnerSlot::new(0),
                &certificate(vec![2]),
                &[recurrent(positive_guard(), decrease(1))],
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
        let handler = |writes| Handler {
            owner: HandlerOwner::Graph(OwnerSlot::new(0)),
            activation: node::HandlerActivation::Port(node::Port::new("in").unwrap()),
            guard: Some(tree(positive_guard(), DataType::Bool)),
            body: vec![DispatchStep::Emit {
                port: node::Port::new("next").unwrap(),
                value: Emission::FreshData(writes),
            }],
        };

        assert_eq!(
            derive(
                OwnerSlot::new(0),
                certificate(vec![]).measure,
                None,
                &[handler(
                    vec![exact(decrease(1)), exact(Expr::Path(path())),]
                )],
            ),
            Err(Error::MeasureDoesNotDecrease),
            "an earlier decrease cannot hide a later overwrite"
        );
        assert_eq!(
            derive(
                OwnerSlot::new(0),
                certificate(vec![]).measure,
                None,
                &[handler(
                    vec![exact(Expr::Path(path())), exact(decrease(1)),]
                )],
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
            derive(
                OwnerSlot::new(0),
                certificate(vec![]).measure,
                Some(invariant),
                &[recurrent(positive_guard(), decrease(1))],
            ),
            Err(Error::UnsupportedInvariant)
        );
    }
}
