//! The Vm end to end, driven through compiled probes — integration
//! tests of the core machine, living in the std crate because they
//! need the compiler.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
// Harness helpers live outside `#[test]` bodies, where the
// allow-unwrap-in-tests toggle does not reach; a broken fixture's
// correct report is a panic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rastok::boundary::{InboundSubmission, Intent, Partner};
use rastok::envelope::{Data, Envelope};
use rastok::journal::Journal;
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::{Vm, VmError};

use rastok::boundary::{
    CallOutcome, Draft, IngressAuthority, Response, SubmissionSequence, TypedError,
};
use rastok::compiler::compile;
use rastok::contract::Registry;
use rastok::envelope::{Encoding, Namespace, Value};
use rastok::journal::{MemJournal, OutboundStatus, Record};

const M0_PROBE: &str = r#"
graph m0_probe : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
  }

  operator shout {
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = data.text
}
  }

  . -> shout -> echo -> .
}
"#;

/// The world scripted: echoes every request back, like `cat`.
#[derive(Default)]
struct EchoWorld {
    seen: Vec<Intent>,
    fail_next: bool,
}

impl Partner for EchoWorld {
    fn start(&mut self, intent: &Intent, request: &Envelope) {
        let _ = request;
        self.seen.push(intent.clone());
    }

    fn events(&mut self) -> Vec<Response> {
        self.seen
            .drain(..)
            .map(|intent| Response {
                call_ref: intent.call_ref,
                outcome: if self.fail_next {
                    CallOutcome::Failure(TypedError {
                        kind: String::from("ExecFailed"),
                        data: Data::encoded(Encoding::Raw, *b"exit 1"),
                    })
                } else {
                    CallOutcome::Success(Data::encoded(Encoding::Raw, *b"salut"))
                },
            })
            .collect()
    }
}

fn submission(text: &str) -> InboundSubmission {
    let data = vec![(String::from("text"), Value::Text(String::from(text)))];
    InboundSubmission {
        authority: IngressAuthority::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Struct(data)),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn text_submission(text: &str) -> InboundSubmission {
    InboundSubmission {
        authority: IngressAuthority::single(node::Path::new(["cli"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Text(String::from(text))),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn run_probe() -> (MemJournal, Vec<rastok::runtime::vm::BoundaryOutput>) {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    (journal, vm.outputs().to_vec())
}

#[test]
fn the_m0_probe_runs_end_to_end() {
    let (journal, outputs) = run_probe();
    // The whole mechanical chain, in causal order: birth, the
    // operator's transaction, the call's open and closure facts,
    // the connector's outcome transaction.
    let kinds: Vec<&str> = journal
        .records()
        .iter()
        .map(|record| match record {
            Record::Inbound(_) => "inbound",
            Record::Transaction(t) => {
                if t.node_path.to_string() == "shout" {
                    "tx:shout"
                } else {
                    "tx:echo"
                }
            }
            Record::Outbound(o) => match o.status {
                OutboundStatus::Open => "outbound:open",
                OutboundStatus::Closed(_) => "outbound:closed",
                OutboundStatus::Abandoned => "outbound:abandoned",
            },
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "inbound",
            "tx:shout",
            "outbound:open",
            "outbound:closed",
            "tx:echo"
        ],
        "the normative record sequence"
    );
    // One envelope left the graph, with the root's origin.
    assert_eq!(outputs.len(), 1);
    let Record::Inbound(root) = &journal.records()[0] else {
        panic!("expected the birth");
    };
    assert_eq!(
        outputs[0].envelope.meta().sys().origin_id(),
        root.envelope.meta().sys().id(),
        "the lineage crosses the world boundary"
    );
}

#[test]
fn two_runs_produce_identical_journals() {
    let (once, first_outputs) = run_probe();
    let (twice, second_outputs) = run_probe();
    assert_eq!(once.records(), twice.records(), "replay determinism");
    assert_eq!(first_outputs, second_outputs);
}

#[test]
fn a_typed_failure_leaves_by_the_err_edge_or_stops() {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld {
        fail_next: true,
        ..EchoWorld::default()
    };
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    // The failure became the connector's err emission; no edge
    // routes err in the probe, so it bubbles to the root error
    // sink instead of vanishing — with its kind intact.
    assert!(vm.outputs().is_empty());
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the outcome transaction");
    };
    assert_eq!(transaction.emissions[0].port.name(), "err");
    assert_eq!(vm.errors().len(), 1);
    let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
        panic!("expected the typed error payload");
    };
    assert_eq!(
        payload.get("kind"),
        Some(&Value::Text(String::from("ExecFailed"))),
        "the error kind survives into the graph"
    );
}

#[test]
fn an_invented_error_kind_is_refused_loudly() {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    let open = journal.open_calls();
    let mut world = ScriptedWorld {
        answers: vec![Response {
            call_ref: open[0].call_ref,
            outcome: CallOutcome::Failure(TypedError {
                kind: String::from("MadeUp"),
                data: Data::encoded(Encoding::Raw, *b""),
            }),
        }],
    };
    let report = vm.pump(&mut journal, &mut world).unwrap();
    // Refused BEFORE the causal fact: the reply is rejected with
    // its reason, and the call stays open — the timeout machinery
    // can still settle it honestly.
    assert_eq!(report.settled, 0);
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].1.contains("MadeUp"));
    assert_eq!(journal.open_calls().len(), 1, "no garbage closure");
    vm.drain(&mut journal).unwrap();
    assert!(!vm.is_stable(), "the obligation is still open");
}

/// A world answering from a script, for integrity tests.
struct ScriptedWorld {
    answers: Vec<Response>,
}

impl Partner for ScriptedWorld {
    fn start(&mut self, _: &Intent, _: &Envelope) {}
    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.answers)
    }
}

