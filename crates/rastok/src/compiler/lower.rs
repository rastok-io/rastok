//! Canonicalization and lowering: HIR to `SemanticHash` and
//! `ExecutionPlan`.
//!
//! The canonical byte form follows `identity.md` §1: insensitive to
//! formatting, file order, and declaration order — nodes, vals,
//! attributes, vars, ports, and edges are sorted before hashing —
//! sensitive to topology, statement order inside a handler, every Rastok expression
//! as its canonical tree, and `var` defaults. Constants
//! hash as the expressions they were written as, before const-eval;
//! spans never enter the bytes. The plan then ships the reduced
//! values.

use crate::compiler::hir::{
    AttrValue, Direction, EdgeTarget, EmitValue, EndpointOwner, Graph as HirGraph, Hir,
    InstanceLifetime, InstancePolicy, Node as HirNode, NodeSemantics, Statement as HirStatement,
};
use crate::datatype::{
    BinaryType, DataType, DateUnit, FloatType, IntegerType, IntegerWidth, IntervalUnit,
    TemporalType, TimeUnit, TimeZone,
};
use crate::expr;
use crate::node;
use crate::plan::{
    AspectSpec, AttrSetting, ClosingPolicy as PlanClosingPolicy, ConnectorSpec, Edge, Emission,
    EndpointRef, EvaluationSite, ExecutionPlan, FieldWrite, Handler, HandlerOwner,
    InstanceLifetime as PlanInstanceLifetime, InstanceSpec, LoopDelay as PlanLoopDelay,
    LoopSpec as PlanLoopSpec, Node, NodeExecution, Owner, OwnerKind, OwnerPath, OwnerSlot,
    OwnerValue, PlanConstant, SemanticHash, Slot, Step, SymbolTable, VarCell,
};
use crate::schema_annotation::{
    AnnotationTable, CostBasis, DataAnnotation, SchemaPathSegment, SensitivityLevel,
};

/// Computes the semantic hash of a checked graph.
#[must_use]
pub fn semantic_hash(hir: &Hir) -> SemanticHash {
    let mut writer = Writer {
        hasher: blake3::Hasher::new(),
        assets: &hir.assets,
    };
    writer.str(&hir.app.name);
    let boundary = sorted_boundary(hir);
    writer.count(boundary.len());
    for decl in &boundary {
        writer.str(decl.port.name());
        writer.direction(decl.direction);
        // The pinned type is identity material — a public contract;
        // absence hashes as absence (the var-init idiom). NOTE: the
        // absence tag itself moved every identity with a non-empty
        // table when it landed — declared in D-2026-08-08-3, pinned
        // by the gauge fixture.
        match &decl.ty {
            None => writer.tag(0),
            Some(ty) => {
                writer.tag(1);
                writer.ty(ty);
            }
        }
        writer.annotations(&decl.annotations);
    }
    let mut vals: Vec<_> = hir.app.vals.iter().collect();
    vals.sort_by(|a, b| a.name.cmp(&b.name));
    writer.count(vals.len());
    for val in vals {
        writer.str(&val.name);
        writer.expr(&val.value.tree.root);
        writer.annotations(&val.annotations);
    }
    let mut nodes: Vec<_> = sorted_nodes(hir);
    writer.count(nodes.len());
    for node in &mut nodes {
        writer.node(node);
    }
    let edges = sorted_edges(hir);
    writer.count(edges.len());
    for (text, _) in edges {
        writer.str(&text);
    }
    let nested_graphs: Vec<_> = sorted_graphs(hir)
        .into_iter()
        .filter(|graph| !graph.path.is_root())
        .collect();
    if !nested_graphs.is_empty() {
        writer.str("rastok.graph.owners.r1");
        writer.count(nested_graphs.len());
        for graph in &nested_graphs {
            writer.str(&graph.path.to_string());
            let mut boundary = graph.boundary.clone();
            boundary.sort_by(|left, right| left.key().cmp(&right.key()));
            writer.count(boundary.len());
            for decl in boundary {
                writer.str(decl.port.name());
                writer.direction(decl.direction);
                match decl.ty {
                    None => writer.tag(0),
                    Some(ty) => {
                        writer.tag(1);
                        writer.ty(&ty);
                    }
                }
                writer.annotations(&decl.annotations);
            }
            let mut vals: Vec<_> = graph.vals.iter().collect();
            vals.sort_by(|left, right| left.name.cmp(&right.name));
            writer.count(vals.len());
            for val in vals {
                writer.str(&val.name);
                writer.expr(&val.value.tree.root);
                writer.annotations(&val.annotations);
            }
        }
    }
    let aspect_graphs: Vec<_> = sorted_graphs(hir)
        .into_iter()
        .filter(|graph| !graph.aspects.is_empty())
        .collect();
    if !aspect_graphs.is_empty() {
        writer.str("rastok.aspects.r1");
        writer.count(aspect_graphs.len());
        for graph in aspect_graphs {
            writer.str(&graph.path.to_string());
            writer.count(graph.aspects.len());
            for aspect in &graph.aspects {
                match aspect {
                    node::Aspect::Loop => {
                        writer.tag(1);
                        match &graph.loop_spec {
                            None => writer.tag(0),
                            Some(spec) => match &spec.fuel {
                                None => writer.tag(0),
                                Some(fuel) => {
                                    writer.tag(1);
                                    writer.tree(&fuel.tree);
                                    match &spec.delay {
                                        None => writer.tag(0),
                                        Some(crate::compiler::hir::LoopDelay::Fixed { after }) => {
                                            writer.tag(1);
                                            writer.tree(&after.tree);
                                        }
                                        Some(crate::compiler::hir::LoopDelay::Exponential {
                                            first,
                                            ceiling,
                                        }) => {
                                            writer.tag(2);
                                            writer.tree(&first.tree);
                                            writer.tree(&ceiling.tree);
                                        }
                                    }
                                }
                            },
                        }
                    }
                    node::Aspect::Saga => writer.tag(2),
                }
            }
            let mut handlers: Vec<_> = graph.handlers.iter().collect();
            handlers.sort_by(|left, right| {
                left.activation
                    .canonical_key()
                    .cmp(&right.activation.canonical_key())
                    .then_with(|| {
                        semantic_handler_order_key(left, hir.assets.as_slice())
                            .cmp(&semantic_handler_order_key(right, hir.assets.as_slice()))
                    })
            });
            writer.count(handlers.len());
            for handler in handlers {
                writer.handler(handler);
            }
        }
    }
    let proved_graphs: Vec<_> = sorted_graphs(hir)
        .into_iter()
        .filter_map(|graph| {
            graph
                .loop_spec
                .as_ref()
                .and_then(|spec| spec.proof.as_ref().map(|proof| (graph, spec, proof)))
        })
        .collect();
    if !proved_graphs.is_empty() {
        writer.str("rastok.loop-proof.r1");
        writer.count(proved_graphs.len());
        for (graph, spec, proof) in proved_graphs {
            writer.str(&graph.path.to_string());
            writer.tree(&proof.measure);
            match &proof.invariant {
                None => writer.tag(0),
                Some(invariant) => {
                    writer.tag(1);
                    writer.tree(invariant);
                }
            }
            if spec.fuel.is_none() {
                match &spec.delay {
                    None => writer.tag(0),
                    Some(crate::compiler::hir::LoopDelay::Fixed { after }) => {
                        writer.tag(1);
                        writer.tree(&after.tree);
                    }
                    Some(crate::compiler::hir::LoopDelay::Exponential { first, ceiling }) => {
                        writer.tag(2);
                        writer.tree(&first.tree);
                        writer.tree(&ceiling.tree);
                    }
                }
            }
        }
    }
    // Extend identity only when the new source declares non-default instance
    // semantics. The persistent singleton still feeds the exact M1.3 stream.
    let configured: Vec<_> = sorted_nodes(hir)
        .into_iter()
        .filter_map(|node| node.instance.as_ref().map(|instance| (node, instance)))
        .collect();
    let configured_graphs: Vec<_> = nested_graphs
        .iter()
        .filter_map(|graph| graph.instance.as_ref().map(|instance| (*graph, instance)))
        .collect();
    if hir.app.instance.is_some() || !configured.is_empty() || !configured_graphs.is_empty() {
        writer.str("rastok.instances.r1");
        writer.count(
            configured.len() + configured_graphs.len() + usize::from(hir.app.instance.is_some()),
        );
        if let Some(instance) = &hir.app.instance {
            writer.tag(1);
            writer.str(&hir.app.name);
            writer.instance(instance);
        }
        for (node, instance) in configured {
            writer.tag(2);
            writer.str(&node.path.to_string());
            writer.instance(instance);
        }
        for (graph, instance) in configured_graphs {
            writer.tag(3);
            writer.str(&graph.path.to_string());
            writer.instance(instance);
        }
    }
    let configured_closing: Vec<_> = sorted_nodes(hir)
        .into_iter()
        .filter_map(|node| {
            node.instance
                .as_ref()
                .and_then(|instance| instance.closing.as_ref().map(|closing| (node, closing)))
        })
        .collect();
    let app_closing = hir
        .app
        .instance
        .as_ref()
        .and_then(|instance| instance.closing.as_ref());
    let configured_graph_closing: Vec<_> = nested_graphs
        .iter()
        .filter_map(|graph| {
            graph
                .instance
                .as_ref()
                .and_then(|instance| instance.closing.as_ref().map(|closing| (*graph, closing)))
        })
        .collect();
    if app_closing.is_some()
        || !configured_closing.is_empty()
        || !configured_graph_closing.is_empty()
    {
        writer.str("rastok.closing.r1");
        writer.count(
            configured_closing.len()
                + configured_graph_closing.len()
                + usize::from(app_closing.is_some()),
        );
        if let Some(closing) = app_closing {
            writer.tag(1);
            writer.str(&hir.app.name);
            writer.tree(&closing.drain_for.tree);
            writer.tree(&closing.settle_for.tree);
        }
        for (node, closing) in configured_closing {
            writer.tag(2);
            writer.str(&node.path.to_string());
            writer.tree(&closing.drain_for.tree);
            writer.tree(&closing.settle_for.tree);
        }
        for (graph, closing) in configured_graph_closing {
            writer.tag(3);
            writer.str(&graph.path.to_string());
            writer.tree(&closing.drain_for.tree);
            writer.tree(&closing.settle_for.tree);
        }
    }
    let mut bytes = [0u8; 32];
    writer.hasher.finalize_xof().fill(&mut bytes);
    SemanticHash::from_bytes(bytes)
}

fn semantic_handler_order_key(
    handler: &crate::compiler::hir::Handler,
    assets: &[crate::plan::AssetBlob],
) -> [u8; 32] {
    let mut writer = Writer {
        hasher: blake3::Hasher::new(),
        assets,
    };
    writer.str("rastok.handler.order.r1");
    writer.handler(handler);
    *writer.hasher.finalize().as_bytes()
}

