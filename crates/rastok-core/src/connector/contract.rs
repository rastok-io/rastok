//! Declarative Connector Contracts shared by the compiler and plan validator.
//!
//! A definition specifies ports, attributes and modes independently of a
//! physical Driver. These values are not serialized: plans carry the resolved
//! facts, which the no_std validator checks against the same definitions.

use super::byte_size;
pub use crate::plan::EvaluationSite;
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

/// One closed logical type used by a real port contract today.
///
/// The enum deliberately grows only when a shipped contract pays for another
/// constructor.  It is not a second type algebra: every case resolves to the
/// canonical [`crate::datatype::DataType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDataType {
    /// UTF-8 text.
    Text,
    /// Variable-size opaque bytes.
    Bytes,
    /// Explicit dynamic payload tree.
    Variant,
    /// Canonical terminal union of one Process invocation.
    ProcessExit,
    /// Canonical terminal union of one File invocation.
    FileExit,
    /// Canonical terminal union of one outbound HTTP call.
    HttpClientExit,
    /// Canonical terminal union of one admitted HTTP server request.
    HttpServerExit,
    /// Canonical terminal union shared by WebSocket client/server sessions.
    WebSocketExit,
}

impl PortDataType {
    /// Constructs the ordinary logical type named by this port rule.
    ///
    /// # Errors
    /// Returns the canonical type constructor's invariant violation.
    pub fn resolve(self) -> Result<crate::datatype::DataType, crate::datatype::TypeError> {
        match self {
            Self::Text => Ok(crate::datatype::DataType::Text),
            Self::Bytes => Ok(crate::datatype::DataType::BYTES),
            Self::Variant => Ok(crate::datatype::DataType::Variant),
            Self::ProcessExit => crate::connector::stdlib::process_exit_type(),
            Self::FileExit => crate::connector::stdlib::file_exit_type(),
            Self::HttpClientExit => crate::connector::stdlib::http_client_exit_type(),
            Self::HttpServerExit => crate::connector::stdlib::http_server_exit_type(),
            Self::WebSocketExit => crate::connector::stdlib::websocket_exit_type(),
        }
    }
}

/// One word-to-type arm of an attribute-selected port rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortTypeChoice {
    /// Word admitted by the selecting attribute.
    pub word: &'static str,
    /// Effective logical type selected by that word.
    pub ty: PortDataType,
}

/// How one contract port obtains its effective logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortTypeRule {
    /// One fixed logical type.
    Fixed(PortDataType),
    /// The exact type resolved for another port in this contract.
    SameAs(&'static str),
    /// A closed choice attribute selects one concrete type.
    AttributeSelected {
        /// Attribute name.
        attribute: &'static str,
        /// Total mapping of that attribute's choice words.
        choices: &'static [PortTypeChoice],
    },
    /// Attribute-selected output whose JSON arm admits an explicit schema.
    /// Other arms remain fixed, and omission preserves the selected type.
    JsonOutput {
        /// Codec-selecting attribute.
        attribute: &'static str,
        /// Default types, including the unrefined JSON `variant` arm.
        choices: &'static [PortTypeChoice],
    },
}

/// One port of a connector sheet under the same direction/type/routing model
/// as every other node port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortContract {
    /// Port name.
    pub name: &'static str,
    /// Input or output direction.
    pub direction: crate::node::PortDirection,
    /// Rule producing the effective logical type for this instantiation.
    pub ty: PortTypeRule,
}

