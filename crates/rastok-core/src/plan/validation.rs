//! Structural validation of an [`ExecutionPlan`] — the invariants
//! the machine dereferences without checking, held once, at a
//! boundary, by a type.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};

use crate::node;
use crate::plan::{EndpointRef, ExecutionPlan, Node, Slot, Step};

/// Why a plan is not executable — one named invariant violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanValidationError {
    detail: String,
}

impl PlanValidationError {
    fn new(detail: String) -> Self {
        Self { detail }
    }
}

impl core::fmt::Display for PlanValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid plan: {}", self.detail)
    }
}

impl core::error::Error for PlanValidationError {}

/// An [`ExecutionPlan`] whose structural invariants have been
/// verified: every slot in bounds, every referenced port declared
/// with the right direction, every handler owned by its node,
/// activation on an input port, emission on an output port, every
/// state write resolvable, paths and names unique. The field is
/// private and there is no unverified constructor and no mutable
/// access — holding the type IS holding the property. The machine
/// and the serializer accept only this type.
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

/// Holds a plan to the `ExecutionPlan` invariants the machine
/// assumes. Well-shaped structure naming impossible references
/// refuses here, typed — the machine never sees it.
fn validate(plan: &ExecutionPlan) -> Result<(), PlanValidationError> {
    let invalid = |detail: String| Err(PlanValidationError::new(detail));
    // M0 lowers no explicit port table — a node's ports are the
    // implicit trio. A declared table, once present, is the contract.
    let has_port = |node: &Node, port: &node::Port, direction: node::Direction| {
        if node.ports.is_empty() {
            match direction {
                node::Direction::Input => port.name() == "in",
                node::Direction::Output => matches!(port.name(), "out" | "err"),
            }
        } else {
            node.ports
                .iter()
                .any(|(candidate, dir)| candidate == port && *dir == direction)
        }
    };
    let mut paths = BTreeSet::new();
    for node in &plan.nodes {
        if !paths.insert(node.path.to_string()) {
            return invalid(format!("duplicate node path `{}`", node.path));
        }
        let mut ports = BTreeSet::new();
        for (port, _) in &node.ports {
            if !ports.insert(port.to_string()) {
                return invalid(format!("duplicate port `{port}` on `{}`", node.path));
            }
        }
        let mut vars = BTreeSet::new();
        for var in &node.vars {
            if !vars.insert(var.name.as_str()) {
                return invalid(format!("duplicate var `{}` on `{}`", var.name, node.path));
            }
        }
    }
    let node_at = |slot: Slot| plan.nodes.get(slot.index());
    let endpoint_ok =
        |endpoint: &EndpointRef, direction: node::Direction| -> Result<(), PlanValidationError> {
            match endpoint.node {
                Some(slot) => {
                    let Some(node) = node_at(slot) else {
                        return Err(PlanValidationError::new(format!(
                            "edge endpoint names slot {} of {} nodes",
                            slot.index(),
                            plan.nodes.len()
                        )));
                    };
                    if !has_port(node, &endpoint.port, direction) {
                        return Err(PlanValidationError::new(format!(
                            "edge endpoint names `{}.{}` which is not a {direction:?} port",
                            node.path, endpoint.port
                        )));
                    }
                    Ok(())
                }
                None => {
                    // The boundary's implicit trio always exists in
                    // M0; a declared boundary port EXTENDS it (the
                    // checker's semantics), it does not replace it.
                    let known = matches!(endpoint.port.name(), "in" | "out" | "err")
                        || plan.boundary.iter().any(|(port, _)| port == &endpoint.port);
                    if !known {
                        return Err(PlanValidationError::new(format!(
                            "edge endpoint names undeclared boundary port `{}`",
                            endpoint.port
                        )));
                    }
                    Ok(())
                }
            }
        };
    for edge in &plan.edges {
        endpoint_ok(&edge.from, node::Direction::Output)?;
        if let Some(to) = &edge.to {
            endpoint_ok(to, node::Direction::Input)?;
        }
    }
    for handler in &plan.handlers {
        let Some(node) = node_at(handler.node) else {
            return invalid(format!(
                "handler names slot {} of {} nodes",
                handler.node.index(),
                plan.nodes.len()
            ));
        };
        let node::HandlerActivation::Port(port) = &handler.activation;
        if !has_port(node, port, node::Direction::Input) {
            return invalid(format!(
                "handler on `{}` activates on `{port}` which is not an input port",
                node.path
            ));
        }
        for step in &handler.body {
            match step {
                Step::Emit { port, .. } => {
                    if !has_port(node, port, node::Direction::Output) {
                        return invalid(format!(
                            "handler on `{}` emits on `{port}` which is not an output port",
                            node.path
                        ));
                    }
                }
                Step::SetVar { name, .. } => {
                    if !node.vars.iter().any(|var| var.name == *name) {
                        return invalid(format!(
                            "handler on `{}` writes undeclared var `{name}`",
                            node.path
                        ));
                    }
                }
                Step::Let { .. } => {}
            }
        }
    }
    Ok(())
}
