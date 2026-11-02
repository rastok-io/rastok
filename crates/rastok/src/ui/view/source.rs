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

pub(super) fn position(
    snapshot: &JsonValue,
    definitions: &[SourceDefinition],
    sources: &[SourceUnit],
    activity: Option<&ActivityView>,
    journal: &MemJournal,
) -> JsonValue {
    let unavailable = |reason| {
        json!({"status": "unavailable", "reason": reason,
        "source_index": null, "first_line": null, "last_line": null, "column": null,
        "node_path": null, "port": null, "selected_arm": null, "fired_lines": []})
    };
    let Some(activity) = activity else {
        return unavailable("No transition selected before admission.");
    };
    if activity.kind == "scenario-action" {
        let current = &snapshot["scenario"]["current"];
        let definition = &current["definition"];
        let action = current["next_action_index"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(1) as usize;
        if let (Some(first), Some(relative)) = (
            definition["first_line"].as_u64(),
            definition["action_lines"][action].as_u64(),
        ) {
            return json!({"status": "available", "reason": null, "source_index": definition["source_index"],
                "first_line": first + relative - 1, "last_line": first + relative - 1,
                "column": null, "node_path": null, "port": null, "selected_arm": null, "fired_lines": []});
        }
    }
    let record = activity.records.and_then(|(from, through)| {
        journal
            .records()
            .get(from..=through)?
            .iter()
            .enumerate()
            .rev()
            .find(|(_, record)| matches!(record, Record::Transaction(_)))
            .or_else(|| {
                journal
                    .records()
                    .get(from..=through)?
                    .iter()
                    .enumerate()
                    .next_back()
            })
            .map(|(offset, record)| (from + offset, record))
    });
    let (owner, port) = match record.map(|(_, record)| record) {
        Some(Record::Transaction(tx)) => (
            Some(tx.owner_path.to_string()),
            match &tx.trigger {
                crate::node::Trigger::Port(port) => Some(port.name().to_owned()),
                _ => None,
            },
        ),
        Some(Record::Ingress(root)) => (
            Some(
                root.connector_path
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            ),
            Some(root.port.name().to_owned()),
        ),
        _ => (activity.node.clone(), None),
    };
    let Some(definition) = definitions
        .iter()
        .find(|definition| Some(definition.owner.to_string()) == owner)
    else {
        return unavailable(
            "This system, mock or synthetic transition has no retained authored source.",
        );
    };
    let mut first = definition.first_line;
    let mut last = definition.last_line;
    let mut column = None;
    let mut definition_offset = 0;
    let source = sources
        .iter()
        .find(|source| source.index == definition.source.index());
    if let Some(port) = &port {
        // Use the existing lexer; comments and literals cannot masquerade as ports.
        let mut lexer = crate::compiler::lexer::Lexer::new(definition.source, &definition.code);
        let mut bag = crate::diagnostics::Bag::new();
        let mut declaration = false;
        loop {
            let token = lexer.next(&mut bag);
            if token.kind == crate::compiler::lexer::TokenKind::Eof {
                break;
            }
            if declaration && token.text == port {
                definition_offset = token.span.range().start;
                first += definition.code[..token.span.range().start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count();
                last = first;
                break;
            }
            declaration = matches!(token.text, "input" | "output");
        }
    }
    let decision = record.map(|(index, _)| &snapshot["history"]["records"][index]["decision"]);
    let mut selected_arm = JsonValue::Null;
    let mut fired = BTreeSet::new();
    if let Some(decision) = decision {
        let mut arms = definition
            .decisions
            .iter()
            .find(|(trigger, _)| Some(trigger.as_str()) == decision["trigger"].as_str())
            .map(|(_, arms)| arms.as_slice())
            .unwrap_or(&[]);
        for step in decision["steps"].as_array().into_iter().flatten() {
            let selected = if step["kind"] == "closed" {
                arms.iter()
                    .enumerate()
                    .find(|(_, arm)| {
                        arm.cases
                            .iter()
                            .any(|case| Some(case.as_str()) == step["case"].as_str())
                    })
                    .or_else(|| arms.iter().enumerate().find(|(_, arm)| arm.fallback))
            } else if let Some(index) = step["selected"].as_u64() {
                arms.iter()
                    .enumerate()
                    .filter(|(_, arm)| !arm.fallback)
                    .nth(index as usize)
            } else {
                arms.iter().enumerate().find(|(_, arm)| arm.fallback)
            };
            let Some((index, arm)) = selected else {
                break;
            };
            (first, last) = arm.lines;
            selected_arm = json!({"index": index, "first_line": first, "last_line": last});
            for line in first..=last {
                if !arm
                    .children
                    .iter()
                    .any(|child| (child.lines.0..=child.lines.1).contains(&line))
                {
                    fired.insert(line);
                }
            }
            arms = &arm.children;
        }
    }
    if let Some(source) = source.filter(|_| selected_arm.is_null()) {
        let mut offsets = source
            .text
            .match_indices(definition.code.as_str())
            .filter_map(|(offset, _)| {
                let line = source.text[..offset]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                (line == definition.first_line).then_some(offset + definition_offset)
            });
        if let Some(offset) = offsets.next().filter(|_| offsets.next().is_none()) {
            let start = source.text[..offset]
                .rfind('\n')
                .map_or(0, |start| start + 1);
            column = Some(source.text[start..offset].chars().count() + 1);
        }
    }
    json!({"status": "available", "reason": null, "source_index": definition.source.index(),
        "first_line": first, "last_line": last, "column": column,
        "node_path": owner, "port": port, "selected_arm": selected_arm, "fired_lines": fired})
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

    use crate::deployment::{
        BootBindings, DeploymentProfile, EnvironmentName, PolicyName, PrivacyDeclaration,
        PrivacyDeclarationName, PrivacyRetention, RecipientPolicy, SecretBinding, SecretMaterial,
        SecretName, SecretProvider, SecretProviderError, SecretProviderId, SecretVersion,
        TransferPolicy,
    };
    struct FixtureSecrets;
    impl SecretProvider for FixtureSecrets {
        fn resolve(&self, _: &SecretBinding) -> Result<SecretMaterial, SecretProviderError> {
            SecretMaterial::new(vec![42; 16])
        }
    }
    fn showcase_boot(with_privacy: bool) -> BootBindings {
        let policy = |name| PolicyName::new(name).unwrap();
        let privacy = with_privacy.then(|| {
            PrivacyDeclaration::new(
                PrivacyDeclarationName::new("privacy.example/showcase-webhook@1").unwrap(),
                policy("showcase/fictional-webhook"),
                vec![policy("purpose/local-synthetic-demonstration")],
                vec![policy("basis/synthetic-fixture-not-production-authority")],
                None,
                None,
                RecipientPolicy::Classes(vec![policy("recipient/simulated-partner")]),
                TransferPolicy::None,
                PrivacyRetention::RunScoped,
            )
            .unwrap()
        });
        let reference = SecretName::new("showcase/partner/token").unwrap();
        BootBindings::resolve(
            DeploymentProfile::new(
                BTreeMap::from([(
                    EnvironmentName::new("PARTNER_URL").unwrap(),
                    Value::Text("https://partner.example.test/events/upsert".into()),
                )]),
                BTreeMap::from([(
                    reference.clone(),
                    SecretBinding::new(
                        reference,
                        SecretProviderId::new("test.local/test@1").unwrap(),
                        "fixture-token",
                        SecretVersion::new("fixture-v1").unwrap(),
                    )
                    .unwrap(),
                )]),
                privacy,
            )
            .unwrap(),
            &FixtureSecrets,
        )
        .unwrap()
    }

    #[test]
    fn gallery_positions_follow_each_retained_prefix() {
        let boot = showcase_boot(true);
        for source in [
            include_str!("../../../../../examples/echo.rkg"),
            include_str!("../../../../../examples/countdown.rkg"),
            include_str!("../../../../../examples/webhook-to-partner.rkg"),
        ] {
            let compilation = compile(&[source]);
            let simulation = &compilation.labs[0].simulations[0];
            let scenario = simulation
                .scenarios
                .iter()
                .find(|scenario| scenario.name == "timeout_then_success")
                .unwrap_or(&simulation.scenarios[0]);
            let mut session = crate::lab::Simulator::try_new_program(
                &compilation.plan,
                compilation.hash,
                crate::plan::GraphVersion(1),
                &simulation.name,
                &simulation.mocks,
                scenario,
                Some(&boot),
            )
            .unwrap();
            let mut positions = Vec::new();
            let mut counters = BTreeSet::new();
            let mut activity = None;
            for boundary in 0..500 {
                let view = super::super::snapshot(SnapshotInput {
                    revision: boundary,
                    mode: "simulate",
                    state: ExecutionState::Paused,
                    plan: &compilation.plan,
                    definitions: &compilation.source_definitions,
                    sources: &compilation.sources,
                    execution: session.execution(),
                    journal: session.journal(),
                    activity: activity.as_ref(),
                    fault: None,
                    scenario: None,
                    session: 0,
                    boundary,
                })
                .unwrap();
                let frame: JsonValue = serde_json::from_slice(&view.json).unwrap();
                for token in frame["source_tokens"].as_array().unwrap() {
                    if token["name"] == "Loop.measure" && token["value"]["available"] == true {
                        counters.insert(token["value"]["rendering"].to_string());
                    }
                }
                let position = frame["current_position"].clone();
                assert!(position.get("status").is_some());
                if position["status"] == "available" {
                    assert_eq!(position["source_index"], 0);
                    let first = position["first_line"].as_u64().unwrap();
                    let last = position["last_line"].as_u64().unwrap();
                    assert!(first > 0 && first <= last && last <= source.lines().count() as u64);
                }
                positions.push(position);
                if session.is_complete() {
                    break;
                }
                let before = session.journal().records().len();
                if session.is_quiescent() {
                    session.next_action().unwrap();
                } else {
                    session.step_transaction().unwrap();
                }
                activity =
                    ActivityView::journal_change(session.journal(), before, Some(boundary + 1));
            }
            assert!(session.is_complete());
            assert_eq!(positions[0]["status"], "unavailable");
            assert!(
                positions
                    .iter()
                    .any(|position| position["port"].is_string())
            );
            if source.contains("graph countdown") {
                assert!(
                    counters.len() > 1,
                    "counter values must follow the selected transition"
                );
                assert!(
                    positions
                        .iter()
                        .any(|position| !position["selected_arm"].is_null())
                );
            }
            // Earlier serialized positions remain independent of the completed head.
            assert_eq!(positions[0]["first_line"], JsonValue::Null);
        }
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
    fn replay_projects_source_positions_without_changing_recording_bytes() {
        let mut compilation = compile(&[include_str!("../../../../../examples/echo.rkg")]);
        let run = crate::lab::run(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &compilation.labs[0],
        )
        .remove(0);
        let journal = MemJournal::from_records(
            crate::plan::GraphVersion(1),
            crate::time::Timestamp::UNIX_EPOCH,
            None,
            &run.records,
        )
        .unwrap();
        let observed = crate::time::LogicalTime::from_nanoseconds(10_000_000_000);
        let bytes = crate::recording::Recording::capture(&compilation, &journal, observed)
            .unwrap()
            .encode()
            .unwrap();
        compilation.sources[0].text = "Physical source text does not enter recordings".into();
        compilation.sources[0].name = "different.rkg".into();
        assert_eq!(
            bytes,
            crate::recording::Recording::capture(&compilation, &journal, observed)
                .unwrap()
                .encode()
                .unwrap()
        );
        let recording = crate::recording::Recording::decode(&bytes).unwrap();
        for through in 0..=recording.records().len() {
            let (journal, execution) = recording.inspect(through).unwrap();
            let activity = through.checked_sub(1).and_then(|before| {
                ActivityView::journal_change(&journal, before, Some(through as u64))
            });
            let view = super::super::snapshot(SnapshotInput {
                revision: through as u64,
                mode: "replay",
                state: ExecutionState::Completed,
                plan: recording.plan(),
                definitions: recording.definitions(),
                sources: &[],
                execution: &execution,
                journal: &journal,
                activity: activity.as_ref(),
                fault: None,
                scenario: None,
                session: 0,
                boundary: through as u64,
            })
            .unwrap();
            let frame: JsonValue = serde_json::from_slice(&view.json).unwrap();
            assert_eq!(frame["sources"][0]["complete"], false);
            assert_eq!(frame["current_position"]["column"], JsonValue::Null);
            assert_eq!(frame["execution"]["legal_commands"], json!(["close"]));
            assert!(
                frame["source_tokens"]
                    .as_array()
                    .is_some_and(|tokens| !tokens.is_empty())
            );
            if through == 0 {
                assert_eq!(frame["current_position"]["status"], "unavailable");
            }
        }
    }

    #[test]
    fn recording_sources_are_explicitly_incomplete() {
        let compilation = compile(&[include_str!("../../../../../examples/echo.rkg")]);
        let units = units(&compilation.plan, &[], &compilation.source_definitions);
        assert_eq!(units[0]["complete"], false);
        assert!(units[0]["text"].as_str().unwrap().contains("graph echo"));
    }
}
