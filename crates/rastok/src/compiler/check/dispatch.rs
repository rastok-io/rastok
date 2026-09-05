//! Typed handler Dispatch, admission filters and ordered multi-emission.

use super::*;

impl Checker<'_, '_> {
    pub(super) fn handler(
        &mut self,
        decl: &crate::compiler::syntax::HandlerDecl,
        node_signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Handler> {
        self.handler_with_activation(decl, node_signature, node, consts, None)
    }

    pub(super) fn handler_with_activation(
        &mut self,
        decl: &crate::compiler::syntax::HandlerDecl,
        node_signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation_override: Option<(node::HandlerActivation, DataType)>,
    ) -> Option<Handler> {
        let (mut activation, fixed_data_type) = if let Some((activation, data_type)) =
            activation_override
        {
            (activation, Some(data_type))
        } else {
            (
                match &decl.trigger {
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
                            Ok(port) => node::HandlerActivation::Port(port),
                            Err(_) => return None,
                        }
                    }
                    SynTrigger::Start(_) => {
                        node::HandlerActivation::Lifecycle(node::Lifecycle::Start)
                    }
                    SynTrigger::Stop(_) => {
                        node::HandlerActivation::Lifecycle(node::Lifecycle::Stop)
                    }
                    SynTrigger::Compensate(span) => {
                        self.deferred("a compensate handler", *span);
                        return None;
                    }
                    SynTrigger::Extension { extension, .. } => {
                        self.deferred("an Extension-qualified handler", extension.span);
                        return None;
                    }
                    SynTrigger::ChildPort { child, .. } => {
                        self.deferred("a child-output handler outside a Graph", child.span);
                        return None;
                    }
                    SynTrigger::Join(names) => {
                        let mut ports = Vec::with_capacity(names.len());
                        let mut unique = BTreeSet::new();
                        for name in names {
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
                                    "a join listens on input ports",
                                );
                                return None;
                            }
                            let Ok(port) = node::Port::new(name.text.clone()) else {
                                return None;
                            };
                            if !unique.insert(port.clone()) {
                                self.instance_refusal(
                                    name.span,
                                    "a join lists each input port exactly once",
                                );
                                return None;
                            }
                            ports.push(port);
                        }
                        if ports.len() < 2 {
                            self.instance_refusal(
                                decl.span,
                                "a join requires at least two input ports",
                            );
                            return None;
                        }
                        node::HandlerActivation::Join {
                            ports,
                            within_ns: None,
                            last: None,
                        }
                    }
                },
                None,
            )
        };
        // A source annotation or instantiated Connector contract on the
        // activation port TYPES the handler's `data`. An unpinned operator
        // port remains a compiler inference variable; `variant` below is only
        // the expression checker's temporary signature, never the port's
        // inferred effective type.
        let declared_data_type = fixed_data_type.or_else(|| match &activation {
            node::HandlerActivation::Port(port) => node_port_pin(node, port),
            node::HandlerActivation::ChildPort { .. } => None,
            node::HandlerActivation::Join { .. } => None,
            node::HandlerActivation::Lifecycle(_) => {
                let mut inputs = node
                    .ports
                    .iter()
                    .filter(|port| port.direction == Direction::Input);
                let Some(first) = inputs.next() else {
                    self.instance_refusal(
                        decl.span,
                        "a lifecycle handler requires an input envelope to cause the instance",
                    );
                    return None;
                };
                let first = first.ty.clone();
                inputs
                    .all(|port| port.ty == first)
                    .then_some(first)
                    .flatten()
            }
        });
        // The expression engine still needs a concrete signature during B2;
        // its `variant` here is parser/checker scaffolding only. The separate
        // `declared_data_type` is what prevents that placeholder from being
        // mistaken for an explicit dynamic port constraint.
        let data_type = declared_data_type.clone().unwrap_or(DataType::Variant);
        // `now()` is the syntax of a context binding, not an evaluator clock
        // read. Only a firing handler owns one logical instant; plan, owner,
        // instance and invocation-setting expressions deliberately do not
        // receive this name.
        let mut signature = node_signature.clone().with("now", DataType::TIMESTAMP);
        match &activation {
            node::HandlerActivation::Join { ports, .. } => {
                for port in ports {
                    signature = signature
                        .with(port.name(), DataType::Variant)
                        .with(
                            format!("{}.data", port.name()),
                            node_port_pin(node, port).unwrap_or(DataType::Variant),
                        )
                        .with_metadata(format!("{}.meta", port.name()));
                }
            }
            _ => {
                signature = signature
                    .with("data", data_type.clone())
                    .with("meta", DataType::Variant);
            }
        }
        let mut where_guard = None;
        let mut join_within = None;
        let mut join_last = None;
        for clause in &decl.guards {
            match clause {
                crate::compiler::syntax::Guard::Where(expr) if where_guard.is_none() => {
                    let tree = self.expr(expr, &signature)?;
                    if !matches!(tree.ty, DataType::Bool | DataType::Variant) {
                        self.bag.push(
                            Diagnostic::new(E_EXPR, Phase::Check, "a where guard must be a bool")
                                .with_span(expr.span)
                                .with_field("detail", "the guard type is not bool"),
                        );
                        return None;
                    }
                    where_guard = Some(tree);
                }
                crate::compiler::syntax::Guard::Where(expr) => {
                    self.bag.push(
                        Diagnostic::new(
                            E_DISPATCH_POSITION,
                            Phase::Check,
                            "a handler admits at most one `where`; combine predicates or use one `match` dispatch",
                        )
                        .with_span(expr.span)
                        .with_field("construct", "where"),
                    );
                }
                crate::compiler::syntax::Guard::Within(expr)
                    if matches!(&activation, node::HandlerActivation::Join { .. })
                        && join_within.is_none() =>
                {
                    let constant = self.constant(expr, node_signature, consts)?;
                    let crate::value::Value::Duration(duration) = &constant.value else {
                        self.instance_refusal(expr.span, "`within` requires a duration");
                        return None;
                    };
                    if *duration <= 0 {
                        self.instance_refusal(expr.span, "`within` must be positive");
                        return None;
                    }
                    join_within = Some(constant);
                }
                crate::compiler::syntax::Guard::Last(count)
                    if matches!(&activation, node::HandlerActivation::Join { .. })
                        && join_last.is_none() =>
                {
                    let span = count.span;
                    let Ok(count) = count.text.parse::<u32>() else {
                        self.instance_refusal(count.span, "`last` must fit a positive u32");
                        return None;
                    };
                    if count == 0 {
                        self.instance_refusal(span, "`last` must be positive");
                        return None;
                    }
                    join_last = Some(count);
                }
                crate::compiler::syntax::Guard::By(expr) => {
                    self.instance_refusal(
                        expr.span,
                        "join `by` was removed; put the correlation expression in `instance = { key = ... }` on this Operator",
                    );
                }
                crate::compiler::syntax::Guard::Having(expr) => {
                    self.deferred("a join aggregate `having` clause", expr.span);
                }
                crate::compiler::syntax::Guard::Within(expr) => {
                    self.instance_refusal(expr.span, "`within` is legal only once on a join");
                }
                crate::compiler::syntax::Guard::Last(count) => {
                    self.instance_refusal(count.span, "`last` is legal only once on a join");
                }
            }
        }
        if matches!(&activation, node::HandlerActivation::Join { .. })
            && where_guard.is_some()
            && join_within.is_none()
            && join_last.is_none()
        {
            self.instance_refusal(
                decl.span,
                "a content-predicate join requires `within` or `last` to bound retained candidates",
            );
            return None;
        }
        if let node::HandlerActivation::Join {
            within_ns, last, ..
        } = &mut activation
        {
            *within_ns = join_within.as_ref().and_then(|bound| match &bound.value {
                crate::value::Value::Duration(duration) => Some(*duration),
                _ => None,
            });
            *last = join_last;
        }
        if matches!(
            &activation,
            node::HandlerActivation::Join {
                within_ns: Some(_),
                ..
            } | node::HandlerActivation::Join { last: Some(_), .. }
        ) {
            let Ok(expiry) = node::Port::new("expired") else {
                return None;
            };
            let expected = node::join_expired_type().ok();
            let declared = node.ports.iter().find(|port| port.port == expiry);
            if !declared.is_some_and(|port| {
                port.direction == Direction::Output && port.ty.as_ref() == expected.as_ref()
            }) {
                self.instance_refusal(
                    decl.span,
                    "a bounded join declares `output expired` with the one-case `JoinExpired` tagged-union type",
                );
                return None;
            }
        }
        let is_join = matches!(&activation, node::HandlerActivation::Join { .. });
        if is_join && let Some(guard) = &where_guard {
            signature = signature.narrowed_by(&guard.root);
        }
        if !is_join
            && where_guard.is_some()
            && !matches!(
                &activation,
                node::HandlerActivation::Port(_) | node::HandlerActivation::ChildPort { .. }
            )
        {
            self.bag.push(
                Diagnostic::new(
                    E_DISPATCH_POSITION,
                    Phase::Check,
                    "`where` is a dispatch filter only on a single input port; joins retain it as an admission condition",
                )
                .with_span(decl.span),
            );
            return None;
        }
        let Some(source_dispatch) = &decl.dispatch else {
            self.deferred("a handler without a dispatch", decl.span);
            return None;
        };
        let dispatch_signature = if !is_join {
            where_guard.as_ref().map_or_else(
                || signature.clone(),
                |guard| signature.narrowed_by(&guard.root),
            )
        } else {
            signature.clone()
        };
        let mut dispatch = self.checked_dispatch(
            source_dispatch,
            &dispatch_signature,
            node,
            consts,
            &activation,
            declared_data_type.as_ref(),
        )?;
        let admission_guard = if is_join {
            where_guard
        } else {
            if let Some(predicate) = where_guard {
                dispatch = Dispatch::OpenMatch {
                    prefix: Vec::new(),
                    arms: vec![OpenArm {
                        predicate,
                        dispatch: Box::new(dispatch),
                    }],
                    fallback: Box::new(Dispatch::Linear(Vec::new())),
                };
            }
            None
        };
        let depth = dispatch_depth(&dispatch);
        if depth > crate::compiler::syntax::MAX_DISPATCH_DEPTH {
            self.bag.push(
                Diagnostic::new(
                    crate::compiler::parser::E_DISPATCH_DEPTH,
                    Phase::Check,
                    "the desugared Dispatch exceeds its depth budget (`where` counts as one match)",
                )
                .with_span(decl.span)
                .with_field("depth", depth.to_string())
                .with_field(
                    "budget",
                    crate::compiler::syntax::MAX_DISPATCH_DEPTH.to_string(),
                ),
            );
            return None;
        }
        if let Err(error) = crate::plan::validate_dispatch(&dispatch, &signature) {
            self.bag.push(
                Diagnostic::new(
                    crate::compiler::E_INTERNAL_PLAN,
                    Phase::Check,
                    "checked Dispatch contradicts the independent core proof",
                )
                .with_span(decl.span)
                .with_field("source", node.path.to_string())
                .with_field("path", error.path.clone())
                .with_field("detail", error.to_string()),
            );
            return None;
        }
        Some(Handler {
            activation,
            join_within,
            admission_guard,
            dispatch,
        })
    }

    pub(super) fn checked_dispatch(
        &mut self,
        dispatch: &SynDispatch,
        signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation: &node::HandlerActivation,
        declared_data_type: Option<&DataType>,
    ) -> Option<Dispatch> {
        match dispatch {
            SynDispatch::Linear { statements, .. } => {
                let mut signature = signature.clone();
                Some(Dispatch::Linear(self.checked_statements(
                    statements,
                    &mut signature,
                    node,
                    consts,
                    activation,
                    declared_data_type,
                )))
            }
            SynDispatch::Match {
                prefix,
                scrutinee,
                arms,
                span,
            } => {
                if let Some(effectful) = prefix
                    .iter()
                    .find(|statement| !matches!(statement, SynStatement::Val(_)))
                {
                    self.bag.push(
                        Diagnostic::new(
                            E_DISPATCH_POSITION,
                            Phase::Check,
                            "only pure `val` declarations may precede a terminal `match`",
                        )
                        .with_span(syntax_statement_span(effectful)),
                    );
                    return None;
                }
                let mut match_signature = signature.clone();
                let checked_prefix = self.checked_statements(
                    prefix,
                    &mut match_signature,
                    node,
                    consts,
                    activation,
                    declared_data_type,
                );
                match scrutinee {
                    Some(scrutinee) => self.checked_closed_match(
                        checked_prefix,
                        scrutinee,
                        arms,
                        *span,
                        &match_signature,
                        node,
                        consts,
                        activation,
                        declared_data_type,
                    ),
                    None => self.checked_open_match(
                        checked_prefix,
                        arms,
                        *span,
                        &match_signature,
                        node,
                        consts,
                        activation,
                        declared_data_type,
                    ),
                }
            }
        }
    }

    pub(super) fn checked_statements(
        &mut self,
        body: &[SynStatement],
        signature: &mut expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation: &node::HandlerActivation,
        declared_data_type: Option<&DataType>,
    ) -> Vec<Statement> {
        let mut statements = Vec::new();
        for statement in body {
            match statement {
                SynStatement::Val(val) => {
                    if self.bindable(&val.name, signature) {
                        let (expected, annotations) = match &val.ty {
                            Some(ty) => match self.resolve_executable_schema(ty) {
                                Some(schema) => {
                                    (Some(schema.ty().clone()), schema.annotations().clone())
                                }
                                None => (None, AnnotationTable::empty()),
                            },
                            None => (None, AnnotationTable::empty()),
                        };
                        if val.ty.is_some() && expected.is_none() {
                            continue;
                        }
                        if let Some(tree) = self.typed_value_tree(
                            &val.value,
                            signature,
                            expected.as_ref(),
                            val.span,
                            &val.name.text,
                        ) {
                            *signature = signature
                                .clone()
                                .with(val.name.text.clone(), tree.ty.clone());
                            statements.push(Statement::Let {
                                name: val.name.text.clone(),
                                value: tree,
                                annotations,
                            });
                        }
                    }
                }
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
                            // The parser already spoke for its
                            // recovery form.
                            SynValue::Error(_) => {}
                            _ => {
                                let tree = if let SynValue::Expr(source) = value {
                                    let raw = self.expr(source, signature);
                                    match raw {
                                        Some(tree)
                                            if declared_data_type.is_none()
                                                && tree.ty == DataType::Variant
                                                && expression_reads_data(&tree.root) =>
                                        {
                                            Some(tree)
                                        }
                                        Some(tree) => self.contextualize_tree(
                                            tree,
                                            &var.ty,
                                            target.span,
                                            &name.text,
                                        ),
                                        None => None,
                                    }
                                } else {
                                    self.typed_value_tree(
                                        value,
                                        signature,
                                        Some(&var.ty),
                                        target.span,
                                        &name.text,
                                    )
                                };
                                if let Some(tree) = tree {
                                    statements.push(Statement::SetVar {
                                        name: name.text.clone(),
                                        value: tree,
                                    });
                                }
                            }
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
                    let expected = node_port_pin(node, &port);
                    if let Some(emit) =
                        self.emit_value(value, signature, expected.as_ref(), name.span, port.name())
                    {
                        if matches!(&activation, node::HandlerActivation::Join { .. })
                            && matches!(&emit, EmitValue::Identity)
                        {
                            self.instance_refusal(
                                name.span,
                                "`out = .` is ambiguous under a join; select a named input explicitly",
                            );
                            continue;
                        }
                        // A pinned output holds its emissions to the
                        // contract — ONE conformance notion, every
                        // EmitValue form.
                        if let Some(expected) = expected
                            && !self.port_conforms(
                                &expected,
                                &emit,
                                BuildOrigin::Handler(declared_data_type),
                                consts,
                                port.name(),
                                name.span,
                            )
                        {
                            continue;
                        }
                        statements.push(Statement::Emit { port, value: emit });
                    }
                }
                SynStatement::MultiEmit { targets, values } => {
                    if let Some(checked) = self.checked_multi_emit(
                        targets,
                        values,
                        signature,
                        node,
                        consts,
                        activation,
                        declared_data_type,
                    ) {
                        statements.push(checked);
                    }
                }
                SynStatement::LoopNext { value, span } => {
                    if !node.extensions.contains(&node::Extension::Loop) {
                        self.loop_refusal(
                            *span,
                            "`Loop.next` requires the owner to declare `+ Loop`",
                        );
                    } else {
                        let expected = resolved_loop_entry_type(node);
                        if let Some(value) =
                            self.emit_value(value, signature, expected.as_ref(), *span, "Loop.next")
                        {
                            statements.push(Statement::LoopCommand(LoopCommand::Next(value)));
                        }
                    }
                }
                SynStatement::LoopAgain(span) => {
                    if node.extensions.contains(&node::Extension::Loop) {
                        statements.push(Statement::LoopCommand(LoopCommand::Again));
                    } else {
                        self.loop_refusal(
                            *span,
                            "`Loop.again` requires the owner to declare `+ Loop`",
                        );
                    }
                }
                SynStatement::LoopDone(span) => {
                    if node.extensions.contains(&node::Extension::Loop) {
                        statements.push(Statement::LoopCommand(LoopCommand::Done));
                    } else {
                        self.loop_refusal(
                            *span,
                            "`Loop.done` requires the owner to declare `+ Loop`",
                        );
                    }
                }
                // `nothing` is the explicit source spelling of an empty
                // linear leaf. Erasing it here makes the normative `where`
                // desugaring byte-for-byte equal to `_ => nothing`.
                SynStatement::Nothing(_) => {}
                SynStatement::Stop(Some(target)) => self.instance_refusal(
                    target.span,
                    "targeted `stop` was removed; bare `stop` closes the current Node instance",
                ),
                SynStatement::Stop(None) => {
                    if *activation == node::HandlerActivation::Lifecycle(node::Lifecycle::Stop) {
                        self.instance_refusal(
                            syntax_statement_span(statement),
                            "an `on stop` handler cannot request closure again",
                        );
                    } else {
                        statements.push(Statement::Stop);
                    }
                }
                SynStatement::Error(_) => {}
            }
        }
        statements
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn checked_closed_match(
        &mut self,
        prefix: Vec<Statement>,
        source: &ExprSource,
        arms: &[crate::compiler::syntax::MatchArm],
        span: Span,
        signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation: &node::HandlerActivation,
        declared_data_type: Option<&DataType>,
    ) -> Option<Dispatch> {
        let scrutinee = self.expr(source, signature)?;
        let Some(domain) = closed_case_domain(&scrutinee.ty) else {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_NOT_CLOSED,
                    Phase::Check,
                    format!(
                        "a closed `match` requires a tagged union, enum, or bool; this scrutinee is {}",
                        scrutinee.ty
                    ),
                )
                .with_span(source.span)
                .with_field("source", node.path.to_string())
                .with_field("type", scrutinee.ty.to_string()),
            );
            return None;
        };
        let domain_set: BTreeSet<_> = domain.iter().cloned().collect();
        let mut selected = BTreeSet::new();
        let mut explicit = Vec::new();
        let mut fallback = None;
        for arm in arms {
            match &arm.selector {
                MatchSelector::Cases(cases) => {
                    let mut names = Vec::new();
                    for case in cases {
                        if !domain_set.contains(&case.text) {
                            self.bag.push(
                                Diagnostic::new(
                                    E_MATCH_NONEXHAUSTIVE,
                                    Phase::Check,
                                    format!(
                                        "`{}` is not a case of the closed match domain",
                                        case.text
                                    ),
                                )
                                .with_span(case.span)
                                .with_field("source", node.path.to_string())
                                .with_field("case", case.text.clone()),
                            );
                            continue;
                        }
                        if !selected.insert(case.text.clone()) {
                            self.bag.push(
                                Diagnostic::new(
                                    E_MATCH_OVERLAP,
                                    Phase::Check,
                                    format!(
                                        "closed match case `{}` appears in two arms",
                                        case.text
                                    ),
                                )
                                .with_span(case.span)
                                .with_field("source", node.path.to_string())
                                .with_field("case", case.text.clone()),
                            );
                            continue;
                        }
                        names.push(case.text.clone());
                    }
                    names.sort();
                    names.dedup();
                    if !names.is_empty() {
                        explicit.push((names, arm));
                    }
                }
                MatchSelector::Fallback(fallback_span) => {
                    if fallback.replace(arm).is_some() {
                        self.bag.push(
                            Diagnostic::new(
                                E_MATCH_OVERLAP,
                                Phase::Check,
                                "a closed match has only one `_` remainder arm",
                            )
                            .with_span(*fallback_span)
                            .with_field("source", node.path.to_string())
                            .with_field("case", "_"),
                        );
                    }
                }
                MatchSelector::Predicate(predicate) => {
                    self.bag.push(
                        Diagnostic::new(
                            E_MATCH_NOT_CLOSED,
                            Phase::Check,
                            "a closed match arm names cases; predicates belong to `match { ... }` without a scrutinee",
                        )
                        .with_span(predicate.span)
                        .with_field("source", node.path.to_string())
                        .with_field("type", scrutinee.ty.to_string()),
                    );
                }
            }
        }
        let remaining: Vec<_> = domain
            .iter()
            .filter(|case| !selected.contains(*case))
            .cloned()
            .collect();
        if fallback.is_none() && !remaining.is_empty() {
            for case in &remaining {
                self.bag.push(
                    Diagnostic::new(
                        E_MATCH_NONEXHAUSTIVE,
                        Phase::Check,
                        format!("closed match leaves case `{case}` uncovered"),
                    )
                    .with_span(span)
                    .with_field("source", node.path.to_string())
                    .with_field("case", case.clone()),
                );
            }
            return None;
        }
        if fallback.is_some() && remaining.is_empty() {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_OVERLAP,
                    Phase::Check,
                    "the `_` arm covers no remaining closed case",
                )
                .with_span(span)
                .with_field("source", node.path.to_string())
                .with_field("case", "_"),
            );
            return None;
        }
        let mut checked = Vec::new();
        for (cases, arm) in explicit {
            let arm_signature = closed_arm_signature(signature, &scrutinee, &cases);
            let dispatch = self.checked_dispatch(
                &arm.dispatch,
                &arm_signature,
                node,
                consts,
                activation,
                declared_data_type,
            )?;
            checked.push(ClosedArm {
                cases,
                dispatch: Box::new(dispatch),
            });
        }
        if let Some(arm) = fallback {
            let arm_signature = closed_arm_signature(signature, &scrutinee, &remaining);
            let dispatch = self.checked_dispatch(
                &arm.dispatch,
                &arm_signature,
                node,
                consts,
                activation,
                declared_data_type,
            )?;
            checked.push(ClosedArm {
                cases: remaining,
                dispatch: Box::new(dispatch),
            });
        }
        checked.sort_by(|left, right| left.cases.cmp(&right.cases));
        self.verify_case_partition(
            &scrutinee.ty,
            checked.iter().map(|arm| arm.cases.as_slice()),
            span,
            &node.path.to_string(),
        )?;
        Some(Dispatch::ClosedMatch {
            prefix,
            scrutinee,
            arms: checked,
        })
    }

    /// Source diagnostics retain their authored spans above. The normalized
    /// table must also satisfy the independent core proof; failure here is
    /// a compiler defect, not another source-language fallback.
    pub(super) fn verify_case_partition<'a>(
        &mut self,
        ty: &DataType,
        arms: impl IntoIterator<Item = &'a [String]>,
        span: Span,
        source: &str,
    ) -> Option<()> {
        match crate::plan::validate_case_partition(ty, arms) {
            Ok(()) => Some(()),
            Err(error) => {
                self.bag.push(
                    Diagnostic::new(
                        crate::compiler::E_INTERNAL_PLAN,
                        Phase::Check,
                        "checked closed match has an invalid case partition",
                    )
                    .with_span(span)
                    .with_field("source", source)
                    .with_field("detail", error.to_string()),
                );
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn checked_open_match(
        &mut self,
        prefix: Vec<Statement>,
        arms: &[crate::compiler::syntax::MatchArm],
        span: Span,
        signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation: &node::HandlerActivation,
        declared_data_type: Option<&DataType>,
    ) -> Option<Dispatch> {
        let mut checked = Vec::new();
        let mut fallback = None;
        for (index, arm) in arms.iter().enumerate() {
            match &arm.selector {
                MatchSelector::Predicate(source) => {
                    if fallback.is_some() {
                        self.bag.push(
                            Diagnostic::new(
                                E_DISPATCH_POSITION,
                                Phase::Check,
                                "the `_` fallback is the final arm of an open match",
                            )
                            .with_span(source.span),
                        );
                        return None;
                    }
                    let predicate = self.expr(source, signature)?;
                    if !matches!(predicate.ty, DataType::Bool | DataType::Variant) {
                        self.bag.push(
                            Diagnostic::new(
                                E_EXPR,
                                Phase::Check,
                                "an open match predicate must be a bool",
                            )
                            .with_span(source.span)
                            .with_field("detail", "the predicate type is not bool"),
                        );
                        return None;
                    }
                    let narrowed = signature.narrowed_by(&predicate.root);
                    let dispatch = self.checked_dispatch(
                        &arm.dispatch,
                        &narrowed,
                        node,
                        consts,
                        activation,
                        declared_data_type,
                    )?;
                    checked.push(OpenArm {
                        predicate,
                        dispatch: Box::new(dispatch),
                    });
                }
                MatchSelector::Fallback(fallback_span) => {
                    if fallback.is_some() {
                        self.bag.push(
                            Diagnostic::new(
                                E_MATCH_OVERLAP,
                                Phase::Check,
                                "an open match has exactly one `_` fallback",
                            )
                            .with_span(*fallback_span)
                            .with_field("source", node.path.to_string())
                            .with_field("case", "_"),
                        );
                        return None;
                    }
                    if index + 1 != arms.len() {
                        self.bag.push(
                            Diagnostic::new(
                                E_DISPATCH_POSITION,
                                Phase::Check,
                                "the `_` fallback is the final arm of an open match",
                            )
                            .with_span(*fallback_span),
                        );
                        return None;
                    }
                    fallback = Some(self.checked_dispatch(
                        &arm.dispatch,
                        signature,
                        node,
                        consts,
                        activation,
                        declared_data_type,
                    )?);
                }
                MatchSelector::Cases(cases) => {
                    let case = cases.first().map_or("<empty>", |case| case.text.as_str());
                    self.bag.push(
                        Diagnostic::new(
                            E_MATCH_NOT_CLOSED,
                            Phase::Check,
                            "an open match arm is a boolean predicate; symbolic cases require `match <expr>`",
                        )
                        .with_span(arm.span)
                        .with_field("source", node.path.to_string())
                        .with_field("type", case),
                    );
                    return None;
                }
            }
        }
        let Some(fallback) = fallback else {
            self.bag.push(
                Diagnostic::new(
                    E_MATCH_NONEXHAUSTIVE,
                    Phase::Check,
                    "an open match requires a final `_` fallback",
                )
                .with_span(span)
                .with_field("source", node.path.to_string())
                .with_field("case", "_"),
            );
            return None;
        };
        Some(Dispatch::OpenMatch {
            prefix,
            arms: checked,
            fallback: Box::new(fallback),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn checked_multi_emit(
        &mut self,
        targets: &[crate::compiler::syntax::LValue],
        values: &[SynValue],
        signature: &expr::Signature,
        node: &Node,
        consts: &BTreeMap<String, crate::value::Value>,
        activation: &node::HandlerActivation,
        declared_data_type: Option<&DataType>,
    ) -> Option<Statement> {
        if values.len() != 1 && values.len() != targets.len() {
            let span = targets
                .first()
                .map_or_else(|| Span::new(SourceId::new(0), 0..0), |target| target.span);
            self.bag.push(
                Diagnostic::new(
                    E_MULTI_EMIT_ARITY,
                    Phase::Check,
                    format!(
                        "this atomic multi-emission has {} targets but {} tuple values",
                        targets.len(),
                        values.len()
                    ),
                )
                .with_span(span)
                .with_field("targets", targets.len().to_string())
                .with_field("values", values.len().to_string()),
            );
            return None;
        }
        let mut seen = BTreeSet::new();
        let mut checked_targets = Vec::new();
        for target in targets {
            if target.segments.len() != 1 {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_DIRECTION,
                        Phase::Check,
                        "an atomic multi-emission target is one ordinary output port",
                    )
                    .with_span(target.span),
                );
                return None;
            }
            let name = &target.segments[0];
            if !seen.insert(name.text.clone()) {
                self.bag.push(
                    Diagnostic::new(
                        E_MULTI_EMIT_DUPLICATE,
                        Phase::Check,
                        format!(
                            "output `{}` appears twice in one atomic multi-emission",
                            name.text
                        ),
                    )
                    .with_span(name.span)
                    .with_field("port", name.text.clone()),
                );
                return None;
            }
            if node.vars.iter().any(|var| var.name == name.text) {
                self.bag.push(
                    Diagnostic::new(
                        E_PORT_DIRECTION,
                        Phase::Check,
                        "an atomic multi-emission cannot target mutable state",
                    )
                    .with_span(name.span)
                    .with_field("port", name.text.clone()),
                );
                return None;
            }
            let Some(direction) = self.node_port_direction(node, &name.text) else {
                self.bag.push(
                    Diagnostic::new(
                        E_RESOLVE_UNKNOWN,
                        Phase::Resolve,
                        format!("this node exposes no port named `{}`", name.text),
                    )
                    .with_span(name.span)
                    .with_field("name", name.text.clone()),
                );
                return None;
            };
            if direction != Direction::Output {
                self.wrong_direction(
                    name.span,
                    &name.text,
                    direction,
                    "an atomic multi-emission targets output ports",
                );
                return None;
            }
            let port = node::Port::new(name.text.clone()).ok()?;
            checked_targets.push((port.clone(), node_port_pin(node, &port), name.span));
        }
        let check_value = |checker: &mut Self,
                           value: &SynValue,
                           expected: Option<&DataType>,
                           span: Span,
                           place: &str|
         -> Option<EmitValue> {
            let emit = checker.emit_value(value, signature, expected, span, place)?;
            if matches!(activation, node::HandlerActivation::Join { .. })
                && matches!(emit, EmitValue::Identity)
            {
                checker.instance_refusal(
                    span,
                    "identity broadcast is ambiguous under a join; select a named input explicitly",
                );
                return None;
            }
            Some(emit)
        };
        let checked_values = if values.len() == 1 {
            let (first_port, first_expected, first_span) = checked_targets.first()?;
            let emit = check_value(
                self,
                &values[0],
                first_expected.as_ref(),
                *first_span,
                first_port.name(),
            )?;
            for (port, expected, span) in &checked_targets {
                if let Some(expected) = expected
                    && !self.port_conforms(
                        expected,
                        &emit,
                        BuildOrigin::Handler(declared_data_type),
                        consts,
                        port.name(),
                        *span,
                    )
                {
                    return None;
                }
            }
            MultiEmitValues::Broadcast(emit)
        } else {
            let mut emits = Vec::with_capacity(values.len());
            for (value, (port, expected, span)) in values.iter().zip(&checked_targets) {
                let emit = check_value(self, value, expected.as_ref(), *span, port.name())?;
                if let Some(expected) = expected
                    && !self.port_conforms(
                        expected,
                        &emit,
                        BuildOrigin::Handler(declared_data_type),
                        consts,
                        port.name(),
                        *span,
                    )
                {
                    return None;
                }
                emits.push(emit);
            }
            MultiEmitValues::Tuple(emits)
        };
        Some(Statement::MultiEmit {
            ports: checked_targets
                .into_iter()
                .map(|(port, _, _)| port)
                .collect(),
            values: checked_values,
        })
    }
}

/// Each selected leaf is a whole reply or explicit silence. Counting exits
/// across sibling arms would invent a completion no execution can produce.
pub(super) fn mock_completion_leaves_hold(dispatch: &Dispatch) -> bool {
    match dispatch {
        Dispatch::Linear(statements) => {
            let mut last = None;
            let mut exits = 0;
            for (port, value) in statements.iter().flat_map(Statement::emissions) {
                if !matches!(value, EmitValue::Nothing) {
                    exits += usize::from(port.name() == "exit");
                    last = Some(port);
                }
            }
            last.is_none_or(|port| port.name() == "exit" && exits == 1)
        }
        Dispatch::ClosedMatch { arms, .. } => arms
            .iter()
            .all(|arm| mock_completion_leaves_hold(&arm.dispatch)),
        Dispatch::OpenMatch { arms, fallback, .. } => {
            arms.iter()
                .all(|arm| mock_completion_leaves_hold(&arm.dispatch))
                && mock_completion_leaves_hold(fallback)
        }
    }
}

pub(super) fn collect_hir_statements<'a>(
    dispatch: &'a Dispatch,
    statements: &mut Vec<&'a Statement>,
) {
    match dispatch {
        Dispatch::Linear(linear) => statements.extend(linear),
        Dispatch::ClosedMatch { prefix, arms, .. } => {
            statements.extend(prefix);
            for arm in arms {
                collect_hir_statements(&arm.dispatch, statements);
            }
        }
        Dispatch::OpenMatch {
            prefix,
            arms,
            fallback,
        } => {
            statements.extend(prefix);
            for arm in arms {
                collect_hir_statements(&arm.dispatch, statements);
            }
            collect_hir_statements(fallback, statements);
        }
    }
}

pub(super) fn collect_syntax_statements<'a>(
    dispatch: &'a SynDispatch,
    statements: &mut Vec<&'a SynStatement>,
) {
    match dispatch {
        SynDispatch::Linear {
            statements: linear, ..
        } => statements.extend(linear),
        SynDispatch::Match { prefix, arms, .. } => {
            statements.extend(prefix);
            for arm in arms {
                collect_syntax_statements(&arm.dispatch, statements);
            }
        }
    }
}

pub(super) fn syntax_statement_span(statement: &SynStatement) -> Span {
    match statement {
        SynStatement::Val(value) => value.span,
        SynStatement::Assign { target, .. } => target.span,
        SynStatement::MultiEmit { targets, .. } => targets
            .first()
            .map_or_else(|| Span::new(SourceId::new(0), 0..0), |target| target.span),
        SynStatement::LoopNext { span, .. }
        | SynStatement::LoopAgain(span)
        | SynStatement::LoopDone(span)
        | SynStatement::Nothing(span) => *span,
        SynStatement::Stop(Some(target)) => target.span,
        SynStatement::Stop(None) | SynStatement::Error(_) => match statement {
            SynStatement::Error(error) => error.span,
            _ => Span::new(SourceId::new(0), 0..0),
        },
    }
}

pub(super) fn same_handler_trigger(
    left: &node::HandlerActivation,
    right: &node::HandlerActivation,
) -> bool {
    match (left, right) {
        (node::HandlerActivation::Port(left), node::HandlerActivation::Port(right)) => {
            left == right
        }
        (node::HandlerActivation::Lifecycle(left), node::HandlerActivation::Lifecycle(right)) => {
            left == right
        }
        (
            node::HandlerActivation::ChildPort {
                child: left_child,
                port: left_port,
            },
            node::HandlerActivation::ChildPort {
                child: right_child,
                port: right_port,
            },
        ) => left_child == right_child && left_port == right_port,
        (
            node::HandlerActivation::Join {
                ports: left_ports, ..
            },
            node::HandlerActivation::Join {
                ports: right_ports, ..
            },
        ) => left_ports == right_ports,
        _ => false,
    }
}

pub(super) fn closed_arm_signature(
    signature: &expr::Signature,
    scrutinee: &expr::Tree,
    cases: &[String],
) -> expr::Signature {
    let is_data =
        matches!(&scrutinee.root, expr::Expr::Path(path) if path.len() == 1 && path[0] == "data");
    let DataType::TaggedUnion(union) = &scrutinee.ty else {
        return signature.clone();
    };
    if !is_data {
        return signature.clone();
    }
    let selected: Vec<_> = union
        .cases()
        .iter()
        .filter(|case| cases.iter().any(|name| name == case.name()))
        .cloned()
        .collect();
    let Ok(union) = TaggedUnionType::new(selected) else {
        return signature.clone();
    };
    signature.clone().with("data", DataType::TaggedUnion(union))
}

pub(super) fn dispatch_depth(dispatch: &Dispatch) -> usize {
    let mut pending = vec![(dispatch, 0)];
    let mut deepest = 0;
    while let Some((dispatch, depth)) = pending.pop() {
        match dispatch {
            Dispatch::Linear(_) => {}
            Dispatch::ClosedMatch { arms, .. } => {
                deepest = deepest.max(depth + 1);
                pending.extend(arms.iter().map(|arm| (arm.dispatch.as_ref(), depth + 1)));
            }
            Dispatch::OpenMatch { arms, fallback, .. } => {
                deepest = deepest.max(depth + 1);
                pending.extend(arms.iter().map(|arm| (arm.dispatch.as_ref(), depth + 1)));
                pending.push((fallback, depth + 1));
            }
        }
    }
    deepest
}
