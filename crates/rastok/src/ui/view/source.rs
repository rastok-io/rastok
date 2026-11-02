//! Source and variable projections consume retained evidence only.

use super::*;
use crate::compiler::SourceUnit;

pub(super) fn units(
    plan: &ValidatedExecutionPlan,
    sources: &[SourceUnit],
    definitions: &[SourceDefinition],
) -> JsonValue {
    let classified = explanation::classified(plan);
    if !sources.is_empty() {
        return json!(
            sources
                .iter()
                .map(|source| {
                    let redacted = classified && !source.lab_lines.is_empty();
                    let text =
                        if redacted {
                            source
                                .text
                                .split_inclusive('\n')
                                .enumerate()
                                .map(|(index, line)| {
                                    if source.lab_lines.iter().any(|(first, last)| {
                                        (*first..=*last).contains(&(index + 1))
                                    }) {
                                        if line.ends_with('\n') {
                                            "// [redacted Lab source]\n"
                                        } else {
                                            "// [redacted Lab source]"
                                        }
                                    } else {
                                        line
                                    }
                                })
                                .collect::<String>()
                        } else {
                            source.text.clone()
                        };
                    json!({"index": source.index, "display_name": source.name,
                "text": text, "line_count": source.text.lines().count(),
                "redacted": redacted, "complete": true, "unavailable_lines": []})
                })
                .collect::<Vec<_>>()
        );
    }
    // Recording format 5 has excerpts, not original source units. Preserve the
    // retained line numbers without claiming that gaps are authored text.
    let mut by_source = BTreeMap::<u32, Vec<Option<String>>>::new();
    for definition in definitions {
        let lines = by_source.entry(definition.source.index()).or_default();
        lines.resize(lines.len().max(definition.last_line), None);
        for (offset, line) in definition.code.lines().enumerate() {
            let index = definition.first_line.saturating_sub(1) + offset;
            if let Some(slot) = lines.get_mut(index) {
                // The outer Graph retains the indentation of its children.
                if slot.is_none() {
                    *slot = Some(line.to_owned());
                }
            }
        }
    }
    json!(by_source.into_iter().map(|(index, lines)| {
        let unavailable = lines.iter().enumerate().filter_map(|(line, value)| value.is_none().then_some(line + 1)).collect::<Vec<_>>();
        json!({"index": index, "display_name": format!("source-{index}.rkg (recorded excerpts)"),
            "line_count": lines.len(), "complete": false, "redacted": false,
            "unavailable_lines": unavailable,
            "text": lines.into_iter().map(|line| line.unwrap_or_else(|| "// Source unavailable in recording".into())).collect::<Vec<_>>().join("\n")})
    }).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(sources: &[&str]) -> crate::compiler::Compilation {
        crate::compiler::compile(
            sources,
            &crate::connector::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn source_units_preserve_full_text_and_indexes_once() {
        let graph = "// header\ngraph echo : App { input in: text; output out: text; in -> out }\n";
        let lab = "lab echo { simulation test on app { scenario one { app.in = \"hello\" } } }\n";
        let compilation = compile(&[graph, lab]);
        let units = units(
            &compilation.plan,
            &compilation.sources,
            &compilation.source_definitions,
        );
        assert_eq!(units.as_array().unwrap().len(), 2);
        assert_eq!(units[0]["text"], graph);
        assert_eq!(units[1]["index"], 1);
        assert_eq!(units[1]["text"], lab);
        assert_eq!(units[0]["line_count"], 2);
    }

    #[test]
    fn classified_units_withhold_lab_literals_and_keep_line_numbers() {
        let source = "type Protected = struct { email: text @personal }\ngraph echo : App { input in: Protected; output out: Protected; in -> out }\nlab echo { simulation test on app { scenario one { app.in = { email = \"private@example.test\" } } } }\n";
        let compilation = compile(&[source]);
        let units = units(
            &compilation.plan,
            &compilation.sources,
            &compilation.source_definitions,
        );
        assert_eq!(units[0]["redacted"], true);
        assert_eq!(units[0]["line_count"], 3);
        assert!(!units.to_string().contains("private@example.test"));
        assert!(units[0]["text"].as_str().unwrap().contains("graph echo"));
    }

    #[test]
    fn recording_sources_are_explicitly_incomplete() {
        let compilation = compile(&[include_str!("../../../../../examples/echo.rkg")]);
        let units = units(&compilation.plan, &[], &compilation.source_definitions);
        assert_eq!(units[0]["complete"], false);
        assert!(units[0]["text"].as_str().unwrap().contains("graph echo"));
    }
}