/// Why a validated sheet could not resolve one concrete port table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortResolutionError {
    /// A required choice attribute is absent from the effective attributes.
    MissingAttribute(&'static str),
    /// The effective word is outside the validated rule's mapping.
    UnknownChoice {
        /// Selecting attribute.
        attribute: &'static str,
        /// Effective word.
        word: String,
    },
    /// A same-as dependency did not resolve; sheet validation normally makes
    /// this unreachable.
    UnresolvedDependency(&'static str),
    /// The canonical type constructor refused.
    InvalidType(crate::datatype::TypeError),
    /// A validated port name unexpectedly failed the core constructor.
    InvalidPort(&'static str),
}

impl core::fmt::Display for PortResolutionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingAttribute(name) => write!(f, "missing effective attribute `{name}`"),
            Self::UnknownChoice { attribute, word } => {
                write!(f, "`{attribute}` has no port-type arm for `{word}`")
            }
            Self::UnresolvedDependency(port) => {
                write!(f, "the type dependency of port `{port}` did not resolve")
            }
            Self::InvalidType(error) => error.fmt(f),
            Self::InvalidPort(name) => write!(f, "`{name}` is not a valid port name"),
        }
    }
}

impl core::error::Error for PortResolutionError {}

/// One declarative connector contract sheet: the attributes every
/// profile shares, and the profiles one selector attribute picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorContract {
    /// Stable semantic identity transported by the categorical plan.
    pub id: &'static str,
    /// The NodeType name (`Process`).
    pub name: &'static str,
    /// External-world roles required from a runtime Driver registration.
    pub roles: crate::connector::ConnectorDirection,
    /// Cardinality and terminal lifecycle of one interaction.
    pub exchange: crate::connector::ExchangeKind,
    /// Ports, their directions and effective type rules.
    pub ports: &'static [PortContract],
    /// The attributes common to every profile.
    pub common: &'static [AttributeSpec],
    /// The profile selector, when the contract has profiles. The
    /// admitted selector words are DERIVED from the profiles — they
    /// are spelled nowhere else.
    pub selector: Option<ModeSelector>,
}

