//! Temporal state carries the same policy as spatial data, including expiry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{compiler, contract::ContractCatalog, plan};

#[test]
fn recovered_graph_outputs_and_expiry_are_redacted_before_logging() {
    use rastok::boundary::{EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence};
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::{Journal, MemJournal};
    use rastok::plan::GraphVersion;
    use rastok::runtime::execution::Execution;
    let source = r#"
type State = struct { n: int, private: text @sensitive(level = restricted) }
graph app : App {
  input in: State
  output out: State
  output exhausted: union { LoopExpired { last_state: State } }
  graph cycle : Loop {
    input in: State
    output err: union { LoopExpired { last_state: State } }
    output out: State
    Loop.fuel = 2
    operator choose {
      input in: State
      output accepted: State
      output revised: State
      on in match {
        data.n <= 0 => accepted = data
        _ => revised = { n = data.n - 1; private = Loop.state.data.private }
      }
    }
    in -> choose
    choose.accepted -> out
    choose.revised -> Loop.next
  }
  in -> cycle -> out
  cycle.err -> exhausted
}
"#;
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let (plan, _) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    for n in [1, 4] {
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Execution::new(&plan, GraphVersion(1));
        vm.accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(rastok::node::Path::new(["policy"]).unwrap()),
                draft: EnvelopeDraft {
                    port: rastok::node::Port::new("in").unwrap(),
                    data: Data::materialized(Value::Struct(vec![
                        ("n".into(), Value::Int(n)),
                        (
                            "private".into(),
                            Value::Text("never-export-this-value".into()),
                        ),
                    ])),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
        for _ in 0..100 {
            let mut restored = Execution::new(&plan, GraphVersion(1));
            restored.recover_from_journal(&journal).unwrap();
            assert_eq!(restored.inspection(), vm.inspection());
            vm = restored;
            if vm.is_stable() {
                break;
            }
            assert!(vm.step(&mut journal).unwrap());
        }
        assert!(vm.is_stable());
        assert_eq!(vm.outputs().len(), 1);
        let output = &vm.outputs()[0];
        assert_eq!(output.port.name(), if n == 1 { "out" } else { "exhausted" });
        let annotations = plan
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port() == &output.port)
            .unwrap()
            .annotations();
        let (sink, receiver) = rastok::observability::json_log_channel(1).unwrap();
        sink.try_observation_with_payload(
            journal.retained_observations().last().unwrap(),
            output.envelope.data(),
            annotations,
        );
        let rendered = receiver.try_drain().join("\n");
        assert!(!rendered.is_empty());
        assert!(!rendered.contains("never-export-this-value"), "{rendered}");
        assert!(
            format!("{:?}", output.envelope.data()).contains("never-export-this-value"),
            "redaction must not rewrite semantic DATA"
        );
    }
}

#[test]
fn operator_commands_preserve_state_policy_through_recurrence_and_expiry() {
    let source = r#"
type State = struct { n: int, private: text @sensitive(level = confidential) }
graph app : App {
  input in: State
  output out: text
  output exhausted: union { LoopExpired { last_state: State } }
  operator cycle : Loop {
    input in: State
    output err: union { LoopExpired { last_state: State } }
    output out: text
    Loop.fuel = 2
    on in match {
      data.n <= 0 => out = Loop.state.data.private
      _ => Loop.next = { n = data.n - 1; private = Loop.state.data.private }
    }
  }
  in -> cycle
  cycle.out -> out
  cycle.err -> exhausted
}
"#;
    assert_classified_outputs(source, &["out", "exhausted"]);
}

