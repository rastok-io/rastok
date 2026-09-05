//! Static endpoint resolution, case routes and destination validation.

use super::*;

impl Checker<'_, '_> {
    /// `ack` deliberately has no graph-response half. Refuse that spelling at
    /// the source edge rather than letting Registry-free plan validation turn
    /// it into an internal-plan diagnostic after cycle analysis has correctly
    /// split the inbound Call boundary.
    pub(super) fn reject_ack_response_edge(
        &mut self,
        graph: &Graph,
        edge: &Edge,
        span: Span,
    ) -> bool {
        let EdgeTarget::Port(target) = &edge.to else {
            return false;
        };
        let EndpointOwner::Node(path) = &target.owner else {
            return false;
        };
        let Some(target) = graph.nodes.iter().find(|node| &node.path == path) else {
            return false;
        };
        let NodeSemantics::Connector { attributes, .. } = &target.semantics else {
            return false;
        };
        let ack = target.connector_contract_id().is_some_and(|contract| {
            contract.as_str() == crate::connector::HTTP_SERVER_CONNECTOR_ID
        }) && attributes.iter().any(|attribute| {
            attribute.name == "mode"
                && matches!(&attribute.value, AttrValue::Word(word) if word == "ack")
        });
        if !ack {
            return false;
        }
        self.bag.push(
            Diagnostic::new(
                E_CONTRACT_ATTR,
                Phase::Check,
                "`HttpServer(mode = ack)` has no graph-response input; select `mode = respond` for a retained API response",
            )
            .with_span(span)
            .with_field("connector", path.to_string())
            .with_field("mode", "ack"),
        );
        true
    }

