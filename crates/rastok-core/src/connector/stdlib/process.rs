//! Portable process Contract rules.

use super::PROCESS_CONNECTOR_ID;
use super::*;
use crate::connector::contract::{
    AttributeSpec, ConnectorContract, ContractMode, DefaultValue, EvaluationSite, ModeSelector,
    ModeSupport, PortContract, PortDataType, PortTypeChoice, PortTypeRule, Presence, ValueDomain,
};
use crate::connector::exchange::{Completion, RequestPreparation, SchedulerCompletion};
use crate::connector::{ConnectorDirection, ExchangeKind};
use crate::datatype::{
    DataType, EnumType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField,
};
use crate::node::PortDirection;

/// Effective logical type of one currently executable Connector port.
///
/// The shared Contract definition resolves the same type for the compiler
/// and the independent package validator.
#[must_use]
pub fn process_port_type(
    contract: &crate::connector::ContractId,
    port: &str,
    codec_word: Option<&str>,
    err_codec_word: Option<&str>,
) -> Option<DataType> {
    if !PROCESS.protocol_ids().any(|id| id == contract.as_str())
        || !matches!(codec_word, Some("lines" | "jsonl" | "bytes"))
    {
        return None;
    }
    PROCESS
        .port_type(port, |attribute| match attribute {
            "codec" => codec_word,
            "err_codec" => err_codec_word,
            _ => None,
        })
        .ok()
}

/// Latest evaluation site admitted by the currently executable `Process`
/// contract for one attribute.
///
/// The shared definition rejects late bindings without linking the compiler
/// catalogue or a physical Driver.
#[must_use]
pub fn process_attribute_site(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    if !PROCESS.protocol_ids().any(|id| id == contract.as_str()) {
        return None;
    }
    if attribute == "idempotent" && contract.as_str() != PROCESS_CONNECTOR_ID {
        return None;
    }
    definition_attribute_site(&PROCESS, attribute)
}

/// The canonical logical type of the conventional `Process.exit` port.
///
/// The Contract, plan validator and Drivers use this same tagged union.
///
/// # Errors
///
/// Returns the canonical type constructor's precise invariant violation.
pub fn process_exit_type() -> Result<DataType, TypeError> {
    let empty = || StructType::new(Vec::new());
    let optional_int = DataType::INT.optional()?;
    let optional_text = DataType::Text.optional()?;
    let exited = TaggedCase::new(
        "Exited",
        StructType::new(alloc::vec![
            TypeField::new("exit_code", DataType::INT),
            TypeField::new("signal", optional_int),
        ])?,
    )?;
    let spawn_failed = TaggedCase::new(
        "SpawnFailed",
        StructType::new(alloc::vec![TypeField::new("message", DataType::Text)])?,
    )?;
    let timeout_expired = TaggedCase::new("TimeoutExpired", empty()?)?;
    let decode_failed = TaggedCase::new(
        "DecodeFailed",
        StructType::new(alloc::vec![
            TypeField::new("line", optional_text),
            TypeField::new("message", DataType::Text),
        ])?,
    )?;
    let capture_failed = TaggedCase::new("CaptureFailed", empty()?)?;
    let outcome_unknown = TaggedCase::new("OutcomeUnknown", empty()?)?;
    let reason = DataType::Enum(EnumType::new(alloc::vec![
        String::from("budget"),
        String::from("capacity"),
    ])?);
    let output_overflow = TaggedCase::new(
        "OutputOverflow",
        StructType::new(alloc::vec![
            TypeField::new("limit", DataType::UINT),
            TypeField::new("reason", reason),
        ])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        exited,
        spawn_failed,
        timeout_expired,
        decode_failed,
        capture_failed,
        outcome_unknown,
        output_overflow,
    ])?))
}

