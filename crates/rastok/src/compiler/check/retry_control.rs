//! Plan-constant Retry policies resolved against the exact outbound Call Contract.

use super::*;

/// The exact Contract/profile cannot host Retry or Call idempotence.
pub(super) const E_RETRY_HOST: Code = Code::new("E-RETRY-HOST");
/// The selected terminal set is malformed, duplicated or outside the Contract.
const E_RETRY_CASES: Code = Code::new("E-RETRY-CASES");
/// A selected terminal cannot justify repetition without idempotence.
const E_RETRY_EFFECT: Code = Code::new("E-RETRY-EFFECT");
/// Extension contributions have no admitted same-owner composition.
const E_EXTENSION_CONFLICT: Code = Code::new("E-EXTENSION-CONFLICT");

pub(super) fn retry_attribute(attribute: &SynAttribute) -> bool {
    attribute
        .target
        .segments
        .first()
        .is_some_and(|name| name.text == "Retry")
}

fn retry_symbols(symbols: &[Name]) -> bool {
    symbols.iter().any(|name| name.text == "Retry")
}

fn body_has_retry(body: &[OperatorItem]) -> bool {
    body.iter().any(
        |item| matches!(item, OperatorItem::Attribute(attribute) if retry_attribute(attribute)),
    )
}

impl Checker<'_, '_> {
    fn retry_val(
        &mut self,
        val: &ValDecl,
        signature: &mut expr::Signature,
        consts: &mut BTreeMap<String, crate::value::Value>,
    ) {
        if self.bindable(&val.name, signature)
            && let Some((value, _)) = self.graph_val(val, signature, consts)
            && let Some(ty) = self.constant_binding_type(&value, val.span)
        {
            *signature = signature.clone().with(val.name.text.clone(), ty);
            consts.insert(val.name.text.clone(), value.value);
        }
    }

