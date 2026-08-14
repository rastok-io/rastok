//! Effective port types derived from a raw plan.
//!
//! `ExecutionPlan` v6 still transports source pins as
//! `Option<DataType>`. This module is the semantic firewall in front of
//! execution: inference variables live only while this function runs, and a
//! [`ResolvedPortTable`] contains neither an option nor a recovery case.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::datatype::{DataType, StructType, TypeField};
use crate::envelope::WriteTarget;
use crate::expr;
use crate::node;
use crate::plan::{
    Emission, EndpointRef, ExecutionPlan, NodeKind, PlanValidationError, Slot, Step,
};

/// Why one effective port type holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeOrigin {
    /// Source or an instantiated connector contract fixed the type.
    Declared,
    /// Graph constraints determined the type.
    Inferred,
}

/// One port after type resolution. An effective type is mandatory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPort {
    port: node::Port,
    direction: node::Direction,
    ty: DataType,
    origin: TypeOrigin,
}

impl ResolvedPort {
    /// Port name.
    #[must_use]
    pub const fn port(&self) -> &node::Port {
        &self.port
    }

    /// Node-side direction.
    #[must_use]
    pub const fn direction(&self) -> node::Direction {
        self.direction
    }

    /// Effective logical payload type.
    #[must_use]
    pub const fn ty(&self) -> &DataType {
        &self.ty
    }

    /// Whether the effective constraint was declared or inferred.
    #[must_use]
    pub const fn origin(&self) -> TypeOrigin {
        self.origin
    }
}

/// Complete effective port tables, parallel to the plan's boundary and node
/// tables. Node rows are indexed by [`Slot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPortTable {
    boundary: Vec<ResolvedPort>,
    nodes: Vec<Vec<ResolvedPort>>,
}

impl ResolvedPortTable {
    /// Effective boundary ports that the plan actually declares or uses.
    #[must_use]
    pub fn boundary(&self) -> &[ResolvedPort] {
        &self.boundary
    }

    /// Effective ports of one plan-derived node slot.
    #[must_use]
    pub fn node(&self, slot: Slot) -> Option<&[ResolvedPort]> {
        self.nodes.get(slot.index()).map(Vec::as_slice)
    }

    /// Effective type at an endpoint already proven to belong to the plan.
    #[must_use]
    pub fn endpoint_type(&self, endpoint: &EndpointRef) -> Option<&DataType> {
        let row = match endpoint.node {
            Some(slot) => self.node(slot)?,
            None => self.boundary(),
        };
        row.iter()
            .find(|resolved| resolved.port == endpoint.port)
            .map(ResolvedPort::ty)
    }

    /// Effective node-port type by slot and name.
    #[must_use]
    pub fn node_port_type(&self, slot: Slot, port: &node::Port) -> Option<&DataType> {
        self.node(slot)?
            .iter()
            .find(|resolved| &resolved.port == port)
            .map(ResolvedPort::ty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PortKey {
    node: Option<usize>,
    port: node::Port,
}

struct PortState {
    key: PortKey,
    direction: node::Direction,
    declared: Option<DataType>,
    produced: Option<DataType>,
    required: Option<TypeRequirement>,
}

#[derive(Clone, Copy)]
struct Flow {
    source: usize,
    target: usize,
}

struct EmissionRule<'plan> {
    input: usize,
    output: usize,
    emission: &'plan Emission,
}

struct ExpressionRule<'plan> {
    input: usize,
    expression: &'plan expr::Tree,
    expected: DataType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TypeRequirement {
    Exact(DataType),
    Struct(BTreeMap<String, TypeRequirement>),
}

struct Solver<'plan> {
    plan: &'plan ExecutionPlan,
    index: BTreeMap<PortKey, usize>,
    states: Vec<PortState>,
    flows: Vec<Flow>,
    emissions: Vec<EmissionRule<'plan>>,
    expressions: Vec<ExpressionRule<'plan>>,
}

impl<'plan> Solver<'plan> {
    fn new(plan: &'plan ExecutionPlan) -> Self {
        Self {
            plan,
            index: BTreeMap::new(),
            states: Vec::new(),
            flows: Vec::new(),
            emissions: Vec::new(),
            expressions: Vec::new(),
        }
    }

    fn ensure(
        &mut self,
        node: Option<usize>,
        port: &node::Port,
        direction: node::Direction,
    ) -> usize {
        let key = PortKey {
            node,
            port: port.clone(),
        };
        if let Some(index) = self.index.get(&key) {
            return *index;
        }
        let index = self.states.len();
        self.states.push(PortState {
            key: key.clone(),
            direction,
            declared: None,
            produced: None,
            required: None,
        });
        self.index.insert(key, index);
        index
    }

    fn endpoint(&mut self, endpoint: &EndpointRef, direction: node::Direction) -> usize {
        self.ensure(endpoint.node.map(Slot::index), &endpoint.port, direction)
    }

