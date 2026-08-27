//! The node-contract METAMODEL (D-2026-08-12-1).
//!
//! This module describes which node declarations are VALID: attribute
//! sheets with a presence (`Required | Optional | Defaulted`), a value
//! domain, optional PROFILES selected by one attribute, and the
//! registry a build wires into its checker. It is compiler-layer data —
//! never serialized, never needed by the no_std runtime. The standard
//! catalogue instancing this metamodel (`PROCESS`, the declared names)
//! lives in [`crate::stdlib`]; resolved, executable instances live in
//! [`crate::plan`].

use std::collections::BTreeMap;

use crate::expr;
pub use crate::plan::EvaluationSite;

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
    fn resolve(self) -> Result<crate::datatype::DataType, crate::datatype::TypeError> {
        match self {
            Self::Text => Ok(crate::datatype::DataType::Text),
            Self::Bytes => Ok(crate::datatype::DataType::BYTES),
            Self::Variant => Ok(crate::datatype::DataType::Variant),
            Self::ProcessExit => crate::boundary::process_exit_type(),
            Self::FileExit => crate::boundary::file_exit_type(),
            Self::HttpClientExit => crate::boundary::http_client_exit_type(),
            Self::HttpServerExit => crate::boundary::http_server_exit_type(),
            Self::WebSocketExit => crate::boundary::websocket_exit_type(),
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
}

/// One port of a connector sheet under the same direction/type/routing model
/// as every other node port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortContract {
    /// Port name.
    pub name: &'static str,
    /// Input or output direction.
    pub direction: crate::node::Direction,
    /// What an unclaimed output does. Inputs must be `RequireWire`.
    pub unwired: crate::plan::UnwiredPolicy,
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

impl std::error::Error for PortResolutionError {}

/// How a connector type name resolves against the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability<'registry> {
    /// The contract sheet is present in THIS registry — the checker
    /// can resolve declarations against it. Whether a Driver ships
    /// is the build's wiring (the Lab mocks need none); the sheet
    /// claims the contract, not the implementation.
    Available(&'registry ConnectorContract),
    /// The name is in the catalogue; its Driver is not in this build.
    Declared,
    /// The name is not a known connector type.
    Unknown,
}

/// How a tagged frontend owns the bytes after `IDENT!`.
///
/// This is compiler-layer framing policy, not host grammar.  The host chooses
/// the frontend production from the adjacent bang alone; only then does the
/// registry select one of these readers.  Adding a real guest compiler may add
/// another registered framing implementation without teaching the parser a
/// language name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendFraming {
    /// One self-delimited JSON value, read by the canonical house reader.
    JsonValue,
    /// A braced block where every byte except balanced braces is verbatim.
    VerbatimBraced,
    /// A braced guest region whose registered contract treats quoted strings
    /// as opaque while balancing braces.
    QuotedBraced,
}

/// The implementation attached to one tagged frontend contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendReader {
    /// Canonical JSON value reader; result type is `variant`.
    Json,
    /// Verbatim UTF-8 text reader; result type is `text`.
    Text,
    /// RFC 4648 decoder over a verbatim block; result type is `bytes`.
    Base64,
    /// The name and framing are known, but no compiler/reader ships in this
    /// build.  The checker emits the named deferral after parsing the region.
    Deferred {
        /// The framing contract this registered frontend owns.
        framing: FrontendFraming,
        /// Stable construct name used by the ordinary deferred diagnostic.
        construct: &'static str,
    },
}

impl FrontendReader {
    /// The framing implementation selected after the host has recognized
    /// `IDENT!`.
    #[must_use]
    pub const fn framing(self) -> FrontendFraming {
        match self {
            Self::Json => FrontendFraming::JsonValue,
            Self::Text | Self::Base64 => FrontendFraming::VerbatimBraced,
            Self::Deferred { framing, .. } => framing,
        }
    }

    /// The logical result type when this build can execute the frontend.
    #[must_use]
    pub const fn result_type(self) -> Option<crate::datatype::DataType> {
        match self {
            Self::Json => Some(crate::datatype::DataType::Variant),
            Self::Text => Some(crate::datatype::DataType::Text),
            Self::Base64 => Some(crate::datatype::DataType::BYTES),
            Self::Deferred { .. } => None,
        }
    }
}

