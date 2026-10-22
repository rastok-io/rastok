use super::*;
use crate::connector::ConnectorDirection;
use crate::connector::ExchangeKind;
use crate::connector::exchange::CallOpen;
use crate::connector::outbound_call_semantics;
use crate::connector::stdlib::PROCESS_CONNECTOR_ID;
use crate::envelope::{Data, Meta, MetaSys};
use crate::instance;
use crate::node;

use crate::journal::{
    Emission,
    deadline::{Deadline, Deadlines},
    ids,
};
use crate::node::Path;
use crate::plan::PlanConstant;
use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::{format, vec};

fn duration(ns: i64) -> PlanConstant {
    PlanConstant {
        expr: crate::expr::compile(
            &format!("duration(\"{ns}ns\")"),
            &crate::expr::Signature::new(),
        )
        .unwrap(),
        resolved: Value::Duration(ns),
    }
}

fn fixed(ns: i64) -> Option<DelayPolicy> {
    Some(DelayPolicy::Fixed {
        after: duration(ns),
    })
}

struct Fixture {
    instance: instance::Generation,
    contract: ContractId,
    policy: Policy,
    call: CallSemantics,
    envelope: Envelope,
    entry: Port,
    opening: Id,
    operation: OperationId,
    plan: SemanticHash,
    pin: Pin,
}

impl Fixture {
    fn new(attempts: u64) -> Self {
        let instance = instance::Generation::new(
            instance::InstanceAddress::singleton(
                &node::OwnerPath::new(vec![String::from("pay")]).unwrap(),
            ),
            0,
        );
        let contract = ContractId::parse(PROCESS_CONNECTOR_ID).unwrap();
        let call = outbound_call_semantics(
            &contract,
            ConnectorDirection::Outbound,
            ExchangeKind::Call,
            Some("job"),
        )
        .unwrap();
        let policy = Policy::try_new(
            call,
            attempts,
            vec!["TimeoutExpired".into()],
            fixed(100),
            true,
        )
        .unwrap();
        let envelope = Envelope::new(
            Meta::new(MetaSys::root(Id::from_bytes([1; 16]), 1)),
            Data::materialized(Value::Text("request".into())),
        );
        let entry = Port::new("in").unwrap();
        let opening = ids::delivery_id("App.in -> pay.in", envelope.meta().sys().id());
        let operation = operation_ref(&instance, &entry, opening);
        let plan = SemanticHash::from_bytes([9; 32]);
        let pin = Pin {
            envelope: envelope.meta().sys().id(),
            prepared: PreparedRequest::seal(vec![1, 2, 3]),
            settings: BTreeMap::from([("command".into(), Value::Text("cat".into()))]),
            key: Some("same-operation".into()),
            opened_at: 0,
        };
        Self {
            instance,
            contract,
            policy,
            call,
            envelope,
            entry,
            opening,
            operation,
            plan,
            pin,
        }
    }

