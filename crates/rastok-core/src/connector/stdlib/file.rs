//! Portable file Contract rules.

use super::FILE_CONNECTOR_ID;
use super::*;
use crate::connector::contract::{
    AttributeSpec, ConnectorContract, ContractMode, DefaultValue, EvaluationSite, ModeSelector,
    ModeSupport, PortContract, PortDataType, PortTypeChoice, PortTypeRule, Presence, ValueDomain,
};
use crate::connector::exchange::{Completion, SchedulerCompletion};
use crate::connector::{ConnectorDirection, ExchangeKind};
use crate::datatype::{DataType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField};
use crate::node::PortDirection;

/// Effective logical type of one currently executable `File` port.
///
/// This reads the shared Contract definition. `watch` is a
/// known source profile but has no executable row until its inbound protocol
/// is specified.
#[must_use]
pub fn file_port_type(
    contract: &crate::connector::ContractId,
    port: &str,
    mode_word: Option<&str>,
) -> Option<DataType> {
    if contract.as_str() != FILE.id
        || !mode_word
            .and_then(|word| FILE.profile(word))
            .is_some_and(|mode| matches!(mode.support, ModeSupport::Available))
    {
        return None;
    }
    FILE.port_type(port, |attribute| {
        (attribute == "mode").then_some(mode_word).flatten()
    })
    .ok()
}

/// Latest evaluation site admitted by the executable `File` contract.
#[must_use]
pub fn file_attribute_site(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    if contract.as_str() != FILE.id {
        return None;
    }
    definition_attribute_site(&FILE, attribute)
}

/// The portable terminal union of one `File` invocation.
///
/// Host error numbers, paths and platform messages deliberately do not enter
/// this causal type. The operation and requested path are already sealed by
/// the outbound intent; the cases describe only portable outcomes.
///
/// # Errors
///
/// Returns the canonical type constructor's precise invariant violation.
pub fn file_exit_type() -> Result<DataType, TypeError> {
    let empty = || StructType::new(Vec::new());
    let completed = TaggedCase::new("Completed", empty()?)?;
    let not_found = TaggedCase::new("NotFound", empty()?)?;
    let not_a_file = TaggedCase::new("NotAFile", empty()?)?;
    let permission_denied = TaggedCase::new("PermissionDenied", empty()?)?;
    let invalid_path = TaggedCase::new("InvalidPath", empty()?)?;
    let read_limit_exceeded = TaggedCase::new(
        "ReadLimitExceeded",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    let atomic_replace_unavailable = TaggedCase::new("AtomicReplaceUnavailable", empty()?)?;
    let io_failed = TaggedCase::new("IoFailed", empty()?)?;
    let timeout_expired = TaggedCase::new("TimeoutExpired", empty()?)?;
    let outcome_unknown = TaggedCase::new("OutcomeUnknown", empty()?)?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        not_found,
        not_a_file,
        permission_denied,
        invalid_path,
        read_limit_exceeded,
        atomic_replace_unavailable,
        io_failed,
        timeout_expired,
        outcome_unknown,
    ])?))
}

/// The portable terminal facts emitted by `File`.
///
/// This Rust enum is a Driver-side construction aid, not a second logical
/// type. [`FileExit::into_value`] is the sole projection into the ordinary
/// canonical tagged union carried by the `exit` port. Host error numbers,
/// paths, and platform messages never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileExit {
    /// The requested read or atomic replacement completed.
    Completed,
    /// No filesystem entry exists at the logical path.
    NotFound,
    /// The entry or one of its required path components is not a file.
    NotAFile,
    /// The host denied the operation.
    PermissionDenied,
    /// The logical path has no portable File spelling.
    InvalidPath,
    /// A complete read would exceed the declared bound.
    ReadLimitExceeded {
        /// Declared read bound in bytes.
        limit: u64,
    },
    /// The host cannot provide the contract's atomic replacement primitive.
    AtomicReplaceUnavailable,
    /// A portable classification above cannot describe the I/O failure.
    IoFailed,
    /// The scheduler's invocation budget won the close race.
    TimeoutExpired,
    /// Cancellation was requested, but no terminal observation arrived
    /// within the instance's explicit settlement budget.
    OutcomeUnknown,
}

impl FileExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::{TaggedUnionValue, Value};

        let (case, payload) = match self {
            Self::Completed => ("Completed", Vec::new()),
            Self::NotFound => ("NotFound", Vec::new()),
            Self::NotAFile => ("NotAFile", Vec::new()),
            Self::PermissionDenied => ("PermissionDenied", Vec::new()),
            Self::InvalidPath => ("InvalidPath", Vec::new()),
            Self::ReadLimitExceeded { limit } => (
                "ReadLimitExceeded",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::AtomicReplaceUnavailable => ("AtomicReplaceUnavailable", Vec::new()),
            Self::IoFailed => ("IoFailed", Vec::new()),
            Self::TimeoutExpired => ("TimeoutExpired", Vec::new()),
            Self::OutcomeUnknown => ("OutcomeUnknown", Vec::new()),
        };
        Value::TaggedUnion(alloc::boxed::Box::new(TaggedUnionValue {
            case: String::from(case),
            payload,
        }))
    }
}