#[test]
fn a_value_fault_becomes_a_typed_error_not_a_lost_envelope() {
    let source = r#"
graph g : App {
  input in: variant
  operator pick {
on in {
  out = data.missing
}
  }
  . -> pick -> .
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    // The input is consumed causally — a transaction exists — and
    // the fault left as a typed error on the root sink.
    assert!(vm.is_stable(), "the machine did not wedge");
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the fault transaction");
    };
    assert_eq!(transaction.emissions[0].port.name(), "err");
    assert_eq!(vm.errors().len(), 1);
    let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
        panic!("expected the typed error payload");
    };
    assert_eq!(
        payload.get("kind"),
        Some(&Value::Text(String::from("RuntimeFault")))
    );
}

/// A journal that refuses commits on demand — the durable-store
/// failure, scripted.
struct FlakyJournal {
    inner: MemJournal,
    refuse_commits: bool,
    refuse_opens: bool,
    refuse_closes: bool,
}

impl FlakyJournal {
    fn healthy(version: GraphVersion) -> Self {
        Self {
            inner: MemJournal::new(version),
            refuse_commits: false,
            refuse_opens: false,
            refuse_closes: false,
        }
    }
}

impl Journal for FlakyJournal {
    fn append_inbound(
        &mut self,
        submission: InboundSubmission,
    ) -> Result<rastok::journal::CommittedInbound, rastok::journal::Error> {
        self.inner.append_inbound(submission)
    }
    fn commit(
        &mut self,
        transaction: rastok::journal::TransactionRecord,
    ) -> Result<(), rastok::journal::Error> {
        if self.refuse_commits {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.commit(transaction)
    }
    fn open_outbound(&mut self, intent: Intent) -> Result<(), rastok::journal::Error> {
        if self.refuse_opens {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.open_outbound(intent)
    }
    fn close_outbound(
        &mut self,
        call_ref: &rastok::boundary::CallRef,
        closure: rastok::journal::Closure,
    ) -> Result<rastok::journal::CloseVerdict, rastok::journal::Error> {
        if self.refuse_closes {
            return Err(rastok::journal::Error::Storage(String::from("scripted")));
        }
        self.inner.close_outbound(call_ref, closure)
    }
    fn find_envelope(&self, id: rastok::envelope::Id) -> Option<Envelope> {
        self.inner.find_envelope(id)
    }
    fn find_call(&self, call_ref: &rastok::boundary::CallRef) -> Option<Intent> {
        self.inner.find_call(call_ref)
    }
    fn open_calls(&self) -> Vec<Intent> {
        self.inner.open_calls()
    }
}

#[test]
fn a_refused_commit_leaves_the_delivery_replayable() {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal {
        refuse_commits: true,
        ..FlakyJournal::healthy(GraphVersion(1))
    };
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    let error = vm.step(&mut journal).unwrap_err();
    assert!(matches!(error, VmError::Journal(_)));
    // The store recovers; the same delivery replays and commits —
    // nothing was consumed by the refusal.
    journal.refuse_commits = false;
    assert!(
        vm.step(&mut journal).unwrap(),
        "the delivery was still there"
    );
    assert!(matches!(
        journal.inner.records().last().unwrap(),
        Record::Transaction(_)
    ));
}

const TIMED_PROBE: &str = r#"
graph timed : App {
  connector echo : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
timeout = duration("5s")
  }

  operator shout {
var count : int = 0
on in where data.text != "" {
  count = count + 1
  out = data.text
}
  }

  . -> shout -> echo -> .
}
"#;

#[test]
fn a_silent_partner_expires_into_a_typed_timeout() {
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.has_open_obligations());
    // Nobody answers; the budget expires at the discrete advance.
    vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
        .unwrap();
    assert!(vm.is_stable(), "the expiry released the machine");
    assert_eq!(vm.now(), 5_000_000_000);
    // The closure is a Timeout fact and the outcome transaction
    // fired under l:timeout, a TimeoutExpired on err.
    let last = journal.records().last().unwrap();
    let Record::Transaction(transaction) = last else {
        panic!("expected the timeout transaction");
    };
    assert_eq!(
        transaction.trigger,
        node::Trigger::Lifecycle(node::Lifecycle::Timeout)
    );
    assert_eq!(transaction.emissions[0].port.name(), "err");
    assert_eq!(vm.errors().len(), 1);
    let Data::Materialized(Value::Map(payload)) = vm.errors()[0].data() else {
        panic!("expected the typed error payload");
    };
    assert_eq!(
        payload.get("kind"),
        Some(&Value::Text(String::from("TimeoutExpired")))
    );
    // The claim lifted: a second submission opens a second call.
    vm.inject(&mut journal, submission("encore")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(journal.open_calls().len(), 1);
}

#[test]
fn a_budget_runs_from_delivery_and_covers_queue_time() {
    // The claim serializes the connector: the second request
    // waits in the mailbox behind the open call. Its budget runs
    // anyway — queue time counts (execution-model §8) — and the
    // call-closed rule applies even to a call that expires
    // before it started: the queued request's call opens and
    // closes at the expiry, back to back.
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(journal.open_calls().len(), 1, "one open, one queued");
    // Nobody answers; both budgets expire at the same sweep.
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.errors().len(), 2, "both requests expired, typed");
    let statuses: Vec<&OutboundStatus> = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Outbound(o) => Some(&o.status),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(status, OutboundStatus::Open))
            .count(),
        2,
        "both calls opened — the queued one at its expiry"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| matches!(
                status,
                OutboundStatus::Closed(rastok::journal::Closure::Timeout)
            ))
            .count(),
        2,
        "both closed by the machine's Timeout fact"
    );
    // Both outcome transactions fired under the timeout
    // lifecycle, never a port trigger.
    let timeout_transactions = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Transaction(t)
                    if t.trigger == node::Trigger::Lifecycle(node::Lifecycle::Timeout)
            )
        })
        .count();
    assert_eq!(timeout_transactions, 2);
    // The sealed timers name the obligations the transactions
    // armed — one per budgeted delivery.
    let sealed: usize = journal
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(t) => Some(t.timers.len()),
            _ => None,
        })
        .sum();
    assert_eq!(sealed, 2);
}

