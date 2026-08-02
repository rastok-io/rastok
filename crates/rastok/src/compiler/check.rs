//! Resolution and semantic checking: syntax trees in, HIR out.
//!
//! The checker resolves names, desugars default ports and envelope
//! sugar into semantic forms, compiles every admitted CEL expression,
//! and refuses explicitly — never accidentally — every grammatical
//! construct outside the executable subset of this build
//! (`E-CHECK-DEFERRED`). Rules with a normative home carry their own
//! codes; `meta.sys` writes are the canonical example.

use crate::cel;
use crate::compiler::hir::{
    AttrValue, Attribute, Const, Direction, Edge, EdgeTarget, EmitValue, Endpoint, FieldWrite,
    Graph, Handler, Hir, Node, Statement, Val, Var,
};
use crate::compiler::syntax::{
    BaseType, CelExpr, ConnectorDecl, EdgeDecl, EdgeEndpoint, GraphDecl, GraphItem, OperatorDecl,
    OperatorItem, PortRef, ScalarType, Statement as SynStatement, StructBody, StructEntry,
    SyntaxTree, TopDecl, Trigger as SynTrigger, TypeRef, ValDecl, Value as SynValue, VarDecl,
};
use crate::diagnostics::{Bag, Code, Diagnostic, Phase, SourceId, Span};
use crate::node;
use crate::stdlib;
use std::collections::BTreeMap;

/// Diagnostic for a unit without exactly one `: App` graph.
pub const E_APP_COUNT: Code = Code::new("E-APP-COUNT");
/// Diagnostic for a duplicate node name inside one graph.
pub const E_DUP_NODE: Code = Code::new("E-DUP-NODE");
/// Diagnostic for a name that resolves to no declaration.
pub const E_RESOLVE_UNKNOWN: Code = Code::new("E-RESOLVE-UNKNOWN");
/// Diagnostic for a port that the addressed node does not expose.
pub const E_PORT_UNKNOWN: Code = Code::new("E-PORT-UNKNOWN");
/// Diagnostic for a write reaching the read-only `meta.sys` namespace.
pub const E_META_SYS_READONLY: Code = Code::new("E-META-SYS-READONLY");
/// Diagnostic adapted from the CEL chain, span mapped to the source.
pub const E_CEL_EXPR: Code = Code::new("E-CEL-EXPR");
/// Diagnostic for grammar admitted but outside the executable subset.
pub const E_CHECK_DEFERRED: Code = Code::new("E-CHECK-DEFERRED");
/// Diagnostic for a connector type without an adapter in this build.
pub const E_CONNECTOR_UNAVAILABLE: Code = Code::new("E-CONNECTOR-UNAVAILABLE");
/// Diagnostic for an attribute violating its contract sheet.
pub const E_CONTRACT_ATTR: Code = Code::new("E-CONTRACT-ATTR");
/// Diagnostic for a mandatory contract attribute left undeclared.
pub const E_CONTRACT_REQUIRED: Code = Code::new("E-CONTRACT-REQUIRED");
/// Diagnostic for a binding name that collides with a visible binding
/// or a reserved handler name — shadowing is refused, which is also
/// what makes compile-time constant substitution sound.
pub const E_DUP_BINDING: Code = Code::new("E-DUP-BINDING");
/// Diagnostic for a port used against its direction.
pub const E_PORT_DIRECTION: Code = Code::new("E-PORT-DIRECTION");
/// Diagnostic for a port name declared or inferred with conflicting
/// roles.
pub const E_PORT_CONFLICT: Code = Code::new("E-PORT-CONFLICT");
/// Diagnostic for a wiring cycle — edges form a DAG.
pub const E_GRAPH_CYCLE: Code = Code::new("E-GRAPH-CYCLE");
/// Diagnostic for a static literal with two keys canonicalizing equal.
pub const E_DUP_KEY: Code = Code::new("E-DUP-KEY");

/// Merged sources of one compilation: the application files. Lab
/// fragments join at the scenario stage.
#[derive(Debug, Clone, Default)]
pub struct Unit {
    /// The parsed files, in command-line order.
    pub files: Vec<(SourceId, SyntaxTree)>,
}

/// Checks a unit into HIR against a contract registry. Diagnostics
/// accumulate in `bag`; `None` means the unit has no single `: App`
/// graph to check. A returned HIR is meaningful only when the bag
/// holds no errors.
#[must_use]
pub fn check(unit: &Unit, registry: &stdlib::Registry, bag: &mut Bag) -> Option<Hir> {
    let mut apps = Vec::new();
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            if let TopDecl::Graph(graph) = decl {
                let is_app = graph.annotation.as_ref().is_some_and(|annotation| {
                    annotation.symbols.iter().any(|symbol| symbol.text == "App")
                });
                if is_app {
                    apps.push(graph);
                }
            }
        }
    }
    if apps.len() != 1 {
        let span = apps
            .last()
            .map_or_else(|| Span::new(SourceId::new(0), 0..0), |graph| graph.span);
        bag.push(
            Diagnostic::new(
                E_APP_COUNT,
                Phase::Resolve,
                format!(
                    "a deployed unit needs exactly one `: App` graph, found {}",
                    apps.len()
                ),
            )
            .with_span(span)
            .with_field("count", apps.len().to_string()),
        );
        return None;
    }
    let app = apps[0];
    let mut checker = Checker { bag, registry };
    // No top-level declaration is silently ignored, and none is
    // silently dropped either: until unit composition and a real
    // liveness analysis exist, every additional top-level declaration
    // defers explicitly. W-DEAD-GRAPH belongs to that future analysis
    // and is deliberately not emitted before it.
    for (_, tree) in &unit.files {
        for decl in &tree.decls {
            match decl {
                TopDecl::Graph(graph) if !std::ptr::eq(app, graph) => {
                    checker.deferred("a top-level graph outside the App", graph.name.span);
                }
                TopDecl::Val(val) => {
                    checker.deferred("a top-level val", val.span);
                }
                _ => {}
            }
        }
    }
    Some(Hir {
        app: checker.graph(app),
    })
}

struct Checker<'bag, 'registry> {
    bag: &'bag mut Bag,
    registry: &'registry stdlib::Registry,
}

impl Checker<'_, '_> {
    fn deferred(&mut self, construct: &'static str, span: Span) {
        self.bag.push(
            Diagnostic::new(
                E_CHECK_DEFERRED,
                Phase::Check,
                format!("{construct} is outside the executable subset of this build"),
            )
            .with_span(span)
            .with_field("construct", construct),
        );
    }

    /// Compiles one CEL span, adapting native diagnostics onto the
    /// source with offset-mapped spans.
    fn cel(&mut self, expr: &CelExpr, signature: &cel::Signature) -> Option<cel::Tree> {
        match cel::compile(&expr.text, signature) {
            Ok(tree) => Some(tree),
            Err(diagnostics) => {
                let base = expr.span.range().start;
                for diagnostic in diagnostics {
                    // The registry owns the codes, and the CEL chain's
                    // typed kind chooses them — never the human text,
                    // which is free to change.
                    let code = match diagnostic.kind {
                        cel::DiagnosticKind::DuplicateMapKey => E_DUP_KEY,
                        cel::DiagnosticKind::General => E_CEL_EXPR,
                    };
                    self.bag.push(
                        Diagnostic::new(code, Phase::Check, diagnostic.message.clone())
                            .with_span(Span::new(
                                expr.span.source(),
                                base + diagnostic.range.start..base + diagnostic.range.end,
                            ))
                            .with_field("detail", diagnostic.message),
                    );
                }
                None
            }
        }
    }