impl ConnectorContract {
    /// Resolves one port from the declared rules without building a port table.
    /// The bounded walk follows only `SameAs` dependencies in this definition.
    ///
    /// # Errors
    /// A missing choice, unknown word, invalid type or unresolved dependency.
    pub fn port_type<'a>(
        &self,
        name: &str,
        word: impl Fn(&str) -> Option<&'a str>,
    ) -> Result<crate::datatype::DataType, PortResolutionError> {
        let initial = self
            .ports
            .iter()
            .find(|port| port.name == name)
            .ok_or(PortResolutionError::UnresolvedDependency("unknown port"))?;
        let mut current = initial;
        for _ in 0..self.ports.len() {
            match current.ty {
                PortTypeRule::Fixed(ty) => {
                    return ty.resolve().map_err(PortResolutionError::InvalidType);
                }
                PortTypeRule::SameAs(other) => {
                    current = self
                        .ports
                        .iter()
                        .find(|port| port.name == other)
                        .ok_or(PortResolutionError::UnresolvedDependency(other))?;
                }
                PortTypeRule::AttributeSelected { attribute, choices }
                | PortTypeRule::JsonOutput { attribute, choices } => {
                    let word =
                        word(attribute).ok_or(PortResolutionError::MissingAttribute(attribute))?;
                    let choice = choices
                        .iter()
                        .find(|choice| choice.word == word)
                        .ok_or_else(|| PortResolutionError::UnknownChoice {
                            attribute,
                            word: word.to_string(),
                        })?;
                    return choice
                        .ty
                        .resolve()
                        .map_err(PortResolutionError::InvalidType);
                }
            }
        }
        Err(PortResolutionError::UnresolvedDependency(initial.name))
    }

    /// Exact Contract identities represented by this definition and its modes.
    pub fn protocol_ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        core::iter::once(self.id).chain(
            self.selector
                .iter()
                .flat_map(|selector| selector.profiles)
                .filter_map(|profile| profile.protocol.map(|protocol| protocol.id)),
        )
    }

    /// The spec of one attribute within the given resolved profile:
    /// the common sheet first, then the profile's own.
    #[must_use]
    pub fn attribute(&self, profile: Option<&ContractMode>, name: &str) -> Option<&AttributeSpec> {
        self.common
            .iter()
            .find(|spec| spec.name == name)
            .or_else(|| {
                profile.and_then(|profile| profile.attributes.iter().find(|spec| spec.name == name))
            })
    }

    /// The profile owning an attribute NAME that the resolved profile
    /// does not — the "belongs to another profile" diagnostic's
    /// source.
    #[must_use]
    pub fn owning_profile(&self, name: &str) -> Option<&ContractMode> {
        let profiles = self.selector.as_ref()?.profiles;
        profiles
            .iter()
            .find(|profile| profile.attributes.iter().any(|spec| spec.name == name))
    }

    /// Resolves the selector word — written or defaulted — to its
    /// profile.
    #[must_use]
    pub fn profile(&self, word: &str) -> Option<&ContractMode> {
        self.selector
            .as_ref()?
            .profiles
            .iter()
            .find(|profile| profile.word == word)
    }

    /// Resolves this sheet's complete travelling port table from effective
    /// choice attributes. The caller passes attributes after defaults have
    /// been materialized, so no runtime ContractCatalog lookup remains.
    ///
    /// # Errors
    ///
    /// A missing/unknown selector, invalid canonical type, or unresolved
    /// same-as dependency refuses the instantiation.
    pub fn resolve_ports(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<Vec<crate::plan::PortDecl>, PortResolutionError> {
        let mut resolved: BTreeMap<&'static str, crate::datatype::DataType> = BTreeMap::new();
        for _ in 0..self.ports.len().saturating_add(1) {
            let mut changed = false;
            for port in self.ports {
                if resolved.contains_key(port.name) {
                    continue;
                }
                let ty = match port.ty {
                    PortTypeRule::Fixed(ty) => {
                        Some(ty.resolve().map_err(PortResolutionError::InvalidType)?)
                    }
                    PortTypeRule::SameAs(other) => resolved.get(other).cloned(),
                    PortTypeRule::AttributeSelected { attribute, choices }
                    | PortTypeRule::JsonOutput { attribute, choices } => {
                        let word = attributes
                            .get(attribute)
                            .ok_or(PortResolutionError::MissingAttribute(attribute))?;
                        let choice = choices.iter().find(|choice| choice.word == word).ok_or(
                            PortResolutionError::UnknownChoice {
                                attribute,
                                word: word.clone(),
                            },
                        )?;
                        Some(
                            choice
                                .ty
                                .resolve()
                                .map_err(PortResolutionError::InvalidType)?,
                        )
                    }
                };
                if let Some(ty) = ty {
                    resolved.insert(port.name, ty);
                    changed = true;
                }
            }
            if resolved.len() == self.ports.len() {
                break;
            }
            if !changed {
                return Err(PortResolutionError::UnresolvedDependency(
                    self.ports
                        .iter()
                        .find(|port| !resolved.contains_key(port.name))
                        .map_or("?", |port| port.name),
                ));
            }
        }
        let mut ports = Vec::with_capacity(self.ports.len());
        for contract in self.ports {
            let port = crate::node::Port::new(contract.name.to_string())
                .map_err(|_| PortResolutionError::InvalidPort(contract.name))?;
            let ty = resolved
                .remove(contract.name)
                .ok_or(PortResolutionError::UnresolvedDependency(contract.name))?;
            ports.push(crate::plan::PortDecl {
                port,
                direction: contract.direction,

                ty: Some(ty),
                annotations: crate::schema_annotation::AnnotationTable::empty(),
            });
        }
        Ok(ports)
    }
}

/// The one attribute selecting a contract's profile, and the profiles
/// themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeSelector {
    /// The selecting attribute's name (`mode`).
    pub attribute: &'static str,
    /// The profile selected when the attribute is omitted; the
    /// checker materializes the word into the effective HIR.
    pub default: Option<&'static str>,
    /// The contract's profiles.
    pub profiles: &'static [ContractMode],
}

