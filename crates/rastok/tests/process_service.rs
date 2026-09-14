//! Resident Process contract, Lab and physical Driver conformance.
use rastok::{compiler::compile, contract::ContractCatalog};

const SERVICE: &str = r#"
type ChildExit = union {
  Exited { exit_code: int, signal: int? }
  SpawnFailed { message: text }
  DecodeFailed { line: text?, message: text }
  OutputOverflow { limit: uint, reason: enum { budget, capacity } }
  CaptureFailed {}
  TimeoutExpired {}
  OutcomeUnknown {}
}
graph resident : App {
  input in: text
  output exit: ChildExit
  connector child : Process {
    mode = service ; command = "cat" ; codec = lines
    effect = idempotent ; behavior = deterministic
    closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  }
  in -> child -> out
  child.err -> nothing
  child.exit -> exit
}

lab resident {
  simulation dialogue on app {
    mock child { on in match {
      data == "quit" => exit = Exited { exit_code = 0; signal = null }
      _ => out = data
    } }
    scenario conversation "two writes, one child" {
      app.in = "hello"
      expect app.out where data == "hello"
      expect nothing app.exit
      app.in = "again"
      expect app.out where data == "again"
      app.in = "quit"
      expect app.exit where data is Exited
    }
  }
}
"#;

#[test]
fn resident_mock_keeps_one_session_across_writes() {
    let compilation = compile(&[SERVICE], &ContractCatalog::try_m0().unwrap())
        .expect("service is an executable Session, not a job per message");
    assert_eq!(
        compilation.plan.plan().nodes[0]
            .connector()
            .map(|s| s.exchange),
        Some(rastok::connector::ConnectorExchange::Session)
    );
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].report.failures.is_empty(),
        "{:?}",
        runs[0].report.failures,
    );
    let statuses: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|r| match r {
            rastok::journal::Record::Outbound(rastok::journal::OutboundRecord::Session(record)) => {
                Some(&record.status)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|s| matches!(s, rastok::journal::SessionStatus::Opening(_)))
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| matches!(s, rastok::journal::SessionStatus::WriteOpen(_)))
            .count(),
        2
    );
}

#[test]
fn gallery_keeps_alice_and_bob_resources_separate_and_reopens_only_on_input() {
    use rastok::journal::{OutboundRecord, Record, SessionStatus};
    let compilation = compile(
        &[include_str!("../../../examples/agent-dialogue.rkg")],
        &ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
    let (plan, _) =
        rastok::plan::decode(&rastok::plan::encode(&compilation.plan, &compilation.hash)).unwrap();
    let runs = rastok::lab::run(&plan, rastok::plan::GraphVersion(1), &compilation.labs[0]);
    for run in &runs {
        assert!(
            run.report.failures.is_empty(),
            "{}: {:?}",
            run.report.scenario,
            run.report.failures
        );
        let openings: Vec<_> = run
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Outbound(OutboundRecord::Session(s))
                    if matches!(s.status, SessionStatus::Opening(_)) =>
                {
                    Some(s.session_ref)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            openings.len(),
            2,
            "two resources, never a process per message"
        );
        assert_ne!(openings[0], openings[1]);
    }
}

#[test]
fn service_refuses_unframed_bytes_and_job_only_or_retired_settings() {
    for attribute in [
        "codec = bytes",
        "err_codec = bytes",
        "timeout = duration(\"1s\")",
        "max_output = \"1MiB\"",
        "correlate = data.id",
        "restart = {}",
    ] {
        let source = if attribute.starts_with("codec") {
            SERVICE.replace("codec = lines", attribute)
        } else {
            SERVICE.replace("mode = service", &format!("mode = service; {attribute}"))
        };
        assert!(
            compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).is_err(),
            "{attribute}"
        );
    }
}

#[test]
fn session_mock_frames_need_no_terminal_but_nothing_may_follow_one() {
    let reply = "exit = Exited { exit_code = 0; signal = null }";
    for replacement in [
        format!("{{ {reply}; out = data }}"),
        format!("{{ {reply}; {reply} }}"),
    ] {
        let source = SERVICE.replace(reply, &replacement);
        let error = compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.message().contains("at most one `exit`")),
            "{error:?}"
        );
    }
}