    /// Compiles and reduces a constant expression, keeping both the
    /// identity-bearing tree and its reduction.
    fn constant(
        &mut self,
        expr: &CelExpr,
        signature: &cel::Signature,
        consts: &BTreeMap<String, cel::Value>,
    ) -> Option<Const> {
        let tree = self.cel(expr, signature)?;
        match cel::const_eval(&tree, consts) {
            Ok(value) => Some(Const { tree, value }),
            Err(error) => {
                self.bag.push(
                    Diagnostic::new(
                        E_CEL_EXPR,
                        Phase::Check,
                        format!("this position requires a constant: {error}"),
                    )
                    .with_span(expr.span)
                    .with_field("detail", error.message),
                );
                None
            }
        }
    }

    fn graph(&mut self, decl: &GraphDecl) -> Graph {
        if let Some(annotation) = &decl.annotation {
            for symbol in &annotation.symbols {
                if symbol.text != "App" {
                    self.deferred("a graph annotation other than App", symbol.span);
                }
            }
        }
        if decl.per.is_some() {
            self.deferred("a keyed graph (per)", decl.span);
        }
        let mut graph = Graph {
            name: decl.name.text.clone(),
            boundary: Vec::new(),
            vals: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let mut consts = BTreeMap::new();
        let mut signature = cel::Signature::new();
        let mut edges = Vec::new();
        for item in &decl.body {
            match item {
                GraphItem::Port(port) => {
                    if port.ty.is_some() {
                        self.deferred("a typed boundary port", port.span);
                    }
                    let direction = match port.direction {
                        crate::compiler::syntax::PortDirection::Input => Direction::Input,
                        crate::compiler::syntax::PortDirection::Output => Direction::Output,
                    };
                    if graph
                        .boundary
                        .iter()
                        .any(|(existing, _)| existing.name() == port.name.text)
                    {
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_CONFLICT,
                                Phase::Check,
                                format!(
                                    "the boundary port `{}` is already declared",
                                    port.name.text
                                ),
                            )
                            .with_span(port.name.span)
                            .with_field("port", port.name.text.clone()),
                        );
                        continue;
                    }
                    match node::Port::new(port.name.text.clone()) {
                        Ok(declared) => graph.boundary.push((declared, direction)),
                        Err(invalid) => self.bag.push(
                            Diagnostic::new(E_PORT_UNKNOWN, Phase::Check, invalid.to_string())
                                .with_span(port.name.span),
                        ),
                    }
                }
                GraphItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some(value) = self.graph_val(val, &signature, &consts)
                    {
                        signature = signature.with(val.name.text.clone(), value_type(&value.value));
                        consts.insert(val.name.text.clone(), value.value.clone());
                        graph.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                        });
                    }
                }
                GraphItem::Operator(operator) => {
                    if let Some(node) = self.operator(operator, &signature, &consts) {
                        self.push_node(&mut graph, node, operator.name.span);
                    }
                }
                GraphItem::Connector(connector) => {
                    if let Some(node) = self.connector(connector, &signature, &consts) {
                        self.push_node(&mut graph, node, connector.name.span);
                    }
                }
                GraphItem::Edge(edge) => edges.push(edge),
                GraphItem::Partition(partition) => {
                    self.deferred("a partition", partition.span);
                }
                GraphItem::Graph(nested) => self.deferred("a nested graph", nested.span),
                GraphItem::Defaults(_) => self.deferred("a defaults block", decl.span),
                GraphItem::Attribute(attribute) => {
                    self.deferred("a graph attribute", attribute.span);
                }
                GraphItem::Handler(handler) => self.deferred("a graph handler", handler.span),
                GraphItem::Error(_) => {}
            }
        }
        for edge in edges {
            if let Some(resolved) = self.edge(edge, &graph) {
                graph.edges.extend(resolved);
            }
        }
        self.reject_cycles(&graph, decl.span);
        graph
    }

    /// Wiring is a DAG (`grammar.md`): a cycle among nodes refuses.
    fn reject_cycles(&mut self, graph: &Graph, span: Span) {
        let index_of = |path: &node::Path| {
            graph
                .nodes
                .iter()
                .position(|candidate| &candidate.path == path)
        };
        let mut successors = vec![Vec::new(); graph.nodes.len()];
        for edge in &graph.edges {
            if let (Some(from), EdgeTarget::Port(to)) = (&edge.from.node, &edge.to)
                && let (Some(from), Some(to)) = (
                    from.segments().first().and_then(|_| index_of(from)),
                    to.node.as_ref().and_then(&index_of),
                )
            {
                successors[from].push(to);
            }
        }
        // Iterative three-state DFS: 0 unvisited, 1 on the stack, 2 done.
        let mut state = vec![0u8; graph.nodes.len()];
        for start in 0..graph.nodes.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            state[start] = 1;
            while let Some((current, next_child)) = stack.pop() {
                if let Some(&child) = successors[current].get(next_child) {
                    stack.push((current, next_child + 1));
                    match state[child] {
                        0 => {
                            state[child] = 1;
                            stack.push((child, 0));
                        }
                        1 => {
                            let name = graph.nodes[child].path.to_string();
                            self.bag.push(
                                Diagnostic::new(
                                    E_GRAPH_CYCLE,
                                    Phase::Check,
                                    format!("the wiring cycles through `{name}`; edges form a DAG"),
                                )
                                .with_span(span)
                                .with_field("node", name),
                            );
                            return;
                        }
                        _ => {}
                    }
                } else {
                    state[current] = 2;
                }
            }
        }
    }

    fn push_node(&mut self, graph: &mut Graph, node: Node, span: Span) {
        let name = node.path.to_string();
        if graph
            .nodes
            .iter()
            .any(|existing| existing.path == node.path)
        {
            self.bag.push(
                Diagnostic::new(
                    E_DUP_NODE,
                    Phase::Resolve,
                    format!("the node name `{name}` is already declared in this graph"),
                )
                .with_span(span)
                .with_field("name", name),
            );
        } else {
            graph.nodes.push(node);
        }
    }

    fn graph_val(
        &mut self,
        val: &ValDecl,
        signature: &cel::Signature,
        consts: &BTreeMap<String, cel::Value>,
    ) -> Option<Const> {
        if val.ty.is_some() {
            self.deferred("a typed val", val.span);
        }
        match &val.value {
            SynValue::Cel(expr) => self.constant(expr, signature, consts),
            _ => {
                self.deferred("a non-expression val initializer", val.span);
                None
            }
        }
    }

    fn operator(
        &mut self,
        decl: &OperatorDecl,
        graph_signature: &cel::Signature,
        graph_consts: &BTreeMap<String, cel::Value>,
    ) -> Option<Node> {
        if let Some(annotation) = &decl.annotation {
            for symbol in &annotation.symbols {
                self.deferred("an operator alias or aspect", symbol.span);
            }
        }
        if decl.per.is_some() {
            self.deferred("a keyed node (per)", decl.span);
        }
        let path = self.node_path(&decl.name.text, decl.name.span)?;
        let mut node = Node {
            path,
            kind: node::Kind::Operator,
            connector_trait: None,
            attributes: Vec::new(),
            vals: Vec::new(),
            vars: Vec::new(),
            declared_ports: Vec::new(),
            handlers: Vec::new(),
        };
        self.node_body(&decl.body, &mut node, graph_signature, graph_consts, false);
        Some(node)
    }

    fn connector(
        &mut self,
        decl: &ConnectorDecl,
        graph_signature: &cel::Signature,
        graph_consts: &BTreeMap<String, cel::Value>,
    ) -> Option<Node> {
        let mut symbols = decl.annotation.symbols.iter();
        let connector_trait = symbols.next().map(|symbol| symbol.text.clone());
        for aspect in symbols {
            self.deferred("a connector aspect", aspect.span);
        }
        if decl.per.is_some() {
            self.deferred("a keyed node (per)", decl.span);
        }
        let path = self.node_path(&decl.name.text, decl.name.span)?;
        let mut node = Node {
            path,
            kind: node::Kind::Connector,
            connector_trait,
            attributes: Vec::new(),
            vals: Vec::new(),
            vars: Vec::new(),
            declared_ports: Vec::new(),
            handlers: Vec::new(),
        };
        self.node_body(&decl.body, &mut node, graph_signature, graph_consts, true);
        self.contract(&node, decl);
        Some(node)
    }

    /// Validates a connector declaration against its contract sheet.
    fn contract(&mut self, node: &Node, decl: &ConnectorDecl) {
        let symbol = &decl.annotation.symbols[0];
        let contract = match self.registry.connector(&symbol.text) {
            stdlib::Availability::Available(contract) => contract,
            stdlib::Availability::Declared => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!(
                            "`{}` is declared in the catalogue; its adapter is not in this build",
                            symbol.text
                        ),
                    )
                    .with_span(symbol.span)
                    .with_field("type", symbol.text.clone()),
                );
                return;
            }
            stdlib::Availability::Unknown => {
                self.bag.push(
                    Diagnostic::new(
                        E_CONNECTOR_UNAVAILABLE,
                        Phase::Check,
                        format!("`{}` is not a known connector type", symbol.text),
                    )
                    .with_span(symbol.span)
                    .with_field("type", symbol.text.clone()),
                );
                return;
            }
        };
        let span_of = |name: &str| {
            decl.body
                .iter()
                .find_map(|item| match item {
                    OperatorItem::Attribute(attribute)
                        if attribute.target.segments.len() == 1
                            && attribute.target.segments[0].text == name =>
                    {
                        Some(attribute.span)
                    }
                    _ => None,
                })
                .unwrap_or(decl.span)
        };
        // Sheet-forbidden attributes refuse on the declaration itself,
        // before any value evaluation.
        for item in &decl.body {
            let OperatorItem::Attribute(attribute) = item else {
                continue;
            };
            if attribute.target.segments.len() != 1 {
                continue;
            }
            let name = &attribute.target.segments[0].text;
            if let Some(stdlib::AttributeSpec {
                domain: stdlib::AttrDomain::Forbidden(reason),
                ..
            }) = contract.attribute(name)
            {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!("`{name}` violates the {} sheet: {reason}", contract.name),
                    )
                    .with_span(attribute.span)
                    .with_field("attribute", name.clone())
                    .with_field("detail", *reason),
                );
            }
        }
        for attribute in &node.attributes {
            if matches!(
                contract.attribute(&attribute.name),
                Some(stdlib::AttributeSpec {
                    domain: stdlib::AttrDomain::Forbidden(_),
                    ..
                })
            ) {
                continue;
            }
            let span = span_of(&attribute.name);
            let Some(spec) = contract.attribute(&attribute.name) else {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!(
                            "`{}` is not on the {} contract sheet",
                            attribute.name, contract.name
                        ),
                    )
                    .with_span(span)
                    .with_field("attribute", attribute.name.clone())
                    .with_field("detail", "not on the contract sheet"),
                );
                continue;
            };
            let fault: Option<String> = match (&spec.domain, &attribute.value) {
                (stdlib::AttrDomain::Forbidden(reason), _) => Some((*reason).to_string()),
                (stdlib::AttrDomain::Word(words), AttrValue::Word(word)) => {
                    if words.contains(&word.as_str()) {
                        if attribute.name == "mode" && word == "service" {
                            self.deferred("the Process service mode", span);
                        }
                        None
                    } else {
                        Some(format!("expected one of {words:?}, got `{word}`"))
                    }
                }
                (stdlib::AttrDomain::Text, AttrValue::Const(constant))
                    if matches!(constant.value, cel::Value::Text(_)) =>
                {
                    None
                }
                (stdlib::AttrDomain::TextList, AttrValue::Const(constant)) => {
                    match &constant.value {
                        cel::Value::List(items)
                            if items.iter().all(|item| matches!(item, cel::Value::Text(_))) =>
                        {
                            None
                        }
                        _ => Some(String::from("expected a list of text values")),
                    }
                }
                (stdlib::AttrDomain::Duration, AttrValue::Const(constant))
                    if matches!(constant.value, cel::Value::Duration(_)) =>
                {
                    None
                }
                (domain, _) => Some(format!("the value does not fit the {domain:?} domain")),
            };
            if let Some(detail) = fault {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_ATTR,
                        Phase::Check,
                        format!(
                            "`{}` violates the {} sheet: {detail}",
                            attribute.name, contract.name
                        ),
                    )
                    .with_span(span)
                    .with_field("attribute", attribute.name.clone())
                    .with_field("detail", detail),
                );
            }
        }
        for spec in contract.attributes {
            if spec.required
                && !node
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == spec.name)
            {
                self.bag.push(
                    Diagnostic::new(
                        E_CONTRACT_REQUIRED,
                        Phase::Check,
                        format!("the {} sheet requires `{}`", contract.name, spec.name),
                    )
                    .with_span(decl.span)
                    .with_field("attribute", spec.name),
                );
            }
        }
    }

    fn node_path(&mut self, name: &str, span: Span) -> Option<node::Path> {
        match node::Path::new([name]) {
            Ok(path) => Some(path),
            Err(invalid) => {
                self.bag.push(
                    Diagnostic::new(E_RESOLVE_UNKNOWN, Phase::Resolve, invalid.to_string())
                        .with_span(span),
                );
                None
            }
        }
    }

    fn node_body(
        &mut self,
        body: &[OperatorItem],
        node: &mut Node,
        graph_signature: &cel::Signature,
        graph_consts: &BTreeMap<String, cel::Value>,
        is_connector: bool,
    ) {
        let mut signature = graph_signature.clone();
        let mut consts = graph_consts.clone();
        // Pass one: everything except handlers, so the port table and
        // the visible bindings are complete before any handler checks.
        for item in body {
            match item {
                OperatorItem::Port(port) => {
                    if is_connector {
                        // A connector's type defines its ports; the
                        // declaration itself is the fault.
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_CONFLICT,
                                Phase::Check,
                                format!(
                                    "`{}`: a connector never declares ports — its type defines them",
                                    port.name.text
                                ),
                            )
                            .with_span(port.span)
                            .with_field("port", port.name.text.clone()),
                        );
                        continue;
                    }
                    if port.ty.is_some() {
                        self.deferred("a typed port", port.span);
                    }
                    let direction = match port.direction {
                        crate::compiler::syntax::PortDirection::Input => Direction::Input,
                        crate::compiler::syntax::PortDirection::Output => Direction::Output,
                    };
                    self.add_port(node, &port.name, direction, "declared");
                }
                OperatorItem::Val(val) => {
                    if self.bindable(&val.name, &signature)
                        && let Some(value) = self.graph_val(val, &signature, &consts)
                    {
                        signature = signature.with(val.name.text.clone(), value_type(&value.value));
                        consts.insert(val.name.text.clone(), value.value.clone());
                        node.vals.push(Val {
                            name: val.name.text.clone(),
                            value,
                        });
                    }
                }
                OperatorItem::Var(var) => {
                    if self.bindable(&var.name, &signature)
                        && let Some(cell) = self.var(var, &signature, &consts)
                    {
                        signature = signature.with(cell.name.clone(), cell.ty.clone());
                        node.vars.push(cell);
                    }
                }
                OperatorItem::Attribute(attribute) => {
                    if is_connector {
                        if let Some(checked) = self.attribute_value(
                            &attribute.value,
                            &signature,
                            &consts,
                            attribute.span,
                        ) {
                            node.attributes.push(Attribute {
                                name: attribute
                                    .target
                                    .segments
                                    .iter()
                                    .map(|segment| segment.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("."),
                                value: checked,
                            });
                        }
                    } else {
                        self.deferred("an operator attribute", attribute.span);
                    }
                }
                OperatorItem::SubBlock(block) => {
                    self.deferred("a connector namespace block", block.span);
                }
                OperatorItem::Handler(_) => {}
                OperatorItem::Error(_) => {}
            }
        }
        // Pass two, operators only: handler triggers and emission
        // targets declare ports implicitly — `input`/`output` exist to
        // pin a type or publish an interface, not as a prerequisite.
        if !is_connector {
            self.infer_ports(body, node);
        }
        // Pass three: handlers, checked against the complete table.
        for item in body {
            if let OperatorItem::Handler(handler) = item {
                if is_connector {
                    self.deferred("a connector handler", handler.span);
                } else if let Some(checked) = self.handler(handler, &signature, node) {
                    node.handlers.push(checked);
                }
            }
        }
    }

    /// Adds one port to a node's semantic table, refusing conflicting
    /// roles. Default ports are implicit and never added; a matching
    /// explicit or inferred use of a default name is checked against
    /// the default direction instead.
    fn add_port(
        &mut self,
        node: &mut Node,
        name: &crate::compiler::syntax::Name,
        direction: Direction,
        origin: &str,
    ) {
        let default_direction = match name.text.as_str() {
            "in" => Some(Direction::Input),
            "out" | "err" => Some(Direction::Output),
            _ => None,
        };
        let existing = default_direction.or_else(|| {
            node.declared_ports
                .iter()
                .find(|(port, _)| port.name() == name.text)
                .map(|(_, direction)| *direction)
        });
        match existing {
            Some(known) if known != direction => {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_CONFLICT,
                        Phase::Check,
                        format!(
                            "`{}` is already {} port; it cannot also be {} ({origin})",
                            name.text,
                            direction_words(known),
                            direction_words(direction),
                        ),
                    )
                    .with_span(name.span)
                    .with_field("port", name.text.clone()),
                );
            }
            Some(_) => {}
            None => {
                if let Ok(port) = node::Port::new(name.text.clone()) {
                    node.declared_ports.push((port, direction));
                } else {
                    self.bag.push(
                        Diagnostic::new(
                            E_PORT_UNKNOWN,
                            Phase::Check,
                            format!("`{}` is not a valid port name", name.text),
                        )
                        .with_span(name.span),
                    );
                }
            }
        }
    }

    /// Infers the ports a node's handlers imply: a trigger names an
    /// input, an emission target names an output. State cells and
    /// constants are not ports.
    fn infer_ports(&mut self, body: &[OperatorItem], node: &mut Node) {
        for item in body {
            let OperatorItem::Handler(handler) = item else {
                continue;
            };
            if let crate::compiler::syntax::Trigger::Port(name) = &handler.trigger {
                self.add_port(node, name, Direction::Input, "fired by a handler");
            }
            let Some(statements) = &handler.body else {
                continue;
            };
            for statement in statements {
                let SynStatement::Assign { target, .. } = statement else {
                    continue;
                };
                if target.segments.len() != 1 {
                    continue;
                }
                let name = &target.segments[0];
                let is_state = node.vars.iter().any(|var| var.name == name.text)
                    || node.vals.iter().any(|val| val.name == name.text)
                    || name.text == "err";
                if !is_state {
                    self.add_port(node, name, Direction::Output, "an emission target");
                }
            }
        }
    }

    fn var(
        &mut self,
        decl: &VarDecl,
        signature: &cel::Signature,
        consts: &BTreeMap<String, cel::Value>,
    ) -> Option<Var> {
        let (ty, int_bounded) = self.state_type(&decl.ty)?;
        let init = match &decl.value {
            None => None,
            Some(SynValue::Cel(expr)) => {
                let constant = self.constant(expr, signature, consts)?;
                if int_bounded
                    && let cel::Value::Int(value) = constant.value
                    && i32::try_from(value).is_err()
                {
                    self.bag.push(
                        Diagnostic::new(
                            E_CEL_EXPR,
                            Phase::Check,
                            format!("`{value}` does not fit `int` (32-bit)"),
                        )
                        .with_span(expr.span)
                        .with_field("detail", "out of the 32-bit int range"),
                    );
                    return None;
                }
                let init_ty = value_type(&constant.value);
                if init_ty != ty && init_ty != cel::Type::Dyn && ty != cel::Type::Dyn {
                    self.bag.push(
                        Diagnostic::new(
                            E_CEL_EXPR,
                            Phase::Check,
                            format!(
                                "`{}` is declared {ty:?}, its initializer is a {init_ty:?}",
                                decl.name.text
                            ),
                        )
                        .with_span(expr.span)
                        .with_field("detail", format!("{init_ty:?} does not fit {ty:?}")),
                    );
                    return None;
                }
                Some(constant)
            }
            Some(_) => {
                self.deferred("a non-expression var initializer", decl.span);
                return None;
            }
        };
        Some(Var {
            name: decl.name.text.clone(),
            ty,
            init,
        })
    }

    /// Whether `name` may introduce a binding here: reserved handler
    /// names and shadowing are refused — which is also what keeps
    /// compile-time constant substitution sound.
    fn bindable(
        &mut self,
        name: &crate::compiler::syntax::Name,
        signature: &cel::Signature,
    ) -> bool {
        let collision = if matches!(name.text.as_str(), "data" | "meta") {
            Some("a reserved handler name")
        } else if signature.get(&name.text).is_some() {
            Some("an already visible binding")
        } else {
            None
        };
        match collision {
            None => true,
            Some(reason) => {
                self.bag.push(
                    Diagnostic::new(
                        E_DUP_BINDING,
                        Phase::Check,
                        format!("`{}` shadows {reason}", name.text),
                    )
                    .with_span(name.span)
                    .with_field("name", name.text.clone()),
                );
                false
            }
        }
    }

    /// Maps a declared state type onto the CEL domain. Returns the
    /// type and whether it carries the 32-bit `int` bound. `long` and
    /// `float` defer explicitly: the CEL domain does not represent
    /// them distinctly yet, and a silent merge would corrupt identity.
    fn state_type(&mut self, ty: &TypeRef) -> Option<(cel::Type, bool)> {
        if ty.optional || ty.alternatives.len() != 1 {
            self.deferred("an optional or union state type", ty.span);
            return None;
        }
        let mapped = match &ty.alternatives[0] {
            BaseType::Scalar(ScalarType::Bool) => (cel::Type::Bool, false),
            BaseType::Scalar(ScalarType::Int) => (cel::Type::Int, true),
            BaseType::Scalar(ScalarType::Long) => {
                self.deferred("the long state type", ty.span);
                return None;
            }
            BaseType::Scalar(ScalarType::Float) => {
                self.deferred("the float state type", ty.span);
                return None;
            }
            BaseType::Scalar(ScalarType::Double) => (cel::Type::Double, false),
            BaseType::Scalar(ScalarType::Text) => (cel::Type::Text, false),
            BaseType::Scalar(ScalarType::Bytes) => (cel::Type::Bytes, false),
            BaseType::Scalar(ScalarType::Duration) => (cel::Type::Duration, false),
            BaseType::Scalar(ScalarType::Timestamp) => (cel::Type::Timestamp, false),
            BaseType::Variant => (cel::Type::Dyn, false),
            _ => {
                self.deferred("this state type", ty.span);
                return None;
            }
        };
        Some(mapped)
    }

    fn attribute_value(
        &mut self,
        value: &SynValue,
        signature: &cel::Signature,
        consts: &BTreeMap<String, cel::Value>,
        span: Span,
    ) -> Option<AttrValue> {
        match value {
            SynValue::Cel(expr) if is_bare_word(&expr.text) => {
                Some(AttrValue::Word(expr.text.clone()))
            }
            SynValue::Cel(expr) => self.constant(expr, signature, consts).map(AttrValue::Const),
            _ => {
                self.deferred("this attribute value form", span);
                None
            }
        }
    }

    fn handler(
        &mut self,
        decl: &crate::compiler::syntax::HandlerDecl,
        node_signature: &cel::Signature,
        node: &Node,
    ) -> Option<Handler> {
        let trigger = match &decl.trigger {
            SynTrigger::Port(name) => {
                let Some(direction) = self.node_port_direction(node, &name.text) else {
                    self.bag.push(
                        Diagnostic::new(
                            E_PORT_UNKNOWN,
                            Phase::Check,
                            format!("this node exposes no port named `{}`", name.text),
                        )
                        .with_span(name.span)
                        .with_field("port", name.text.clone()),
                    );
                    return None;
                };
                if direction != Direction::Input {
                    self.wrong_direction(
                        name.span,
                        &name.text,
                        direction,
                        "a handler listens on an input port",
                    );
                    return None;
                }
                match node::Port::new(name.text.clone()) {
                    Ok(port) => node::Trigger::Port(port),
                    Err(_) => return None,
                }
            }
            SynTrigger::Start(_) => node::Trigger::Lifecycle(node::Lifecycle::Start),
            SynTrigger::Stop(_) => node::Trigger::Lifecycle(node::Lifecycle::Stop),
            SynTrigger::Compensate(span) => {
                self.deferred("a compensate handler", *span);
                return None;
            }
            SynTrigger::Aspect { aspect, .. } => {
                self.deferred("an aspect-qualified handler", aspect.span);
                return None;
            }
            SynTrigger::Join(_) => {
                self.deferred("a join handler", decl.span);
                return None;
            }
        };
        let mut signature = node_signature
            .clone()
            .with("data", cel::Type::Dyn)
            .with("meta", cel::Type::Dyn);
        let mut guard = None;
        for clause in &decl.guards {
            match clause {
                crate::compiler::syntax::Guard::Where(expr) if guard.is_none() => {
                    let tree = self.cel(expr, &signature)?;
                    if !matches!(tree.ty, cel::Type::Bool | cel::Type::Dyn) {
                        self.bag.push(
                            Diagnostic::new(
                                E_CEL_EXPR,
                                Phase::Check,
                                "a where guard must be a bool",
                            )
                            .with_span(expr.span)
                            .with_field("detail", "the guard type is not bool"),
                        );
                        return None;
                    }
                    guard = Some(tree);
                }
                crate::compiler::syntax::Guard::Where(expr) => {
                    self.deferred("a second where guard", expr.span);
                }
                _ => {
                    self.deferred("a join guard clause", decl.span);
                }
            }
        }
        let Some(body) = &decl.body else {
            self.deferred("a body-less handler outside Loop", decl.span);
            return None;
        };
        let mut statements = Vec::new();
        for statement in body {
            match statement {
                SynStatement::Val(val) => match &val.value {
                    SynValue::Cel(expr) => {
                        if self.bindable(&val.name, &signature)
                            && let Some(tree) = self.cel(expr, &signature)
                        {
                            signature = signature.with(val.name.text.clone(), tree.ty.clone());
                            statements.push(Statement::Let {
                                name: val.name.text.clone(),
                                value: tree,
                            });
                        }
                    }
                    _ => self.deferred("a non-expression local val", val.span),
                },
                SynStatement::Assign { target, ty, value } => {
                    if ty.is_some() {
                        self.deferred("a typed assignment", target.span);
                    }
                    if target.segments.len() > 1 {
                        if target.segments[0].text == "meta" && target.segments[1].text == "sys" {
                            self.meta_sys_write(target.span);
                        } else {
                            self.deferred("a dotted statement target", target.span);
                        }
                        continue;
                    }
                    let name = &target.segments[0];
                    if let Some(var) = node.vars.iter().find(|var| var.name == name.text) {
                        match value {
                            SynValue::Cel(expr) => {
                                if let Some(tree) = self.cel(expr, &signature) {
                                    if tree.ty != var.ty
                                        && tree.ty != cel::Type::Dyn
                                        && var.ty != cel::Type::Dyn
                                    {
                                        self.bag.push(
                                            Diagnostic::new(
                                                E_CEL_EXPR,
                                                Phase::Check,
                                                format!(
                                                    "`{}` is a {:?}, this value is a {:?}",
                                                    name.text, var.ty, tree.ty
                                                ),
                                            )
                                            .with_span(expr.span)
                                            .with_field(
                                                "detail",
                                                format!("{:?} does not fit {:?}", tree.ty, var.ty),
                                            ),
                                        );
                                        continue;
                                    }
                                    statements.push(Statement::SetVar {
                                        name: name.text.clone(),
                                        value: tree,
                                    });
                                }
                            }
                            _ => self.deferred("a non-expression state write", target.span),
                        }
                        continue;
                    }
                    let Some(direction) = self.node_port_direction(node, &name.text) else {
                        self.bag.push(
                            Diagnostic::new(
                                E_RESOLVE_UNKNOWN,
                                Phase::Resolve,
                                format!(
                                    "`{}` names neither a port nor a state cell here",
                                    name.text
                                ),
                            )
                            .with_span(name.span)
                            .with_field("name", name.text.clone()),
                        );
                        continue;
                    };
                    if direction != Direction::Output {
                        self.wrong_direction(
                            name.span,
                            &name.text,
                            direction,
                            "an emission targets an output port",
                        );
                        continue;
                    }
                    let Ok(port) = node::Port::new(name.text.clone()) else {
                        continue;
                    };
                    if let Some(emit) = self.emit_value(value, &signature) {
                        statements.push(Statement::Emit { port, value: emit });
                    }
                }
                SynStatement::ErrEmit { kind, .. } => {
                    self.deferred("a typed-error emission", kind.span);
                }
                SynStatement::MultiEmit { targets, .. } => {
                    let span = targets.first().map_or(decl.span, |target| target.span);
                    self.deferred("an atomic multi-emit", span);
                }
                SynStatement::Yield(_) => self.deferred("an implicit yield", decl.span),
                SynStatement::Stop(_) => self.deferred("a stop action", decl.span),
                SynStatement::Error(_) => {}
            }
        }
        Some(Handler {
            trigger,
            guard,
            body: statements,
        })
    }

    fn meta_sys_write(&mut self, span: Span) {
        self.bag.push(
            Diagnostic::new(
                E_META_SYS_READONLY,
                Phase::Check,
                "meta.sys is read-only and runtime-owned; it is never an lvalue",
            )
            .with_span(span),
        );
    }

    fn emit_value(&mut self, value: &SynValue, signature: &cel::Signature) -> Option<EmitValue> {
        match value {
            SynValue::Envelope(None) => Some(EmitValue::Identity),
            SynValue::Envelope(Some(body)) => {
                self.field_writes(body, signature).map(EmitValue::Patch)
            }
            SynValue::EnvelopeSugar(body) => {
                self.field_writes(body, signature).map(EmitValue::Replace)
            }
            SynValue::Nothing(_) => Some(EmitValue::Nothing),
            SynValue::Cel(expr) => self.cel(expr, signature).map(EmitValue::Data),
            SynValue::Struct(body) | SynValue::Map(body) => {
                let span = body
                    .entries
                    .first()
                    .map(|entry| match entry {
                        StructEntry::Field { key, .. } => key.span,
                        StructEntry::Block { name, .. } => name.span,
                    })
                    .unwrap_or_else(|| Span::new(SourceId::new(0), 0..0));
                self.deferred("a struct or map literal emission", span);
                None
            }
            SynValue::Tagged(tagged) => {
                self.deferred("a guest or tagged emission", tagged.tag.span);
                None
            }
        }
    }

    fn field_writes(
        &mut self,
        body: &StructBody,
        signature: &cel::Signature,
    ) -> Option<Vec<FieldWrite>> {
        let mut writes = Vec::new();
        for entry in &body.entries {
            match entry {
                StructEntry::Field { key, value } => {
                    let path: Vec<String> = key
                        .segments
                        .iter()
                        .map(|segment| segment.text.clone())
                        .collect();
                    if path.first().map(String::as_str) == Some("meta")
                        && path.get(1).map(String::as_str) == Some("sys")
                    {
                        self.meta_sys_write(key.span);
                        continue;
                    }
                    match value {
                        SynValue::Cel(expr) => {
                            if let Some(tree) = self.cel(expr, signature) {
                                writes.push(FieldWrite { path, value: tree });
                            }
                        }
                        _ => self.deferred("a nested envelope builder", key.span),
                    }
                }
                StructEntry::Block { name, .. } => {
                    self.deferred("a block inside an envelope builder", name.span);
                }
            }
        }
        Some(writes)
    }

    fn edge(&mut self, decl: &EdgeDecl, graph: &Graph) -> Option<Vec<Edge>> {
        if decl.bidirectional {
            self.deferred("a bidirectional edge", decl.span);
            return None;
        }
        if decl.guard.is_some() {
            self.deferred("an edge guard (when)", decl.span);
            return None;
        }
        if decl.config.is_some() {
            self.deferred("an edge configuration block", decl.span);
            return None;
        }
        let mut resolved = Vec::new();
        for pair in decl.endpoints.windows(2) {
            let from = self.endpoint(&pair[0], graph, true)?;
            let to = match &pair[1] {
                EdgeEndpoint::Nothing(_) => EdgeTarget::Discard,
                other => EdgeTarget::Port(self.endpoint(other, graph, false)?),
            };
            resolved.push(Edge { from, to });
        }
        Some(resolved)
    }

    fn endpoint(
        &mut self,
        endpoint: &EdgeEndpoint,
        graph: &Graph,
        is_source: bool,
    ) -> Option<Endpoint> {
        let reference = match endpoint {
            EdgeEndpoint::Ref(reference) => reference,
            EdgeEndpoint::Branch(refs) => {
                let span = refs.first().map_or_else(
                    || Span::new(SourceId::new(0), 0..0),
                    |reference| reference.span,
                );
                self.deferred("a branch edge", span);
                return None;
            }
            EdgeEndpoint::Nothing(span) => {
                self.deferred("nothing as an edge source", *span);
                return None;
            }
        };
        self.port_ref(reference, graph, is_source)
    }

    /// The direction a port has on a node: declared ports first, then
    /// the connector contract's port sheet, then the pre-bound
    /// defaults. `None` means the node exposes no such port.
    fn node_port_direction(&self, target: &Node, port_name: &str) -> Option<Direction> {
        if let Some((_, direction)) = target
            .declared_ports
            .iter()
            .find(|(port, _)| port.name() == port_name)
        {
            return Some(*direction);
        }
        if let Some(trait_name) = &target.connector_trait
            && let stdlib::Availability::Available(contract) = self.registry.connector(trait_name)
        {
            if contract.input_ports.contains(&port_name) {
                return Some(Direction::Input);
            }
            if contract.output_ports.contains(&port_name) {
                return Some(Direction::Output);
            }
            return None;
        }
        match port_name {
            "in" => Some(Direction::Input),
            "out" | "err" => Some(Direction::Output),
            _ => None,
        }
    }

    fn wrong_direction(&mut self, span: Span, port: &str, direction: Direction, required: &str) {
        let have = match direction {
            Direction::Input => "an input",
            Direction::Output => "an output",
        };
        self.bag.push(
            Diagnostic::new(
                E_PORT_DIRECTION,
                Phase::Check,
                format!("`{port}` is {have} port; {required}"),
            )
            .with_span(span)
            .with_field("port", port)
            .with_field(
                "direction",
                match direction {
                    Direction::Input => "input",
                    Direction::Output => "output",
                },
            ),
        );
    }

    fn port_ref(
        &mut self,
        reference: &PortRef,
        graph: &Graph,
        is_source: bool,
    ) -> Option<Endpoint> {
        if reference.boundary {
            // As a source the boundary feeds the graph (an input port
            // of the graph); as a target it drains it (an output).
            let expected = if is_source {
                Direction::Input
            } else {
                Direction::Output
            };
            let port_name = match reference.segments.first() {
                None => {
                    if is_source {
                        "in"
                    } else {
                        "out"
                    }
                }
                Some(name) => {
                    let Some((_, direction)) = graph
                        .boundary
                        .iter()
                        .find(|(port, _)| port.name() == name.text)
                    else {
                        self.bag.push(
                            Diagnostic::new(
                                E_PORT_UNKNOWN,
                                Phase::Check,
                                format!("`.{}` names no declared boundary port", name.text),
                            )
                            .with_span(name.span)
                            .with_field("port", name.text.clone()),
                        );
                        return None;
                    };
                    if *direction != expected {
                        let required = if is_source {
                            "a boundary edge source must be an input of the graph"
                        } else {
                            "a boundary edge target must be an output of the graph"
                        };
                        self.wrong_direction(name.span, &name.text, *direction, required);
                        return None;
                    }
                    name.text.as_str()
                }
            };
            return node::Port::new(port_name)
                .ok()
                .map(|port| Endpoint { node: None, port });
        }
        let node_name = &reference.segments[0];
        let Some(target) = graph
            .nodes
            .iter()
            .find(|candidate| candidate.path.segments()[0] == node_name.text)
        else {
            self.bag.push(
                Diagnostic::new(
                    E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!("`{}` names no node in this graph", node_name.text),
                )
                .with_span(node_name.span)
                .with_field("name", node_name.text.clone()),
            );
            return None;
        };
        let port_name = match reference.segments.len() {
            1 => {
                if is_source {
                    "out"
                } else {
                    "in"
                }
            }
            2 => reference.segments[1].text.as_str(),
            _ => {
                self.deferred("a composed edge path", reference.span);
                return None;
            }
        };
        let Some(direction) = self.node_port_direction(target, port_name) else {
            self.bag.push(
                Diagnostic::new(
                    E_PORT_UNKNOWN,
                    Phase::Check,
                    format!("`{}` exposes no port named `{}`", node_name.text, port_name),
                )
                .with_span(reference.span)
                .with_field("port", port_name),
            );
            return None;
        };
        let expected = if is_source {
            Direction::Output
        } else {
            Direction::Input
        };
        if direction != expected {
            let required = if is_source {
                "an edge source must be an output of its node"
            } else {
                "an edge target must be an input of its node"
            };
            self.wrong_direction(reference.span, port_name, direction, required);
            return None;
        }
        node::Port::new(port_name).ok().map(|port| Endpoint {
            node: Some(target.path.clone()),
            port,
        })
    }
}