    /// `next` is consumed only by the temporal Loop manager. Letting a source
    /// edge mention it would turn recurrence back into an ordinary spatial
    /// route and make the DAG claim false.
    pub(super) fn reject_loop_next_edge(&mut self, graph: &Graph, edge: &Edge, span: Span) -> bool {
        fn graph_at<'graph>(
            graph: &'graph Graph,
            path: &crate::plan::OwnerPath,
        ) -> Option<&'graph Graph> {
            if &graph.path == path {
                return Some(graph);
            }
            graph
                .graphs
                .iter()
                .find_map(|nested| graph_at(nested, path))
        }
        let is_next = |endpoint: &Endpoint| {
            let EndpointOwner::Graph(path) = &endpoint.owner else {
                return false;
            };
            endpoint.port.name() == "next"
                && graph_at(graph, path)
                    .is_some_and(|owner| owner.extensions.contains(&node::Extension::Loop))
        };
        let rejected =
            is_next(&edge.from) || matches!(&edge.to, EdgeTarget::Port(target) if is_next(target));
        if rejected {
            self.loop_refusal(
                span,
                "Loop.next is a temporal control port and cannot appear in a spatial edge",
            );
        }
        rejected
    }

    /// Every output requiring a wire either reaches an edge or is discarded
    /// in writing (`-> nothing`); a port that just leaks gets flagged.
    /// Outputs whose resolved policy is `RootSink` are exempt. The source
    /// convention commonly gives that policy to `err`, but the warning
    /// decision itself never inspects the spelling.
    pub(super) fn warn_unwired_outputs(&mut self, graph: &Graph) {
        for node in &graph.nodes {
            // Emission inference normally materializes the row already; sweep
            // the bodies as a defensive union so diagnostics never depend on
            // that construction order.
            let mut outputs: Vec<node::Port> = node
                .ports
                .iter()
                .filter(|decl| decl.direction == Direction::Output)
                .map(|decl| decl.port.clone())
                .collect();
            for handler in &node.handlers {
                let mut statements = Vec::new();
                collect_hir_statements(&handler.dispatch, &mut statements);
                for statement in statements {
                    if let Statement::Emit { port, .. } = statement
                        && !outputs.contains(port)
                    {
                        outputs.push(port.clone());
                    }
                }
            }
            for port in &outputs {
                if node
                    .ports
                    .iter()
                    .find(|decl| decl.port == *port)
                    .is_some_and(|decl| decl.unwired == crate::plan::UnwiredPolicy::RootSink)
                {
                    continue;
                }
                let wired = graph.edges.iter().any(|edge| {
                    matches!(&edge.from.owner, EndpointOwner::Node(path) if path == &node.path)
                        && edge.from.port == *port
                }) || graph.routes.iter().any(|route| {
                    matches!(&route.source.owner, EndpointOwner::Node(path) if path == &node.path)
                        && route.source.port == *port
                }) || graph.handlers.iter().any(|handler| {
                    matches!(
                        &handler.activation,
                        node::HandlerActivation::ChildPort { child, port: observed }
                            if child == &node.path && observed == port
                    )
                });
                if !wired {
                    self.bag.push(
                        Diagnostic::new(
                            W_UNWIRED_OUTPUT,
                            Phase::Check,
                            format!(
                                "`{}.{port}` is emitted but wired nowhere; \
                                 route it or discard it with `-> nothing`",
                                node.path
                            ),
                        )
                        .with_field("node", node.path.to_string())
                        .with_field("port", port.name().to_string()),
                    );
                }
            }
        }
    }

    /// Every boundary convention actually used by an edge becomes an
    /// ordinary table row. Syntax may still abbreviate `.`, but the plan and
    /// VM never infer a port from that spelling.
    pub(super) fn materialize_boundary_ports(&mut self, graph: &mut Graph) {
        let mut used = Vec::new();
        for edge in &graph.edges {
            if matches!(&edge.from.owner, EndpointOwner::Graph(path) if path == &graph.path) {
                used.push((edge.from.port.clone(), Direction::Input));
            }
            if let EdgeTarget::Port(target) = &edge.to
                && matches!(&target.owner, EndpointOwner::Graph(path) if path == &graph.path)
            {
                used.push((target.port.clone(), Direction::Output));
            }
        }
        for (port, direction) in used {
            if graph.boundary.iter().any(|decl| decl.port == port) {
                continue;
            }
            graph.boundary.push(crate::plan::PortDecl {
                port,
                direction,
                unwired: crate::plan::UnwiredPolicy::RequireWire,
                ty: None,
                annotations: AnnotationTable::empty(),
            });
        }
    }

    /// Conventional node ports are source sugar only. If an edge uses one
    /// that no body declaration/inference has materialized, add the ordinary
    /// row before HIR leaves the checker. Later layers never recover a port
    /// from its spelling.
    pub(super) fn materialize_node_edge_ports(&mut self, graph: &mut Graph) {
        let mut used = Vec::new();
        for edge in &graph.edges {
            if let EndpointOwner::Node(path) = &edge.from.owner {
                used.push((path.clone(), edge.from.port.clone(), Direction::Output));
            }
            if let EdgeTarget::Port(target) = &edge.to
                && let EndpointOwner::Node(path) = &target.owner
            {
                used.push((path.clone(), target.port.clone(), Direction::Input));
            }
        }
        for (path, port, direction) in used {
            let Some(node) = graph.nodes.iter_mut().find(|node| node.path == path) else {
                continue;
            };
            if node.ports.iter().any(|decl| decl.port == port) {
                continue;
            }
            node.ports.push(crate::plan::PortDecl {
                unwired: if port.name() == "err" {
                    crate::plan::UnwiredPolicy::RootSink
                } else {
                    crate::plan::UnwiredPolicy::RequireWire
                },
                port,
                direction,
                ty: None,
                annotations: AnnotationTable::empty(),
            });
        }
    }

    /// Wiring is a DAG (`grammar.md`): a cycle among nodes refuses.
    pub(super) fn reject_cycles(&mut self, graph: &Graph, span: Span) {
        let index_of = |path: &node::Path| {
            graph
                .nodes
                .iter()
                .position(|candidate| &candidate.path == path)
        };
        let mut successors = vec![Vec::new(); graph.nodes.len()];
        for edge in &graph.edges {
            if let (EndpointOwner::Node(from), EdgeTarget::Port(to)) = (&edge.from.owner, &edge.to)
                && let EndpointOwner::Node(to) = &to.owner
                && let (Some(from), Some(to)) = (
                    from.segments().first().and_then(|_| index_of(from)),
                    index_of(to),
                )
            {
                let target_is_inbound_boundary = matches!(
                    &graph.nodes[to].semantics,
                    NodeSemantics::Connector {
                        roles,
                        exchange,
                        ..
                    } if roles.has_inbound()
                        && (matches!(exchange, crate::connector::ConnectorExchange::Session)
                            || (!roles.has_outbound()
                                && matches!(exchange, crate::connector::ConnectorExchange::Call)))
                );
                if target_is_inbound_boundary {
                    // A response/message input and a world-origin output are
                    // independent halves of one external boundary. There is
                    // no hidden `in -> out` execution edge through this Node.
                    continue;
                }
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

    pub(super) fn edge(&mut self, decl: &EdgeDecl, graph: &Graph) -> Option<ResolvedRouting> {
        if decl.bidirectional {
            self.deferred("a bidirectional edge", decl.span);
            return None;
        }
        if decl.guard.is_some() {
            self.bag.push(
                Diagnostic::new(
                    E_WIRING_MIGRATION,
                    Phase::Check,
                    "edge `when` was removed; route a closed value with `-> match { ... }` or decide in an Operator",
                )
                .with_span(decl.span)
                .with_field("construct", "when"),
            );
            return None;
        }
        if decl.config.is_some() {
            self.deferred("an edge configuration block", decl.span);
            return None;
        }
        let [first, ..] = decl.endpoints.as_slice() else {
            return None;
        };
        let mut rules = Vec::new();
        let mut edges = Vec::new();
        if let EdgeEndpoint::SourceSet(refs) = first {
            if decl.endpoints.len() != 2 {
                self.bag.push(
                    Diagnostic::new(
                        E_WIRING_MIGRATION,
                        Phase::Check,
                        "a source set is an mpsc fan-in declaration and cannot continue as a path chain",
                    )
                    .with_span(decl.span)
                    .with_field("construct", "source set"),
                );
                return None;
            }
            if refs.len() < 2 {
                self.bag.push(
                    Diagnostic::new(
                        E_ROUTE_DUPLICATE,
                        Phase::Check,
                        "a source set requires at least two distinct sources",
                    )
                    .with_span(decl.span)
                    .with_field("source", "source set"),
                );
                return None;
            }
            let mut sources = BTreeSet::new();
            for reference in refs {
                let source = self.port_ref(reference, graph, true)?;
                if !sources.insert(source.clone()) {
                    self.bag.push(
                        Diagnostic::new(
                            E_ROUTE_DUPLICATE,
                            Phase::Check,
                            "a source appears twice in one source set",
                        )
                        .with_span(reference.span)
                        .with_field("source", endpoint_display(&source)),
                    );
                    return None;
                }
            }
            for source in sources {
                let (rule, physical) =
                    self.route_rule(source, &decl.endpoints[1], graph, decl.span)?;
                rules.push(rule);
                edges.extend(physical);
            }
        } else {
            for pair in decl.endpoints.windows(2) {
                let EdgeEndpoint::Ref(reference) = &pair[0] else {
                    self.bag.push(
                        Diagnostic::new(
                            E_WIRING_MIGRATION,
                            Phase::Check,
                            "a route set, case match, or `nothing` is terminal and cannot become a source",
                        )
                        .with_span(decl.span)
                        .with_field("construct", "route chain"),
                    );
                    return None;
                };
                let source = self.port_ref(reference, graph, true)?;
                let (rule, physical) = self.route_rule(source, &pair[1], graph, decl.span)?;
                rules.push(rule);
                edges.extend(physical);
            }
        }
        for (index, rule) in rules.iter().enumerate() {
            if let Some((code, detail, case)) = rules[..index]
                .iter()
                .find_map(|existing| route_conflict(existing, rule))
            {
                self.bag.push(
                    Diagnostic::new(code, Phase::Check, detail)
                        .with_span(decl.span)
                        .with_field("source", endpoint_display(&rule.source))
                        .with_field("case", case),
                );
                return None;
            }
        }
        let mut seen_edges = BTreeSet::new();
        edges.retain(|edge| seen_edges.insert(edge_display(edge)));
        Some(ResolvedRouting { edges, rules })
    }

    pub(super) fn route_rule(
        &mut self,
        source: Endpoint,
        endpoint: &EdgeEndpoint,
        graph: &Graph,
        span: Span,
    ) -> Option<(RouteRule, Vec<Edge>)> {
        if let EdgeEndpoint::Match(route_match) = endpoint {
            return self.route_match(source, route_match, graph);
        }
        let destinations = self.route_destinations(endpoint, graph, span)?;
        self.validate_route_types(&source, &destinations, graph, span, None)?;
        let edges = physical_edges(&source, &destinations);
        Some((
            RouteRule {
                source,
                selection: RouteSelection::All(destinations),
            },
            edges,
        ))
    }

    pub(super) fn route_destinations(
        &mut self,
        endpoint: &EdgeEndpoint,
        graph: &Graph,
        span: Span,
    ) -> Option<Vec<RouteDestination>> {
        let mut destinations = match endpoint {
            EdgeEndpoint::Ref(reference) => {
                vec![self.route_destination_ref(reference, graph)?]
            }
            EdgeEndpoint::DestinationSet(items) => {
                if items.len() < 2 {
                    self.bag.push(
                        Diagnostic::new(
                            E_ROUTE_DUPLICATE,
                            Phase::Check,
                            "a destination set requires at least two distinct destinations",
                        )
                        .with_span(span)
                        .with_field("source", "destination set"),
                    );
                    return None;
                }
                let mut checked = Vec::with_capacity(items.len());
                for item in items {
                    checked.push(match item {
                        SynEdgeDestination::Ref(reference) => {
                            self.route_destination_ref(reference, graph)?
                        }
                        SynEdgeDestination::Nothing(_) => RouteDestination::Discard,
                    });
                }
                checked
            }
            EdgeEndpoint::Nothing(_) => vec![RouteDestination::Discard],
            EdgeEndpoint::SourceSet(_) => {
                self.bag.push(
                    Diagnostic::new(
                        E_WIRING_MIGRATION,
                        Phase::Check,
                        "a source set is legal only before the first arrow",
                    )
                    .with_span(span)
                    .with_field("construct", "source set"),
                );
                return None;
            }
            EdgeEndpoint::Match(_) => unreachable!("route matches are handled by route_rule"),
        };
        destinations.sort();
        if destinations.windows(2).any(|pair| pair[0] == pair[1]) {
            self.bag.push(
                Diagnostic::new(
                    E_ROUTE_DUPLICATE,
                    Phase::Check,
                    "a destination appears twice in one route set",
                )
                .with_span(span)
                .with_field("source", "destination set"),
            );
            return None;
        }
        if destinations.len() > 1 && destinations.contains(&RouteDestination::Discard) {
            self.bag.push(
                Diagnostic::new(
                    E_ROUTE_MIXED,
                    Phase::Check,
                    "`nothing` cannot coexist with a real destination for the same source case",
                )
                .with_span(span)
                .with_field("case", "all"),
            );
            return None;
        }
        Some(destinations)
    }

    pub(super) fn route_destination_ref(
        &mut self,
        reference: &PortRef,
        graph: &Graph,
    ) -> Option<RouteDestination> {
        if reference
            .segments
            .iter()
            .any(|segment| segment.text == "Loop")
        {
            return self
                .loop_command_target(reference, graph)
                .map(RouteDestination::LoopCommand);
        }
        self.port_ref(reference, graph, false)
            .map(RouteDestination::Port)
    }

    pub(super) fn loop_command_target(
        &mut self,
        reference: &PortRef,
        graph: &Graph,
    ) -> Option<LoopCommandTarget> {
        let (owner_name, command) = match reference.segments.as_slice() {
            [extension, command] if extension.text == "Loop" => (None, command),
            [owner, extension, command] if extension.text == "Loop" => (Some(owner), command),
            _ => {
                self.loop_refusal(
                    reference.span,
                    "a Loop manager destination is `Loop.next|again|done` or `child.Loop.next|again|done`",
                );
                return None;
            }
        };
        let kind = match command.text.as_str() {
            "next" => LoopCommandKind::Next,
            "again" => LoopCommandKind::Again,
            "done" => LoopCommandKind::Done,
            _ => {
                self.loop_refusal(
                    command.span,
                    "a Loop manager command is exactly `next`, `again`, or `done`",
                );
                return None;
            }
        };
        let (owner, owns_loop) = match owner_name {
            None => (
                EndpointOwner::Graph(graph.path.clone()),
                graph.extensions.contains(&node::Extension::Loop),
            ),
            Some(name) => {
                if let Some(target) = graph.nodes.iter().find(|candidate| {
                    candidate.path.segments().last().map(String::as_str) == Some(name.text.as_str())
                }) {
                    (
                        EndpointOwner::Node(target.path.clone()),
                        target.extensions.contains(&node::Extension::Loop),
                    )
                } else if let Some(target) = graph
                    .graphs
                    .iter()
                    .find(|candidate| candidate.name == name.text)
                {
                    (
                        EndpointOwner::Graph(target.path.clone()),
                        target.extensions.contains(&node::Extension::Loop),
                    )
                } else {
                    self.bag.push(
                        Diagnostic::new(
                            E_RESOLVE_UNKNOWN,
                            Phase::Resolve,
                            format!("`{}` names no child Node in this Graph", name.text),
                        )
                        .with_span(name.span)
                        .with_field("name", name.text.clone()),
                    );
                    return None;
                }
            }
        };
        if !owns_loop {
            self.loop_refusal(
                reference.span,
                "a `Loop.*` destination requires that exact owner to declare `+ Loop`",
            );
            return None;
        }
        Some(LoopCommandTarget { owner, kind })
    }

    pub(super) fn route_match(
        &mut self,
        source: Endpoint,
        route_match: &EdgeMatch,
        graph: &Graph,
    ) -> Option<(RouteRule, Vec<Edge>)> {
        let source_ty = endpoint_pinned_type(&source, graph);
        let Some((source_ty, domain)) = source_ty
            .as_ref()
            .and_then(|ty| closed_case_domain(ty).map(|domain| (ty, domain)))
        else {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_NOT_CLOSED,
                    Phase::Check,
                    "route `match` requires a pinned tagged union, enum, or bool source",
                )
                .with_span(route_match.span)
                .with_field("source", endpoint_display(&source))
                .with_field(
                    "type",
                    source_ty.map_or_else(|| "unresolved".to_string(), |ty| ty.to_string()),
                ),
            );
            return None;
        };
        let domain_set: BTreeSet<_> = domain.iter().cloned().collect();
        let mut selected = BTreeSet::new();
        let mut explicit = Vec::new();
        let mut fallback = None;
        for arm in &route_match.arms {
            match &arm.cases {
                EdgeCaseSet::Cases(cases) => {
                    let mut names = Vec::new();
                    for case in cases {
                        if !domain_set.contains(&case.text) {
                            self.bag.push(
                                Diagnostic::new(
                                    E_MATCH_NONEXHAUSTIVE,
                                    Phase::Check,
                                    format!("`{}` is not a productible source case", case.text),
                                )
                                .with_span(case.span)
                                .with_field("source", endpoint_display(&source))
                                .with_field("case", case.text.clone()),
                            );
                            return None;
                        }
                        if !selected.insert(case.text.clone()) {
                            self.bag.push(
                                Diagnostic::new(
                                    E_MATCH_OVERLAP,
                                    Phase::Check,
                                    format!("route case `{}` appears in two arms", case.text),
                                )
                                .with_span(case.span)
                                .with_field("source", endpoint_display(&source))
                                .with_field("case", case.text.clone()),
                            );
                            return None;
                        }
                        names.push(case.text.clone());
                    }
                    names.sort();
                    names.dedup();
                    explicit.push((names, arm));
                }
                EdgeCaseSet::Fallback(span) => {
                    if fallback.replace(arm).is_some() {
                        self.bag.push(
                            Diagnostic::new(
                                E_MATCH_OVERLAP,
                                Phase::Check,
                                "a route match has only one `_` remainder arm",
                            )
                            .with_span(*span)
                            .with_field("source", endpoint_display(&source))
                            .with_field("case", "_"),
                        );
                        return None;
                    }
                }
            }
        }
        let remaining: Vec<_> = domain
            .iter()
            .filter(|case| !selected.contains(*case))
            .cloned()
            .collect();
        if fallback.is_none() && !remaining.is_empty() {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_NONEXHAUSTIVE,
                    Phase::Check,
                    format!(
                        "route match leaves productible case(s) {} uncovered",
                        remaining.join(", ")
                    ),
                )
                .with_span(route_match.span)
                .with_field("source", endpoint_display(&source))
                .with_field("case", remaining.join("|")),
            );
            return None;
        }
        if fallback.is_some() && remaining.is_empty() {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_OVERLAP,
                    Phase::Check,
                    "the `_` route arm covers no remaining productible case",
                )
                .with_span(route_match.span)
                .with_field("source", endpoint_display(&source))
                .with_field("case", "_"),
            );
            return None;
        }
        let mut arms = Vec::new();
        for (cases, arm) in explicit {
            let destinations = self.route_destinations(&arm.destination, graph, arm.span)?;
            self.validate_route_types(&source, &destinations, graph, arm.span, Some(&cases))?;
            arms.push(RouteArm {
                cases,
                destinations,
            });
        }
        if let Some(arm) = fallback {
            let destinations = self.route_destinations(&arm.destination, graph, arm.span)?;
            self.validate_route_types(&source, &destinations, graph, arm.span, Some(&remaining))?;
            arms.push(RouteArm {
                cases: remaining,
                destinations,
            });
        }
        arms.sort_by(|left, right| left.cases.cmp(&right.cases));
        self.verify_case_partition(
            source_ty,
            arms.iter().map(|arm| arm.cases.as_slice()),
            route_match.span,
            &endpoint_display(&source),
        )?;
        let edges = arms
            .iter()
            .flat_map(|arm| physical_edges(&source, &arm.destinations))
            .collect();
        Some((
            RouteRule {
                source,
                selection: RouteSelection::Cases(arms),
            },
            edges,
        ))
    }

    pub(super) fn validate_route_types(
        &mut self,
        source: &Endpoint,
        destinations: &[RouteDestination],
        graph: &Graph,
        span: Span,
        cases: Option<&[String]>,
    ) -> Option<()> {
        let source_ty = endpoint_pinned_type(source, graph);
        let selected_ty = match source_ty.as_ref() {
            Some(ty) => match narrowed_route_type(ty, cases.unwrap_or(&[])) {
                Ok(selected) => Some(selected),
                Err(error) => {
                    self.type_constructor_refusal(span, error);
                    return None;
                }
            },
            None => None,
        };
        for destination in destinations {
            let target_ty = match destination {
                RouteDestination::Port(target) => endpoint_pinned_type(target, graph),
                RouteDestination::LoopCommand(target) if target.kind == LoopCommandKind::Next => {
                    loop_owner_entry_type(&target.owner, graph)
                }
                RouteDestination::Discard | RouteDestination::LoopCommand(_) => None,
            };
            if let (Some(source_ty), Some(target_ty)) = (&selected_ty, target_ty)
                && !source_ty.is_assignable_to(&target_ty)
            {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_TYPE,
                        Phase::Check,
                        format!(
                            "this route connects {source_ty} to {target_ty}; pinned ends must agree"
                        ),
                    )
                    .with_span(span)
                    .with_field("port", route_destination_display(destination))
                    .with_field("expected", target_ty.to_string())
                    .with_field("found", source_ty.to_string()),
                );
                return None;
            }
        }
        Some(())
    }

    /// The direction carried by a node's materialized port table.
    pub(super) fn node_port_direction(&self, target: &Node, port_name: &str) -> Option<Direction> {
        target
            .ports
            .iter()
            .find(|decl| decl.port.name() == port_name)
            .map(|decl| decl.direction)
    }

    pub(super) fn port_ref(
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
                    let Some(decl) = graph
                        .boundary
                        .iter()
                        .find(|decl| decl.port.name() == name.text)
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
                    if decl.direction != expected {
                        let required = if is_source {
                            "a boundary edge source must be an input of the graph"
                        } else {
                            "a boundary edge target must be an output of the graph"
                        };
                        self.wrong_direction(name.span, &name.text, decl.direction, required);
                        return None;
                    }
                    name.text.as_str()
                }
            };
            return node::Port::new(port_name).ok().map(|port| Endpoint {
                owner: EndpointOwner::Graph(graph.path.clone()),
                port,
            });
        }
        if reference
            .segments
            .iter()
            .any(|segment| segment.text == "Loop")
        {
            return self.loop_expired_source(reference, graph);
        }
        let node_name = &reference.segments[0];
        let target_node = graph.nodes.iter().find(|candidate| {
            candidate
                .path
                .segments()
                .last()
                .is_some_and(|part| part == &node_name.text)
        });
        let target_graph = graph
            .graphs
            .iter()
            .find(|candidate| candidate.name == node_name.text);
        let own_boundary = (reference.segments.len() == 1)
            .then(|| {
                graph
                    .boundary
                    .iter()
                    .find(|port| port.port.name() == node_name.text)
                    .map(|port| port.direction)
                    .or(match node_name.text.as_str() {
                        "in" => Some(Direction::Input),
                        "out" => Some(Direction::Output),
                        _ => None,
                    })
            })
            .flatten();
        if own_boundary.is_some() && (target_node.is_some() || target_graph.is_some()) {
            self.bag.push(
                Diagnostic::new(
                    E_NAME_CLASH,
                    Phase::Resolve,
                    format!(
                        "`{}` names both a Graph port and a child; ports and children share one namespace",
                        node_name.text
                    ),
                )
                .with_span(node_name.span)
                .with_field("name", node_name.text.clone()),
            );
            return None;
        }
        if let Some(direction) = own_boundary {
            let expected = if is_source {
                Direction::Input
            } else {
                Direction::Output
            };
            if direction != expected {
                let required = if is_source {
                    "inside a Graph, its input port is a source"
                } else {
                    "inside a Graph, its output port is a destination"
                };
                self.wrong_direction(reference.span, &node_name.text, direction, required);
                return None;
            }
            return node::Port::new(node_name.text.clone())
                .ok()
                .map(|port| Endpoint {
                    owner: EndpointOwner::Graph(graph.path.clone()),
                    port,
                });
        }
        if target_node.is_none() && target_graph.is_none() {
            self.bag.push(
                Diagnostic::new(
                    E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!(
                        "`{}` names neither a Graph port nor a child Node in this Graph",
                        node_name.text
                    ),
                )
                .with_span(node_name.span)
                .with_field("name", node_name.text.clone()),
            );
            return None;
        }
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
        // B7 makes the conventional terminal output executable beside the
        // ordinary in/out/err ports. Further contract-specific ports remain
        // transported-but-deferred until their Driver slice exists.
        if target_node.is_some_and(|target| target.connector_contract_id().is_some())
            && !matches!(port_name, "in" | "out" | "err" | "exit")
        {
            self.deferred("a connector edge beyond the default trio", reference.span);
            return None;
        }
        let direction = if let Some(target) = target_node {
            self.node_port_direction(target, port_name).or_else(|| {
                if target.connector_contract_id().is_some() {
                    return None;
                }
                match port_name {
                    "in" => Some(Direction::Input),
                    "out" | "err" => Some(Direction::Output),
                    _ => None,
                }
            })
        } else {
            target_graph.and_then(|target| {
                target
                    .boundary
                    .iter()
                    .find(|decl| decl.port.name() == port_name)
                    .map(|decl| decl.direction)
                    .or(match port_name {
                        "in" => Some(Direction::Input),
                        "out" | "err" => Some(Direction::Output),
                        _ => None,
                    })
            })
        };
        let Some(direction) = direction else {
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
        let owner = target_node.map_or_else(
            || target_graph.map(|target| EndpointOwner::Graph(target.path.clone())),
            |target| Some(EndpointOwner::Node(target.path.clone())),
        )?;
        node::Port::new(port_name)
            .ok()
            .map(|port| Endpoint { owner, port })
    }

    pub(super) fn loop_expired_source(
        &mut self,
        reference: &PortRef,
        graph: &Graph,
    ) -> Option<Endpoint> {
        let [owner_name, extension, endpoint] = reference.segments.as_slice() else {
            self.loop_refusal(
                reference.span,
                "only a child Loop's `child.Loop.expired` output is a spatial edge source",
            );
            return None;
        };
        if extension.text != "Loop" || endpoint.text != "expired" {
            self.loop_refusal(
                reference.span,
                "Loop.next, Loop.again, and Loop.done are manager destinations; the only Loop edge source is `child.Loop.expired`",
            );
            return None;
        }
        let owner = if let Some(target) = graph.nodes.iter().find(|candidate| {
            candidate.path.segments().last().map(String::as_str) == Some(owner_name.text.as_str())
        }) {
            if target.loop_spec.is_none() {
                self.loop_refusal(
                    reference.span,
                    "`Loop.expired` requires that exact child to own a valid `+ Loop` Extension",
                );
                return None;
            }
            EndpointOwner::Node(target.path.clone())
        } else if let Some(target) = graph
            .graphs
            .iter()
            .find(|candidate| candidate.name == owner_name.text)
        {
            if target.loop_spec.is_none() {
                self.loop_refusal(
                    reference.span,
                    "`Loop.expired` requires that exact child to own a valid `+ Loop` Extension",
                );
                return None;
            }
            EndpointOwner::Graph(target.path.clone())
        } else {
            self.bag.push(
                Diagnostic::new(
                    E_RESOLVE_UNKNOWN,
                    Phase::Resolve,
                    format!("`{}` names no child Node in this Graph", owner_name.text),
                )
                .with_span(owner_name.span)
                .with_field("name", owner_name.text.clone()),
            );
            return None;
        };
        Some(Endpoint {
            owner,
            port: node::Port::new("Loop.expired").ok()?,
        })
    }
}

