//! Shared graph and execution vocabulary.
//!
//! Governing rules: [`grammar.md`] for the node and port vocabulary,
//! [`extensions.md`] for owner contributions, [`stdlib.md`] §2
//! for the `behavior` attribute, and [`envelope.md`] §3 for the trigger
//! namespaces.
//!
//! [`grammar.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/grammar.md
//! [`extensions.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/extensions.md
//! [`stdlib.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/stdlib.md
//! [`envelope.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/envelope.md

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::datatype::{DataType, StructType, TaggedCase, TaggedUnionType, TypeError};

/// Error returned when a name would break the vocabulary's structural
/// invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidName {
    reason: &'static str,
}

impl fmt::Display for InvalidName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason)
    }
}

impl core::error::Error for InvalidName {}

/// Stable root-relative node address.
///
/// A node is addressed by the chain of declaration names from the `: App`
/// root graph down to itself: a `codex` connector declared inside a
/// `sessions` graph is `sessions.codex`. The address names the node in
/// the composition tree — it is not a wiring route; edges connect
/// `(Path, Port)` endpoints.
///
/// The dotted rendering enters the identity recipes, so it must be
/// injective: segments may not be empty or contain `.`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Path {
    segments: Vec<String>,
}

impl Path {
    /// Builds a path from root-relative segments.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] if no segment is given, or if a segment is
    /// empty or contains `.`.
    pub fn new<I, S>(segments: I) -> Result<Self, InvalidName>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segments: Vec<String> = segments.into_iter().map(Into::into).collect();
        if segments.is_empty() {
            return Err(InvalidName {
                reason: "a path needs at least one segment",
            });
        }
        for segment in &segments {
            if segment.is_empty() {
                return Err(InvalidName {
                    reason: "a path segment cannot be empty",
                });
            }
            if segment.contains('.') {
                return Err(InvalidName {
                    reason: "a path segment cannot contain '.'",
                });
            }
        }
        Ok(Self { segments })
    }

    /// The root-relative segments, outermost first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.segments.join("."))
    }
}

/// Root-relative lexical owner path. The root App Graph is the empty path;
/// every nested Graph or Node appends one non-empty segment. This differs
/// deliberately from [`Path`], whose empty form is never a Node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerPath(Vec<String>);

impl OwnerPath {
    /// The root App Graph path.
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Builds a root-relative owner path.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] when a segment is empty or contains `.`.
    pub fn new<I, S>(segments: I) -> Result<Self, InvalidName>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let segments: Vec<String> = segments.into_iter().map(Into::into).collect();
        if segments.is_empty() {
            return Ok(Self::root());
        }
        Path::new(segments.clone())?;
        Ok(Self(segments))
    }

    /// The path segments, outermost first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Whether this is the root App Graph.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Enclosing lexical Graph path. The root has no parent.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (!self.0.is_empty()).then(|| Self(self.0[..self.0.len() - 1].to_vec()))
    }

    /// Deepest lexical path shared by both owners.
    #[must_use]
    pub fn common_ancestor(&self, other: &Self) -> Self {
        let length = self
            .0
            .iter()
            .zip(&other.0)
            .take_while(|(left, right)| left == right)
            .count();
        Self(self.0[..length].to_vec())
    }
}

impl From<&Path> for OwnerPath {
    fn from(path: &Path) -> Self {
        Self(path.segments().to_vec())
    }
}

impl fmt::Display for OwnerPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

/// Compile-time resolved owner reference. Like [`crate::plan::Slot`], the freely
/// constructible value carries no validity claim until it sits in a
/// [`crate::plan::ValidatedExecutionPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerSlot(u32);

impl OwnerSlot {
    /// Wraps an owner-table index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The index inside [`crate::plan::ExecutionPlan::owners`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Named node endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Port {
    name: String,
}

impl Port {
    /// Builds a port from its declared or inferred name.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] if the name is empty.
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidName> {
        let name = name.into();
        if name.is_empty() {
            return Err(InvalidName {
                reason: "a port name cannot be empty",
            });
        }
        Ok(Self { name })
    }

    /// Conventional connector output carrying primary result data.
    ///
    /// This is only an infallible spelling of the ordinary name `out`; it
    /// grants the port no special routing or completion semantics.
    #[must_use]
    pub fn conventional_out() -> Self {
        Self {
            name: String::from("out"),
        }
    }

    /// Conventional connector output carrying diagnostic data.
    ///
    /// This is only an infallible spelling of the ordinary name `err`.
    #[must_use]
    pub fn conventional_err() -> Self {
        Self {
            name: String::from("err"),
        }
    }