    pub(super) fn retry_connector(
        &mut self,
        decl: &ConnectorDecl,
        path: &node::Path,
        inherited_signature: &expr::Signature,
        inherited_consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<crate::compiler::hir::RetrySpec> {
        if !retry_symbols(&decl.annotation.symbols) && !body_has_retry(&decl.body) {
            return None;
        }
        let mut attempts = None;
        let mut delay = None;
        let mut selected = None;
        let before = self.bag.diagnostics().len();
        let symbols = &decl.annotation.symbols;
        let owns_retry = retry_symbols(symbols);
        let mut seen = BTreeSet::new();
        for (index, symbol) in symbols.iter().enumerate() {
            if !seen.insert(symbol.text.as_str()) {
                self.node_annotation_refusal(
                    symbol,
                    format!("`{}` is declared twice", symbol.text),
                );
                continue;
            }
            if index > 0 && !matches!(symbol.text.as_str(), "Retry" | "Loop") {
                self.node_annotation_refusal(
                    symbol,
                    format!("`{}` is not an admitted Connector Extension", symbol.text),
                );
            }
        }
        if owns_retry && symbols.iter().any(|s| s.text == "Loop") {
            let detail = "Loop and Retry on the same owner are not admitted; put the Retry Connector inside a Loop Graph";
            self.bag.push(
                Diagnostic::new(E_EXTENSION_CONFLICT, Phase::Check, detail)
                    .with_span(decl.name.span)
                    .with_field("owner", path.to_string())
                    .with_field("left", "Loop")
                    .with_field("right", "Retry")
                    .with_field("detail", detail),
            );
            return None;
        }
        let symbol = symbols.first()?;
        let contract::Availability::Available(sheet) = self.registry.connector(&symbol.text) else {
            self.retry_refusal(
                E_RETRY_HOST,
                symbol.span,
                "Retry/idempotence requires an available exact outbound Call Contract",
            );
            return None;
        };
        let mut attributes = BTreeMap::new();
        for item in &decl.body {
            let OperatorItem::Attribute(attribute) = item else {
                continue;
            };
            let name = attribute
                .target
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(".");
            if attributes.insert(name.clone(), attribute).is_some() {
                self.retry_refusal(
                    E_RETRY_ATTRIBUTE,
                    attribute.span,
                    &format!("`{name}` is declared twice"),
                );
            }
        }
        let mode = if let Some(selector) = &sheet.selector {
            if let Some(attribute) = attributes.get(selector.attribute) {
                let SynValue::Expr(source) = &attribute.value else {
                    self.retry_refusal(
                        E_RETRY_HOST,
                        attribute.span,
                        "the Call profile must be a bare selector word",
                    );
                    return None;
                };
                Some(source.text.as_str())
            } else {
                selector.default
            }
        } else {
            None
        };
        let profile = mode.and_then(|word| sheet.profile(word));
        let (id, roles, exchange) = profile
            .and_then(|p| p.protocol)
            .map_or((sheet.id, sheet.roles, sheet.exchange), |p| {
                (p.id, p.roles, p.exchange)
            });
        let Ok(id) = crate::connector::ContractId::parse(id) else {
            self.retry_refusal(
                E_RETRY_HOST,
                symbol.span,
                "invalid exact Call Contract identity",
            );
            return None;
        };
        let Some(call) = crate::connector::outbound_call_semantics(&id, roles, exchange, mode)
        else {
            self.retry_refusal(E_RETRY_HOST, decl.name.span, "Retry/idempotence requires a finite outbound Call: Process job, File read/replace or HttpClient; no Event, Session or inbound Call");
            return None;
        };
        let fixed_guarantee = id.as_str() == crate::connector::stdlib::FILE_CONNECTOR_ID;
        let mut idempotent = fixed_guarantee;
        let mut signature = inherited_signature.clone();
        let mut consts = inherited_consts.clone();
        for item in &decl.body {
            let attribute = match item {
                OperatorItem::Val(val) => {
                    self.retry_val(val, &mut signature, &mut consts);
                    continue;
                }
                OperatorItem::Attribute(attribute) => attribute,
                _ => continue,
            };
            let name = attribute
                .target
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(".");
            match name.as_str() {
                "idempotent" => {
                    if let Some(value) = self.retry_constant(attribute, &signature, &consts) {
                        match value.value {
                            crate::value::Value::Bool(value) if !fixed_guarantee || value => idempotent = value,
                            crate::value::Value::Bool(_) => self.retry_refusal(E_RETRY_ATTRIBUTE, attribute.span, "File fixes idempotent = true; a contradictory override is not admitted"),
                            _ => self.retry_refusal(E_RETRY_ATTRIBUTE, attribute.span, "`idempotent` must be a plan-constant boolean"),
                        }
                    }
                }
                "Retry.attempts" if owns_retry => {
                    if let Some(value) = self.retry_constant(attribute, &signature, &consts) {
                        if matches!(value.value, crate::value::Value::Int(n) if n > 0)
                            || matches!(value.value, crate::value::Value::Uint(n) if n > 0)
                        {
                            attempts = Some(value);
                        } else {
                            self.retry_refusal(E_RETRY_ATTRIBUTE, attribute.span,
                                "`Retry.attempts` must be a positive integer, first attempt included");
                        }
                    }
                }
                "Retry.on" if owns_retry => {}
                "Retry.delay" if owns_retry => {
                    delay = self.extension_delay(
                        attribute,
                        &signature,
                        &consts,
                        "Retry",
                        E_RETRY_ATTRIBUTE,
                    );
                }
                "timeout" if owns_retry => {
                    // C3 must also prove dynamic instance-bound timeouts. The
                    // source gate qualifies constants without inventing that proof.
                    if let Some(value) = self.retry_constant(attribute, &signature, &consts)
                        && !matches!(value.value, crate::value::Value::Duration(n) if n > 0)
                    {
                        self.retry_refusal(
                            E_RETRY_ATTRIBUTE,
                            attribute.span,
                            "Retry requires a positive Call timeout",
                        );
                    }
                }
                _ if retry_attribute(attribute) => {
                    let detail = if !owns_retry && name.starts_with("Retry.") {
                        "Retry attributes require this Connector's `+ Retry` Extension".to_string()
                    } else {
                        format!(
                            "unknown `{name}`; expected Retry.attempts, Retry.on or Retry.delay; Retry has no command, port or state binding"
                        )
                    };
                    self.retry_refusal(E_RETRY_ATTRIBUTE, attribute.span, &detail);
                }
                _ => {}
            }
        }
        if owns_retry {
            for required in ["Retry.attempts", "Retry.on", "timeout"] {
                if !attributes.contains_key(required) {
                    self.retry_refusal(
                        E_RETRY_ATTRIBUTE,
                        decl.name.span,
                        &format!("Retry requires `{required}`"),
                    );
                }
            }
            if let Some(attribute) = attributes.get("Retry.on")
                && let Some(cases) = self.retry_case_names(attribute, call)
            {
                if let Err(error) = call.validate_retry_cases(&cases, idempotent) {
                    if let crate::connector::RetryCaseError::Unsafe { case, visibility } = &error {
                        self.bag.push(
                            Diagnostic::new(E_RETRY_EFFECT, Phase::Check, error.to_string())
                                .with_span(attribute.span)
                                .with_field("owner", path.to_string())
                                .with_field("case", case)
                                .with_field("visibility", visibility.to_string())
                                .with_field("idempotent", idempotent.to_string())
                                .with_field("detail", error.to_string()),
                        );
                    } else {
                        self.retry_refusal(E_RETRY_CASES, attribute.span, &error.to_string());
                    }
                }
                selected = Some(cases);
            }
        }
        if self.bag.diagnostics().len() != before {
            return None;
        }
        Some(crate::compiler::hir::RetrySpec {
            attempts: attempts?,
            cases: selected?,
            delay,
            entry: node::Port::new(call.request()).ok()?,
            terminal: node::Port::new(call.terminal()).ok()?,
        })
    }

    fn retry_constant(
        &mut self,
        attribute: &SynAttribute,
        signature: &expr::Signature,
        consts: &BTreeMap<String, crate::value::Value>,
    ) -> Option<Const> {
        let tree = self.typed_value_tree(
            &attribute.value,
            signature,
            None,
            attribute.span,
            "Retry policy",
        )?;
        self.reduce_constant(tree, attribute.span, consts)
    }

    fn retry_case_names(
        &mut self,
        attribute: &SynAttribute,
        call: crate::connector::CallSemantics,
    ) -> Option<Vec<String>> {
        let SynValue::Expr(source) = &attribute.value else {
            self.retry_refusal(
                E_RETRY_CASES,
                attribute.span,
                "Retry.on is a static list of terminal case names, without predicates",
            );
            return None;
        };
        let mut signature = expr::Signature::new();
        for (case, _) in call.terminal_cases() {
            signature = signature.with(*case, DataType::Text);
        }
        let tree = match expr::compile(&source.text, &signature) {
            Ok(tree) => tree,
            Err(errors) => {
                let detail = errors
                    .iter()
                    .map(|error| error.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                self.retry_refusal(
                    E_RETRY_CASES,
                    attribute.span,
                    &format!("Retry.on requires terminal case names: {detail}"),
                );
                return None;
            }
        };
        let expr::Expr::List(items) = tree.root else {
            self.retry_refusal(
                E_RETRY_CASES,
                attribute.span,
                "Retry.on is a static list of terminal case names",
            );
            return None;
        };
        let mut cases = BTreeSet::new();
        for item in items {
            let expr::Expr::Path(path) = item else {
                self.retry_refusal(
                    E_RETRY_CASES,
                    attribute.span,
                    "Retry.on items are bare terminal names, not strings or predicates",
                );
                return None;
            };
            let [case] = path.as_slice() else {
                self.retry_refusal(
                    E_RETRY_CASES,
                    attribute.span,
                    "Retry.on items are unqualified terminal names",
                );
                return None;
            };
            if !cases.insert(case.clone()) {
                self.retry_refusal(
                    E_RETRY_CASES,
                    attribute.span,
                    &format!("`{case}` appears twice in Retry.on"),
                );
                return None;
            }
        }
        Some(cases.into_iter().collect())
    }

    pub(super) fn retry_refusal(&mut self, code: Code, span: Span, detail: &str) {
        self.bag.push(
            Diagnostic::new(code, Phase::Check, detail)
                .with_span(span)
                .with_field("detail", detail),
        );
    }
}
