//! M0.5 spike, volet A: the Rastok kernel on an emulated Cortex-M33
//! (QEMU mps2-an505). Boots from the sealed plan blob, runs the echo
//! graph's transactions, and reports: blob-open cost, per-transaction
//! cost, blake3 id cost (all in calibrated instruction counts), and
//! the heap watermark. Timing is a PROXY under `-icount` — the
//! verdict note states the caveats.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use core::mem::MaybeUninit;

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use embedded_alloc::LlffHeap as Heap;
use panic_semihosting as _;

use rastok_core::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
use rastok_core::envelope::{Data, Id, Namespace, Scalar, Value};
use rastok_core::journal::MemJournal;
use rastok_core::node::Path;
use rastok_core::plan::GraphVersion;
use rastok_core::runtime::vm::Vm;
use rastok_core::{blob, journal};

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 256 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

/// The sealed plan of examples/echo.rkg, produced by the compiler on
/// the host — the embedded entry point is the blob, never source.
static ECHO_BLOB: &[u8] = include_bytes!("../echo.blob");

struct Ticks {
    syst: cortex_m::peripheral::SYST,
}

impl Ticks {
    fn new(mut syst: cortex_m::peripheral::SYST) -> Self {
        syst.set_clock_source(SystClkSource::Core);
        syst.set_reload(0x00FF_FFFF);
        syst.clear_current();
        syst.enable_counter();
        Self { syst }
    }

    /// Down-counting tick delta across `f`, single-wrap tolerant.
    fn measure<R>(&mut self, f: impl FnOnce() -> R) -> (u64, R) {
        self.syst.clear_current();
        let start = cortex_m::peripheral::SYST::get_current();
        let out = f();
        let end = cortex_m::peripheral::SYST::get_current();
        let delta = if end <= start {
            u64::from(start - end)
        } else {
            u64::from(start) + u64::from(0x00FF_FFFF - end)
        };
        (delta, out)
    }
}

/// A busy loop with a known instruction count (2 per iteration:
/// subs + bne), used to calibrate ticks → instructions.
fn spin(mut n: u32) {
    while n > 0 {
        n = core::hint::black_box(n - 1);
    }
}

fn submission(text: &str, marker: u8) -> InboundSubmission {
    let mut app = Namespace::new();
    let _ = marker;
    let _ = &mut app;
    InboundSubmission {
        authority: IngressAuthority::single(Path::new(["pico"]).unwrap()),
        draft: Draft {
            data: Data::materialized(Value::Scalar(Scalar::Text(String::from(text)))),
            mime_type: None,
            app,
        },
        sequence: SubmissionSequence::Next,
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        HEAP.init(&raw mut HEAP_MEM as usize, HEAP_SIZE);
    }
    let peripherals = cortex_m::Peripherals::take().unwrap();
    let mut ticks = Ticks::new(peripherals.SYST);

    // Calibration: 1M iterations of a 2-instruction loop.
    let (calib_ticks, ()) = ticks.measure(|| spin(1_000_000));
    hprintln!("calib: {} ticks for ~2000000 instructions", calib_ticks);

    // 1. Open the sealed blob.
    let (open_ticks, opened) = ticks.measure(|| blob::open(ECHO_BLOB));
    let (plan, hash) = opened.expect("the blob opens");
    hprintln!("blob_open: {} ticks", open_ticks);
    hprintln!("semantic_hash: {}", hash);
    hprintln!("heap_after_open: {} bytes", HEAP.used());

    // 2. Boot and run: each round injects one envelope and steps the
    //    relay transaction; the delivery to the connector then opens
    //    its journaled call (round 1) or queues behind the claim.
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    let mut peak = HEAP.used();
    let mut tx_ticks_total: u64 = 0;
    let mut tx_min = u64::MAX;
    let mut tx_max = 0u64;
    const ROUNDS: u32 = 32;
    for round in 0..ROUNDS {
        vm.inject(&mut journal, submission("salut", round as u8))
            .expect("the submission lands");
        let (t, stepped) = ticks.measure(|| vm.step(&mut journal));
        assert!(stepped.expect("the relay fires"), "work was ready");
        tx_ticks_total += t;
        tx_min = tx_min.min(t);
        tx_max = tx_max.max(t);
        peak = peak.max(HEAP.used());
    }
    hprintln!(
        "relay_tx over {} rounds: avg {} / min {} / max {} ticks",
        ROUNDS,
        tx_ticks_total / u64::from(ROUNDS),
        tx_min,
        tx_max
    );

    // 3. blake3 id derivation on a 64-byte frame, 1000 rounds.
    let frame = [0x5Au8; 64];
    let (id_ticks, last) = ticks.measure(|| {
        let mut last = Id::from_bytes([0; 16]);
        for _ in 0..1000 {
            last = Id::derive(0x02, &[&frame]);
        }
        last
    });
    hprintln!(
        "blake3_id x1000: {} ticks total (last: {})",
        id_ticks,
        last
    );
    let _ = journal::ids::delivery_ref("bench.out>echo.in", last);

    hprintln!("heap_peak: {} bytes of {}", peak, HEAP_SIZE);
    hprintln!("records: {}", journal.records().len());
    hprintln!("SPIKE_OK");
    debug::exit(debug::EXIT_SUCCESS);
    loop {}
}