#[test]
fn graph_routes_preserve_state_policy_and_the_qualified_expiry() {
    let source = r#"
type State = struct { n: int, private: text @sensitive(level = confidential) }
graph app : App {
  input in: State
  output out: text
  output exhausted: union { LoopExpired { last_state: State } }
  graph cycle : Loop {
    input in: State
    output err: union { LoopExpired { last_state: State } }
    output out: text
    Loop.fuel = 2
    operator decide {
      input in: State
      output accepted: text
      output revised: State
      on in match {
        data.n <= 0 => accepted = Loop.state.data.private
        _ => revised = { n = data.n - 1; private = Loop.state.data.private }
      }
    }
    in -> decide
    decide.accepted -> out
    decide.revised -> Loop.next
  }
  in -> cycle
  cycle.out -> out
  cycle.err -> exhausted
}
"#;
    assert_classified_outputs(source, &["out", "exhausted"]);
}

#[test]
fn a_next_command_can_classify_previously_public_loop_state() {
    let source = r#"
graph app : App {
  input in: int
  output out: int
  output exhausted: union { LoopExpired { last_state: int } }
  operator cycle : Loop {
    input in: int
    output err: union { LoopExpired { last_state: int } }
    output out: int
    Loop.fuel = 2
    on in match {
      data <= 0 => out = data
      _ => {
        val classified: int @sensitive(level = restricted) = 0
        Loop.next = classified
      }
    }
  }
  in -> cycle
  cycle.out -> out
  cycle.err -> exhausted
}
"#;
    assert_classified_outputs(source, &["out", "exhausted"]);
}

#[test]
fn a_classified_again_cause_taints_retained_state_without_replacing_it() {
    let source = r#"
graph app : App + Loop {
  input in: int
  output err: union { LoopExpired { last_state: int } }
  Loop.fuel = 2
  operator decide {
    input in: int
    output out: int
    on in {
      val reason: int @sensitive(level = restricted) = 1
      out = reason
    }
  }
  in -> decide
  decide.out -> Loop.again
}
"#;
    assert_classified_outputs(source, &["err"]);
}

#[test]
fn intermediate_graph_policy_survives_executable_edge_flattening() {
    let source = r#"
type Private = int @sensitive(level = restricted)
graph app : App {
  input in: int
  output out: int
  graph guarded {
    input in: Private
    output out: int
    operator copy { input in: int; output out: int; on in { out = data } }
    in -> copy -> out
  }
  in -> guarded -> out
}
"#;
    assert_classified_outputs(source, &["out"]);
}

#[test]
fn accounting_meaning_survives_a_command_copy_but_not_a_calculation() {
    for (next, expected) in [("data", true), ("data + data", false)] {
        let source = format!(
            r#"
type Tokens = uint @measure(id = input_tokens, unit = "token")
graph app : App {{
  input in: Tokens
  output exhausted: union {{ LoopExpired {{ last_state: uint }} }}
  operator cycle : Loop {{
    input in: uint
    output err: union {{ LoopExpired {{ last_state: uint }} }}
    Loop.fuel = 2
    on in {{ Loop.next = {next} }}
  }}
  in -> cycle
  cycle.err -> exhausted
}}
"#
        );
        let result = compiler::compile(&[&source], &ContractCatalog::try_m0().unwrap());
        if expected {
            assert_classified_outputs(&source, &["exhausted"]);
        } else {
            let error = result.unwrap_err();
            assert!(
                format!("{error:?}").contains("E-DATA-ANNOTATION"),
                "{error:?}"
            );
        }
    }
}

fn assert_classified_outputs(source: &str, outputs: &[&str]) {
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap())
        .unwrap_or_else(|error| panic!("{error:?}"));
    let (decoded, hash) = plan::decode(&plan::encode(&compiled.plan, &compiled.hash)).unwrap();
    assert_eq!(hash, compiled.hash);
    assert_eq!(decoded.resolved_ports(), compiled.plan.resolved_ports());
    for output in outputs {
        let port = decoded
            .resolved_ports()
            .boundary()
            .iter()
            .find(|port| port.port().name() == *output)
            .unwrap();
        assert!(
            !port.annotations().is_empty(),
            "policy was lost at {output}"
        );
    }
}