/// The semantic terminal facts emitted by `Process`.
///
/// This Rust enum is a Driver-side construction aid, not a second logical
/// type. [`ProcessExit::into_value`] is the sole projection into the ordinary
/// canonical [`crate::value::Value::TaggedUnion`] carried by the `exit` port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessExit {
    /// The child status was observed; non-zero is data, not an automatic
    /// failure classification. `exit_code = -1` denotes a signal-only status.
    Exited {
        /// Portable process exit code, or `-1` when only a signal exists.
        exit_code: i32,
        /// Unix signal when available.
        signal: Option<i32>,
    },
    /// The OS refused to create the child.
    SpawnFailed {
        /// Boundary diagnostic.
        message: String,
    },
    /// The scheduler's call budget won the close race.
    TimeoutExpired,
    /// Captured stdout or stderr did not inhabit its selected framing.
    DecodeFailed {
        /// Framing or decoding diagnostic.
        message: String,
        /// Offending JSONL line when one exists.
        line: Option<String>,
    },
    /// A pipe or status capture could not be completed.
    CaptureFailed,
    /// Cancellation was requested, but no terminal observation arrived
    /// within the instance's explicit settlement budget.
    OutcomeUnknown,
    /// The aggregate capture bound or representable host capacity was hit.
    OutputOverflow {
        /// Declared aggregate byte limit.
        limit: u64,
        /// Why the bounded capture stopped.
        reason: OutputOverflowReason,
    },
}

/// Why a bounded Process capture stopped.
///
/// Keeping this closed beside [`ProcessExit`] prevents a Driver from
/// constructing an enum value outside the logical `ProcessExit` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputOverflowReason {
    /// The declared aggregate byte budget was crossed.
    Budget,
    /// The host refused capture capacity.
    Capacity,
}

impl OutputOverflowReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Capacity => "capacity",
        }
    }
}

impl ProcessExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::{TaggedUnionValue, Value};

        let optional_int = |value: Option<i32>| Value::StructuralUnion {
            branch: u16::from(value.is_some()),
            value: alloc::boxed::Box::new(
                value.map_or(Value::Null, |value| Value::Int(i64::from(value))),
            ),
        };
        let optional_text = |value: Option<String>| Value::StructuralUnion {
            branch: u16::from(value.is_some()),
            value: alloc::boxed::Box::new(value.map_or(Value::Null, Value::Text)),
        };
        let (case, payload) = match self {
            Self::Exited { exit_code, signal } => (
                "Exited",
                alloc::vec![
                    (String::from("exit_code"), Value::Int(i64::from(exit_code))),
                    (String::from("signal"), optional_int(signal)),
                ],
            ),
            Self::SpawnFailed { message } => (
                "SpawnFailed",
                alloc::vec![(String::from("message"), Value::Text(message))],
            ),
            Self::TimeoutExpired => ("TimeoutExpired", Vec::new()),
            Self::DecodeFailed { message, line } => (
                "DecodeFailed",
                alloc::vec![
                    (String::from("line"), optional_text(line)),
                    (String::from("message"), Value::Text(message)),
                ],
            ),
            Self::CaptureFailed => ("CaptureFailed", Vec::new()),
            Self::OutcomeUnknown => ("OutcomeUnknown", Vec::new()),
            Self::OutputOverflow { limit, reason } => (
                "OutputOverflow",
                alloc::vec![
                    (String::from("limit"), Value::Uint(limit)),
                    (
                        String::from("reason"),
                        Value::Enum(String::from(reason.as_str()))
                    ),
                ],
            ),
        };
        Value::TaggedUnion(alloc::boxed::Box::new(TaggedUnionValue {
            case: String::from(case),
            payload,
        }))
    }
}

/// Frames one Process request or returns its exact typed pre-call refusal.
///
/// This helper is shared by the real Driver and scripted Process worlds; it
/// is not an Execution fallback. The selected Driver binding calls it explicitly.
#[must_use]
pub fn prepare_process_request(codec: RequestCodec, data: &envelope::Data) -> RequestPreparation {
    match prepare_request(codec, data) {
        Ok(prepared) => RequestPreparation::Prepared(prepared),
        Err(message) => {
            RequestPreparation::Rejected(Completion::process_exit(ProcessExit::DecodeFailed {
                message,
                line: None,
            }))
        }
    }
}

/// Projects a machine-decided terminal fact into the Process contract.
///
/// Like [`prepare_process_request`], this is selected by a bound Process
/// Driver and is never concrete-name dispatch in the Execution.
#[must_use]
pub fn process_scheduler_completion(completion: SchedulerCompletion) -> Completion {
    Completion::process_exit(match completion {
        SchedulerCompletion::TimeoutExpired => ProcessExit::TimeoutExpired,
        SchedulerCompletion::OutcomeUnknown => ProcessExit::OutcomeUnknown,
    })
}