    fn authority(&self) -> Opening<'_> {
        Opening {
            instance: &self.instance,
            delivery: self.opening,
            plan: self.plan,
            contract: &self.contract,
            call: self.call,
            policy: &self.policy,
            envelope: &self.envelope,
        }
    }

    fn attempt(&self, ordinal: u64) -> AttemptId {
        AttemptId {
            operation: self.operation,
            ordinal,
        }
    }

    fn opened(&self) -> Fact {
        Fact::Opened {
            operation: self.operation,
            opening: self.opening,
            entry: self.entry.clone(),
            plan: self.plan,
            contract: self.contract.clone(),
            pin: self.pin.clone(),
        }
    }

    fn admitted(&self, ordinal: u64, previous: Option<(CallId, Id)>) -> Fact {
        let (envelope, delivery) = match previous {
            None => (self.envelope.clone(), self.opening),
            Some((call, parent)) => {
                let id = admission_ref(self.operation, ordinal, call);
                let parent = MetaSys::emission(parent, self.envelope.meta().sys(), 1);
                let envelope = Envelope::new(
                    Meta::new(MetaSys::emission(id, &parent, 1))
                        .with_app(self.envelope.meta().app().clone())
                        .with_ext(self.envelope.meta().ext().clone()),
                    self.envelope.data().clone(),
                );
                (
                    envelope,
                    ids::delivery_id(&admission_edge(self.operation, &self.entry), id),
                )
            }
        };
        Fact::Admitted {
            attempt: self.attempt(ordinal),
            envelope,
            delivery,
        }
    }

    fn start(&self) -> RetryView {
        let mut fold = RetryView::default();
        fold.apply(
            &manager(vec![self.opened(), self.admitted(1, None)]),
            Some(&self.authority()),
            0,
        )
        .unwrap();
        fold
    }

    fn intent(&self, fold: &RetryView) -> CallOpen {
        let state = fold.operation(self.operation).unwrap();
        let edge = if state.ordinal == 1 {
            "App.in -> pay.in".into()
        } else {
            admission_edge(self.operation, &self.entry)
        };
        CallOpen {
            call_id: ids::call_id(state.delivery, self.pin.prepared.digest()),
            emitting_id: state.envelope.meta().sys().id(),
            edge,
            connector_path: Path::new(["pay"]).unwrap(),
            prepared: self.pin.prepared.clone(),
            idempotency_key: self.pin.key.clone(),
            resolved_settings: self.pin.settings.clone(),
            logical_time: state.admitted_at().unwrap(),
        }
    }

    fn settle(
        &self,
        fold: &mut RetryView,
        ordinal: u64,
        case: &str,
        at: i64,
        stopping: bool,
    ) -> (CallOpen, Vec<Emission>, Batch) {
        let intent = self.intent(fold);
        fold.open_call(self.attempt(ordinal), &intent).unwrap();
        if stopping {
            fold.begin_closing(&self.instance);
        }
        let schedule = (!stopping && self.policy.repeats(ordinal, case))
            .then(|| self.policy.due_after(ordinal, at))
            .flatten()
            .map(|due| {
                Deadlines::try_new(
                    at,
                    vec![Deadline {
                        kind: crate::journal::DeadlineKind::RecurrenceDelay,
                        reference: admission_ref(self.operation, ordinal + 1, intent.call_id),
                        due_at: due,
                    }],
                )
                .unwrap()
            });
        fold.settle(
            self.attempt(ordinal),
            &Settlement {
                intent: &intent,
                case,
                closed_at: at,
                schedule: schedule.as_ref(),
            },
        )
        .unwrap();
        let parent = fold
            .operation(self.operation)
            .unwrap()
            .envelope
            .meta()
            .sys();
        let emissions: Vec<_> = [
            ("out", Value::Text("stdout".into())),
            ("err", Value::Text("stderr".into())),
            (
                "exit",
                Value::TaggedUnion(Box::new(crate::value::TaggedUnionValue {
                    case: case.into(),
                    payload: vec![],
                })),
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (port, value))| Emission {
            port: Port::new(port).unwrap(),
            index: 0,
            envelope: Envelope::new(
                Meta::new(MetaSys::emission(
                    Id::derive(
                        crate::envelope::IdKind::Emission,
                        &[parent.id().as_bytes(), &(index as u32).to_le_bytes()],
                    ),
                    parent,
                    1,
                )),
                Data::materialized(value),
            ),
        })
        .collect();
        let batch = Batch {
            attempt: self.attempt(ordinal),
            call: intent.call_id,
            emissions: emissions
                .iter()
                .map(|e| e.envelope.meta().sys().id())
                .collect(),
        };
        (intent, emissions, batch)
    }
}

fn manager(facts: Vec<Fact>) -> RetryRecord {
    RetryRecord {
        context: None,
        facts,
    }
}

fn carrier(
    f: &Fixture,
    packet: RetryRecord,
    emissions: Vec<Emission>,
    manager: bool,
) -> crate::journal::TransactionRecord {
    crate::journal::TransactionRecord {
        owner_path: f.instance.address().owner().clone(),
        instance: f.instance.clone(),
        parent_instance: None,
        close_scope: None,
        trigger: if manager {
            crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Retry)
        } else {
            crate::node::Trigger::Port(f.entry.clone())
        },
        cause: manager.then_some(f.envelope.meta().sys().id()),
        exchange_exit: None,
        loop_record: None,
        instance_commands: None,
        retry_record: Some(Box::new(packet)),
        deadlines: None,
        logical_time: None,
        consumed: if manager {
            vec![]
        } else {
            vec![crate::journal::ConsumedPosition {
                edge: "App.in -> pay.in".into(),
                source: f.instance.clone(),
                seq: 0,
            }]
        },
        join_inputs: vec![],
        state_delta: vec![],
        emissions,
        routes: vec![],
        deadline_ids: vec![],
    }
}