/// The pinned type of an edge endpoint, when its port declares one:
/// a node's table entry (the connector's effective out pin
/// included), or a declared boundary port. An unannotated conventional or
/// named port answers `None` here and is closed by validated-plan inference.
pub(super) fn endpoint_pinned_type(endpoint: &Endpoint, graph: &Graph) -> Option<DataType> {
    if endpoint.port.name() == "Loop.expired" {
        let entry = loop_owner_entry_type(&endpoint.owner, graph)?;
        return node::loop_expired_type(entry).ok();
    }
    match &endpoint.owner {
        EndpointOwner::Graph(path) => graph_at_path(graph, path)?
            .boundary
            .iter()
            .find(|decl| decl.port == endpoint.port)
            .and_then(|decl| decl.ty.clone()),
        EndpointOwner::Node(path) => {
            node_at_path(graph, path).and_then(|node| node_port_pin(node, &endpoint.port))
        }
    }
}

/// Stable human spelling for checked endpoints. Diagnostics use the checked
/// owner rather than reconstructing source sugar, so a default port and its
/// explicit spelling report the same location.
pub(super) fn endpoint_display(endpoint: &Endpoint) -> String {
    match &endpoint.owner {
        EndpointOwner::Node(path) => format!("{path}.{}", endpoint.port),
        EndpointOwner::Graph(path) if path.is_root() => format!(".{}", endpoint.port),
        EndpointOwner::Graph(path) => format!("{path}.{}", endpoint.port),
    }
}