    /// Conventional connector output carrying the terminal invocation fact.
    ///
    /// Closure remains a control fact; this constructor only names the
    /// ordinary `exit` data port.
    #[must_use]
    pub fn conventional_exit() -> Self {
        Self {
            name: String::from("exit"),
        }
    }

    /// The port name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// Semantic direction of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// The port receives envelopes.
    Input,
    /// The port emits envelopes.
    Output,
}

/// Category of a node declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Pure graph-internal computation.
    Operator,
    /// Node allowed to talk to an external system.
    Connector,
    /// Composed subgraph exposed as one node.
    Graph,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Operator => "operator",
            Self::Connector => "connector",
            Self::Graph => "graph",
        })
    }
}

/// Closed stdlib Extension vocabulary.
///
/// An Extension never changes whether its owner is a Graph, Operator, or
/// Connector. It adds a separately validated protocol to that Node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Extension {
    /// The deployed unit's unique public root Graph.
    App,
    /// Finite temporal recursion over an admissible Node's entry interface.
    Loop,
    /// Bounded protected repetition of one outbound Call operation.
    Retry,
    /// Journal-folded compensation workflow.
    Saga,
}

impl Extension {
    /// Resolves one exact source spelling.
    #[must_use]
    pub const fn from_source_name(name: &str) -> Option<Self> {
        match name.as_bytes() {
            b"App" => Some(Self::App),
            b"Loop" => Some(Self::Loop),
            b"Retry" => Some(Self::Retry),
            b"Saga" => Some(Self::Saga),
            _ => None,
        }
    }

    /// The canonical source spelling.
    #[must_use]
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::App => "App",
            Self::Loop => "Loop",
            Self::Retry => "Retry",
            Self::Saga => "Saga",
        }
    }
}

impl fmt::Display for Extension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.source_name())
    }
}

#[cfg(test)]
mod taxonomy_tests {
    use super::Extension;

    #[test]
    fn extension_source_names_are_closed_and_canonical() {
        for extension in [Extension::App, Extension::Loop, Extension::Saga] {
            assert_eq!(
                Extension::from_source_name(extension.source_name()),
                Some(extension)
            );
        }
        assert_eq!(Extension::from_source_name("Effect"), None);
        assert_eq!(Extension::from_source_name("loop"), None);
    }
}

/// Behavior declared by the DSL attribute `behavior`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Behavior {
    /// Same inputs, same outputs — replayable by re-execution.
    Deterministic,
    /// Outcome may vary — replayable only from the journal.
    Probabilistic,
}

impl fmt::Display for Behavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Deterministic => "deterministic",
            Self::Probabilistic => "probabilistic",
        })
    }
}

/// Lifecycle transaction kind — the closed `l:` registry of
/// [`envelope.md`] §3.
///
/// [`envelope.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/envelope.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Lifecycle {
    /// Internal Retry manager fact; never an authored handler activation.
    Retry = 10,
    /// Instance start.
    Start = 1,
    /// Instance stop.
    Stop = 2,
    /// Durable request that enters bounded Closing. This is an internal
    /// runtime trigger, never a source handler activation.
    Closing = 7,
    /// Terminal observation of an admitted inbound Call. This is an internal
    /// Connector transaction trigger, never a source handler activation.
    Reply = 8,
    /// Sole terminal observation of one Session. This is an internal
    /// Connector transaction trigger, never a source handler activation.
    Exit = 9,
    /// Loop continuation transaction.
    Next = 3,
    /// Expired per-call timeout budget.
    Timeout = 4,
    /// Instance-state eviction.
    Evict = 5,
    /// Post-crash recovery transaction.
    Recover = 6,
}

impl From<Lifecycle> for u8 {
    fn from(kind: Lifecycle) -> Self {
        kind as u8
    }
}

/// A byte which does not name an allocated lifecycle transaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownLifecycle(pub u8);

impl fmt::Display for UnknownLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown lifecycle kind byte {}", self.0)
    }
}

impl core::error::Error for UnknownLifecycle {}

impl TryFrom<u8> for Lifecycle {
    type Error = UnknownLifecycle;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Start),
            2 => Ok(Self::Stop),
            3 => Ok(Self::Next),
            4 => Ok(Self::Timeout),
            5 => Ok(Self::Evict),
            6 => Ok(Self::Recover),
            7 => Ok(Self::Closing),
            8 => Ok(Self::Reply),
            9 => Ok(Self::Exit),
            10 => Ok(Self::Retry),
            other => Err(UnknownLifecycle(other)),
        }
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Retry => "retry",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Closing => "closing",
            Self::Reply => "reply",
            Self::Exit => "exit",
            Self::Next => "next",
            Self::Timeout => "timeout",
            Self::Evict => "evict",
            Self::Recover => "recover",
        })
    }
}