/// One profile of a contract: a name the selector picks, the build's
/// support for it, and its own attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractMode {
    /// A profile may select a different exact exchange contract. Omission
    /// preserves the sheet's contract, roles and exchange byte for byte.
    pub protocol: Option<ProfileProtocol>,
    /// The selector word (`job`).
    pub word: &'static str,
    /// Whether THIS build executes the profile — distinct from the
    /// contract knowing it.
    pub support: ModeSupport,
    /// The attributes existing only in this profile. A field of
    /// another profile is simply ABSENT here — there is no
    /// `Forbidden` state (D-2026-08-12-1).
    pub attributes: &'static [AttributeSpec],
}

/// Resolved protocol of a profile whose resource lifecycle differs from the
/// sheet's default (for example a resident Process versus a finite job).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileProtocol {
    /// Exact versioned contract, not an alias for the default protocol.
    pub id: &'static str,
    /// Required Driver views.
    pub roles: crate::connector::ConnectorDirection,
    /// Interaction cardinality.
    pub exchange: crate::connector::ExchangeKind,
}

/// The build's relationship to a profile the contract knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSupport {
    /// The build validates and executes the profile.
    Available,
    /// The build defers the profile with this named reason; its
    /// attributes are declared but not validated (their domains may
    /// be deferred-only).
    Deferred(&'static str),
}

/// One attribute of a contract sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSpec {
    /// The attribute key.
    pub name: &'static str,
    /// What the source must, may, or need not provide.
    pub presence: Presence,
    /// The admitted value domain.
    pub domain: ValueDomain,
    /// The latest phase at which this attribute may become a value.
    /// Checking chooses the earliest phase where the written expression
    /// closes; this is a ceiling, never a request to delay a constant.
    pub latest_site: EvaluationSite,
}

/// The presence a contract demands in the source, and what absence
/// resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// The author must write the attribute.
    Required,
    /// Absence has a meaning of its own and survives into the plan.
    Optional,
    /// Absence is sugar for the contract's default: the checker
    /// injects it into the effective HIR, so omission and the exact
    /// spelling of the default are ONE identity.
    Defaulted(DefaultValue),
}

/// A contract default — a closed-choice word or a closed, constant Rastok
/// expression (text, lists and durations are already expressions; there is no
/// second literal algebra).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultValue {
    /// A word of the attribute's `Choice` domain.
    Choice(&'static str),
    /// A constant Rastok expression source, compiled through the SAME chain as
    /// author expressions. It must not read `data`, `meta`, node
    /// state, application `val`s, `env()` or `secret()` — a standard
    /// contract introduces no invisible dependency.
    Expression(&'static str),
}

/// The value domain of a contract attribute.
///
/// Implemented domains are admitted on available profiles; the rest are
/// DECLARED for deferred profiles (the sheet says exactly what it knows) and
/// admitted only behind [`ModeSupport::Deferred`]. Hosted catalogue validation
/// checks this before accepting a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueDomain {
    /// A plan-constant boolean guarantee, not a choice word.
    Boolean,
    /// One of a closed set of bare words.
    Choice(&'static [&'static str]),
    /// A constant text value.
    Text,
    /// A constant opaque byte value.
    Bytes,
    /// A constant list of text values.
    TextList,
    /// A non-empty, duplicate-free ordered list drawn from a closed text set.
    ChoiceList(&'static [&'static str]),
    /// A constant duration value.
    Duration,
    /// A constant byte-size string (`"8MiB"`) — judged by
    /// [`byte_size`], the ONE parser the contract and the Driver
    /// share: what validates is what executes.
    ByteSize,
    /// A text-keyed map whose values are all text.
    TextMap,
    /// Known request fields as a typed, width-subtyped struct. V1 owns
    /// `content_type: media_type`; absent fields remain absent.
    TypedHttpHeaders,
    /// Extension request fields: ordinary-case names to ordered UTF-8 lines.
    CustomHttpHeaders,
    /// A text-valued Rastok expression path over invocation data
    /// (`correlate = data.id`) — never a boot-phase constant or an arbitrary
    /// computed expression.
    ExprPath,
    /// A typed struct value. Deferred-only.
    Struct,
    /// A typed map value. Deferred-only.
    Map,
}

