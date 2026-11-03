//! Terminal assertion reports use source spans and the debugger's safe values.

use crate::compiler::{
    Compilation,
    hir::{ExpectFilter, ExpectTarget, ScenarioStep},
};
use crate::lab::ScenarioReport;

/// Formats failed assertions without re-running the scenario or its effects.
/// Source positions and text come only from the compilation's source table.
/// Classified evidence, including predicate literals, stays hidden.
#[must_use]
pub fn scenario_failures(compilation: &Compilation, report: &ScenarioReport) -> Vec<String> {
    let definition = compilation.scenario_definitions.iter().find(|definition| {
        definition.simulation == report.simulation && definition.scenario == report.scenario
    });
    let scenario = compilation
        .labs
        .iter()
        .flat_map(|lab| &lab.simulations)
        .find(|simulation| simulation.name == report.simulation)
        .and_then(|simulation| {
            simulation
                .scenarios
                .iter()
                .find(|scenario| scenario.name == report.scenario)
        });
    let mut lines = Vec::new();
    for (index, message) in report.failures.iter().enumerate() {
        let Some(failure) = report
            .assertions
            .iter()
            .find(|failure| failure.failure_index == index)
        else {
            lines.push(message.clone());
            continue;
        };
        let source =
            definition.and_then(|definition| definition.expectations.get(&failure.action_index));
        let unit = source.and_then(|source| {
            compilation
                .sources
                .iter()
                .find(|unit| unit.index == source.span.source().index())
        });
        let position = source
            .zip(unit)
            .and_then(|(source, unit)| {
                unit.text.get(..source.span.range().start).map(|prefix| {
                    format!(
                        "({}:{})",
                        unit.name,
                        prefix.bytes().filter(|byte| *byte == b'\n').count() + 1
                    )
                })
            })
            .unwrap_or_else(|| String::from("(source position unavailable)"));
        let Some(observed) = &failure.observed else {
            lines.push(format!("expect [REDACTED]  {position}"));
            lines.push(String::from("observed: [REDACTED]"));
            lines.push(String::from(
                "Assertion evidence is redacted because this program carries classified data.",
            ));
            continue;
        };
        let authored = source
            .zip(unit)
            .and_then(|(source, unit)| unit.text.get(source.span.range()));
        lines.push(format!(
            "{}  {position}",
            authored
                .unwrap_or("expect (source text unavailable)")
                .trim()
        ));
        let Some(ScenarioStep::Expect(expectation)) =
            scenario.and_then(|scenario| scenario.steps.get(failure.action_index))
        else {
            lines.push(String::from("assertion definition unavailable"));
            continue;
        };
        let target = match &expectation.target {
            ExpectTarget::Boundary(port) => format!("app.{port}"),
            ExpectTarget::Node { path, port } => format!("{path}.{port}"),
        };
        if observed.is_empty() {
            lines.push(String::from("nothing observed"));
        } else {
            for data in observed {
                // The same JSON content drives the Assertion inspector, including
                // exact integers, decimals and encoded-but-unmaterialized values.
                let content = data.get("content").unwrap_or(data);
                lines.push(format!("observed {target}: {content}"));
            }
        }
        let expected = expectation.count;
        let Some(matched) = failure.matched else {
            lines.push(format!("{message}; count unknown, expected {expected}"));
            continue;
        };
        let predicate = if matches!(expectation.filter, Some(ExpectFilter::Where(_))) {
            let predicate = source.zip(unit).and_then(|(source, unit)| {
                source
                    .predicate
                    .and_then(|span| unit.text.get(span.range()))
            });
            let predicate = predicate.map_or_else(
                || String::from("predicate (source text unavailable)"),
                |text| format!("where {}", text.trim()),
            );
            format!(
                "{predicate} is false for {} of {} deliveries; ",
                observed.len().saturating_sub(matched),
                observed.len()
            )
        } else {
            String::new()
        };
        if predicate.is_empty() {
            lines.push(message.clone());
        } else {
            lines.push(format!("{predicate}count {matched}, expected {expected}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn unavailable_positions_are_named_instead_of_reconstructed() {
        let mut compilation = crate::compiler::compile(
            &["graph demo : App { input in: text; output out: text; in -> out }\nlab demo { simulation s { scenario failed { app.in = \"seen\"\nexpect 2 app.out } } }"],
            &crate::connector::contract::ContractCatalog::try_m0().unwrap(),
        ).unwrap();
        let run = crate::lab::run_program(
            &compilation.plan,
            compilation.hash,
            crate::plan::GraphVersion(1),
            &compilation.labs[0],
            None,
        )
        .remove(0);
        compilation.scenario_definitions.clear();
        let text = super::scenario_failures(&compilation, &run.report).join("\n");
        assert!(text.contains("(source position unavailable)"), "{text}");
        assert!(text.contains("observed app.out: \"seen\""), "{text}");
        assert!(
            text.contains("expected 2 matching deliveries on app.out, but observed 1"),
            "{text}"
        );
    }
}
