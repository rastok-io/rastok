//! Measure pending completion batches, cumulative journal retention and
//! recovery using the same allocator watermark as the relay qualification.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use cortex_m_semihosting::hprintln;
use rastok_core::{
    connector::{driver::*, exchange::*, stdlib::*},
    envelope::{Data, Namespace, Value},
    journal::MemJournal,
    node::{Path, Port},
    plan::{GraphVersion, SemanticHash, ValidatedExecutionPlan},
    runtime::execution::Execution,
};

struct World {
    ready: Vec<CallResponse>,
    starts: usize,
    preparations: usize,
    frames: usize,
    bytes: usize,
}

impl OutboundDrivers for World {
    fn prepare_request(
        &mut self,
        _: &Path,
        data: &Data,
        _: &BTreeMap<String, Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        self.preparations += 1;
        Ok(prepare_process_request(RequestCodec::Lines, data))
    }
    fn start(
        &mut self,
        intent: &CallOpen,
        _: &rastok_core::envelope::Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        self.starts += 1;
        let channel = || {
            (0..self.frames)
                .map(|_| Data::materialized(Value::Text("x".repeat(self.bytes))))
                .collect()
        };
        self.ready.push(CallResponse {
            call_id: intent.call_id,
            completion: Completion::process(channel(), channel(), ProcessExit::TimeoutExpired),
        });
        Ok(CallStart::Started)
    }
    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
        Ok(core::mem::take(&mut self.ready))
    }
    fn scheduler_completion(
        &mut self,
        _: &Path,
        event: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(process_scheduler_completion(event))
    }
    fn cancel(&mut self, _: &Path, _: &CallId) -> Result<(), DriverRouteError> {
        Ok(())
    }
}

fn parameter(value: Option<&str>, default: usize) -> usize {
    value
        .map(|value| value.parse().expect("positive integer spike parameter"))
        .unwrap_or(default)
}

pub(super) fn run(plan: &ValidatedExecutionPlan, hash: SemanticHash) -> usize {
    let operations = parameter(option_env!("RASTOK_RETRY_OPERATIONS"), 1);
    let frames = parameter(option_env!("RASTOK_RETRY_FRAMES"), 4);
    let bytes = parameter(option_env!("RASTOK_RETRY_FRAME_BYTES"), 4);
    assert!(operations > 0 && frames > 0 && bytes > 0);
    let mut world = World {
        ready: Vec::new(),
        starts: 0,
        preparations: 0,
        frames,
        bytes,
    };
    let mut vm = Execution::new_program(plan, hash, GraphVersion(1), 0, None);
    let mut journal = MemJournal::new(GraphVersion(1));
    for _ in 0..operations {
        vm.accept_input(
            &mut journal,
            IngressProposal {
                provenance: IngressProvenance::single(Path::new(["spike"]).unwrap()),
                draft: EnvelopeDraft {
                    port: Port::new("in").unwrap(),
                    data: Data::materialized(Value::Text(String::from("request"))),
                    ext: Namespace::new(),
                },
                sequence: IngressSequence::Next,
                session: None,
            },
        )
        .unwrap();
    }
    let mut now = 0;
    for _ in 0..4096 {
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        vm.poll_drivers(&mut journal, &mut world).unwrap();
        vm.run_ready_with_bindings(&mut journal, &mut world)
            .unwrap();
        if vm.is_stable() {
            break;
        }
        if world.ready.is_empty() {
            now += 1_000_000;
            vm.advance_to(&mut journal, &mut world, now).unwrap();
        }
    }
    assert!(vm.is_stable());
    assert_eq!(world.preparations, operations);
    hprintln!(
        "retry: {} operations / {} Calls / {} frames per channel / {} bytes per frame",
        operations,
        world.starts,
        frames,
        bytes
    );
    hprintln!(
        "retry_live_heap: used {} / peak {} bytes",
        super::HEAP.inner.used(),
        super::HEAP.peak.load(core::sync::atomic::Ordering::Relaxed)
    );
    let records = journal.records().len();
    drop(vm);
    let mut restored = Execution::new_program(plan, hash, GraphVersion(1), now, None);
    let starts = world.starts;
    restored
        .recover_with_bindings(&journal, &mut world)
        .unwrap();
    assert_eq!(world.starts, starts);
    assert_eq!(world.preparations, operations);
    assert!(restored.is_stable());
    hprintln!(
        "retry_recovered_heap: used {} / peak {} bytes",
        super::HEAP.inner.used(),
        super::HEAP.peak.load(core::sync::atomic::Ordering::Relaxed)
    );
    records
}