/// Boundary ports in canonical order — by (port, direction); the
/// optional pinned type never decides an order.
fn sorted_boundary(hir: &Hir) -> Vec<crate::plan::PortDecl> {
    let mut boundary = hir.app.boundary.clone();
    boundary.sort_by(|a, b| a.key().cmp(&b.key()));
    boundary
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatEdge {
    scope: crate::plan::OwnerPath,
    from: crate::compiler::hir::Endpoint,
    to: EdgeTarget,
}

/// Graph boundaries are compile-time composition seams. The categorical plan
/// carries their owner rows, but executable routes connect the first concrete
/// producer to the first concrete consumer (or the root App boundary).
fn sorted_edges(hir: &Hir) -> Vec<(String, FlatEdge)> {
    let mut adjacency: std::collections::BTreeMap<crate::compiler::hir::Endpoint, Vec<EdgeTarget>> =
        std::collections::BTreeMap::new();
    collect_edge_adjacency(&hir.app, &mut adjacency);
    for targets in adjacency.values_mut() {
        targets.sort_by_key(edge_target_text);
    }

    let mut flattened = Vec::new();
    let starts: Vec<_> = adjacency
        .keys()
        .filter(|endpoint| match &endpoint.owner {
            EndpointOwner::Node(_) => true,
            EndpointOwner::Graph(path) => path.is_root(),
        })
        .cloned()
        .collect();
    for start in starts {
        let mut visiting = std::collections::BTreeSet::new();
        flatten_from(&start, &start, &adjacency, &mut visiting, &mut flattened);
    }
    flattened.sort_by_key(flat_edge_text);
    flattened.dedup();
    flattened
        .into_iter()
        .map(|edge| (flat_edge_text(&edge), edge))
        .collect()
}

fn collect_edge_adjacency(
    graph: &HirGraph,
    adjacency: &mut std::collections::BTreeMap<crate::compiler::hir::Endpoint, Vec<EdgeTarget>>,
) {
    for edge in &graph.edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }
    for nested in &graph.graphs {
        collect_edge_adjacency(nested, adjacency);
    }
}

fn flatten_from(
    origin: &crate::compiler::hir::Endpoint,
    current: &crate::compiler::hir::Endpoint,
    adjacency: &std::collections::BTreeMap<crate::compiler::hir::Endpoint, Vec<EdgeTarget>>,
    visiting: &mut std::collections::BTreeSet<crate::compiler::hir::Endpoint>,
    flattened: &mut Vec<FlatEdge>,
) {
    if !visiting.insert(current.clone()) {
        return;
    }
    for target in adjacency.get(current).into_iter().flatten() {
        match target {
            EdgeTarget::Discard => flattened.push(FlatEdge {
                scope: endpoint_scope(origin),
                from: origin.clone(),
                to: EdgeTarget::Discard,
            }),
            EdgeTarget::Port(endpoint) => match &endpoint.owner {
                EndpointOwner::Graph(path) if !path.is_root() => {
                    flatten_from(origin, endpoint, adjacency, visiting, flattened);
                }
                EndpointOwner::Node(_) | EndpointOwner::Graph(_) => {
                    flattened.push(FlatEdge {
                        scope: common_graph_scope(origin, endpoint),
                        from: origin.clone(),
                        to: EdgeTarget::Port(endpoint.clone()),
                    });
                }
            },
        }
    }
    visiting.remove(current);
}

fn endpoint_scope(endpoint: &crate::compiler::hir::Endpoint) -> crate::plan::OwnerPath {
    match &endpoint.owner {
        EndpointOwner::Node(path) => crate::plan::OwnerPath::from(path)
            .parent()
            .unwrap_or_else(crate::plan::OwnerPath::root),
        EndpointOwner::Graph(path) => path.clone(),
    }
}

fn common_graph_scope(
    left: &crate::compiler::hir::Endpoint,
    right: &crate::compiler::hir::Endpoint,
) -> crate::plan::OwnerPath {
    endpoint_scope(left).common_ancestor(&endpoint_scope(right))
}

fn edge_target_text(target: &EdgeTarget) -> String {
    match target {
        EdgeTarget::Port(endpoint) => endpoint_text(endpoint),
        EdgeTarget::Discard => String::from("!discard"),
    }
}

fn flat_edge_text(edge: &FlatEdge) -> String {
    format!(
        "{}>{}",
        endpoint_text(&edge.from),
        edge_target_text(&edge.to)
    )
}

