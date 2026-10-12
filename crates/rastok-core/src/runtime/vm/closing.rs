//! Stop products drain through the same live sibling instances. Quiescent
//! mailboxes alone do not prove a receiver is finished: an upstream stop
//! handler may still feed it, including through an intermediate Graph.

use super::*;
use crate::instance;

impl Vm<'_> {
    pub(super) fn pending_stop_receivers(&self, scope: &instance::Generation) -> BTreeSet<Slot> {
        let Some(scope_owner) = self.instances.get(scope).map(|instance| instance.owner) else {
            return BTreeSet::new();
        };
        let mut pending = Vec::new();
        for handler in &self.plan.handlers {
            if handler.activation != node::HandlerActivation::Lifecycle(node::Lifecycle::Stop)
                || !self.closing.iter().any(|reference| {
                    self.instances.get(reference).is_some_and(|instance| {
                        instance.owner == self.plan.nodes[handler.owner.index()].owner
                            && self.is_within_instance(reference, scope)
                    })
                })
            {
                continue;
            }
            pending.extend(
                handler
                    .dispatch
                    .steps()
                    .flat_map(|step| step.emissions())
                    .filter(|(_, value)| !matches!(value, crate::plan::EmitValue::Nothing))
                    .map(|(port, _)| (handler.owner, port.clone())),
            );
        }
        let mut visited = BTreeSet::new();
        let mut receivers = BTreeSet::new();
        while let Some((source, port)) = pending.pop() {
            if !visited.insert((source, port.clone())) {
                continue;
            }
            for edge in self
                .plan
                .edges
                .iter()
                .filter(|edge| edge.from.node == Some(source) && edge.from.port == port)
            {
                let Some(target) = &edge.to else { continue };
                let Some(slot) = target.node else { continue };
                let node = &self.plan.nodes[slot.index()];
                // Once a product leaves the closing scope, a later return is
                // external admission, not work for this old generation.
                if !self.owner_chains[node.owner.index()]
                    .as_deref()
                    .is_some_and(|chain| chain.contains(&scope_owner))
                {
                    continue;
                }
                receivers.insert(slot);
                if !node.input_drives_outputs() {
                    continue;
                }
                match &node.execution {
                    NodeExecution::Operator => {
                        for handler in self.plan.handlers.iter().filter(|handler| {
                            handler.owner == slot
                                && match &handler.activation {
                                    node::HandlerActivation::Port(port) => *port == target.port,
                                    node::HandlerActivation::Join { ports, .. } => {
                                        ports.contains(&target.port)
                                    }
                                    node::HandlerActivation::Lifecycle(_) => false,
                                }
                        }) {
                            pending.extend(
                                handler
                                    .dispatch
                                    .steps()
                                    .flat_map(|step| step.emissions())
                                    .filter(|(_, value)| {
                                        !matches!(value, crate::plan::EmitValue::Nothing)
                                    })
                                    .map(|(port, _)| (slot, port.clone())),
                            );
                        }
                    }
                    NodeExecution::Connector(_) => {
                        pending.extend(
                            node.ports
                                .iter()
                                .filter(|port| port.direction == node::PortDirection::Output)
                                .map(|port| (slot, port.port.clone())),
                        );
                    }
                }
            }
        }
        receivers
    }
}
