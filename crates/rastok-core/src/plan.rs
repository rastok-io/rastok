//! Serializable compiled execution model.
//!
//! The `ExecutionPlan` is what ships to runtime hosts: tables plus
//! canonical CEL trees — no closures, no host references. Attribute
//! constants are the boot-time reductions; the semantic identity was
//! taken earlier, over the canonical HIR (`identity.md` §1), and rides
//! beside the plan as [`SemanticHash`].

mod serialization;
mod validation;

pub use serialization::{DecodeError, FORMAT_VERSION, MAGIC, decode, encode};
pub use validation::{PlanValidationError, ValidatedExecutionPlan};

use crate::cel;
use crate::node;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Compile-time resolved node reference. The claim is scoped: slots
/// STORED in a [`ValidatedExecutionPlan`] are proven in bounds, but
/// the type itself is freely constructible — so every public API
/// that takes a caller's `Slot` next to a plan VERIFIES it and
/// refuses typed, never indexes on faith.
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

/// One entry of a port table — a node's or the boundary's: the name,
/// the direction, and the OPTIONAL pinned type. A present type is a
/// public contract (identity material, transported in the package);
/// an absent one is inference territory. The canonical order key is
/// `(port, direction)` — the type never decides an order.
#[derive(Debug, Clone, PartialEq)]
pub struct PortDecl {
    /// The port name.
    pub port: node::Port,
    /// The declared direction.
    pub direction: node::Direction,
    /// The pinned type, when annotated.
    pub ty: Option<cel::Type>,
}

impl PortDecl {
    /// An untyped entry — the whole M0 subset.
    #[must_use]
    pub fn untyped(port: node::Port, direction: node::Direction) -> Self {
        Self {
            port,
            direction,
            ty: None,
        }
    }

    /// The canonical order key.
    #[must_use]
    pub fn key(&self) -> (&node::Port, node::Direction) {
        (&self.port, self.direction)
    }
}

/// The compiled graph, in the database sense: tables of nodes, edges,
/// and handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    /// The nodes, in canonical order; [`Slot`]s index this table.
    pub nodes: Vec<Node>,
    /// Boundary ports declared on the graph, in canonical order; the
    /// defaults `in`, `out`, and `err` exist implicitly and are
    /// never in this table.
    pub boundary: Vec<PortDecl>,
    /// The wiring, desugared to explicit pairs, in canonical order —
    /// the same order the semantic hash sees.
    pub edges: Vec<Edge>,
    /// Every handler, in canonical order — (slot, activation key).
    /// Declaration order between handlers carries no identity; only
    /// the statements INSIDE a handler are order-sensitive.
    pub handlers: Vec<Handler>,
    /// The resolved `meta.app` schema — inferred from the graph's
    /// writes, travelling so the validator answers without a
    /// Registry (D-2026-08-10-1): flat dotted field, unified type.
    /// Sorted by field; prefix-free.
    pub app_schema: Vec<(alloc::string::String, cel::Type)>,
    /// The resolved `meta.ext` vocabulary — the standard
    /// `content_type` plus the instantiated sheets' fields.
    pub ext_schema: Vec<(alloc::string::String, cel::Type)>,
}

impl ExecutionPlan {
    /// The canonical text of one edge — the same rendering the
    /// consumed cursors and the budget obligations seal.
    #[must_use]
    pub fn edge_text(&self, edge: &Edge) -> String {
        let end = |endpoint: &EndpointRef| match endpoint.node {
            Some(slot) => format!("{}.{}", self.nodes[slot.index()].path, endpoint.port),
            None => format!(".{}", endpoint.port),
        };
        let to = match &edge.to {
            Some(endpoint) => end(endpoint),
            None => String::from("!discard"),
        };
        format!("{}>{to}", end(&edge.from))
    }
}

/// What a planned node is — the valid states, and only those: an
/// operator never carries a connector spec, a connector never lacks
/// one.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// Pure graph-internal computation.
    Operator,
    /// External-effect adapter under a contract sheet.
    Connector(ConnectorSpec),
}

/// One planned node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Root-relative address — id-recipe material.
    pub path: node::Path,
    /// What the node is.
    pub kind: NodeKind,
    /// The node's semantic port table — declared and inferred, in
    /// canonical order; the defaults `in`, `out`, `err` are implicit.
    pub ports: Vec<PortDecl>,
    /// Declared state cells with their boot-time defaults, in
    /// canonical order.
    pub vars: Vec<VarCell>,
}

impl Node {
    /// The connector configuration, when the node is a connector.
    #[must_use]
    pub fn connector(&self) -> Option<&ConnectorSpec> {
        match &self.kind {
            NodeKind::Connector(spec) => Some(spec),
            NodeKind::Operator => None,
        }
    }
}

/// A connector's planned configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorSpec {
    /// The contract sheet name (`Process`).
    pub contract: String,
    /// The declared effect — typed at the HIR/plan frontier, where
    /// distinctions become irrevocable.
    pub effect: node::Effect,
    /// The declared behavior, likewise typed.
    pub behavior: node::Behavior,
    /// The remaining declared attributes, reduced; the typed axes
    /// above are not repeated here.
    pub attributes: Vec<(String, AttrSetting)>,
    /// The contract's typed error kinds — the runtime validates every
    /// settled failure against this set, so no partner can invent one.
    pub error_set: Vec<String>,
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
    /// The declared activation pattern — not the causal trigger a
    /// transaction records; see [`node::HandlerActivation`].
    pub activation: node::HandlerActivation,
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
    /// The checked landing site.
    pub target: crate::envelope::WriteTarget,
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
    /// `None` when the range is exhausted; monotonicity never wraps.
    pub fn allocate(&mut self, hash: SemanticHash) -> Option<GraphVersion> {
        let next = GraphVersion(match self.rows.last() {
            None => 1,
            Some((version, _)) => version.0.checked_add(1)?,
        });
        self.rows.push((next, hash));
        Some(next)
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
        assert_eq!(first, Some(GraphVersion(1)));
        assert_eq!(second, Some(GraphVersion(2)));
        assert_eq!(rollback, Some(GraphVersion(3)));
        assert_eq!(
            manifest.hash_of(rollback.unwrap()),
            Some(SemanticHash::from_bytes([1; 32]))
        );
    }
}