fn is_bare_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !text.as_bytes()[0].is_ascii_digit()
        && !matches!(text, "true" | "false" | "null")
}

fn direction_words(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "an input",
        Direction::Output => "an output",
    }
}

fn value_type(value: &cel::Value) -> cel::Type {
    match value {
        cel::Value::Bool(_) => cel::Type::Bool,
        cel::Value::Int(_) => cel::Type::Int,
        cel::Value::Uint(_) => cel::Type::Uint,
        cel::Value::Double(_) => cel::Type::Double,
        cel::Value::Text(_) => cel::Type::Text,
        cel::Value::Bytes(_) => cel::Type::Bytes,
        cel::Value::Duration(_) => cel::Type::Duration,
        cel::Value::Timestamp(_) => cel::Type::Timestamp,
        cel::Value::Null => cel::Type::Null,
        cel::Value::List(_) => cel::Type::List(Box::new(cel::Type::Dyn)),
        cel::Value::Map(_) => cel::Type::Map(Box::new(cel::Type::Dyn)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::parser::parse;

    fn check_source(source: &str) -> (Option<Hir>, Bag) {
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), source, &mut bag);
        let unit = Unit {
            files: vec![(SourceId::new(0), tree)],
        };
        let hir = check(&unit, &stdlib::Registry::m0(), &mut bag);
        (hir, bag)
    }

    fn diagnostic_codes(bag: &Bag) -> Vec<Code> {
        bag.diagnostics()
            .iter()
            .map(crate::diagnostics::Diagnostic::code)
            .collect()
    }

    fn check_clean(source: &str) -> Hir {
        let (hir, bag) = check_source(source);
        assert!(
            !bag.has_errors(),
            "unexpected diagnostics: {:#?}",
            bag.diagnostics()
        );
        hir.expect("an app graph")
    }

    const M0_PROBE: &str = r#"
graph m0_probe : App {
  val greeting = "hello"

  connector echo : Process {
    mode = job ; command = "cat"
    codec = lines
    effect = idempotent ; behavior = deterministic
  }

  operator shout {
    var count : int = 0
    on in where data.text != "" {
      count = count + 1
      out = envelope { data.text = data.text }
    }
  }

  . -> shout -> echo -> .
}
"#;

    #[test]
    fn the_m0_probe_checks_into_hir() {
        let hir = check_clean(M0_PROBE);
        assert_eq!(hir.app.name, "m0_probe");
        assert_eq!(hir.app.vals[0].name, "greeting");
        assert_eq!(hir.app.nodes.len(), 2);
        let echo = &hir.app.nodes[0];
        assert_eq!(echo.kind, node::Kind::Connector);
        assert_eq!(echo.connector_trait.as_deref(), Some("Process"));
        assert_eq!(echo.attributes.len(), 5);
        assert_eq!(
            echo.attributes[0].value,
            AttrValue::Word(String::from("job"))
        );
        let AttrValue::Const(command) = &echo.attributes[1].value else {
            panic!("expected a constant attribute");
        };
        assert_eq!(command.value, cel::Value::Text(String::from("cat")));
        let shout = &hir.app.nodes[1];
        assert_eq!(shout.kind, node::Kind::Operator);
        assert_eq!(shout.vars[0].ty, cel::Type::Int);
        let handler = &shout.handlers[0];
        assert_eq!(
            handler.trigger,
            node::Trigger::Port(node::Port::new("in").unwrap())
        );
        assert!(handler.guard.is_some());
        assert!(matches!(&handler.body[0], Statement::SetVar { name, .. } if name == "count"));
        let Statement::Emit { port, value } = &handler.body[1] else {
            panic!("expected the emission");
        };
        assert_eq!(port.name(), "out");
        let EmitValue::Patch(writes) = value else {
            panic!("expected an envelope patch");
        };
        assert_eq!(writes[0].path, ["data", "text"]);
    }

    #[test]
    fn edges_desugar_to_explicit_pairs() {
        let hir = check_clean(M0_PROBE);
        let edges = &hir.app.edges;
        assert_eq!(edges.len(), 3);
        assert_eq!(edges[0].from.node, None);
        assert_eq!(edges[0].from.port.name(), "in");
        let EdgeTarget::Port(to) = &edges[0].to else {
            panic!("expected a port target");
        };
        assert_eq!(to.node.as_ref().unwrap().to_string(), "shout");
        assert_eq!(to.port.name(), "in");
        let EdgeTarget::Port(last) = &edges[2].to else {
            panic!("expected a port target");
        };
        assert_eq!(last.node, None);
        assert_eq!(last.port.name(), "out");
    }

    #[test]
    fn a_meta_sys_write_carries_its_normative_code() {
        let source = "graph g : App {\n  operator o {\n    on in {\n      out = envelope { meta.sys.id = data.x }\n    }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_META_SYS_READONLY)
            .expect("the meta.sys refusal");
        assert!(diagnostic.message().contains("read-only"));
    }

    #[test]
    fn a_unit_needs_exactly_one_app() {
        let (hir, bag) = check_source("graph a : App { }\ngraph b : App { }\n");
        assert!(hir.is_none());
        assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
        let (hir, bag) = check_source("graph a { }\n");
        assert!(hir.is_none());
        assert_eq!(bag.diagnostics()[0].code(), E_APP_COUNT);
    }

    #[test]
    fn unknown_names_and_ports_are_refused() {
        let source = "graph g : App {\n  operator o { }\n  o -> ghost\n  o.mystery -> o\n}\n";
        let (_, bag) = check_source(source);
        let codes: Vec<_> = bag
            .diagnostics()
            .iter()
            .map(crate::diagnostics::Diagnostic::code)
            .collect();
        assert!(codes.contains(&E_RESOLVE_UNKNOWN));
        assert!(codes.contains(&E_PORT_UNKNOWN));
    }

    #[test]
    fn duplicate_nodes_are_refused() {
        let (_, bag) = check_source("graph g : App {\n  operator o { }\n  operator o { }\n}\n");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == E_DUP_NODE)
        );
    }

    #[test]
    fn cel_faults_map_back_to_source_spans() {
        let source = "graph g : App {\n  operator o {\n    on in where mystery > 0 { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CEL_EXPR)
            .expect("a CEL diagnostic");
        let span = diagnostic.spans()[0];
        let range = span.range();
        assert_eq!(&source[range], "mystery > 0");
    }

    #[test]
    fn a_missing_mandatory_axis_violates_the_sheet() {
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n    behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_REQUIRED)
            .expect("the missing-effect refusal");
        assert_eq!(diagnostic.fields()[0].1, "effect");
    }

    #[test]
    fn a_domain_violation_names_the_sheet() {
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = utf16\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
            .expect("the codec refusal");
        assert!(diagnostic.message().contains("Process sheet"));
    }

    #[test]
    fn correlate_on_a_job_carries_the_sheet_reason() {
        let source = "graph g : App {\n  connector c : Process {\n    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n    correlate = data.id\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CONTRACT_ATTR)
            .expect("the correlate refusal");
        assert!(diagnostic.message().contains("process boundary"));
    }

    #[test]
    fn unavailable_and_unknown_connector_types_split() {
        let declared = "graph g : App {\n  connector h : HttpServer { }\n  . -> h -> .\n}\n";
        let (_, bag) = check_source(declared);
        assert!(diagnostic_codes(&bag).contains(&E_CONNECTOR_UNAVAILABLE));
        assert!(bag.diagnostics()[0].message().contains("catalogue"));
        let unknown = "graph g : App {\n  connector d : Postgres { }\n  . -> d -> .\n}\n";
        let (_, bag) = check_source(unknown);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("not a known connector type"))
        );
    }

    #[test]
    fn a_struct_emission_is_no_longer_silently_dropped() {
        let source = "graph g : App {\n  operator o {\n    on in { out = struct { n = 1 } }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(
            bag.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == E_CHECK_DEFERRED
                    && diagnostic.fields()[0].1 == "a struct or map literal emission"
            }),
            "the emission must refuse, not vanish: {:#?}",
            bag.diagnostics()
        );
    }

    #[test]
    fn no_top_level_declaration_is_silently_ignored() {
        // Until unit composition and real liveness exist, an extra
        // top-level graph is an explicit deferral — an ERROR, so its
        // unchecked content can never ride along silently.
        let source = "graph main : App { }\ngraph orphan { }\nval loose = 1\n";
        let (_, bag) = check_source(source);
        assert!(bag.has_errors());
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert!(constructs.contains(&"a top-level graph outside the App"));
        assert!(constructs.contains(&"a top-level val"));
    }

    #[test]
    fn a_var_initializer_must_fit_its_declared_type() {
        let source = "graph g : App {\n  operator o {\n    var count : int = \"oops\"\n    on in { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_CEL_EXPR)
            .expect("the type mismatch");
        assert!(diagnostic.message().contains("declared Int"));
    }

    #[test]
    fn shadowing_and_reserved_names_are_refused() {
        let source = "graph g : App {\n  val limit = 1\n  operator o {\n    var limit : int = 0\n    var data : int = 0\n    on in { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let shadows: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_DUP_BINDING)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert_eq!(shadows, ["limit", "data"]);
    }

    #[test]
    fn handler_and_emission_ports_are_inferred() {
        // The reviewer's reproduction: `on event` implies an input,
        // `result = envelope` implies an output — no declaration
        // needed. This program is conforming and must check cleanly.
        let source = "graph g : App {\n  input feed\n  output sink\n\n  operator o {\n    on event {\n      result = envelope\n    }\n  }\n\n  .feed -> o.event\n  o.result -> .sink\n}\n";
        let hir = check_clean(source);
        let ports = &hir.app.nodes[0].declared_ports;
        assert!(
            ports
                .iter()
                .any(|(port, d)| port.name() == "event" && *d == Direction::Input)
        );
        assert!(
            ports
                .iter()
                .any(|(port, d)| port.name() == "result" && *d == Direction::Output)
        );
    }

    #[test]
    fn conflicting_port_roles_are_refused() {
        // The same name inferred as input and output.
        let source = "graph g : App {\n  operator o {\n    on chan {\n      chan = envelope\n    }\n  }\n  . -> o.chan -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
        // Explicit double declaration at the boundary.
        let source = "graph g : App {\n  input feed\n  output feed\n  operator o { on in { out = envelope } }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_CONFLICT));
    }

    #[test]
    fn a_connector_never_declares_ports() {
        let source = "graph g : App {\n  connector c : Process {\n    input extra\n    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> c -> .\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_PORT_CONFLICT)
            .expect("the connector port refusal");
        assert!(diagnostic.message().contains("its type defines them"));
    }

    #[test]
    fn physical_integer_types_are_not_silently_merged() {
        let long = "graph g : App {\n  operator o {\n    var n : long = 0\n    on in { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(long);
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert!(constructs.contains(&"the long state type"));
        let overflow = "graph g : App {\n  operator o {\n    var n : int = 2147483648\n    on in { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(overflow);
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("does not fit `int`"))
        );
    }

    #[test]
    fn ports_are_used_with_their_direction() {
        // `o.in` as an edge source, a handler on `out`, and an
        // emission into `in` all refuse.
        let source =
            "graph g : App {\n  operator o {\n    on out { in = envelope }\n  }\n  o.in -> o\n}\n";
        let (_, bag) = check_source(source);
        let directions = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
            .count();
        assert_eq!(
            directions,
            2,
            "handler + edge source: {:#?}",
            bag.diagnostics()
        );
        let emission =
            "graph g : App {\n  operator o {\n    on in { in = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(emission);
        assert!(diagnostic_codes(&bag).contains(&E_PORT_DIRECTION));
    }

    #[test]
    fn boundary_ports_carry_direction() {
        let source = "graph g : App {\n  input feed\n  operator o { on in { out = envelope } }\n  o.out -> .feed\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_PORT_DIRECTION)
            .expect("an input boundary port cannot drain the graph");
        assert!(diagnostic.message().contains("boundary edge target"));
    }

    #[test]
    fn wiring_cycles_are_refused() {
        let source = "graph g : App {\n  operator a { on in { out = envelope } }\n  operator b { on in { out = envelope } }\n  a -> b\n  b -> a\n}\n";
        let (_, bag) = check_source(source);
        let diagnostic = bag
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == E_GRAPH_CYCLE)
            .expect("the cycle must refuse");
        assert!(diagnostic.message().contains("DAG"));
        let chain = "graph g : App {\n  operator a { on in { out = envelope } }\n  operator b { on in { out = envelope } }\n  . -> a -> b -> .\n}\n";
        let (_, bag) = check_source(chain);
        assert!(!diagnostic_codes(&bag).contains(&E_GRAPH_CYCLE));
    }

    #[test]
    fn guarded_constructs_defer_explicitly() {
        let source = "graph g : App {\n  partition user { key = meta.app.id }\n  operator o {\n    on (a, b) { out = envelope }\n  }\n  . -> o -> .\n}\n";
        let (_, bag) = check_source(source);
        let constructs: Vec<&str> = bag
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == E_CHECK_DEFERRED)
            .map(|diagnostic| diagnostic.fields()[0].1.as_str())
            .collect();
        assert!(constructs.contains(&"a partition"));
        assert!(constructs.contains(&"a join handler"));
    }
}