impl ValueDomain {
    /// Whether this domain is implemented by the current build's
    /// validation — the others may only appear behind a deferred
    /// profile.
    #[must_use]
    pub fn implemented(&self) -> bool {
        matches!(
            self,
            Self::Boolean
                | Self::Choice(_)
                | Self::Text
                | Self::Bytes
                | Self::TextList
                | Self::ChoiceList(_)
                | Self::Duration
                | Self::ByteSize
                | Self::TextMap
                | Self::TypedHttpHeaders
                | Self::CustomHttpHeaders
                | Self::ExprPath
        )
    }

    /// The canonical static type expected from one implemented expression
    /// domain. Choice words are syntax rather than expressions; deferred-only
    /// domains deliberately have no executable type here.
    #[must_use]
    pub fn expression_type(&self) -> Option<crate::datatype::DataType> {
        match self {
            Self::Boolean => Some(crate::datatype::DataType::Bool),
            Self::Text | Self::ByteSize => Some(crate::datatype::DataType::Text),
            Self::Bytes => Some(crate::datatype::DataType::BYTES),
            Self::TextList => Some(crate::datatype::DataType::array(
                crate::datatype::DataType::Text,
            )),
            Self::ChoiceList(_) => Some(crate::datatype::DataType::array(
                crate::datatype::DataType::Text,
            )),
            Self::Duration => Some(crate::datatype::DataType::DURATION),
            Self::TextMap => Some(crate::datatype::DataType::text_map(
                crate::datatype::DataType::Text,
            )),
            Self::TypedHttpHeaders => None,
            Self::CustomHttpHeaders => Some(crate::plan::custom_http_headers_type()),
            Self::ExprPath => Some(crate::datatype::DataType::Text),
            Self::Choice(_) | Self::Struct | Self::Map => None,
        }
    }

    /// Judges the resolved static type of one expression before a Driver can
    /// observe it. Typed HTTP fields are width-subtyped; other implemented
    /// expression domains own one exact type.
    #[must_use]
    pub fn admits_type(&self, ty: &crate::datatype::DataType) -> bool {
        match self {
            Self::TypedHttpHeaders => crate::plan::typed_http_headers_type_holds(ty),
            Self::Choice(_) | Self::Struct | Self::Map => false,
            _ => self.expression_type().as_ref() == Some(ty),
        }
    }

    /// Judges a CONSTANT value against the domain — the ONE
    /// predicate shared by author-written values and sheet defaults.
    #[must_use]
    pub fn admits(&self, value: &crate::value::Value) -> bool {
        match self {
            Self::Boolean => matches!(value, crate::value::Value::Bool(_)),
            Self::Choice(_) | Self::ExprPath | Self::Struct | Self::Map => false,
            Self::Text => matches!(value, crate::value::Value::Text(_)),
            Self::Bytes => matches!(value, crate::value::Value::Bytes(_)),
            Self::TextList => match value {
                crate::value::Value::Array(items) => items
                    .iter()
                    .all(|item| matches!(item, crate::value::Value::Text(_))),
                _ => false,
            },
            Self::ChoiceList(words) => match value {
                crate::value::Value::Array(items) if !items.is_empty() => {
                    let mut seen = alloc::collections::BTreeSet::new();
                    items.iter().all(|item| match item {
                        crate::value::Value::Text(word) => {
                            words.contains(&word.as_str()) && seen.insert(word.as_str())
                        }
                        _ => false,
                    })
                }
                _ => false,
            },
            Self::Duration => matches!(value, crate::value::Value::Duration(_)),
            Self::ByteSize => {
                matches!(value, crate::value::Value::Text(text) if byte_size(text).is_some())
            }
            Self::TextMap => match value {
                crate::value::Value::Map(entries) => entries
                    .values()
                    .all(|value| matches!(value, crate::value::Value::Text(_))),
                _ => false,
            },
            Self::TypedHttpHeaders => crate::plan::typed_http_headers_value_holds(value),
            Self::CustomHttpHeaders => crate::plan::custom_http_headers_value_holds(value),
        }
    }
}