pub(super) fn route_destination_display(destination: &RouteDestination) -> String {
    match destination {
        RouteDestination::Port(endpoint) => endpoint_display(endpoint),
        RouteDestination::Discard => String::from("nothing"),
        RouteDestination::LoopCommand(target) => {
            let owner = match &target.owner {
                EndpointOwner::Node(path) => format!("{path}."),
                EndpointOwner::Graph(path) if path.is_root() => String::new(),
                EndpointOwner::Graph(path) => format!("{path}."),
            };
            let command = match target.kind {
                LoopCommandKind::Next => "next",
                LoopCommandKind::Again => "again",
                LoopCommandKind::Done => "done",
            };
            format!("{owner}Loop.{command}")
        }
    }
}

/// The spatial projection used by the v15 cycle/type machinery while R3
/// retains richer routing in HIR. Manager commands deliberately project to no
/// edge: temporal recursion must never enter the spatial DAG.
pub(super) fn physical_edges(source: &Endpoint, destinations: &[RouteDestination]) -> Vec<Edge> {
    destinations
        .iter()
        .filter_map(|destination| {
            let to = match destination {
                RouteDestination::Port(endpoint) => EdgeTarget::Port(endpoint.clone()),
                RouteDestination::Discard => EdgeTarget::Discard,
                RouteDestination::LoopCommand(_) => return None,
            };
            Some(Edge {
                from: source.clone(),
                to,
            })
        })
        .collect()
}