/// Standard declarative Contract; physical Drivers implement these resolved rules.
pub const PROCESS: ConnectorContract = ConnectorContract {
    id: PROCESS_CONNECTOR_ID,
    name: "Process",
    roles: ConnectorDirection::Outbound,
    exchange: ExchangeKind::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: PortDirection::Input,

            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "jsonl",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "out",
            direction: PortDirection::Output,

            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "jsonl",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "err",
            direction: PortDirection::Output,

            ty: PortTypeRule::AttributeSelected {
                attribute: "err_codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "exit",
            direction: PortDirection::Output,

            // The Contract, port table and completion check share this union.
            ty: PortTypeRule::Fixed(PortDataType::ProcessExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "args",
            presence: Presence::Defaulted(DefaultValue::Expression("[]")),
            domain: ValueDomain::TextList,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic", "probabilistic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // The framing is the sheet's most semantic decision —
            // bytes, digest, reply type and error surface all follow
            // it — so it is never chosen silently (D-2026-08-11-1,
            // reaffirmed by D-2026-08-12-1).
            name: "codec",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["bytes", "jsonl", "lines"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // stderr is an independent physical channel. It never inherits
            // the stdout codec and defaults to one text value per line.
            name: "err_codec",
            presence: Presence::Defaulted(DefaultValue::Choice("lines")),
            domain: ValueDomain::Choice(&["bytes", "lines"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "command",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "cwd",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "env",
            presence: Presence::Optional,
            domain: ValueDomain::TextMap,
            latest_site: EvaluationSite::Instance,
        },
    ],
    selector: Some(ModeSelector {
        attribute: "mode",
        default: Some("job"),
        profiles: &[
            ContractMode {
                protocol: None,
                word: "job",
                support: ModeSupport::Available,
                attributes: &[
                    AttributeSpec {
                        name: "idempotent",
                        presence: Presence::Defaulted(DefaultValue::Expression("false")),
                        domain: ValueDomain::Boolean,
                        latest_site: EvaluationSite::Plan,
                    },
                    AttributeSpec {
                        name: "timeout",
                        presence: Presence::Optional,
                        domain: ValueDomain::Duration,
                        latest_site: EvaluationSite::Instance,
                    },
                    AttributeSpec {
                        // Judged by `contract::byte_size` — the same
                        // parser the Driver enforces the budget
                        // with (D-2026-08-12-1, review round).
                        name: "max_output",
                        presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
                        domain: ValueDomain::ByteSize,
                        latest_site: EvaluationSite::Plan,
                    },
                    AttributeSpec {
                        // Bypassing a detached one-shot is
                        // meaningless: the domain has ONE word, and
                        // the diagnostic says "fixed to".
                        name: "on_timeout",
                        presence: Presence::Defaulted(DefaultValue::Choice("kill")),
                        domain: ValueDomain::Choice(&["kill"]),
                        latest_site: EvaluationSite::Plan,
                    },
                ],
            },
            ContractMode {
                protocol: Some(crate::connector::contract::ProfileProtocol {
                    id: crate::connector::stdlib::PROCESS_SERVICE_CONNECTOR_ID,
                    roles: ConnectorDirection::InboundAndOutbound,
                    exchange: ExchangeKind::Session,
                }),
                word: "service",
                support: ModeSupport::Available,
                attributes: &[AttributeSpec {
                    name: "max_frame",
                    presence: Presence::Defaulted(DefaultValue::Expression(r#""1MiB""#)),
                    domain: ValueDomain::ByteSize,
                    latest_site: EvaluationSite::Plan,
                }],
            },
        ],
    }),
};

/// Checks the resolved Process mode and its exact protocol restrictions.
pub(super) fn validate(node: &Node, spec: &ConnectorSpec) -> Result<(), PlanValidationError> {
    let service = spec.contract.as_str() == crate::connector::stdlib::PROCESS_SERVICE_CONNECTOR_ID;
    let mode = PROCESS.profile(if service { "service" } else { "job" });
    let roles = mode
        .and_then(|mode| mode.protocol)
        .map_or(PROCESS.roles, |protocol| protocol.roles);
    let exchange = mode
        .and_then(|mode| mode.protocol)
        .map_or(PROCESS.exchange, |protocol| protocol.exchange);
    if spec.roles != roles {
        return Err(PlanValidationError::ContractRoleMismatch {
            node: node.path.clone(),
        });
    }
    if spec.exchange != exchange {
        return Err(PlanValidationError::ContractExchangeMismatch {
            node: node.path.clone(),
        });
    }
    match spec.attribute("mode") {
        Some(crate::plan::AttrSetting::Word(word))
            if word == if service { "service" } else { "job" } => {}
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
    }
    match spec.attribute("on_timeout") {
        None if service => {}
        Some(crate::plan::AttrSetting::Word(word)) if !service && word == "kill" => {}
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("on_timeout"),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "on_timeout",
            });
        }
    }
    match spec.attribute("command") {
        Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(_))) => {}
        Some(crate::plan::AttrSetting::Expr { site, expr })
            if *site == EvaluationSite::Instance && expr.ty == DataType::Text => {}
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("command"),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "command",
            });
        }
    }
    match spec.attribute("args") {
        Some(crate::plan::AttrSetting::Value(crate::value::Value::Array(items)))
            if items
                .iter()
                .all(|item| matches!(item, crate::value::Value::Text(_))) => {}
        Some(crate::plan::AttrSetting::Expr { site, expr })
            if *site == EvaluationSite::Instance && expr.ty == DataType::array(DataType::Text) => {}
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("args"),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: "args",
            });
        }
    }
    if let Some(cwd) = spec.attribute("cwd") {
        let valid = match cwd {
            crate::plan::AttrSetting::Value(crate::value::Value::Text(_)) => true,
            crate::plan::AttrSetting::Expr { site, expr } => {
                *site == EvaluationSite::Instance && expr.ty == DataType::Text
            }
            _ => false,
        };
        if !valid {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("cwd"),
            });
        }
    }
    if let Some(env) = spec.attribute("env") {
        let valid = match env {
            crate::plan::AttrSetting::Value(crate::value::Value::Map(entries)) => entries
                .values()
                .all(|value| matches!(value, crate::value::Value::Text(_))),
            crate::plan::AttrSetting::Expr { site, expr } => {
                *site == EvaluationSite::Instance && expr.ty == DataType::text_map(DataType::Text)
            }
            _ => false,
        };
        if !valid {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("env"),
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
        if !valid || service {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("timeout"),
            });
        }
    }
    let size_attribute = if service { "max_frame" } else { "max_output" };
    match spec.attribute(size_attribute) {
        Some(crate::plan::AttrSetting::Value(crate::value::Value::Text(value)))
            if crate::connector::byte_size(value).is_some_and(|size| !service || size > 0) => {}
        Some(_) => {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from(size_attribute),
            });
        }
        None => {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: size_attribute,
            });
        }
    }
    let codec = match spec.attribute("codec") {
        Some(crate::plan::AttrSetting::Word(word))
            if matches!(word.as_str(), "bytes" | "jsonl" | "lines")
                && !(service && word == "bytes") =>
        {
            word.as_str()
        }
        Some(_) => {
            return Err(PlanValidationError::UnknownCodec {
                node: node.path.clone(),
            });
        }
        None => {
            return Err(PlanValidationError::MissingCodec {
                node: node.path.clone(),
            });
        }
    };
    let err_codec = match spec.attribute("err_codec") {
        Some(crate::plan::AttrSetting::Word(word))
            if matches!(word.as_str(), "bytes" | "lines") && !(service && word == "bytes") =>
        {
            word.as_str()
        }
        Some(_) => {
            return Err(PlanValidationError::UnknownErrCodec {
                node: node.path.clone(),
            });
        }
        None => {
            return Err(PlanValidationError::MissingErrCodec {
                node: node.path.clone(),
            });
        }
    };
    for (name, _) in &spec.attributes {
        if name != "mode" && (name == "behavior" || PROCESS.attribute(mode, name).is_none()) {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: name.clone(),
            });
        }
    }

    for declared in PROCESS.ports {
        let (name, direction) = (declared.name, declared.direction);
        let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
            return Err(PlanValidationError::MissingContractPort {
                node: node.path.clone(),
                port: name,
            });
        };
        let expected_ty = effective_contract_port(
            node,
            node.loop_spec(),
            name,
            crate::connector::stdlib::process_port_type(
                &spec.contract,
                name,
                Some(codec),
                Some(err_codec),
            ),
        );
        if port.direction != direction || port.ty != expected_ty {
            return Err(PlanValidationError::ContractPortMismatch {
                node: node.path.clone(),
                port: port.port.clone(),
            });
        }
    }
    if let Some(port) = node.ports.iter().find(|port| {
        !PROCESS
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