/// Lowers a checked graph into the shipped plan and its symbol table.
pub fn lower(hir: &Hir) -> Result<(ExecutionPlan, SymbolTable), LowerError> {
    let nodes = sorted_nodes(hir);
    for graph in sorted_graphs(hir) {
        let expected = if graph.path.is_root() {
            node::Type::App
        } else {
            node::Type::Graph
        };
        if graph.ty != expected {
            return Err(LowerError {
                owner: graph.path.to_string(),
                detail: "a checked Graph carries an incompatible Node type",
            });
        }
        if graph
            .aspects
            .iter()
            .any(|aspect| *aspect != node::Aspect::Loop)
            || (graph.aspects.contains(&node::Aspect::Loop) && graph.loop_spec.is_none())
            || (!graph.aspects.contains(&node::Aspect::Loop) && graph.loop_spec.is_some())
        {
            return Err(LowerError {
                owner: graph.path.to_string(),
                detail: "a Graph aspect reached lowering without its complete categorical specification",
            });
        }
    }
    if let Some(node) = nodes.iter().find(|node| !node.aspects.is_empty()) {
        return Err(LowerError {
            owner: node.path.to_string(),
            detail: "a Node aspect reached lowering before its plan gate",
        });
    }
    let slot_of = |path: &node::Path| {
        Slot::new(
            u32::try_from(
                nodes
                    .iter()
                    .position(|candidate| &candidate.path == path)
                    .unwrap_or_else(|| unreachable!("edges resolved against these nodes")),
            )
            .unwrap_or_else(|_| unreachable!("node count fits u32")),
        )
    };
    let mut plan = ExecutionPlan {
        owners: Vec::new(),
        nodes: Vec::new(),
        boundary: sorted_boundary(hir),
        edges: Vec::new(),
        handlers: Vec::new(),
        // DERIVED from the writes the hash already seals — carried
        // for the ContractCatalog-less validator (D-2026-08-10-1), never
        // hashed twice.
        app_schema: hir.app_schema.clone(),
        ext_schema: hir.ext_schema.clone(),
        assets: hir.assets.clone(),
    };
    enum CheckedOwner<'hir> {
        Graph(&'hir HirGraph),
        Node(&'hir HirNode),
    }
    let mut checked_owners: Vec<(OwnerPath, CheckedOwner<'_>)> = sorted_graphs(hir)
        .into_iter()
        .map(|graph| (graph.path.clone(), CheckedOwner::Graph(graph)))
        .collect();
    checked_owners.extend(
        nodes
            .iter()
            .map(|node| (OwnerPath::from(&node.path), CheckedOwner::Node(node))),
    );
    checked_owners.sort_by(|left, right| left.0.cmp(&right.0));

    let mut owner_slots = std::collections::BTreeMap::new();
    let mut node_owners = std::collections::BTreeMap::new();
    for (index, (path, checked)) in checked_owners.into_iter().enumerate() {
        let owner_index = u32::try_from(index).map_err(|_| LowerError {
            owner: path.to_string(),
            detail: "the owner table exceeds its u32 slot domain",
        })?;
        let owner_slot = OwnerSlot::new(owner_index);
        let parent_path = path.parent();
        let parent = match parent_path.as_ref() {
            None => None,
            Some(parent_path) => Some(*owner_slots.get(parent_path).ok_or_else(|| LowerError {
                owner: path.to_string(),
                detail: "the checked owner tree lost its lexical parent",
            })?),
        };
        let graph_path = match &checked {
            CheckedOwner::Graph(graph) => graph.path.clone(),
            CheckedOwner::Node(node) => {
                OwnerPath::from(&node.path)
                    .parent()
                    .ok_or_else(|| LowerError {
                        owner: node.path.to_string(),
                        detail: "an executable Node cannot own the root Graph path",
                    })?
            }
        };
        let mut constants = graph_constant_environment(hir, &graph_path);
        let (kind, ports, aspects, instance, vals) = match checked {
            CheckedOwner::Graph(graph) => {
                let mut ports = if graph.path.is_root() {
                    Vec::new()
                } else {
                    graph.boundary.clone()
                };
                ports.sort_by(|left, right| left.key().cmp(&right.key()));
                let aspects = graph
                    .loop_spec
                    .as_ref()
                    .map(|spec| vec![AspectSpec::Loop(lower_loop_spec(spec, &constants))])
                    .unwrap_or_default();
                (
                    OwnerKind::Graph,
                    ports,
                    aspects,
                    lower_instance(graph.instance.as_ref(), &constants, &graph.path.to_string())?,
                    lower_owner_values(&graph.vals),
                )
            }
            CheckedOwner::Node(node) => {
                constants.extend(
                    node.vals
                        .iter()
                        .map(|value| (value.name.clone(), value.value.value.clone())),
                );
                let slot = slot_of(&node.path);
                node_owners.insert(node.path.clone(), owner_slot);
                (
                    OwnerKind::Node(slot),
                    Vec::new(),
                    Vec::new(),
                    lower_instance(node.instance.as_ref(), &constants, &node.path.to_string())?,
                    lower_owner_values(&node.vals),
                )
            }
        };
        plan.owners.push(Owner {
            path: path.clone(),
            parent,
            kind,
            ports,
            aspects,
            instance,
            vals,
        });
        owner_slots.insert(path, owner_slot);
    }
    let mut symbols = SymbolTable { nodes: Vec::new() };
    for hir_node in &nodes {
        let owner = node_owners
            .get(&hir_node.path)
            .copied()
            .ok_or_else(|| LowerError {
                owner: hir_node.path.to_string(),
                detail: "the canonical owner table omitted an executable Node",
            })?;
        symbols.nodes.push(hir_node.path.to_string());
        let owner_path =
            OwnerPath::new(hir_node.path.segments().iter().cloned()).map_err(|_| LowerError {
                owner: hir_node.path.to_string(),
                detail: "a checked Node path did not form an owner path",
            })?;
        let graph_path = owner_path.parent().unwrap_or_else(OwnerPath::root);
        let mut constants = graph_runtime_constant_environment(hir, &graph_path);
        constants.extend(
            hir_node
                .vals
                .iter()
                .filter(|value| value.annotations.is_empty())
                .map(|value| (value.name.clone(), value.value.value.clone())),
        );
        // Declaratively unordered collections ship in canonical order,
        // so equal hashes always mean equal plans.
        let mut attributes: Vec<_> = hir_node
            .attributes()
            .iter()
            .filter(|attribute| !matches!(attribute.name.as_str(), "effect" | "behavior"))
            .map(|attribute| {
                let setting = match &attribute.value {
                    AttrValue::Word(word) => AttrSetting::Word(word.clone()),
                    AttrValue::Const(constant) => AttrSetting::Value(constant.value.clone()),
                    AttrValue::Expr { site, tree } => AttrSetting::Expr {
                        site: *site,
                        expr: substitute_tree(tree, &constants),
                    },
                };
                (attribute.name.clone(), setting)
            })
            .collect();
        attributes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut ports = hir_node.ports.clone();
        ports.sort_by(|a, b| a.key().cmp(&b.key()));
        let mut vars: Vec<_> = hir_node
            .vars
            .iter()
            .map(|var| VarCell {
                name: var.name.clone(),
                ty: var.ty.clone(),
                init: var.init.as_ref().map(|init| init.value.clone()),
                annotations: var.annotations.clone(),
            })
            .collect();
        vars.sort_by(|a, b| a.name.cmp(&b.name));
        plan.nodes.push(Node {
            path: hir_node.path.clone(),
            execution: match (&hir_node.ty, &hir_node.semantics) {
                (
                    node::Type::Connector(contract),
                    NodeSemantics::Connector {
                        roles, exchange, ..
                    },
                ) => NodeExecution::Connector(ConnectorSpec {
                    contract: contract.clone(),
                    roles: *roles,
                    exchange: *exchange,
                    effect: typed_effect(hir_node),
                    behavior: typed_behavior(hir_node),
                    attributes,
                }),
                (node::Type::Operator, NodeSemantics::Operator) => NodeExecution::Operator,
                _ => {
                    return Err(LowerError {
                        owner: hir_node.path.to_string(),
                        detail: "a checked Node type disagrees with its family semantics",
                    });
                }
            },
            ports,
            vars,
            owner,
        });
    }
    for hir_node in &nodes {
        let slot = slot_of(&hir_node.path);
        // Plain constants are substituted into shipped trees. Annotated
        // constants deliberately remain owner-value paths so the independent
        // policy pass can prove that dependency instead of losing it in a
        // literal rewrite.
        let graph_path = OwnerPath::from(&hir_node.path)
            .parent()
            .ok_or_else(|| LowerError {
                owner: hir_node.path.to_string(),
                detail: "an executable Node cannot own the root Graph path",
            })?;
        let mut consts = graph_runtime_constant_environment(hir, &graph_path);
        for val in &hir_node.vals {
            if val.annotations.is_empty() {
                consts.insert(val.name.clone(), val.value.value.clone());
            }
        }
        let subst = |tree: &expr::Tree| substitute_tree(tree, &consts);
        // Declaration order between handlers carries no semantics and
        // no identity (identity.md): they ship sorted by their
        // activation's canonical key, the same key the hash sorts by.
        let mut handlers: Vec<_> = hir_node.handlers.iter().collect();
        handlers.sort_by_key(|handler| handler.activation.canonical_key());
        for handler in handlers {
            plan.handlers.push(Handler {
                owner: HandlerOwner::Node(slot),
                activation: handler.activation.clone(),
                guard: handler.guard.as_ref().map(&subst),
                body: handler
                    .body
                    .iter()
                    .map(|statement| match statement {
                        HirStatement::Let {
                            name,
                            value,
                            annotations,
                        } => Step::Let {
                            name: name.clone(),
                            expr: subst(value),
                            annotations: annotations.clone(),
                        },
                        HirStatement::SetVar { name, value } => Step::SetVar {
                            name: name.clone(),
                            expr: subst(value),
                        },
                        HirStatement::Emit { port, value } => Step::Emit {
                            port: port.clone(),
                            value: lower_emission(value, &subst),
                        },
                        HirStatement::Stop => Step::Stop,
                    })
                    .collect(),
            });
        }
    }
    for graph in sorted_graphs(hir) {
        let owner = *owner_slots.get(&graph.path).ok_or_else(|| LowerError {
            owner: graph.path.to_string(),
            detail: "the canonical owner table omitted a Graph handler owner",
        })?;
        let constants = graph_runtime_constant_environment(hir, &graph.path);
        let subst = |tree: &expr::Tree| substitute_tree(tree, &constants);
        let mut handlers: Vec<_> = graph
            .handlers
            .iter()
            .map(|handler| Handler {
                owner: HandlerOwner::Graph(owner),
                activation: handler.activation.clone(),
                guard: handler.guard.as_ref().map(&subst),
                body: handler
                    .body
                    .iter()
                    .map(|statement| lower_statement(statement, &subst))
                    .collect(),
            })
            .collect();
        handlers.sort_by(|left, right| {
            left.activation
                .canonical_key()
                .cmp(&right.activation.canonical_key())
                .then_with(|| left.canonical_order_key().cmp(&right.canonical_order_key()))
        });
        plan.handlers.extend(handlers);
    }
    plan.handlers.sort_by(|left, right| {
        left.owner
            .cmp(&right.owner)
            .then_with(|| {
                left.activation
                    .canonical_key()
                    .cmp(&right.activation.canonical_key())
            })
            .then_with(|| left.canonical_order_key().cmp(&right.canonical_order_key()))
    });
    for (_, edge) in sorted_edges(hir) {
        let endpoint = |end: &crate::compiler::hir::Endpoint| -> Result<EndpointRef, LowerError> {
            let node = match &end.owner {
                EndpointOwner::Node(path) => Some(slot_of(path)),
                EndpointOwner::Graph(path) if path.is_root() => None,
                EndpointOwner::Graph(path) => {
                    return Err(LowerError {
                        owner: path.to_string(),
                        detail: "a nested Graph boundary survived composition lowering",
                    });
                }
            };
            Ok(EndpointRef {
                node,
                port: end.port.clone(),
            })
        };
        let scope = *owner_slots.get(&edge.scope).ok_or_else(|| LowerError {
            owner: edge.scope.to_string(),
            detail: "a lowered edge lost its lexical Graph scope",
        })?;
        plan.edges.push(Edge {
            scope,
            from: endpoint(&edge.from)?,
            to: match &edge.to {
                EdgeTarget::Port(end) => Some(endpoint(end)?),
                EdgeTarget::Discard => None,
            },
        });
    }
    Ok((plan, symbols))
}

fn lower_owner_values(values: &[crate::compiler::hir::Val]) -> Vec<OwnerValue> {
    let mut lowered: Vec<_> = values
        .iter()
        .map(|value| OwnerValue {
            name: value.name.clone(),
            site: EvaluationSite::Plan,
            expr: value.value.tree.clone(),
            resolved: Some(value.value.value.clone()),
            annotations: value.annotations.clone(),
        })
        .collect();
    lowered.sort_by(|left, right| left.name.cmp(&right.name));
    lowered
}

fn lower_loop_spec(
    spec: &crate::compiler::hir::LoopSpec,
    constants: &std::collections::BTreeMap<String, crate::value::Value>,
) -> PlanLoopSpec {
    let constant = |value: &crate::compiler::hir::Const| PlanConstant {
        expr: substitute_tree(&value.tree, constants),
        resolved: value.value.clone(),
    };
    PlanLoopSpec {
        fuel: spec.fuel.as_ref().map(constant),
        delay: spec.delay.as_ref().map(|delay| match delay {
            crate::compiler::hir::LoopDelay::Fixed { after } => PlanLoopDelay::Fixed {
                after: constant(after),
            },
            crate::compiler::hir::LoopDelay::Exponential { first, ceiling } => {
                PlanLoopDelay::Exponential {
                    first: constant(first),
                    ceiling: constant(ceiling),
                }
            }
        }),
        proof: spec.proof.clone(),
    }
}

fn lower_statement(statement: &HirStatement, subst: &impl Fn(&expr::Tree) -> expr::Tree) -> Step {
    match statement {
        HirStatement::Let {
            name,
            value,
            annotations,
        } => Step::Let {
            name: name.clone(),
            expr: subst(value),
            annotations: annotations.clone(),
        },
        HirStatement::SetVar { name, value } => Step::SetVar {
            name: name.clone(),
            expr: subst(value),
        },
        HirStatement::Emit { port, value } => Step::Emit {
            port: port.clone(),
            value: lower_emission(value, subst),
        },
        HirStatement::Stop => Step::Stop,
    }
}

fn lower_instance(
    policy: Option<&InstancePolicy>,
    constants: &std::collections::BTreeMap<String, crate::value::Value>,
    owner: &str,
) -> Result<InstanceSpec, LowerError> {
    let Some(policy) = policy else {
        return Ok(InstanceSpec {
            key: None,
            lifetime: PlanInstanceLifetime::Persistent,
            closing: None,
        });
    };
    let duration = |value: &crate::compiler::hir::Const| match &value.value {
        crate::value::Value::Duration(nanos) => Ok(*nanos),
        _ => Err(LowerError {
            owner: owner.to_string(),
            detail: "the checked instance lifetime lost its duration carrier",
        }),
    };
    let lifetime = match &policy.lifetime {
        InstanceLifetime::Persistent => PlanInstanceLifetime::Persistent,
        InstanceLifetime::Idle(value) => PlanInstanceLifetime::Idle(duration(value)?),
        InstanceLifetime::Fixed(value) => PlanInstanceLifetime::Fixed(duration(value)?),
        InstanceLifetime::SinglePass => PlanInstanceLifetime::SinglePass,
        InstanceLifetime::Tumbling(value) => PlanInstanceLifetime::Tumbling(duration(value)?),
    };
    Ok(InstanceSpec {
        key: policy
            .key
            .as_ref()
            .map(|key| substitute_tree(key, constants)),
        lifetime,
        closing: match &policy.closing {
            None => None,
            Some(closing) => Some(PlanClosingPolicy {
                drain_for: duration(&closing.drain_for)?,
                settle_for: duration(&closing.settle_for)?,
            }),
        },
    })
}

/// A checked instance policy whose runtime profile is not executable at the
/// current M1.4 gate. V10 retains transport for every closed profile; lowering refuses
/// before packaging until the corresponding scheduler rule exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    /// Source owner whose checked policy cannot ship yet.
    pub owner: String,
    /// Named missing transport/runtime gate.
    pub detail: &'static str,
}

fn lower_emission(value: &EmitValue, subst: &impl Fn(&expr::Tree) -> expr::Tree) -> Emission {
    let writes = |fields: &[crate::compiler::hir::FieldWrite]| {
        fields
            .iter()
            .map(|write| FieldWrite {
                target: write.target.clone(),
                expr: subst(&write.value),
            })
            .collect()
    };
    match value {
        EmitValue::Identity => Emission::Identity,
        EmitValue::Patch(fields) => Emission::Patch(writes(fields)),
        EmitValue::FreshData(fields) => Emission::Replace(writes(fields)),
        EmitValue::Data(tree) => Emission::Data(subst(tree)),
        EmitValue::Nothing => Emission::Nothing,
    }
}

/// The canonical spelling of a resolved write target — what the
/// semantic hash seals, alias-free.
fn canonical_target(target: &crate::envelope::WriteTarget) -> String {
    match target {
        crate::envelope::WriteTarget::Data(path) if path.is_empty() => String::from("data"),
        crate::envelope::WriteTarget::Data(path) => format!("data.{}", path.join(".")),
        crate::envelope::WriteTarget::App(field) => format!("meta.app.{field}"),
        crate::envelope::WriteTarget::Ext(field) => format!("meta.ext.{field}"),
    }
}

/// Rewrites every constant reference into its literal value; member
/// access on a constant becomes evaluable indexing. Sound because
/// shadowing is refused at check time.
fn substitute_tree(
    tree: &expr::Tree,
    consts: &std::collections::BTreeMap<String, crate::value::Value>,
) -> expr::Tree {
    expr::Tree {
        root: substitute(&tree.root, consts),
        ty: tree.ty.clone(),
    }
}

fn substitute(
    expr: &expr::Expr,
    consts: &std::collections::BTreeMap<String, crate::value::Value>,
) -> expr::Expr {
    let recurse = |inner: &expr::Expr| Box::new(substitute(inner, consts));
    match expr {
        expr::Expr::Asset(_) => expr.clone(),
        expr::Expr::Path(segments) => match consts.get(&segments[0]) {
            None => expr.clone(),
            Some(value) => {
                let mut result = expr::Expr::Literal(value.clone());
                for segment in &segments[1..] {
                    result = expr::Expr::Index {
                        base: Box::new(result),
                        index: Box::new(expr::Expr::Literal(crate::value::Value::Text(
                            segment.clone(),
                        ))),
                    };
                }
                result
            }
        },
        expr::Expr::Literal(_) => expr.clone(),
        expr::Expr::Unary { op, expr } => expr::Expr::Unary {
            op: *op,
            expr: recurse(expr),
        },
        expr::Expr::Binary { op, left, right } => expr::Expr::Binary {
            op: *op,
            left: recurse(left),
            right: recurse(right),
        },
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => expr::Expr::Ternary {
            cond: recurse(cond),
            then: recurse(then),
            otherwise: recurse(otherwise),
        },
        expr::Expr::Index { base, index } => expr::Expr::Index {
            base: recurse(base),
            index: recurse(index),
        },
        expr::Expr::Call { function, args } => expr::Expr::Call {
            function: *function,
            args: args.iter().map(|arg| substitute(arg, consts)).collect(),
        },
        expr::Expr::List(items) => {
            expr::Expr::List(items.iter().map(|item| substitute(item, consts)).collect())
        }
        expr::Expr::MapLit(entries) => expr::Expr::MapLit(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, consts)))
                .collect(),
        ),
        expr::Expr::Struct(fields) => expr::Expr::Struct(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, consts)))
                .collect(),
        ),
        expr::Expr::Tagged {
            union,
            case,
            fields,
        } => expr::Expr::Tagged {
            union: union.clone(),
            case: case.clone(),
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, consts)))
                .collect(),
        },
        expr::Expr::Is { expr, target } => expr::Expr::Is {
            expr: recurse(expr),
            target: target.clone(),
        },
        expr::Expr::As { expr, target } => expr::Expr::As {
            expr: recurse(expr),
            target: target.clone(),
        },
    }
}