/// Narrows a closed source type to the exact cases selected by one route arm.
/// Constructors are used again so the HIR cannot contain an ad-hoc subset that
/// the independent plan validator could not reconstruct.
pub(super) fn narrowed_route_type(ty: &DataType, cases: &[String]) -> Result<DataType, TypeError> {
    if cases.is_empty() {
        return Ok(ty.clone());
    }
    match ty {
        DataType::Enum(_) => EnumType::new(cases.to_vec()).map(DataType::Enum),
        DataType::TaggedUnion(union) => {
            let selected = union
                .cases()
                .iter()
                .filter(|case| cases.iter().any(|name| name == case.name()))
                .cloned()
                .collect();
            TaggedUnionType::new(selected).map(DataType::TaggedUnion)
        }
        // A boolean arm still carries the ordinary bool type; literal-bool
        // refinement is control information, not a new logical DataType.
        DataType::Bool => Ok(DataType::Bool),
        _ => Ok(ty.clone()),
    }
}

pub(super) fn route_selection_arms(
    selection: &RouteSelection,
) -> Vec<(Option<&[String]>, &[RouteDestination])> {
    match selection {
        RouteSelection::All(destinations) => vec![(None, destinations)],
        RouteSelection::Cases(arms) => arms
            .iter()
            .map(|arm| (Some(arm.cases.as_slice()), arm.destinations.as_slice()))
            .collect(),
    }
}