    fn declare(&mut self, index: usize, ty: DataType) -> Result<(), PlanValidationError> {
        match self.states[index].declared.as_ref() {
            Some(existing) if existing != &ty => Err(self.conflict(index, existing.clone(), ty)),
            _ => {
                self.states[index].declared = Some(ty);
                Ok(())
            }
        }
    }

    fn collect(mut self) -> Result<Self, PlanValidationError> {
        for decl in &self.plan.boundary {
            let index = self.ensure(None, &decl.port, decl.direction);
            if let Some(ty) = &decl.ty {
                self.declare(index, ty.clone())?;
            }
        }
        for (node_index, planned) in self.plan.nodes.iter().enumerate() {
            for decl in &planned.ports {
                let index = self.ensure(Some(node_index), &decl.port, decl.direction);
                if let Some(ty) = &decl.ty {
                    self.declare(index, ty.clone())?;
                }
            }
            if let NodeKind::Connector(spec) = &planned.kind {
                for (name, direction) in [
                    ("in", node::Direction::Input),
                    ("out", node::Direction::Output),
                    ("err", node::Direction::Output),
                ] {
                    let Ok(port) = node::Port::new(name) else {
                        continue;
                    };
                    let Some(ty) = crate::boundary::resolved_port_type(spec, &port) else {
                        continue;
                    };
                    let index = self.ensure(Some(node_index), &port, direction);
                    self.declare(index, ty)?;
                }
            }
        }
        for edge in &self.plan.edges {
            let source = self.endpoint(&edge.from, node::Direction::Output);
            if let Some(target) = &edge.to {
                let target = self.endpoint(target, node::Direction::Input);
                self.flows.push(Flow { source, target });
            }
        }
        for handler in &self.plan.handlers {
            let node_index = handler.node.index();
            let node::HandlerActivation::Port(input_port) = &handler.activation;
            let input = self.ensure(Some(node_index), input_port, node::Direction::Input);
            if let Some(guard) = &handler.guard {
                self.expressions.push(ExpressionRule {
                    input,
                    expression: guard,
                    expected: DataType::Bool,
                });
            }
            for step in &handler.body {
                match step {
                    Step::Emit { port, value } => {
                        let output = self.ensure(Some(node_index), port, node::Direction::Output);
                        if emission_preserves_payload(value) {
                            self.flows.push(Flow {
                                source: input,
                                target: output,
                            });
                        } else if !matches!(value, Emission::Nothing) {
                            self.emissions.push(EmissionRule {
                                input,
                                output,
                                emission: value,
                            });
                        }
                    }
                    Step::SetVar { name, expr } => {
                        if let Some(expected) = self.plan.nodes[node_index]
                            .vars
                            .iter()
                            .find(|cell| cell.name == *name)
                            .map(|cell| cell.ty.clone())
                        {
                            self.expressions.push(ExpressionRule {
                                input,
                                expression: expr,
                                expected,
                            });
                        }
                    }
                    Step::Let { .. } => {}
                }
            }
        }
        Ok(self)
    }