/// Nodes in canonical order: (kind, path), per `identity.md` §1's
/// sorted top-level declarations. Slots index this order.
fn sorted_nodes(hir: &Hir) -> Vec<&HirNode> {
    fn collect<'hir>(graph: &'hir HirGraph, nodes: &mut Vec<&'hir HirNode>) {
        nodes.extend(&graph.nodes);
        for nested in &graph.graphs {
            collect(nested, nodes);
        }
    }
    let mut nodes = Vec::new();
    collect(&hir.app, &mut nodes);
    nodes.sort_by_key(|node| (kind_rank(node.kind()), node.path.to_string()));
    nodes
}

fn sorted_graphs(hir: &Hir) -> Vec<&HirGraph> {
    fn collect<'hir>(graph: &'hir HirGraph, graphs: &mut Vec<&'hir HirGraph>) {
        graphs.push(graph);
        for nested in &graph.graphs {
            collect(nested, graphs);
        }
    }
    let mut graphs = Vec::new();
    collect(&hir.app, &mut graphs);
    graphs.sort_by(|left, right| left.path.cmp(&right.path));
    graphs
}

fn graph_for_path<'hir>(
    graph: &'hir HirGraph,
    path: &crate::plan::OwnerPath,
) -> Option<&'hir HirGraph> {
    if &graph.path == path {
        return Some(graph);
    }
    graph
        .graphs
        .iter()
        .find_map(|nested| graph_for_path(nested, path))
}

fn graph_constant_environment(
    hir: &Hir,
    path: &crate::plan::OwnerPath,
) -> std::collections::BTreeMap<String, crate::value::Value> {
    let mut values = std::collections::BTreeMap::new();
    let mut cursor = Some(path.clone());
    let mut chain = Vec::new();
    while let Some(path) = cursor {
        chain.push(path.clone());
        cursor = path.parent();
    }
    chain.reverse();
    for path in chain {
        if let Some(graph) = graph_for_path(&hir.app, &path) {
            values.extend(
                graph
                    .vals
                    .iter()
                    .map(|value| (value.name.clone(), value.value.value.clone())),
            );
        }
    }
    values
}

/// Constants safe to substitute into a runtime expression without erasing a
/// policy dependency. Annotated values stay as lexical paths: their runtime
/// value already lives in the owner bundle, and the categorical validator
/// must still be able to derive their policy from that binding.
fn graph_runtime_constant_environment(
    hir: &Hir,
    path: &crate::plan::OwnerPath,
) -> std::collections::BTreeMap<String, crate::value::Value> {
    let mut values = std::collections::BTreeMap::new();
    let mut cursor = Some(path.clone());
    let mut chain = Vec::new();
    while let Some(path) = cursor {
        chain.push(path.clone());
        cursor = path.parent();
    }
    chain.reverse();
    for path in chain {
        if let Some(graph) = graph_for_path(&hir.app, &path) {
            values.extend(
                graph
                    .vals
                    .iter()
                    .filter(|value| value.annotations.is_empty())
                    .map(|value| (value.name.clone(), value.value.value.clone())),
            );
        }
    }
    values
}

/// The checker guarantees the mandatory axes exist with valid words
/// before any plan is built; the plan makes them irrevocable types.
fn typed_effect(node: &HirNode) -> node::Effect {
    let word = attribute_word(node, "effect");
    match word {
        "none" => node::Effect::None,
        "idempotent" => node::Effect::Class(node::EffectClass::Idempotent),
        "compensable" => node::Effect::Class(node::EffectClass::Compensable),
        "irreversible" => node::Effect::Class(node::EffectClass::Irreversible),
        other => unreachable!("the contract sheet admitted `{other}`"),
    }
}

fn typed_behavior(node: &HirNode) -> node::Behavior {
    match attribute_word(node, "behavior") {
        "deterministic" => node::Behavior::Deterministic,
        "probabilistic" => node::Behavior::Probabilistic,
        other => unreachable!("the contract sheet admitted `{other}`"),
    }
}

fn attribute_word<'hir>(node: &'hir HirNode, name: &str) -> &'hir str {
    node.attributes()
        .iter()
        .find_map(|attribute| match (&attribute.name, &attribute.value) {
            (attr, AttrValue::Word(word)) if attr == name => Some(word.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| unreachable!("the contract sheet requires `{name}`"))
}

fn kind_rank(kind: node::Kind) -> u8 {
    match kind {
        node::Kind::Operator => 0,
        node::Kind::Connector => 1,
        node::Kind::Graph => 2,
    }
}

fn endpoint_text(endpoint: &crate::compiler::hir::Endpoint) -> String {
    match &endpoint.owner {
        EndpointOwner::Node(path) => format!("{path}.{}", endpoint.port),
        EndpointOwner::Graph(path) if path.is_root() => format!(".{}", endpoint.port),
        EndpointOwner::Graph(path) => format!("{path}.{}", endpoint.port),
    }
}

/// Deterministic canonical byte writer feeding blake3. Every variable
/// datum is length- or count-prefixed; every variant carries a tag —
/// the same injectivity discipline as the id framing.
struct Writer<'assets> {
    hasher: blake3::Hasher,
    /// The asset table — `Expr::Asset` hashes by content digest.
    assets: &'assets [crate::plan::AssetBlob],
}

