//! Registry-free derivation of policy tables across canonical plan operations.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::data_policy::{
    self, Environment, PropagationError, alternatives, nest, overlay, replace_data_path,
    security_taint,
};
use crate::datatype::DataType;
use crate::envelope::WriteTarget;
use crate::node;
use crate::schema_annotation::{AnnotationTable, SchemaPathSegment};

use super::type_resolution::ResolvedPortTable;
use super::{
    AspectSpec, Emission, EndpointRef, ExecutionPlan, Handler, HandlerOwner, OwnerKind, OwnerSlot,
    Slot, Step,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolutionError {
    pub(super) location: String,
    pub(super) reason: PropagationError,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PortKey {
    Boundary(node::Port),
    Node(Slot, node::Port),
    Graph(OwnerSlot, node::Port),
}

#[derive(Debug, Clone)]
struct PortState {
    key: PortKey,
    direction: node::Direction,
    ty: DataType,
    declared: AnnotationTable,
    current: AnnotationTable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VarKey {
    node: Slot,
    name: String,
}

#[derive(Debug, Clone)]
struct VarState {
    key: VarKey,
    ty: DataType,
    declared: AnnotationTable,
    current: AnnotationTable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ValueKey {
    owner: OwnerSlot,
    name: String,
}

#[derive(Debug, Clone)]
struct ValueState {
    key: ValueKey,
    ty: DataType,
    declared: AnnotationTable,
    current: AnnotationTable,
}

#[derive(Debug, Clone)]
struct Outcome {
    ports: BTreeMap<PortKey, AnnotationTable>,
}

struct Solver<'a> {
    plan: &'a ExecutionPlan,
    ports: BTreeMap<PortKey, PortState>,
    vars: BTreeMap<VarKey, VarState>,
    values: BTreeMap<ValueKey, ValueState>,
}

impl<'a> Solver<'a> {
    fn new(plan: &'a ExecutionPlan, resolved: &ResolvedPortTable) -> Self {
        let mut ports = BTreeMap::new();
        for port in resolved.boundary() {
            let key = PortKey::Boundary(port.port().clone());
            ports.insert(
                key.clone(),
                PortState {
                    key,
                    direction: port.direction(),
                    ty: port.ty().clone(),
                    declared: port.annotations().clone(),
                    current: port.annotations().clone(),
                },
            );
        }
        for index in 0..plan.nodes.len() {
            let slot = Slot::new(u32::try_from(index).unwrap_or(u32::MAX));
            if let Some(row) = resolved.node(slot) {
                for port in row {
                    let key = PortKey::Node(slot, port.port().clone());
                    ports.insert(
                        key.clone(),
                        PortState {
                            key,
                            direction: port.direction(),
                            ty: port.ty().clone(),
                            declared: port.annotations().clone(),
                            current: port.annotations().clone(),
                        },
                    );
                }
            }
        }
        for (index, owner) in plan.owners.iter().enumerate() {
            if owner.path.is_root() || owner.kind != OwnerKind::Graph {
                continue;
            }
            let slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
            for port in &owner.ports {
                let Some(ty) = &port.ty else {
                    continue;
                };
                let key = PortKey::Graph(slot, port.port.clone());
                ports.insert(
                    key.clone(),
                    PortState {
                        key,
                        direction: port.direction,
                        ty: ty.clone(),
                        declared: port.annotations.clone(),
                        current: port.annotations.clone(),
                    },
                );
            }
        }
        let mut vars = BTreeMap::new();
        for (index, node) in plan.nodes.iter().enumerate() {
            let slot = Slot::new(u32::try_from(index).unwrap_or(u32::MAX));
            for var in &node.vars {
                let key = VarKey {
                    node: slot,
                    name: var.name.clone(),
                };
                vars.insert(
                    key.clone(),
                    VarState {
                        key,
                        ty: var.ty.clone(),
                        declared: var.annotations.clone(),
                        current: var.annotations.clone(),
                    },
                );
            }
        }
        let mut values = BTreeMap::new();
        for (index, owner) in plan.owners.iter().enumerate() {
            let owner_slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
            for value in &owner.vals {
                let key = ValueKey {
                    owner: owner_slot,
                    name: value.name.clone(),
                };
                values.insert(
                    key.clone(),
                    ValueState {
                        key,
                        ty: value.expr.ty.clone(),
                        declared: value.annotations.clone(),
                        current: value.annotations.clone(),
                    },
                );
            }
        }
        Self {
            plan,
            ports,
            vars,
            values,
        }
    }

    fn solve(mut self) -> Result<Outcome, ResolutionError> {
        let limit = self
            .ports
            .len()
            .saturating_add(self.vars.len())
            .saturating_add(self.values.len())
            .saturating_mul(8)
            .saturating_add(8);
        for _ in 0..limit {
            let (port_producers, var_producers, value_producers) = self.derive_pass()?;
            let mut changed = false;
            for state in self.ports.values_mut() {
                let derived = match port_producers.get(&state.key) {
                    None => AnnotationTable::empty(),
                    Some(producers) => {
                        alternatives(producers.clone()).map_err(|reason| ResolutionError {
                            location: port_location(self.plan, &state.key),
                            reason,
                        })?
                    }
                };
                let next =
                    overlay(&derived, &state.declared).map_err(|reason| ResolutionError {
                        location: port_location(self.plan, &state.key),
                        reason,
                    })?;
                changed |= next != state.current;
                state.current = next;
            }
            for state in self.vars.values_mut() {
                let derived = match var_producers.get(&state.key) {
                    None => AnnotationTable::empty(),
                    Some(producers) => {
                        alternatives(producers.clone()).map_err(|reason| ResolutionError {
                            location: var_location(self.plan, &state.key),
                            reason,
                        })?
                    }
                };
                let next =
                    overlay(&derived, &state.declared).map_err(|reason| ResolutionError {
                        location: var_location(self.plan, &state.key),
                        reason,
                    })?;
                changed |= next != state.current;
                state.current = next;
            }
            for state in self.values.values_mut() {
                let derived = match value_producers.get(&state.key) {
                    None => AnnotationTable::empty(),
                    Some(producers) => {
                        alternatives(producers.clone()).map_err(|reason| ResolutionError {
                            location: value_location(self.plan, &state.key),
                            reason,
                        })?
                    }
                };
                let next =
                    overlay(&derived, &state.declared).map_err(|reason| ResolutionError {
                        location: value_location(self.plan, &state.key),
                        reason,
                    })?;
                changed |= next != state.current;
                state.current = next;
            }
            if !changed {
                return Ok(Outcome {
                    ports: self
                        .ports
                        .into_iter()
                        .map(|(key, state)| (key, state.current))
                        .collect(),
                });
            }
        }
        Err(ResolutionError {
            location: String::from("plan data-policy graph"),
            reason: PropagationError::UnresolvedExpression,
        })
    }

    fn derive_pass(&self) -> Result<Pass, ResolutionError> {
        let mut ports = BTreeMap::<PortKey, Vec<AnnotationTable>>::new();
        let mut vars = BTreeMap::<VarKey, Vec<AnnotationTable>>::new();
        let mut values = BTreeMap::<ValueKey, Vec<AnnotationTable>>::new();

        for edge in &self.plan.edges {
            let source = endpoint_key(&edge.from);
            let Some(target) = edge.to.as_ref().map(endpoint_key) else {
                continue;
            };
            let table = self.port(&source)?.current.clone();
            ports.entry(target).or_default().push(table);
        }

        // Temporal recurrence is a route into the next iteration, not a
        // spatial edge. It nevertheless carries the exact same policy.
        for (index, owner) in self.plan.owners.iter().enumerate() {
            if !owner
                .aspects
                .iter()
                .any(|aspect| matches!(aspect, AspectSpec::Loop(_)))
            {
                continue;
            }
            let owner_slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
            let next = self.graph_port_key(owner_slot, "next")?;
            let input = self.graph_port_key(owner_slot, "in")?;
            ports
                .entry(input.clone())
                .or_default()
                .push(self.port(&next)?.current.clone());

            let expired = self.graph_port_key(owner_slot, "expired")?;
            let wrapped = nest(
                &self.port(&input)?.current,
                &[
                    SchemaPathSegment::TaggedUnionCase(String::from("LoopExpired")),
                    SchemaPathSegment::Field(String::from("last_state")),
                ],
            )
            .map_err(|reason| ResolutionError {
                location: port_location(self.plan, &expired),
                reason,
            })?;
            ports.entry(expired).or_default().push(wrapped);
        }

        self.derive_owner_values(&mut values)?;

        for (handler_index, handler) in self.plan.handlers.iter().enumerate() {
            let (mut environment, input) = self.handler_environment(handler)?;
            let guard_taint = match &handler.guard {
                None => AnnotationTable::empty(),
                Some(guard) => security_taint([data_policy::expression(guard, &environment)
                    .map_err(|reason| ResolutionError {
                        location: format!("handler {handler_index} guard"),
                        reason,
                    })?])
                .map_err(|reason| ResolutionError {
                    location: format!("handler {handler_index} guard"),
                    reason,
                })?,
            };
            for step in &handler.body {
                match step {
                    Step::Let {
                        name,
                        expr,
                        annotations,
                    } => {
                        let derived =
                            data_policy::expression(expr, &environment).map_err(|reason| {
                                ResolutionError {
                                    location: format!("handler {handler_index} let `{name}`"),
                                    reason,
                                }
                            })?;
                        let derived =
                            overlay(&derived, &guard_taint).map_err(|reason| ResolutionError {
                                location: format!("handler {handler_index} let `{name}`"),
                                reason,
                            })?;
                        let expected =
                            overlay(&derived, annotations).map_err(|reason| ResolutionError {
                                location: format!("handler {handler_index} let `{name}`"),
                                reason,
                            })?;
                        environment.insert(name.clone(), expr.ty.clone(), expected.clone());
                    }
                    Step::SetVar { name, expr } => {
                        let key = VarKey {
                            node: handler.owner.node().ok_or_else(|| ResolutionError {
                                location: format!("handler {handler_index} var `{name}`"),
                                reason: PropagationError::UnknownBinding(name.clone()),
                            })?,
                            name: name.clone(),
                        };
                        let derived =
                            data_policy::expression(expr, &environment).map_err(|reason| {
                                ResolutionError {
                                    location: var_location(self.plan, &key),
                                    reason,
                                }
                            })?;
                        let derived =
                            overlay(&derived, &guard_taint).map_err(|reason| ResolutionError {
                                location: var_location(self.plan, &key),
                                reason,
                            })?;
                        vars.entry(key).or_default().push(derived);
                    }
                    Step::Emit { port, value } => {
                        let key = self.handler_output(handler.owner, port)?;
                        if let Some(mut derived) = self.emission(
                            value,
                            input.as_ref(),
                            &environment,
                            self.port(&key)?.ty.clone(),
                            handler_index,
                        )? {
                            derived = overlay(&derived, &guard_taint).map_err(|reason| {
                                ResolutionError {
                                    location: port_location(self.plan, &key),
                                    reason,
                                }
                            })?;
                            ports.entry(key).or_default().push(derived);
                        }
                    }
                    Step::Stop => {}
                }
            }
        }
        Ok((ports, vars, values))
    }

    fn derive_owner_values(
        &self,
        producers: &mut BTreeMap<ValueKey, Vec<AnnotationTable>>,
    ) -> Result<(), ResolutionError> {
        for index in 0..self.plan.owners.len() {
            let owner_slot = OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX));
            let inputs = self.owner_inputs(owner_slot);
            if inputs.is_empty() {
                self.derive_owner_values_for_input(owner_slot, None, producers)?;
            } else {
                for input in inputs {
                    self.derive_owner_values_for_input(owner_slot, Some(input), producers)?;
                }
            }
        }
        Ok(())
    }

    fn derive_owner_values_for_input(
        &self,
        owner: OwnerSlot,
        input: Option<&PortState>,
        producers: &mut BTreeMap<ValueKey, Vec<AnnotationTable>>,
    ) -> Result<(), ResolutionError> {
        let mut environment = self.owner_environment(owner, input)?;
        if let Some((ty, annotations)) = self.instance_key_policy(owner)? {
            environment.insert("instance.key", ty, annotations);
        }
        for value in &self.plan.owners[owner.index()].vals {
            let key = ValueKey {
                owner,
                name: value.name.clone(),
            };
            let derived = data_policy::expression(&value.expr, &environment).map_err(|reason| {
                ResolutionError {
                    location: value_location(self.plan, &key),
                    reason,
                }
            })?;
            producers.entry(key).or_default().push(derived);
        }
        Ok(())
    }

    fn owner_inputs(&self, owner: OwnerSlot) -> Vec<&PortState> {
        match self.plan.owners[owner.index()].kind {
            OwnerKind::Node(slot) => self
                .ports
                .values()
                .filter(|state| {
                    state.direction == node::Direction::Input
                        && matches!(state.key, PortKey::Node(candidate, _) if candidate == slot)
                })
                .collect(),
            OwnerKind::Graph => self
                .graph_port_states(owner)
                .into_iter()
                .filter(|state| state.direction == node::Direction::Input)
                .collect(),
        }
    }

    fn owner_environment(
        &self,
        owner: OwnerSlot,
        input: Option<&PortState>,
    ) -> Result<Environment, ResolutionError> {
        let mut environment = Environment::new()
            .with("meta", DataType::Variant, AnnotationTable::empty())
            .with("now", DataType::TIMESTAMP, AnnotationTable::empty());
        if let Some(input) = input {
            environment.insert("data", input.ty.clone(), input.current.clone());
        }
        let mut lineage = Vec::new();
        let mut cursor = Some(owner);
        while let Some(slot) = cursor {
            lineage.push(slot);
            cursor = self.plan.owners[slot.index()].parent;
        }
        for slot in lineage.into_iter().rev() {
            for state in self.values.values().filter(|state| state.key.owner == slot) {
                environment.insert(
                    state.key.name.clone(),
                    state.ty.clone(),
                    state.current.clone(),
                );
            }
        }
        Ok(environment)
    }

    fn instance_key_policy(
        &self,
        owner: OwnerSlot,
    ) -> Result<Option<(DataType, AnnotationTable)>, ResolutionError> {
        let Some(key) = &self.plan.owners[owner.index()].instance.key else {
            return Ok(None);
        };
        let inputs = self.owner_inputs(owner);
        let mut candidates = Vec::new();
        if inputs.is_empty() {
            let environment = self.owner_environment(owner, None)?;
            candidates.push(
                data_policy::expression(key, &environment).map_err(|reason| ResolutionError {
                    location: format!("instance key on `{}`", self.plan.owners[owner.index()].path),
                    reason,
                })?,
            );
        } else {
            for input in inputs {
                let environment = self.owner_environment(owner, Some(input))?;
                candidates.push(
                    data_policy::expression(key, &environment).map_err(|reason| {
                        ResolutionError {
                            location: format!(
                                "instance key on `{}`",
                                self.plan.owners[owner.index()].path
                            ),
                            reason,
                        }
                    })?,
                );
            }
        }
        let annotations = alternatives(candidates).map_err(|reason| ResolutionError {
            location: format!("instance key on `{}`", self.plan.owners[owner.index()].path),
            reason,
        })?;
        Ok(Some((key.ty.clone(), annotations)))
    }

    fn handler_environment(
        &self,
        handler: &Handler,
    ) -> Result<(Environment, Option<AnnotationTable>), ResolutionError> {
        let mut environment = Environment::new()
            .with("meta", DataType::Variant, AnnotationTable::empty())
            .with("now", DataType::TIMESTAMP, AnnotationTable::empty());
        let mut ordinary_input = None;
        match (handler.owner, &handler.activation) {
            (HandlerOwner::Node(slot), node::HandlerActivation::Port(port)) => {
                let state = self.port(&PortKey::Node(slot, port.clone()))?;
                environment.insert("data", state.ty.clone(), state.current.clone());
                ordinary_input = Some(state.current.clone());
            }
            (HandlerOwner::Node(slot), node::HandlerActivation::Join { ports, .. }) => {
                for port in ports {
                    let state = self.port(&PortKey::Node(slot, port.clone()))?;
                    environment.insert(
                        format!("{}.data", port.name()),
                        state.ty.clone(),
                        state.current.clone(),
                    );
                    environment.insert(
                        format!("{}.meta", port.name()),
                        DataType::Variant,
                        AnnotationTable::empty(),
                    );
                }
            }
            (HandlerOwner::Node(slot), node::HandlerActivation::Lifecycle(_)) => {
                let inputs: Vec<_> = self
                    .ports
                    .values()
                    .filter(|state| {
                        matches!(state.key, PortKey::Node(candidate, _)
                            if candidate == slot)
                            && state.direction == node::Direction::Input
                    })
                    .collect();
                if let Some(first) = inputs.first() {
                    let table = alternatives(inputs.iter().map(|state| state.current.clone()))
                        .map_err(|reason| ResolutionError {
                            location: format!(
                                "lifecycle handler on {}",
                                self.plan.nodes[slot.index()].path
                            ),
                            reason,
                        })?;
                    environment.insert("data", first.ty.clone(), table.clone());
                    ordinary_input = Some(table);
                }
            }
            (HandlerOwner::Graph(owner), node::HandlerActivation::Port(port)) => {
                let key = self.graph_port_key(owner, port.name())?;
                let state = self.port(&key)?;
                environment.insert("data", state.ty.clone(), state.current.clone());
                ordinary_input = Some(state.current.clone());
            }
            (HandlerOwner::Graph(_), node::HandlerActivation::ChildPort { child, port }) => {
                let key = self.child_port_key(child, port)?;
                let state = self.port(&key)?;
                environment.insert("data", state.ty.clone(), state.current.clone());
                ordinary_input = Some(state.current.clone());
            }
            (HandlerOwner::Graph(owner), node::HandlerActivation::Lifecycle(_)) => {
                let inputs: Vec<_> = self
                    .graph_port_states(owner)
                    .into_iter()
                    .filter(|state| state.direction == node::Direction::Input)
                    .collect();
                if let Some(first) = inputs.first() {
                    let table = alternatives(inputs.iter().map(|state| state.current.clone()))
                        .map_err(|reason| ResolutionError {
                            location: format!(
                                "Graph lifecycle handler `{}`",
                                self.plan.owners[owner.index()].path
                            ),
                            reason,
                        })?;
                    environment.insert("data", first.ty.clone(), table.clone());
                    ordinary_input = Some(table);
                }
            }
            (HandlerOwner::Graph(_), node::HandlerActivation::Join { .. })
            | (HandlerOwner::Node(_), node::HandlerActivation::ChildPort { .. }) => {}
        }

        let owner = match handler.owner {
            HandlerOwner::Node(slot) => self.plan.nodes[slot.index()].owner,
            HandlerOwner::Graph(owner) => owner,
        };
        let lexical = self.owner_environment(owner, None)?;
        for (name, binding) in lexical.bindings() {
            environment.insert(
                name.clone(),
                binding.ty().clone(),
                binding.annotations().clone(),
            );
        }
        if let Some((ty, annotations)) = self.instance_key_policy(owner)? {
            environment.insert("instance.key", ty, annotations);
        }

        if let HandlerOwner::Node(slot) = handler.owner {
            for state in self.vars.values().filter(|state| state.key.node == slot) {
                environment.insert(
                    state.key.name.clone(),
                    state.ty.clone(),
                    state.current.clone(),
                );
            }
        }
        if let HandlerOwner::Graph(owner) = handler.owner
            && self.plan.owners[owner.index()]
                .aspects
                .iter()
                .any(|aspect| matches!(aspect, AspectSpec::Loop(_)))
        {
            let key = self.graph_port_key(owner, "in")?;
            let state = self.port(&key)?;
            environment.insert("Loop.state.data", state.ty.clone(), state.current.clone());
            environment.insert(
                "Loop.state.meta",
                DataType::Variant,
                AnnotationTable::empty(),
            );
        }
        Ok((environment, ordinary_input))
    }

    fn emission(
        &self,
        emission: &Emission,
        input: Option<&AnnotationTable>,
        environment: &Environment,
        output_type: DataType,
        handler_index: usize,
    ) -> Result<Option<AnnotationTable>, ResolutionError> {
        let location = format!("handler {handler_index} emission");
        let result = match emission {
            Emission::Nothing => return Ok(None),
            Emission::Identity => input.cloned().unwrap_or_default(),
            Emission::Data(tree) => data_policy::expression(tree, environment)
                .map_err(|reason| ResolutionError { location, reason })?,
            Emission::Patch(writes) | Emission::Replace(writes) => {
                let mut result = if matches!(emission, Emission::Patch(_)) {
                    input.cloned().unwrap_or_default()
                } else {
                    AnnotationTable::empty()
                };
                for write in writes {
                    let derived =
                        data_policy::expression(&write.expr, environment).map_err(|reason| {
                            ResolutionError {
                                location: format!("handler {handler_index} field write"),
                                reason,
                            }
                        })?;
                    match &write.target {
                        WriteTarget::Data(path) => {
                            result = replace_data_path(&result, &output_type, path, &derived)
                                .map_err(|reason| ResolutionError {
                                    location: format!("handler {handler_index} data write"),
                                    reason,
                                })?;
                        }
                        WriteTarget::App(_) | WriteTarget::Ext(_) if !derived.is_empty() => {
                            return Err(ResolutionError {
                                location: format!("handler {handler_index} metadata write"),
                                reason: PropagationError::InvalidProjection(String::from(
                                    "annotated data cannot enter metadata without a policy schema",
                                )),
                            });
                        }
                        WriteTarget::App(_) | WriteTarget::Ext(_) => {}
                    }
                }
                result
            }
        };
        Ok(Some(result))
    }

    fn handler_output(
        &self,
        owner: HandlerOwner,
        port: &node::Port,
    ) -> Result<PortKey, ResolutionError> {
        let key = match owner {
            HandlerOwner::Node(slot) => PortKey::Node(slot, port.clone()),
            HandlerOwner::Graph(slot) => self.graph_port_key(slot, port.name())?,
        };
        self.port(&key)?;
        Ok(key)
    }

    fn graph_port_key(&self, owner: OwnerSlot, name: &str) -> Result<PortKey, ResolutionError> {
        let port = node::Port::new(name).map_err(|_| ResolutionError {
            location: format!("Graph `{}`", self.plan.owners[owner.index()].path),
            reason: PropagationError::UnknownBinding(name.into()),
        })?;
        let key = if self.plan.owners[owner.index()].path.is_root() {
            PortKey::Boundary(port)
        } else {
            PortKey::Graph(owner, port)
        };
        self.port(&key)?;
        Ok(key)
    }

    fn graph_port_states(&self, owner: OwnerSlot) -> Vec<&PortState> {
        if self.plan.owners[owner.index()].path.is_root() {
            self.ports
                .values()
                .filter(|state| matches!(state.key, PortKey::Boundary(_)))
                .collect()
        } else {
            self.ports
                .values()
                .filter(
                    |state| matches!(state.key, PortKey::Graph(candidate, _) if candidate == owner),
                )
                .collect()
        }
    }

    fn child_port_key(
        &self,
        child: &node::Path,
        port: &node::Port,
    ) -> Result<PortKey, ResolutionError> {
        if let Some((index, _)) = self
            .plan
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.path == *child)
        {
            let key = PortKey::Node(
                Slot::new(u32::try_from(index).unwrap_or(u32::MAX)),
                port.clone(),
            );
            self.port(&key)?;
            return Ok(key);
        }
        if let Some((index, _)) = self.plan.owners.iter().enumerate().find(|(_, owner)| {
            owner.kind == OwnerKind::Graph && owner.path.segments() == child.segments()
        }) {
            let key = PortKey::Graph(
                OwnerSlot::new(u32::try_from(index).unwrap_or(u32::MAX)),
                port.clone(),
            );
            self.port(&key)?;
            return Ok(key);
        }
        Err(ResolutionError {
            location: format!("child port `{child}.{port}`"),
            reason: PropagationError::UnknownBinding(format!("{child}.{port}")),
        })
    }

    fn port(&self, key: &PortKey) -> Result<&PortState, ResolutionError> {
        self.ports.get(key).ok_or_else(|| ResolutionError {
            location: port_location(self.plan, key),
            reason: PropagationError::UnknownBinding(port_location(self.plan, key)),
        })
    }
}

