//! Closed Extension admission and structural annotation migrations.

use super::*;

impl Checker<'_, '_> {
    pub(super) fn node_annotation_refusal(&mut self, symbol: &Name, detail: impl Into<String>) {
        let detail = detail.into();
        self.bag.push(
            Diagnostic::new(E_NODE_ANNOTATION, Phase::Check, detail.clone())
                .with_span(symbol.span)
                .with_field("name", symbol.text.clone())
                .with_field("detail", detail),
        );
    }

    pub(super) fn graph_extensions(
        &mut self,
        decl: &GraphDecl,
        is_root: bool,
    ) -> Vec<node::Extension> {
        let symbols = decl
            .annotation
            .as_ref()
            .map_or(&[][..], |annotation| annotation.symbols.as_slice());
        let mut extensions = Vec::new();
        for symbol in symbols {
            if symbol.text == "Graph" {
                self.node_annotation_refusal(
                    symbol,
                    "`Graph` is structural, not an Extension; remove `: Graph`",
                );
                continue;
            }
            let Some(extension) = node::Extension::from_source_name(&symbol.text) else {
                let detail = if symbol.text == "Operator" {
                    "`Operator` is structural and cannot annotate a Graph".to_string()
                } else {
                    format!("`{}` is not a known Graph Extension", symbol.text)
                };
                self.node_annotation_refusal(symbol, detail);
                continue;
            };
            if extensions.contains(&extension) {
                self.node_annotation_refusal(
                    symbol,
                    format!("the `{extension}` Extension is declared twice"),
                );
                continue;
            }
            match extension {
                node::Extension::App if !is_root => {
                    self.node_annotation_refusal(
                        symbol,
                        "`App` is the deployed unit's unique root Extension",
                    );
                    continue;
                }
                node::Extension::Saga => self.deferred("the Saga Extension", symbol.span),
                node::Extension::App | node::Extension::Loop => {}
            }
            extensions.push(extension);
        }
        if is_root && !extensions.contains(&node::Extension::App) {
            self.node_annotation_refusal(
                &decl.name,
                "the deployed root Graph requires the `App` Extension",
            );
        }
        extensions.sort();
        extensions
    }

    pub(super) fn operator_annotation(&mut self, decl: &OperatorDecl) -> Vec<node::Extension> {
        let symbols = decl
            .annotation
            .as_ref()
            .map_or(&[][..], |annotation| annotation.symbols.as_slice());
        let mut seen = Vec::new();
        for symbol in symbols {
            if symbol.text == "Operator" {
                self.node_annotation_refusal(
                    symbol,
                    "`Operator` is structural, not an Extension; remove `: Operator`",
                );
                continue;
            }
            let Some(extension) = node::Extension::from_source_name(&symbol.text) else {
                let detail = if matches!(
                    symbol.text.as_str(),
                    "Reducer" | "Aggregator" | "Window" | "Joiner" | "Router"
                ) {
                    format!(
                        "`{}` is an Operator pattern, not an Extension; write the ordinary `var`, `instance`, `lifetime`, join, and emission primitives it abbreviates",
                        symbol.text
                    )
                } else {
                    format!("`{}` is not a known Operator Extension", symbol.text)
                };
                self.node_annotation_refusal(symbol, detail);
                continue;
            };
            if seen.contains(&extension) {
                self.node_annotation_refusal(
                    symbol,
                    format!("the `{extension}` Extension is declared twice"),
                );
                continue;
            }
            seen.push(extension);
            match extension {
                node::Extension::Loop => {}
                node::Extension::App => self.node_annotation_refusal(
                    symbol,
                    "the `App` Extension belongs only to the root Graph",
                ),
                node::Extension::Saga => self.node_annotation_refusal(
                    symbol,
                    "the future `Saga` Extension belongs to a Graph",
                ),
            }
        }
        seen.retain(|extension| *extension == node::Extension::Loop);
        seen.sort();
        seen
    }

    pub(super) fn connector_extensions(&mut self, symbols: &[Name]) -> Vec<node::Extension> {
        let mut seen = Vec::new();
        for symbol in symbols {
            let Some(extension) = node::Extension::from_source_name(&symbol.text) else {
                self.node_annotation_refusal(
                    symbol,
                    format!("`{}` is not a known Connector Extension", symbol.text),
                );
                continue;
            };
            if seen.contains(&extension) {
                self.node_annotation_refusal(
                    symbol,
                    format!("the `{extension}` Extension is declared twice"),
                );
                continue;
            }
            seen.push(extension);
            match extension {
                node::Extension::Loop => {}
                node::Extension::App => self.node_annotation_refusal(
                    symbol,
                    "the `App` Extension belongs only to the root Graph",
                ),
                node::Extension::Saga => self.node_annotation_refusal(
                    symbol,
                    "the future `Saga` Extension belongs to a Graph",
                ),
            }
        }
        seen.retain(|extension| *extension == node::Extension::Loop);
        seen.sort();
        seen
    }
}
