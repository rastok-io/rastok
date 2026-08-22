//! The standard catalogue — concrete instances of the
//! [`crate::contract`] metamodel.
//!
//! Governing rule: [`stdlib.md`] — the ten-case Connector template and
//! the six contracts. Contracts are data: the checker validates
//! declarations against these sheets without linking any Driver, and
//! Drivers implement them later. M0 ships the `Process` sheet with
//! its job profile; the rest of the catalogue is declared so its names
//! refuse explicitly rather than accidentally.
//!
//! [`stdlib.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/stdlib.md

use crate::connector::{ConnectorRoles, PROCESS_CONNECTOR_ID};
use crate::contract::{
    AttributeSpec, ConnectorContract, ContractCatalog, DefaultValue, EvaluationSite,
    FrontendContract, FrontendFraming, FrontendReader, PortContract, PortDataType, PortTypeChoice,
    PortTypeRule, Presence, Profile, ProfileSelector, ProfileSupport, ValueDomain,
};
use crate::node::Direction;
use crate::plan::UnwiredPolicy;

/// The `Process` sheet of `stdlib.md` §3.3 on the D-2026-08-12-1
/// metamodel: the common attributes, and the `mode` selector picking
/// `job` (default, available) or `service` (known to the contract,
/// deferred by this build — its attribute NAMES are normative, their
/// domains deferred-only).
pub const PROCESS: ConnectorContract = ConnectorContract {
    id: PROCESS_CONNECTOR_ID,
    name: "Process",
    roles: ConnectorRoles::Outbound,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
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
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
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
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
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
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            // One exact union is shared by the contract, v7 port table,
            // completion gate and Process Driver.
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
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent", "compensable", "irreversible"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // A declared budget; absence means NO budget — a real
            // Optional, not a hidden default.
            name: "timeout",
            presence: Presence::Optional,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "env",
            presence: Presence::Optional,
            domain: ValueDomain::TextMap,
            latest_site: EvaluationSite::Instance,
        },
    ],
    selector: Some(ProfileSelector {
        attribute: "mode",
        default: Some("job"),
        profiles: &[
            Profile {
                word: "job",
                support: ProfileSupport::Available,
                attributes: &[
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
            Profile {
                word: "service",
                support: ProfileSupport::Deferred("the Process service mode"),
                attributes: &[
                    AttributeSpec {
                        // `correlate = data.id` — an expression over
                        // runtime data, not a boot-phase constant.
                        name: "correlate",
                        presence: Presence::Optional,
                        domain: ValueDomain::ExprPath,
                        latest_site: EvaluationSite::Invocation,
                    },
                    AttributeSpec {
                        // The restart policy struct (`{ backoff = …,
                        // max_attempts = … }`) — typed with the
                        // service slice.
                        name: "restart",
                        presence: Presence::Optional,
                        domain: ValueDomain::Struct,
                        latest_site: EvaluationSite::Instance,
                    },
                ],
            },
        ],
    }),
};

/// The catalogue names declared by `stdlib.md` whose Drivers are not
/// in the M0 build.
const DECLARED: &[&str] = &[
    "HttpServer",
    "HttpClient",
    "Clock",
    "Calendar",
    "Memory",
    "File",
];

const M0_FRONTENDS: &[FrontendContract] = &[
    FrontendContract {
        name: "json",
        reader: FrontendReader::Json,
    },
    FrontendContract {
        name: "text",
        reader: FrontendReader::Text,
    },
    FrontendContract {
        name: "base64",
        reader: FrontendReader::Base64,
    },
    FrontendContract {
        name: "jq",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the jq guest frontend",
        },
    },
    FrontendContract {
        name: "sql",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the SQL guest frontend",
        },
    },
    FrontendContract {
        name: "bash",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the bash guest frontend",
        },
    },
    FrontendContract {
        name: "python",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Python guest frontend",
        },
    },
    FrontendContract {
        name: "rust",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Rust guest frontend",
        },
    },
    FrontendContract {
        name: "avro",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Avro data frontend",
        },
    },
    FrontendContract {
        name: "protobuf",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Protobuf data frontend",
        },
    },
    FrontendContract {
        name: "parquet",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Parquet data frontend",
        },
    },
    FrontendContract {
        name: "msgpack",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the MessagePack data frontend",
        },
    },
    FrontendContract {
        name: "csv",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the CSV data frontend",
        },
    },
];

impl ContractCatalog {
    /// The M0 registry through its fallible door — the same
    /// validation any extension walks.
    ///
    /// # Errors
    ///
    /// The first [`crate::contract::SheetDefect`] of the standard
    /// catalogue.
    pub fn try_m0() -> Result<Self, crate::contract::SheetDefect> {
        let base = Self {
            connectors: std::collections::BTreeMap::new(),
            declared: DECLARED.to_vec(),
            frontends: Vec::new(),
        };
        // The standard catalogue walks through the SAME door as any
        // extension — never an unvalidated authority.
        M0_FRONTENDS
            .iter()
            .cloned()
            .try_fold(base, Self::try_with_frontend)?
            .try_with(PROCESS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Availability;

    #[test]
    fn the_catalogue_resolves_the_three_availabilities() {
        let catalogue = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert!(matches!(
            catalogue.connector("Process"),
            Availability::Available(contract) if contract.name == "Process"
        ));
        assert_eq!(catalogue.connector("HttpServer"), Availability::Declared);
        assert_eq!(catalogue.connector("Postgres"), Availability::Unknown);
    }

    #[test]
    fn the_process_sheet_holds_its_own_door() {
        // The sheet validates at the registry door — defaults
        // compiled, profiles coherent.
        assert!(crate::contract::validate_sheet(&PROCESS).is_ok());
        let process = PROCESS;
        let job = process.profile("job").unwrap();
        let effect = process.attribute(Some(job), "effect").unwrap();
        assert_eq!(effect.presence, Presence::Required);
        assert!(process.attribute(Some(job), "max_output").is_some());
        assert!(process.attribute(Some(job), "restart").is_none());
        assert_eq!(
            process.owning_profile("restart").map(|p| p.word),
            Some("service")
        );
        let ports = PROCESS
            .resolve_ports(&std::collections::BTreeMap::from([
                (String::from("codec"), String::from("lines")),
                (String::from("err_codec"), String::from("lines")),
            ]))
            .expect("the effective Process sheet resolves");
        assert_eq!(ports.len(), 4);
        let process_exit = crate::boundary::process_exit_type().expect("the target type is valid");
        assert!(
            ports
                .iter()
                .any(|port| port.port.name() == "exit" && port.ty.as_ref() == Some(&process_exit))
        );
    }

    #[test]
    fn frontends_share_one_contract_registry() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert_eq!(
            registry.frontend("json").map(|frontend| frontend.reader),
            Some(FrontendReader::Json)
        );
        assert!(matches!(
            registry.frontend("jq").map(|frontend| frontend.reader),
            Some(FrontendReader::Deferred { .. })
        ));
        assert_eq!(
            registry
                .frontend("text")
                .map(|frontend| frontend.reader.result_type()),
            Some(Some(crate::datatype::DataType::Text))
        );
        assert_eq!(registry.frontend("starlark"), None);
    }
}