type Pass = (
    BTreeMap<PortKey, Vec<AnnotationTable>>,
    BTreeMap<VarKey, Vec<AnnotationTable>>,
    BTreeMap<ValueKey, Vec<AnnotationTable>>,
);

fn endpoint_key(endpoint: &EndpointRef) -> PortKey {
    match endpoint.node {
        Some(slot) => PortKey::Node(slot, endpoint.port.clone()),
        None => PortKey::Boundary(endpoint.port.clone()),
    }
}

fn port_location(plan: &ExecutionPlan, key: &PortKey) -> String {
    match key {
        PortKey::Boundary(port) => format!("boundary port `{port}`"),
        PortKey::Node(slot, port) => format!("port `{}.{port}`", plan.nodes[slot.index()].path),
        PortKey::Graph(owner, port) => {
            format!("Graph port `{}.{port}`", plan.owners[owner.index()].path)
        }
    }
}

fn var_location(plan: &ExecutionPlan, key: &VarKey) -> String {
    format!("var `{}.{}`", plan.nodes[key.node.index()].path, key.name)
}

fn value_location(plan: &ExecutionPlan, key: &ValueKey) -> String {
    let owner = &plan.owners[key.owner.index()].path;
    if owner.is_root() {
        format!("App val `{}`", key.name)
    } else {
        format!("val `{owner}.{}`", key.name)
    }
}

