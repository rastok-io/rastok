//! Structural validation of an [`ExecutionPlan`] — every invariant
//! the machine dereferences without checking and every canonical
//! order the plan documents, held once, at a boundary, by a type.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};

use crate::cel;
use crate::node;
use crate::plan::{Emission, EndpointRef, ExecutionPlan, Node, NodeKind, Slot, Step};

/// Why a plan is not executable — one named invariant violation. The
/// variants are semantic, not prose: a caller can match on the class
/// of defect; `Display` renders the human diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    /// A slot indexes past the node table.
    SlotOutOfBounds {
        /// The impossible reference.
        slot: Slot,
        /// The table it missed.
        node_count: usize,
    },
    /// Two nodes share a path.
    DuplicateNodePath {
        /// The repeated path.
        path: node::Path,
    },
    /// One node declares a port twice.
    DuplicatePort {
        /// The declaring node.
        node: node::Path,
        /// The repeated port.
        port: node::Port,
    },
    /// One node declares a var twice.
    DuplicateVar {
        /// The declaring node.
        node: node::Path,
        /// The repeated name.
        name: String,
    },
    /// The boundary declares a port name twice.
    DuplicateBoundaryPort {
        /// The repeated port.
        port: node::Port,
    },
    /// A connector declares an attribute twice.
    DuplicateAttribute {
        /// The declaring node.
        node: node::Path,
        /// The repeated attribute.
        name: String,
    },
    /// One node declares two handlers for one activation — the
    /// second would silently shadow the first at dispatch.
    DuplicateHandler {
        /// The declaring node.
        node: node::Path,
        /// The doubled activation port.
        activation: node::Port,
    },
    /// A connector's error set names one kind twice.
    DuplicateErrorKind {
        /// The declaring node.
        node: node::Path,
        /// The repeated kind.
        kind: String,
    },
    /// A CEL map literal names one key twice.
    DuplicateMapKey {
        /// The repeated key.
        key: String,
    },
    /// A port table declares one of the implicit defaults — `in`,
    /// `out`, and `err` are always present and never declared, on
    /// nodes and at the boundary alike.
    ImplicitPortDeclared {
        /// The declaring node; `None` is the boundary.
        node: Option<node::Path>,
        /// The implicit port.
        port: node::Port,
    },
    /// A referenced port exists, but not in the referenced role —
    /// an edge from an input, an edge into an output, an activation
    /// on an output, an emission on an input.
    WrongPortDirection {
        /// The owning node; `None` is the boundary.
        node: Option<node::Path>,
        /// The port.
        port: node::Port,
        /// The role the reference required.
        expected: node::Direction,
    },
    /// An edge references a boundary port the plan does not know.
    UnknownBoundaryPort {
        /// The unknown port.
        port: node::Port,
    },
    /// A handler writes a var its node does not declare.
    UnknownVariable {
        /// The writing node.
        node: node::Path,
        /// The undeclared name.
        name: String,
    },
    /// A collection the plan documents as canonically ordered is not.
    NonCanonicalOrder {
        /// Which collection broke the order.
        collection: &'static str,
    },
    /// The plan's canonical body would nest beyond the format
    /// reader's budget — the one plan `encode` would write that
    /// `decode` could not read back, refused here so it cannot exist.
    NestingBudgetExceeded {
        /// The deepest level the body would reach (root = 0).
        depth: usize,
        /// The reader's budget ([`crate::canonical_cbor::MAX_NESTING`]).
        budget: usize,
    },
}

