//! Declarative contracts for standard capabilities.
//!
//! Governing rule: [`stdlib.md`] — the ten-case Connector template and
//! the six contracts. Contracts are data: the checker validates
//! declarations against these sheets without linking any adapter, and
//! adapters implement them later. M0 ships the `Process` sheet with
//! its job mode; the rest of the catalogue is declared so its names
//! refuse explicitly rather than accidentally.
//!
//! [`stdlib.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/stdlib.md

/// How a connector type name resolves against the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability<'registry> {
    /// The contract sheet is present and an adapter ships.
    Available(&'registry ConnectorContract),
    /// The name is in the catalogue; its adapter is not in this build.
    Declared,
    /// The name is not a known connector type.
    Unknown,
}

/// One declarative connector contract sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorContract {
    /// The NodeType name (`Process`).
    pub name: &'static str,
    /// The envelope ports the type defines.
    pub ports: &'static [&'static str],
    /// The attribute sheet.
    pub attributes: &'static [AttributeSpec],
    /// The typed error set; every member reaches the `err` port.
    pub error_set: &'static [&'static str],
}

impl ConnectorContract {
    /// The spec of one attribute, if the sheet declares it.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&AttributeSpec> {
        self.attributes.iter().find(|spec| spec.name == name)
    }
}

/// One attribute of a contract sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSpec {
    /// The attribute key.
    pub name: &'static str,
    /// Whether a declaration must provide it.
    pub required: bool,
    /// The admitted value domain.
    pub domain: AttrDomain,
}

/// The value domain of a contract attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrDomain {
    /// One of a closed set of bare keywords.
    Word(&'static [&'static str]),
    /// A constant text value.
    Text,
    /// A constant list of text values.
    TextList,
    /// A constant duration value.
    Duration,
    /// Present in the sheet but a contract violation in the M0 job
    /// mode, with the sheet's own reason.
    Forbidden(&'static str),
}

/// Guest identity the manifest pins; M0 declares the languages with
/// no available frontend so their names refuse explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestContract {
    /// The guest language tag (`jq`, `sql`).
    pub language: &'static str,
    /// Whether a frontend and executor ship in this build.
    pub available: bool,
}

/// The contract registry a build wires into its checker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    connectors: Vec<ConnectorContract>,
    declared: Vec<&'static str>,
    guests: Vec<GuestContract>,
}

/// The `Process` sheet of `stdlib.md` §3.3, restricted to the M0 job
/// mode: `mode = service` is deferred separately by the checker.
pub const PROCESS: ConnectorContract = ConnectorContract {
    name: "Process",
    ports: &["in", "out", "err"],
    attributes: &[
        AttributeSpec {
            name: "command",
            required: true,
            domain: AttrDomain::Text,
        },
        AttributeSpec {
            name: "args",
            required: false,
            domain: AttrDomain::TextList,
        },
        AttributeSpec {
            name: "mode",
            required: false,
            domain: AttrDomain::Word(&["job", "service"]),
        },
        AttributeSpec {
            name: "codec",
            required: true,
            domain: AttrDomain::Word(&["jsonl", "lines"]),
        },
        AttributeSpec {
            name: "effect",
            required: true,
            domain: AttrDomain::Word(&["idempotent", "compensable", "irreversible"]),
        },
        AttributeSpec {
            name: "behavior",
            required: true,
            domain: AttrDomain::Word(&["deterministic", "probabilistic"]),
        },
        AttributeSpec {
            name: "timeout",
            required: false,
            domain: AttrDomain::Duration,
        },
        AttributeSpec {
            // A size string ("8MiB"); a dedicated size domain can come
            // with the adapter that enforces it.
            name: "max_output",
            required: false,
            domain: AttrDomain::Text,
        },
        AttributeSpec {
            name: "correlate",
            required: false,
            domain: AttrDomain::Forbidden("the process boundary is the correlation on a job"),
        },
        AttributeSpec {
            name: "restart",
            required: false,
            domain: AttrDomain::Forbidden(
                "re-running a one-shot is a graph decision, never a supervisor reflex",
            ),
        },
        AttributeSpec {
            name: "on_timeout",
            required: false,
            domain: AttrDomain::Forbidden("on_timeout is forced to kill on a job"),
        },
    ],
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
    /// The M0 registry: the `Process` sheet, the declared catalogue,
    /// and the guest languages with no available frontend.
    #[must_use]
    pub fn m0() -> Self {
        Self {
            connectors: vec![PROCESS],
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
        }
    }

    /// Resolves a connector type name.
    #[must_use]
    pub fn connector(&self, name: &str) -> Availability<'_> {
        if let Some(contract) = self
            .connectors
            .iter()
            .find(|contract| contract.name == name)
        {
            return Availability::Available(contract);
        }
        if self.declared.contains(&name) {
            return Availability::Declared;
        }
        Availability::Unknown
    }

    /// The guest contract for a language tag, if declared.
    #[must_use]
    pub fn guest(&self, language: &str) -> Option<&GuestContract> {
        self.guests.iter().find(|guest| guest.language == language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_resolves_the_three_availabilities() {
        let registry = Registry::m0();
        assert!(matches!(
            registry.connector("Process"),
            Availability::Available(contract) if contract.name == "Process"
        ));
        assert_eq!(registry.connector("HttpServer"), Availability::Declared);
        assert_eq!(registry.connector("Postgres"), Availability::Unknown);
    }

    #[test]
    fn the_process_sheet_pins_the_mandatory_axes() {
        let effect = PROCESS.attribute("effect").unwrap();
        assert!(effect.required);
        let behavior = PROCESS.attribute("behavior").unwrap();
        assert!(behavior.required);
        assert!(PROCESS.attribute("correlate").is_some());
        assert!(PROCESS.attribute("listen").is_none());
        assert!(PROCESS.error_set.contains(&"ExecFailed"));
    }

    #[test]
    fn guests_are_declared_but_unavailable() {
        let registry = Registry::m0();
        assert_eq!(
            registry.guest("jq").map(|guest| guest.available),
            Some(false)
        );
        assert_eq!(registry.guest("python"), None);
    }
}
