//! Checked Connector declarations and the compiler catalogue.
//!
//! Portable declaration types come from the core. Hosted validation checks source
//! names and default expressions before admitting a declaration to the catalogue.

use crate::{contract::FrontendContract, expr};
pub use rastok_core::connector::contract::*;
use std::collections::BTreeMap;

/// How a connector type name resolves against the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability<'registry> {
    /// The contract sheet is present in THIS registry — the checker
    /// can resolve declarations against it. Whether a Driver ships
    /// is the build's wiring (the Lab mocks need none); the sheet
    /// claims the contract, not the implementation.
    Available(&'registry ConnectorContract),
    /// The name is declared, but no executable Contract declaration is available.
    Declared,
    /// The name is not a known connector type.
    Unknown,
}

/// A Connector declaration failed catalogue validation.
///
/// The compiler uses admitted declarations to resolve ports and defaults
/// without repeating these checks. These errors identify an invalid Rust
/// declaration, not an error in the author's RKG source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractValidationError {
    /// The stable contract identity is not canonical.
    InvalidConnectorId {
        /// Exact spelling supplied by the sheet.
        id: &'static str,
        /// Canonical grammar category that refused it.
        reason: crate::connector::ContractIdError,
    },
    /// Two source symbols claim one stable semantic identity.
    ConnectorIdCollision {
        /// Canonical id claimed by both sheets.
        id: &'static str,
        /// Sheet already admitted under that id.
        existing: &'static str,
        /// Different sheet attempting to reuse it.
        incoming: &'static str,
    },
    /// A port name the language refuses.
    InvalidPortName(&'static str),
    /// One name in both direction lists, or listed twice.
    ConflictingPort(&'static str),
    /// A default port listed against its implicit direction.
    MisdirectedDefault(&'static str),
    /// The conventional terminal row contradicts the exchange lifecycle:
    /// Event has none; Call and Session have exactly one output `exit`.
    ExchangeTerminalMismatch {
        /// Exchange declared by the sheet.
        exchange: crate::connector::ExchangeKind,
        /// Number of conventional output terminal rows found.
        exits: usize,
    },
    /// The role set cannot carry this exchange's complete generic protocol:
    /// Event is exactly inbound, while Session needs the outbound view that
    /// reports open/write/terminal observations.
    ExchangeRoleMismatch {
        /// Exchange declared by the sheet.
        exchange: crate::connector::ExchangeKind,
        /// Incompatible Driver roles declared by the sheet.
        roles: crate::connector::ConnectorDirection,
    },
    /// A same-as rule names no port on the sheet.
    UnknownPortDependency {
        /// Dependent port.
        port: &'static str,
        /// Missing source port.
        dependency: &'static str,
    },
    /// Same-as rules form a cycle.
    CyclicPortTypes(&'static str),
    /// An attribute-selected rule names no closed choice attribute.
    UnknownPortTypeAttribute {
        /// Port carrying the rule.
        port: &'static str,
        /// Missing or non-choice attribute.
        attribute: &'static str,
    },
    /// A port-type selector was allowed to bind after plan construction.
    LatePortTypeAttribute {
        /// Port carrying the rule.
        port: &'static str,
        /// Attribute that must be plan-resolved.
        attribute: &'static str,
    },
    /// A selected port rule repeats or omits a choice arm.
    PortTypeChoiceMismatch {
        /// Port carrying the rule.
        port: &'static str,
        /// Selecting attribute.
        attribute: &'static str,
    },
    /// A fixed/selected port type violates the canonical type algebra.
    InvalidPortType {
        /// Port carrying the rule.
        port: &'static str,
        /// Canonical type refusal.
        reason: String,
    },
    /// An attribute declared twice within one scope (the common
    /// sheet, or one profile), or in both the common sheet and a
    /// profile.
    DuplicateAttribute(&'static str),
    /// Two profiles carry one word.
    DuplicateProfile(&'static str),
    /// The selector's default names no profile.
    UnknownDefaultProfile(&'static str),
    /// The selector attribute collides with a declared attribute.
    SelectorCollision(&'static str),
    /// A `Choice` domain lists one word twice.
    DuplicateChoice(&'static str),
    /// A `Choice` domain with no word at all — an attribute nobody
    /// could ever write.
    EmptyChoice(&'static str),
    /// A selector with no profile — nothing to select.
    NoProfiles(&'static str),
    /// A sheet name (port, attribute, selector, profile word,
    /// choice word, or error kind) the DSL cannot express.
    InexpressibleName(&'static str),
    /// A deferred-only domain sits in an AVAILABLE profile or the
    /// common sheet.
    UnimplementedDomain(&'static str),
    /// A default that does not compile, reduce, or fit its domain.
    InvalidDefault {
        /// The defaulted attribute.
        attribute: &'static str,
        /// What went wrong.
        reason: String,
    },
}

impl core::fmt::Display for ContractValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConnectorId { id, reason } => {
                write!(f, "connector id `{id}` is invalid: {reason}")
            }
            Self::ConnectorIdCollision {
                id,
                existing,
                incoming,
            } => write!(
                f,
                "connector id `{id}` is claimed by both `{existing}` and `{incoming}`"
            ),
            Self::InvalidPortName(name) => write!(f, "`{name}` is not a valid port name"),
            Self::ConflictingPort(name) => {
                write!(f, "`{name}` is listed twice or in both directions")
            }
            Self::MisdirectedDefault(name) => {
                write!(f, "`{name}` is a default port listed against its direction")
            }
            Self::ExchangeTerminalMismatch { exchange, exits } => write!(
                f,
                "the {exchange} exchange declares {exits} terminal `exit` outputs"
            ),
            Self::ExchangeRoleMismatch { exchange, roles } => {
                write!(f, "the {exchange} exchange cannot use {roles} Driver roles")
            }
            Self::UnknownPortDependency { port, dependency } => write!(
                f,
                "port `{port}` derives its type from unknown port `{dependency}`"
            ),
            Self::CyclicPortTypes(port) => {
                write!(f, "port `{port}` participates in a same-as type cycle")
            }
            Self::UnknownPortTypeAttribute { port, attribute } => write!(
                f,
                "port `{port}` selects its type from unknown/non-choice attribute `{attribute}`"
            ),
            Self::LatePortTypeAttribute { port, attribute } => write!(
                f,
                "port `{port}` selects its type from `{attribute}`, which must have `plan` as its latest evaluation site"
            ),
            Self::PortTypeChoiceMismatch { port, attribute } => write!(
                f,
                "port `{port}` does not map every `{attribute}` choice exactly once"
            ),
            Self::InvalidPortType { port, reason } => {
                write!(f, "port `{port}` has an invalid logical type: {reason}")
            }
            Self::DuplicateAttribute(name) => write!(f, "`{name}` is declared twice"),
            Self::DuplicateProfile(word) => write!(f, "profile `{word}` is declared twice"),
            Self::UnknownDefaultProfile(word) => {
                write!(
                    f,
                    "the selector defaults to `{word}`, which names no profile"
                )
            }
            Self::SelectorCollision(name) => {
                write!(f, "`{name}` is both the selector and a declared attribute")
            }
            Self::DuplicateChoice(name) => {
                write!(f, "`{name}`'s choice domain lists one word twice")
            }
            Self::EmptyChoice(name) => {
                write!(
                    f,
                    "`{name}`'s choice domain is empty — nothing could satisfy it"
                )
            }
            Self::NoProfiles(attribute) => {
                write!(f, "the `{attribute}` selector has no profile to select")
            }
            Self::InexpressibleName(name) => {
                write!(
                    f,
                    "`{name}` is not expressible in the DSL's identifier grammar"
                )
            }
            Self::UnimplementedDomain(name) => write!(
                f,
                "`{name}` carries a deferred-only domain outside a deferred profile"
            ),
            Self::InvalidDefault { attribute, reason } => {
                write!(f, "`{attribute}`'s default is invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for ContractValidationError {}

/// Whether a sheet-declared name is expressible in the DSL — the
/// identifier grammar the sheet's ports, attributes, selector,
/// profile words and choice words must all satisfy: an
/// admitted sheet is an authority, so nothing inexpressible enters.
pub(crate) fn expressible_name(text: &str) -> bool {
    // `nothing` is excluded with the literals: the parser turns it
    // into the no-emission sentinel, so it can never reach an
    // `AttrValue::Word`. (Finer per-position grammars — contract
    // name vs attribute vs choice word — come with real cases.)
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !text.as_bytes()[0].is_ascii_digit()
        && !matches!(text, "true" | "false" | "null" | "nothing")
}

/// Judges one default against its spec — compiling a Rastok expression default
/// through the real chain (closed signature, no consts: a standard
/// contract reads nothing).
fn validate_default(spec: &AttributeSpec) -> Result<(), ContractValidationError> {
    let Presence::Defaulted(default) = &spec.presence else {
        return Ok(());
    };
    match default {
        DefaultValue::Choice(word) => match &spec.domain {
            ValueDomain::Choice(words) if words.contains(word) => Ok(()),
            _ => Err(ContractValidationError::InvalidDefault {
                attribute: spec.name,
                reason: format!("`{word}` is not a word of the attribute's domain"),
            }),
        },
        DefaultValue::Expression(source) => {
            let tree = expr::compile(source, &expr::Signature::new()).map_err(|_| {
                ContractValidationError::InvalidDefault {
                    attribute: spec.name,
                    reason: format!("`{source}` does not compile as closed Rastok expression"),
                }
            })?;
            let value = expr::const_eval(&tree, &BTreeMap::new()).map_err(|error| {
                ContractValidationError::InvalidDefault {
                    attribute: spec.name,
                    reason: format!("`{source}` is not constant: {}", error.message),
                }
            })?;
            if spec.domain.admits(&value) {
                Ok(())
            } else {
                Err(ContractValidationError::InvalidDefault {
                    attribute: spec.name,
                    reason: format!("`{source}` reduces outside the {:?} domain", spec.domain),
                })
            }
        }
    }
}

fn validate_attributes(
    specs: &[AttributeSpec],
    seen: &mut std::collections::BTreeSet<&'static str>,
    deferred_profile: bool,
) -> Result<(), ContractValidationError> {
    let mut local = std::collections::BTreeSet::new();
    for spec in specs {
        if !expressible_name(spec.name) {
            return Err(ContractValidationError::InexpressibleName(spec.name));
        }
        if !local.insert(spec.name) || seen.contains(spec.name) {
            return Err(ContractValidationError::DuplicateAttribute(spec.name));
        }
        if let ValueDomain::Choice(words) = &spec.domain {
            if words.is_empty() {
                return Err(ContractValidationError::EmptyChoice(spec.name));
            }
            let mut unique = std::collections::BTreeSet::new();
            for word in *words {
                if !expressible_name(word) {
                    return Err(ContractValidationError::InexpressibleName(word));
                }
                if !unique.insert(*word) {
                    return Err(ContractValidationError::DuplicateChoice(spec.name));
                }
            }
        }
        if let ValueDomain::ChoiceList(words) = &spec.domain {
            if words.is_empty() {
                return Err(ContractValidationError::EmptyChoice(spec.name));
            }
            let mut unique = std::collections::BTreeSet::new();
            for word in *words {
                if !unique.insert(*word) {
                    return Err(ContractValidationError::DuplicateChoice(spec.name));
                }
            }
        }
        if spec.domain.implemented() {
            validate_default(spec)?;
        } else {
            if !deferred_profile {
                return Err(ContractValidationError::UnimplementedDomain(spec.name));
            }
            // A domain nobody can judge carries no default — a
            // default that cannot be validated is a lie waiting.
            if matches!(spec.presence, Presence::Defaulted(_)) {
                return Err(ContractValidationError::InvalidDefault {
                    attribute: spec.name,
                    reason: String::from("a deferred-only domain cannot carry a default"),
                });
            }
        }
    }
    Ok(())
}

/// Holds a sheet to the rules the checker will assume forever after.
pub fn validate_sheet(contract: &ConnectorContract) -> Result<(), ContractValidationError> {
    crate::connector::ContractId::parse(contract.id).map_err(|reason| {
        ContractValidationError::InvalidConnectorId {
            id: contract.id,
            reason,
        }
    })?;
    if !expressible_name(contract.name) {
        return Err(ContractValidationError::InexpressibleName(contract.name));
    }
    let mut seen = std::collections::BTreeSet::new();
    for port in contract.ports {
        let name = port.name;
        let direction = port.direction;
        if !expressible_name(name) || crate::node::Port::new(name.to_string()).is_err() {
            return Err(ContractValidationError::InvalidPortName(name));
        }
        if !seen.insert(name) {
            return Err(ContractValidationError::ConflictingPort(name));
        }
        let implicit = match name {
            "in" => Some(crate::node::PortDirection::Input),
            "out" | "err" | "exit" => Some(crate::node::PortDirection::Output),
            _ => None,
        };
        if let Some(expected) = implicit
            && expected != direction
        {
            return Err(ContractValidationError::MisdirectedDefault(name));
        }
    }
    let mut names = std::collections::BTreeSet::new();
    validate_attributes(contract.common, &mut names, false)?;
    for spec in contract.common {
        names.insert(spec.name);
    }
    if let Some(selector) = &contract.selector {
        if !expressible_name(selector.attribute) {
            return Err(ContractValidationError::InexpressibleName(
                selector.attribute,
            ));
        }
        if selector.profiles.is_empty() {
            return Err(ContractValidationError::NoProfiles(selector.attribute));
        }
        if names.contains(selector.attribute) {
            return Err(ContractValidationError::SelectorCollision(
                selector.attribute,
            ));
        }
        let mut words = std::collections::BTreeSet::new();
        let mut identities = std::collections::BTreeSet::from([contract.id]);
        for profile in selector.profiles {
            if let Some(protocol) = profile.protocol {
                if !identities.insert(protocol.id) {
                    return Err(ContractValidationError::ConnectorIdCollision {
                        id: protocol.id,
                        existing: contract.name,
                        incoming: profile.word,
                    });
                }
                crate::connector::ContractId::parse(protocol.id).map_err(|reason| {
                    ContractValidationError::InvalidConnectorId {
                        id: protocol.id,
                        reason,
                    }
                })?;
                let exits = contract
                    .ports
                    .iter()
                    .filter(|p| {
                        p.name == "exit" && p.direction == crate::node::PortDirection::Output
                    })
                    .count();
                if (protocol.exchange == crate::connector::ExchangeKind::Event && exits != 0)
                    || (protocol.exchange != crate::connector::ExchangeKind::Event && exits != 1)
                {
                    return Err(ContractValidationError::ExchangeTerminalMismatch {
                        exchange: protocol.exchange,
                        exits,
                    });
                }
                if (protocol.exchange == crate::connector::ExchangeKind::Event
                    && protocol.roles != crate::connector::ConnectorDirection::Inbound)
                    || (protocol.exchange == crate::connector::ExchangeKind::Session
                        && !protocol.roles.has_outbound())
                {
                    return Err(ContractValidationError::ExchangeRoleMismatch {
                        exchange: protocol.exchange,
                        roles: protocol.roles,
                    });
                }
            }
            if !expressible_name(profile.word) {
                return Err(ContractValidationError::InexpressibleName(profile.word));
            }
            if !words.insert(profile.word) {
                return Err(ContractValidationError::DuplicateProfile(profile.word));
            }
            if profile
                .attributes
                .iter()
                .any(|spec| spec.name == selector.attribute)
            {
                return Err(ContractValidationError::SelectorCollision(
                    selector.attribute,
                ));
            }
            // Common-vs-profile collisions refuse; two PROFILES may
            // each own a name (none does today).
            let mut shared = names.clone();
            validate_attributes(
                profile.attributes,
                &mut shared,
                matches!(profile.support, ModeSupport::Deferred(_)),
            )?;
        }
        if let Some(default) = selector.default
            && !words.contains(default)
        {
            return Err(ContractValidationError::UnknownDefaultProfile(default));
        }
    }
    // A port type may depend on one common closed choice, or on the profile
    // selector itself. Profile-local attributes are deliberately excluded:
    // sibling profiles may reuse one attribute name with different domains,
    // so that spelling alone cannot name one deterministic type rule.
    let choice_domain = |attribute: &str| -> Option<Vec<&'static str>> {
        contract
            .common
            .iter()
            .find_map(|spec| {
                (spec.name == attribute)
                    .then_some(&spec.domain)
                    .and_then(|domain| match domain {
                        ValueDomain::Choice(words) => Some(words.to_vec()),
                        _ => None,
                    })
            })
            .or_else(|| {
                contract.selector.as_ref().and_then(|selector| {
                    (selector.attribute == attribute).then_some(
                        selector
                            .profiles
                            .iter()
                            .map(|profile| profile.word)
                            .collect::<Vec<_>>(),
                    )
                })
            })
    };
    for port in contract.ports {
        match port.ty {
            PortTypeRule::Fixed(ty) => {
                let resolved =
                    ty.resolve()
                        .map_err(|reason| ContractValidationError::InvalidPortType {
                            port: port.name,
                            reason: reason.to_string(),
                        })?;
                resolved.check_bounds().map_err(|reason| {
                    ContractValidationError::InvalidPortType {
                        port: port.name,
                        reason: reason.to_string(),
                    }
                })?;
            }
            PortTypeRule::SameAs(dependency) => {
                if !contract
                    .ports
                    .iter()
                    .any(|candidate| candidate.name == dependency)
                {
                    return Err(ContractValidationError::UnknownPortDependency {
                        port: port.name,
                        dependency,
                    });
                }
                let mut visited = std::collections::BTreeSet::new();
                let mut next = port.name;
                loop {
                    if !visited.insert(next) {
                        return Err(ContractValidationError::CyclicPortTypes(port.name));
                    }
                    let Some(candidate) = contract.ports.iter().find(|item| item.name == next)
                    else {
                        break;
                    };
                    match candidate.ty {
                        PortTypeRule::SameAs(other) => next = other,
                        _ => break,
                    }
                }
            }
            PortTypeRule::AttributeSelected { attribute, choices }
            | PortTypeRule::JsonOutput { attribute, choices } => {
                if matches!(port.ty, PortTypeRule::JsonOutput { .. })
                    && (port.direction != crate::node::PortDirection::Output
                        || !choices.iter().any(|choice| {
                            choice.word == "json" && choice.ty == PortDataType::Variant
                        }))
                {
                    return Err(ContractValidationError::PortTypeChoiceMismatch {
                        port: port.name,
                        attribute,
                    });
                }
                let Some(words) = choice_domain(attribute) else {
                    return Err(ContractValidationError::UnknownPortTypeAttribute {
                        port: port.name,
                        attribute,
                    });
                };
                if contract
                    .common
                    .iter()
                    .find(|spec| spec.name == attribute)
                    .is_some_and(|spec| spec.latest_site != EvaluationSite::Plan)
                {
                    return Err(ContractValidationError::LatePortTypeAttribute {
                        port: port.name,
                        attribute,
                    });
                }
                let mut arms = std::collections::BTreeSet::new();
                for choice in choices {
                    if !arms.insert(choice.word) || !words.contains(&choice.word) {
                        return Err(ContractValidationError::PortTypeChoiceMismatch {
                            port: port.name,
                            attribute,
                        });
                    }
                    let resolved = choice.ty.resolve().map_err(|reason| {
                        ContractValidationError::InvalidPortType {
                            port: port.name,
                            reason: reason.to_string(),
                        }
                    })?;
                    resolved.check_bounds().map_err(|reason| {
                        ContractValidationError::InvalidPortType {
                            port: port.name,
                            reason: reason.to_string(),
                        }
                    })?;
                }
                if words.iter().any(|word| !arms.contains(word)) {
                    return Err(ContractValidationError::PortTypeChoiceMismatch {
                        port: port.name,
                        attribute,
                    });
                }
            }
        }
    }
    let exits = contract
        .ports
        .iter()
        .filter(|port| port.name == "exit" && port.direction == crate::node::PortDirection::Output)
        .count();
    let terminal_shape_holds = match contract.exchange {
        crate::connector::ExchangeKind::Event => exits == 0,
        crate::connector::ExchangeKind::Call | crate::connector::ExchangeKind::Session => {
            exits == 1
        }
    };
    if !terminal_shape_holds {
        return Err(ContractValidationError::ExchangeTerminalMismatch {
            exchange: contract.exchange,
            exits,
        });
    }
    let roles_hold = match contract.exchange {
        crate::connector::ExchangeKind::Event => {
            contract.roles == crate::connector::ConnectorDirection::Inbound
        }
        crate::connector::ExchangeKind::Call => true,
        crate::connector::ExchangeKind::Session => contract.roles.has_outbound(),
    };
    if !roles_hold {
        return Err(ContractValidationError::ExchangeRoleMismatch {
            exchange: contract.exchange,
            roles: contract.roles,
        });
    }
    Ok(())
}

/// The contract catalogue a build wires into its checker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractCatalog {
    pub(crate) connectors: BTreeMap<crate::connector::ContractId, ConnectorContract>,
    pub(crate) declared: Vec<&'static str>,
    pub(crate) frontends: Vec<FrontendContract>,
}

impl ContractCatalog {
    /// Extends the catalogue with one more contract sheet — the seam
    /// every M1 connector lands through (`File`, the HTTP pair), and
    /// what a test contract uses to exercise the transport of
    /// resolved port tables. FALLIBLE: a sheet is validated before
    /// it becomes an authority — the checker materializes tables and
    /// defaults from it and never re-checks. A source name already present
    /// is replaced, but a second source name may never alias the same stable
    /// identity.
    ///
    /// # Errors
    ///
    /// The first [`ContractValidationError`], named.
    pub fn try_with(
        mut self,
        contract: ConnectorContract,
    ) -> Result<Self, ContractValidationError> {
        validate_sheet(&contract)?;
        let id = crate::connector::ContractId::parse(contract.id).map_err(|reason| {
            ContractValidationError::InvalidConnectorId {
                id: contract.id,
                reason,
            }
        })?;
        for protocol_id in contract.protocol_ids() {
            if let Some(existing) = self.connectors.values().find(|existing| {
                existing.name != contract.name
                    && existing.protocol_ids().any(|id| id == protocol_id)
            }) {
                return Err(ContractValidationError::ConnectorIdCollision {
                    id: protocol_id,
                    existing: existing.name,
                    incoming: contract.name,
                });
            }
        }
        self.connectors
            .retain(|_, existing| existing.name != contract.name);
        self.declared.retain(|declared| *declared != contract.name);
        self.connectors.insert(id, contract);
        Ok(self)
    }

    /// Checked declarations in deterministic Contract-id order.
    pub fn connectors(&self) -> impl Iterator<Item = &ConnectorContract> {
        self.connectors.values()
    }

    /// Names declared without an executable Contract, in catalogue order.
    pub fn declared_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.declared.iter().copied()
    }

    /// Resolves a connector type name.
    #[must_use]
    pub fn connector(&self, name: &str) -> Availability<'_> {
        if let Some(contract) = self
            .connectors
            .values()
            .find(|contract| contract.name == name)
        {
            return Availability::Available(contract);
        }
        if self.declared.contains(&name) {
            return Availability::Declared;
        }
        Availability::Unknown
    }

    /// Resolves the exact stable identity carried by a categorical plan.
    #[must_use]
    pub fn connector_by_id(&self, id: &crate::connector::ContractId) -> Option<&ConnectorContract> {
        self.connectors.get(id).or_else(|| {
            self.connectors.values().find(|contract| {
                contract.selector.as_ref().is_some_and(|selector| {
                    selector.profiles.iter().any(|profile| {
                        profile
                            .protocol
                            .is_some_and(|protocol| protocol.id == id.as_str())
                    })
                })
            })
        })
    }
}