/// One tagged frontend known to a compiler build.
///
/// The name is deliberately ordinary: it is never a parser keyword.  The
/// contract is consulted only after the adjacent `!` has selected the grammar
/// production.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendContract {
    /// Source name before `!` (`json`, `jq`, …).
    pub name: &'static str,
    /// Reader/compiler and, through it, the framing and output type rules.
    pub reader: FrontendReader,
}

/// One declarative connector contract sheet: the attributes every
/// profile shares, and the profiles one selector attribute picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorContract {
    /// Stable semantic identity transported by the categorical plan.
    pub id: &'static str,
    /// The NodeType name (`Process`).
    pub name: &'static str,
    /// External-world roles required from a runtime Driver registration.
    pub roles: crate::connector::ConnectorRoles,
    /// Cardinality and terminal lifecycle of one interaction.
    pub exchange: crate::connector::ConnectorExchange,
    /// Ports, their directions, effective type rules and unwired policy.
    pub ports: &'static [PortContract],
    /// The attributes common to every profile.
    pub common: &'static [AttributeSpec],
    /// The profile selector, when the contract has profiles. The
    /// admitted selector words are DERIVED from the profiles — they
    /// are spelled nowhere else.
    pub selector: Option<ProfileSelector>,
}

impl ConnectorContract {
    /// The spec of one attribute within the given resolved profile:
    /// the common sheet first, then the profile's own.
    #[must_use]
    pub fn attribute(&self, profile: Option<&Profile>, name: &str) -> Option<&AttributeSpec> {
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
    pub fn owning_profile(&self, name: &str) -> Option<&Profile> {
        let profiles = self.selector.as_ref()?.profiles;
        profiles
            .iter()
            .find(|profile| profile.attributes.iter().any(|spec| spec.name == name))
    }

    /// Resolves the selector word — written or defaulted — to its
    /// profile.
    #[must_use]
    pub fn profile(&self, word: &str) -> Option<&Profile> {
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
                    PortTypeRule::AttributeSelected { attribute, choices } => {
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
                unwired: contract.unwired,
                ty: Some(ty),
            });
        }
        Ok(ports)
    }
}

/// The one attribute selecting a contract's profile, and the profiles
/// themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSelector {
    /// The selecting attribute's name (`mode`).
    pub attribute: &'static str,
    /// The profile selected when the attribute is omitted; the
    /// checker materializes the word into the effective HIR.
    pub default: Option<&'static str>,
    /// The contract's profiles.
    pub profiles: &'static [Profile],
}

/// One profile of a contract: a name the selector picks, the build's
/// support for it, and its own attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The selector word (`job`).
    pub word: &'static str,
    /// Whether THIS build executes the profile — distinct from the
    /// contract knowing it.
    pub support: ProfileSupport,
    /// The attributes existing only in this profile. A field of
    /// another profile is simply ABSENT here — there is no
    /// `Forbidden` state (D-2026-08-12-1).
    pub attributes: &'static [AttributeSpec],
}

/// The build's relationship to a profile the contract knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSupport {
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
/// admitted only behind [`ProfileSupport::Deferred`] — [`validate_sheet`]
/// holds that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueDomain {
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
            Self::Choice(_)
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
                    let mut seen = std::collections::BTreeSet::new();
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

/// The normative ceiling of a declared byte size: 4 GiB, inclusive
/// — target-independent, so the same source validates identically
/// on every machine (a 32-bit host compares lengths in `u64` and
/// never allocates the bound up front).
pub use crate::connector::MAX_BYTE_SIZE;

/// Reads a declared byte size (`"8MiB"`, `"512KiB"`, `"100B"`) — an
/// explicit unit is required, and the result is bounded by
/// [`MAX_BYTE_SIZE`]. The ONE rule shared by the contract's domain
/// judgement and the Driver's budget: a size the checker accepted
/// is a size the runtime enforces, on every target width.
#[must_use]
pub fn byte_size(text: &str) -> Option<u64> {
    crate::connector::byte_size(text)
}

