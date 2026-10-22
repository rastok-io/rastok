//! Resident Process contract, Lab and physical Driver conformance.
use rastok::{compiler::compile, connector::contract::ContractCatalog};

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
     behavior = deterministic
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
        compilation.plan.plan().nodes[1]
            .connector()
            .map(|s| s.exchange),
        Some(rastok::connector::ExchangeKind::Session)
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
            rastok::journal::Record::Egress(rastok::journal::EgressRecord::Session(record)) => {
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
    use rastok::journal::{EgressRecord, Record, SessionStatus};
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
                Record::Egress(EgressRecord::Session(s))
                    if matches!(s.status, SessionStatus::Opening(_)) =>
                {
                    Some(s.session_id)
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
fn process_modes_share_one_identity_without_aliasing_another_catalogue_entry() {
    use rastok::connector::{contract::*, stdlib::*, *};
    let mut alias = rastok::connector::stdlib::PROCESS;
    alias.name = "Alias";
    alias.id = PROCESS_CONNECTOR_ID;
    alias.selector = None;
    assert!(matches!(
        ContractCatalog::try_m0().unwrap().try_with(alias),
        Err(ContractValidationError::ConnectorIdCollision { .. })
    ));
    let mut service_default = rastok::connector::stdlib::PROCESS;
    service_default.selector = Some(ModeSelector {
        attribute: "mode",
        default: Some("service"),
        modes: &[ContractMode {
            word: "service",
            support: ModeSupport::Available,
            attributes: &[],
            protocol: Some(ModeProtocol {
                roles: ConnectorDirection::InboundAndOutbound,
                exchange: ExchangeKind::Session,
            }),
        }],
    });
    let catalogue = ContractCatalog::default()
        .try_with(service_default)
        .unwrap();
    let id = ContractId::parse(PROCESS_CONNECTOR_ID).unwrap();
    let sheet = catalogue.connector_by_id(&id).unwrap();
    assert_eq!(sheet.id, PROCESS_CONNECTOR_ID);
    assert_eq!(
        sheet.mode("service").unwrap().protocol.unwrap().exchange,
        ExchangeKind::Session
    );
}

#[test]
fn independent_core_validation_rejects_forged_service_modes() {
    use rastok::{connector::*, plan::*};
    let compiled = compile(&[SERVICE], &ContractCatalog::try_m0().unwrap()).unwrap();
    for sabotage in 0..10 {
        let mut plan = compiled.plan.clone().into_inner();
        let NodeKind::Connector(spec) = &mut plan.nodes[1].body else {
            panic!("connector");
        };
        match sabotage {
            0 => spec.roles = ConnectorDirection::Outbound,
            1 => spec.exchange = ExchangeKind::Call,
            2 => {
                spec.attributes
                    .iter_mut()
                    .find(|(name, _)| name == "mode")
                    .unwrap()
                    .1 = AttrSetting::Word("job".into())
            }
            3 => spec.attributes.retain(|(name, _)| name != "max_frame"),
            4 => spec.attributes.retain(|(name, _)| name != "mode"),
            5 => {
                spec.attributes
                    .iter_mut()
                    .find(|(name, _)| name == "mode")
                    .unwrap()
                    .1 = AttrSetting::Word("unknown".into())
            }
            6 => {
                spec.attributes
                    .iter_mut()
                    .find(|(name, _)| name == "mode")
                    .unwrap()
                    .1 = AttrSetting::Value(rastok::value::Value::Text("service".into()))
            }
            7 => spec.contract = ContractId::parse("std/process@4").unwrap(),
            8 => spec.contract = ContractId::parse("std/process@5").unwrap(),
            _ => {
                spec.idempotent = Some(PlanConstant {
                    resolved: rastok::value::Value::Bool(true),
                    expr: rastok::expr::compile("true", &rastok::expr::Signature::new()).unwrap(),
                })
            }
        }
        assert!(
            ValidatedExecutionPlan::try_new(plan).is_err(),
            "sabotage {sabotage}"
        );
    }
}

#[test]
fn job_and_service_resolve_one_contract_with_distinct_checked_protocols() {
    use rastok::connector::{ConnectorDirection, ExchangeKind, stdlib::PROCESS_CONNECTOR_ID};
    let catalogue = ContractCatalog::try_m0().unwrap();
    let job_source = include_str!("../../../examples/echo.rkg");
    let job = compile(&[&job_source.replace("mode = job ;", "")], &catalogue).unwrap();
    let explicit = compile(&[job_source], &catalogue).unwrap();
    assert_eq!(
        job.hash, explicit.hash,
        "omitting the default mode changes no identity"
    );
    let service = compile(&[SERVICE], &catalogue).unwrap();
    for (compilation, direction, exchange) in [
        (&job, ConnectorDirection::Outbound, ExchangeKind::Call),
        (
            &service,
            ConnectorDirection::InboundAndOutbound,
            ExchangeKind::Session,
        ),
    ] {
        let (decoded, hash) =
            rastok::plan::decode(&rastok::plan::encode(&compilation.plan, &compilation.hash))
                .unwrap();
        assert_eq!(hash, compilation.hash);
        let spec = decoded
            .plan()
            .nodes
            .iter()
            .find_map(|node| node.connector())
            .unwrap();
        assert_eq!(spec.contract.as_str(), PROCESS_CONNECTOR_ID);
        assert_eq!(spec.roles, direction);
        assert_eq!(spec.exchange, exchange);
    }
}

#[test]
fn one_graph_selects_job_and_service_models_by_mode_without_ambiguity() {
    let source = r#"
graph mixed : App {
  input in: text
  output job: text
  output service: text
  connector once : Process { command = "cat"; codec = lines; behavior = deterministic }
  connector resident : Process {
    mode = service; command = "cat"; codec = lines; behavior = deterministic
    closing = { drain_for = duration("1s"); settle_for = duration("1s") }
  }
  in -> once
  in -> resident
  once.out -> job
  resident.out -> service
  once.err -> nothing
  once.exit -> nothing
  resident.err -> nothing
  resident.exit -> nothing
}
lab mixed {
  simulation modes on app {
    mock once { on in { out = data; exit = Exited { exit_code = 0; signal = null } } }
    mock resident { on in { out = data } }
    scenario interleaved "one Call per input, one shared Session" {
      app.in = "first"
      expect app.job where data == "first"
      expect app.service where data == "first"
      app.in = "second"
      expect app.job where data == "second"
      expect app.service where data == "second"
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let registry = rastok::connector::drivers::hosted_registry(None).unwrap();
    for spec in compilation
        .plan
        .plan()
        .nodes
        .iter()
        .filter_map(|node| node.connector())
    {
        let Some(rastok::plan::AttrSetting::Word(mode)) = spec.attribute("mode") else {
            panic!("the compiler materializes the mode");
        };
        let (name, _) = registry
            .require(&spec.contract, spec.roles, spec.exchange, Some(mode), None)
            .unwrap();
        assert_eq!(
            name.as_str(),
            if mode == "job" {
                "process-job"
            } else {
                "process-service"
            }
        );
    }
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert!(run.report.failures.is_empty(), "{:?}", run.report.failures);
    use rastok::journal::{CallStatus, EgressRecord, Record, SessionStatus};
    let calls = run
        .records
        .iter()
        .filter(|record| {
            matches!(record,
                Record::Egress(EgressRecord::Call(call)) if matches!(call.status, CallStatus::Open)
            )
        })
        .count();
    let sessions = run.records.iter().filter(|record| matches!(record,
        Record::Egress(EgressRecord::Session(session)) if matches!(session.status, SessionStatus::Opening(_))
    )).count();
    assert_eq!(calls, 2);
    assert_eq!(sessions, 1);
}

#[cfg(unix)]
#[allow(clippy::unwrap_used, clippy::disallowed_types)]
mod real {
    use super::*;
    use rastok::{
        connector::drivers::{RuntimeDrivers, hosted_registry},
        connector::exchange::*,
        envelope::*,
        journal::{EgressRecord, MemJournal, Record, SessionStatus},
        plan::GraphVersion,
        runtime::execution::Execution,
    };

    fn input(port: &str, text: &str) -> IngressProposal {
        IngressProposal {
            provenance: IngressProvenance::single(rastok::node::Path::new(["test"]).unwrap()),
            draft: EnvelopeDraft {
                port: rastok::node::Port::new(port).unwrap(),
                data: Data::materialized(Value::Text(text.into())),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        }
    }
    fn until<'a>(
        execution: &mut Execution<'a>,
        journal: &mut MemJournal,
        world: &mut RuntimeDrivers<'a>,
        predicate: impl Fn(&Execution<'a>, &MemJournal) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            execution.run_ready_with_drivers(journal, world).unwrap();
            world.poll_inbound(execution, journal).unwrap();
            execution.poll_drivers(journal, world).unwrap();
            execution.run_ready_with_drivers(journal, world).unwrap();
            if predicate(execution, journal) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "resident child failed to progress: {:?}",
                execution.outputs()
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn openings(journal: &MemJournal) -> usize {
        journal.records().iter().filter(|r| matches!(r, Record::Egress(EgressRecord::Session(s)) if matches!(s.status, SessionStatus::Opening(_)))).count()
    }

    /// Marked OS integration: no network and no model; only a local child.
    #[test]
    fn unix_service_reuses_a_child_then_observes_quit_and_opens_fresh() {
        let source = SERVICE.replace("command = \"cat\"", r#"command = "/bin/sh"; args = ["-c", "while IFS= read -r line; do if [ \"$line\" = quit ]; then exit 0; fi; printf '%s\\n' \"$line\"; done"]"#);
        let compiled = compile(&[source.as_str()], &ContractCatalog::try_m0().unwrap()).unwrap();
        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let registry = hosted_registry(None).unwrap();
        let mut world = RuntimeDrivers::prepare(&compiled.plan, &registry).unwrap();
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
            execution.accept_input(&mut journal, submission).unwrap();
            until(&mut execution, &mut journal, &mut world, |execution, _| {
                execution.outputs().len() == i + 1
            });
            if i < 3 {
                assert_eq!(openings(&journal), 1);
            }
            let before = journal.records().to_vec();
            execution
                .recover_with_drivers(&journal, &mut world)
                .unwrap();
            assert_eq!(journal.records(), before, "recovery never repeats a write");
        }
        assert_eq!(openings(&journal), 2);
        assert!(!execution.has_open_obligations());
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
            let mut world = RuntimeDrivers::prepare(&compiled.plan, &registry).unwrap();
            let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
            let mut journal = MemJournal::new(GraphVersion(1));
            execution
                .accept_input(&mut journal, input("in", "ready"))
                .unwrap();
            until(&mut execution, &mut journal, &mut world, |execution, _| {
                execution.outputs().len() == 1
            });
            execution
                .accept_input(&mut journal, input("close", "close this generation"))
                .unwrap();
            execution
                .run_ready_with_drivers(&mut journal, &mut world)
                .unwrap();
            execution
                .advance_to(&mut journal, &mut world, 1_000_000_000)
                .unwrap();
            if ignore_term {
                execution
                    .advance_to(&mut journal, &mut world, 2_000_000_000)
                    .unwrap();
            }
            until(&mut execution, &mut journal, &mut world, |execution, _| {
                execution.outputs().len() == 2
            });
            let Data::Materialized(Value::TaggedUnion(exit)) =
                execution.outputs()[1].envelope.data()
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
            assert!(!execution.has_open_obligations());
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
        let mut world = RuntimeDrivers::prepare(&compiled.plan, &registry).unwrap();
        let mut execution = Execution::new(&compiled.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        execution
            .accept_input(&mut journal, input("in", "start"))
            .unwrap();
        until(&mut execution, &mut journal, &mut world, |execution, _| {
            execution.outputs().len() == 2
        });
        let Data::Materialized(Value::TaggedUnion(exit)) = execution.outputs()[1].envelope.data()
        else {
            panic!("typed terminal");
        };
        assert_eq!(exit.case, "DecodeFailed");
        assert_eq!(execution.outputs()[0].port.name(), "out");
    }
}
