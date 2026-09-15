//! Loop host policies, state bindings and re-derived termination evidence.

use super::*;

impl Checker<'_, '_> {
    /// Resolve the Graph's Loop policy before descendant handlers so their
    /// `Loop.state` bindings refer to its exact entry type.
    pub(super) fn loop_declaration(
        &mut self,
        decl: &GraphDecl,
        graph: &Graph,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
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
                "fuel" | "delay" | "invariant" | "measure" | "entry" | "terminal" | "expired" => {
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

        let input_ports: Vec<_> = graph
            .boundary
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
        let mut terminals = if let Some(attribute) = selected.get("terminal") {
            self.loop_port_selection(attribute, &output_ports, true)?
        } else {
            output_ports
                .iter()
                .filter(|port| matches!(port.port.name(), "out" | "err"))
                .map(|port| port.port.clone())
                .collect()
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
        let mut spec = LoopSpec {
            entry,
            terminals,
            expired: None,
            fuel,
            delay,
            proof,
        };
        self.resolve_loop_expiry(
            &mut spec,
            &graph.boundary,
            selected.get("expired").copied(),
            None,
            decl.span,
        )
        .then_some(spec)
    }

    /// Resolve entry, terminal outputs and finite application policy from the
    /// executable Node's ports and, for a Connector, its Contract profile.
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
                "fuel" | "delay" | "invariant" | "measure" | "entry" | "terminal" | "expired" => {
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
            output_ports
                .iter()
                .filter(|port| matches!(port.port.name(), "out" | "err"))
                .map(|port| port.port.clone())
                .collect()
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
        let mut spec = LoopSpec {
            entry,
            terminals,
            expired: None,
            fuel,
            delay,
            proof,
        };
        self.resolve_loop_expiry(
            &mut spec,
            &node.ports,
            selected.get("expired").copied(),
            call_semantics.map(|call| call.terminal()),
            owner_span,
        )
        .then_some(spec)
    }

    fn resolve_loop_expiry(
        &mut self,
        spec: &mut LoopSpec,
        ports: &[crate::plan::PortDecl],
        selection: Option<&SynAttribute>,
        call_terminal: Option<&str>,
        span: Span,
    ) -> bool {
        if selection.is_some() && call_terminal.is_some() {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_TERMINAL,
                    Phase::Check,
                    "a Call uses its Contract terminal; `Loop.expired` cannot override it",
                )
                .with_span(span),
            );
            return false;
        }
        let expiry_is_impossible = spec.fuel.is_none() && spec.proof.is_some();
        if expiry_is_impossible && selection.is_none() {
            return true;
        }
        let outputs: Vec<_> = ports
            .iter()
            .filter(|port| port.direction == Direction::Output)
            .collect();
        let target = if let Some(attribute) = selection {
            let Some(selected) = self.loop_port_selection(attribute, &outputs, false) else {
                return false;
            };
            selected.into_iter().next()
        } else {
            ports
                .iter()
                .find(|port| {
                    port.direction == Direction::Output
                        && port.port.name() == call_terminal.unwrap_or("err")
                })
                .map(|port| port.port.clone())
        };
        if let Some(target) = &target
            && call_terminal.is_none()
            && !spec.terminals.contains(target)
        {
            self.bag.push(
                Diagnostic::new(
                    E_LOOP_TERMINAL,
                    Phase::Check,
                    "`Loop.expired` must select a terminal output of this host",
                )
                .with_span(span)
                .with_field("port", target.to_string()),
            );
            return false;
        }
        if expiry_is_impossible {
            return true;
        }
        let state = ports
            .iter()
            .find(|port| port.port == spec.entry)
            .and_then(|port| port.ty.clone());
        let expiry = target.and_then(|target| {
            let ty = ports.iter().find(|port| port.port == target)?.ty.as_ref()?;
            let DataType::TaggedUnion(union) = ty else {
                return None;
            };
            let state = state?;
            let effective = if call_terminal.is_some() {
                node::loop_call_exit_type(union, state).ok()?
            } else {
                let expected = node::loop_expired_type(state).ok()?;
                if !expected.is_assignable_to(ty) {
                    return None;
                }
                ty.clone()
            };
            Some(crate::plan::LoopExpiry {
                port: target,
                ty: effective,
            })
        });
        if expiry.is_none() {
            self.bag.push(Diagnostic::new(E_UNWIRED_OUTPUT, Phase::Check,
                "Loop exhaustion needs a terminal union: declare `output err` accepting `LoopExpired`, set `Loop.expired = <port>`, or prove finiteness")
                .with_span(span));
            return false;
        }
        spec.expired = expiry;
        true
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
        let expiry = attribute
            .target
            .segments
            .last()
            .is_some_and(|name| name.text == "expired");
        let label = if list {
            "Loop.terminal"
        } else if expiry {
            "Loop.expired"
        } else {
            "Loop.entry"
        };
        let mut signature = expr::Signature::new();
        for candidate in candidates {
            signature = signature.with(candidate.port.name(), DataType::Text);
        }
        let tree = if expiry {
            let SynValue::Expr(source) = &attribute.value else {
                self.bag.push(
                    Diagnostic::new(
                        E_LOOP_TERMINAL,
                        Phase::Check,
                        "`Loop.expired` must name one terminal output of this host",
                    )
                    .with_span(attribute.span),
                );
                return None;
            };
            match expr::compile(&source.text, &signature) {
                Ok(tree) => tree,
                Err(_) => {
                    self.bag.push(
                        Diagnostic::new(
                            E_LOOP_TERMINAL,
                            Phase::Check,
                            "`Loop.expired` must name one terminal output of this host",
                        )
                        .with_span(attribute.span),
                    );
                    return None;
                }
            }
        } else {
            self.typed_value_tree(&attribute.value, &signature, None, attribute.span, label)?
        };
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
                        if expiry {
                            E_LOOP_TERMINAL
                        } else {
                            E_LOOP_ENTRY
                        },
                        Phase::Check,
                        if expiry {
                            "`Loop.expired` is one terminal output port name"
                        } else {
                            "`Loop.entry` is one input port name"
                        },
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
                let code = if list || expiry {
                    E_LOOP_TERMINAL
                } else {
                    E_LOOP_ENTRY
                };
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
                let code = if list || expiry {
                    E_LOOP_TERMINAL
                } else {
                    E_LOOP_ENTRY
                };
                self.bag.push(
                    Diagnostic::new(
                        code,
                        Phase::Check,
                        format!(
                            "`{name}` is not a Loop {} port",
                            if list || expiry {
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

    pub(super) fn complete_operator_loop_proof(&mut self, node: &mut Node, span: Span) {
        let Some(spec) = node.loop_spec.as_mut() else {
            return;
        };
        let Some(proof) = spec.proof.as_ref() else {
            return;
        };
        let Some(handler) = node.handlers.iter().find(|handler| {
            handler.activation == node::HandlerActivation::Port(spec.entry.clone())
        }) else {
            return;
        };
        match crate::loop_proof::derive_dispatch(
            proof.measure.clone(),
            proof.invariant.clone(),
            &handler.dispatch,
        ) {
            Ok(derived) => spec.proof = Some(derived),
            Err(error) => {
                self.loop_refusal(span, &format!("this Loop cannot be proved finite: {error}"))
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
        self.extension_delay(attribute, signature, consts, "Loop", E_LOOP)
    }

    pub(super) fn extension_delay(
        &mut self,
        attribute: &SynAttribute,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
        extension: &str,
        code: Code,
    ) -> Option<LoopDelay> {
        let refuse = |checker: &mut Self, span: Span, detail: String| {
            checker.bag.push(
                Diagnostic::new(code, Phase::Check, &detail)
                    .with_span(span)
                    .with_field("detail", detail),
            );
        };
        let SynValue::Braced(body) = &attribute.value else {
            refuse(
                self,
                attribute.span,
                format!("`{extension}.delay` must be a composite with a `kind` field"),
            );
            return None;
        };
        let mut fields: BTreeMap<String, (&SynValue, Span)> = BTreeMap::new();
        for entry in &body.entries {
            let StructEntry::Field { key, value } = entry else {
                refuse(
                    self,
                    attribute.span,
                    format!("`{extension}.delay` admits fields, not nested configuration blocks"),
                );
                return None;
            };
            if key.segments.len() != 1 {
                refuse(
                    self,
                    key.span,
                    format!("{extension}.delay fields are single names"),
                );
                return None;
            }
            let name = key.segments[0].text.clone();
            if fields.insert(name.clone(), (value, key.span)).is_some() {
                refuse(
                    self,
                    key.span,
                    format!("`{extension}.delay.{name}` is declared twice"),
                );
                return None;
            }
        }
        let Some((kind, kind_span)) = fields.remove("kind") else {
            refuse(
                self,
                attribute.span,
                format!("`{extension}.delay` requires exactly one `kind`"),
            );
            return None;
        };
        let SynValue::Expr(kind) = kind else {
            refuse(
                self,
                kind_span,
                format!("`{extension}.delay.kind` is a bare choice word"),
            );
            return None;
        };
        if !is_bare_word(&kind.text) {
            refuse(
                self,
                kind.span,
                format!("`{extension}.delay.kind` is a bare choice word"),
            );
            return None;
        }
        let delay = {
            let mut duration = |name: &str| {
                let Some((value, span)) = fields.remove(name) else {
                    refuse(
                        self,
                        attribute.span,
                        format!(
                            "`{extension}.delay(kind = {})` requires `{name}`",
                            kind.text
                        ),
                    );
                    return None;
                };
                let tree = self.typed_value_tree(
                    value,
                    signature,
                    Some(&DataType::DURATION),
                    span,
                    &format!("{extension}.delay.{name}"),
                )?;
                let constant = self.reduce_constant(tree, span, consts)?;
                match constant.value {
                    crate::value::Value::Duration(nanos) if nanos > 0 => Some(constant),
                    _ => {
                        refuse(
                            self,
                            span,
                            format!("`{extension}.delay.{name}` must be a positive duration"),
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
                        refuse(
                            self,
                            attribute.span,
                            format!(
                                "`{extension}.delay.ceiling` must be no smaller than `{extension}.delay.first`"
                            ),
                        );
                        return None;
                    }
                    LoopDelay::Exponential { first, ceiling }
                }
                other => {
                    refuse(
                        self,
                        kind.span,
                        format!(
                            "unknown {extension}.delay kind `{other}`; expected fixed or exponential"
                        ),
                    );
                    return None;
                }
            }
        };
        if let Some((unexpected, (_, span))) = fields.into_iter().next() {
            refuse(
                self,
                span,
                format!(
                    "`{extension}.delay(kind = {})` has no `{unexpected}` field",
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