#[test]
fn the_service_profile_cannot_alias_a_job_or_another_catalogue_entry() {
    use rastok::{connector::*, contract::*};
    let mut alias = rastok::stdlib::PROCESS;
    alias.name = "Alias";
    alias.id = PROCESS_SERVICE_CONNECTOR_ID;
    alias.selector = None;
    assert!(matches!(
        ContractCatalog::try_m0().unwrap().try_with(alias),
        Err(SheetDefect::ConnectorIdCollision { .. })
    ));
    let mut broken = rastok::stdlib::PROCESS;
    broken.selector = Some(ProfileSelector {
        attribute: "mode",
        default: Some("service"),
        profiles: &[Profile {
            word: "service",
            support: ProfileSupport::Available,
            attributes: &[],
            protocol: Some(ProfileProtocol {
                id: PROCESS_CONNECTOR_ID,
                roles: ConnectorRoles::InboundAndOutbound,
                exchange: ConnectorExchange::Session,
            }),
        }],
    });
    assert!(matches!(
        ContractCatalog::default().try_with(broken),
        Err(SheetDefect::ConnectorIdCollision { .. })
    ));
}

#[test]
fn independent_core_validation_rejects_forged_service_profiles() {
    use rastok::{connector::*, plan::*};
    let compiled = compile(&[SERVICE], &ContractCatalog::try_m0().unwrap()).unwrap();
    for sabotage in 0..4 {
        let mut plan = compiled.plan.clone().into_inner();
        let NodeExecution::Connector(spec) = &mut plan.nodes[0].execution else {
            panic!("connector");
        };
        match sabotage {
            0 => spec.roles = ConnectorRoles::Outbound,
            1 => spec.exchange = ConnectorExchange::Call,
            2 => spec.contract = ContractId::parse(PROCESS_CONNECTOR_ID).unwrap(),
            _ => spec.attributes.retain(|(name, _)| name != "max_frame"),
        }
        assert!(
            ValidatedExecutionPlan::try_new(plan).is_err(),
            "sabotage {sabotage}"
        );
    }
}

#[cfg(unix)]
#[allow(clippy::unwrap_used, clippy::disallowed_types)]
mod real {
    use super::*;
    use rastok::{
        boundary::*,
        connectors::{RuntimeBindings, hosted_registry},
        envelope::*,
        journal::{MemJournal, OutboundRecord, Record, SessionStatus},
        plan::GraphVersion,
        runtime::vm::Vm,
    };