pub(super) fn route_case_overlap(left: Option<&[String]>, right: Option<&[String]>) -> Vec<String> {
    match (left, right) {
        (None, None) => vec![String::from("all")],
        (None, Some(cases)) | (Some(cases), None) => cases.to_vec(),
        (Some(left), Some(right)) => left
            .iter()
            .filter(|case| right.contains(case))
            .cloned()
            .collect(),
    }
}

/// Detects conflicts in additive declarations for one source. `nothing` is an
/// exclusive disposition even across declarations; real destinations remain
/// additive, provided the exact same destination is not selected twice.
pub(super) fn route_conflict(
    existing: &RouteRule,
    candidate: &RouteRule,
) -> Option<(Code, String, String)> {
    if existing.source != candidate.source {
        return None;
    }
    if matches!(existing.selection, RouteSelection::Cases(_))
        && matches!(candidate.selection, RouteSelection::Cases(_))
    {
        return Some((
            E_MATCH_DUPLICATE,
            String::from("a source may have at most one closed route `match`"),
            String::from("all"),
        ));
    }
    for (left_cases, left_destinations) in route_selection_arms(&existing.selection) {
        for (right_cases, right_destinations) in route_selection_arms(&candidate.selection) {
            let overlap = route_case_overlap(left_cases, right_cases);
            if overlap.is_empty() {
                continue;
            }
            let case = overlap.join("|");
            let left_discards = left_destinations.contains(&RouteDestination::Discard);
            let right_discards = right_destinations.contains(&RouteDestination::Discard);
            let left_real = left_destinations
                .iter()
                .any(|destination| *destination != RouteDestination::Discard);
            let right_real = right_destinations
                .iter()
                .any(|destination| *destination != RouteDestination::Discard);
            if (left_discards && right_real) || (right_discards && left_real) {
                return Some((
                    E_ROUTE_MIXED,
                    String::from(
                        "`nothing` cannot coexist with a real destination for the same source case",
                    ),
                    case,
                ));
            }
            if let Some(duplicate) = left_destinations
                .iter()
                .find(|destination| right_destinations.contains(destination))
            {
                return Some((
                    E_ROUTE_DUPLICATE,
                    format!(
                        "destination `{}` is selected twice for the same source case",
                        route_destination_display(duplicate)
                    ),
                    case,
                ));
            }
        }
    }
    None
}

/// The canonical text of one checked edge — the human face of
/// E-DUP-EDGE, mirroring the plan's rendering.
pub(super) fn edge_display(edge: &Edge) -> String {
    let to = match &edge.to {
        EdgeTarget::Port(endpoint) => endpoint_display(endpoint),
        EdgeTarget::Discard => String::from("!discard"),
    };
    format!("{}>{to}", endpoint_display(&edge.from))
}
