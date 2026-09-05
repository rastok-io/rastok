//! Registry-free closure proofs shared by handler Dispatch and case routing.
//!
//! This verifier inspects an already resolved source type. It neither infers
//! productibility nor trusts a transported list of supposedly possible cases.
//! Source normalization happens before this boundary; malformed tables are
//! rejected, never sorted or repaired here.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::datatype::DataType;

/// The canonical symbolic domain of a closed match, derived from its type.
///
/// Bool, enum and tagged union are the only admitted domains. A structural
/// union, an optional type or `variant` cannot acquire named cases merely
/// because an arm table happens to enumerate its observed values.
#[must_use]
pub fn closed_case_domain(ty: &DataType) -> Option<Vec<String>> {
    match ty {
        DataType::Bool => Some(vec![String::from("false"), String::from("true")]),
        DataType::Enum(symbols) => Some(symbols.symbols().to_vec()),
        DataType::TaggedUnion(union) => Some(
            union
                .cases()
                .iter()
                .map(|case| case.name().to_string())
                .collect(),
        ),
        _ => None,
    }
}

/// A closed arm table contradicts its resolved domain or canonical order.
/// Arm indices are zero-based positions in the checked table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasePartitionError {
    /// The source has no admitted closed symbolic domain.
    NotClosed,
    /// An arm selects no case.
    EmptyArm {
        /// Empty arm position.
        arm: usize,
    },
    /// Cases inside one arm are not strictly increasing.
    CaseOrder {
        /// Non-canonical arm position.
        arm: usize,
    },
    /// Arms are not strictly increasing by their canonical case sets.
    ArmOrder {
        /// First out-of-order arm position.
        arm: usize,
    },
    /// A case is not part of the resolved source domain.
    UnknownCase {
        /// Arm carrying the unknown case.
        arm: usize,
        /// Unknown symbolic name.
        case: String,
    },
    /// Two selections name the same case, within one arm or across arms.
    Overlap {
        /// Arm carrying the repeated selection.
        arm: usize,
        /// Repeated symbolic name.
        case: String,
    },
    /// A source case has no arm; the first canonical missing case is named.
    MissingCase {
        /// Uncovered symbolic name.
        case: String,
    },
}

impl fmt::Display for CasePartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotClosed => f.write_str("match domain is not a bool, enum or tagged union"),
            Self::EmptyArm { arm } => write!(f, "match arm {arm} has no cases"),
            Self::CaseOrder { arm } => write!(f, "match arm {arm} has non-canonical case order"),
            Self::ArmOrder { arm } => write!(f, "match arm {arm} is out of canonical order"),
            Self::UnknownCase { arm, case } => {
                write!(f, "match arm {arm} names unknown case `{case}`")
            }
            Self::Overlap { arm, case } => {
                write!(f, "match arm {arm} selects case `{case}` more than once")
            }
            Self::MissingCase { case } => write!(f, "match leaves case `{case}` uncovered"),
        }
    }
}

impl core::error::Error for CasePartitionError {}