fn has_policy_declaration(plan: &ExecutionPlan, resolved: &ResolvedPortTable) -> bool {
    resolved
        .boundary()
        .iter()
        .any(|port| !port.annotations().is_empty())
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            let slot = Slot::new(u32::try_from(index).unwrap_or(u32::MAX));
            resolved
                .node(slot)
                .is_some_and(|ports| ports.iter().any(|port| !port.annotations().is_empty()))
                || node.vars.iter().any(|cell| !cell.annotations.is_empty())
        })
        || plan.owners.iter().any(|owner| {
            owner.ports.iter().any(|port| !port.annotations.is_empty())
                || owner.vals.iter().any(|value| !value.annotations.is_empty())
        })
        || plan.handlers.iter().any(|handler| {
            handler.body.iter().any(
                |step| matches!(step, Step::Let { annotations, .. } if !annotations.is_empty()),
            )
        })
}

pub(super) fn resolve(
    plan: &ExecutionPlan,
    resolved: &ResolvedPortTable,
) -> Result<ResolvedPortTable, ResolutionError> {
    // With no source policy there is nothing an operation may derive: exact
    // transforms preserve the empty table and calculations cannot invent one.
    // This is the zero-walk path for the overwhelmingly common M1 plan.
    if !has_policy_declaration(plan, resolved) {
        return Ok(resolved.clone());
    }
    let outcome = Solver::new(plan, resolved).solve()?;
    let mut effective = resolved.clone();
    for (key, annotations) in outcome.ports {
        let updated = match &key {
            PortKey::Boundary(port) => effective.set_annotations(None, port, annotations),
            PortKey::Node(slot, port) => effective.set_annotations(Some(*slot), port, annotations),
            // Nested Graph boundaries have no separate inferred-port table.
            // Their declarations remain the policy authority and this pass
            // still derives every handler flow through them.
            PortKey::Graph(_, _) => true,
        };
        if !updated {
            return Err(ResolutionError {
                location: port_location(plan, &key),
                reason: PropagationError::UnknownBinding(port_location(plan, &key)),
            });
        }
    }
    Ok(effective)
}