#[test]
fn a_fan_out_expires_its_own_call_not_a_siblings() {
    // The seventh review's counter-example: one envelope fans out
    // to two budgeted connectors; the shorter budget must expire
    // ITS call — the obligation is (connector, delivery), never
    // the delivery alone.
    let source = r#"
graph fanout : App {
  connector alpha : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
timeout = duration("500ms")
  }

  connector beta : Process {
mode = job ; command = "cat"
codec = lines
effect = idempotent ; behavior = deterministic
timeout = duration("100ms")
  }

  operator shout {
on in { out = data.text }
  }

  . -> shout
  shout -> alpha -> nothing
  shout -> beta -> nothing
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(journal.open_calls().len(), 2, "both calls opened");
    // Only beta's budget is due at 100ms.
    vm.advance(&mut journal, &mut world, 100_000_000).unwrap();
    let open = journal.open_calls();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].connector_path.to_string(),
        "alpha",
        "alpha's call survived its sibling's expiry"
    );
}

#[test]
fn an_answer_and_an_expiry_race_to_a_single_closure() {
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    // The partner answers first; the expiry then loses the race —
    // one closure fact, one outcome transaction, no duplicate.
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
        .unwrap();
    assert!(vm.is_stable());
    let closures = journal
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record,
                Record::Outbound(o) if matches!(o.status, OutboundStatus::Closed(_))
            )
        })
        .count();
    assert_eq!(closures, 1, "close-once");
    assert_eq!(vm.outputs().len(), 1);
    assert!(vm.errors().is_empty(), "no phantom timeout");
}

#[test]
fn a_late_answer_is_a_reported_observation_not_a_silence() {
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    let call_ref = journal.open_calls()[0].call_ref;
    // The budget expires first…
    vm.advance(&mut journal, &mut EchoWorld::default(), 5_000_000_000)
        .unwrap();
    // …then the world answers a closed call.
    let mut late = ScriptedWorld {
        answers: vec![Response {
            call_ref,
            outcome: CallOutcome::Success(Data::encoded(Encoding::Raw, *b"tard")),
        }],
    };
    let report = vm.pump(&mut journal, &mut late).unwrap();
    assert_eq!(report.settled, 0);
    assert_eq!(report.discarded.len(), 1, "the late answer is visible");
    assert!(report.uncorrelated.is_empty());
}