    fn solve(mut self) -> Result<ResolvedPortTable, PlanValidationError> {
        // Bounds can become informative in either direction. Each useful
        // pass types, widens, or narrows at least one finite type tree; the
        // product bound is deliberately generous while still making a
        // malformed foreign plan terminate deterministically.
        let limit = self
            .states
            .len()
            .saturating_mul(
                self.flows
                    .len()
                    .saturating_add(self.emissions.len())
                    .saturating_add(self.expressions.len())
                    .saturating_add(1),
            )
            .saturating_add(1);
        for _ in 0..limit {
            let mut changed = false;
            // Contract/source bounds travel first. In particular, an
            // expression reading through an explicitly dynamic container
            // must see that `variant` fact instead of fabricating a precise
            // structural requirement from the path spelling.
            for index in 0..self.flows.len() {
                let flow = self.flows[index];
                if let Some(source) = self.producer_hint(flow.source) {
                    changed |= self.produce(flow.target, source)?;
                }
                if let Some(required) = self.consumer_requirement(flow.target)? {
                    changed |= self.require(flow.source, required)?;
                }
            }
            for index in 0..self.expressions.len() {
                let (input, expression, expected) = {
                    let rule = &self.expressions[index];
                    (rule.input, rule.expression, rule.expected.clone())
                };
                changed |= self.constrain_expression(input, expression, &expected)?;
            }
            for index in 0..self.emissions.len() {
                let (input, output, emission) = {
                    let rule = &self.emissions[index];
                    (rule.input, rule.output, rule.emission)
                };
                let input_type = self.effective_hint(input)?;
                let output_type = self.effective_hint(output)?;
                if let Some(ty) = emission_type(
                    emission,
                    input_type.as_ref(),
                    output_type.as_ref(),
                    |required| self.require(input, required),
                )? {
                    changed |= self.produce(output, ty)?;
                }
            }
            if !changed {
                // An open structural requirement is not itself a runtime
                // type. Once all declared and produced bounds have
                // propagated, an otherwise-unanchored source can infer the
                // smallest closed type that satisfies what its consumers
                // actually read. Seed that exact type only at the fixed
                // point, then let ordinary flow propagation carry it.
                let mut has_upstream = alloc::vec![false; self.states.len()];
                for flow in &self.flows {
                    has_upstream[flow.target] = true;
                }
                for (index, state) in self.states.iter_mut().enumerate() {
                    if state.declared.is_none()
                        && state.produced.is_none()
                        && !has_upstream[index]
                        && let Some(required) = state.required.as_ref()
                    {
                        state.produced = Some(close_requirement(required)?);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
        }

        let mut effective = Vec::with_capacity(self.states.len());
        for index in 0..self.states.len() {
            let Some(ty) = self.effective_type(index)? else {
                return Err(self.unresolved(&self.states[index]));
            };
            effective.push(ty);
        }
        for rule in &self.expressions {
            if expression_reads_data(&rule.expression.root) {
                if resolved_expression_type(&rule.expression.root, &effective[rule.input])
                    .is_some_and(|actual| actual.is_assignable_to(&rule.expected))
                {
                    continue;
                }
                let Some(requirement) =
                    expression_requirement(&rule.expression.root, &rule.expected)
                else {
                    return Err(self.unresolved(&self.states[rule.input]));
                };
                if !satisfies(&effective[rule.input], &requirement) {
                    return Err(self.conflict(
                        rule.input,
                        close_requirement(&requirement)?,
                        effective[rule.input].clone(),
                    ));
                }
            } else if !rule.expression.ty.is_assignable_to(&rule.expected) {
                return Err(self.conflict(
                    rule.input,
                    rule.expected.clone(),
                    rule.expression.ty.clone(),
                ));
            }
        }
        for rule in &self.emissions {
            let input = &effective[rule.input];
            let output = &effective[rule.output];
            let inferred =
                emission_type(rule.emission, Some(input), Some(output), |requirement| {
                    if satisfies(input, &requirement) {
                        Ok(false)
                    } else {
                        Err(PlanValidationError::ConflictingPortTypes {
                            node: self.states[rule.input]
                                .key
                                .node
                                .map(|slot| self.plan.nodes[slot].path.clone()),
                            port: self.states[rule.input].key.port.clone(),
                            expected: close_requirement(&requirement)?,
                            found: input.clone(),
                        })
                    }
                })?;
            let Some(inferred) = inferred else {
                return Err(self.unresolved(&self.states[rule.output]));
            };
            if !inferred.is_assignable_to(output) {
                return Err(self.conflict(rule.output, output.clone(), inferred));
            }
        }
        // One final directional audit after every bound has settled.
        for flow in &self.flows {
            let source = &effective[flow.source];
            let target = &effective[flow.target];
            if !source.is_assignable_to(target) {
                return Err(self.conflict(flow.target, target.clone(), source.clone()));
            }
        }

        let mut boundary = Vec::new();
        let mut nodes = alloc::vec![Vec::new(); self.plan.nodes.len()];
        for (state, ty) in self.states.into_iter().zip(effective) {
            let resolved = ResolvedPort {
                port: state.key.port,
                direction: state.direction,
                ty,
                origin: if state.declared.is_some() {
                    TypeOrigin::Declared
                } else {
                    TypeOrigin::Inferred
                },
            };
            match state.key.node {
                Some(node) => nodes[node].push(resolved),
                None => boundary.push(resolved),
            }
        }
        boundary.sort_by(|a, b| (a.port(), a.direction()).cmp(&(b.port(), b.direction())));
        for row in &mut nodes {
            row.sort_by(|a, b| (a.port(), a.direction()).cmp(&(b.port(), b.direction())));
        }
        Ok(ResolvedPortTable { boundary, nodes })
    }

    fn produce(&mut self, index: usize, candidate: DataType) -> Result<bool, PlanValidationError> {
        candidate
            .validate()
            .map_err(|reason| PlanValidationError::InvalidDataType {
                location: self.location(index),
                reason,
            })?;
        if let Some(declared) = self.states[index].declared.as_ref()
            && !candidate.is_assignable_to(declared)
        {
            return Err(self.conflict(index, declared.clone(), candidate));
        }
        if let Some(required) = self.states[index].required.as_ref()
            && !satisfies(&candidate, required)
        {
            return Err(self.conflict(index, close_requirement(required)?, candidate));
        }
        let Some(current) = self.states[index].produced.as_ref() else {
            self.states[index].produced = Some(candidate);
            return Ok(true);
        };
        let joined = join_produced(current, &candidate)
            .map_err(|_| self.conflict(index, current.clone(), candidate))?;
        if &joined == current {
            Ok(false)
        } else {
            self.states[index].produced = Some(joined);
            Ok(true)
        }
    }

    fn require(
        &mut self,
        index: usize,
        candidate: TypeRequirement,
    ) -> Result<bool, PlanValidationError> {
        if let Some(declared) = self.states[index].declared.as_ref()
            && !satisfies(declared, &candidate)
        {
            return Err(self.conflict(index, close_requirement(&candidate)?, declared.clone()));
        }
        if let Some(produced) = self.states[index].produced.as_ref()
            && !satisfies(produced, &candidate)
        {
            return Err(self.conflict(index, close_requirement(&candidate)?, produced.clone()));
        }
        let Some(current) = self.states[index].required.as_ref() else {
            self.states[index].required = Some(candidate);
            return Ok(true);
        };
        let Some(merged) = merge_requirements(current, &candidate) else {
            return Err(self.conflict(
                index,
                close_requirement(current)?,
                close_requirement(&candidate)?,
            ));
        };
        if &merged == current {
            Ok(false)
        } else {
            self.states[index].required = Some(merged);
            Ok(true)
        }
    }

    fn producer_hint(&self, index: usize) -> Option<DataType> {
        self.states[index]
            .declared
            .clone()
            .or_else(|| self.states[index].produced.clone())
    }

    fn consumer_requirement(
        &self,
        index: usize,
    ) -> Result<Option<TypeRequirement>, PlanValidationError> {
        if let Some(declared) = self.states[index].declared.as_ref() {
            return Ok(Some(TypeRequirement::Exact(declared.clone())));
        }
        if let Some(required) = self.states[index].required.as_ref() {
            return Ok(Some(required.clone()));
        }
        Ok(self.states[index]
            .produced
            .as_ref()
            .cloned()
            .map(TypeRequirement::Exact))
    }

    fn effective_hint(&self, index: usize) -> Result<Option<DataType>, PlanValidationError> {
        self.effective_type(index)
    }

    fn effective_type(&self, index: usize) -> Result<Option<DataType>, PlanValidationError> {
        let state = &self.states[index];
        if let Some(declared) = state.declared.as_ref() {
            return Ok(Some(declared.clone()));
        }
        if let Some(produced) = state.produced.as_ref() {
            return Ok(Some(produced.clone()));
        }
        state.required.as_ref().map(close_requirement).transpose()
    }

    fn constrain_expression(
        &mut self,
        input: usize,
        expression: &expr::Tree,
        expected: &DataType,
    ) -> Result<bool, PlanValidationError> {
        if !expression_reads_data(&expression.root) {
            if !expression.ty.is_assignable_to(expected) {
                return Err(self.conflict(input, expected.clone(), expression.ty.clone()));
            }
            return Ok(false);
        }
        if let Some(input_type) = self.effective_hint(input)?
            && let Some(actual) = resolved_expression_type(&expression.root, &input_type)
        {
            if actual.is_assignable_to(expected) {
                return Ok(false);
            }
            return Err(self.conflict(input, expected.clone(), actual));
        }
        let Some(required) = expression_requirement(&expression.root, expected) else {
            return Ok(false);
        };
        self.require(input, required)
    }

    fn location(&self, index: usize) -> String {
        let state = &self.states[index];
        match state.key.node {
            Some(slot) => alloc::format!("{}.{}", self.plan.nodes[slot].path, state.key.port),
            None => alloc::format!("boundary.{}", state.key.port),
        }
    }

    fn conflict(&self, index: usize, expected: DataType, found: DataType) -> PlanValidationError {
        let state = &self.states[index];
        PlanValidationError::ConflictingPortTypes {
            node: state
                .key
                .node
                .map(|slot| self.plan.nodes[slot].path.clone()),
            port: state.key.port.clone(),
            expected,
            found,
        }
    }

    fn unresolved(&self, state: &PortState) -> PlanValidationError {
        PlanValidationError::UnresolvedPortType {
            node: state
                .key
                .node
                .map(|slot| self.plan.nodes[slot].path.clone()),
            port: state.key.port.clone(),
        }
    }
}

pub(super) fn resolve(plan: &ExecutionPlan) -> Result<ResolvedPortTable, PlanValidationError> {
    Solver::new(plan).collect()?.solve()
}

fn emission_type(
    emission: &Emission,
    input: Option<&DataType>,
    expected: Option<&DataType>,
    mut require_input: impl FnMut(TypeRequirement) -> Result<bool, PlanValidationError>,
) -> Result<Option<DataType>, PlanValidationError> {
    match emission {
        Emission::Nothing => Ok(None),
        Emission::Identity => Ok(input.cloned()),
        Emission::Data(tree) => expression_type(tree, input, expected, &mut require_input),
        Emission::Patch(fields) => {
            writes_type(fields, input.cloned(), input, expected, &mut require_input)
        }
        Emission::Replace(fields) => {
            let empty = StructType::new(Vec::new())
                .map(DataType::Struct)
                .map_err(|reason| PlanValidationError::InvalidDataType {
                    location: String::from("inferred replacement payload"),
                    reason,
                })?;
            writes_type(fields, Some(empty), input, expected, &mut require_input)
        }
    }
}

fn expression_type(
    tree: &expr::Tree,
    input: Option<&DataType>,
    expected: Option<&DataType>,
    require_input: &mut impl FnMut(TypeRequirement) -> Result<bool, PlanValidationError>,
) -> Result<Option<DataType>, PlanValidationError> {
    if matches!(&tree.root, expr::Expr::Path(path) if path.as_slice() == ["data"]) {
        return Ok(input.cloned());
    }
    if expression_reads_data(&tree.root) {
        if let Some(input) = input
            && let Some(resolved) = resolved_expression_type(&tree.root, input)
        {
            return Ok(Some(resolved));
        }
        if tree.ty != DataType::Variant {
            return Ok(Some(tree.ty.clone()));
        }
        let Some(expected) = expected else {
            return Ok(None);
        };
        let Some(requirement) = expression_requirement(&tree.root, expected) else {
            return Ok(None);
        };
        require_input(requirement)?;
        return Ok(Some(expected.clone()));
    }
    match &tree.root {
        expr::Expr::List(items) if items.is_empty() => Ok(expected
            .filter(|ty| matches!(ty, DataType::Array(_) | DataType::Variant))
            .cloned()),
        expr::Expr::MapLit(entries) if entries.is_empty() => Ok(expected
            .filter(|ty| matches!(ty, DataType::Map(_) | DataType::Variant))
            .cloned()),
        _ => Ok(intrinsic_expression_type(&tree.root).or_else(|| Some(tree.ty.clone()))),
    }
}

fn writes_type(
    fields: &[crate::plan::FieldWrite],
    mut current: Option<DataType>,
    input: Option<&DataType>,
    expected: Option<&DataType>,
    require_input: &mut impl FnMut(TypeRequirement) -> Result<bool, PlanValidationError>,
) -> Result<Option<DataType>, PlanValidationError> {
    for field in fields {
        let WriteTarget::Data(path) = &field.target else {
            continue;
        };
        let leaf_expected = expected.and_then(|ty| type_at_path(ty, path));
        let Some(leaf) = expression_type(&field.expr, input, leaf_expected, require_input)? else {
            return Ok(None);
        };
        if path.is_empty() {
            current = Some(leaf);
        } else {
            current = Some(write_type_path(current, path, leaf)?);
        }
    }
    Ok(current)
}

fn emission_preserves_payload(emission: &Emission) -> bool {
    match emission {
        Emission::Identity => true,
        Emission::Data(tree) => {
            matches!(&tree.root, expr::Expr::Path(path) if path.as_slice() == ["data"])
        }
        Emission::Patch(fields) | Emission::Replace(fields) => {
            // Match the runtime's whole-payload witness rule: metadata-only
            // patches preserve data, and a final whole-data `data = data`
            // write makes every earlier payload write dead. This is a type
            // flow, not a spelling-based Variant fallback.
            let mut data_writes = fields
                .iter()
                .filter(|field| matches!(field.target, WriteTarget::Data(_)));
            let Some(last) = data_writes.next_back() else {
                return matches!(emission, Emission::Patch(_));
            };
            matches!(&last.target, WriteTarget::Data(path) if path.is_empty())
                && matches!(&last.expr.root, expr::Expr::Path(path) if path.as_slice() == ["data"])
        }
        Emission::Nothing => false,
    }
}

fn type_at_path<'a>(ty: &'a DataType, path: &[String]) -> Option<&'a DataType> {
    let mut current = ty;
    for segment in path {
        let DataType::Struct(structure) = current else {
            return None;
        };
        current = structure
            .fields()
            .iter()
            .find(|field| field.name() == segment)?
            .ty();
    }
    Some(current)
}

fn requirement_at_path(path: &[String], leaf: TypeRequirement) -> TypeRequirement {
    path.iter().rev().fold(leaf, |nested, segment| {
        TypeRequirement::Struct(BTreeMap::from([(segment.clone(), nested)]))
    })
}

fn close_requirement(requirement: &TypeRequirement) -> Result<DataType, PlanValidationError> {
    match requirement {
        TypeRequirement::Exact(ty) => Ok(ty.clone()),
        TypeRequirement::Struct(fields) => {
            let mut closed = Vec::with_capacity(fields.len());
            for (name, field) in fields {
                closed.push(TypeField::new(name.clone(), close_requirement(field)?));
            }
            StructType::new(closed)
                .map(DataType::Struct)
                .map_err(|reason| PlanValidationError::InvalidDataType {
                    location: String::from("inferred structural constraint"),
                    reason,
                })
        }
    }
}

fn satisfies(ty: &DataType, requirement: &TypeRequirement) -> bool {
    match requirement {
        TypeRequirement::Exact(expected) => ty.is_assignable_to(expected),
        TypeRequirement::Struct(required) => {
            let DataType::Struct(structure) = ty else {
                return false;
            };
            required.iter().all(|(name, requirement)| {
                structure
                    .fields()
                    .iter()
                    .find(|field| field.name() == name)
                    .is_some_and(|field| satisfies(field.ty(), requirement))
            })
        }
    }
}

fn merge_requirements(left: &TypeRequirement, right: &TypeRequirement) -> Option<TypeRequirement> {
    match (left, right) {
        (TypeRequirement::Exact(left), TypeRequirement::Exact(right)) => {
            if left.is_assignable_to(right) {
                Some(TypeRequirement::Exact(left.clone()))
            } else if right.is_assignable_to(left) {
                Some(TypeRequirement::Exact(right.clone()))
            } else {
                None
            }
        }
        (TypeRequirement::Exact(exact), structure @ TypeRequirement::Struct(_))
        | (structure @ TypeRequirement::Struct(_), TypeRequirement::Exact(exact)) => {
            satisfies(exact, structure).then(|| TypeRequirement::Exact(exact.clone()))
        }
        (TypeRequirement::Struct(left), TypeRequirement::Struct(right)) => {
            let mut merged = left.clone();
            for (name, requirement) in right {
                match merged.get(name) {
                    Some(existing) => {
                        merged.insert(name.clone(), merge_requirements(existing, requirement)?);
                    }
                    None => {
                        merged.insert(name.clone(), requirement.clone());
                    }
                }
            }
            Some(TypeRequirement::Struct(merged))
        }
    }
}

fn join_produced(
    left: &DataType,
    right: &DataType,
) -> Result<DataType, crate::datatype::TypeError> {
    if left.is_assignable_to(right) {
        Ok(right.clone())
    } else if right.is_assignable_to(left) {
        Ok(left.clone())
    } else {
        DataType::structural_union(alloc::vec![left.clone(), right.clone()])
    }
}

fn expression_requirement(expr: &expr::Expr, expected: &DataType) -> Option<TypeRequirement> {
    match expr {
        expr::Expr::Path(path) if path.first().is_some_and(|root| root == "data") => Some(
            requirement_at_path(&path[1..], TypeRequirement::Exact(expected.clone())),
        ),
        expr::Expr::Path(_) | expr::Expr::Literal(_) | expr::Expr::Asset(_) => None,
        expr::Expr::Unary { op, expr } => expression_requirement(
            expr,
            match op {
                expr::UnaryOp::Not => &DataType::Bool,
                expr::UnaryOp::Neg => expected,
            },
        ),
        expr::Expr::Binary { op, left, right } => {
            use expr::BinaryOp::{And, Eq, Ge, Gt, Le, Lt, Ne, Or};
            let (left_expected, right_expected) = match op {
                And | Or => (Some(DataType::Bool), Some(DataType::Bool)),
                Eq | Ne | Lt | Le | Gt | Ge => (
                    intrinsic_expression_type(right),
                    intrinsic_expression_type(left),
                ),
                _ => (Some(expected.clone()), Some(expected.clone())),
            };
            merge_optional_requirements(
                left_expected
                    .as_ref()
                    .and_then(|ty| expression_requirement(left, ty)),
                right_expected
                    .as_ref()
                    .and_then(|ty| expression_requirement(right, ty)),
            )
        }
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => merge_optional_requirements(
            expression_requirement(cond, &DataType::Bool),
            merge_optional_requirements(
                expression_requirement(then, expected),
                expression_requirement(otherwise, expected),
            ),
        ),
        expr::Expr::Index { base, index } => {
            let collection = match intrinsic_expression_type(index) {
                Some(DataType::Text) => DataType::text_map(expected.clone()),
                Some(DataType::Integer(_)) => DataType::array(expected.clone()),
                _ => return None,
            };
            expression_requirement(base, &collection)
        }
        expr::Expr::Call { function, args } => {
            let argument = match function {
                expr::Function::Duration | expr::Function::Timestamp | expr::Function::Decimal => {
                    Some(DataType::Text)
                }
                expr::Function::Size => None,
            }?;
            args.first()
                .and_then(|expr| expression_requirement(expr, &argument))
        }
        expr::Expr::List(items) => {
            let DataType::Array(array) = expected else {
                return None;
            };
            items.iter().fold(None, |merged, item| {
                merge_optional_requirements(merged, expression_requirement(item, array.element()))
            })
        }
        expr::Expr::MapLit(entries) => {
            let DataType::Map(map) = expected else {
                return None;
            };
            entries.iter().fold(None, |merged, (_, value)| {
                merge_optional_requirements(merged, expression_requirement(value, map.value()))
            })
        }
        // Narrowing inspects the input but does not infer that every value on
        // the source port inhabits the tested target.
        expr::Expr::Is { .. } | expr::Expr::As { .. } => None,
    }
}

fn merge_optional_requirements(
    left: Option<TypeRequirement>,
    right: Option<TypeRequirement>,
) -> Option<TypeRequirement> {
    match (left, right) {
        (Some(left), Some(right)) => merge_requirements(&left, &right),
        (Some(requirement), None) | (None, Some(requirement)) => Some(requirement),
        (None, None) => None,
    }
}

/// Resolves one expression against a known input type. This is deliberately
/// smaller than the expression checker's B3 work: it exists so B2 can tell a
/// real dynamic value from an unsolved variable. Once a path crosses an
/// explicit `variant`, its result remains `variant`; no precise constraint is
/// invented from later field/index spelling.
fn resolved_expression_type(expr: &expr::Expr, input: &DataType) -> Option<DataType> {
    match expr {
        expr::Expr::Path(path) if path.first().is_some_and(|root| root == "data") => {
            let mut current = input.clone();
            for segment in &path[1..] {
                current = match current {
                    DataType::Struct(structure) => structure
                        .fields()
                        .iter()
                        .find(|field| field.name() == segment)?
                        .ty()
                        .clone(),
                    DataType::Map(map) if map.key() == &DataType::Text => map.value().clone(),
                    DataType::Variant => DataType::Variant,
                    _ => return None,
                };
            }
            Some(current)
        }
        expr::Expr::Path(_) => intrinsic_expression_type(expr),
        expr::Expr::Literal(value) => intrinsic_value_type(value),
        expr::Expr::Asset(_) => Some(DataType::BYTES),
        expr::Expr::Unary { op, expr } => match op {
            expr::UnaryOp::Not => Some(DataType::Bool),
            expr::UnaryOp::Neg => resolved_expression_type(expr, input),
        },
        expr::Expr::Binary { op, left, right } => match op {
            expr::BinaryOp::Eq
            | expr::BinaryOp::Ne
            | expr::BinaryOp::Lt
            | expr::BinaryOp::Le
            | expr::BinaryOp::Gt
            | expr::BinaryOp::Ge
            | expr::BinaryOp::And
            | expr::BinaryOp::Or => Some(DataType::Bool),
            _ => {
                let left = resolved_expression_type(left, input)?;
                let right = resolved_expression_type(right, input)?;
                join_produced(&left, &right).ok()
            }
        },
        expr::Expr::Ternary {
            then, otherwise, ..
        } => {
            let then = resolved_expression_type(then, input)?;
            let otherwise = resolved_expression_type(otherwise, input)?;
            join_produced(&then, &otherwise).ok()
        }
        expr::Expr::Index { base, .. } => match resolved_expression_type(base, input)? {
            DataType::Array(array) => Some(array.element().clone()),
            DataType::Map(map) => Some(map.value().clone()),
            DataType::Variant => Some(DataType::Variant),
            _ => None,
        },
        expr::Expr::Call { function, .. } => Some(match function {
            expr::Function::Duration => DataType::DURATION,
            expr::Function::Timestamp => DataType::TIMESTAMP,
            expr::Function::Size => DataType::INT,
            expr::Function::Decimal => DataType::Variant,
        }),
        expr::Expr::List(items) => {
            let mut element = None;
            for item in items {
                let item = resolved_expression_type(item, input)?;
                element = Some(match element {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            element.map(DataType::array)
        }
        expr::Expr::MapLit(entries) => {
            let mut value = None;
            for (_, item) in entries {
                let item = resolved_expression_type(item, input)?;
                value = Some(match value {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            value.map(DataType::text_map)
        }
        expr::Expr::Is { .. } => Some(DataType::Bool),
        expr::Expr::As { target, .. } => Some(narrow_target_type(target)),
    }
}

fn intrinsic_expression_type(expr: &expr::Expr) -> Option<DataType> {
    match expr {
        expr::Expr::Literal(value) => intrinsic_value_type(value),
        expr::Expr::Asset(_) => Some(DataType::BYTES),
        expr::Expr::Unary { op, expr } => match op {
            expr::UnaryOp::Not => Some(DataType::Bool),
            expr::UnaryOp::Neg => intrinsic_expression_type(expr),
        },
        expr::Expr::Binary { op, left, right } => match op {
            expr::BinaryOp::Eq
            | expr::BinaryOp::Ne
            | expr::BinaryOp::Lt
            | expr::BinaryOp::Le
            | expr::BinaryOp::Gt
            | expr::BinaryOp::Ge
            | expr::BinaryOp::And
            | expr::BinaryOp::Or => Some(DataType::Bool),
            _ => {
                let left = intrinsic_expression_type(left)?;
                let right = intrinsic_expression_type(right)?;
                join_produced(&left, &right).ok()
            }
        },
        expr::Expr::Call { function, .. } => Some(match function {
            expr::Function::Duration => DataType::DURATION,
            expr::Function::Timestamp => DataType::TIMESTAMP,
            expr::Function::Size => DataType::INT,
            expr::Function::Decimal => DataType::Variant,
        }),
        expr::Expr::List(items) => {
            let mut element = None;
            for item in items {
                let item = intrinsic_expression_type(item)?;
                element = Some(match element {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            Some(DataType::array(element.unwrap_or(DataType::Variant)))
        }
        expr::Expr::MapLit(entries) => {
            let mut value = None;
            for (_, item) in entries {
                let item = intrinsic_expression_type(item)?;
                value = Some(match value {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            Some(DataType::text_map(value.unwrap_or(DataType::Variant)))
        }
        expr::Expr::Is { .. } => Some(DataType::Bool),
        expr::Expr::As { target, .. } => Some(narrow_target_type(target)),
        expr::Expr::Path(_) | expr::Expr::Ternary { .. } | expr::Expr::Index { .. } => None,
    }
}

fn intrinsic_value_type(value: &crate::value::Value) -> Option<DataType> {
    Some(match value {
        crate::value::Value::Bool(_) => DataType::Bool,
        crate::value::Value::Int(value) => {
            if i32::try_from(*value).is_ok() {
                DataType::INT
            } else {
                DataType::LONG
            }
        }
        crate::value::Value::Uint(_) => DataType::UINT,
        crate::value::Value::Double(_) => DataType::DOUBLE,
        crate::value::Value::Text(_) => DataType::Text,
        crate::value::Value::Bytes(_) => DataType::BYTES,
        crate::value::Value::Duration(_) => DataType::DURATION,
        crate::value::Value::Timestamp(_) => DataType::TIMESTAMP,
        crate::value::Value::Null => DataType::Null,
        crate::value::Value::Decimal { .. } => DataType::Variant,
        crate::value::Value::Array(items) => {
            let mut element = None;
            for item in items {
                let item = intrinsic_value_type(item)?;
                element = Some(match element {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            DataType::array(element.unwrap_or(DataType::Variant))
        }
        crate::value::Value::Map(entries) => {
            let mut item_type = None;
            for item in entries.values() {
                let item = intrinsic_value_type(item)?;
                item_type = Some(match item_type {
                    Some(current) => join_produced(&current, &item).ok()?,
                    None => item,
                });
            }
            DataType::text_map(item_type.unwrap_or(DataType::Variant))
        }
        crate::value::Value::Struct(fields) => {
            let mut resolved = Vec::with_capacity(fields.len());
            for (name, value) in fields {
                resolved.push(TypeField::new(name.clone(), intrinsic_value_type(value)?));
            }
            DataType::Struct(StructType::new(resolved).ok()?)
        }
        crate::value::Value::Enum(_)
        | crate::value::Value::StructuralUnion { .. }
        | crate::value::Value::TaggedUnion(_) => DataType::Variant,
    })
}

fn write_type_path(
    current: Option<DataType>,
    path: &[String],
    leaf: DataType,
) -> Result<DataType, PlanValidationError> {
    let Some((head, rest)) = path.split_first() else {
        return Ok(leaf);
    };
    let mut fields: BTreeMap<String, DataType> = match current {
        Some(DataType::Struct(structure)) => structure
            .fields()
            .iter()
            .map(|field| (field.name().to_string(), field.ty().clone()))
            .collect(),
        Some(DataType::Variant) => return Ok(DataType::Variant),
        _ => BTreeMap::new(),
    };
    let nested = write_type_path(fields.remove(head), rest, leaf)?;
    fields.insert(head.clone(), nested);
    let structure = StructType::new(
        fields
            .into_iter()
            .map(|(name, ty)| TypeField::new(name, ty))
            .collect(),
    )
    .map_err(|reason| PlanValidationError::InvalidDataType {
        location: String::from("inferred record payload"),
        reason,
    })?;
    Ok(DataType::Struct(structure))
}

fn expression_reads_data(expr: &expr::Expr) -> bool {
    let mut stack = alloc::vec![expr];
    while let Some(expr) = stack.pop() {
        match expr {
            expr::Expr::Path(path) if path.first().is_some_and(|root| root == "data") => {
                return true;
            }
            expr::Expr::Path(_) | expr::Expr::Literal(_) | expr::Expr::Asset(_) => {}
            expr::Expr::Unary { expr, .. }
            | expr::Expr::Is { expr, .. }
            | expr::Expr::As { expr, .. } => stack.push(expr),
            expr::Expr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                stack.push(cond);
                stack.push(then);
                stack.push(otherwise);
            }
            expr::Expr::Index { base, index } => {
                stack.push(base);
                stack.push(index);
            }
            expr::Expr::Call { args, .. } | expr::Expr::List(args) => stack.extend(args),
            expr::Expr::MapLit(entries) => {
                stack.extend(entries.iter().map(|(_, value)| value));
            }
        }
    }
    false
}

fn narrow_target_type(target: &expr::NarrowTarget) -> DataType {
    match target {
        expr::NarrowTarget::Type(target) => target.ty().clone(),
        expr::NarrowTarget::Case { payload, .. } => (**payload).clone(),
    }
}