/// Projects a machine-decided terminal fact into the File contract.
///
/// This is selected by a bound File Driver; the generic scheduler never
/// dispatches on a concrete Connector name.
#[must_use]
pub fn file_scheduler_completion(completion: SchedulerCompletion) -> Completion {
    Completion::file_exit(match completion {
        SchedulerCompletion::TimeoutExpired => FileExit::TimeoutExpired,
        SchedulerCompletion::OutcomeUnknown => FileExit::OutcomeUnknown,
    })
}

/// Standard declarative Contract; physical Drivers implement these resolved rules.
pub const FILE: ConnectorContract = ConnectorContract {
    id: FILE_CONNECTOR_ID,
    name: "File",
    roles: ConnectorDirection::Outbound,
    exchange: ExchangeKind::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: PortDirection::Input,

            ty: PortTypeRule::AttributeSelected {
                attribute: "mode",
                choices: &[
                    PortTypeChoice {
                        word: "read",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "replace",
                        ty: PortDataType::Bytes,
                    },
                    // The deferred selector word keeps this table total. It
                    // promises no future trigger shape: lifting an inbound
                    // watch requires an explicit contract-version decision.
                    PortTypeChoice {
                        word: "watch",
                        ty: PortDataType::Variant,
                    },
                ],
            },
        },
        PortContract {
            name: "out",
            direction: PortDirection::Output,

            ty: PortTypeRule::Fixed(PortDataType::Bytes),
        },
        PortContract {
            name: "exit",
            direction: PortDirection::Output,

            ty: PortTypeRule::Fixed(PortDataType::FileExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "path",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "idempotent",
            presence: Presence::Defaulted(DefaultValue::Expression("true")),
            domain: ValueDomain::Boolean,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Defaulted(DefaultValue::Choice("deterministic")),
            domain: ValueDomain::Choice(&["deterministic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "timeout",
            presence: Presence::Optional,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
    ],
    selector: Some(ModeSelector {
        attribute: "mode",
        // Reading versus replacing is user intent and never inferred from the
        // payload or wiring.
        default: None,
        profiles: &[
            ContractMode {
                protocol: None,
                word: "read",
                support: ModeSupport::Available,
                attributes: &[AttributeSpec {
                    name: "max_read",
                    presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
                    domain: ValueDomain::ByteSize,
                    latest_site: EvaluationSite::Plan,
                }],
            },
            ContractMode {
                protocol: None,
                word: "replace",
                support: ModeSupport::Available,
                attributes: &[],
            },
            ContractMode {
                protocol: None,
                word: "watch",
                support: ModeSupport::Deferred("the File watch profile"),
                attributes: &[],
            },
        ],
    }),
};

/// Checks the resolved File operation and its operation-specific limits.
pub(super) fn validate(node: &Node, spec: &ConnectorSpec) -> Result<(), PlanValidationError> {
    if spec.roles != FILE.roles {
        return Err(PlanValidationError::ContractRoleMismatch {
            node: node.path.clone(),
        });
    }
    if spec.exchange != FILE.exchange {
        return Err(PlanValidationError::ContractExchangeMismatch {
            node: node.path.clone(),
        });
    }
    if spec.behavior != node::Behavior::Deterministic {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("behavior"),
        });
    }
    let mode = match spec.attribute("mode") {
        Some(crate::plan::AttrSetting::Word(word))
            if FILE
                .profile(word)
                .is_some_and(|mode| matches!(mode.support, ModeSupport::Available)) =>
        {
            word.as_str()
        }
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("mode"),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "mode",
            });
        }
    };
    match spec.attribute("path") {
        Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(_))) => {}
        Some(crate::plan::AttrSetting::Expr { site, expr })
            if matches!(site, EvaluationSite::Instance | EvaluationSite::Invocation)
                && expr.ty == DataType::Text => {}
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("path"),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "path",
            });
        }
    }
    if let Some(timeout) = spec.attribute("timeout") {
        let valid = match timeout {
            crate::plan::AttrSetting::Value(crate::value::Value::Duration(_)) => true,
            crate::plan::AttrSetting::Expr { site, expr } => {
                *site == EvaluationSite::Instance && expr.ty == DataType::DURATION
            }
            _ => false,
        };
        if !valid {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("timeout"),
            });
        }
    }
    match (mode, spec.attribute("max_read")) {
        ("read", Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(value))))
            if crate::connector::byte_size(value).is_some() => {}
        ("read", None) => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "max_read",
            });
        }
        ("replace", None) => {}
        _ => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("max_read"),
            });
        }
    }
    for (name, _) in &spec.attributes {
        if name != "mode"
            && (name == "behavior" || FILE.attribute(FILE.profile(mode), name).is_none())
        {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: name.clone(),
            });
        }
    }

    for declared in FILE.ports {
        let (name, direction) = (declared.name, declared.direction);
        let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
            return Err(PlanValidationError::MissingContractPort {
                node: node.path.clone(),
                port: name,
            });
        };
        if port.direction != direction
            || port.ty
                != effective_contract_port(
                    node,
                    node.loop_spec(),
                    name,
                    crate::connector::stdlib::file_port_type(&spec.contract, name, Some(mode)),
                )
        {
            return Err(PlanValidationError::ContractPortMismatch {
                node: node.path.clone(),
                port: port.port.clone(),
            });
        }
    }
    if let Some(port) = node.ports.iter().find(|port| {
        !FILE
            .ports
            .iter()
            .any(|declared| declared.name == port.port.name())
    }) {
        return Err(PlanValidationError::UnexpectedContractPort {
            node: node.path.clone(),
            port: port.port.clone(),
        });
    }
    Ok(())
}