    fn input(port: &str, text: &str) -> InboundSubmission {
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                rastok::node::Path::new(["test"]).unwrap(),
            ),
            port: rastok::node::Port::new(port).unwrap(),
            draft: Draft {
                data: Data::materialized(Value::Text(text.into())),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        }
    }
    fn until<'a>(
        vm: &mut Vm<'a>,
        journal: &mut MemJournal,
        world: &mut RuntimeBindings<'a>,
        predicate: impl Fn(&Vm<'a>, &MemJournal) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            vm.drain_with_bindings(journal, world).unwrap();
            world.pump_inbound(vm, journal).unwrap();
            vm.pump(journal, world).unwrap();
            vm.drain_with_bindings(journal, world).unwrap();
            if predicate(vm, journal) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "resident child failed to progress: {:?}",
                vm.outputs()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn openings(journal: &MemJournal) -> usize {
        journal.records().iter().filter(|r| matches!(r, Record::Outbound(OutboundRecord::Session(s)) if matches!(s.status, SessionStatus::Opening(_)))).count()
    }

    /// Marked OS integration: no network and no model; only a local child.
    #[test]
    fn unix_service_reuses_a_child_then_observes_quit_and_opens_fresh() {
        let source = SERVICE.replace("command = \"cat\"", r#"command = "/bin/sh"; args = ["-c", "while IFS= read -r line; do if [ \"$line\" = quit ]; then exit 0; fi; printf '%s\\n' \"$line\"; done"]"#);
        let compiled = compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let registry = hosted_registry(None).unwrap();
        let mut world = RuntimeBindings::try_new(&compiled.plan, &registry).unwrap();
        for (i, text) in ["hello", "again", "quit", "fresh", "quit"]
            .iter()
            .enumerate()
        {
            let mut submission = input("in", text);
            // A websocket resource token carried by the sender cannot select
            // a Process resource. Its exact Node instance owns that choice.
            submission.draft.ext.insert(
                "session_id".into(),
                MetaValue::Scalar(Scalar::Text("00000000000000000000000000000001".into())),
            );
            vm.inject(&mut journal, submission).unwrap();
            until(&mut vm, &mut journal, &mut world, |vm, _| {
                vm.outputs().len() == i + 1
            });
            if i < 3 {
                assert_eq!(openings(&journal), 1);
            }
            let before = journal.records().to_vec();
            vm.recover_with_bindings(&journal, &mut world).unwrap();
            assert_eq!(journal.records(), before, "recovery never repeats a write");
        }
        assert_eq!(openings(&journal), 2);
        assert!(!vm.has_open_obligations());
    }

    /// Marked OS integration: retained logical deadlines trigger Unix signals.
    #[test]
    fn unix_closing_observes_sigterm_or_retains_unknown_after_forcing_a_survivor() {
        for ignore_term in [false, true] {
            let mut source = SERVICE.replace(
                "input in: text\n  output exit",
                "input in: text\n  input close: text\n  output exit",
            );
            source = source.replace("in -> child -> out", "operator stop_child { input in: text; output out: text; on in { out = data } }\n close -> stop_child\n stop_child.out -> child.instance.stop\n in -> child -> out");
            if ignore_term {
                source = source.replace("command = \"cat\"", r#"command = "/bin/sh"; args = ["-c", "trap '' TERM; while IFS= read -r line; do printf '%s\\n' \"$line\"; done"]"#);
            }
            let compiled =
                compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).unwrap();
            let registry = hosted_registry(None).unwrap();
            let mut world = RuntimeBindings::try_new(&compiled.plan, &registry).unwrap();
            let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
            let mut journal = MemJournal::new(GraphVersion(1));
            vm.inject(&mut journal, input("in", "ready")).unwrap();
            until(&mut vm, &mut journal, &mut world, |vm, _| {
                vm.outputs().len() == 1
            });
            vm.inject(&mut journal, input("close", "close this generation"))
                .unwrap();
            vm.drain_with_bindings(&mut journal, &mut world).unwrap();
            vm.advance(&mut journal, &mut world, 1_000_000_000).unwrap();
            if ignore_term {
                vm.advance(&mut journal, &mut world, 2_000_000_000).unwrap();
            }
            until(&mut vm, &mut journal, &mut world, |vm, _| {
                vm.outputs().len() == 2
            });
            let Data::Materialized(Value::TaggedUnion(exit)) = vm.outputs()[1].envelope.data()
            else {
                panic!("typed terminal");
            };
            assert_eq!(
                exit.case,
                if ignore_term {
                    "OutcomeUnknown"
                } else {
                    "Exited"
                }
            );
            assert!(!vm.has_open_obligations());
        }
    }

    /// Marked OS integration: stdout frames are visible before natural EOF.
    #[test]
    fn malformed_service_json_keeps_prior_frames_and_has_one_typed_terminal() {
        let source = SERVICE.replace("codec = lines", "codec = jsonl").replace(
            "command = \"cat\"",
            r#"command = "/bin/sh"; args = ["-c", "read line; printf '1\\n{invalid}\\n'"]"#,
        );
        // The separate Lab script uses text; only the graph is needed here.
        let graph = source.split("lab resident").next().unwrap();
        let compiled = compile(&[graph], &ContractCatalog::try_m0().unwrap()).unwrap();
        let registry = hosted_registry(None).unwrap();
        let mut world = RuntimeBindings::try_new(&compiled.plan, &registry).unwrap();
        let mut vm = Vm::boot(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        vm.inject(&mut journal, input("in", "start")).unwrap();
        until(&mut vm, &mut journal, &mut world, |vm, _| {
            vm.outputs().len() == 2
        });
        let Data::Materialized(Value::TaggedUnion(exit)) = vm.outputs()[1].envelope.data() else {
            panic!("typed terminal");
        };
        assert_eq!(exit.case, "DecodeFailed");
        assert_eq!(vm.outputs()[0].port.name(), "out");
    }
}
