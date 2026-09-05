//! Loop host policies, state bindings and the retained v15 proof bridge.

use super::*;

impl Checker<'_, '_> {
    /// Materializes the Loop-owned half of a Graph boundary after the source
    /// declarations are known. `in` remains the user-owned state pin;
    /// `next` and `expired` have exactly one historical Extension authority. Ordinary
    /// outputs keep their domain names and all terminate a Loop run.
    pub(super) fn contribute_loop_boundary(&mut self, graph: &mut Graph, span: Span) -> bool {
        let input = graph.boundary.iter().find(|port| port.port.name() == "in");
        let Some(state_type) = input.and_then(|port| {
            (port.direction == Direction::Input)
                .then(|| port.ty.clone())
                .flatten()
        }) else {
            self.loop_refusal(
                span,
                "a `: Loop` Graph declares typed `input in` as its iteration-state carrier",
            );
            return false;
        };
        if let Some(extra) = graph
            .boundary
            .iter()
            .find(|port| port.direction == Direction::Input && port.port.name() != "in")
        {
            self.loop_refusal(
                span,
                &format!(
                    "a `: Loop` Graph has one iteration input `in`; `{}` cannot be a second input",
                    extra.port
                ),
            );
            return false;
        }
        if let Some(collision) = graph
            .boundary
            .iter()
            .find(|port| matches!(port.port.name(), "next" | "expired"))
        {
            self.loop_refusal(
                span,
                &format!(
                    "`{}` is owned by the Loop Extension and cannot be declared explicitly",
                    collision.port
                ),
            );
            return false;
        }
        let Ok(next) = node::Port::new("next") else {
            return false;
        };
        let Ok(expired) = node::Port::new("expired") else {
            return false;
        };
        let Ok(expired_type) = node::loop_expired_type(state_type.clone()) else {
            self.loop_refusal(span, "the LoopExpired carrier could not be represented");
            return false;
        };
        graph.boundary.push(crate::plan::PortDecl {
            port: next,
            direction: Direction::Output,
            unwired: crate::plan::UnwiredPolicy::RequireWire,
            ty: Some(state_type),
            annotations: AnnotationTable::empty(),
        });
        graph.boundary.push(crate::plan::PortDecl {
            port: expired,
            direction: Direction::Output,
            unwired: crate::plan::UnwiredPolicy::RequireWire,
            ty: Some(expired_type),
            annotations: AnnotationTable::empty(),
        });
        true
    }

