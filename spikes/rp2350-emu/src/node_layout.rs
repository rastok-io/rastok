//! Allocation experiment, not a proposed package codec or executable plan.
//!
//! The same validated declarations are copied into three physical layouts.
//! Each copy has one Node header and one body per declaration, with handlers
//! inside Operators. A second comparison includes reference wiring and route
//! composition. Neither comparison qualifies a new decoder or a final VM.

mod wiring;

use alloc::{boxed::Box, vec::Vec};
use core::{mem::size_of, sync::atomic::Ordering};
use cortex_m_semihosting::hprintln;
use rastok_core::{
    expr,
    instance::InstanceSpec,
    node::{HandlerActivation, OwnerPath},
    plan::{
        ConnectorSpec, Dispatch, ExecutionPlan, ExtensionSpec, Handler, NodeKind,
        OwnerValue, PortDecl, VarCell,
    },
};

/// This probe uses the existing canonical owner preorder as the one Node
/// index. It does not change any identity recipe or transported index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NodeIndex(u32);

impl From<rastok_core::node::NodeIndex> for NodeIndex {
    fn from(owner: rastok_core::node::NodeIndex) -> Self {
        Self(u32::try_from(owner.index()).unwrap())
    }
}

#[derive(PartialEq)]
struct Header {
    path: OwnerPath,
    parent: Option<NodeIndex>,
    ports: Vec<PortDecl>,
    extensions: Vec<ExtensionSpec>,
    instance: InstanceSpec,
    vals: Vec<OwnerValue>,
}

#[derive(PartialEq)]
struct HandlerBody {
    activation: HandlerActivation,
    guard: Option<expr::Tree>,
    dispatch: Dispatch,
}

#[derive(PartialEq)]
struct OperatorBody {
    vars: Vec<VarCell>,
    handlers: Vec<HandlerBody>,
}

/// The declaration-only sample leaves this vector empty. The combined sample
/// populates it with the same checked wiring, translated to Node indexes.
#[derive(PartialEq)]
struct GraphBody {
    wiring: Vec<wiring::RouteRule>,
}

struct Node<B> {
    header: Header,
    body: B,
}

enum InlineBody {
    Graph(GraphBody),
    Operator(OperatorBody),
    Connector(ConnectorSpec),
}

enum BoxedBody {
    Graph(Box<GraphBody>),
    Operator(Box<OperatorBody>),
    Connector(Box<ConnectorSpec>),
}

enum BodyIndex {
    Graph(usize),
    Operator(usize),
    Connector(usize),
}

struct Tables {
    nodes: Vec<Node<BodyIndex>>,
    graphs: Vec<GraphBody>,
    operators: Vec<OperatorBody>,
    connectors: Vec<ConnectorSpec>,
}

fn header(plan: &ExecutionPlan, index: usize) -> Header {
    let owner = &plan.nodes[index];
    let ports = &owner.ports;
    Header {
        path: owner.path.clone(),
        parent: owner.parent.map(NodeIndex::from),
        ports: ports.clone(),
        extensions: owner.extensions.clone(),
        instance: owner.instance.clone(),
        vals: owner.vals.clone(),
    }
}

fn body(
    plan: &ExecutionPlan,
    index: usize,
    inventory: &[rastok_core::plan::RouteRule],
) -> InlineBody {
    let node = &plan.nodes[index];
    match &node.body {
        NodeKind::Graph => InlineBody::Graph(GraphBody {
            wiring: wiring::reference(plan, index, inventory),
        }),
        NodeKind::Connector(spec) => InlineBody::Connector(spec.as_ref().clone()),
        NodeKind::Operator(operator) => {
            let handlers = operator.handlers
                .iter()
                .map(|handler| HandlerBody {
                    activation: handler.activation.clone(),
                    guard: handler.guard.clone(),
                    dispatch: handler.dispatch.clone(),
                })
                .collect();
            InlineBody::Operator(OperatorBody {
                vars: operator.vars.clone(),
                handlers,
            })
        }
    }
}