/// Why a contract sheet cannot enter the registry — the sheet is an
/// AUTHORITY once admitted (the checker materializes port tables and
/// injects defaults from it without re-checking), so it is held at
/// the door. A defect is the Rust contract's fault, never the RKG
/// author's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetDefect {
    /// The stable contract identity is not canonical.
    InvalidConnectorId {
        /// Exact spelling supplied by the sheet.
        id: &'static str,
        /// Canonical grammar category that refused it.
        reason: crate::connector::ConnectorIdError,
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
    /// An input tried to carry an output-only unwired policy.
    InputHasSinkPolicy(&'static str),
    /// The conventional terminal row contradicts the exchange lifecycle:
    /// Event has none; Call and Session have exactly one output `exit`.
    ExchangeTerminalMismatch {
        /// Exchange declared by the sheet.
        exchange: crate::connector::ConnectorExchange,
        /// Number of conventional output terminal rows found.
        exits: usize,
    },
    /// The role set cannot carry this exchange's complete generic protocol:
    /// Event is exactly inbound, while Session needs the outbound view that
    /// reports open/write/terminal observations.
    ExchangeRoleMismatch {
        /// Exchange declared by the sheet.
        exchange: crate::connector::ConnectorExchange,
        /// Incompatible Driver roles declared by the sheet.
        roles: crate::connector::ConnectorRoles,
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

impl core::fmt::Display for SheetDefect {
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
            Self::InputHasSinkPolicy(name) => {
                write!(
                    f,
                    "input port `{name}` cannot carry an unwired output policy"
                )
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

impl std::error::Error for SheetDefect {}

/// Whether a sheet-declared name is expressible in the DSL — the
/// identifier grammar the sheet's ports, attributes, selector,
/// profile words and choice words must all satisfy: an
/// admitted sheet is an authority, so nothing inexpressible enters.
fn expressible_name(text: &str) -> bool {
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
fn validate_default(spec: &AttributeSpec) -> Result<(), SheetDefect> {
    let Presence::Defaulted(default) = &spec.presence else {
        return Ok(());
    };
    match default {
        DefaultValue::Choice(word) => match &spec.domain {
            ValueDomain::Choice(words) if words.contains(word) => Ok(()),
            _ => Err(SheetDefect::InvalidDefault {
                attribute: spec.name,
                reason: format!("`{word}` is not a word of the attribute's domain"),
            }),
        },
        DefaultValue::Expression(source) => {
            let tree = expr::compile(source, &expr::Signature::new()).map_err(|_| {
                SheetDefect::InvalidDefault {
                    attribute: spec.name,
                    reason: format!("`{source}` does not compile as closed Rastok expression"),
                }
            })?;
            let value = expr::const_eval(&tree, &BTreeMap::new()).map_err(|error| {
                SheetDefect::InvalidDefault {
                    attribute: spec.name,
                    reason: format!("`{source}` is not constant: {}", error.message),
                }
            })?;
            if spec.domain.admits(&value) {
                Ok(())
            } else {
                Err(SheetDefect::InvalidDefault {
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
) -> Result<(), SheetDefect> {
    let mut local = std::collections::BTreeSet::new();
    for spec in specs {
        if !expressible_name(spec.name) {
            return Err(SheetDefect::InexpressibleName(spec.name));
        }
        if !local.insert(spec.name) || seen.contains(spec.name) {
            return Err(SheetDefect::DuplicateAttribute(spec.name));
        }
        if let ValueDomain::Choice(words) = &spec.domain {
            if words.is_empty() {
                return Err(SheetDefect::EmptyChoice(spec.name));
            }
            let mut unique = std::collections::BTreeSet::new();
            for word in *words {
                if !expressible_name(word) {
                    return Err(SheetDefect::InexpressibleName(word));
                }
                if !unique.insert(*word) {
                    return Err(SheetDefect::DuplicateChoice(spec.name));
                }
            }
        }
        if let ValueDomain::ChoiceList(words) = &spec.domain {
            if words.is_empty() {
                return Err(SheetDefect::EmptyChoice(spec.name));
            }
            let mut unique = std::collections::BTreeSet::new();
            for word in *words {
                if !unique.insert(*word) {
                    return Err(SheetDefect::DuplicateChoice(spec.name));
                }
            }
        }
        if spec.domain.implemented() {
            validate_default(spec)?;
        } else {
            if !deferred_profile {
                return Err(SheetDefect::UnimplementedDomain(spec.name));
            }
            // A domain nobody can judge carries no default — a
            // default that cannot be validated is a lie waiting.
            if matches!(spec.presence, Presence::Defaulted(_)) {
                return Err(SheetDefect::InvalidDefault {
                    attribute: spec.name,
                    reason: String::from("a deferred-only domain cannot carry a default"),
                });
            }
        }
    }
    Ok(())
}

/// Holds a sheet to the rules the checker will assume forever after.
pub fn validate_sheet(contract: &ConnectorContract) -> Result<(), SheetDefect> {
    crate::connector::ConnectorId::parse(contract.id).map_err(|reason| {
        SheetDefect::InvalidConnectorId {
            id: contract.id,
            reason,
        }
    })?;
    if !expressible_name(contract.name) {
        return Err(SheetDefect::InexpressibleName(contract.name));
    }
    let mut seen = std::collections::BTreeSet::new();
    for port in contract.ports {
        let name = port.name;
        let direction = port.direction;
        if !expressible_name(name) || crate::node::Port::new(name.to_string()).is_err() {
            return Err(SheetDefect::InvalidPortName(name));
        }
        if !seen.insert(name) {
            return Err(SheetDefect::ConflictingPort(name));
        }
        let implicit = match name {
            "in" => Some(crate::node::Direction::Input),
            "out" | "err" | "exit" => Some(crate::node::Direction::Output),
            _ => None,
        };
        if let Some(expected) = implicit
            && expected != direction
        {
            return Err(SheetDefect::MisdirectedDefault(name));
        }
        if direction == crate::node::Direction::Input
            && port.unwired != crate::plan::UnwiredPolicy::RequireWire
        {
            return Err(SheetDefect::InputHasSinkPolicy(name));
        }
    }
    let mut names = std::collections::BTreeSet::new();
    validate_attributes(contract.common, &mut names, false)?;
    for spec in contract.common {
        names.insert(spec.name);
    }
    if let Some(selector) = &contract.selector {
        if !expressible_name(selector.attribute) {
            return Err(SheetDefect::InexpressibleName(selector.attribute));
        }
        if selector.profiles.is_empty() {
            return Err(SheetDefect::NoProfiles(selector.attribute));
        }
        if names.contains(selector.attribute) {
            return Err(SheetDefect::SelectorCollision(selector.attribute));
        }
        let mut words = std::collections::BTreeSet::new();
        for profile in selector.profiles {
            if !expressible_name(profile.word) {
                return Err(SheetDefect::InexpressibleName(profile.word));
            }
            if !words.insert(profile.word) {
                return Err(SheetDefect::DuplicateProfile(profile.word));
            }
            if profile
                .attributes
                .iter()
                .any(|spec| spec.name == selector.attribute)
            {
                return Err(SheetDefect::SelectorCollision(selector.attribute));
            }
            // Common-vs-profile collisions refuse; two PROFILES may
            // each own a name (none does today).
            let mut shared = names.clone();
            validate_attributes(
                profile.attributes,
                &mut shared,
                matches!(profile.support, ProfileSupport::Deferred(_)),
            )?;
        }
        if let Some(default) = selector.default
            && !words.contains(default)
        {
            return Err(SheetDefect::UnknownDefaultProfile(default));
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
                let resolved = ty
                    .resolve()
                    .map_err(|reason| SheetDefect::InvalidPortType {
                        port: port.name,
                        reason: reason.to_string(),
                    })?;
                resolved
                    .validate()
                    .map_err(|reason| SheetDefect::InvalidPortType {
                        port: port.name,
                        reason: reason.to_string(),
                    })?;
            }
            PortTypeRule::SameAs(dependency) => {
                if !contract
                    .ports
                    .iter()
                    .any(|candidate| candidate.name == dependency)
                {
                    return Err(SheetDefect::UnknownPortDependency {
                        port: port.name,
                        dependency,
                    });
                }
                let mut visited = std::collections::BTreeSet::new();
                let mut next = port.name;
                loop {
                    if !visited.insert(next) {
                        return Err(SheetDefect::CyclicPortTypes(port.name));
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
            PortTypeRule::AttributeSelected { attribute, choices } => {
                let Some(words) = choice_domain(attribute) else {
                    return Err(SheetDefect::UnknownPortTypeAttribute {
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
                    return Err(SheetDefect::LatePortTypeAttribute {
                        port: port.name,
                        attribute,
                    });
                }
                let mut arms = std::collections::BTreeSet::new();
                for choice in choices {
                    if !arms.insert(choice.word) || !words.contains(&choice.word) {
                        return Err(SheetDefect::PortTypeChoiceMismatch {
                            port: port.name,
                            attribute,
                        });
                    }
                    let resolved =
                        choice
                            .ty
                            .resolve()
                            .map_err(|reason| SheetDefect::InvalidPortType {
                                port: port.name,
                                reason: reason.to_string(),
                            })?;
                    resolved
                        .validate()
                        .map_err(|reason| SheetDefect::InvalidPortType {
                            port: port.name,
                            reason: reason.to_string(),
                        })?;
                }
                if words.iter().any(|word| !arms.contains(word)) {
                    return Err(SheetDefect::PortTypeChoiceMismatch {
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
        .filter(|port| port.name == "exit" && port.direction == crate::node::Direction::Output)
        .count();
    let terminal_shape_holds = match contract.exchange {
        crate::connector::ConnectorExchange::Event => exits == 0,
        crate::connector::ConnectorExchange::Call
        | crate::connector::ConnectorExchange::Session => exits == 1,
    };
    if !terminal_shape_holds {
        return Err(SheetDefect::ExchangeTerminalMismatch {
            exchange: contract.exchange,
            exits,
        });
    }
    let roles_hold = match contract.exchange {
        crate::connector::ConnectorExchange::Event => {
            contract.roles == crate::connector::ConnectorRoles::Inbound
        }
        crate::connector::ConnectorExchange::Call => true,
        crate::connector::ConnectorExchange::Session => contract.roles.has_outbound(),
    };
    if !roles_hold {
        return Err(SheetDefect::ExchangeRoleMismatch {
            exchange: contract.exchange,
            roles: contract.roles,
        });
    }
    Ok(())
}

/// The contract catalogue a build wires into its checker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractCatalog {
    pub(crate) connectors: BTreeMap<crate::connector::ConnectorId, ConnectorContract>,
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
    /// The first [`SheetDefect`], named.
    pub fn try_with(mut self, contract: ConnectorContract) -> Result<Self, SheetDefect> {
        validate_sheet(&contract)?;
        let id = crate::connector::ConnectorId::parse(contract.id).map_err(|reason| {
            SheetDefect::InvalidConnectorId {
                id: contract.id,
                reason,
            }
        })?;
        if let Some(existing) = self.connectors.get(&id)
            && existing.name != contract.name
        {
            return Err(SheetDefect::ConnectorIdCollision {
                id: contract.id,
                existing: existing.name,
                incoming: contract.name,
            });
        }
        self.connectors
            .retain(|_, existing| existing.name != contract.name);
        self.declared.retain(|declared| *declared != contract.name);
        self.connectors.insert(id, contract);
        Ok(self)
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
    pub fn connector_by_id(
        &self,
        id: &crate::connector::ConnectorId,
    ) -> Option<&ConnectorContract> {
        self.connectors.get(id)
    }

    /// Extends the registry with a tagged frontend contract.  A repeated name
    /// intentionally replaces the previous contract, matching connector
    /// extension and making application registries composable.
    ///
    /// # Errors
    ///
    /// Returns [`SheetDefect::InexpressibleName`] when the frontend name cannot
    /// be written as the `IDENT` before `!`.
    pub fn try_with_frontend(mut self, frontend: FrontendContract) -> Result<Self, SheetDefect> {
        if !expressible_name(frontend.name) {
            return Err(SheetDefect::InexpressibleName(frontend.name));
        }
        self.frontends
            .retain(|existing| existing.name != frontend.name);
        self.frontends.push(frontend);
        Ok(self)
    }

    /// The tagged frontend contract for a source name, if registered.
    #[must_use]
    pub fn frontend(&self, name: &str) -> Option<&FrontendContract> {
        self.frontends.iter().find(|frontend| frontend.name == name)
    }
}
