//! Shared graph and execution vocabulary.
//!
//! Governing rules: [`grammar.md`] for the node and port vocabulary,
//! [`effects-and-sagas.md`] §2.2 for the effect classes, [`stdlib.md`] §2
//! for the `behavior` attribute, and [`envelope.md`] §3 for the trigger
//! namespaces.
//!
//! [`grammar.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/grammar.md
//! [`effects-and-sagas.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/effects-and-sagas.md
//! [`stdlib.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/stdlib.md
//! [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md

use std::fmt;

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

impl std::error::Error for InvalidName {}

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

/// External-effect risk category of a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectClass {
    /// Safe to repeat: replaying the call cannot change the outcome.
    Idempotent,
    /// Repeatable only through an explicit compensation.
    Compensable,
    /// Neither repeatable nor compensable.
    Irreversible,
}

impl fmt::Display for EffectClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idempotent => "idempotent",
            Self::Compensable => "compensable",
            Self::Irreversible => "irreversible",
        })
    }
}

/// Declared effect of a connector: an explicit absence, or one of the
/// three classes. The three classes stay three; `none` is not a fourth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    /// Explicitly declared absence of external effect.
    None,
    /// Declared external effect of the given class.
    Class(EffectClass),
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Class(class) => class.fmt(f),
        }
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
/// [`envelope.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/envelope.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lifecycle {
    /// Instance start.
    Start,
    /// Instance stop.
    Stop,
    /// Saga compensation transaction.
    Compensate,
    /// Loop continuation transaction.
    Next,
    /// Expired per-call timeout budget.
    Timeout,
    /// Partition-state eviction.
    Evict,
    /// Post-crash recovery transaction.
    Recover,
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Compensate => "compensate",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Effect::None.to_string(), "none");
        assert_eq!(
            Effect::Class(EffectClass::Compensable).to_string(),
            "compensable"
        );
        assert_eq!(Behavior::Probabilistic.to_string(), "probabilistic");
    }
}
