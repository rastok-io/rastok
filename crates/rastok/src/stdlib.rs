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
    /// The contract sheet is present in THIS registry — the checker
    /// can resolve declarations against it. Whether an adapter ships
    /// is the build's wiring (the Lab mocks need none); the sheet
    /// claims the contract, not the implementation.
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
    /// The envelope ports the type defines as inputs.
    pub input_ports: &'static [&'static str],
    /// The envelope ports the type defines as outputs.
    pub output_ports: &'static [&'static str],
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

/// Why a contract sheet cannot enter the registry — the sheet is an
/// AUTHORITY once admitted (the checker materializes port tables
/// from it without re-checking), so it is held at the door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetDefect {
    /// A port name the language refuses.
    InvalidPortName(&'static str),
    /// One name in both direction lists, or listed twice.
    ConflictingPort(&'static str),
    /// A default port listed against its implicit direction.
    MisdirectedDefault(&'static str),
    /// An attribute declared twice.
    DuplicateAttribute(&'static str),
    /// An error kind declared twice.
    DuplicateErrorKind(&'static str),
}

impl core::fmt::Display for SheetDefect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPortName(name) => write!(f, "`{name}` is not a valid port name"),
            Self::ConflictingPort(name) => {
                write!(f, "`{name}` is listed twice or in both directions")
            }
            Self::MisdirectedDefault(name) => {
                write!(f, "`{name}` is a default port listed against its direction")
            }
            Self::DuplicateAttribute(name) => write!(f, "`{name}` is declared twice"),
            Self::DuplicateErrorKind(name) => write!(f, "`{name}` is declared twice"),
        }
    }
}

impl std::error::Error for SheetDefect {}

/// Holds a sheet to the rules the checker will assume forever after.
fn validate_sheet(contract: &ConnectorContract) -> Result<(), SheetDefect> {
    let mut seen = std::collections::BTreeSet::new();
    for (name, direction) in contract
        .input_ports
        .iter()
        .map(|name| (*name, crate::node::Direction::Input))
        .chain(
            contract
                .output_ports
                .iter()
                .map(|name| (*name, crate::node::Direction::Output)),
        )
    {
        if crate::node::Port::new(name.to_string()).is_err() {
            return Err(SheetDefect::InvalidPortName(name));
        }
        if !seen.insert(name) {
            return Err(SheetDefect::ConflictingPort(name));
        }
        let implicit = match name {
            "in" => Some(crate::node::Direction::Input),
            "out" | "err" => Some(crate::node::Direction::Output),
            _ => None,
        };
        if let Some(expected) = implicit
            && expected != direction
        {
            return Err(SheetDefect::MisdirectedDefault(name));
        }
    }
    let mut names = std::collections::BTreeSet::new();
    for spec in contract.attributes {
        if !names.insert(spec.name) {
            return Err(SheetDefect::DuplicateAttribute(spec.name));
        }
    }
    let mut kinds = std::collections::BTreeSet::new();
    for kind in contract.error_set {
        if !kinds.insert(*kind) {
            return Err(SheetDefect::DuplicateErrorKind(kind));
        }
    }
    Ok(())
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
    input_ports: &["in"],
    output_ports: &["out", "err"],
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

    /// Extends the registry with one more contract sheet — the seam
    /// every M1 connector lands through (`File`, the HTTP pair), and
    /// what a test contract uses to exercise the transport of
    /// resolved port tables. FALLIBLE: a sheet is validated before
    /// it becomes an authority — the checker materializes tables
    /// from it and never re-checks. A name already present is
    /// replaced.
    ///
    /// # Errors
    ///
    /// The first [`SheetDefect`], named.
    pub fn try_with(mut self, contract: ConnectorContract) -> Result<Self, SheetDefect> {
        validate_sheet(&contract)?;
        self.connectors
            .retain(|existing| existing.name != contract.name);
        self.declared.retain(|declared| *declared != contract.name);
        self.connectors.push(contract);
        Ok(self)
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