fn inline(
    plan: &ExecutionPlan,
    inventory: &[rastok_core::plan::RouteRule],
) -> Vec<Node<InlineBody>> {
    let mut nodes = Vec::with_capacity(plan.nodes.len());
    for index in 0..plan.nodes.len() {
        nodes.push(Node {
            header: header(plan, index),
            body: body(plan, index, inventory),
        });
    }
    nodes
}

fn boxed(plan: &ExecutionPlan, inventory: &[rastok_core::plan::RouteRule]) -> Vec<Node<BoxedBody>> {
    let mut nodes = Vec::with_capacity(plan.nodes.len());
    for index in 0..plan.nodes.len() {
        let body = match body(plan, index, inventory) {
            InlineBody::Graph(body) => BoxedBody::Graph(Box::new(body)),
            InlineBody::Operator(body) => BoxedBody::Operator(Box::new(body)),
            InlineBody::Connector(body) => BoxedBody::Connector(Box::new(body)),
        };
        nodes.push(Node {
            header: header(plan, index),
            body,
        });
    }
    nodes
}

fn tables(plan: &ExecutionPlan, inventory: &[rastok_core::plan::RouteRule]) -> Tables {
    let mut graphs = 0;
    let mut operators = 0;
    let mut connectors = 0;
    for owner in &plan.nodes {
        match owner.body {
            NodeKind::Graph => graphs += 1,
            NodeKind::Operator(_) => operators += 1,
            NodeKind::Connector(_) => connectors += 1,
        }
    }
    let mut tables = Tables {
        nodes: Vec::with_capacity(plan.nodes.len()),
        graphs: Vec::with_capacity(graphs),
        operators: Vec::with_capacity(operators),
        connectors: Vec::with_capacity(connectors),
    };
    for index in 0..plan.nodes.len() {
        let body = match body(plan, index, inventory) {
            InlineBody::Graph(body) => {
                tables.graphs.push(body);
                BodyIndex::Graph(tables.graphs.len() - 1)
            }
            InlineBody::Operator(body) => {
                tables.operators.push(body);
                BodyIndex::Operator(tables.operators.len() - 1)
            }
            InlineBody::Connector(body) => {
                tables.connectors.push(body);
                BodyIndex::Connector(tables.connectors.len() - 1)
            }
        };
        tables.nodes.push(Node {
            header: header(plan, index),
            body,
        });
    }
    tables
}

// Compare all retained declaration fields, not just a byte count. Validation
// occurs after sampling, so temporary comparison copies are not attributed
// to candidate construction. The original plan never leaves the running VM.
fn check_inline(
    plan: &ExecutionPlan,
    nodes: &[Node<InlineBody>],
    inventory: &[rastok_core::plan::RouteRule],
) {
    assert_eq!(nodes.len(), plan.nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        assert!(node.header == header(plan, index));
        match (&node.body, body(plan, index, inventory)) {
            (InlineBody::Graph(left), InlineBody::Graph(right)) => assert!(left == &right),
            (InlineBody::Operator(left), InlineBody::Operator(right)) => assert!(left == &right),
            (InlineBody::Connector(left), InlineBody::Connector(right)) => assert!(left == &right),
            _ => panic!("node nature changed"),
        }
    }
}

fn check_boxed(
    plan: &ExecutionPlan,
    nodes: &[Node<BoxedBody>],
    inventory: &[rastok_core::plan::RouteRule],
) {
    assert_eq!(nodes.len(), plan.nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        assert!(node.header == header(plan, index));
        match (&node.body, body(plan, index, inventory)) {
            (BoxedBody::Graph(left), InlineBody::Graph(right)) => assert!(left.as_ref() == &right),
            (BoxedBody::Operator(left), InlineBody::Operator(right)) => {
                assert!(left.as_ref() == &right)
            }
            (BoxedBody::Connector(left), InlineBody::Connector(right)) => {
                assert!(left.as_ref() == &right)
            }
            _ => panic!("node nature changed"),
        }
    }
}