#[test]
fn retry_manager_retention_does_not_invent_a_semantic_now_read() {
    let f = Fixture::new(3);
    let mut opened = carrier(
        &f,
        manager(vec![f.opened(), f.admitted(1, None)]),
        vec![],
        true,
    );
    // Pin.opened_at retains preparation time; no DSL expression read now().
    // Later admissions retain their time in the mandatory Call timeout deadline.
    opened.logical_time = None;
    validate_transaction(&opened).unwrap();
    opened.logical_time = Some(0);
    assert_eq!(validate_transaction(&opened), Err(Error::RecordShape));
}

#[test]
fn retained_retry_carriers_cannot_hide_dispositions_or_application_work() {
    let f = Fixture::new(3);
    let mut fold = f.start();
    let (_, emissions, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
    let producer = carrier(&f, held(&batch), emissions, false);
    validate_transaction(&producer).unwrap();
    let opened = carrier(
        &f,
        manager(vec![f.opened(), f.admitted(1, None)]),
        vec![],
        true,
    );
    validate_transaction(&opened).unwrap();
    for mutation in 0..6 {
        let mut forged = producer.clone();
        match mutation {
            0 => forged.retry_record.as_mut().unwrap().facts.clear(),
            1 => forged.trigger = crate::node::Trigger::Lifecycle(crate::node::Lifecycle::Retry),
            2 => forged.emissions.swap(0, 1),
            3 => forged.routes.push(crate::journal::RoutedDelivery {
                delivery: batch.emissions[0],
                edge: "pay.out -> sink.in".into(),
                target: f.instance.clone(),
            }),
            4 => forged.state_delta.push(("hidden".into(), Value::Int(1))),
            _ => forged.retry_record.as_mut().unwrap().context = None,
        }
        assert!(
            validate_transaction(&forged).is_err(),
            "mutation {mutation}"
        );
        let mut journal = crate::journal::MemJournal::new(crate::plan::GraphVersion(1));
        assert!(matches!(
            crate::journal::Journal::commit(&mut journal, forged),
            Err(crate::journal::Error::InvalidRetryProtocol(_))
        ));
        assert!(journal.records().is_empty());
    }
    let mut orphan = opened;
    orphan.retry_record = None;
    assert_eq!(validate_transaction(&orphan), Err(Error::RecordShape));
    assert!(
        crate::journal::codec::Decoder::default()
            .decode(&crate::journal::codec::encode(
                &crate::journal::Record::Transaction(Box::new(orphan))
            ))
            .is_err()
    );
}

#[test]
fn retry_admissions_are_resolved_as_retained_envelopes_without_port_emissions() {
    let f = Fixture::new(3);
    let record = crate::journal::Record::Transaction(Box::new(carrier(
        &f,
        manager(vec![f.opened(), f.admitted(1, None)]),
        vec![],
        true,
    )));
    assert_eq!(
        crate::journal::recorded_envelope(&[record], f.envelope.meta().sys().id()),
        Some(&f.envelope)
    );
}
fn held(batch: &Batch) -> RetryRecord {
    RetryRecord {
        context: Some(batch.attempt),
        facts: vec![Fact::Held(batch.clone())],
    }
}
fn publish(batch: &Batch) -> RetryRecord {
    RetryRecord {
        context: Some(batch.attempt),
        facts: vec![
            Fact::Published(batch.clone()),
            Fact::Closed {
                attempt: batch.attempt,
                reason: CloseReason::Final,
            },
        ],
    }
}
fn advance(batch: &Batch) -> RetryRecord {
    manager(vec![
        Fact::Consumed(batch.clone()),
        Fact::Advanced {
            from: batch.attempt,
            to: batch.attempt.ordinal + 1,
            call: batch.call,
        },
    ])
}

fn refuses_unchanged(fold: &mut RetryView, packet: &RetryRecord, expected: Error, at: i64) {
    let before = fold.clone();
    assert_eq!(fold.apply(packet, None, at), Err(expected));
    assert_eq!(*fold, before);
}

#[test]
fn exact_delivery_and_generation_separate_operations_with_identical_requests() {
    let f = Fixture::new(3);
    assert_ne!(
        f.operation,
        operation_ref(&f.instance, &f.entry, Id::from_bytes([2; 16]))
    );
    assert_ne!(
        f.operation,
        operation_ref(
            &instance::Generation::new(f.instance.address().clone(), 1),
            &f.entry,
            f.opening
        )
    );
    assert_ne!(
        f.operation,
        operation_ref(&f.instance, &Port::new("request").unwrap(), f.opening)
    );
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[0x0a]);
    for bytes in [
        f.instance.address().as_bytes(),
        &0_u64.to_le_bytes(),
        b"in",
        f.opening.as_bytes(),
    ] {
        hasher.update(&(bytes.len() as u32).to_le_bytes());
        hasher.update(bytes);
    }
    let expected = Id::from_bytes(hasher.finalize().as_bytes()[..16].try_into().unwrap());
    assert_eq!(f.operation.id(), expected);
    let call = CallId::new(Id::from_bytes([3; 16]));
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[0x04]);
    for bytes in [
        expected.as_bytes().as_slice(),
        &2_u64.to_le_bytes(),
        call.id().as_bytes(),
    ] {
        hasher.update(&(bytes.len() as u32).to_le_bytes());
        hasher.update(bytes);
    }
    let expected_admission = Id::from_bytes(hasher.finalize().as_bytes()[..16].try_into().unwrap());
    assert_eq!(admission_ref(f.operation, 2, call), expected_admission);
    assert_eq!(
        (expected.to_string(), expected_admission.to_string()),
        (
            String::from("f09790a0bce6f64ec5ae4edacec19f2f"),
            String::from("05cbdcc63115d77e8aeff05c0f1958dc")
        )
    );
    assert_ne!(
        admission_ref(f.operation, 2, call),
        admission_ref(f.operation, 3, call)
    );
    assert_ne!(
        admission_ref(f.operation, 2, call),
        admission_ref(f.operation, 2, CallId::new(Id::from_bytes([4; 16])))
    );
    assert!(admission_edge(f.operation, &f.entry).starts_with("@retry:"));
}

