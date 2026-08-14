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
    /// The NodeType name (`Process`).
    pub name: &'static str,
    /// The envelope ports the type defines as inputs.
    pub input_ports: &'static [&'static str],
    /// The envelope ports the type defines as outputs.
    pub output_ports: &'static [&'static str],
    /// The attributes common to every profile.
    pub common: &'static [AttributeSpec],
    /// The profile selector, when the contract has profiles. The
    /// admitted selector words are DERIVED from the profiles — they
    /// are spelled nowhere else.
    pub selector: Option<ProfileSelector>,
    /// The typed error set; every member reaches the `err` port.
    pub error_set: &'static [&'static str],
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
/// The first five are implemented; the rest are DECLARED for
/// deferred profiles (the sheet says exactly what it knows) and
/// admitted only behind [`ProfileSupport::Deferred`] —
/// [`validate_sheet`] holds that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueDomain {
    /// One of a closed set of bare words.
    Choice(&'static [&'static str]),
    /// A constant text value.
    Text,
    /// A constant list of text values.
    TextList,
    /// A constant duration value.
    Duration,
    /// A constant byte-size string (`"8MiB"`) — judged by
    /// [`byte_size`], the ONE parser the contract and the adapter
    /// share: what validates is what executes.
    ByteSize,
    /// A Rastok expression path over runtime data (`correlate = data.id`) — not a
    /// boot-phase constant. Deferred-only.
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
            Self::Choice(_) | Self::Text | Self::TextList | Self::Duration | Self::ByteSize
        )
    }

    /// Judges a CONSTANT value against the domain — the ONE
    /// predicate shared by author-written values and sheet defaults.
    #[must_use]
    pub fn admits(&self, value: &crate::value::Value) -> bool {
        match self {
            Self::Choice(_) | Self::ExprPath | Self::Struct | Self::Map => false,
            Self::Text => matches!(value, crate::value::Value::Text(_)),
            Self::TextList => match value {
                crate::value::Value::Array(items) => items
                    .iter()
                    .all(|item| matches!(item, crate::value::Value::Text(_))),
                _ => false,
            },
            Self::Duration => matches!(value, crate::value::Value::Duration(_)),
            Self::ByteSize => {
                matches!(value, crate::value::Value::Text(text) if byte_size(text).is_some())
            }
        }
    }
}

/// The normative ceiling of a declared byte size: 4 GiB, inclusive
/// — target-independent, so the same source validates identically
/// on every machine (a 32-bit host compares lengths in `u64` and
/// never allocates the bound up front).
pub const MAX_BYTE_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Reads a declared byte size (`"8MiB"`, `"512KiB"`, `"100B"`) — an
/// explicit unit is required, and the result is bounded by
/// [`MAX_BYTE_SIZE`]. The ONE rule shared by the contract's domain
/// judgement and the adapter's budget: a size the checker accepted
/// is a size the runtime enforces, on every target width.
#[must_use]
pub fn byte_size(text: &str) -> Option<u64> {
    // ASCII whitespace exactly — the normative rule is byte-level
    // deterministic, no Unicode class in sight.
    let text = text.trim_matches(|c: char| c.is_ascii_whitespace());
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (digits, suffix) = text.split_at(split);
    if digits.is_empty() {
        return None;
    }
    let base: u64 = digits.parse().ok()?;
    let factor: u64 = match suffix.trim_matches(|c: char| c.is_ascii_whitespace()) {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    let size = base.checked_mul(factor)?;
    (size <= MAX_BYTE_SIZE).then_some(size)
}

/// Why a contract sheet cannot enter the registry — the sheet is an
/// AUTHORITY once admitted (the checker materializes port tables and
/// injects defaults from it without re-checking), so it is held at
/// the door. A defect is the Rust contract's fault, never the RKG
/// author's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetDefect {
    /// A port name the language refuses.
    InvalidPortName(&'static str),
    /// One name in both direction lists, or listed twice.
    ConflictingPort(&'static str),
    /// A default port listed against its implicit direction.
    MisdirectedDefault(&'static str),
    /// An attribute declared twice within one scope (the common
    /// sheet, or one profile), or in both the common sheet and a
    /// profile.
    DuplicateAttribute(&'static str),
    /// An error kind declared twice.
    DuplicateErrorKind(&'static str),
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
            Self::InvalidPortName(name) => write!(f, "`{name}` is not a valid port name"),
            Self::ConflictingPort(name) => {
                write!(f, "`{name}` is listed twice or in both directions")
            }
            Self::MisdirectedDefault(name) => {
                write!(f, "`{name}` is a default port listed against its direction")
            }
            Self::DuplicateAttribute(name) => write!(f, "`{name}` is declared twice"),
            Self::DuplicateErrorKind(name) => write!(f, "`{name}` is declared twice"),
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
/// profile words, choice words and error kinds must all satisfy: an
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
    if !expressible_name(contract.name) {
        return Err(SheetDefect::InexpressibleName(contract.name));
    }
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
        if !expressible_name(name) || crate::node::Port::new(name.to_string()).is_err() {
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
    let mut kinds = std::collections::BTreeSet::new();
    for kind in contract.error_set {
        if !expressible_name(kind) {
            return Err(SheetDefect::InexpressibleName(kind));
        }
        if !kinds.insert(*kind) {
            return Err(SheetDefect::DuplicateErrorKind(kind));
        }
    }
    Ok(())
}

/// The contract registry a build wires into its checker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    pub(crate) connectors: Vec<ConnectorContract>,
    pub(crate) declared: Vec<&'static str>,
    pub(crate) frontends: Vec<FrontendContract>,
}

impl Registry {
    /// Extends the registry with one more contract sheet — the seam
    /// every M1 connector lands through (`File`, the HTTP pair), and
    /// what a test contract uses to exercise the transport of
    /// resolved port tables. FALLIBLE: a sheet is validated before
    /// it becomes an authority — the checker materializes tables and
    /// defaults from it and never re-checks. A name already present
    /// is replaced.
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
