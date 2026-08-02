//! Serializable compiled execution model.
//!
//! The `ExecutionPlan` is what ships to runtime hosts: tables plus
//! canonical CEL trees — no closures, no host references. Attribute
//! constants are the boot-time reductions; the semantic identity was
//! taken earlier, over the canonical HIR (`identity.md` §1), and rides
//! beside the plan as [`SemanticHash`].

use crate::cel;
use crate::node;
use std::fmt;

/// Compile-time resolved node reference; runtime resolution errors
/// are impossible by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slot(u32);

impl Slot {
    /// Wraps a node index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// The node index inside [`ExecutionPlan::nodes`].
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The compiled graph, in the database sense: tables of nodes, edges,
/// and handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    /// The nodes, in canonical order; [`Slot`]s index this table.
    pub nodes: Vec<Node>,
    /// The wiring, desugared to explicit pairs.
    pub edges: Vec<Edge>,
    /// Every handler of every node, in per-node declaration order.
    pub handlers: Vec<Handler>,
}

/// One planned node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address — id-recipe material.
    pub path: node::Path,
    /// Operator or connector.
    pub kind: node::Kind,
    /// The connector configuration, when a connector.
    pub connector: Option<ConnectorSpec>,
    /// Declared state cells with their boot-time defaults.
    pub vars: Vec<VarCell>,
}

/// A connector's planned configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorSpec {
    /// The contract sheet name (`Process`).
    pub contract: String,
    /// The declared attributes, reduced.
    pub attributes: Vec<(String, AttrSetting)>,
}

impl ConnectorSpec {
    /// The setting of one attribute, if declared.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&AttrSetting> {
        self.attributes
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, setting)| setting)
    }
}

/// One reduced attribute setting.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrSetting {
    /// A bare enum-like keyword.
    Word(String),
    /// A reduced constant.
    Value(cel::Value),
}

/// One planned state cell.
#[derive(Debug, Clone, PartialEq)]
pub struct VarCell {
    /// The cell name.
    pub name: String,
    /// The cell's CEL-facing type.
    pub ty: cel::Type,
    /// The boot-time default, when declared.
    pub init: Option<cel::Value>,
}

/// One planned handler: a triggered, ordered statement body.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    /// The owning node.
    pub node: Slot,
    /// What fires it.
    pub trigger: node::Trigger,
    /// The compiled guard, when present.
    pub guard: Option<cel::Tree>,
    /// The steps, in text order — the order fixing emission indices.
    pub body: Vec<Step>,
}

/// One executable step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Transaction-local binding.
    Let {
        /// The binding name.
        name: String,
        /// The expression.
        expr: cel::Tree,
    },
    /// Emission on a port.
    Emit {
        /// The port.
        port: node::Port,
        /// What is emitted.
        value: Emission,
    },
    /// State-cell write.
    SetVar {
        /// The cell name.
        name: String,
        /// The expression.
        expr: cel::Tree,
    },
}

/// What an emission builds.
#[derive(Debug, Clone, PartialEq)]
pub enum Emission {
    /// Re-emit the input unchanged.
    Identity,
    /// Inherit the input; patch the listed fields.
    Patch(Vec<FieldWrite>),
    /// Full-data replacement; meta untouched.
    Replace(Vec<FieldWrite>),
    /// A computed value becoming the whole `data`.
    Data(cel::Tree),
    /// No emission on this port this transaction.
    Nothing,
}

/// One writable field assignment inside an emission.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldWrite {
    /// The dotted writable path.
    pub path: Vec<String>,
    /// The expression.
    pub expr: cel::Tree,
}

/// One planned edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// The emitting end.
    pub from: EndpointRef,
    /// The receiving end; `None` is the explicit discard.
    pub to: Option<EndpointRef>,
}

/// One resolved edge end; a boundary end has no node.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointRef {
    /// The node, or `None` for the graph's boundary.
    pub node: Option<Slot>,
    /// The port.
    pub port: node::Port,
}

/// Diagnostic and trace symbol information — a side table, never
/// identity material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    /// Slot index to node path text.
    pub nodes: Vec<String>,
}

/// `blake3(canonical HIR)` — the full 256-bit semantic identity of a
/// graph (`identity.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticHash([u8; 32]);

impl SemanticHash {
    /// Wraps a digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for SemanticHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Sequential build/deploy number — a flyweight alias of the semantic
/// hash; the manifest is the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphVersion(pub u64);

/// The append-only (version ↔ hash) ledger. In embedded mode — every
/// simulation included — it is re-seeded at each boot, so the first
/// allocation is deterministically version 1 and golden traces stay
/// stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    rows: Vec<(GraphVersion, SemanticHash)>,
}

impl Manifest {
    /// A fresh embedded-mode manifest.
    #[must_use]
    pub fn embedded() -> Self {
        Self::default()
    }

    /// Allocates the next version for a deployment of `hash`: always
    /// `max + 1`, append-only — a rollback receives a new number.
    pub fn allocate(&mut self, hash: SemanticHash) -> GraphVersion {
        let next = GraphVersion(self.rows.last().map_or(1, |(version, _)| version.0 + 1));
        self.rows.push((next, hash));
        next
    }

    /// The hash a version points at, if allocated.
    #[must_use]
    pub fn hash_of(&self, version: GraphVersion) -> Option<SemanticHash> {
        self.rows
            .iter()
            .find(|(allocated, _)| *allocated == version)
            .map(|(_, hash)| *hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_allocates_monotonically() {
        let mut manifest = Manifest::embedded();
        let first = manifest.allocate(SemanticHash::from_bytes([1; 32]));
        let second = manifest.allocate(SemanticHash::from_bytes([2; 32]));
        let rollback = manifest.allocate(SemanticHash::from_bytes([1; 32]));
        assert_eq!(first, GraphVersion(1));
        assert_eq!(second, GraphVersion(2));
        assert_eq!(rollback, GraphVersion(3));
        assert_eq!(
            manifest.hash_of(rollback),
            Some(SemanticHash::from_bytes([1; 32]))
        );
    }
}
