//! Registry-free derivation of policy tables across canonical plan operations.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::data_policy::{
    self, Environment, PropagationError, alternatives, nest, overlay, replace_data_path,
    security_taint,
};
use crate::datatype::DataType;
use crate::envelope::WriteTarget;
use crate::node;
use crate::schema_annotation::{
    AnnotationTable, DataAnnotation, SchemaAnnotationError, SchemaPathSegment,
};

use super::type_resolution::ResolvedPortTable;
use super::{
    DispatchStep, EmitValue, Endpoint, EndpointOwner, ExecutionPlan, Handler, LoopCommand,
    LoopCommandKind, NodeIndex, RouteDestination, RouteSelection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolutionError {
    pub(super) location: String,
    pub(super) reason: PropagationError,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PortKey {
    Boundary(node::Port),
    Node(NodeIndex, node::Port),
    Graph(node::NodeIndex, node::Port),
}

#[derive(Debug, Clone)]
struct PortState {
    key: PortKey,
    direction: node::PortDirection,
    ty: DataType,
    declared: AnnotationTable,
    current: AnnotationTable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VarKey {
    node: NodeIndex,
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
    owner: node::NodeIndex,
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
            let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
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
        for (index, owner) in plan.nodes.iter().enumerate() {
            if owner.path.is_root() || owner.kind() != node::Kind::Graph {
                continue;
            }
            let slot = node::NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
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
            let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
            for var in node.vars() {
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
        for (index, owner) in plan.nodes.iter().enumerate() {
            let owner_slot = node::NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
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
        // A recurrence may initially read an as-yet empty inferred entry.
        // Establish the fixed point before requiring exact accounting meaning
        // on every producer. The strict pass is mandatory before publication.
        let mut strict = !self
            .plan
            .nodes
            .iter()
            .any(|owner| owner.loop_spec().is_some());
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
                        producer_policy(producers, strict).map_err(|reason| ResolutionError {
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
                        producer_policy(producers, strict).map_err(|reason| ResolutionError {
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
                        producer_policy(producers, strict).map_err(|reason| ResolutionError {
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
                if !strict {
                    strict = true;
                    continue;
                }
                let mut ports = BTreeMap::new();
                for (key, state) in self.ports {
                    ports.insert(key, state.current);
                }
                return Ok(Outcome { ports });
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

        // Follow each authored crossing so intermediate Graph policies are
        // preserved before composition reaches the final Executor.
        for rule in self.plan.reference_routes() {
            let source = self.scoped_endpoint_key(&rule.source)?;
            let arms: Vec<_> = match &rule.selection {
                RouteSelection::All(destinations) => vec![(None, destinations.as_slice())],
                RouteSelection::Cases(arms) => arms
                    .iter()
                    .map(|arm| (Some(arm.cases.as_slice()), arm.destinations.as_slice()))
                    .collect(),
            };
            for (cases, destinations) in arms {
                let table = self.port(&source)?.current.clone();
                let table = match cases {
                    None => table,
                    Some(cases) => AnnotationTable::new(table.entries().iter().filter(|(path, _)| {
                        !matches!(path.segments().first(), Some(SchemaPathSegment::TaggedUnionCase(case)) if !cases.contains(case))
                    }).map(|(path, set)| (path.clone(), set.clone())).collect()).map_err(|reason| ResolutionError {
                        location: port_location(self.plan, &source),
                        reason: PropagationError::InvalidDerivedTable(reason),
                    })?,
                };
                for destination in destinations {
                    match destination {
                        RouteDestination::Port(target) => {
                            ports
                                .entry(self.scoped_endpoint_key(target)?)
                                .or_default()
                                .push(table.clone());
                        }
                        RouteDestination::LoopCommand(command)
                            if command.kind != LoopCommandKind::Done =>
                        {
                            let owner = self.endpoint_owner(&command.owner)?;
                            let input = self.loop_input_key(owner)?;
                            let next = if command.kind == LoopCommandKind::Next {
                                table.clone()
                            } else {
                                let taint = security_taint([table.clone()]).map_err(|reason| {
                                    ResolutionError {
                                        location: port_location(self.plan, &source),
                                        reason,
                                    }
                                })?;
                                overlay(&self.port(&input)?.current, &taint).map_err(|reason| {
                                    ResolutionError {
                                        location: port_location(self.plan, &input),
                                        reason,
                                    }
                                })?
                            };
                            ports.entry(input).or_default().push(next);
                        }
                        _ => {}
                    }
                }
            }
        }

        for (index, owner) in self.plan.nodes.iter().enumerate() {
            let Some(expiry) = owner.loop_spec().and_then(|spec| spec.expired.as_ref()) else {
                continue;
            };
            let owner_slot = node::NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
            let input = self.loop_input_key(owner_slot)?;
            let expired = self.owner_port_key(owner_slot, expiry.port.name())?;
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

        for (handler_index, (handler_owner, handler)) in self.plan.handlers().enumerate() {
            let (environment, input) = self.handler_environment(handler_owner, handler)?;
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
            let mut pending = alloc::vec![(&handler.dispatch, environment, guard_taint)];
            while let Some((dispatch, mut environment, guard_taint)) = pending.pop() {
                let steps = match dispatch {
                    super::Dispatch::Linear(steps)
                    | super::Dispatch::ClosedMatch { prefix: steps, .. }
                    | super::Dispatch::OpenMatch { prefix: steps, .. } => steps,
                };
                for step in steps {
                    match step {
                        DispatchStep::Let {
                            name,
                            value: expr,
                            annotations,
                        } => {
                            let derived =
                                data_policy::expression(expr, &environment).map_err(|reason| {
                                    ResolutionError {
                                        location: format!("handler {handler_index} let `{name}`"),
                                        reason,
                                    }
                                })?;
                            let derived = overlay(&derived, &guard_taint).map_err(|reason| {
                                ResolutionError {
                                    location: format!("handler {handler_index} let `{name}`"),
                                    reason,
                                }
                            })?;
                            let expected = overlay(&derived, annotations).map_err(|reason| {
                                ResolutionError {
                                    location: format!("handler {handler_index} let `{name}`"),
                                    reason,
                                }
                            })?;
                            environment.insert(name.clone(), expr.ty.clone(), expected.clone());
                        }
                        DispatchStep::SetVar { name, value: expr } => {
                            let key = VarKey {
                                node: handler_owner,
                                name: name.clone(),
                            };
                            let derived =
                                data_policy::expression(expr, &environment).map_err(|reason| {
                                    ResolutionError {
                                        location: var_location(self.plan, &key),
                                        reason,
                                    }
                                })?;
                            let derived = overlay(&derived, &guard_taint).map_err(|reason| {
                                ResolutionError {
                                    location: var_location(self.plan, &key),
                                    reason,
                                }
                            })?;
                            vars.entry(key).or_default().push(derived);
                        }
                        DispatchStep::Emit { .. } | DispatchStep::MultiEmit { .. } => {
                            for (port, value) in step.emissions() {
                                let key = self.handler_output(handler_owner, port)?;
                                if let Some(mut derived) = self.emission(
                                    value,
                                    input.as_ref(),
                                    &environment,
                                    self.port(&key)?.ty.clone(),
                                    handler_index,
                                )? {
                                    derived =
                                        overlay(&derived, &guard_taint).map_err(|reason| {
                                            ResolutionError {
                                                location: port_location(self.plan, &key),
                                                reason,
                                            }
                                        })?;
                                    ports.entry(key).or_default().push(derived);
                                }
                            }
                        }
                        DispatchStep::LoopCommand(command) => {
                            let owner =
                                self.loop_owner(handler_owner, handler).ok_or_else(|| {
                                    ResolutionError {
                                        location: format!("handler {handler_index} command"),
                                        reason: PropagationError::UnknownBinding(
                                            "Loop.state".into(),
                                        ),
                                    }
                                })?;
                            let entry = self.loop_input_key(owner)?;
                            let table = match command {
                                LoopCommand::Next(value) => self.emission(
                                    value,
                                    input.as_ref(),
                                    &environment,
                                    self.port(&entry)?.ty.clone(),
                                    handler_index,
                                )?,
                                LoopCommand::Again => Some(self.port(&entry)?.current.clone()),
                                LoopCommand::Done => None,
                            };
                            if let Some(table) = table {
                                let derived = overlay(&table, &guard_taint).map_err(|reason| {
                                    ResolutionError {
                                        location: port_location(self.plan, &entry),
                                        reason,
                                    }
                                })?;
                                ports.entry(entry).or_default().push(derived);
                            }
                        }
                        DispatchStep::Stop => {}
                    }
                }
                let error = |reason| ResolutionError {
                    location: format!("handler {handler_index} dispatch"),
                    reason,
                };
                let taint = |predicate: &crate::expr::Tree, previous: &AnnotationTable| {
                    let decision =
                        security_taint([
                            data_policy::expression(predicate, &environment).map_err(&error)?
                        ])
                        .map_err(&error)?;
                    overlay(previous, &decision).map_err(&error)
                };
                match dispatch {
                    super::Dispatch::Linear(_) => {}
                    super::Dispatch::ClosedMatch {
                        scrutinee, arms, ..
                    } => {
                        let decision = taint(scrutinee, &guard_taint)?;
                        for arm in arms.iter().rev() {
                            pending.push((
                                &arm.dispatch,
                                environment
                                    .selected_cases(scrutinee, &arm.cases)
                                    .map_err(&error)?,
                                decision.clone(),
                            ));
                        }
                    }
                    super::Dispatch::OpenMatch { arms, fallback, .. } => {
                        let mut decision = guard_taint;
                        for arm in arms {
                            decision = taint(&arm.predicate, &decision)?;
                            pending.push((
                                &arm.dispatch,
                                environment
                                    .narrowed_by(&arm.predicate.root)
                                    .map_err(&error)?,
                                decision.clone(),
                            ));
                        }
                        pending.push((fallback, environment, decision));
                    }
                }
            }
        }
        Ok((ports, vars, values))
    }

    fn derive_owner_values(
        &self,
        producers: &mut BTreeMap<ValueKey, Vec<AnnotationTable>>,
    ) -> Result<(), ResolutionError> {
        for index in 0..self.plan.nodes.len() {
            let owner_slot = node::NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
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
        owner: node::NodeIndex,
        input: Option<&PortState>,
        producers: &mut BTreeMap<ValueKey, Vec<AnnotationTable>>,
    ) -> Result<(), ResolutionError> {
        let mut environment = self.owner_environment(owner, input)?;
        if let Some((ty, annotations)) = self.instance_key_policy(owner)? {
            environment.insert("instance.key", ty, annotations);
        }
        for value in &self.plan.nodes[owner.index()].vals {
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

    fn owner_inputs(&self, owner: node::NodeIndex) -> Vec<&PortState> {
        match self.plan.nodes[owner.index()].kind() {
            node::Kind::Operator | node::Kind::Connector => self
                .ports
                .values()
                .filter(|state| {
                    state.direction == node::PortDirection::Input
                        && matches!(state.key, PortKey::Node(candidate, _) if candidate == owner)
                })
                .collect(),
            node::Kind::Graph => self
                .graph_port_states(owner)
                .into_iter()
                .filter(|state| state.direction == node::PortDirection::Input)
                .collect(),
        }
    }

    fn owner_environment(
        &self,
        owner: node::NodeIndex,
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
            cursor = self.plan.nodes[slot.index()].parent;
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
        owner: node::NodeIndex,
    ) -> Result<Option<(DataType, AnnotationTable)>, ResolutionError> {
        let Some(key) = &self.plan.nodes[owner.index()].instance.key else {
            return Ok(None);
        };
        let inputs = self.owner_inputs(owner);
        let mut candidates = Vec::new();
        if inputs.is_empty() {
            let environment = self.owner_environment(owner, None)?;
            candidates.push(
                data_policy::expression(key, &environment).map_err(|reason| ResolutionError {
                    location: format!("instance key on `{}`", self.plan.nodes[owner.index()].path),
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
                                self.plan.nodes[owner.index()].path
                            ),
                            reason,
                        }
                    })?,
                );
            }
        }
        let annotations = alternatives(candidates).map_err(|reason| ResolutionError {
            location: format!("instance key on `{}`", self.plan.nodes[owner.index()].path),
            reason,
        })?;
        Ok(Some((key.ty.clone(), annotations)))
    }

    fn handler_environment(
        &self,
        handler_owner: NodeIndex,
        handler: &Handler,
    ) -> Result<(Environment, Option<AnnotationTable>), ResolutionError> {
        let mut environment = Environment::new()
            .with("meta", DataType::Variant, AnnotationTable::empty())
            .with("now", DataType::TIMESTAMP, AnnotationTable::empty());
        let mut ordinary_input = None;
        match (handler_owner, &handler.activation) {
            (slot, node::HandlerActivation::Port(port)) => {
                let state = self.port(&PortKey::Node(slot, port.clone()))?;
                environment.insert("data", state.ty.clone(), state.current.clone());
                ordinary_input = Some(state.current.clone());
            }
            (slot, node::HandlerActivation::Join { ports, .. }) => {
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
            (slot, node::HandlerActivation::Lifecycle(_)) => {
                let inputs: Vec<_> = self
                    .ports
                    .values()
                    .filter(|state| {
                        matches!(state.key, PortKey::Node(candidate, _)
                            if candidate == slot)
                            && state.direction == node::PortDirection::Input
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
        }

        let owner = handler_owner;
        let lexical = self.owner_environment(owner, None)?;
        for (name, binding) in lexical.bindings() {
            environment.insert(
                name.clone(),
                binding.ty().clone(),
                binding.annotations().clone(),
            );
        }
        if let Some(owner) = self.loop_owner(handler_owner, handler) {
            let entry = self.loop_input_key(owner)?;
            let state = self.port(&entry)?;
            environment.insert("Loop.state.data", state.ty.clone(), state.current.clone());
            environment.insert(
                "Loop.state.meta",
                DataType::Variant,
                AnnotationTable::empty(),
            );
            let envelope = nest(&state.current, &[SchemaPathSegment::Field("data".into())])
                .map_err(|reason| ResolutionError {
                    location: port_location(self.plan, &entry),
                    reason,
                })?;
            environment.insert("Loop.state", DataType::Variant, envelope);
        }
        if let Some((ty, annotations)) = self.instance_key_policy(owner)? {
            environment.insert("instance.key", ty, annotations);
        }

        for state in self
            .vars
            .values()
            .filter(|state| state.key.node == handler_owner)
        {
            environment.insert(
                state.key.name.clone(),
                state.ty.clone(),
                state.current.clone(),
            );
        }
        Ok((environment, ordinary_input))
    }

    fn emission(
        &self,
        emission: &EmitValue,
        input: Option<&AnnotationTable>,
        environment: &Environment,
        output_type: DataType,
        handler_index: usize,
    ) -> Result<Option<AnnotationTable>, ResolutionError> {
        let location = format!("handler {handler_index} emission");
        let result = match emission {
            EmitValue::Nothing => return Ok(None),
            EmitValue::Identity => input.cloned().unwrap_or_default(),
            EmitValue::Data(tree) => data_policy::expression(tree, environment)
                .map_err(|reason| ResolutionError { location, reason })?,
            EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
                let mut result = if matches!(emission, EmitValue::Patch(_)) {
                    input.cloned().unwrap_or_default()
                } else {
                    AnnotationTable::empty()
                };
                for write in writes {
                    let derived =
                        data_policy::expression(&write.value, environment).map_err(|reason| {
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
        owner: NodeIndex,
        port: &node::Port,
    ) -> Result<PortKey, ResolutionError> {
        let key = PortKey::Node(owner, port.clone());
        self.port(&key)?;
        Ok(key)
    }

    fn loop_owner(&self, handler_owner: NodeIndex, handler: &Handler) -> Option<node::NodeIndex> {
        if matches!(handler.activation, node::HandlerActivation::Lifecycle(_)) {
            return None;
        }
        let mut current = Some(handler_owner);
        while let Some(owner) = current {
            if self.plan.nodes[owner.index()].loop_spec().is_some() {
                return Some(owner);
            }
            current = self.plan.nodes[owner.index()].parent;
        }
        None
    }

    fn loop_input_key(&self, owner: node::NodeIndex) -> Result<PortKey, ResolutionError> {
        let spec = self.plan.nodes[owner.index()]
            .loop_spec()
            .ok_or_else(|| ResolutionError {
                location: self.plan.nodes[owner.index()].path.to_string(),
                reason: PropagationError::UnknownBinding("Loop".into()),
            })?;
        self.owner_port_key(owner, spec.entry.name())
    }

    fn endpoint_owner(&self, endpoint: &EndpointOwner) -> Result<node::NodeIndex, ResolutionError> {
        let path = match endpoint {
            EndpointOwner::Node(path) => crate::node::OwnerPath::from(path),
            EndpointOwner::Graph(path) => path.clone(),
        };
        self.plan
            .nodes
            .iter()
            .position(|owner| owner.path == path)
            .map(|index| node::NodeIndex::new(index as u32))
            .ok_or_else(|| ResolutionError {
                location: path.to_string(),
                reason: PropagationError::UnknownBinding(path.to_string()),
            })
    }

    fn scoped_endpoint_key(&self, endpoint: &Endpoint) -> Result<PortKey, ResolutionError> {
        self.owner_port_key(self.endpoint_owner(&endpoint.owner)?, endpoint.port.name())
    }

    fn owner_port_key(
        &self,
        owner: node::NodeIndex,
        name: &str,
    ) -> Result<PortKey, ResolutionError> {
        let port = node::Port::new(name).map_err(|_| ResolutionError {
            location: format!("Graph `{}`", self.plan.nodes[owner.index()].path),
            reason: PropagationError::UnknownBinding(name.into()),
        })?;
        let key = if self.plan.nodes[owner.index()].kind() != node::Kind::Graph {
            PortKey::Node(owner, port)
        } else if self.plan.nodes[owner.index()].path.is_root() {
            PortKey::Boundary(port)
        } else {
            PortKey::Graph(owner, port)
        };
        self.port(&key)?;
        Ok(key)
    }

    fn graph_port_states(&self, owner: node::NodeIndex) -> Vec<&PortState> {
        if self.plan.nodes[owner.index()].path.is_root() {
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

fn producer_policy(
    tables: &[AnnotationTable],
    strict: bool,
) -> Result<AnnotationTable, PropagationError> {
    match alternatives(tables.iter().cloned()) {
        Err(PropagationError::ConflictingMeaning { .. }) if !strict => tables
            .iter()
            .try_fold(AnnotationTable::empty(), |known, table| {
                overlay(&known, table)
            }),
        result => result,
    }
}

fn port_location(plan: &ExecutionPlan, key: &PortKey) -> String {
    match key {
        PortKey::Boundary(port) => format!("boundary port `{port}`"),
        PortKey::Node(slot, port) => format!("port `{}.{port}`", plan.nodes[slot.index()].path),
        PortKey::Graph(owner, port) => {
            format!("Graph port `{}.{port}`", plan.nodes[owner.index()].path)
        }
    }
}

fn var_location(plan: &ExecutionPlan, key: &VarKey) -> String {
    format!("var `{}.{}`", plan.nodes[key.node.index()].path, key.name)
}

fn value_location(plan: &ExecutionPlan, key: &ValueKey) -> String {
    let owner = &plan.nodes[key.owner.index()].path;
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
            let slot = NodeIndex::new(u32::try_from(index).unwrap_or(u32::MAX));
            resolved
                .node(slot)
                .is_some_and(|ports| ports.iter().any(|port| !port.annotations().is_empty()))
                || node.vars().iter().any(|cell| !cell.annotations.is_empty())
        })
        || plan.nodes.iter().any(|owner| {
            owner.ports.iter().any(|port| !port.annotations.is_empty())
                || owner.vals.iter().any(|value| !value.annotations.is_empty())
        })
        || plan.handlers().any(|(_, handler)| {
            handler.dispatch.steps().any(
            |step| matches!(step, DispatchStep::Let { annotations, .. } if !annotations.is_empty()),
        )
        })
}

fn validate_definitions(plan: &ExecutionPlan) -> Result<(), ResolutionError> {
    let mut definitions = BTreeMap::new();
    let mut measures = BTreeMap::new();
    for port in plan.boundary() {
        validate_table_definitions(&port.annotations, &mut definitions, &mut measures)?;
    }
    for node in &plan.nodes {
        for port in &node.ports {
            validate_table_definitions(&port.annotations, &mut definitions, &mut measures)?;
        }
    }
    for node in &plan.nodes {
        for var in node.vars() {
            validate_table_definitions(&var.annotations, &mut definitions, &mut measures)?;
        }
    }
    for owner in &plan.nodes {
        if !owner.path.is_root() {
            for port in &owner.ports {
                validate_table_definitions(&port.annotations, &mut definitions, &mut measures)?;
            }
        }
    }
    for owner in &plan.nodes {
        for value in &owner.vals {
            validate_table_definitions(&value.annotations, &mut definitions, &mut measures)?;
        }
    }
    for (_, handler) in plan.handlers() {
        for step in handler.dispatch.steps() {
            if let DispatchStep::Let { annotations, .. } = step {
                validate_table_definitions(annotations, &mut definitions, &mut measures)?;
            }
        }
    }
    Ok(())
}

fn validate_table_definitions<'a>(
    table: &'a AnnotationTable,
    definitions: &mut BTreeMap<&'a str, &'a DataAnnotation>,
    measures: &mut BTreeMap<&'a str, &'a DataAnnotation>,
) -> Result<(), ResolutionError> {
    for (path, set) in table.entries() {
        for annotation in set.annotations() {
            let measurement = match annotation {
                DataAnnotation::Declared(application) => application.measure_id(),
                _ => None,
            };
            if let Some(id) = measurement
                && measures
                    .insert(id, annotation)
                    .is_some_and(|previous| previous != annotation)
            {
                return Err(ResolutionError {
                    location: format!("measure `{id}` at {path}"),
                    reason: PropagationError::InvalidDerivedTable(
                        SchemaAnnotationError::InvalidApplication {
                            definition: id.to_string(),
                            detail: "one measure id has inconsistent units or interpretations",
                        },
                    ),
                });
            }
            if let DataAnnotation::Declared(application) = annotation
                && definitions
                    .insert(application.definition(), annotation)
                    .is_some_and(|previous| match previous {
                        DataAnnotation::Declared(previous) => {
                            previous.binding() != application.binding()
                                || previous.argument_type() != application.argument_type()
                        }
                        _ => true,
                    })
            {
                return Err(ResolutionError {
                    location: format!("annotation `{}` at {path}", application.definition()),
                    reason: PropagationError::InvalidDerivedTable(
                        SchemaAnnotationError::InvalidApplication {
                            definition: application.definition().to_string(),
                            detail: "one declaring name has inconsistent argument schemas or bindings",
                        },
                    ),
                });
            }
        }
    }
    Ok(())
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
    validate_definitions(plan)?;
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
