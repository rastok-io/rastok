//! Exchange reconstruction retains obligations without repeating physical I/O.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler;
use rastok::connector::exchange::Completion;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::InboundCallObservation;
use rastok::connector::exchange::InboundExchangeId;
use rastok::connector::exchange::IngressAdmission;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProposalHandle;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::connector::exchange::PendingIngress;
use rastok::connector::stdlib::HttpServerExit;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::node;
use rastok::plan::{self, GraphVersion};
use rastok::runtime::execution::Execution;

fn compile(source: &str) -> plan::ValidatedExecutionPlan {
    let compiled = compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    plan::decode(&plan::encode(&compiled.plan, &compiled.hash))
        .unwrap()
        .0
}

fn proposal(reference: u64, text: &str) -> PendingIngress {
    PendingIngress {
        handle: IngressProposalHandle::new(reference),
        proposal: IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["server"]).unwrap()),
            draft: EnvelopeDraft {
                port: node::Port::conventional_out(),
                data: Data::materialized(Value::Text(text.into())),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        },
    }
}

fn recover(vm: &mut Execution<'_>, journal: &MemJournal) {
    let retained = journal.records().to_vec();
    vm.recover_from_journal(journal).unwrap();
    assert_eq!(
        journal.records(),
        retained,
        "reconstruction commits nothing"
    );
}

#[test]
fn inbound_calls_recover_the_exact_request_and_response_once() {
    for respond in [false, true] {
        let source = format!(
            r#"
graph app : App {{
  output out: text
  output exit: variant
  connector server : HttpServer {{
    listen = "127.0.0.1:0"; codec = text
    mode = {}
    {}
  }}
  operator relay {{ input in: text; output out: text; on in {{ out = data }} }}
  server.out -> relay
  relay.out -> {}
  server.exit -> exit
}}
"#,
            if respond { "respond" } else { "ack" },
            if respond {
                "timeout = duration(\"3s\")"
            } else {
                ""
            },
            if respond { "{ out; server.in }" } else { "out" }
        );
        let plan = compile(&source);
        let mut vm = Execution::new(&plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut calls = Vec::new();
        for (reference, text) in [(1, "alice"), (2, "bob")] {
            let IngressAdmission::Accepted(accepted) = vm
                .accept_connector_output(&mut journal, &proposal(reference, text))
                .unwrap()
            else {
                panic!("uncongested ingress");
            };
            let Some(InboundExchangeId::Call(call)) = accepted.exchange else {
                panic!("Call reference");
            };
            calls.push(call);
            recover(&mut vm, &journal);
        }
        while vm.step(&mut journal).unwrap() {
            recover(&mut vm, &journal);
        }
        for call_id in calls {
            let completion = Completion::http_server_exit(HttpServerExit::Completed {
                status: if respond { 200 } else { 202 },
            });
            let event = InboundCallObservation {
                call_id,
                completion,
            };
            assert_eq!(vm.queue_inbound_call_events(vec![event.clone()]).queued, 1);
            vm.run_ready(&mut journal).unwrap();
            recover(&mut vm, &journal);
            assert_eq!(
                vm.queue_inbound_call_events(vec![event]).queued,
                0,
                "closed once"
            );
        }
        assert_eq!(
            vm.outputs()
                .iter()
                .filter(|output| output.port == node::Port::conventional_out())
                .count(),
            2
        );
        assert_eq!(
            vm.outputs()
                .iter()
                .filter(|output| output.port == node::Port::conventional_exit())
                .count(),
            2
        );
    }
}