impl Writer<'_> {
    fn tag(&mut self, tag: u8) {
        self.hasher.update(&[tag]);
    }

    fn count(&mut self, count: usize) {
        let count = u32::try_from(count).unwrap_or_else(|_| unreachable!("canonical counts fit"));
        self.u32(count);
    }

    fn str(&mut self, text: &str) {
        self.count(text.len());
        self.hasher.update(text.as_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.hasher.update(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.hasher.update(&value.to_le_bytes());
    }

    fn direction(&mut self, direction: Direction) {
        self.tag(match direction {
            Direction::Input => 1,
            Direction::Output => 2,
        });
    }

    /// Conditional semantic framing for policy metadata. Empty tables write
    /// no byte, preserving every pre-M1.8 semantic identity exactly.
    fn annotations(&mut self, table: &AnnotationTable) {
        if table.is_empty() {
            return;
        }
        self.str("rastok.metering.r1");
        self.count(table.entries().len());
        for (path, set) in table.entries() {
            self.count(path.segments().len());
            for segment in path.segments() {
                match segment {
                    SchemaPathSegment::Field(name) => {
                        self.tag(1);
                        self.str(name);
                    }
                    SchemaPathSegment::ArrayElement => self.tag(2),
                    SchemaPathSegment::MapKey => self.tag(3),
                    SchemaPathSegment::MapValue => self.tag(4),
                    SchemaPathSegment::StructuralUnionBranch(index) => {
                        self.tag(5);
                        self.u32(u32::from(*index));
                    }
                    SchemaPathSegment::TaggedUnionCase(name) => {
                        self.tag(6);
                        self.str(name);
                    }
                }
            }
            self.count(set.annotations().len());
            for annotation in set.annotations() {
                match annotation {
                    DataAnnotation::Sensitive(level) => {
                        self.tag(1);
                        self.tag(match level {
                            SensitivityLevel::Internal => 1,
                            SensitivityLevel::Confidential => 2,
                            SensitivityLevel::Restricted => 3,
                        });
                    }
                    DataAnnotation::Secret => self.tag(2),
                    DataAnnotation::Measure(id) => {
                        self.tag(3);
                        self.str(id.as_str());
                    }
                    DataAnnotation::Cost(cost) => {
                        self.tag(4);
                        self.tag(match cost.basis() {
                            CostBasis::ProviderReported => 1,
                            CostBasis::PricebookCalculated => 2,
                        });
                        match cost.currency() {
                            None => self.tag(0),
                            Some(currency) => {
                                self.tag(1);
                                self.str(currency.as_str());
                            }
                        }
                    }
                    DataAnnotation::Personal(class) => {
                        self.tag(5);
                        self.tag(
                            u8::from(class.is_special_category())
                                | (u8::from(class.is_criminal_offence()) << 1)
                                | (u8::from(class.is_identifier()) << 2),
                        );
                    }
                }
            }
        }
    }

    /// Stable domain tags for the logical data-type vocabulary — never a Rust
    /// `Debug` rendering, which renames could silently change.
    fn ty(&mut self, ty: &DataType) {
        // The historical scalar aliases retain their domain tags. Composite
        // and parameterized forms extend that registry; the v7 package uses
        // its own transport tags over the same resolved DataType authority.
        if ty == &DataType::INT {
            self.tag(2);
            return;
        }
        if ty == &DataType::UINT {
            self.tag(3);
            return;
        }
        if ty == &DataType::DOUBLE {
            self.tag(4);
            return;
        }
        if ty == &DataType::BYTES {
            self.tag(6);
            return;
        }
        if ty == &DataType::DURATION {
            self.tag(7);
            return;
        }
        if ty == &DataType::TIMESTAMP {
            self.tag(8);
            return;
        }
        if let DataType::Array(array) = ty
            && array.length().is_none()
        {
            self.tag(11);
            self.ty(array.element());
            return;
        }
        if let DataType::Map(map) = ty
            && map.key() == &DataType::Text
        {
            self.tag(12);
            self.ty(map.value());
            return;
        }

        match ty {
            DataType::Bool => self.tag(1),
            DataType::Text => self.tag(5),
            DataType::MediaType => self.tag(26),
            DataType::Null => self.tag(9),
            DataType::Variant => self.tag(10),
            DataType::Integer(integer) => {
                self.tag(13);
                let (signedness, width) = match integer {
                    IntegerType::Signed(width) => (1, *width),
                    IntegerType::Unsigned(width) => (2, *width),
                };
                self.tag(signedness);
                self.tag(match width {
                    IntegerWidth::Bits8 => 8,
                    IntegerWidth::Bits16 => 16,
                    IntegerWidth::Bits32 => 32,
                    IntegerWidth::Bits64 => 64,
                });
            }
            DataType::Float(float) => {
                self.tag(14);
                self.tag(match float {
                    FloatType::Binary16 => 16,
                    FloatType::Binary32 => 32,
                    FloatType::Binary64 => 64,
                });
            }
            DataType::Decimal(decimal) => {
                self.tag(15);
                self.tag(decimal.precision());
                self.i32(i32::from(decimal.scale()));
            }
            DataType::Number => self.tag(16),
            DataType::Bytes(BinaryType::Variable) => self.tag(6),
            DataType::Bytes(BinaryType::Fixed(length)) => {
                self.tag(17);
                self.u32(length.get());
            }
            DataType::Temporal(temporal) => {
                self.tag(18);
                match temporal {
                    TemporalType::Date(unit) => {
                        self.tag(1);
                        self.tag(match unit {
                            DateUnit::Day => 1,
                            DateUnit::Millisecond => 2,
                        });
                    }
                    TemporalType::Time(unit) => {
                        self.tag(2);
                        self.time_unit(*unit);
                    }
                    TemporalType::Timestamp { unit, zone } => {
                        self.tag(3);
                        self.time_unit(*unit);
                        match zone {
                            TimeZone::Utc => self.tag(1),
                            TimeZone::Named(name) => {
                                self.tag(2);
                                self.str(name.as_str());
                            }
                        }
                    }
                    TemporalType::TimestampNtz(unit) => {
                        self.tag(4);
                        self.time_unit(*unit);
                    }
                    TemporalType::Duration(unit) => {
                        self.tag(5);
                        self.time_unit(*unit);
                    }
                    TemporalType::Interval(unit) => {
                        self.tag(6);
                        self.tag(match unit {
                            IntervalUnit::YearMonth => 1,
                            IntervalUnit::DayTime => 2,
                            IntervalUnit::MonthDayNano => 3,
                            IntervalUnit::MonthDayMillisUnsigned => 4,
                        });
                    }
                }
            }
            DataType::Uuid => self.tag(19),
            DataType::Struct(structure) => {
                self.tag(20);
                self.count(structure.fields().len());
                for field in structure.fields() {
                    self.str(field.name());
                    self.ty(field.ty());
                }
            }
            DataType::Map(map) => {
                self.tag(21);
                self.ty(map.key());
                self.ty(map.value());
            }
            DataType::Array(array) => {
                self.tag(22);
                self.ty(array.element());
                match array.length() {
                    None => self.tag(0),
                    Some(length) => {
                        self.tag(1);
                        self.u32(length.get());
                    }
                }
            }
            DataType::Enum(enumeration) => {
                self.tag(23);
                self.count(enumeration.symbols().len());
                for symbol in enumeration.symbols() {
                    self.str(symbol);
                }
            }
            DataType::StructuralUnion(union) => {
                self.tag(24);
                self.count(union.branches().len());
                for branch in union.branches() {
                    self.ty(branch);
                }
            }
            DataType::TaggedUnion(union) => {
                self.tag(25);
                self.count(union.cases().len());
                for case in union.cases() {
                    self.str(case.name());
                    self.count(case.payload().fields().len());
                    for field in case.payload().fields() {
                        self.str(field.name());
                        self.ty(field.ty());
                    }
                }
            }
        }
    }

    fn time_unit(&mut self, unit: TimeUnit) {
        self.tag(match unit {
            TimeUnit::Second => 1,
            TimeUnit::Millisecond => 2,
            TimeUnit::Microsecond => 3,
            TimeUnit::Nanosecond => 4,
        });
    }

    fn node(&mut self, node: &HirNode) {
        self.tag(match node.kind() {
            node::Kind::Operator => 1,
            node::Kind::Connector => 2,
            node::Kind::Graph => 3,
        });
        self.str(&node.path.to_string());
        match (&node.ty, &node.semantics) {
            (node::Type::Operator, crate::compiler::hir::NodeSemantics::Operator) => self.tag(0),
            (
                node::Type::Connector(contract),
                crate::compiler::hir::NodeSemantics::Connector {
                    roles, exchange, ..
                },
            ) => {
                self.tag(1);
                self.str(contract.as_str());
                self.tag(roles.tag());
                self.tag(exchange.tag());
            }
            _ => {
                // Executable HIR cannot contain an incompatible type/family
                // pair. Keep the hash total without granting that recovery
                // state a semantic spelling.
                self.tag(u8::MAX);
            }
        }
        let mut vals: Vec<_> = node.vals.iter().collect();
        vals.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(vals.len());
        for val in vals {
            self.str(&val.name);
            self.tree(&val.value.tree);
            self.annotations(&val.annotations);
        }
        let mut attributes: Vec<_> = node.attributes().iter().collect();
        attributes.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(attributes.len());
        for attribute in attributes {
            self.str(&attribute.name);
            match &attribute.value {
                AttrValue::Word(word) => {
                    self.tag(1);
                    self.str(word);
                }
                AttrValue::Const(constant) => {
                    self.tag(2);
                    self.tree(&constant.tree);
                }
                AttrValue::Expr { site, tree } => {
                    self.tag(3);
                    self.tag(*site as u8);
                    self.tree(tree);
                }
            }
        }
        let mut vars: Vec<_> = node.vars.iter().collect();
        vars.sort_by(|a, b| a.name.cmp(&b.name));
        self.count(vars.len());
        for var in vars {
            self.str(&var.name);
            self.ty(&var.ty);
            match &var.init {
                None => self.tag(0),
                Some(init) => {
                    self.tag(1);
                    self.tree(&init.tree);
                }
            }
            self.annotations(&var.annotations);
        }
        let mut ports: Vec<_> = node.ports.iter().collect();
        ports.sort_by_key(|decl| (decl.port.name(), decl.direction));
        self.count(ports.len());
        for decl in ports {
            self.str(decl.port.name());
            self.tag(match (decl.direction, decl.unwired) {
                (Direction::Input, crate::plan::UnwiredPolicy::RequireWire) => 1,
                (Direction::Output, crate::plan::UnwiredPolicy::RequireWire) => 2,
                (Direction::Output, crate::plan::UnwiredPolicy::RootSink) => 3,
                (Direction::Input, crate::plan::UnwiredPolicy::RootSink) => 4,
            });
            // The pinned type, present-or-absent — same idiom as the
            // boundary and the var initializers.
            match &decl.ty {
                None => self.tag(0),
                Some(ty) => {
                    self.tag(1);
                    self.ty(ty);
                }
            }
            self.annotations(&decl.annotations);
        }
        // Sorted by activation key: the identity is insensitive to
        // declaration order between handlers (identity.md) — only
        // statements INSIDE a handler are order-sensitive.
        let mut handlers: Vec<_> = node.handlers.iter().collect();
        handlers.sort_by_key(|handler| handler.activation.canonical_key());
        self.count(handlers.len());
        for handler in handlers {
            self.handler(handler);
        }
    }

    fn handler(&mut self, handler: &crate::compiler::hir::Handler) {
        // Hash material: the canonical activation key. Display is
        // presentation and free to move.
        self.str(&handler.activation.canonical_key());
        if let node::HandlerActivation::Join { last, .. } = &handler.activation {
            self.str("rastok.join.bounds.r1");
            self.tag(1);
            match &handler.join_within {
                None => self.tag(0),
                Some(within) => {
                    self.tag(1);
                    self.tree(&within.tree);
                }
            }
            match last {
                None => self.tag(0),
                Some(count) => {
                    self.tag(1);
                    self.u32(*count);
                }
            }
        }
        match &handler.guard {
            None => self.tag(0),
            Some(guard) => {
                self.tag(1);
                self.tree(guard);
            }
        }
        self.count(handler.body.len());
        for statement in &handler.body {
            match statement {
                HirStatement::Let {
                    name,
                    value,
                    annotations,
                } => {
                    self.tag(1);
                    self.str(name);
                    self.tree(value);
                    self.annotations(annotations);
                }
                HirStatement::Emit { port, value } => {
                    self.tag(2);
                    self.str(port.name());
                    self.emission(value);
                }
                HirStatement::SetVar { name, value } => {
                    self.tag(3);
                    self.str(name);
                    self.tree(value);
                }
                HirStatement::Stop => self.tag(4),
            }
        }
    }

    fn instance(&mut self, instance: &InstancePolicy) {
        match &instance.key {
            None => self.tag(0),
            Some(key) => {
                self.tag(1);
                self.tree(key);
            }
        }
        match &instance.lifetime {
            InstanceLifetime::Persistent => self.tag(1),
            InstanceLifetime::Idle(value) => {
                self.tag(2);
                self.tree(&value.tree);
            }
            InstanceLifetime::Fixed(value) => {
                self.tag(3);
                self.tree(&value.tree);
            }
            InstanceLifetime::SinglePass => self.tag(4),
            InstanceLifetime::Tumbling(value) => {
                self.tag(5);
                self.tree(&value.tree);
            }
        }
    }

    fn emission(&mut self, value: &EmitValue) {
        match value {
            EmitValue::Identity => self.tag(1),
            EmitValue::Patch(writes) => {
                self.tag(2);
                self.field_writes(writes);
            }
            EmitValue::FreshData(writes) => {
                self.tag(3);
                self.field_writes(writes);
            }
            EmitValue::Data(tree) => {
                self.tag(4);
                self.tree(tree);
            }
            EmitValue::Nothing => self.tag(5),
        }
    }

    fn field_writes(&mut self, writes: &[crate::compiler::hir::FieldWrite]) {
        self.count(writes.len());
        for write in writes {
            // The RESOLVED target is identity material — identity.md
            // makes the hash sugar-spelling-insensitive, so
            // `meta.flag` and `meta.app.flag` seal identically.
            self.str(&canonical_target(&write.target));
            self.tree(&write.value);
        }
    }

    /// A compiled tree seals its STATIC TYPE with its expression —
    /// `json! [1]` (variant) and `[1]` (list) are different programs
    /// (D-2026-08-10-7), exactly as a pinned port differs from an
    /// unpinned one.
    fn tree(&mut self, tree: &expr::Tree) {
        self.ty(&tree.ty);
        self.expr(&tree.root);
    }

    fn expr(&mut self, expr: &expr::Expr) {
        match expr {
            expr::Expr::Literal(value) => {
                self.tag(1);
                self.value(value);
            }
            expr::Expr::Asset(index) => {
                // The CONTENT digest is the identity — the path is
                // spelling and the index is layout (D-2026-08-10-5).
                self.tag(9);
                let digest = self
                    .assets
                    .get(*index as usize)
                    .map(|blob| blob.digest)
                    .unwrap_or_else(|| unreachable!("the checker minted the index"));
                self.hasher.update(&digest);
            }
            expr::Expr::Path(segments) => {
                self.tag(2);
                self.str(&segments.join("."));
            }
            expr::Expr::Unary { op, expr } => {
                self.tag(3);
                self.tag(match op {
                    expr::UnaryOp::Not => 1,
                    expr::UnaryOp::Neg => 2,
                });
                self.expr(expr);
            }
            expr::Expr::Binary { op, left, right } => {
                self.tag(4);
                self.tag(match op {
                    expr::BinaryOp::Mul => 1,
                    expr::BinaryOp::Div => 2,
                    expr::BinaryOp::Rem => 3,
                    expr::BinaryOp::Add => 4,
                    expr::BinaryOp::Sub => 5,
                    expr::BinaryOp::Eq => 6,
                    expr::BinaryOp::Ne => 7,
                    expr::BinaryOp::Lt => 8,
                    expr::BinaryOp::Le => 9,
                    expr::BinaryOp::Gt => 10,
                    expr::BinaryOp::Ge => 11,
                    expr::BinaryOp::And => 12,
                    expr::BinaryOp::Or => 13,
                });
                self.expr(left);
                self.expr(right);
            }
            expr::Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.tag(5);
                self.expr(cond);
                self.expr(then);
                self.expr(otherwise);
            }
            expr::Expr::Index { base, index } => {
                self.tag(6);
                self.expr(base);
                self.expr(index);
            }
            expr::Expr::Call { function, args } => {
                self.tag(7);
                self.tag(match function {
                    expr::Function::Asset => 5,
                    expr::Function::Duration => 1,
                    expr::Function::Timestamp => 2,
                    expr::Function::Size => 3,
                    expr::Function::Decimal => 4,
                    expr::Function::MediaType => 6,
                });
                self.count(args.len());
                for arg in args {
                    self.expr(arg);
                }
            }
            expr::Expr::List(items) => {
                self.tag(8);
                self.count(items.len());
                for item in items {
                    self.expr(item);
                }
            }
            expr::Expr::MapLit(entries) => {
                self.tag(9);
                // Maps are semantically unordered; the canonical form
                // sorts keys (duplicates are rejected at compilation).
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (key, value) in sorted {
                    self.str(key);
                    self.expr(value);
                }
            }
            expr::Expr::Struct(fields) => {
                self.tag(12);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, value) in sorted {
                    self.str(name);
                    self.expr(value);
                }
            }
            expr::Expr::Tagged {
                union,
                case,
                fields,
            } => {
                self.tag(13);
                self.ty(&DataType::TaggedUnion((**union).clone()));
                self.str(case);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, value) in sorted {
                    self.str(name);
                    self.expr(value);
                }
            }
            expr::Expr::Is { expr, target } => {
                self.tag(10);
                self.expr(expr);
                self.narrow_target(target);
            }
            expr::Expr::As { expr, target } => {
                self.tag(11);
                self.expr(expr);
                self.narrow_target(target);
            }
        }
    }

    fn narrow_target(&mut self, target: &expr::NarrowTarget) {
        match target {
            expr::NarrowTarget::Type(target) => {
                self.tag(1);
                self.ty(target.ty());
                match target.discriminator() {
                    None => self.tag(0),
                    Some(discriminator) => {
                        self.tag(1);
                        self.str(discriminator.field());
                        self.count(discriminator.mapping().len());
                        for (external, case) in discriminator.mapping() {
                            self.str(external);
                            self.str(case);
                        }
                    }
                }
            }
            expr::NarrowTarget::Case { name, payload } => {
                self.tag(2);
                self.str(name);
                self.ty(payload);
            }
        }
    }

    fn value(&mut self, value: &crate::value::Value) {
        match value {
            crate::value::Value::Decimal { unscaled, scale } => {
                // Tag 13 in the HASH register: the canonical pair —
                // reached by `json` literals (D-2026-08-10-7).
                self.tag(13);
                self.hasher.update(&unscaled.to_be_bytes());
                self.hasher.update(&scale.to_be_bytes());
            }
            crate::value::Value::Bool(b) => {
                self.tag(1);
                self.tag(u8::from(*b));
            }
            crate::value::Value::Int(i) => {
                self.tag(2);
                self.hasher.update(&i.to_le_bytes());
            }
            crate::value::Value::Uint(u) => {
                self.tag(3);
                self.hasher.update(&u.to_le_bytes());
            }
            crate::value::Value::Double(d) => {
                self.tag(4);
                self.hasher.update(&d.to_bits().to_le_bytes());
            }
            crate::value::Value::Text(text) => {
                self.tag(5);
                self.str(text);
            }
            crate::value::Value::Bytes(bytes) => {
                self.tag(6);
                self.count(bytes.len());
                self.hasher.update(bytes);
            }
            crate::value::Value::Duration(nanos) => {
                self.tag(7);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Timestamp(nanos) => {
                self.tag(8);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Date(days) => {
                self.tag(17);
                self.hasher.update(&days.to_le_bytes());
            }
            crate::value::Value::Time(nanos) => {
                self.tag(18);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::TimestampNtz(nanos) => {
                self.tag(19);
                self.hasher.update(&nanos.to_le_bytes());
            }
            crate::value::Value::Interval {
                months,
                days,
                nanoseconds,
            } => {
                self.tag(20);
                self.hasher.update(&months.to_le_bytes());
                self.hasher.update(&days.to_le_bytes());
                self.hasher.update(&nanoseconds.to_le_bytes());
            }
            crate::value::Value::Uuid(bytes) => {
                self.tag(21);
                self.hasher.update(bytes);
            }
            crate::value::Value::Null => self.tag(9),
            crate::value::Value::Array(items) => {
                self.tag(10);
                self.count(items.len());
                for item in items {
                    self.value(item);
                }
            }
            crate::value::Value::Map(map) => {
                self.tag(11);
                self.count(map.len());
                for (key, item) in map {
                    self.str(key);
                    self.value(item);
                }
            }
            crate::value::Value::KeyedMap(entries) => {
                self.tag(22);
                let mut sorted: Vec<_> = entries.iter().collect();
                sorted.sort_by_cached_key(|(key, _)| {
                    crate::canonical_cbor::encode(&crate::identity::encoding::value_tagged(key))
                });
                self.count(sorted.len());
                for (key, item) in sorted {
                    self.value(key);
                    self.value(item);
                }
            }
            crate::value::Value::Struct(fields) => {
                // A literal or reduced host constructor reaches this form.
                // Structs are semantically unordered, so identity uses the
                // same canonical field-name order as the value registry.
                self.tag(12);
                let mut sorted: Vec<_> = fields.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, item) in sorted {
                    self.str(name);
                    self.value(item);
                }
            }
            crate::value::Value::Enum(symbol) => {
                self.tag(14);
                self.str(symbol);
            }
            crate::value::Value::StructuralUnion { branch, value } => {
                self.tag(15);
                self.hasher.update(&branch.to_le_bytes());
                self.value(value);
            }
            crate::value::Value::TaggedUnion(tagged) => {
                self.tag(16);
                self.str(&tagged.case);
                let mut sorted: Vec<_> = tagged.payload.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                self.count(sorted.len());
                for (name, item) in sorted {
                    self.str(name);
                    self.value(item);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DataType;
    use crate::compiler::compile;
    use crate::contract::ContractCatalog;
    use crate::diagnostics::{Bag, SourceId};
    use crate::node;
    use crate::plan::{AttrSetting, Emission, GraphVersion, Manifest, Step};
    use crate::schema_annotation::{AnnotationKind, DataAnnotation, SchemaPath, SensitivityLevel};

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in where data.text != "" {
      out = data.text
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_compiles_to_a_plan() {
        let compilation = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        assert!(compilation.warnings.is_empty());
        let plan = &compilation.plan;
        assert_eq!(plan.nodes.len(), 2);
        // Canonical order: operators before connectors.
        assert_eq!(plan.nodes[0].path.to_string(), "shout");
        assert_eq!(plan.nodes[1].path.to_string(), "echo");
        let spec = plan.nodes[1].connector().expect("a connector spec");
        assert_eq!(
            spec.contract.as_str(),
            crate::connector::PROCESS_CONNECTOR_ID
        );
        assert_eq!(spec.roles, crate::connector::ConnectorRoles::Outbound);
        assert_eq!(
            spec.attribute("command"),
            Some(&AttrSetting::Value(crate::value::Value::Text(
                String::from("cat")
            )))
        );
        assert_eq!(plan.handlers.len(), 1);
        let handler = &plan.handlers[0];
        assert_eq!(handler.owner.node().map(|slot| slot.index()), Some(0));
        assert_eq!(
            handler.activation,
            node::HandlerActivation::Port(node::Port::new("in").unwrap())
        );
        assert!(matches!(
            &handler.body[0],
            Step::Emit {
                value: Emission::Data(_),
                ..
            }
        ));
        assert_eq!(plan.edges.len(), 3);
        assert_eq!(compilation.symbols.nodes, ["shout", "echo"]);
        let resolved = plan.resolved_ports();
        let boundary_in = resolved
            .boundary()
            .iter()
            .find(|port| port.port().name() == "in")
            .expect("the used boundary input resolves");
        assert!(matches!(boundary_in.ty(), DataType::Struct(_)));
        assert_eq!(boundary_in.origin(), crate::plan::TypeOrigin::Inferred);
        assert_eq!(
            resolved
                .node(crate::plan::Slot::new(0))
                .expect("the operator row")
                .iter()
                .find(|port| port.port().name() == "out")
                .expect("the operator output")
                .ty(),
            &DataType::Text
        );
    }

    #[test]
    fn annotated_schema_crosses_source_hir_v15_and_decode_exactly() {
        let source = "type Secret = text @secret\ngraph app : App { input in: Secret }\n";
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let source_id = SourceId::new(0);
        let mut bag = Bag::new();
        let tree = crate::compiler::parser::parse(source_id, source, &registry, &mut bag);
        let unit = crate::compiler::check::Unit {
            files: vec![(source_id, tree)],
        };
        let hir = crate::compiler::check::check(&unit, &registry, &mut bag, None)
            .expect("the annotated program checks");
        assert!(!bag.has_errors(), "v15 owns the transport gate");
        let hash = super::semantic_hash(&hir);
        let (raw, _) = super::lower(&hir).expect("annotations lower beside their schema");
        let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(raw)
            .expect("the core revalidates the annotation table");
        let bytes = crate::plan::encode(&plan, &hash);
        let (decoded, decoded_hash) =
            crate::plan::decode(&bytes).expect("the v15 package reads back");
        assert_eq!(decoded_hash, hash);
        assert_eq!(decoded, plan);
        let annotations = &decoded.plan().boundary[0].annotations;
        assert!(!annotations.is_empty());
        assert_eq!(
            decoded.resolved_ports().boundary()[0].annotations(),
            annotations,
            "the raw and independently derived resolved schemas agree"
        );

        let mut vm = crate::runtime::vm::Vm::boot(&decoded, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        assert!(matches!(
            vm.step(&mut journal),
            Err(crate::runtime::vm::VmError::UnsupportedDataPolicy { .. })
        ));
    }

    #[test]
    fn v15_transports_val_var_and_let_annotations_at_their_own_schema_sites() {
        let source = r#"
graph app : App {
  input in: text
  val label: text @sensitive(level = internal) = "demo"
  operator o {
    var credential: text @secret = "placeholder"
    on in {
      val tokens: int @measure(id = gen_ai.usage.output_tokens) = 1
      out = .
    }
  }
  . -> o -> .
}
"#;
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let compilation = compile(&[source], &registry).expect("the annotated program compiles");
        assert!(
            !compilation.plan.plan().owners[0].vals[0]
                .annotations
                .is_empty()
        );
        assert!(
            !compilation.plan.plan().nodes[0].vars[0]
                .annotations
                .is_empty()
        );
        assert!(matches!(
            &compilation.plan.plan().handlers[0].body[0],
            Step::Let { annotations, .. } if !annotations.is_empty()
        ));

        let bytes = crate::plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, hash) = crate::plan::decode(&bytes).expect("v15 reads every schema site");
        assert_eq!(hash, compilation.hash);
        assert_eq!(decoded, compilation.plan);
    }

    #[test]
    fn annotations_move_only_annotated_semantic_identity() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let source = |annotation: &str| {
            format!(
                "graph app : App {{\n  operator o {{\n    input in: text{annotation}\n    on in {{ out = data }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let plain =
            compile(&[source("").as_str()], &registry).expect("the unannotated program compiles");
        let internal = compile(
            &[source(" @sensitive(level = internal)").as_str()],
            &registry,
        )
        .expect("the annotated program compiles");
        let spaced = compile(
            &[source(" @sensitive( level = internal )").as_str()],
            &registry,
        )
        .expect("formatting is not identity");
        let restricted = compile(
            &[source(" @sensitive(level = restricted)").as_str()],
            &registry,
        )
        .expect("the second policy compiles");
        assert_ne!(plain.hash, internal.hash);
        assert_eq!(internal.hash, spaced.hash);
        assert_ne!(internal.hash, restricted.hash);
        assert_eq!(
            internal.hash.to_string(),
            "f984f003953161e8fa93025787bd92a44fd18a8615aee1a0fbe4b9f1ba22defe",
            "the rastok.metering.r1 spelling is identity material"
        );
    }

    #[test]
    fn exact_data_policy_crosses_copy_projection_and_categorical_decode() {
        let source = r#"
type Usage = struct {
  prompt: text @sensitive(level = confidential)
  tokens: int @measure(id = gen_ai.usage.input_tokens)
}
graph app : App {
  input in: Usage
  output out: int
  operator relay {
    input in: Usage
    output out: Usage
    on in { out = data }
  }
  operator project {
    input in: Usage
    output out: int
    on in { out = data.tokens }
  }
  . -> relay -> project -> .
}
"#;
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let compilation = compile(&[source], &registry).expect("exact policy copy compiles");
        let input = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "in")
            .expect("boundary input");
        let relay_slot = compilation
            .plan
            .plan()
            .nodes
            .iter()
            .position(|node| node.path.to_string() == "relay")
            .map(|index| crate::plan::Slot::new(u32::try_from(index).expect("small plan")))
            .expect("relay node");
        let relay_output = compilation
            .plan
            .resolved_ports()
            .node(relay_slot)
            .expect("relay port row")
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("relay output");
        assert_eq!(relay_output.annotations(), input.annotations());

        let output = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("boundary output");
        assert!(matches!(
            output
                .annotations()
                .entries()
                .get(&SchemaPath::root())
                .map(|set| set.annotations()),
            Some([DataAnnotation::Measure(_)])
        ));

        let bytes = crate::plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, hash) = crate::plan::decode(&bytes).expect("v15 re-derives policy flow");
        assert_eq!(hash, compilation.hash);
        assert_eq!(decoded, compilation.plan);
    }

    #[test]
    fn a_calculation_retains_security_but_drops_accounting_meaning() {
        let source = r#"
type Count = int
  @sensitive(level = internal)
  @measure(id = gen_ai.usage.input_tokens)
graph app : App {
  input in: Count
  output out: int
  operator increment {
    on in { out = data + 1 }
  }
  . -> increment -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the derived value has one honest policy");
        let output = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("boundary output");
        let root = output
            .annotations()
            .entries()
            .get(&SchemaPath::root())
            .expect("the security taint reaches the result");
        assert_eq!(
            root.annotations(),
            [DataAnnotation::Sensitive(SensitivityLevel::Internal)]
        );
        assert!(root.annotations().iter().all(|annotation| {
            !matches!(
                annotation.kind(),
                AnnotationKind::Measure | AnnotationKind::Cost
            )
        }));
    }

    #[test]
    fn a_sensitive_guard_taints_the_selected_output() {
        let source = r#"
type Input = struct {
  allow: bool @sensitive(level = restricted)
  payload: text
}
graph app : App {
  input in: Input
  output out: text
  operator gate {
    input in: Input
    output out: text
    on in where data.allow { out = data.payload }
  }
  . -> gate -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("control dependence is policy dependence");
        let output = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("boundary output");
        assert!(matches!(
            output
                .annotations()
                .entries()
                .get(&SchemaPath::root())
                .map(|set| set.annotations()),
            Some([DataAnnotation::Sensitive(SensitivityLevel::Restricted)])
        ));
    }

    #[test]
    fn an_app_val_keeps_its_policy_inside_a_child_node() {
        let source = r#"
graph app : App {
  input in: text
  output out: text
  val label: text @sensitive(level = confidential) = "classified label"
  operator relay {
    input in: text
    output out: text
    on in { out = label }
  }
  . -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("App vals are lexical policy bindings for child Nodes");
        let output = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("boundary output");
        assert!(
            matches!(
                output
                    .annotations()
                    .entries()
                    .get(&SchemaPath::root())
                    .map(|set| set.annotations()),
                Some([DataAnnotation::Sensitive(SensitivityLevel::Confidential)])
            ),
            "{:?}",
            output.annotations()
        );
    }

    #[test]
    fn incompatible_accounting_producers_refuse_as_source_policy() {
        let source = r#"
type Tokens = int @measure(id = gen_ai.usage.input_tokens)
graph app : App {
  input metered: Tokens
  input plain: int
  output out: int
  operator merge {
    input metered: Tokens
    input plain: int
    output out: int
    on metered { out = data }
    on plain { out = data }
  }
  .metered -> merge.metered
  .plain -> merge.plain
  merge.out -> .out
}
"#;
        let diagnostics = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("one port cannot sometimes mean tokens and sometimes not");
        let diagnostic = diagnostics
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == crate::compiler::check::E_DATA_ANNOTATION)
            .expect("the policy diagnostic");
        assert_eq!(diagnostic.fields()[0].1, "propagation");
        assert!(diagnostic.message().contains("cannot be derived"));
        assert!(diagnostic.fields()[2].1.contains("Measure"));
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code() != crate::compiler::E_INTERNAL_PLAN)
        );
    }

    #[test]
    fn derived_policy_does_not_masquerade_as_a_source_declaration() {
        let source = r#"
type Classified = text @sensitive(level = confidential)
graph app : App {
  input in: Classified
  operator relay { on in { out = data } }
  . -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("the policy reaches the boundary output");
        let raw_output = compilation
            .plan
            .plan()
            .boundary
            .iter()
            .find(|port| port.port.name() == "out");
        assert!(
            raw_output.is_none_or(|port| port.annotations.is_empty()),
            "lowering must not rewrite a derived verdict into source authority"
        );
        let resolved_output = compilation
            .plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == "out")
            .expect("the effective output is total");
        assert!(!resolved_output.annotations().is_empty());

        let bytes = crate::plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, _) = crate::plan::decode(&bytes)
            .expect("decode independently reconstructs the effective verdict");
        assert_eq!(decoded, compilation.plan);
    }

    #[test]
    fn sensitive_policy_executes_while_secret_policy_stays_fail_closed() {
        let source = r#"
type Classified = text @sensitive(level = confidential)
graph app : App {
  input in: Classified
  operator relay { on in { out = data } }
  . -> relay -> .
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("sensitive policy is executable after A3");
        let mut vm = crate::runtime::vm::Vm::boot(&compilation.plan, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        let submission = crate::boundary::InboundSubmission {
            provenance: crate::boundary::InboundEnvelopeProvenance::single(
                node::Path::new(["test"]).expect("valid provenance path"),
            ),
            port: node::Port::new("in").expect("valid port"),
            draft: crate::boundary::Draft {
                data: crate::envelope::Data::materialized(crate::value::Value::Text(String::from(
                    "classified",
                ))),
                ext: crate::envelope::Namespace::new(),
            },
            sequence: crate::boundary::SubmissionSequence::Next,
        };
        vm.inject(&mut journal, submission)
            .expect("the classified envelope is admitted");
        vm.drain(&mut journal)
            .expect("A3 executes a proved sensitive flow");
        assert_eq!(vm.outputs().len(), 1);
        let annotations = vm
            .boundary_annotations(&node::Port::new("out").expect("valid output"))
            .expect("the presentation boundary owns its verdict");
        let value = vm.outputs()[0]
            .envelope
            .data()
            .materialized_value()
            .expect("the output stays materialized");
        let rendered = format!(
            "{:?}",
            crate::data_policy::RedactedValue::new(value, annotations)
        );
        assert_eq!(rendered, "[REDACTED]");
        assert!(!rendered.contains("classified"));
    }

    #[test]
    fn personal_data_stays_fail_closed_until_a_privacy_profile_is_attested() {
        let compilation = compile(
            &["graph app : App { input in: text @personal(identifier = true) }\n"],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("personal-data policy crosses source and categorical validation");
        let mut vm = crate::runtime::vm::Vm::boot(&compilation.plan, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        let error = vm
            .step(&mut journal)
            .expect_err("A4 privacy attestation has not landed yet");
        assert!(matches!(
            error,
            crate::runtime::vm::VmError::UnsupportedDataPolicy { .. }
        ));
        assert!(
            error
                .to_string()
                .contains("personal-data boundary port `in`")
        );
        assert!(
            journal.records().is_empty(),
            "the refusal precedes mutation"
        );
    }

    #[test]
    fn an_unused_annotated_alias_remains_structural_sugar() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let plain = compile(&["graph app : App {}\n"], &registry).expect("the empty App compiles");
        let alias = compile(
            &["type Secret = text @secret\ngraph app : App {}\n"],
            &registry,
        )
        .expect("an unused structural alias compiles");
        assert_eq!(plain.hash, alias.hash);
        assert_eq!(plain.plan, alias.plan);
    }

    #[test]
    fn loop_identity_tracks_policy_but_not_formatting() {
        let source = |fuel: u32, spaced: bool| {
            if spaced {
                format!(
                    "graph app : App + Loop {{\n  input in : int\n  output out : int\n  Loop.fuel = {fuel}\n  on in {{ out = data }}\n}}\n"
                )
            } else {
                format!(
                    "graph app:App+Loop{{input in:int;output out:int;Loop.fuel={fuel};on in{{out=data}}}}"
                )
            }
        };
        let compile_hash = |source: String| {
            compile(
                &[source.as_str()],
                &ContractCatalog::try_m0().expect("the standard catalogue holds"),
            )
            .expect("the categorical Loop compiles")
            .hash
        };
        let compact = compile_hash(source(3, false));
        let formatted = compile_hash(source(3, true));
        let different_fuel = compile_hash(source(4, true));
        assert_eq!(compact, formatted);
        assert_ne!(formatted, different_fuel);
    }

    #[test]
    fn the_mandatory_axes_ship_typed() {
        let compilation = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let spec = compilation.plan.nodes[1].connector().unwrap();
        assert_eq!(
            spec.effect,
            crate::node::Effect::Class(crate::node::EffectClass::Idempotent)
        );
        assert_eq!(spec.behavior, crate::node::Behavior::Deterministic);
        // The typed axes are not repeated in the attribute bag.
        assert!(spec.attribute("effect").is_none());
        assert!(spec.attribute("behavior").is_none());
    }

    #[test]
    fn the_hash_ignores_formatting_and_declaration_order() {
        let original = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let same = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        assert_eq!(original, same);
        // Same program: comments added, attributes spaced differently,
        // operator declared before the connector.
        let reordered = r#"
// a comment that must not enter the identity
graph m0_probe : App {
  operator shout {
    input in: struct { text: text }
    on in where data.text != "" {
      out = data.text
    }
  }

  connector echo : Process {
    mode  =  job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  . -> shout -> echo -> .
}
"#;
        assert_eq!(
            compile(
                &[reordered],
                &ContractCatalog::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original
        );
    }

    #[test]
    fn the_hash_is_sensitive_to_meaning() {
        let original = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let renamed = M0_PROBE.replace("shout", "whisper");
        assert_ne!(
            compile(
                &[renamed.as_str()],
                &ContractCatalog::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original,
            "renaming a node is an identity change - no alpha-equivalence"
        );
        let regexed = M0_PROBE.replace("data.text != \"\"", "data.text != \"x\"");
        assert_ne!(
            compile(
                &[regexed.as_str()],
                &ContractCatalog::try_m0().expect("the standard catalogue holds")
            )
            .unwrap()
            .hash,
            original,
            "a Rastok expression is identity material"
        );
    }

    #[test]
    fn the_semantic_hash_is_pinned_byte_for_byte() {
        // The frozen identity of the M0 probe. If this test fails, the
        // canonical form changed: that is an identity-contract change
        // and must be deliberate, reviewed, and re-blessed.
        // DELIBERATE MOVE 2026-08-10 (D-2026-08-10-7): a compiled
        // tree seals its STATIC TYPE with its root, so a `json`
        // document and its Rastok expression twin are distinct programs — every
        // tree-carrying graph moved once.
        // DELIBERATE MOVE 2026-08-12 (D-2026-08-12-1): contract
        // defaults are injected into the effective HIR before the
        // hash (`args = []`, `max_output = "8MiB"`,
        // `on_timeout = kill` on this probe) — omission and the
        // exact spelling of a default are ONE identity, so every
        // Process-carrying graph moved once.
        // DELIBERATE MOVE 2026-08-12, same day (review round): the
        // former Process error set gained `CaptureFailed` (a pipe READ
        // fault is its own kind).
        // DELIBERATE FIXTURE MIGRATION 2026-08-14 (M1.3 B2): the
        // probe now emits `data.text`, not a one-field record, because
        // the Process `lines` input is text. The identity recipe did
        // not move; this previously ill-typed program did.
        // DELIBERATE MOVE 2026-08-15 (M1.3 B5): complete resolved
        // Connector port rows and their routing policies replace the
        // travelling error set. The Process sheet also materializes
        // `err_codec = lines` before hashing.
        // DELIBERATE MOVE 2026-08-15 (M1.3 B7): jsonl replies are unit
        // emissions, stderr has its resolved framing, and every Process
        // invocation terminates on the explicit `exit` port.
        // DELIBERATE MOVE 2026-08-15 (M1.3 B8): v7 seals exact resolved
        // types and the total effective port table. The former Process.exit
        // and decimal transport bridges are gone; wide integer literals keep
        // their honest int64 type.
        // DELIBERATE FIXTURE MIGRATION 2026-08-15 (M1.3 B9): the first
        // structural inspection now names the operator's closed input type;
        // `variant` can no longer be inspected without `is`/`as` narrowing.
        // DELIBERATE MOVE 2026-08-22 (M1.5 C1): the source symbol
        // `Process` resolves to `rastok/process@1`, and the stable id plus
        // required outbound role are now canonical HIR identity.
        // DELIBERATE MOVE 2026-08-25 (M1.6 H1): the contract's Call exchange
        // lifecycle becomes explicit canonical HIR and categorical v11 plan
        // data. Existing Call causal records do not move.
        // H4.1 moves only this Process package container to v12; the probe
        // uses no newly typed media field, so its semantic hash stays fixed.
        let hash = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        assert_eq!(
            hash.to_string(),
            "2933618d21f03ae7b7835998f1f150aac5de27eb78e8aea9412db7dde76489b0"
        );
    }

    #[test]
    fn boundary_ports_are_identity_material() {
        let bare = "graph g : App {\n  operator o { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  . -> o -> .\n}\n";
        let with_port = "graph g : App {\n  input feed: text\n  operator o { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  . -> o -> .\n}\n";
        let bare_hash = compile(
            &[bare],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let ported = compile(
            &[with_port],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_ne!(
            ported.hash, bare_hash,
            "a declared boundary port is identity"
        );
        assert_eq!(ported.plan.boundary.len(), 3);
        assert!(
            ported
                .plan
                .boundary
                .iter()
                .any(|port| port.port.name() == "feed")
        );
    }

    #[test]
    fn the_plan_and_the_hash_share_one_edge_order() {
        let forward = "graph g : App {\n  operator a { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  . -> a -> b -> .\n}\n";
        let reordered = "graph g : App {\n  operator a { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  b -> .\n  a -> b\n  . -> a\n}\n";
        let one = compile(
            &[forward],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let two = compile(
            &[reordered],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(one.hash, two.hash);
        assert_eq!(one.plan.edges, two.plan.edges);
    }

    #[test]
    fn unordered_collections_ship_canonically() {
        let with_attrs = |attrs: &str| {
            format!(
                "graph g : App {{\n  connector c : Process {{\n{attrs}  }}\n  . -> c -> .\n}}\n"
            )
        };
        let one = compile(
            &[with_attrs("    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let two = compile(
            &[with_attrs("    behavior = deterministic ; effect = idempotent\n    codec = lines ; command = \"cat\"\n").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(one.hash, two.hash);
        assert_eq!(
            one.plan, two.plan,
            "attribute order must not leak into the plan"
        );
        // Map literals: one canonical form for hash, plan, and eval.
        let with_map = |map: &str| {
            format!(
                "graph g : App {{\n  operator o {{\n    input in: struct {{ anchor: text }}\n    on in where data.anchor == \"\" && size({map}) > 0 {{ out = . }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let forward = compile(
            &[with_map("{'a': 1, 'b': 2}").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let reversed = compile(
            &[with_map("{'b': 2, 'a': 1}").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        assert_eq!(forward.hash, reversed.hash);
        assert_eq!(forward.plan, reversed.plan);
    }

    #[test]
    fn inferred_ports_travel_in_the_plan() {
        let source = "graph g : App {\n  input feed: text\n  output sink\n  operator o {\n    on event { result = . }\n  }\n  .feed -> o.event\n  o.result -> .sink\n}\n";
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let ports = &compilation.plan.nodes[0].ports;
        assert_eq!(ports.len(), 2, "only the operator's real ports travel");
        assert!(ports.iter().any(|decl| decl.port.name() == "event"));
        assert!(ports.iter().any(|decl| decl.port.name() == "result"));
        assert!(
            ports.iter().all(|decl| decl.port.name() != "err"),
            "an operator has no hidden runtime-fault producer"
        );
    }

    #[test]
    fn constants_reach_both_the_hash_and_the_plan() {
        let with_val = |n: &str| {
            format!(
                "graph g : App {{\n  operator o {{\n    input in: struct {{ n: int }}\n    val threshold = {n}\n    on in where data.n > threshold {{ out = . }}\n  }}\n  . -> o -> .\n}}\n"
            )
        };
        let five = compile(
            &[with_val("5").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let six = compile(
            &[with_val("6").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        // A local constant is identity material...
        assert_ne!(five.hash, six.hash);
        // ...and the shipped guard carries its value: no free
        // `threshold` binding survives in the plan.
        let guard = five.plan.handlers[0].guard.as_ref().unwrap();
        let rendered = format!("{guard:?}");
        assert!(
            !rendered.contains("threshold"),
            "the constant must be substituted: {rendered}"
        );
        assert!(rendered.contains("Int(5)"));
    }

    #[test]
    fn a_plan_with_errors_is_unrepresentable() {
        let bag = compile(
            &["graph g : App {\n  connector c : Process { }\n  . -> c -> .\n}\n"],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("missing mandatory attributes must refuse");
        assert!(bag.has_errors());
    }

    #[test]
    fn versions_alias_the_hash_through_the_manifest() {
        let hash = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        let mut manifest = Manifest::embedded();
        assert_eq!(manifest.allocate(hash), Some(GraphVersion(1)));
        assert_eq!(manifest.hash_of(GraphVersion(1)), Some(hash));
    }
}