fn check_tables(plan: &ExecutionPlan, tables: &Tables, inventory: &[rastok_core::plan::RouteRule]) {
    assert_eq!(tables.nodes.len(), plan.nodes.len());
    for (index, node) in tables.nodes.iter().enumerate() {
        assert!(node.header == header(plan, index));
        match (&node.body, body(plan, index, inventory)) {
            (BodyIndex::Graph(at), InlineBody::Graph(right)) => {
                assert!(tables.graphs[*at] == right)
            }
            (BodyIndex::Operator(at), InlineBody::Operator(right)) => {
                assert!(tables.operators[*at] == right)
            }
            (BodyIndex::Connector(at), InlineBody::Connector(right)) => {
                assert!(tables.connectors[*at] == right)
            }
            _ => panic!("node nature changed"),
        }
    }
}

fn measure<T>(name: &str, build: impl FnOnce() -> T, check: impl FnOnce(&T)) {
    let before = super::HEAP.inner.used();
    let previous_peak = super::HEAP.peak.swap(before, Ordering::Relaxed);
    let candidate = build();
    core::hint::black_box(&candidate);
    let resident = super::HEAP.inner.used() - before;
    let peak = super::HEAP.peak.load(Ordering::Relaxed) - before;
    hprintln!(
        "node_layout_{}: resident {} / construction_peak {} / stack_root {} bytes",
        name,
        resident,
        peak,
        size_of::<T>()
    );
    check(&candidate);
    drop(candidate);
    assert_eq!(
        super::HEAP.inner.used(),
        before,
        "probe leaked a declaration"
    );
    // Keep the true whole-run high-water mark as well as this isolated sample.
    super::HEAP.peak.fetch_max(previous_peak, Ordering::Relaxed);
}

pub(super) fn compare(plan: &ExecutionPlan) {
    hprintln!(
        "node_layout_sizes: header {} / inline_node {} / boxed_node {} / table_node {} / connector {} / handler {} / owned_handler {} bytes",
        size_of::<Header>(),
        size_of::<Node<InlineBody>>(),
        size_of::<Node<BoxedBody>>(),
        size_of::<Node<BodyIndex>>(),
        size_of::<ConnectorSpec>(),
        size_of::<Handler>(),
        size_of::<HandlerBody>(),
    );
    hprintln!(
        "node_layout_control: {} Nodes / {} executors / {} handlers; routing excluded",
        plan.nodes.len(),
        plan.nodes.iter().filter(|node| node.kind() != rastok_core::node::Kind::Graph).count(),
        plan.handlers().count(),
    );
    measure(
        "current",
        || plan.nodes.clone(),
        |nodes| assert!(nodes == &plan.nodes),
    );
    measure(
        "inline",
        || inline(plan, &[]),
        |nodes| check_inline(plan, nodes, &[]),
    );
    measure(
        "boxed",
        || boxed(plan, &[]),
        |nodes| check_boxed(plan, nodes, &[]),
    );
    measure(
        "tables",
        || tables(plan, &[]),
        |tables| check_tables(plan, tables, &[]),
    );
    compare_with_wiring(plan);
}

fn compare_with_wiring(plan: &ExecutionPlan) {
    // Reconstructing the source inventory is preparation of the fixture, not
    // loading a future package. The measured phase includes composition and
    // keeps declarations, reference wiring and derived indexes alive together.
    let inventory = wiring::inventory(plan);
    measure(
        "current_with_wiring",
        || {
            (
                plan.nodes.clone(),
                plan.edges.clone(),
                plan.routes.clone(),
                plan.boundary_routes.clone(),
                wiring::scope_chains(plan),
            )
        },
        |_| {},
    );
    measure(
        "inline_with_wiring",
        || {
            (
                inline(plan, &inventory),
                wiring::Indexes::derive(plan, &inventory),
            )
        },
        |(nodes, indexes)| {
            check_inline(plan, nodes, &inventory);
            indexes.check(plan, &inventory);
        },
    );
    measure(
        "boxed_with_wiring",
        || {
            (
                boxed(plan, &inventory),
                wiring::Indexes::derive(plan, &inventory),
            )
        },
        |(nodes, indexes)| {
            check_boxed(plan, nodes, &inventory);
            indexes.check(plan, &inventory);
        },
    );
    measure(
        "tables_with_wiring",
        || {
            (
                tables(plan, &inventory),
                wiring::Indexes::derive(plan, &inventory),
            )
        },
        |(tables, indexes)| {
            check_tables(plan, tables, &inventory);
            indexes.check(plan, &inventory);
        },
    );
}