#[test]
fn delayed_receipt_recovers_the_original_due_time_and_never_reopens_the_old_call() {
    let f = Fixture::new(3);
    let mut fold = f.start();
    let (first, emissions, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
    fold.validate_emissions(&held(&batch), &emissions).unwrap();
    fold.apply(&held(&batch), None, 11).unwrap();
    let mut restored = fold.clone(); // retained-prefix fold, not a manager queue
    assert_eq!(
        restored.operation(f.operation).unwrap().pending.as_ref(),
        Some(&batch)
    );
    assert_eq!(
        restored.open_call(f.attempt(1), &first),
        Err(Error::Transition)
    );
    restored.apply(&advance(&batch), None, 250).unwrap();
    let state = restored.operation(f.operation).unwrap();
    assert_eq!(state.due_at, Some(110));
    assert!(state.pending.is_none());
    let parent = *batch.emissions.last().unwrap();
    restored
        .apply(
            &manager(vec![f.admitted(2, Some((first.call_id, parent)))]),
            None,
            250,
        )
        .unwrap();
    let (second, emissions, batch) = f.settle(&mut restored, 2, "Exited", 260, false);
    assert_ne!(first.call_id, second.call_id);
    assert_eq!(first.prepared, second.prepared);
    assert_eq!(first.idempotency_key, second.idempotency_key);
    restored
        .validate_emissions(&publish(&batch), &emissions)
        .unwrap();
    restored.apply(&publish(&batch), None, 261).unwrap();
    assert_eq!(
        restored.operation(f.operation).unwrap().phase,
        Phase::Closed
    );
    assert_eq!(
        restored.operation(f.operation).unwrap().closed,
        Some(CloseReason::Final)
    );
    assert_eq!(restored.operation(f.operation).unwrap().due_at, None);
}

#[test]
fn the_bound_publishes_the_last_failure_without_an_extra_attempt_or_wrapper() {
    for attempts in [1, 3] {
        let f = Fixture::new(attempts);
        let mut fold = f.start();
        for ordinal in 1..=attempts {
            let at = ordinal as i64 * 1000;
            let (intent, emissions, batch) =
                f.settle(&mut fold, ordinal, "TimeoutExpired", at, false);
            if ordinal == attempts {
                refuses_unchanged(&mut fold, &held(&batch), Error::Transition, at);
                fold.validate_emissions(&publish(&batch), &emissions)
                    .unwrap();
                fold.apply(&publish(&batch), None, at).unwrap();
            } else {
                refuses_unchanged(&mut fold, &publish(&batch), Error::Transition, at);
                fold.apply(&held(&batch), None, at).unwrap();
                fold.apply(&advance(&batch), None, at).unwrap();
                fold.apply(
                    &manager(vec![f.admitted(
                        ordinal + 1,
                        Some((intent.call_id, *batch.emissions.last().unwrap())),
                    )]),
                    None,
                    at + 100,
                )
                .unwrap();
            }
        }
        assert_eq!(fold.operation(f.operation).unwrap().ordinal, attempts);
        assert_eq!(
            fold.operation(f.operation).unwrap().closed,
            Some(CloseReason::Final)
        );
    }
}

#[test]
fn closing_during_backoff_consumes_once_without_publishing_any_failure() {
    for consumed in [false, true] {
        let f = Fixture::new(3);
        let mut fold = f.start();
        let (_, _, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
        fold.apply(&held(&batch), None, 10).unwrap();
        if consumed {
            fold.apply(&advance(&batch), None, 10).unwrap();
        }
        fold.begin_closing(&f.instance);
        let close = Fact::Closed {
            attempt: f.attempt(1),
            reason: CloseReason::Stopped,
        };
        let facts = if consumed {
            vec![close]
        } else {
            vec![Fact::Consumed(batch.clone()), close]
        };
        refuses_unchanged(&mut fold, &publish(&batch), Error::Transition, 20);
        if !consumed {
            refuses_unchanged(&mut fold, &advance(&batch), Error::Transition, 20);
        }
        fold.apply(&manager(facts), None, 20).unwrap();
        assert_eq!(
            fold.operation(f.operation).unwrap().closed,
            Some(CloseReason::Stopped)
        );
        assert!(fold.operation(f.operation).unwrap().pending.is_none());
        refuses_unchanged(&mut fold, &advance(&batch), Error::Transition, 200);
    }
}

#[test]
fn closing_after_settlement_does_not_change_its_already_selected_disposition() {
    let f = Fixture::new(3);
    let mut fold = f.start();
    let (_, _, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
    fold.begin_closing(&f.instance);
    refuses_unchanged(&mut fold, &publish(&batch), Error::Transition, 11);
    fold.apply(&held(&batch), None, 11).unwrap();
    fold.apply(
        &manager(vec![
            Fact::Consumed(batch.clone()),
            Fact::Closed {
                attempt: batch.attempt,
                reason: CloseReason::Stopped,
            },
        ]),
        None,
        12,
    )
    .unwrap();
    // A still-open Call when Closing wins follows ordinary final settlement.
    let mut fold = f.start();
    let (_, _, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, true);
    fold.apply(&publish(&batch), None, 11).unwrap();
}

#[test]
fn atomic_pairs_whole_batch_and_exact_admissions_cannot_be_forged() {
    let f = Fixture::new(3);
    let mut fold = f.start();
    let (intent, emissions, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
    for ids in [
        vec![],
        batch.emissions[..2].to_vec(),
        vec![batch.emissions[0]; 3],
        batch.emissions.iter().rev().copied().collect(),
    ] {
        let mut forged = batch.clone();
        forged.emissions = ids;
        assert_eq!(
            fold.validate_emissions(&held(&forged), &emissions),
            Err(Error::Batch)
        );
    }
    fold.apply(&held(&batch), None, 10).unwrap();
    refuses_unchanged(
        &mut fold,
        &manager(vec![Fact::Consumed(batch.clone())]),
        Error::RecordShape,
        11,
    );
    let mut forged = batch.clone();
    forged.emissions.swap(0, 1);
    refuses_unchanged(&mut fold, &advance(&forged), Error::Batch, 11);
    let early =
        manager(vec![f.admitted(
            2,
            Some((intent.call_id, *batch.emissions.last().unwrap())),
        )]);
    refuses_unchanged(&mut fold, &early, Error::Transition, 111);
    fold.apply(&advance(&batch), None, 20).unwrap();
    refuses_unchanged(&mut fold, &advance(&batch), Error::Batch, 20);
    refuses_unchanged(&mut fold, &early, Error::Timing, 109);
    let mut bad = early.clone();
    if let Fact::Admitted { delivery, .. } = &mut bad.facts[0] {
        *delivery = f.opening;
    }
    refuses_unchanged(&mut fold, &bad, Error::AdmissionIdentity, 110);
    fold.apply(&early, None, 110).unwrap();
    let mut changed = f.intent(&fold);
    changed
        .resolved_settings
        .insert("command".into(), Value::Text("different".into()));
    let before = fold.clone();
    assert_eq!(
        fold.open_call(f.attempt(2), &changed),
        Err(Error::PinnedCall)
    );
    assert_eq!(fold, before);
}

#[test]
fn pending_initial_stop_has_no_fake_admission_and_successor_generation_is_independent() {
    let f = Fixture::new(3);
    let mut fold = RetryView::default();
    fold.apply(&manager(vec![f.opened()]), Some(&f.authority()), 0)
        .unwrap();
    fold.begin_closing(&instance::Generation::new(f.instance.address().clone(), 1));
    refuses_unchanged(
        &mut fold,
        &manager(vec![Fact::Closed {
            attempt: f.attempt(0),
            reason: CloseReason::Stopped,
        }]),
        Error::Transition,
        0,
    );
    fold.begin_closing(&f.instance);
    fold.apply(
        &manager(vec![Fact::Closed {
            attempt: f.attempt(0),
            reason: CloseReason::Stopped,
        }]),
        None,
        0,
    )
    .unwrap();
    assert_eq!(fold.operation(f.operation).unwrap().ordinal, 0);
    refuses_unchanged(
        &mut fold,
        &manager(vec![f.admitted(1, None)]),
        Error::Transition,
        1,
    );
}

#[test]
fn policy_and_saturating_delays_are_closed_and_bounded_without_hidden_time() {
    let f = Fixture::new(3);
    for (attempts, cases, idempotent) in [
        (0, vec![], true),
        (1, vec!["TimeoutExpired".into()], false),
        (3, vec!["RetryExpired".into()], true),
        (3, vec!["Exited".into(), "Exited".into()], true),
    ] {
        assert_eq!(
            Policy::try_new(f.call, attempts, cases, None, idempotent),
            Err(Error::Policy)
        );
    }
    assert!(Policy::try_new(f.call, 3, vec!["SpawnFailed".into()], None, false).is_ok());
    let exponential = DelayPolicy::Exponential {
        first: duration(3),
        ceiling: duration(10),
    };
    for (n, expected) in [
        (0, None),
        (1, Some(3)),
        (2, Some(6)),
        (3, Some(10)),
        (64, Some(10)),
        (u64::MAX, Some(10)),
    ] {
        assert_eq!(exponential.duration_after(n), expected);
    }
    let policy = Policy::try_new(
        f.call,
        u64::MAX,
        vec!["TimeoutExpired".into()],
        Some(exponential),
        true,
    )
    .unwrap();
    assert_eq!(policy.due_after(2, i64::MAX - 1), Some(i64::MAX));
    assert_eq!(policy.due_after(1, i64::MIN), Some(i64::MIN + 3));
    assert!(!policy.repeats(0, "TimeoutExpired"));
    assert!(!policy.repeats(u64::MAX, "TimeoutExpired"));
    assert_eq!(
        Policy::try_new(f.call, 3, vec![], fixed(0), true),
        Err(Error::Policy)
    );
}

#[test]
fn missing_renewed_foreign_and_final_deadlines_refuse_atomically() {
    let f = Fixture::new(3);
    let mut fold = f.start();
    let intent = f.intent(&fold);
    fold.open_call(f.attempt(1), &intent).unwrap();
    let expected = admission_ref(f.operation, 2, intent.call_id);
    let valid = Deadline {
        kind: crate::journal::DeadlineKind::RecurrenceDelay,
        reference: expected,
        due_at: 110,
    };
    let candidates = [
        None,
        Some(
            Deadlines::try_new(
                20,
                vec![Deadline {
                    due_at: 120,
                    ..valid
                }],
            )
            .unwrap(),
        ),
        Some(
            Deadlines::try_new(
                10,
                vec![Deadline {
                    due_at: 111,
                    ..valid
                }],
            )
            .unwrap(),
        ),
        Some(
            Deadlines::try_new(
                10,
                vec![Deadline {
                    kind: crate::journal::DeadlineKind::CallTimeout,
                    ..valid
                }],
            )
            .unwrap(),
        ),
        Some(
            Deadlines::try_new(
                10,
                vec![Deadline {
                    reference: Id::from_bytes([0; 16]),
                    ..valid
                }],
            )
            .unwrap(),
        ),
        Some(Deadlines::try_new(10, vec![valid, valid]).unwrap()),
    ];
    for schedule in candidates {
        let before = fold.clone();
        assert_eq!(
            fold.settle(
                f.attempt(1),
                &Settlement {
                    intent: &intent,
                    case: "TimeoutExpired",
                    closed_at: 10,
                    schedule: schedule.as_ref()
                }
            ),
            Err(Error::Timing)
        );
        assert_eq!(fold, before);
    }
    let schedule = Deadlines::try_new(10, vec![valid]).unwrap();
    assert_eq!(
        fold.settle(
            f.attempt(1),
            &Settlement {
                intent: &intent,
                case: "Exited",
                closed_at: 10,
                schedule: Some(&schedule)
            }
        ),
        Err(Error::Timing)
    );
    assert_eq!(
        fold.settle(
            f.attempt(1),
            &Settlement {
                intent: &intent,
                case: "RetryExpired",
                closed_at: 10,
                schedule: None
            }
        ),
        Err(Error::Terminal)
    );
    fold.settle(
        f.attempt(1),
        &Settlement {
            intent: &intent,
            case: "TimeoutExpired",
            closed_at: 10,
            schedule: Some(&schedule),
        },
    )
    .unwrap();
    let before = fold.clone();
    assert_eq!(
        fold.settle(
            f.attempt(1),
            &Settlement {
                intent: &intent,
                case: "Exited",
                closed_at: 11,
                schedule: None
            }
        ),
        Err(Error::Transition)
    );
    assert_eq!(fold, before);
}

#[test]
fn opening_is_bound_to_original_plan_contract_input_and_exact_identity() {
    let f = Fixture::new(3);
    let packet = manager(vec![f.opened(), f.admitted(1, None)]);
    let mut fold = RetryView::default();
    assert_eq!(fold.apply(&packet, None, 0), Err(Error::OriginalBinding));
    let mut authority = f.authority();
    authority.delivery = Id::from_bytes([80; 16]);
    assert_eq!(
        fold.apply(&packet, Some(&authority), 0),
        Err(Error::OriginalBinding)
    );
    assert_eq!(fold, RetryView::default());
    let other_call = outbound_call_semantics(
        &ContractId::parse(crate::connector::stdlib::FILE_CONNECTOR_ID).unwrap(),
        ConnectorDirection::Outbound,
        ExchangeKind::Call,
        Some("read"),
    )
    .unwrap();
    let other_policy = Policy::try_new(other_call, 3, vec![], None, true).unwrap();
    let mut authority = f.authority();
    authority.policy = &other_policy;
    assert_eq!(
        fold.apply(&packet, Some(&authority), 0),
        Err(Error::OriginalBinding)
    );
    assert_eq!(fold, RetryView::default());
    for which in 0..5 {
        let mut bad = packet.clone();
        if let Fact::Opened {
            operation,
            plan,
            contract,
            pin,
            ..
        } = &mut bad.facts[0]
        {
            match which {
                0 => *operation = OperationId::from_id(Id::from_bytes([0; 16])),
                1 => *plan = SemanticHash::from_bytes([0; 32]),
                2 => {
                    *contract =
                        ContractId::parse(crate::connector::stdlib::FILE_CONNECTOR_ID).unwrap()
                }
                3 => pin.envelope = Id::from_bytes([0; 16]),
                _ => pin.opened_at = 1,
            }
        }
        assert!(fold.apply(&bad, Some(&f.authority()), 0).is_err());
        assert_eq!(fold, RetryView::default());
    }
    // A bad initial admission rolls back Opened in the same transaction too.
    let mut bad = packet.clone();
    if let Fact::Admitted { delivery, .. } = &mut bad.facts[1] {
        *delivery = Id::from_bytes([0; 16]);
    }
    assert_eq!(
        fold.apply(&bad, Some(&f.authority()), 0),
        Err(Error::AdmissionIdentity)
    );
    assert_eq!(fold, RetryView::default());
    fold.apply(&packet, Some(&f.authority()), 0).unwrap();
    let before = fold.clone();
    assert_eq!(
        fold.apply(&packet, Some(&f.authority()), 0),
        Err(Error::DuplicateOperation)
    );
    assert_eq!(fold, before);
    refuses_unchanged(&mut fold, &RetryRecord::default(), Error::RecordShape, 0);
    refuses_unchanged(
        &mut fold,
        &RetryRecord {
            context: Some(f.attempt(1)),
            facts: vec![],
        },
        Error::RecordShape,
        0,
    );
}

#[test]
fn each_algebra_prefix_reconstructs_without_a_saved_manager_or_driver() {
    #[derive(Clone)]
    enum Retained {
        Record(RetryRecord, i64),
        Open(CallOpen, AttemptId),
        Settle(CallOpen, AttemptId, String, i64, Option<Deadlines>),
    }
    fn apply(f: &Fixture, fold: &mut RetryView, fact: &Retained) {
        match fact {
            Retained::Record(packet, at) => fold.apply(packet, Some(&f.authority()), *at).unwrap(),
            Retained::Open(intent, attempt) => fold.open_call(*attempt, intent).unwrap(),
            Retained::Settle(intent, attempt, case, at, schedule) => fold
                .settle(
                    *attempt,
                    &Settlement {
                        intent,
                        case,
                        closed_at: *at,
                        schedule: schedule.as_ref(),
                    },
                )
                .unwrap(),
        }
    }
    let f = Fixture::new(3);
    let mut history = vec![Retained::Record(
        manager(vec![f.opened(), f.admitted(1, None)]),
        0,
    )];
    let mut live = f.start();
    for ordinal in 1..=3 {
        let at = ordinal as i64 * 1000;
        let intent = f.intent(&live);
        history.push(Retained::Open(intent.clone(), f.attempt(ordinal)));
        let schedule = (ordinal < 3).then(|| {
            Deadlines::try_new(
                at,
                vec![Deadline {
                    kind: crate::journal::DeadlineKind::RecurrenceDelay,
                    reference: admission_ref(f.operation, ordinal + 1, intent.call_id),
                    due_at: at + 100,
                }],
            )
            .unwrap()
        });
        history.push(Retained::Settle(
            intent.clone(),
            f.attempt(ordinal),
            "TimeoutExpired".into(),
            at,
            schedule,
        ));
        let (_, _, batch) = f.settle(&mut live, ordinal, "TimeoutExpired", at, false);
        let packet = if ordinal < 3 {
            held(&batch)
        } else {
            publish(&batch)
        };
        live.apply(&packet, None, at).unwrap();
        history.push(Retained::Record(packet, at));
        if ordinal < 3 {
            let packet = advance(&batch);
            live.apply(&packet, None, at + 250).unwrap();
            history.push(Retained::Record(packet, at + 250));
            let packet = manager(vec![f.admitted(
                ordinal + 1,
                Some((intent.call_id, *batch.emissions.last().unwrap())),
            )]);
            live.apply(&packet, None, at + 250).unwrap();
            history.push(Retained::Record(packet, at + 250));
        }
    }
    let mut expected = RetryView::default();
    for (index, fact) in history.iter().enumerate() {
        apply(&f, &mut expected, fact);
        let mut recovered = RetryView::default();
        for retained in &history[..=index] {
            apply(&f, &mut recovered, retained);
        }
        assert_eq!(recovered, expected, "prefix {index}");
        let snapshots: Vec<_> = recovered.operations().collect();
        assert!(
            snapshots
                .iter()
                .filter(|(_, state)| state.pending.is_some())
                .count()
                <= 1
        );
    }
    assert_eq!(expected, live);
    // This tests the fact algebra, not full record-carrier admission,
    // Worker admission/receipt caps or spatial RecoveryFold integration.
}

#[test]
fn repeated_travelling_metadata_uses_canonical_nan_and_signed_zero_identity() {
    use crate::envelope::{MetaValue, Scalar};
    let mut f = Fixture::new(3);
    let meta = f.envelope.meta().clone().with_app(BTreeMap::from([
        ("nan".into(), MetaValue::Scalar(Scalar::Double(f64::NAN))),
        ("zero".into(), MetaValue::Scalar(Scalar::Double(-0.0))),
    ]));
    f.envelope = Envelope::new(meta, f.envelope.data().clone());
    let mut fold = f.start();
    let (intent, emissions, batch) = f.settle(&mut fold, 1, "TimeoutExpired", 10, false);
    fold.apply(&held(&batch), None, 10).unwrap();
    fold.apply(&advance(&batch), None, 110).unwrap();
    let terminal = emissions.last().unwrap().envelope.meta().sys().id();
    let valid = manager(vec![f.admitted(2, Some((intent.call_id, terminal)))]);
    let mut changed = valid.clone();
    if let Fact::Admitted { envelope, .. } = &mut changed.facts[0] {
        let mut app = envelope.meta().app().clone();
        app.insert("zero".into(), MetaValue::Scalar(Scalar::Double(0.0)));
        *envelope = Envelope::new(
            envelope.meta().clone().with_app(app),
            envelope.data().clone(),
        );
    }
    assert_eq!(
        fold.apply(&changed, None, 110),
        Err(Error::AdmissionIdentity)
    );
    fold.apply(&valid, None, 110).unwrap();
    assert_eq!(fold.operation(f.operation).unwrap().ordinal, 2);
}
