//! The standard catalogue — concrete instances of the
//! [`crate::contract`] metamodel.
//!
//! Governing rule: [`stdlib.md`] — the ten-case Connector template and
//! the six contracts. Contracts are data: the checker validates
//! declarations against these sheets without linking any adapter, and
//! adapters implement them later. M0 ships the `Process` sheet with
//! its job profile; the rest of the catalogue is declared so its names
//! refuse explicitly rather than accidentally.
//!
//! [`stdlib.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/stdlib.md

use crate::contract::{
    AttributeSpec, ConnectorContract, DefaultValue, Presence, Profile, ProfileSelector,
    ProfileSupport, Registry, ValueDomain,
};

/// Guest identity the manifest pins; M0 declares the languages with
/// no available frontend so their names refuse explicitly. Not a node
/// contract — it stays beside the catalogue, outside the metamodel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestContract {
    /// The guest language tag (`jq`, `sql`).
    pub language: &'static str,
    /// Whether a frontend and executor ship in this build.
    pub available: bool,
}

/// The `Process` sheet of `stdlib.md` §3.3 on the D-2026-08-12-1
/// metamodel: the common attributes, and the `mode` selector picking
/// `job` (default, available) or `service` (known to the contract,
/// deferred by this build — its attribute NAMES are normative, their
/// domains deferred-only).
pub const PROCESS: ConnectorContract = ConnectorContract {
    name: "Process",
    input_ports: &["in"],
    output_ports: &["out", "err"],
    common: &[
        AttributeSpec {
            name: "args",
            presence: Presence::Defaulted(DefaultValue::CelExpression("[]")),
            domain: ValueDomain::TextList,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic", "probabilistic"]),
        },
        AttributeSpec {
            // The framing is the sheet's most semantic decision —
            // bytes, digest, reply type and error surface all follow
            // it — so it is never chosen silently (D-2026-08-11-1,
            // reaffirmed by D-2026-08-12-1).
            name: "codec",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["jsonl", "lines"]),
        },
        AttributeSpec {
            name: "command",
            presence: Presence::Required,
            domain: ValueDomain::Text,
        },
        AttributeSpec {
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent", "compensable", "irreversible"]),
        },
        AttributeSpec {
            // A declared budget; absence means NO budget — a real
            // Optional, not a hidden default.
            name: "timeout",
            presence: Presence::Optional,
            domain: ValueDomain::Duration,
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
                        // parser the adapter enforces the budget
                        // with (D-2026-08-12-1, review round).
                        name: "max_output",
                        presence: Presence::Defaulted(DefaultValue::CelExpression(r#""8MiB""#)),
                        domain: ValueDomain::ByteSize,
                    },
                    AttributeSpec {
                        // Bypassing a detached one-shot is
                        // meaningless: the domain has ONE word, and
                        // the diagnostic says "fixed to".
                        name: "on_timeout",
                        presence: Presence::Defaulted(DefaultValue::Choice("kill")),
                        domain: ValueDomain::Choice(&["kill"]),
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
                        domain: ValueDomain::CelPath,
                    },
                    AttributeSpec {
                        // The restart policy struct (`{ backoff = …,
                        // max_attempts = … }`) — typed with the
                        // service slice.
                        name: "restart",
                        presence: Presence::Optional,
                        domain: ValueDomain::Struct,
                    },
                ],
            },
        ],
    }),
    error_set: &[
        "SpawnFailed",
        "DecodeError",
        "StderrLine",
        "TimeoutExpired",
        "ExecFailed",
        "OutputOverflow",
    ],
};

/// The catalogue names declared by `stdlib.md` whose adapters are not
/// in the M0 build.
const DECLARED: &[&str] = &[
    "HttpServer",
    "HttpClient",
    "Clock",
    "Calendar",
    "Memory",
    "File",
];

impl Registry {
    /// The M0 registry through its fallible door — the same
    /// validation any extension walks.
    ///
    /// # Errors
    ///
    /// The first [`crate::contract::SheetDefect`] of the standard
    /// catalogue.
    pub fn try_m0() -> Result<Self, crate::contract::SheetDefect> {
        let base = Self {
            connectors: Vec::new(),
            declared: DECLARED.to_vec(),
            guests: vec![
                GuestContract {
                    language: "jq",
                    available: false,
                },
                GuestContract {
                    language: "sql",
                    available: false,
                },
            ],
        };
        // The standard catalogue walks through the SAME door as any
        // extension — never an unvalidated authority.
        base.try_with(PROCESS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Availability;

    #[test]
    fn the_registry_resolves_the_three_availabilities() {
        let registry = Registry::try_m0().expect("the standard catalogue holds");
        assert!(matches!(
            registry.connector("Process"),
            Availability::Available(contract) if contract.name == "Process"
        ));
        assert_eq!(registry.connector("HttpServer"), Availability::Declared);
        assert_eq!(registry.connector("Postgres"), Availability::Unknown);
    }

    #[test]
    fn the_process_sheet_holds_its_own_door() {
        // The sheet validates at the registry door — defaults
        // compiled, profiles coherent.
        assert!(crate::contract::validate_sheet(&PROCESS).is_ok());
        let job = PROCESS.profile("job").unwrap();
        let effect = PROCESS.attribute(Some(job), "effect").unwrap();
        assert_eq!(effect.presence, Presence::Required);
        assert!(PROCESS.attribute(Some(job), "max_output").is_some());
        assert!(PROCESS.attribute(Some(job), "restart").is_none());
        assert_eq!(
            PROCESS.owning_profile("restart").map(|p| p.word),
            Some("service")
        );
        assert!(PROCESS.error_set.contains(&"ExecFailed"));
    }

    #[test]
    fn guests_are_declared_but_unavailable() {
        let registry = Registry::try_m0().expect("the standard catalogue holds");
        assert_eq!(
            registry.guest("jq").map(|guest| guest.available),
            Some(false)
        );
        assert_eq!(registry.guest("python"), None);
    }
}