/// Proves a canonical, disjoint, exhaustive partition of the source domain.
///
/// The same check applies to a Dispatch table and to a route case table;
/// their different destinations/bodies cannot affect closure. `_` must have
/// been expanded into its exact remainder before calling. Working storage
/// is bounded by the type's domain, not by the untrusted arm table.
///
/// # Errors
///
/// Returns the first violated closure or ordering invariant without changing
/// the input table. In particular, duplicate or unknown cases never make up
/// for a missing case just because a claimed case count is correct.
pub fn validate_case_partition<'a>(
    ty: &DataType,
    arms: impl IntoIterator<Item = &'a [String]>,
) -> Result<(), CasePartitionError> {
    let domain = closed_case_domain(ty).ok_or(CasePartitionError::NotClosed)?;
    let mut selected = vec![false; domain.len()];
    let mut previous_arm: Option<&[String]> = None;
    for (arm, cases) in arms.into_iter().enumerate() {
        if cases.is_empty() {
            return Err(CasePartitionError::EmptyArm { arm });
        }
        let mut previous_case: Option<&String> = None;
        for case in cases {
            let index =
                domain
                    .binary_search(case)
                    .map_err(|_| CasePartitionError::UnknownCase {
                        arm,
                        case: case.clone(),
                    })?;
            if selected[index] {
                return Err(CasePartitionError::Overlap {
                    arm,
                    case: case.clone(),
                });
            }
            if previous_case.is_some_and(|previous| previous > case) {
                return Err(CasePartitionError::CaseOrder { arm });
            }
            selected[index] = true;
            previous_case = Some(case);
        }
        if previous_arm.is_some_and(|previous| previous >= cases) {
            return Err(CasePartitionError::ArmOrder { arm });
        }
        previous_arm = Some(cases);
    }
    if let Some(index) = selected.iter().position(|selected| !selected) {
        return Err(CasePartitionError::MissingCase {
            case: domain[index].clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{EnumType, MAX_TYPE_MEMBERS, StructType, TaggedCase, TaggedUnionType};
    use alloc::format;

    fn arms(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|case| (*case).to_string()).collect())
            .collect()
    }

    fn verify(ty: &DataType, rows: &[&[&str]]) -> Result<(), CasePartitionError> {
        validate_case_partition(ty, arms(rows).iter().map(Vec::as_slice))
    }

    #[test]
    fn the_domain_comes_from_the_type_not_the_arm_claims() {
        for ty in [
            DataType::INT,
            DataType::Text,
            DataType::Variant,
            DataType::Bool.optional().unwrap(),
        ] {
            assert_eq!(closed_case_domain(&ty), None);
            assert_eq!(
                verify(&ty, &[&["false"], &["true"]]),
                Err(CasePartitionError::NotClosed)
            );
        }
        assert_eq!(
            closed_case_domain(&DataType::Bool),
            Some(vec!["false".into(), "true".into()])
        );
    }

    #[test]
    fn boolean_partitions_are_exhaustive_without_an_implicit_fallback() {
        assert_eq!(verify(&DataType::Bool, &[&["false"], &["true"]]), Ok(()));
        assert_eq!(verify(&DataType::Bool, &[&["false", "true"]]), Ok(()));
        assert_eq!(
            verify(&DataType::Bool, &[&["true"]]),
            Err(CasePartitionError::MissingCase {
                case: "false".into()
            })
        );
        assert_eq!(
            verify(&DataType::Bool, &[]),
            Err(CasePartitionError::MissingCase {
                case: "false".into()
            })
        );
        assert_eq!(
            verify(&DataType::Bool, &[&["_"]]),
            Err(CasePartitionError::UnknownCase {
                arm: 0,
                case: "_".into()
            })
        );
    }

    #[test]
    fn enum_and_tagged_union_names_have_the_same_partition_rule() {
        let enumeration =
            DataType::Enum(EnumType::new(vec!["c".into(), "a".into(), "b".into()]).unwrap());
        let union = DataType::TaggedUnion(
            TaggedUnionType::new(
                ["c", "a", "b"]
                    .into_iter()
                    .map(|case| TaggedCase::new(case, StructType::new(vec![]).unwrap()).unwrap())
                    .collect(),
            )
            .unwrap(),
        );
        for ty in [enumeration, union] {
            assert_eq!(
                closed_case_domain(&ty),
                Some(vec!["a".into(), "b".into(), "c".into()])
            );
            assert_eq!(verify(&ty, &[&["a", "c"], &["b"]]), Ok(()));
            assert_eq!(verify(&ty, &[&["a"], &["b", "c"]]), Ok(()));
            assert_eq!(
                verify(&ty, &[&["a"], &["b"], &["other"]]),
                Err(CasePartitionError::UnknownCase {
                    arm: 2,
                    case: "other".into()
                })
            );
        }
    }

    #[test]
    fn duplicate_counts_cannot_masquerade_as_coverage() {
        assert_eq!(
            verify(&DataType::Bool, &[&["false", "false"]]),
            Err(CasePartitionError::Overlap {
                arm: 0,
                case: "false".into()
            })
        );
        assert_eq!(
            verify(&DataType::Bool, &[&["false"], &["false", "true"]]),
            Err(CasePartitionError::Overlap {
                arm: 1,
                case: "false".into()
            })
        );
    }

    #[test]
    fn noncanonical_tables_are_rejected_without_repair() {
        assert_eq!(
            verify(&DataType::Bool, &[&[]]),
            Err(CasePartitionError::EmptyArm { arm: 0 })
        );
        assert_eq!(
            verify(&DataType::Bool, &[&["true", "false"]]),
            Err(CasePartitionError::CaseOrder { arm: 0 })
        );
        assert_eq!(
            verify(&DataType::Bool, &[&["true"], &["false"]]),
            Err(CasePartitionError::ArmOrder { arm: 1 })
        );
        let table = arms(&[&["true"], &["false"]]);
        let before = table.clone();
        assert!(validate_case_partition(&DataType::Bool, table.iter().map(Vec::as_slice)).is_err());
        assert_eq!(table, before);
    }

    #[test]
    fn the_maximum_type_domain_is_admitted_without_a_larger_table_budget() {
        let names: Vec<String> = (0..MAX_TYPE_MEMBERS)
            .map(|index| format!("case{index:03}"))
            .collect();
        let ty = DataType::Enum(EnumType::new(names.clone()).unwrap());
        let table: Vec<Vec<String>> = names.into_iter().map(|name| vec![name]).collect();
        assert_eq!(
            validate_case_partition(&ty, table.iter().map(Vec::as_slice)),
            Ok(())
        );
    }

    #[test]
    fn bounded_adversarial_tables_match_an_independent_flattening_oracle() {
        let candidates = arms(&[
            &[],
            &["false"],
            &["true"],
            &["false", "true"],
            &["true", "false"],
            &["false", "false"],
            &["ghost"],
            &["_"],
        ]);
        // All tables of zero through three arms, including empty, overlapping,
        // reversed, incomplete and out-of-domain selections.
        for length in 0..=3 {
            for mut code in 0..candidates.len().pow(length) {
                let mut table = Vec::new();
                for _ in 0..length {
                    table.push(candidates[code % candidates.len()].as_slice());
                    code /= candidates.len();
                }
                let mut flattened: Vec<&str> = table
                    .iter()
                    .flat_map(|row| row.iter().map(String::as_str))
                    .collect();
                flattened.sort();
                let expected = flattened == ["false", "true"]
                    && table
                        .iter()
                        .all(|row| !row.is_empty() && row.windows(2).all(|pair| pair[0] < pair[1]))
                    && table.windows(2).all(|pair| pair[0] < pair[1]);
                assert_eq!(
                    validate_case_partition(&DataType::Bool, table.iter().copied()).is_ok(),
                    expected,
                    "{table:?}"
                );
            }
        }
    }
}
