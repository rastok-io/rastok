//! Canonicalization and lowering: HIR to `SemanticHash` and
//! `ExecutionPlan`.
//!
//! The canonical byte form follows `identity.md` §1: insensitive to
//! formatting, file order, and declaration order — nodes, vals,
//! attributes, vars, ports, and edges are sorted before hashing —
//! sensitive to topology, statement order inside a handler, every Rastok expression
//! as its canonical tree, and `var` defaults. Owner constants
//! hash as the expressions they were written as, before const-eval;
//! unit-global constants have already been folded and substituted at check.
//! spans never enter the bytes. The plan then ships the reduced
//! values.

use super::routing;
use crate::instance;

use crate::compiler::hir::{
    AttrValue, Dispatch as HirDispatch, EdgeTarget, EmitValue, Endpoint, EndpointOwner,
    Graph as HirGraph, Hir, InstanceLifetime, InstancePolicy, LoopSpec, Node as HirNode,
    NodeSemantics, PortDirection, RouteDestination, RouteSelection, Statement as HirStatement,
};
use crate::datatype::{
    BinaryType, DataType, DateUnit, FloatType, IntegerType, IntegerWidth, IntervalUnit,
    TemporalType, TimeUnit, TimeZone,
};
use crate::expr;
use crate::node;
use crate::plan::{
    AttrSetting, ConnectorSpec, DelayPolicy, DispatchStep, Edge, EdgeEnd, EvaluationSite,
    ExecutionPlan, ExtensionSpec, FieldWrite, Handler, LoopSpec as PlanLoopSpec, Node,
    NodeExecution, Owner, OwnerKind, OwnerValue, PlanConstant, SemanticHash, Slot, SymbolTable,
    VarCell,
};
use crate::schema_annotation::{
    AnnotationTable, DataAnnotation, SchemaPathSegment, SensitivityLevel,
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
    writer.routing(hir);
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
    let mut loop_owners: Vec<_> = sorted_graphs(hir)
        .into_iter()
        .filter_map(|graph| {
            graph
                .loop_spec
                .as_ref()
                .map(|spec| (EndpointOwner::Graph(graph.path.clone()), spec))
        })
        .chain(sorted_nodes(hir).into_iter().filter_map(|node| {
            node.loop_spec
                .as_ref()
                .map(|spec| (EndpointOwner::Node(node.path.clone()), spec))
        }))
        .collect();
    if !loop_owners.is_empty() {
        loop_owners.sort_by(|left, right| left.0.cmp(&right.0));
        writer.str("rastok.loop.r2");
        writer.count(loop_owners.len());
        for (owner, spec) in loop_owners {
            writer.endpoint_owner(&owner);
            writer.loop_spec(spec);
        }
    }
    let retry_owners: Vec<_> = sorted_nodes(hir)
        .into_iter()
        .filter_map(|node| node.retry_spec.as_ref().map(|spec| (node, spec)))
        .collect();
    if !retry_owners.is_empty() {
        writer.str("rastok.retry.r1");
        writer.count(retry_owners.len());
        for (node, spec) in retry_owners {
            writer.endpoint_owner(&EndpointOwner::Node(node.path.clone()));
            writer.tree(&spec.attempts.tree);
            writer.count(spec.cases.len());
            for case in &spec.cases {
                writer.str(case);
            }
            writer.delay(spec.delay.as_ref());
            writer.str(spec.entry.name());
            writer.str(spec.terminal.name());
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

/// Boundary ports in canonical order — by (port, direction); the
/// optional pinned type never decides an order.
fn sorted_boundary(hir: &Hir) -> Vec<crate::plan::PortDecl> {
    let mut boundary = hir.app.boundary.clone();
    boundary.sort_by(|a, b| a.key().cmp(&b.key()));
    boundary
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatEdge {
    scope: crate::node::OwnerPath,
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

fn endpoint_scope(endpoint: &crate::compiler::hir::Endpoint) -> crate::node::OwnerPath {
    match &endpoint.owner {
        EndpointOwner::Node(path) => crate::node::OwnerPath::from(path)
            .parent()
            .unwrap_or_else(crate::node::OwnerPath::root),
        EndpointOwner::Graph(path) => path.clone(),
    }
}

fn common_graph_scope(
    left: &crate::compiler::hir::Endpoint,
    right: &crate::compiler::hir::Endpoint,
) -> crate::node::OwnerPath {
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
        let owns_app = graph.extensions.contains(&node::Extension::App);
        if owns_app != graph.path.is_root() {
            return Err(LowerError {
                owner: graph.path.to_string(),
                detail: "the App Extension does not match the checked root Graph",
            });
        }
        if graph
            .extensions
            .iter()
            .any(|extension| !matches!(extension, node::Extension::App | node::Extension::Loop))
            || (graph.extensions.contains(&node::Extension::Loop) && graph.loop_spec.is_none())
            || (!graph.extensions.contains(&node::Extension::Loop) && graph.loop_spec.is_some())
        {
            return Err(LowerError {
                owner: graph.path.to_string(),
                detail: "a Graph Extension reached lowering without its complete categorical specification",
            });
        }
        let unsupported_command =
            |route: &crate::plan::RouteRule, destination: &RouteDestination| {
                let RouteDestination::LoopCommand(command) = destination else {
                    return false;
                };
                if let crate::plan::EndpointOwner::Graph(owner) = &command.owner {
                    return !sorted_graphs(hir)
                        .iter()
                        .any(|graph| graph.path == *owner && graph.loop_spec.is_some());
                }
                command.owner != route.source.owner || !nodes.iter().any(|node|
                matches!(&route.source.owner, crate::plan::EndpointOwner::Node(path) if *path == node.path)
                && matches!(node.semantics, NodeSemantics::Connector { .. })
                && node.loop_spec.is_some())
            };
        if graph.routes.iter().any(|route| match &route.selection {
            RouteSelection::All(destinations) => destinations
                .iter()
                .any(|destination| unsupported_command(route, destination)),
            RouteSelection::Cases(arms) => arms
                .iter()
                .flat_map(|arm| &arm.destinations)
                .any(|destination| unsupported_command(route, destination)),
        }) {
            return Err(LowerError {
                owner: graph.path.to_string(),
                detail: "Loop manager commands require the R7 execution protocol",
            });
        }
    }
    for node in &nodes {
        if !node.extensions.is_empty()
            && !(node.extensions.as_slice() == [node::Extension::Loop] && node.loop_spec.is_some()
                || node.extensions.as_slice() == [node::Extension::Retry]
                    && node.retry_spec.is_some())
        {
            return Err(LowerError {
                owner: node.path.to_string(),
                detail: "the Extension needs a complete, admitted host policy",
            });
        }
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
        boundary_routes: Vec::new(),
        owners: Vec::new(),
        nodes: Vec::new(),
        boundary: sorted_boundary(hir),
        edges: Vec::new(),
        routes: Vec::new(),
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
    let mut checked_owners: Vec<(node::OwnerPath, CheckedOwner<'_>)> = sorted_graphs(hir)
        .into_iter()
        .map(|graph| (graph.path.clone(), CheckedOwner::Graph(graph)))
        .collect();
    checked_owners.extend(
        nodes
            .iter()
            .map(|node| (node::OwnerPath::from(&node.path), CheckedOwner::Node(node))),
    );
    checked_owners.sort_by(|left, right| left.0.cmp(&right.0));

    let mut owner_slots = std::collections::BTreeMap::new();
    let mut node_owners = std::collections::BTreeMap::new();
    for (index, (path, checked)) in checked_owners.into_iter().enumerate() {
        let owner_index = u32::try_from(index).map_err(|_| LowerError {
            owner: path.to_string(),
            detail: "the owner table exceeds its u32 slot domain",
        })?;
        let owner_slot = node::OwnerSlot::new(owner_index);
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
                node::OwnerPath::from(&node.path)
                    .parent()
                    .ok_or_else(|| LowerError {
                        owner: node.path.to_string(),
                        detail: "an executable Node cannot own the root Graph path",
                    })?
            }
        };
        let mut constants = graph_constant_environment(hir, &graph_path);
        let (kind, ports, extensions, instance, vals) = match checked {
            CheckedOwner::Graph(graph) => {
                let mut ports = if graph.path.is_root() {
                    Vec::new()
                } else {
                    graph.boundary.clone()
                };
                ports.sort_by(|left, right| left.key().cmp(&right.key()));
                let mut extensions = graph
                    .loop_spec
                    .as_ref()
                    .map(|spec| {
                        vec![ExtensionSpec::Loop(Box::new(lower_loop_spec(
                            spec, &constants,
                        )))]
                    })
                    .unwrap_or_default();
                if graph.path.is_root() {
                    extensions.insert(0, ExtensionSpec::App);
                }
                (
                    OwnerKind::Graph,
                    ports,
                    extensions,
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
                    if let Some(spec) = &node.loop_spec {
                        vec![ExtensionSpec::Loop(Box::new(lower_loop_spec(
                            spec, &constants,
                        )))]
                    } else if let Some(spec) = &node.retry_spec {
                        vec![ExtensionSpec::Retry(Box::new(crate::plan::RetrySpec {
                            attempts: PlanConstant {
                                expr: substitute_tree(&spec.attempts.tree, &constants),
                                resolved: spec.attempts.value.clone(),
                            },
                            cases: spec.cases.clone(),
                            delay: lower_delay(spec.delay.as_ref(), &constants),
                            entry: spec.entry.clone(),
                            terminal: spec.terminal.clone(),
                        }))]
                    } else {
                        Vec::new()
                    },
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
            extensions,
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
            node::OwnerPath::new(hir_node.path.segments().iter().cloned()).map_err(|_| {
                LowerError {
                    owner: hir_node.path.to_string(),
                    detail: "a checked Node path did not form an owner path",
                }
            })?;
        let graph_path = owner_path.parent().unwrap_or_else(node::OwnerPath::root);
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
            .filter(|attribute| attribute.name != "behavior")
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
            execution: match &hir_node.semantics {
                NodeSemantics::Connector {
                    contract,
                    roles,
                    exchange,
                    ..
                } => NodeExecution::Connector(ConnectorSpec {
                    contract: contract.clone(),
                    roles: *roles,
                    exchange: *exchange,
                    idempotent: hir_node.attributes().iter().find_map(|attribute| {
                        if attribute.name != "idempotent" {
                            return None;
                        }
                        let AttrValue::Const(constant) = &attribute.value else {
                            return None;
                        };
                        Some(PlanConstant {
                            expr: substitute_tree(&constant.tree, &constants),
                            resolved: constant.value.clone(),
                        })
                    }),
                    behavior: typed_behavior(hir_node),
                    attributes,
                }),
                NodeSemantics::Operator => NodeExecution::Operator,
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
        let graph_path = node::OwnerPath::from(&hir_node.path)
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
                owner: slot,
                activation: handler.activation.clone(),
                guard: handler.admission_guard.as_ref().map(&subst),
                dispatch: lower_dispatch(&handler.dispatch, &subst),
            });
        }
    }
    plan.handlers.sort_by(|left, right| {
        left.owner.cmp(&right.owner).then_with(|| {
            left.activation
                .canonical_key()
                .cmp(&right.activation.canonical_key())
        })
    });
    plan.routes = routing::flatten(&hir.app).map_err(|error| LowerError {
        owner: endpoint_text(&error.source),
        detail: "Graph boundary composition contradicts checked routing dispositions",
    })?;
    if plan
        .owners
        .iter()
        .any(|owner| owner.kind == OwnerKind::Graph && !owner.path.is_root())
        || hir.app.loop_spec.is_some()
        || plan.routes.iter().any(|rule| match &rule.selection {
            RouteSelection::All(set) => set
                .iter()
                .any(|destination| matches!(destination, RouteDestination::InstanceStop(_))),
            RouteSelection::Cases(arms) => arms
                .iter()
                .flat_map(|arm| &arm.destinations)
                .any(|destination| matches!(destination, RouteDestination::InstanceStop(_))),
        })
    {
        plan.boundary_routes = routing::inventory(&hir.app);
    }
    for (_, edge) in sorted_edges(hir) {
        if let Some(rule) = plan.routes.iter().find(|rule| rule.source == edge.from)
            && !routing::selects(rule, &edge.to)
        {
            continue;
        }
        let endpoint = |end: &crate::compiler::hir::Endpoint| -> Result<EdgeEnd, LowerError> {
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
            Ok(EdgeEnd {
                node,
                port: end.port.clone(),
            })
        };
        let scope = *owner_slots.get(&edge.scope).ok_or_else(|| LowerError {
            owner: edge.scope.to_string(),
            detail: "a lowered edge lost its lexical Graph scope",
        })?;
        plan.edges.push(Edge {
            owner: scope,
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
        entry: spec.entry.clone(),
        terminals: spec.terminals.clone(),
        expired: spec.expired.clone(),
        fuel: spec.fuel.as_ref().map(constant),
        delay: lower_delay(spec.delay.as_ref(), constants),
        proof: spec.proof.clone(),
    }
}

fn lower_delay(
    delay: Option<&crate::compiler::hir::DelayPolicy>,
    constants: &std::collections::BTreeMap<String, crate::value::Value>,
) -> Option<DelayPolicy> {
    let constant = |value: &crate::compiler::hir::Const| PlanConstant {
        expr: substitute_tree(&value.tree, constants),
        resolved: value.value.clone(),
    };
    delay.map(|delay| match delay {
        crate::compiler::hir::DelayPolicy::Fixed { after } => DelayPolicy::Fixed {
            after: constant(after),
        },
        crate::compiler::hir::DelayPolicy::Exponential { first, ceiling } => {
            DelayPolicy::Exponential {
                first: constant(first),
                ceiling: constant(ceiling),
            }
        }
    })
}

fn lower_statement(
    statement: &HirStatement,
    subst: &impl Fn(&expr::Tree) -> expr::Tree,
) -> DispatchStep {
    match statement {
        HirStatement::Let {
            name,
            value,
            annotations,
        } => DispatchStep::Let {
            name: name.clone(),
            value: subst(value),
            annotations: annotations.clone(),
        },
        HirStatement::SetVar { name, value } => DispatchStep::SetVar {
            name: name.clone(),
            value: subst(value),
        },
        HirStatement::Emit { port, value } => DispatchStep::Emit {
            port: port.clone(),
            value: substitute_emission(value, subst),
        },
        HirStatement::Stop => DispatchStep::Stop,
        HirStatement::MultiEmit { ports, values } => DispatchStep::MultiEmit {
            ports: ports.clone(),
            values: match values {
                crate::plan::MultiEmitValues::Broadcast(value) => {
                    crate::plan::MultiEmitValues::Broadcast(substitute_emission(value, subst))
                }
                crate::plan::MultiEmitValues::Tuple(values) => crate::plan::MultiEmitValues::Tuple(
                    values
                        .iter()
                        .map(|value| substitute_emission(value, subst))
                        .collect(),
                ),
            },
        },
        HirStatement::LoopCommand(command) => DispatchStep::LoopCommand(match command {
            crate::plan::LoopCommand::Next(value) => {
                crate::plan::LoopCommand::Next(substitute_emission(value, subst))
            }
            crate::plan::LoopCommand::Again => crate::plan::LoopCommand::Again,
            crate::plan::LoopCommand::Done => crate::plan::LoopCommand::Done,
        }),
    }
}

fn v15_handler_parts(
    handler: &crate::compiler::hir::Handler,
) -> Option<(Option<&expr::Tree>, &[HirStatement])> {
    handler.v15_linear_parts()
}

fn lower_instance(
    policy: Option<&InstancePolicy>,
    constants: &std::collections::BTreeMap<String, crate::value::Value>,
    owner: &str,
) -> Result<instance::InstanceSpec, LowerError> {
    let Some(policy) = policy else {
        return Ok(instance::InstanceSpec {
            key: None,
            lifetime: instance::Lifetime::Persistent,
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
        InstanceLifetime::Persistent => instance::Lifetime::Persistent,
        InstanceLifetime::Idle(value) => instance::Lifetime::Idle(duration(value)?),
        InstanceLifetime::Fixed(value) => instance::Lifetime::Fixed(duration(value)?),
        InstanceLifetime::SinglePass => instance::Lifetime::SinglePass,
        InstanceLifetime::Tumbling(value) => instance::Lifetime::Tumbling(duration(value)?),
    };
    Ok(instance::InstanceSpec {
        key: policy
            .key
            .as_ref()
            .map(|key| substitute_tree(key, constants)),
        lifetime,
        closing: match &policy.closing {
            None => None,
            Some(closing) => Some(instance::Closing {
                drain_for: duration(&closing.drain_for)?,
                settle_for: duration(&closing.settle_for)?,
            }),
        },
    })
}

/// A checked source construct cannot be packaged for the selected execution
/// model. Refusal precedes any externally visible effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    /// Source owner whose checked construct cannot ship yet.
    pub owner: String,
    /// Named missing transport/runtime gate.
    pub detail: &'static str,
}

fn lower_dispatch(
    dispatch: &rastok_core::plan::Dispatch,
    subst: &impl Fn(&expr::Tree) -> expr::Tree,
) -> rastok_core::plan::Dispatch {
    use rastok_core::plan::{ClosedArm, Dispatch, OpenArm};
    let steps = |steps: &[HirStatement]| {
        steps
            .iter()
            .map(|step| lower_statement(step, subst))
            .collect()
    };
    match dispatch {
        Dispatch::Linear(body) => Dispatch::Linear(steps(body)),
        Dispatch::ClosedMatch {
            prefix,
            scrutinee,
            arms,
        } => Dispatch::ClosedMatch {
            prefix: steps(prefix),
            scrutinee: subst(scrutinee),
            arms: arms
                .iter()
                .map(|arm| ClosedArm {
                    cases: arm.cases.clone(),
                    dispatch: Box::new(lower_dispatch(&arm.dispatch, subst)),
                })
                .collect(),
        },
        Dispatch::OpenMatch {
            prefix,
            arms,
            fallback,
        } => Dispatch::OpenMatch {
            prefix: steps(prefix),
            arms: arms
                .iter()
                .map(|arm| OpenArm {
                    predicate: subst(&arm.predicate),
                    dispatch: Box::new(lower_dispatch(&arm.dispatch, subst)),
                })
                .collect(),
            fallback: Box::new(lower_dispatch(fallback, subst)),
        },
    }
}

fn substitute_emission(value: &EmitValue, subst: &impl Fn(&expr::Tree) -> expr::Tree) -> EmitValue {
    let writes = |fields: &[FieldWrite]| {
        fields
            .iter()
            .map(|write| FieldWrite {
                target: write.target.clone(),
                value: subst(&write.value),
            })
            .collect()
    };
    match value {
        EmitValue::Identity => EmitValue::Identity,
        EmitValue::Patch(fields) => EmitValue::Patch(writes(fields)),
        EmitValue::FreshData(fields) => EmitValue::FreshData(writes(fields)),
        EmitValue::Data(tree) => EmitValue::Data(subst(tree)),
        EmitValue::Nothing => EmitValue::Nothing,
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
        root: super::constants::substitute(&tree.root, &|expression| {
            let expr::Expr::Path(segments) = expression else {
                return None;
            };
            let mut result = expr::Expr::Literal(consts.get(segments.first()?)?.clone());
            for segment in &segments[1..] {
                result = expr::Expr::Index {
                    base: Box::new(result),
                    index: Box::new(expr::Expr::Literal(crate::value::Value::Text(
                        segment.clone(),
                    ))),
                };
            }
            Some(result)
        }),
        ty: tree.ty.clone(),
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
    path: &crate::node::OwnerPath,
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
    path: &crate::node::OwnerPath,
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
    path: &crate::node::OwnerPath,
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
    /// The ordinary edge stream already seals unconditional spatial fan-out.
    /// Only case selection and manager commands add material here. Universal
    /// destinations are merged into each case so declaration boundaries and
    /// grouping cannot affect identity.
    fn routing(&mut self, hir: &Hir) {
        let mut sources = std::collections::BTreeMap::<_, Vec<_>>::new();
        for graph in sorted_graphs(hir) {
            for route in &graph.routes {
                sources
                    .entry(&route.source)
                    .or_default()
                    .push(&route.selection);
            }
        }
        sources.retain(|_, selections| {
            selections.iter().any(|selection| match selection {
                RouteSelection::Cases(_) => true,
                RouteSelection::All(destinations) => destinations.iter().any(|destination| {
                    matches!(
                        destination,
                        RouteDestination::LoopCommand(_) | RouteDestination::InstanceStop(_)
                    )
                }),
            })
        });
        if sources.is_empty() {
            return;
        }
        self.str("rastok.routing.r1");
        self.count(sources.len());
        for (source, selections) in sources {
            self.endpoint(source);
            let mut universal = std::collections::BTreeSet::new();
            let mut cases = std::collections::BTreeMap::<_, std::collections::BTreeSet<_>>::new();
            for selection in selections {
                match selection {
                    RouteSelection::All(destinations) => universal.extend(destinations),
                    RouteSelection::Cases(arms) => {
                        for arm in arms {
                            for case in &arm.cases {
                                cases.entry(case).or_default().extend(&arm.destinations);
                            }
                        }
                    }
                }
            }
            if cases.is_empty() {
                self.tag(1);
                self.route_destinations(&universal);
            } else {
                self.tag(2);
                self.count(cases.len());
                for (case, mut destinations) in cases {
                    self.str(case);
                    destinations.extend(&universal);
                    self.route_destinations(&destinations);
                }
            }
        }
    }

    fn route_destinations(&mut self, destinations: &std::collections::BTreeSet<&RouteDestination>) {
        self.count(destinations.len());
        for destination in destinations {
            match destination {
                RouteDestination::Port(endpoint) => {
                    self.tag(1);
                    self.endpoint(endpoint);
                }
                RouteDestination::Discard => self.tag(2),
                RouteDestination::InstanceStop(target) => {
                    self.tag(4);
                    self.endpoint_owner(&EndpointOwner::Graph(target.scope.clone()));
                    self.endpoint_owner(&target.owner);
                }
                RouteDestination::LoopCommand(command) => {
                    self.tag(3);
                    self.endpoint_owner(&command.owner);
                    self.tag(command.kind.tag());
                }
            }
        }
    }

    fn endpoint_owner(&mut self, owner: &EndpointOwner) {
        match owner {
            EndpointOwner::Node(path) => {
                self.tag(1);
                self.str(&path.to_string());
            }
            EndpointOwner::Graph(path) => {
                self.tag(2);
                self.str(&path.to_string());
            }
        }
    }

    fn endpoint(&mut self, endpoint: &Endpoint) {
        self.endpoint_owner(&endpoint.owner);
        self.str(endpoint.port.name());
    }

    fn loop_spec(&mut self, spec: &LoopSpec) {
        self.str(spec.entry.name());
        self.count(spec.terminals.len());
        for terminal in &spec.terminals {
            self.str(terminal.name());
        }
        match &spec.expired {
            Some(expired) => {
                self.tag(1);
                self.str(expired.port.name());
                self.ty(&expired.ty);
            }
            None => self.tag(0),
        }
        match &spec.fuel {
            None => self.tag(0),
            Some(fuel) => {
                self.tag(1);
                self.tree(&fuel.tree);
            }
        }
        self.delay(spec.delay.as_ref());
        match &spec.proof {
            None => self.tag(0),
            Some(proof) => {
                self.tag(1);
                self.tree(&proof.measure);
                match &proof.invariant {
                    None => self.tag(0),
                    Some(invariant) => {
                        self.tag(1);
                        self.tree(invariant);
                    }
                }
            }
        }
    }

    fn delay(&mut self, delay: Option<&crate::compiler::hir::DelayPolicy>) {
        match delay {
            None => self.tag(0),
            Some(crate::compiler::hir::DelayPolicy::Fixed { after }) => {
                self.tag(1);
                self.tree(&after.tree);
            }
            Some(crate::compiler::hir::DelayPolicy::Exponential { first, ceiling }) => {
                self.tag(2);
                self.tree(&first.tree);
                self.tree(&ceiling.tree);
            }
        }
    }

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

    fn direction(&mut self, direction: PortDirection) {
        self.tag(match direction {
            PortDirection::Input => 1,
            PortDirection::Output => 2,
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
                    DataAnnotation::Declared(application) => {
                        self.tag(6);
                        self.tag(match application.binding() {
                            crate::schema_annotation::AnnotationBinding::Metadata => 1,
                            crate::schema_annotation::AnnotationBinding::Measure => 2,
                        });
                        self.str(application.definition());
                        self.ty(application.argument_type());
                        self.value(application.arguments());
                    }
                    DataAnnotation::Sensitive(level) => {
                        self.tag(1);
                        self.tag(match level {
                            SensitivityLevel::Internal => 1,
                            SensitivityLevel::Confidential => 2,
                            SensitivityLevel::Restricted => 3,
                        });
                    }
                    DataAnnotation::Secret => self.tag(2),
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
        match &node.semantics {
            crate::compiler::hir::NodeSemantics::Operator => self.tag(0),
            crate::compiler::hir::NodeSemantics::Connector {
                contract,
                roles,
                exchange,
                ..
            } => {
                self.tag(1);
                self.str(contract.as_str());
                self.tag(roles.tag());
                self.tag(exchange.tag());
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
            self.tag(match decl.direction {
                PortDirection::Input => 1,
                PortDirection::Output => 2,
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
        if let Some((guard, body)) = v15_handler_parts(handler) {
            match guard {
                None => self.tag(0),
                Some(guard) => {
                    self.tag(1);
                    self.tree(guard);
                }
            }
            self.count(body.len());
            for statement in body {
                self.statement(statement);
            }
            return;
        }
        // R3-only semantic form. It cannot ship in v15, but its checked HIR
        // still has a deterministic identity for equivalence and sabotage
        // tests before the single R4 package bump.
        self.str("rastok.dispatch.r1");
        match &handler.admission_guard {
            None => self.tag(0),
            Some(guard) => {
                self.tag(1);
                self.tree(guard);
            }
        }
        self.dispatch(&handler.dispatch);
    }

    fn dispatch(&mut self, dispatch: &HirDispatch) {
        match dispatch {
            HirDispatch::Linear(statements) => {
                self.tag(1);
                self.count(statements.len());
                for statement in statements {
                    self.statement(statement);
                }
            }
            HirDispatch::ClosedMatch {
                prefix,
                scrutinee,
                arms,
            } => {
                self.tag(2);
                self.count(prefix.len());
                for statement in prefix {
                    self.statement(statement);
                }
                self.tree(scrutinee);
                self.count(arms.len());
                for arm in arms {
                    self.count(arm.cases.len());
                    for case in &arm.cases {
                        self.str(case);
                    }
                    self.dispatch(&arm.dispatch);
                }
            }
            HirDispatch::OpenMatch {
                prefix,
                arms,
                fallback,
            } => {
                self.tag(3);
                self.count(prefix.len());
                for statement in prefix {
                    self.statement(statement);
                }
                self.count(arms.len());
                for arm in arms {
                    self.tree(&arm.predicate);
                    self.dispatch(&arm.dispatch);
                }
                self.dispatch(fallback);
            }
        }
    }

    fn statement(&mut self, statement: &HirStatement) {
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
            HirStatement::MultiEmit { ports, values } => {
                self.tag(5);
                self.count(ports.len());
                for port in ports {
                    self.str(port.name());
                }
                match values {
                    crate::compiler::hir::MultiEmitValues::Broadcast(value) => {
                        self.tag(1);
                        self.emission(value);
                    }
                    crate::compiler::hir::MultiEmitValues::Tuple(values) => {
                        self.tag(2);
                        self.count(values.len());
                        for value in values {
                            self.emission(value);
                        }
                    }
                }
            }
            HirStatement::LoopCommand(command) => {
                self.tag(6);
                self.tag(command.kind().tag());
                if let crate::compiler::hir::LoopCommand::Next(value) = command {
                    self.emission(value);
                }
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
                    .unwrap_or_else(|| unreachable!("the checker assigned the index"));
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
                    expr::Function::Has => 7,
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
                    crate::canonical_cbor::encode(&crate::canonical::value(key))
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
    use crate::plan::{AttrSetting, DispatchStep, EmitValue, GraphVersion, Manifest};
    use crate::schema_annotation::{
        AnnotationKind, AnnotationSet, AnnotationTable, DataAnnotation, SchemaPath,
        SensitivityLevel,
    };

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
  }

  operator shout {
    input in: struct { text: text }
    on in where data.text != "" {
      out = data.text
    }
  }

  in -> shout -> echo -> out
  echo.err -> nothing
  echo.exit -> nothing
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
        assert_eq!(spec.roles, crate::connector::ConnectorDirection::Outbound);
        assert_eq!(
            spec.attribute("command"),
            Some(&AttrSetting::Value(crate::value::Value::Text(
                String::from("cat")
            )))
        );
        assert_eq!(plan.handlers.len(), 1);
        let handler = &plan.handlers[0];
        assert_eq!(handler.owner.index(), 0);
        assert_eq!(
            handler.activation,
            node::HandlerActivation::Port(node::Port::new("in").unwrap())
        );
        assert!(matches!(
            handler.dispatch.steps().next().unwrap(),
            DispatchStep::Emit {
                value: EmitValue::Data(_),
                ..
            }
        ));
        assert_eq!(plan.edges.len(), 5);
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
        let source = "type Classified = text @sensitive(level = restricted)\ngraph app : App { input in: Classified }\n";
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
        assert_eq!(vm.step(&mut journal), Ok(false));
        assert!(journal.records().is_empty());
    }

    #[test]
    fn a_forged_secret_policy_stays_fail_closed_before_mutation() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        let source_id = SourceId::new(0);
        let source = "graph app : App { input in: text @sensitive(level = restricted) }\n";
        let mut bag = Bag::new();
        let tree = crate::compiler::parser::parse(source_id, source, &registry, &mut bag);
        let unit = crate::compiler::check::Unit {
            files: vec![(source_id, tree)],
        };
        let mut hir = crate::compiler::check::check(&unit, &registry, &mut bag, None)
            .expect("the non-secret source checks");
        hir.app.boundary[0].annotations = AnnotationTable::new(vec![(
            SchemaPath::root(),
            AnnotationSet::new(vec![DataAnnotation::Secret])
                .expect("one secret annotation is canonical"),
        )])
        .expect("the forged policy table is structurally valid");
        let hash = super::semantic_hash(&hir);
        let (raw, _) = super::lower(&hir).expect("the forged HIR lowers for validation defense");
        let plan = crate::plan::ValidatedExecutionPlan::try_from_lowered(raw)
            .expect("categorical validation retains the defensive secret policy");
        let bytes = crate::plan::encode(&plan, &hash);
        let (decoded, _) = crate::plan::decode(&bytes).expect("the forged package decodes");
        let mut vm = crate::runtime::vm::Vm::boot(&decoded, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        assert!(matches!(
            vm.step(&mut journal),
            Err(crate::runtime::vm::VmFault::UnsupportedDataPolicy { .. })
        ));
        assert!(journal.records().is_empty());
    }

    #[test]
    fn v15_transports_val_var_and_let_annotations_at_their_own_schema_sites() {
        let source = r#"
graph app : App {
  input in: text
  val label: text @sensitive(level = internal) = "demo"
  operator o {
    var credential: text @sensitive(level = restricted) = "placeholder"
    on in {
      val tokens: int @measure(id = output_tokens, unit = "token") = 1
      out = .
    }
  }
  in -> o -> out
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
            compilation.plan.plan().handlers[0].dispatch.steps().next().unwrap(),
            DispatchStep::Let { annotations, .. } if !annotations.is_empty()
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
                "graph app : App {{\n  operator o {{\n    input in: text{annotation}\n    on in {{ out = data }}\n  }}\n  in -> o -> out\n}}\n"
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
  tokens: int @measure(id = input_tokens, unit = "token")
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
  in -> relay -> project -> out
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
            Some([DataAnnotation::Declared(application)])
                if application.binding() == crate::schema_annotation::AnnotationBinding::Measure
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
  @measure(id = input_tokens, unit = "token")
graph app : App {
  input in: Count
  output out: int
  operator increment {
    on in { out = data + 1 }
  }
  in -> increment -> out
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
        assert!(
            root.annotations()
                .iter()
                .all(|annotation| { !matches!(annotation.kind(), AnnotationKind::Measure) })
        );
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
  in -> gate -> out
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
  in -> relay -> out
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
type Tokens = int @measure(id = input_tokens, unit = "token")
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
  metered -> merge.metered
  plain -> merge.plain
  merge.out -> out
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
  in -> relay -> out
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
    fn sensitive_policy_executes_and_redacts() {
        let source = r#"
type Classified = text @sensitive(level = confidential)
graph app : App {
  input in: Classified
  operator relay { on in { out = data } }
  in -> relay -> out
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("sensitive policy is executable after A3");
        let mut vm = crate::runtime::vm::Vm::boot(&compilation.plan, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        let submission = crate::boundary::IngressProposal {
            provenance: crate::boundary::IngressProvenance::single(
                node::Path::new(["test"]).expect("valid provenance path"),
            ),
            draft: crate::boundary::EnvelopeDraft {
                port: node::Port::new("in").expect("valid port"),
                data: crate::envelope::Data::materialized(crate::value::Value::Text(String::from(
                    "classified",
                ))),
                ext: crate::envelope::Namespace::new(),
            },
            sequence: crate::boundary::IngressSequence::Next,
            session: None,
        };
        vm.inject(&mut journal, submission)
            .expect("the classified envelope is admitted");
        vm.run_ready(&mut journal)
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
            .expect_err("personal data requires an A4 privacy attestation");
        assert!(matches!(
            error,
            crate::runtime::vm::VmFault::UnsupportedDataPolicy { .. }
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

        struct NoSecrets;
        impl crate::deployment::SecretProvider for NoSecrets {
            fn resolve(
                &self,
                binding: &crate::deployment::SecretBinding,
            ) -> Result<crate::deployment::SecretMaterial, crate::deployment::SecretProviderError>
            {
                Err(crate::deployment::SecretProviderError::NotFound(
                    binding.reference().clone(),
                ))
            }
        }
        let policy = |value| {
            crate::deployment::PolicyName::new(value)
                .unwrap_or_else(|error| panic!("policy ref: {error}"))
        };
        let privacy = crate::deployment::PrivacyDeclaration::new(
            crate::deployment::PrivacyDeclarationName::new("privacy.example/support@1")
                .expect("stable privacy profile"),
            policy("ropa/support"),
            vec![policy("purpose/support")],
            vec![policy("basis/contract")],
            None,
            None,
            crate::deployment::RecipientPolicy::None,
            crate::deployment::TransferPolicy::None,
            crate::deployment::PrivacyRetention::RunScoped,
        )
        .expect("complete ordinary-personal-data profile");
        let profile = crate::deployment::DeploymentProfile::new(
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
            Some(privacy),
        )
        .expect("deployment profile");
        let boot = crate::deployment::BootBindings::resolve(profile, &NoSecrets)
            .expect("a profile without secrets resolves");

        let mut attested =
            crate::runtime::vm::Vm::boot_with_deployment(&compilation.plan, GraphVersion(1), &boot);
        let mut attested_journal = crate::journal::MemJournal::new_with_deployment(
            GraphVersion(1),
            boot.attestation().clone(),
        );
        assert_eq!(
            crate::journal::Journal::privacy_retention(&attested_journal),
            Some(crate::deployment::PrivacyRetention::RunScoped)
        );
        let submission = crate::boundary::IngressProposal {
            provenance: crate::boundary::IngressProvenance::single(
                node::Path::new(["test"]).expect("valid provenance path"),
            ),
            draft: crate::boundary::EnvelopeDraft {
                port: node::Port::new("in").expect("valid port"),
                data: crate::envelope::Data::materialized(crate::value::Value::Text(String::from(
                    "subject-42",
                ))),
                ext: crate::envelope::Namespace::new(),
            },
            sequence: crate::boundary::IngressSequence::Next,
            session: None,
        };
        attested
            .inject(&mut attested_journal, submission)
            .expect("the attested run-scoped journal admits personal data");
        assert_eq!(attested_journal.records().len(), 1);

        let mut mismatched =
            crate::runtime::vm::Vm::boot_with_deployment(&compilation.plan, GraphVersion(1), &boot);
        let mut unattested_journal = crate::journal::MemJournal::new(GraphVersion(1));
        let error = mismatched
            .step(&mut unattested_journal)
            .expect_err("the run header is an execution precondition");
        assert_eq!(
            error,
            crate::runtime::vm::VmFault::DeploymentAttestationMismatch
        );
        assert!(unattested_journal.records().is_empty());

        let regulated = compile(
            &["graph app : App { input in: text @personal(special = true, criminal = true) }\n"],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("Article 9 and 10 flags compile together");
        let mut missing_conditions =
            crate::runtime::vm::Vm::boot_with_deployment(&regulated.plan, GraphVersion(1), &boot);
        let mut missing_conditions_journal = crate::journal::MemJournal::new_with_deployment(
            GraphVersion(1),
            boot.attestation().clone(),
        );
        assert!(matches!(
            missing_conditions.step(&mut missing_conditions_journal),
            Err(crate::runtime::vm::VmFault::UnsupportedDataPolicy { .. })
        ));
        assert!(missing_conditions_journal.records().is_empty());

        let regulated_privacy = crate::deployment::PrivacyDeclaration::new(
            crate::deployment::PrivacyDeclarationName::new("privacy.example/regulated@1")
                .expect("stable privacy profile"),
            policy("ropa/regulated"),
            vec![policy("purpose/regulated-support")],
            vec![policy("basis/legal-obligation")],
            Some(policy("article9/substantial-public-interest")),
            Some(policy("article10/official-authority")),
            crate::deployment::RecipientPolicy::None,
            crate::deployment::TransferPolicy::None,
            crate::deployment::PrivacyRetention::RunScoped,
        )
        .expect("both regulated regimes are attested");
        let regulated_profile = crate::deployment::DeploymentProfile::new(
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
            Some(regulated_privacy),
        )
        .expect("regulated deployment profile");
        let regulated_boot =
            crate::deployment::BootBindings::resolve(regulated_profile, &NoSecrets)
                .expect("a profile without secrets resolves");
        let mut admitted = crate::runtime::vm::Vm::boot_with_deployment(
            &regulated.plan,
            GraphVersion(1),
            &regulated_boot,
        );
        let mut admitted_journal = crate::journal::MemJournal::new_with_deployment(
            GraphVersion(1),
            regulated_boot.attestation().clone(),
        );
        assert_eq!(admitted.step(&mut admitted_journal), Ok(false));
        assert!(admitted_journal.records().is_empty());

        let disclosed = compile(
            &[r#"
graph app : App {
  input in: text @personal
  connector model : HttpClient {
    url = "https://example.test"
    method = "POST"
    timeout = duration("1s")
    codec = text
    idempotent = true
    behavior = deterministic
  }
  in -> model -> out
  model.exit -> nothing
}
"#],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a personal-data disclosure compiles into policy");
        let mut undeclared_recipient =
            crate::runtime::vm::Vm::boot_with_deployment(&disclosed.plan, GraphVersion(1), &boot);
        let mut undeclared_recipient_journal = crate::journal::MemJournal::new_with_deployment(
            GraphVersion(1),
            boot.attestation().clone(),
        );
        assert!(matches!(
            undeclared_recipient.step(&mut undeclared_recipient_journal),
            Err(crate::runtime::vm::VmFault::UnsupportedDataPolicy { .. })
        ));

        let recipient_privacy = crate::deployment::PrivacyDeclaration::new(
            crate::deployment::PrivacyDeclarationName::new("privacy.example/disclosure@1")
                .expect("stable privacy profile"),
            policy("ropa/disclosure"),
            vec![policy("purpose/model-service")],
            vec![policy("basis/contract")],
            None,
            None,
            crate::deployment::RecipientPolicy::Classes(vec![policy("recipient/model-provider")]),
            crate::deployment::TransferPolicy::None,
            crate::deployment::PrivacyRetention::RunScoped,
        )
        .expect("the external recipient is explicit");
        let recipient_boot = crate::deployment::BootBindings::resolve(
            crate::deployment::DeploymentProfile::new(
                std::collections::BTreeMap::new(),
                std::collections::BTreeMap::new(),
                Some(recipient_privacy),
            )
            .expect("recipient deployment profile"),
            &NoSecrets,
        )
        .expect("a profile without secrets resolves");
        let mut disclosed_vm = crate::runtime::vm::Vm::boot_with_deployment(
            &disclosed.plan,
            GraphVersion(1),
            &recipient_boot,
        );
        let mut disclosed_journal = crate::journal::MemJournal::new_with_deployment(
            GraphVersion(1),
            recipient_boot.attestation().clone(),
        );
        assert_eq!(disclosed_vm.step(&mut disclosed_journal), Ok(false));
    }

    #[test]
    fn boot_references_cross_v15_without_sealing_physical_values() {
        let source = r#"
graph app : App {
  connector client : HttpClient {
    url = env("SERVICE_URL")
    auth_bearer = secret("providers/openai/token")
    method = "POST"
    timeout = duration("2s")
    codec = json
    idempotent = true
    behavior = deterministic
  }
  client.out -> nothing
  client.exit -> nothing
}
"#;
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("declared boot references compile");
        let spec = compilation.plan.plan().nodes[0]
            .connector()
            .expect("HttpClient connector");
        for name in ["auth_bearer", "url"] {
            assert!(matches!(
                spec.attribute(name),
                Some(AttrSetting::Expr {
                    site: crate::plan::EvaluationSite::Boot,
                    expr,
                }) if crate::expr::boot_reference(&expr.root)
                    .expect("canonical boot path")
                    .is_some()
            ));
        }
        let bytes = crate::plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, hash) = crate::plan::decode(&bytes).expect("v15 reads boot references");
        assert_eq!(hash, compilation.hash);
        assert_eq!(decoded, compilation.plan);
        let rendered = format!("{decoded:?}");
        assert!(rendered.contains("providers/openai/token"));
        assert!(!rendered.contains("physical-secret"));

        let mut unbound = crate::runtime::vm::Vm::boot(&decoded, GraphVersion(1));
        let mut journal = crate::journal::MemJournal::new(GraphVersion(1));
        assert!(matches!(
            unbound.step(&mut journal),
            Err(crate::runtime::vm::VmFault::MissingDeploymentBinding { .. })
        ));
        assert!(journal.records().is_empty());
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
                    "graph app : App {{\n  input in : int\n  output out : int\n  operator looped : Loop {{ input in : int; output out : int; output err : union {{ LoopExpired {{ last_state : int }} }}; Loop.fuel = {fuel}; on in {{ out = data }} }}\n  in -> looped -> out\n  looped.err -> nothing\n}}\n"
                )
            } else {
                format!(
                    "graph app:App{{input in:int;output out:int;operator looped:Loop{{input in:int;output out:int;output err:union{{LoopExpired{{last_state:int}}}};Loop.fuel={fuel};on in{{out=data}}}};in->looped->out;looped.err->nothing}}"
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
    fn the_call_guarantee_ships_with_its_original_plan() {
        let compilation = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let spec = compilation.plan.nodes[1].connector().unwrap();
        assert!(spec.is_idempotent());
        assert_eq!(
            spec.attribute("idempotent"),
            Some(&AttrSetting::Value(crate::value::Value::Bool(true)))
        );
        assert_eq!(spec.behavior, crate::node::Behavior::Deterministic);
        // There is no global effect attribute or duplicated behavior word.
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
    idempotent = true
    behavior = deterministic
  }

  in -> shout -> echo -> out
  echo.exit -> nothing
  echo.err -> nothing
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
        // The Process Contract/idempotence cutover is inventoried in
        // docs/architecture/retry-c4-qualification.md; its causal trace stays fixed.
        let hash = compile(
            &[M0_PROBE],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap()
        .hash;
        assert_eq!(
            hash.to_string(),
            "6a49eb768c73af74cff64dc40bd08009e711fc1456c2b92e9e5f4f71e2508433"
        );
    }

    #[test]
    fn boundary_ports_are_identity_material() {
        let bare = "graph g : App {\n  operator o { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  in -> o -> out\n}\n";
        let with_port = "graph g : App {\n  input feed: text\n  operator o { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  in -> o -> out\n}\n";
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
        let forward = "graph g : App {\n  operator a { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  in -> a -> b -> out\n}\n";
        let reordered = "graph g : App {\n  operator a { input in: struct { anchor: text } on in where data.anchor == \"\" { out = . } }\n  operator b { on in { out = . } }\n  b -> out\n  a -> b\n  in -> a\n}\n";
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
                "graph g : App {{\n  connector c : Process {{\n{attrs}  }}\n  in -> c -> out\n  c.err -> nothing\n  c.exit -> nothing\n}}\n"
            )
        };
        let one = compile(
            &[with_attrs("    command = \"cat\" ; codec = lines\n    idempotent = true ; behavior = deterministic\n").as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .unwrap();
        let two = compile(
            &[with_attrs("    behavior = deterministic ; idempotent = true\n    codec = lines ; command = \"cat\"\n").as_str()],
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
                "graph g : App {{\n  operator o {{\n    input in: struct {{ anchor: text }}\n    on in where data.anchor == \"\" && size({map}) > 0 {{ out = . }}\n  }}\n  in -> o -> out\n}}\n"
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
        let source = "graph g : App {\n  input feed: text\n  output sink\n  operator o {\n    on event { result = . }\n  }\n  feed -> o.event\n  o.result -> sink\n}\n";
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
                "graph g : App {{\n  operator o {{\n    input in: struct {{ n: int }}\n    val threshold = {n}\n    on in where data.n > threshold {{ out = . }}\n  }}\n  in -> o -> out\n}}\n"
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
        let (guard, _) = five.plan.handlers[0].dispatch.filter_parts().unwrap();
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
            &["graph g : App {\n  connector c : Process { }\n  in -> c -> out\n}\n"],
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