impl core::fmt::Display for PlanValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SlotOutOfBounds { slot, node_count } => write!(
                f,
                "invalid plan: slot {} of {node_count} nodes",
                slot.index()
            ),
            Self::DuplicateNodePath { path } => {
                write!(f, "invalid plan: duplicate node path `{path}`")
            }
            Self::DuplicatePort { node, port } => {
                write!(f, "invalid plan: duplicate port `{port}` on `{node}`")
            }
            Self::DuplicateVar { node, name } => {
                write!(f, "invalid plan: duplicate var `{name}` on `{node}`")
            }
            Self::DuplicateBoundaryPort { port } => {
                write!(f, "invalid plan: duplicate boundary port `{port}`")
            }
            Self::DuplicateAttribute { node, name } => {
                write!(f, "invalid plan: duplicate attribute `{name}` on `{node}`")
            }
            Self::DuplicateHandler { node, activation } => {
                write!(
                    f,
                    "invalid plan: `{node}` declares two handlers on `{activation}`"
                )
            }
            Self::DuplicateErrorKind { node, kind } => {
                write!(f, "invalid plan: duplicate error kind `{kind}` on `{node}`")
            }
            Self::DuplicateMapKey { key } => {
                write!(f, "invalid plan: duplicate map literal key `{key}`")
            }
            Self::ImplicitPortDeclared { node, port } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}` declares the implicit port `{port}`"
                ),
                None => write!(
                    f,
                    "invalid plan: the boundary declares the implicit port `{port}`"
                ),
            },
            Self::WrongPortDirection {
                node,
                port,
                expected,
            } => match node {
                Some(node) => write!(
                    f,
                    "invalid plan: `{node}.{port}` is not usable as {expected:?}"
                ),
                None => write!(
                    f,
                    "invalid plan: boundary port `{port}` is not usable as graph {expected:?}"
                ),
            },
            Self::UnknownBoundaryPort { port } => {
                write!(f, "invalid plan: undeclared boundary port `{port}`")
            }
            Self::UnknownVariable { node, name } => {
                write!(f, "invalid plan: `{node}` writes undeclared var `{name}`")
            }
            Self::NonCanonicalOrder { collection } => {
                write!(f, "invalid plan: `{collection}` is not in canonical order")
            }
            Self::NestingBudgetExceeded { depth, budget } => write!(
                f,
                "invalid plan: the body nests {depth} levels deep; the format reads at most {budget}"
            ),
        }
    }
}

impl core::error::Error for PlanValidationError {}

/// An [`ExecutionPlan`] whose structural invariants have been
/// verified: every slot in bounds, every referenced port declared in
/// the referenced role — boundary conventions inverted, as the graph
/// sees them — every handler owned by its node, every state write
/// resolvable, names unique, and every collection the plan documents
/// as canonical actually in canonical order. The field is private
/// and there is no unverified constructor and no mutable access —
/// holding the type IS holding the property. The machine and the
/// codec accept only this type.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedExecutionPlan(ExecutionPlan);

impl ValidatedExecutionPlan {
    /// Validates a raw plan.
    ///
    /// # Errors
    ///
    /// The first violated invariant, named.
    pub fn try_new(plan: ExecutionPlan) -> Result<Self, PlanValidationError> {
        validate(&plan)?;
        Ok(Self(plan))
    }

    /// Read access to the validated plan.
    #[must_use]
    pub fn plan(&self) -> &ExecutionPlan {
        &self.0
    }

    /// Gives the raw plan back — the validity claim does not travel
    /// with it.
    #[must_use]
    pub fn into_inner(self) -> ExecutionPlan {
        self.0
    }
}

impl TryFrom<ExecutionPlan> for ValidatedExecutionPlan {
    type Error = PlanValidationError;

    fn try_from(plan: ExecutionPlan) -> Result<Self, Self::Error> {
        Self::try_new(plan)
    }
}

// Read-only ergonomics: the plan's tables stay reachable through the
// wrapper. There is deliberately NO DerefMut — mutation would void
// the property the type asserts.
impl core::ops::Deref for ValidatedExecutionPlan {
    type Target = ExecutionPlan;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The node ordering the lowering emits: operators before
/// connectors, then by path — mirrored here so a reordered node
/// table refuses instead of minting a second byte form of the same
/// graph.
fn kind_rank(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Operator => 0,
        NodeKind::Connector(_) => 1,
    }
}

fn validate(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    // Canonical orders first: a plan that fails here is a different
    // byte form of some graph, and nothing downstream should mint an
    // opinion about it. Strictly increasing everywhere — ties are
    // duplicates and refuse as such below or here.
    for pair in plan.nodes.windows(2) {
        // Uniqueness before order, here as everywhere: two entries
        // for one path are a DUPLICATE first — the dedicated variant
        // must name the defect it exists for, not hide behind the
        // broken sort the duplication implies.
        if pair[0].path == pair[1].path {
            return Err(PlanValidationError::DuplicateNodePath {
                path: pair[0].path.clone(),
            });
        }
        if (kind_rank(&pair[0].kind), pair[0].path.to_string())
            >= (kind_rank(&pair[1].kind), pair[1].path.to_string())
        {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "nodes",
            });
        }
    }
    for pair in plan.boundary.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(PlanValidationError::DuplicateBoundaryPort {
                port: pair[0].0.clone(),
            });
        }
        if pair[0] >= pair[1] {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "boundary",
            });
        }
    }
    // The implicit trio is exactly that — implicit: `in`, `out`, and
    // `err` are always present and never appear in a declared table,
    // on nodes or at the boundary. The checker holds the surface to
    // the same rule, so a table naming them is a byte form the
    // lowering can never emit.
    let implicit = |port: &node::Port| matches!(port.name(), "in" | "out" | "err");
    if let Some((port, _)) = plan.boundary.iter().find(|(port, _)| implicit(port)) {
        return Err(PlanValidationError::ImplicitPortDeclared {
            node: None,
            port: port.clone(),
        });
    }
    let mut paths = BTreeSet::new();
    for node in &plan.nodes {
        if !paths.insert(node.path.clone()) {
            return Err(PlanValidationError::DuplicateNodePath {
                path: node.path.clone(),
            });
        }
        if let Some((port, _)) = node.ports.iter().find(|(port, _)| implicit(port)) {
            return Err(PlanValidationError::ImplicitPortDeclared {
                node: Some(node.path.clone()),
                port: port.clone(),
            });
        }
        for pair in node.ports.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(PlanValidationError::DuplicatePort {
                    node: node.path.clone(),
                    port: pair[0].0.clone(),
                });
            }
            if pair[0] >= pair[1] {
                return Err(PlanValidationError::NonCanonicalOrder {
                    collection: "ports",
                });
            }
        }
        for pair in node.vars.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(PlanValidationError::DuplicateVar {
                    node: node.path.clone(),
                    name: pair[0].name.clone(),
                });
            }
            if pair[0].name > pair[1].name {
                return Err(PlanValidationError::NonCanonicalOrder { collection: "vars" });
            }
        }
        if let NodeKind::Connector(spec) = &node.kind {
            for pair in spec.attributes.windows(2) {
                if pair[0].0 == pair[1].0 {
                    return Err(PlanValidationError::DuplicateAttribute {
                        node: node.path.clone(),
                        name: pair[0].0.clone(),
                    });
                }
                if pair[0].0 > pair[1].0 {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "attributes",
                    });
                }
            }
            // The error set is a SET — the hash reads it sorted, so
            // an unsorted table would be a second byte form of the
            // same contract.
            for pair in spec.error_set.windows(2) {
                if pair[0] == pair[1] {
                    return Err(PlanValidationError::DuplicateErrorKind {
                        node: node.path.clone(),
                        kind: pair[0].clone(),
                    });
                }
                if pair[0] > pair[1] {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "error_set",
                    });
                }
            }
        }
    }

    // The reader's nesting budget, enforced on the writing side: a
    // plan whose body would nest beyond what `decode` reads cannot
    // earn the type, so `encode ⇒ decode` is total. Measured without
    // recursion — nothing below (nor the codec, nor the hasher) may
    // recurse over the plan before this bound holds.
    let depth = super::serialization::deepest_level(plan);
    if depth >= crate::canonical_cbor::MAX_NESTING {
        return Err(PlanValidationError::NestingBudgetExceeded {
            depth,
            budget: crate::canonical_cbor::MAX_NESTING,
        });
    }

    // References: slots, ports in their roles, handlers, vars. The
    // implicit trio is always available in its fixed directions, and
    // a declared table EXTENDS it — the same union the checker
    // applies to the surface language (`add_port` never adds a
    // default), so the two can never disagree about a program.
    let has_port = |node: &Node, port: &node::Port, direction: node::Direction| match port.name() {
        "in" => direction == node::Direction::Input,
        "out" | "err" => direction == node::Direction::Output,
        _ => node
            .ports
            .iter()
            .any(|(candidate, dir)| candidate == port && *dir == direction),
    };
    let node_at = |slot: Slot| {
        plan.nodes
            .get(slot.index())
            .ok_or(PlanValidationError::SlotOutOfBounds {
                slot,
                node_count: plan.nodes.len(),
            })
    };
    // `position` is the node-side role of the endpoint: Output for a
    // source, Input for a target. The boundary convention INVERTS:
    // an edge sourced at the boundary consumes a graph INPUT, an
    // edge targeting the boundary feeds a graph OUTPUT — the same
    // rule the checker applies to the surface language.
    let endpoint_ok =
        |endpoint: &EndpointRef, position: node::Direction| -> Result<(), PlanValidationError> {
            match endpoint.node {
                Some(slot) => {
                    let node = node_at(slot)?;
                    if !has_port(node, &endpoint.port, position) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: endpoint.port.clone(),
                            expected: position,
                        });
                    }
                    Ok(())
                }
                None => {
                    let expected = match position {
                        node::Direction::Output => node::Direction::Input,
                        node::Direction::Input => node::Direction::Output,
                    };
                    // The implicit trio first — `in` is the graph's
                    // input, `out` and `err` its outputs — then the
                    // declared table, which extends it and (checked
                    // above) never contains it.
                    let actual = match endpoint.port.name() {
                        "in" => Some(node::Direction::Input),
                        "out" | "err" => Some(node::Direction::Output),
                        _ => plan
                            .boundary
                            .iter()
                            .find(|(port, _)| port == &endpoint.port)
                            .map(|(_, declared)| *declared),
                    };
                    match actual {
                        None => Err(PlanValidationError::UnknownBoundaryPort {
                            port: endpoint.port.clone(),
                        }),
                        Some(declared) if declared != expected => {
                            Err(PlanValidationError::WrongPortDirection {
                                node: None,
                                port: endpoint.port.clone(),
                                expected,
                            })
                        }
                        Some(_) => Ok(()),
                    }
                }
            }
        };
    for edge in &plan.edges {
        endpoint_ok(&edge.from, node::Direction::Output)?;
        if let Some(to) = &edge.to {
            endpoint_ok(to, node::Direction::Input)?;
        }
    }
    // The edge order renders through `edge_text`, which dereferences
    // slots — bounds first, order after.
    if !plan
        .edges
        .windows(2)
        .all(|w| plan.edge_text(&w[0]) < plan.edge_text(&w[1]))
    {
        return Err(PlanValidationError::NonCanonicalOrder {
            collection: "edges",
        });
    }
    for handler in &plan.handlers {
        let node = node_at(handler.node)?;
        let node::HandlerActivation::Port(port) = &handler.activation;
        if !has_port(node, port, node::Direction::Input) {
            return Err(PlanValidationError::WrongPortDirection {
                node: Some(node.path.clone()),
                port: port.clone(),
                expected: node::Direction::Input,
            });
        }
        if let Some(guard) = &handler.guard {
            expr_canonical(&guard.root)?;
        }
        for step in &handler.body {
            match step {
                Step::Emit { port, value } => {
                    if !has_port(node, port, node::Direction::Output) {
                        return Err(PlanValidationError::WrongPortDirection {
                            node: Some(node.path.clone()),
                            port: port.clone(),
                            expected: node::Direction::Output,
                        });
                    }
                    match value {
                        Emission::Data(tree) => expr_canonical(&tree.root)?,
                        Emission::Patch(writes) | Emission::Replace(writes) => {
                            for write in writes {
                                expr_canonical(&write.expr.root)?;
                            }
                        }
                        Emission::Identity | Emission::Nothing => {}
                    }
                }
                Step::SetVar { name, expr } => {
                    if !node.vars.iter().any(|var| var.name == *name) {
                        return Err(PlanValidationError::UnknownVariable {
                            node: node.path.clone(),
                            name: name.clone(),
                        });
                    }
                    expr_canonical(&expr.root)?;
                }
                Step::Let { expr, .. } => expr_canonical(&expr.root)?,
            }
        }
    }
    // Handler order is canonical — (slot, activation key), the same
    // key the lowering and the hash sort by, because declaration
    // order between handlers carries neither semantics nor identity.
    // A tie is one activation fired twice: dispatch would silently
    // shadow the second, so it refuses by name. Slot bounds are
    // already proven above.
    for pair in plan.handlers.windows(2) {
        let key =
            |handler: &crate::plan::Handler| (handler.node.index(), handler.activation.to_string());
        if key(&pair[0]) == key(&pair[1]) {
            let node::HandlerActivation::Port(port) = &pair[0].activation;
            return Err(PlanValidationError::DuplicateHandler {
                node: node_at(pair[0].node)?.path.clone(),
                activation: port.clone(),
            });
        }
        if key(&pair[0]) > key(&pair[1]) {
            return Err(PlanValidationError::NonCanonicalOrder {
                collection: "handlers",
            });
        }
    }
    Ok(())
}

/// Every map literal in a shipped CEL tree is construction-sorted —
/// the parser sorts them, so an unsorted or duplicated literal is a
/// second byte form of the same expression, which the re-encoding
/// check cannot catch (it faithfully reproduces whatever order the
/// tree carries). Recursion is safe here: the nesting budget has
/// already bounded the tree.
fn expr_canonical(expr: &cel::Expr) -> Result<(), PlanValidationError> {
    match expr {
        cel::Expr::Literal(_) | cel::Expr::Path(_) => Ok(()),
        cel::Expr::Unary { expr, .. } => expr_canonical(expr),
        cel::Expr::Binary { left, right, .. } => {
            expr_canonical(left)?;
            expr_canonical(right)
        }
        cel::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_canonical(cond)?;
            expr_canonical(then)?;
            expr_canonical(otherwise)
        }
        cel::Expr::Index { base, index } => {
            expr_canonical(base)?;
            expr_canonical(index)
        }
        cel::Expr::Call { args, .. } => args.iter().try_for_each(expr_canonical),
        cel::Expr::List(items) => items.iter().try_for_each(expr_canonical),
        cel::Expr::MapLit(pairs) => {
            for pair in pairs.windows(2) {
                if pair[0].0 == pair[1].0 {
                    return Err(PlanValidationError::DuplicateMapKey {
                        key: pair[0].0.clone(),
                    });
                }
                if pair[0].0 > pair[1].0 {
                    return Err(PlanValidationError::NonCanonicalOrder {
                        collection: "map literal",
                    });
                }
            }
            pairs
                .iter()
                .map(|(_, value)| value)
                .try_for_each(expr_canonical)
        }
    }
}