/// What fired a node transaction. A transaction property, never an
/// envelope field.
///
/// The rendering is namespaced (`p:` / `l:`) so port names and lifecycle
/// names stay collision-free forever.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// Arrival on a port; for a join, the port of the triggering envelope.
    Port(Port),
    /// Transaction not triggered by a port arrival.
    Lifecycle(Lifecycle),
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Port(port) => write!(f, "p:{port}"),
            Self::Lifecycle(lifecycle) => write!(f, "l:{lifecycle}"),
        }
    }
}

/// What a handler declares it fires on — the *activation pattern* —
/// as distinct from [`Trigger`], the *cause* a committed transaction
/// records. The two coincide for a single-port handler, and diverge
/// the moment joins exist: `on (a, b)` will activate on the pair
/// while the causal trigger of its transaction remains the port of
/// the last-arrived envelope.
///
/// Join groundwork — invariants fixed now so the variant lands
/// without reshaping this seam:
///
/// - a join *wait* must not hold the node's mailbox claimed; claims
///   are for open external calls only, and partial arrivals keep
///   consuming;
/// - partial join inputs live in durable internal state owned by the
///   runtime, never in the Rastok expression-visible variable space;
/// - the canonical order of a join's inputs is the port order
///   declared in the handler, and every derived form (ids, hashes,
///   bindings) reads them in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerActivation {
    /// Arrival on a single port.
    Port(Port),
    /// Presence of one delivery on every port, inside one owning Operator
    /// instance. Port order is semantic: it fixes bindings, contribution
    /// order, and the activation identity. Bounds retain candidates; they
    /// never select another instance.
    Join {
        /// Ports in source-declared order.
        ports: Vec<Port>,
        /// Maximum retention in virtual nanoseconds, when declared.
        within_ns: Option<i64>,
        /// Maximum retained candidates per port, when declared.
        last: Option<u32>,
    },
    /// One lexical-owner instance lifecycle transition.
    Lifecycle(Lifecycle),
}

impl HandlerActivation {
    /// The canonical identity key — what the lowering sorts by, the
    /// semantic hash reads, and the plan validator audits. This
    /// string is IDENTITY material and must not move; `Display` is
    /// presentation and may. A future join activation extends this
    /// key PRESERVING its declared port order — `(a, b)` and
    /// `(b, a)` are different identities (identity.md), while the
    /// order between handlers stays insensitive.
    #[must_use]
    pub fn canonical_key(&self) -> alloc::string::String {
        match self {
            Self::Port(port) => alloc::format!("p:{port}"),
            Self::Join { ports, .. } => {
                let mut key = String::from("j:(");
                for (index, port) in ports.iter().enumerate() {
                    if index != 0 {
                        key.push(',');
                    }
                    key.push_str(port.name());
                }
                key.push(')');
                key
            }
            Self::Lifecycle(lifecycle) => alloc::format!("l:{lifecycle}"),
        }
    }
}

impl fmt::Display for HandlerActivation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Coincides with the canonical key today; only the key is
        // pinned.
        match self {
            Self::Port(port) => write!(f, "p:{port}"),
            Self::Join { ports, .. } => {
                f.write_str("j:(")?;
                for (index, port) in ports.iter().enumerate() {
                    if index != 0 {
                        f.write_str(",")?;
                    }
                    port.fmt(f)?;
                }
                f.write_str(")")
            }
            Self::Lifecycle(lifecycle) => write!(f, "l:{lifecycle}"),
        }
    }
}

/// Canonical logical type of the conventional join-expiry output. It is an
/// ordinary one-case tagged union using the same port/type machinery as every
/// other output. The executable M1.4 surface requires the port name
/// `expired`; that is a v1 convention, not a privileged error category.
///
/// # Errors
///
/// Returns the canonical type constructor's invariant violation.
pub fn join_expired_type() -> Result<DataType, TypeError> {
    let payload = StructType::new(Vec::new())?;
    let case = TaggedCase::new("JoinExpired", payload)?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        case,
    ])?))
}

/// Canonical manager outcome carried by a Loop host's terminal union.
///
/// The payload keeps the last proposed iteration state under the exact input
/// carrier. It is an ordinary one-case tagged union, not a contributed port.
///
/// # Errors
///
/// Returns the canonical type constructor's invariant violation.
pub fn loop_expired_type(state: DataType) -> Result<DataType, TypeError> {
    let payload = StructType::new(alloc::vec![crate::datatype::TypeField::new(
        "last_state",
        state,
    )])?;
    let case = TaggedCase::new("LoopExpired", payload)?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        case,
    ])?))
}