#[test]
fn a_refused_expiry_stays_armed_and_replays() {
    // The eighth review's invariant: after any refusal, the
    // obligation and its delivery are either fully resolved or
    // replayable — never half-consumed, never duplicated.
    // Case: the pending call's Timeout closure is refused.
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal {
        refuse_closes: true,
        ..FlakyJournal::healthy(GraphVersion(1))
    };
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("salut")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(
        vm.advance(&mut journal, &mut world, 5_000_000_000).is_err(),
        "the refused closure surfaces"
    );
    // Heal and replay: the timer was never acked.
    journal.refuse_closes = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(
        vm.errors().len(),
        1,
        "exactly one typed timeout — no duplicate fact"
    );
}

#[test]
fn a_refused_open_on_a_queued_expiry_replays_without_duplicates() {
    // Case: the queued delivery's open fact is refused at expiry.
    let compilation = compile(
        &[TIMED_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(journal.inner.open_calls().len(), 1, "one open, one queued");
    journal.refuse_opens = true;
    assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
    journal.refuse_opens = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.errors().len(), 2, "both expired exactly once");
}

#[test]
fn a_refused_commit_on_a_queued_expiry_loses_nothing() {
    // Case: the queued delivery expires and the timeout
    // transaction's commit is refused — the delivery must stay.
    let compilation = compile(
        &["graph g : App {\n  connector echo : Process {\n    mode = job ; command = \"cat\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n    timeout = duration(\"5s\")\n  }\n  operator wrap {\n    on in { out = . }\n  }\n  . -> wrap -> echo -> nothing\n}\n"],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = FlakyJournal::healthy(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, text_submission("salut")).unwrap();
    // Drive only the wrap transaction, then expire the conforming
    // delivery while it is still queued. B2's ingress gate makes the
    // former unframeable-payload setup unreachable by construction.
    vm.step(&mut journal).unwrap();
    let facts_before = journal.inner.records().len();
    journal.refuse_commits = true;
    assert!(vm.advance(&mut journal, &mut world, 5_000_000_000).is_err());
    assert!(
        journal.inner.records()[facts_before..]
            .iter()
            .all(|record| !matches!(record, Record::Transaction(_))),
        "the open and closure facts may commit, but the refused transaction may not"
    );
    // Heal and replay: the delivery was never removed, the timer
    // never acked — exactly one typed error results.
    journal.refuse_commits = false;
    vm.advance(&mut journal, &mut world, 5_000_000_000).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.errors().len(), 1, "one typed error, no duplicate");
    let Data::Materialized(rastok::envelope::Value::Map(payload)) = vm.errors()[0].data() else {
        panic!("expected the typed error payload");
    };
    assert_eq!(
        payload.get("kind"),
        Some(&rastok::envelope::Value::Text(String::from(
            "TimeoutExpired"
        ))),
        "the kind survived the replay"
    );
    let terminal_transactions: Vec<_> = journal
        .inner
        .records()
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(t) if t.node_path.to_string() == "echo" => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        terminal_transactions.len(),
        1,
        "exactly one transaction consumed the delivery"
    );
    assert_eq!(terminal_transactions[0].consumed.len(), 1);
}

#[test]
fn a_suspended_call_keeps_its_mailbox_claimed() {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    let mut world = EchoWorld::default();
    vm.inject(&mut journal, submission("un")).unwrap();
    vm.inject(&mut journal, submission("deux")).unwrap();
    vm.drain(&mut journal).unwrap();
    // Two requests reached the connector, but only one call is
    // open: the second waits for the mailbox claim to lift.
    assert_eq!(journal.open_calls().len(), 1);
    assert!(vm.has_open_obligations());
    // Settle, then the claim lifts and the second call opens.
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    assert_eq!(journal.open_calls().len(), 1, "the second call is now open");
    vm.pump(&mut journal, &mut world).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable());
    assert_eq!(vm.outputs().len(), 2);
}

#[test]
fn a_filtered_input_reaches_stability_without_output() {
    let compilation = compile(
        &[M0_PROBE],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
    vm.inject(&mut journal, submission("")).unwrap();
    vm.drain(&mut journal).unwrap();
    assert!(vm.is_stable(), "the guard filtered; nothing is pending");
    assert!(vm.outputs().is_empty());
    // The consumption is still a causal fact.
    assert!(matches!(
        journal.records()[1],
        Record::Transaction(ref t) if t.emissions.is_empty()
    ));
}