    /// Checks a Graph-owned body through the ordinary handler checker. The
    /// temporary context only supplies the Graph's boundary table; it creates
    /// neither a hidden Operator nor a second statement algebra.
    pub(super) fn graph_handler(
        &mut self,
        decl: &crate::compiler::syntax::HandlerDecl,
        graph: &Graph,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Handler> {
        let context = Node {
            path: node::Path::new(["__graph_handler"]).ok()?,
            extensions: Vec::new(),
            loop_spec: None,
            semantics: NodeSemantics::Operator,
            vals: graph.vals.clone(),
            vars: Vec::new(),
            ports: graph.boundary.clone(),
            handlers: Vec::new(),
            instance: None,
        };
        let SynTrigger::ChildPort { child, port } = &decl.trigger else {
            if matches!(
                decl.trigger,
                SynTrigger::Port(_) | SynTrigger::Start(_) | SynTrigger::Stop(_)
            ) {
                return self.handler(decl, signature, &context, consts);
            }
            self.deferred("this Graph handler trigger", decl.span);
            return None;
        };

        let child_name = child.text.as_str();
        let child_node = graph.nodes.iter().find(|candidate| {
            candidate.path.segments().last().map(String::as_str) == Some(child_name)
        });
        let child_graph = graph.graphs.iter().find(|candidate| {
            candidate.path.segments().last().map(String::as_str) == Some(child_name)
        });
        let (child_path, ports) = match (child_node, child_graph) {
            (Some(child), None) => (child.path.clone(), &child.ports),
            (None, Some(child)) => {
                let Some(path) = node::Path::new(child.path.segments().iter().cloned()).ok() else {
                    self.loop_refusal(decl.span, "the child Graph path is not representable");
                    return None;
                };
                (path, &child.boundary)
            }
            _ => {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_UNKNOWN,
                        Phase::Check,
                        format!("this Graph has no direct child named `{child_name}`"),
                    )
                    .with_span(child.span)
                    .with_field("node", child.text.clone()),
                );
                return None;
            }
        };
        let Some(decl_port) = ports
            .iter()
            .find(|candidate| candidate.port.name() == port.text)
        else {
            self.bag.push(
                Diagnostic::new(
                    E_PORT_UNKNOWN,
                    Phase::Check,
                    format!("child `{child_name}` exposes no port named `{}`", port.text),
                )
                .with_span(port.span)
                .with_field("port", port.text.clone()),
            );
            return None;
        };
        if decl_port.direction != Direction::Output {
            self.wrong_direction(
                port.span,
                &port.text,
                decl_port.direction,
                "a Graph handler observes a child output port",
            );
            return None;
        }
        let Some(data_type) = decl_port.ty.clone() else {
            self.loop_refusal(
                port.span,
                "a child output observed by Loop must have one resolved type",
            );
            return None;
        };
        let Ok(port) = node::Port::new(port.text.clone()) else {
            return None;
        };
        self.handler_with_activation(
            decl,
            signature,
            &context,
            consts,
            Some((
                node::HandlerActivation::ChildPort {
                    child: child_path,
                    port,
                },
                data_type,
            )),
        )
    }

    /// Checks the source half of the first Loop gate while lowering remains
    /// fail-closed. Keeping this here, beside instance attributes, gives every
    /// qualified spelling one owner and prevents generic graph-attribute
    /// deferrals from becoming a second interpretation.
    pub(super) fn loop_declaration(
        &mut self,
        decl: &GraphDecl,
        graph: &Graph,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
        legacy_graph_handlers: bool,
    ) -> Option<LoopSpec> {
        let attributes: Vec<_> = decl
            .body
            .iter()
            .filter_map(|item| match item {
                GraphItem::Attribute(attribute) if is_loop_attribute(attribute) => Some(attribute),
                _ => None,
            })
            .collect();
        let owns_loop = graph.extensions.contains(&node::Extension::Loop);
        if attributes.is_empty() {
            if owns_loop {
                self.loop_refusal(
                    decl.span,
                    "a `: Loop` Graph requires positive `Loop.fuel` or a `Loop.measure` that can be proved finite",
                );
            }
            return None;
        }
        if !owns_loop {
            for attribute in attributes {
                self.loop_refusal(
                    attribute.span,
                    "a `Loop.*` attribute requires the Graph's `+ Loop` Extension",
                );
            }
            return None;
        }

        let mut selected: BTreeMap<String, &SynAttribute> = BTreeMap::new();
        let mut unsupported = None;
        let mut invalid = false;
        for attribute in attributes {
            let name = match attribute.target.segments.as_slice() {
                [owner, name] if owner.text == "Loop" => name.text.as_str(),
                [name]
                    if matches!(
                        name.text.as_str(),
                        "fuel"
                            | "delay"
                            | "invariant"
                            | "measure"
                            | "entry"
                            | "terminal"
                            | "jitter"
                    ) =>
                {
                    self.loop_refusal(
                        attribute.span,
                        &format!(
                            "`{}` is Extension-owned; write `Loop.{}`",
                            name.text, name.text
                        ),
                    );
                    invalid = true;
                    continue;
                }
                [name] => name.text.as_str(),
                _ => {
                    self.loop_refusal(
                        attribute.span,
                        "Loop attributes have exactly one qualified name, such as `Loop.fuel`",
                    );
                    invalid = true;
                    continue;
                }
            };
            match name {
                "fuel" | "delay" | "invariant" | "measure" | "entry" | "terminal" => {
                    if selected.insert(name.to_string(), attribute).is_some() {
                        self.loop_refusal(
                            attribute.span,
                            &format!("`Loop.{name}` is declared twice"),
                        );
                        invalid = true;
                    }
                }
                "jitter" => {
                    if unsupported
                        .replace(("Loop.jitter", attribute.span))
                        .is_some()
                    {
                        self.loop_refusal(attribute.span, "`Loop.jitter` is declared twice");
                        invalid = true;
                    }
                }
                "variant" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.variant` was removed; use finite `Loop.fuel` now, or theorem-tier `Loop.measure`",
                    );
                    invalid = true;
                }
                "backoff" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.backoff` was removed; write `Loop.delay = { ... }`",
                    );
                    invalid = true;
                }
                "limit" | "max_steps" => {
                    self.loop_refusal(
                        attribute.span,
                        &format!("`{name}` was removed; write `Loop.fuel`"),
                    );
                    invalid = true;
                }
                "decreases" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.decreases` was removed; Rocq-compatible termination vocabulary uses `Loop.measure`",
                    );
                    invalid = true;
                }
                "until" => {
                    self.loop_refusal(
                        attribute.span,
                        "`until` was removed; terminal paths emit an ordinary output or `stop`, while recursive paths emit `next`",
                    );
                    invalid = true;
                }
                "initial" | "source" | "value" => {
                    self.loop_refusal(
                        attribute.span,
                        &format!(
                            "`{name}` was removed; iteration state enters through `in` and is available as `Loop.state`"
                        ),
                    );
                    invalid = true;
                }
                other => {
                    self.loop_refusal(
                        attribute.span,
                        &format!(
                            "unknown Loop attribute `{other}`; expected fuel, delay, invariant, measure, entry, or terminal"
                        ),
                    );
                    invalid = true;
                }
            }
        }
        if invalid {
            return None;
        }
        if selected.contains_key("invariant") && !selected.contains_key("measure") {
            self.loop_refusal(
                selected
                    .get("invariant")
                    .map_or(decl.span, |attribute| attribute.span),
                "`Loop.invariant` accompanies a `Loop.measure` proof",
            );
            return None;
        }
        if let Some((construct, span)) = unsupported {
            self.deferred(construct, span);
            return None;
        }

        if legacy_graph_handlers
            && (selected.contains_key("entry") || selected.contains_key("terminal"))
        {
            self.bag.push(
                Diagnostic::new(
                    E_GRAPH_HANDLER,
                    Phase::Check,
                    "`Loop.entry` and `Loop.terminal` belong to the Graph-scope Loop model; move Graph handlers into an Operator",
                )
                .with_span(decl.span)
                .with_field("owner", graph.path.to_string()),
            );
            return None;
        }

        let input_ports: Vec<_> = graph
            .boundary
            .iter()
            .filter(|port| port.direction == Direction::Input)
            .collect();
        let entry = if legacy_graph_handlers {
            let Some(input) = input_ports.iter().find(|port| port.port.name() == "in") else {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_ENTRY,
                        Phase::Check,
                        "the historical Loop Graph compatibility path requires `input in`",
                    )
                    .with_span(decl.span)
                    .with_field("owner", graph.path.to_string()),
                );
                return None;
            };
            input.port.clone()
        } else if let Some(attribute) = selected.get("entry") {
            self.loop_port_selection(attribute, &input_ports, false)?
                .into_iter()
                .next()?
        } else if input_ports.len() == 1 {
            input_ports[0].port.clone()
        } else if let Some(input) = input_ports.iter().find(|port| port.port.name() == "in") {
            input.port.clone()
        } else {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_ENTRY,
                    Phase::Check,
                    "Loop needs one recurrent Graph input; declare `Loop.entry = <input>`",
                )
                .with_span(decl.span)
                .with_field("owner", graph.path.to_string()),
            );
            return None;
        };

        let output_ports: Vec<_> = graph
            .boundary
            .iter()
            .filter(|port| port.direction == Direction::Output)
            .collect();
        let mut terminals = if legacy_graph_handlers {
            output_ports.iter().map(|port| port.port.clone()).collect()
        } else if let Some(attribute) = selected.get("terminal") {
            self.loop_port_selection(attribute, &output_ports, true)?
        } else {
            match output_ports.as_slice() {
                [] => Vec::new(),
                [only] => vec![only.port.clone()],
                _ => {
                    self.bag.push(
                        Diagnostic::new(
                            E_LOOP_TERMINAL,
                            Phase::Check,
                            "a Loop Graph with several outputs declares the terminal set explicitly with `Loop.terminal = [...]`",
                        )
                        .with_span(decl.span)
                        .with_field("owner", graph.path.to_string()),
                    );
                    return None;
                }
            }
        };
        terminals.sort();
        terminals.dedup();

        let fuel = match selected.get("fuel") {
            Some(attribute) => Some(self.loop_fuel(attribute, signature, consts)?),
            None => None,
        };
        let proof = match selected.get("measure") {
            Some(attribute) => {
                let state_type = graph
                    .boundary
                    .iter()
                    .find(|port| port.port == entry && port.direction == Direction::Input)
                    .and_then(|port| port.ty.clone());
                let Some(state_type) = state_type else {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.measure` requires the recurrent Graph input to have a closed type",
                    );
                    return None;
                };
                let theorem_signature = if legacy_graph_handlers {
                    signature.clone().with("data", state_type)
                } else {
                    loop_state_signature(signature, state_type)
                };
                let measure = self.typed_value_tree(
                    &attribute.value,
                    &theorem_signature,
                    None,
                    attribute.span,
                    "Loop.measure",
                )?;
                if !matches!(
                    measure.ty,
                    DataType::Integer(crate::datatype::IntegerType::Signed(_))
                ) {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.measure` must be a signed integer expression",
                    );
                    return None;
                }
                let invariant = match selected.get("invariant") {
                    None => None,
                    Some(attribute) => Some(self.typed_value_tree(
                        &attribute.value,
                        &theorem_signature,
                        Some(&DataType::Bool),
                        attribute.span,
                        "Loop.invariant",
                    )?),
                };
                Some(crate::loop_proof::Certificate {
                    measure,
                    invariant,
                    decreases: Vec::new(),
                })
            }
            None => None,
        };
        if fuel.is_none() && proof.is_none() {
            self.loop_refusal(
                decl.span,
                "a `: Loop` Graph requires positive `Loop.fuel` or a `Loop.measure` that can be proved finite",
            );
            return None;
        }
        let delay = match selected.get("delay") {
            Some(attribute) => Some(self.loop_delay(attribute, signature, consts)?),
            None => None,
        };
        Some(LoopSpec {
            entry,
            terminals,
            fuel,
            delay,
            proof,
        })
    }

    /// Checks the same Loop source contract on an executable Node. The v15
    /// runtime cannot transport it, but retaining the complete resolved shape
    /// in HIR lets R4 perform one categorical bump instead of teaching the old
    /// plan a temporary partial meaning.
    pub(super) fn node_loop_declaration(
        &mut self,
        body: &[OperatorItem],
        node: &Node,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
        owner_span: Span,
    ) -> Option<LoopSpec> {
        let attributes: Vec<_> = body
            .iter()
            .filter_map(|item| match item {
                OperatorItem::Attribute(attribute) if is_loop_attribute(attribute) => {
                    Some(attribute)
                }
                _ => None,
            })
            .collect();
        let owns_loop = node.extensions.contains(&node::Extension::Loop);
        if attributes.is_empty() {
            if owns_loop {
                self.loop_refusal(
                    owner_span,
                    "a `+ Loop` Executor requires positive `Loop.fuel` or a `Loop.measure` that can be proved finite",
                );
            }
            return None;
        }
        if !owns_loop {
            for attribute in attributes {
                self.loop_refusal(
                    attribute.span,
                    "a `Loop.*` attribute requires this Executor to declare `+ Loop`",
                );
            }
            return None;
        }

        let call_semantics = match &node.semantics {
            NodeSemantics::Operator => None,
            NodeSemantics::Connector { contract, .. } => {
                let Some(profile) = connector_call_semantics(node) else {
                    self.bag.push(
                        Diagnostic::new(
                            E_LOOP_HOST,
                            Phase::Check,
                            "Loop requires an Operator or a resolved outbound Call Contract with closed frame and terminal guarantees",
                        )
                        .with_span(owner_span)
                        .with_field("node", node.path.to_string())
                        .with_field("contract", contract.to_string()),
                    );
                    return None;
                };
                Some(profile)
            }
        };
        let connector_call = call_semantics.is_some();

        let mut selected: BTreeMap<String, &SynAttribute> = BTreeMap::new();
        let mut invalid = false;
        for attribute in attributes {
            let name = match attribute.target.segments.as_slice() {
                [extension, name] if extension.text == "Loop" => name.text.as_str(),
                [name] => {
                    self.loop_refusal(
                        attribute.span,
                        &format!(
                            "`{}` is Extension-owned; write `Loop.{}`",
                            name.text, name.text
                        ),
                    );
                    invalid = true;
                    continue;
                }
                _ => {
                    self.loop_refusal(
                        attribute.span,
                        "Loop attributes have exactly one qualified name, such as `Loop.fuel`",
                    );
                    invalid = true;
                    continue;
                }
            };
            match name {
                "fuel" | "delay" | "invariant" | "measure" | "entry" | "terminal" => {
                    if selected.insert(name.to_string(), attribute).is_some() {
                        self.loop_refusal(
                            attribute.span,
                            &format!("`Loop.{name}` is declared twice"),
                        );
                        invalid = true;
                    }
                }
                "jitter" => {
                    self.deferred("Loop.jitter", attribute.span);
                    invalid = true;
                }
                "variant" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.variant` was removed; use `Loop.fuel` or theorem-tier `Loop.measure`",
                    );
                    invalid = true;
                }
                "backoff" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.backoff` was removed; write `Loop.delay = { ... }`",
                    );
                    invalid = true;
                }
                "limit" | "max_steps" => {
                    self.loop_refusal(
                        attribute.span,
                        &format!("`{name}` was removed; write `Loop.fuel`"),
                    );
                    invalid = true;
                }
                "decreases" => {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.decreases` was removed; use `Loop.measure`",
                    );
                    invalid = true;
                }
                "initial" | "source" | "value" | "until" => {
                    self.loop_refusal(attribute.span, &format!("removed Loop attribute `{name}`"));
                    invalid = true;
                }
                other => {
                    self.loop_refusal(
                        attribute.span,
                        &format!(
                            "unknown Loop attribute `{other}`; expected fuel, delay, measure, invariant, entry, or terminal"
                        ),
                    );
                    invalid = true;
                }
            }
        }
        if invalid {
            return None;
        }

        let input_ports: Vec<_> = node
            .ports
            .iter()
            .filter(|port| port.direction == Direction::Input)
            .collect();
        let entry = if let Some(attribute) = selected.get("entry") {
            self.loop_port_selection(attribute, &input_ports, false)?
                .into_iter()
                .next()?
        } else if input_ports.len() == 1 {
            input_ports[0].port.clone()
        } else if let Some(input) = input_ports.iter().find(|port| port.port.name() == "in") {
            input.port.clone()
        } else {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_ENTRY,
                    Phase::Check,
                    "Loop needs one recurrent input; declare `Loop.entry = <input>`",
                )
                .with_span(owner_span)
                .with_field("node", node.path.to_string()),
            );
            return None;
        };

        let output_ports: Vec<_> = node
            .ports
            .iter()
            .filter(|port| port.direction == Direction::Output)
            .collect();
        if let Some(profile) = call_semantics
            && (input_ports.len() != 1 || entry.name() != profile.request())
        {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_ENTRY,
                    Phase::Check,
                    "a Loop Call admits only its Contract's sole request input",
                )
                .with_span(owner_span)
                .with_field("node", node.path.to_string())
                .with_field("expected", profile.request())
                .with_field("found", entry.to_string()),
            );
            return None;
        }
        let mut terminals = if connector_call {
            if let Some(attribute) = selected.get("terminal") {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_TERMINAL,
                        Phase::Check,
                        "an outbound Call Connector terminates on its Contract exit; `Loop.terminal` is not applicable",
                    )
                    .with_span(attribute.span)
                    .with_field("node", node.path.to_string()),
                );
                return None;
            }
            Vec::new()
        } else if let Some(attribute) = selected.get("terminal") {
            self.loop_port_selection(attribute, &output_ports, true)?
        } else {
            match output_ports.as_slice() {
                [] => Vec::new(),
                [only] => vec![only.port.clone()],
                _ => {
                    self.bag.push(
                        Diagnostic::new(
                            E_LOOP_TERMINAL,
                            Phase::Check,
                            "a Loop Operator with several outputs declares the terminal set explicitly with `Loop.terminal = [...]`",
                        )
                        .with_span(owner_span)
                        .with_field("node", node.path.to_string()),
                    );
                    return None;
                }
            }
        };
        terminals.sort();
        terminals.dedup();

        let fuel = match selected.get("fuel") {
            Some(attribute) => Some(self.loop_fuel(attribute, signature, consts)?),
            None => None,
        };
        let proof = match selected.get("measure") {
            Some(attribute) if connector_call => {
                self.loop_refusal(
                    attribute.span,
                    "an opaque Connector result cannot discharge `Loop.measure`; declare positive `Loop.fuel`",
                );
                return None;
            }
            Some(attribute) => {
                let state_type = node
                    .ports
                    .iter()
                    .find(|port| port.port == entry)
                    .and_then(|port| port.ty.clone());
                let Some(state_type) = state_type else {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.measure` requires the recurrent input to have a closed type",
                    );
                    return None;
                };
                let theorem_signature = loop_state_signature(signature, state_type);
                let measure = self.typed_value_tree(
                    &attribute.value,
                    &theorem_signature,
                    None,
                    attribute.span,
                    "Loop.measure",
                )?;
                if !matches!(
                    measure.ty,
                    DataType::Integer(crate::datatype::IntegerType::Signed(_))
                ) {
                    self.loop_refusal(
                        attribute.span,
                        "`Loop.measure` must be a signed integer expression",
                    );
                    return None;
                }
                let invariant = match selected.get("invariant") {
                    None => None,
                    Some(invariant) => Some(self.typed_value_tree(
                        &invariant.value,
                        &theorem_signature,
                        Some(&DataType::Bool),
                        invariant.span,
                        "Loop.invariant",
                    )?),
                };
                Some(crate::loop_proof::Certificate {
                    measure,
                    invariant,
                    decreases: Vec::new(),
                })
            }
            None => {
                if let Some(invariant) = selected.get("invariant") {
                    self.loop_refusal(
                        invariant.span,
                        "`Loop.invariant` accompanies a `Loop.measure` proof",
                    );
                    return None;
                }
                None
            }
        };
        if fuel.is_none() && proof.is_none() {
            self.loop_refusal(
                owner_span,
                "a `+ Loop` Executor requires positive `Loop.fuel` or a `Loop.measure` that can be proved finite",
            );
            return None;
        }
        let delay = match selected.get("delay") {
            Some(attribute) => Some(self.loop_delay(attribute, signature, consts)?),
            None => None,
        };
        Some(LoopSpec {
            entry,
            terminals,
            fuel,
            delay,
            proof,
        })
    }

    /// Resolves port names carried by Loop's source attributes without
    /// confusing them with runtime value expressions. All candidate names get
    /// one dummy type solely so the expression parser can own comments,
    /// commas, brackets, and diagnostics.
    pub(super) fn loop_port_selection(
        &mut self,
        attribute: &SynAttribute,
        candidates: &[&crate::plan::PortDecl],
        list: bool,
    ) -> Option<Vec<node::Port>> {
        let mut signature = expr::Signature::new();
        for candidate in candidates {
            signature = signature.with(candidate.port.name(), DataType::Text);
        }
        let tree = self.typed_value_tree(
            &attribute.value,
            &signature,
            None,
            attribute.span,
            if list { "Loop.terminal" } else { "Loop.entry" },
        )?;
        let paths: Vec<Vec<String>> = if list {
            let expr::Expr::List(items) = tree.root else {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_TERMINAL,
                        Phase::Check,
                        "`Loop.terminal` is a non-empty list of output port names",
                    )
                    .with_span(attribute.span),
                );
                return None;
            };
            let mut paths = Vec::with_capacity(items.len());
            for item in items {
                let expr::Expr::Path(path) = item else {
                    self.bag.push(
                        Diagnostic::new(
                            E_LOOP_TERMINAL,
                            Phase::Check,
                            "every `Loop.terminal` item must name one output port",
                        )
                        .with_span(attribute.span),
                    );
                    return None;
                };
                paths.push(path);
            }
            paths
        } else {
            let expr::Expr::Path(path) = tree.root else {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_ENTRY,
                        Phase::Check,
                        "`Loop.entry` is one input port name",
                    )
                    .with_span(attribute.span),
                );
                return None;
            };
            vec![path]
        };
        if paths.is_empty() {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_TERMINAL,
                    Phase::Check,
                    "`Loop.terminal` requires at least one output port",
                )
                .with_span(attribute.span),
            );
            return None;
        }
        let mut ports = Vec::with_capacity(paths.len());
        for path in paths {
            let [name] = path.as_slice() else {
                let code = if list { E_LOOP_TERMINAL } else { E_LOOP_ENTRY };
                self.bag.push(
                    Diagnostic::new(
                        code,
                        Phase::Check,
                        "a Loop port selector is one unqualified name",
                    )
                    .with_span(attribute.span),
                );
                return None;
            };
            let Some(candidate) = candidates
                .iter()
                .find(|candidate| candidate.port.name() == name)
            else {
                let code = if list { E_LOOP_TERMINAL } else { E_LOOP_ENTRY };
                self.bag.push(
                    Diagnostic::new(
                        code,
                        Phase::Check,
                        format!(
                            "`{name}` is not a Loop {} port",
                            if list {
                                "terminal output"
                            } else {
                                "entry input"
                            }
                        ),
                    )
                    .with_span(attribute.span)
                    .with_field("port", name.clone()),
                );
                return None;
            };
            if ports.contains(&candidate.port) {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_TERMINAL,
                        Phase::Check,
                        format!("`{name}` appears twice in `Loop.terminal`"),
                    )
                    .with_span(attribute.span)
                    .with_field("port", name.clone()),
                );
                return None;
            }
            ports.push(candidate.port.clone());
        }
        Some(ports)
    }

    /// Produces the hosted proof evidence only after the ordinary Graph
    /// handlers have been checked. The `no_std` validator derives the same
    /// evidence again from the transported plan, so this step cannot grant
    /// authority to a compiler-only claim.
    pub(super) fn complete_loop_proof(&mut self, graph: &mut Graph, span: Span) -> bool {
        let handlers = loop_proof_handlers(graph);
        let Some(spec) = graph.loop_spec.as_mut() else {
            return false;
        };
        let Some(proof) = spec.proof.take() else {
            return true;
        };
        let Some(handlers) = handlers else {
            self.loop_refusal(
                span,
                "a Loop proof over recursive `match` Dispatch requires the v16 proof checker",
            );
            return false;
        };
        match crate::loop_proof::derive(
            crate::plan::OwnerSlot::new(0),
            proof.measure,
            proof.invariant,
            &handlers,
        ) {
            Ok(proof) => {
                spec.proof = Some(proof);
                true
            }
            Err(error) => {
                self.loop_refusal(span, &format!("this Loop cannot be proved finite: {error}"));
                false
            }
        }
    }

    pub(super) fn loop_fuel(
        &mut self,
        attribute: &SynAttribute,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Const> {
        if matches!(attribute.value, SynValue::Error(_)) {
            return None;
        }
        let tree = self.typed_value_tree(
            &attribute.value,
            signature,
            None,
            attribute.span,
            "Loop.fuel",
        )?;
        let constant = self.reduce_constant(tree, attribute.span, consts)?;
        if matches!(
            constant.value,
            crate::value::Value::Int(value) if value > 0
        ) || matches!(
            constant.value,
            crate::value::Value::Uint(value) if value > 0
        ) {
            Some(constant)
        } else {
            self.loop_refusal(attribute.span, "`Loop.fuel` must be a positive integer");
            None
        }
    }

    pub(super) fn loop_delay(
        &mut self,
        attribute: &SynAttribute,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<LoopDelay> {
        let SynValue::Braced(body) = &attribute.value else {
            self.loop_refusal(
                attribute.span,
                "`Loop.delay` must be a composite with a `kind` field",
            );
            return None;
        };
        let mut fields: BTreeMap<String, (&SynValue, Span)> = BTreeMap::new();
        for entry in &body.entries {
            let StructEntry::Field { key, value } = entry else {
                self.loop_refusal(
                    attribute.span,
                    "`Loop.delay` admits fields, not nested configuration blocks",
                );
                return None;
            };
            if key.segments.len() != 1 {
                self.loop_refusal(key.span, "Loop.delay fields are single names");
                return None;
            }
            let name = key.segments[0].text.clone();
            if fields.insert(name.clone(), (value, key.span)).is_some() {
                self.loop_refusal(key.span, &format!("`Loop.delay.{name}` is declared twice"));
                return None;
            }
        }
        let Some((kind, kind_span)) = fields.remove("kind") else {
            self.loop_refusal(attribute.span, "`Loop.delay` requires exactly one `kind`");
            return None;
        };
        let SynValue::Expr(kind) = kind else {
            self.loop_refusal(kind_span, "`Loop.delay.kind` is a bare choice word");
            return None;
        };
        if !is_bare_word(&kind.text) {
            self.loop_refusal(kind.span, "`Loop.delay.kind` is a bare choice word");
            return None;
        }
        let delay = {
            let mut duration = |name: &str| {
                let Some((value, span)) = fields.remove(name) else {
                    self.loop_refusal(
                        attribute.span,
                        &format!("`Loop.delay(kind = {})` requires `{name}`", kind.text),
                    );
                    return None;
                };
                let tree = self.typed_value_tree(
                    value,
                    signature,
                    Some(&DataType::DURATION),
                    span,
                    &format!("Loop.delay.{name}"),
                )?;
                let constant = self.reduce_constant(tree, span, consts)?;
                match constant.value {
                    crate::value::Value::Duration(nanos) if nanos > 0 => Some(constant),
                    _ => {
                        self.loop_refusal(
                            span,
                            &format!("`Loop.delay.{name}` must be a positive duration"),
                        );
                        None
                    }
                }
            };
            match kind.text.as_str() {
                "fixed" => LoopDelay::Fixed {
                    after: duration("after")?,
                },
                "exponential" => {
                    let first = duration("first")?;
                    let ceiling = duration("ceiling")?;
                    let (
                        crate::value::Value::Duration(first_nanos),
                        crate::value::Value::Duration(ceiling_nanos),
                    ) = (&first.value, &ceiling.value)
                    else {
                        return None;
                    };
                    if ceiling_nanos < first_nanos {
                        self.loop_refusal(
                            attribute.span,
                            "`Loop.delay.ceiling` must be no smaller than `Loop.delay.first`",
                        );
                        return None;
                    }
                    LoopDelay::Exponential { first, ceiling }
                }
                other => {
                    self.loop_refusal(
                        kind.span,
                        &format!(
                            "unknown Loop.delay kind `{other}`; expected fixed or exponential"
                        ),
                    );
                    return None;
                }
            }
        };
        if let Some((unexpected, (_, span))) = fields.into_iter().next() {
            self.loop_refusal(
                span,
                &format!(
                    "`Loop.delay(kind = {})` has no `{unexpected}` field",
                    kind.text
                ),
            );
            return None;
        }
        Some(delay)
    }

    pub(super) fn loop_refusal(&mut self, span: Span, detail: &str) {
        self.bag.push(
            Diagnostic::new(E_LOOP, Phase::Check, detail)
                .with_span(span)
                .with_field("detail", detail),
        );
    }

    pub(super) fn validate_call_command_route(
        &mut self,
        source: &Endpoint,
        destinations: &[RouteDestination],
        cases: Option<&[String]>,
        graph: &Graph,
        span: Span,
    ) -> Option<()> {
        use crate::connector::CallCommandError;

        let EndpointOwner::Node(path) = &source.owner else {
            return Some(());
        };
        let Some(node) = node_at_path(graph, path).filter(|node| node.loop_spec.is_some()) else {
            return Some(());
        };
        let Some(contract) = connector_call_semantics(node) else {
            return Some(());
        };
        for destination in destinations {
            let RouteDestination::LoopCommand(target) = destination else {
                continue;
            };
            if target.owner != source.owner {
                continue;
            }
            let effect = node.attributes().iter().find_map(|attribute| {
                if attribute.name != "effect" {
                    return None;
                }
                let AttrValue::Word(word) = &attribute.value else {
                    return None;
                };
                match word.as_str() {
                    "none" => Some(node::Effect::None),
                    "idempotent" => Some(node::Effect::Class(node::EffectClass::Idempotent)),
                    "compensable" => Some(node::Effect::Class(node::EffectClass::Compensable)),
                    "irreversible" => Some(node::Effect::Class(node::EffectClass::Irreversible)),
                    _ => None,
                }
            })?;
            if let Err(error) =
                contract.validate_command(source.port.name(), cases, target.kind, effect)
            {
                let code = match &error {
                    CallCommandError::RepeatedFrames => E_LOOP_CARDINALITY,
                    CallCommandError::UnsafeRecurrence { .. } => E_LOOP_EFFECT,
                    CallCommandError::UnknownPort | CallCommandError::UnknownCase(_) => {
                        crate::compiler::E_INTERNAL_PLAN
                    }
                };
                self.bag.push(
                    Diagnostic::new(code, Phase::Check, error.to_string())
                        .with_span(span)
                        .with_field("source", endpoint_display(source))
                        .with_field("destination", route_destination_display(destination))
                        .with_field("effect", effect.to_string()),
                );
                return None;
            }
        }
        Some(())
    }
}

fn connector_call_semantics(node: &Node) -> Option<crate::connector::CallSemantics> {
    let NodeSemantics::Connector {
        contract,
        roles,
        exchange,
        attributes,
    } = &node.semantics
    else {
        return None;
    };
    let mode = attributes.iter().find_map(|attribute| {
        if attribute.name != "mode" {
            return None;
        }
        match &attribute.value {
            AttrValue::Word(word) => Some(word.as_str()),
            _ => None,
        }
    });
    crate::connector::outbound_call_semantics(contract, *roles, *exchange, mode)
}

pub(super) fn graph_loop_entry_type(graph: &Graph) -> Option<DataType> {
    let entry = &graph.loop_spec.as_ref()?.entry;
    graph
        .boundary
        .iter()
        .find(|port| &port.port == entry && port.direction == Direction::Input)?
        .ty
        .clone()
}

pub(super) fn loop_owner_entry_type(owner: &EndpointOwner, graph: &Graph) -> Option<DataType> {
    match owner {
        EndpointOwner::Node(path) => node_at_path(graph, path).and_then(resolved_loop_entry_type),
        EndpointOwner::Graph(path) => graph_at_path(graph, path).and_then(graph_loop_entry_type),
    }
}

pub(super) fn is_loop_attribute(attribute: &SynAttribute) -> bool {
    let Some(first) = attribute.target.segments.first() else {
        return false;
    };
    first.text == "Loop"
        || (attribute.target.segments.len() == 1
            && matches!(
                first.text.as_str(),
                "fuel"
                    | "delay"
                    | "invariant"
                    | "measure"
                    | "entry"
                    | "terminal"
                    | "jitter"
                    | "variant"
                    | "backoff"
                    | "initial"
                    | "source"
                    | "value"
                    | "until"
                    | "limit"
                    | "max_steps"
                    | "decreases"
            ))
}

/// Mirrors only the already-checked handler algebra into the core proof
/// boundary. Plan lowering performs its own full conversion later; this local
/// copy exists so an unsupported theorem is diagnosed at its source instead
/// of first appearing as an internal plan refusal.
pub(super) fn loop_proof_handlers(graph: &Graph) -> Option<Vec<crate::plan::Handler>> {
    let emission = |value: &EmitValue| match value {
        EmitValue::Identity => crate::plan::Emission::Identity,
        EmitValue::Patch(fields) | EmitValue::FreshData(fields) => {
            let writes = fields
                .iter()
                .map(|field| crate::plan::FieldWrite {
                    target: field.target.clone(),
                    expr: field.value.clone(),
                })
                .collect();
            if matches!(value, EmitValue::Patch(_)) {
                crate::plan::Emission::Patch(writes)
            } else {
                crate::plan::Emission::Replace(writes)
            }
        }
        EmitValue::Data(tree) => crate::plan::Emission::Data(tree.clone()),
        EmitValue::Nothing => crate::plan::Emission::Nothing,
    };
    graph
        .handlers
        .iter()
        .map(|handler| {
            let (guard, body) = handler.v15_linear_parts()?;
            let body: Option<Vec<_>> = body
                .iter()
                .map(|statement| match statement {
                    Statement::Let {
                        name,
                        value,
                        annotations,
                    } => Some(crate::plan::Step::Let {
                        name: name.clone(),
                        expr: value.clone(),
                        annotations: annotations.clone(),
                    }),
                    Statement::Emit { port, value } => Some(crate::plan::Step::Emit {
                        port: port.clone(),
                        value: emission(value),
                    }),
                    Statement::SetVar { name, value } => Some(crate::plan::Step::SetVar {
                        name: name.clone(),
                        expr: value.clone(),
                    }),
                    Statement::Stop => Some(crate::plan::Step::Stop),
                    Statement::MultiEmit { .. } | Statement::LoopCommand(_) => None,
                })
                .collect();
            Some(crate::plan::Handler {
                owner: crate::plan::HandlerOwner::Graph(crate::plan::OwnerSlot::new(0)),
                activation: handler.activation.clone(),
                guard: guard.cloned(),
                body: body?,
            })
        })
        .collect()
}

pub(super) fn resolved_loop_entry_type(node: &Node) -> Option<DataType> {
    let entry = &node.loop_spec.as_ref()?.entry;
    node.ports
        .iter()
        .find(|port| &port.port == entry && port.direction == Direction::Input)?
        .ty
        .clone()
}

/// An inner Loop replaces these exact bindings; passive Graph scopes simply
/// carry them to their descendants. No runtime context is invented here.
pub(super) fn loop_state_signature(
    signature: &expr::Signature,
    state_type: DataType,
) -> expr::Signature {
    signature
        .clone()
        .with("Loop.state", DataType::Variant)
        .with("Loop.state.data", state_type)
        .with_metadata("Loop.state.meta")
}