/// Extends a Call terminal without changing the Driver's production contract.
///
/// # Errors
/// A conflicting case or the canonical union member budget refuses.
pub fn loop_call_exit_type(
    terminal: &TaggedUnionType,
    state: DataType,
) -> Result<DataType, TypeError> {
    let mut cases = terminal.cases().to_vec();
    cases.push(TaggedCase::new(
        "LoopExpired",
        StructType::new(alloc::vec![crate::datatype::TypeField::new(
            "last_state",
            state
        ),])?,
    )?);
    TaggedUnionType::new(cases).map(DataType::TaggedUnion)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_wire_bytes_are_explicit_and_unknown_bytes_refuse() {
        use super::{Lifecycle, UnknownLifecycle};
        let registry = [
            (1, Lifecycle::Start),
            (2, Lifecycle::Stop),
            (3, Lifecycle::Next),
            (4, Lifecycle::Timeout),
            (5, Lifecycle::Evict),
            (6, Lifecycle::Recover),
            (7, Lifecycle::Closing),
            (8, Lifecycle::Reply),
            (9, Lifecycle::Exit),
            (10, Lifecycle::Retry),
        ];
        for byte in 0..=u8::MAX {
            match registry.iter().find(|(tag, _)| *tag == byte) {
                Some((_, kind)) => {
                    assert_eq!(u8::from(*kind), byte);
                    assert_eq!(Lifecycle::try_from(byte), Ok(*kind));
                }
                None => assert_eq!(Lifecycle::try_from(byte), Err(UnknownLifecycle(byte))),
            }
        }
    }

    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn path_renders_dotted() {
        let path = Path::new(["billing", "retry"]).unwrap();
        assert_eq!(path.to_string(), "billing.retry");
        assert_eq!(path.segments(), ["billing", "retry"]);
    }

    #[test]
    fn path_rejects_degenerate_shapes() {
        assert!(Path::new(Vec::<String>::new()).is_err());
        assert!(Path::new(["billing", ""]).is_err());
        assert!(Path::new(["billing.retry"]).is_err());
    }

    #[test]
    fn path_ordering_is_segment_wise() {
        let parent = Path::new(["a"]).unwrap();
        let child = Path::new(["a", "b"]).unwrap();
        let sibling = Path::new(["b"]).unwrap();
        assert!(parent < child);
        assert!(child < sibling);
    }

    #[test]
    fn path_equality_ignores_construction_route() {
        let from_slices = Path::new(["a", "b"]).unwrap();
        let from_strings = Path::new(vec![String::from("a"), String::from("b")]).unwrap();
        assert_eq!(from_slices, from_strings);
    }

    #[test]
    fn port_holds_its_name() {
        let port = Port::new("out").unwrap();
        assert_eq!(port.name(), "out");
        assert_eq!(port.to_string(), "out");
        assert!(Port::new("").is_err());
    }

    #[test]
    fn port_ordering_is_by_name() {
        assert!(Port::new("err").unwrap() < Port::new("in").unwrap());
        assert!(Port::new("in").unwrap() < Port::new("out").unwrap());
    }

    #[test]
    fn trigger_namespaces_stay_collision_free() {
        let port = Trigger::Port(Port::new("start").unwrap());
        let lifecycle = Trigger::Lifecycle(Lifecycle::Start);
        assert_ne!(port, lifecycle);
        assert_eq!(port.to_string(), "p:start");
        assert_eq!(lifecycle.to_string(), "l:start");
    }

    #[test]
    fn declared_vocabulary_spellings_are_normative() {
        assert_eq!(Kind::Connector.to_string(), "connector");
        assert_eq!(Behavior::Probabilistic.to_string(), "probabilistic");
    }

    #[test]
    fn loop_expiry_keeps_the_exact_iteration_carrier() {
        let DataType::TaggedUnion(union) = loop_expired_type(DataType::Text).unwrap() else {
            panic!("LoopExpired is a tagged union");
        };
        let [case] = union.cases() else {
            panic!("LoopExpired has one terminal case");
        };
        assert_eq!(case.name(), "LoopExpired");
        let [field] = case.payload().fields() else {
            panic!("LoopExpired keeps one last_state field");
        };
        assert_eq!(field.name(), "last_state");
        assert_eq!(field.ty(), &DataType::Text);
    }
}
