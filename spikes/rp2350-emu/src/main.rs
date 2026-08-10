//! M0.5 spike, volet A: the Rastok kernel on an emulated Cortex-M33
//! (QEMU mps2-an505). Boots from the sealed plan package, runs the echo
//! graph's transactions, and reports: package-open cost, per-transaction
//! cost, blake3 id cost (all in calibrated instruction counts), and
//! the heap watermark. Timing is a PROXY under `-icount` — the
//! verdict note states the caveats.

#![no_std]
#![no_main]

extern crate alloc;

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
use rastok_core::journal;

/// The allocator, wrapped to record a true high-water mark: the
/// peak is updated inside every successful allocation, so no
/// transient allocation can slip between two samples.
struct WatermarkHeap {
    inner: Heap,
    peak: core::sync::atomic::AtomicUsize,
}

// SAFETY: delegates allocation to the inner heap unchanged; the
// wrapper only observes `used()` after a successful alloc. Single
// core, no interrupts allocating — the critical-section feature of
// the inner heap covers the superloop.
unsafe impl core::alloc::GlobalAlloc for WatermarkHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { core::alloc::GlobalAlloc::alloc(&self.inner, layout) };
        if !ptr.is_null() {
            self.peak
                .fetch_max(self.inner.used(), core::sync::atomic::Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        unsafe { core::alloc::GlobalAlloc::dealloc(&self.inner, ptr, layout) }
    }
}

#[global_allocator]
static HEAP: WatermarkHeap = WatermarkHeap {
    inner: Heap::empty(),
    peak: core::sync::atomic::AtomicUsize::new(0),
};

const HEAP_SIZE: usize = 256 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

/// The sealed plan of examples/echo.rkg, produced by the compiler on
/// the host — the embedded entry point is the package, never source.
static ECHO_PACKAGE: &[u8] = include_bytes!("../echo.rkp");

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

/// The calibration loop: two instructions per iteration — `subs` +
/// `bne` — BY CONSTRUCTION, written in inline assembly so no
/// optimization level can change the count (the first measurement's
/// Rust loop compiled to five instructions via `black_box`, and
/// every conversion ran ≈ 2.5× low). Verified against the
/// disassembly; the verdict note carries the repro command.
fn spin(n: u32) {
    // SAFETY: register-only arithmetic on a scratch copy of `n`;
    // no memory access, no stack use; the condition flags are
    // clobbered, which `asm!` assumes by default.
    unsafe {
        core::arch::asm!(
            "1:",
            "subs {count}, {count}, #1",
            "bne 1b",
            count = inout(reg) n => _,
            options(nomem, nostack),
        );
    }
}

fn submission(text: &str, marker: u8) -> InboundSubmission {
    let mut ext = Namespace::new();
    let _ = marker;
    let _ = &mut ext;
    InboundSubmission {
        authority: IngressAuthority::single(Path::new(["pico"]).unwrap()),
        port: rastok_core::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Scalar(Scalar::Text(String::from(text)))),
            ext,
        },
        sequence: SubmissionSequence::Next,
    }
}

#[entry]
fn main() -> ! {
    // SAFETY: called exactly once, before the first allocation, on a
    // single core; HEAP_MEM is exclusively owned by the allocator
    // from here on.
    unsafe {
        HEAP.inner.init(&raw mut HEAP_MEM as usize, HEAP_SIZE);
    }
    let peripherals = cortex_m::Peripherals::take().unwrap();
    let mut ticks = Ticks::new(peripherals.SYST);

    // Calibration: 1M iterations of the asm-exact 2-instruction loop.
    let (calib_ticks, ()) = ticks.measure(|| spin(1_000_000));
    hprintln!("calib: {} ticks for 2000000 instructions (asm-exact)", calib_ticks);

    // 1. Open the sealed package.
    let (open_ticks, opened) = ticks.measure(|| rastok_core::plan::decode(ECHO_PACKAGE));
    let (plan, hash) = opened.expect("the package opens");
    hprintln!("pkg_decode: {} ticks", open_ticks);
    hprintln!("semantic_hash: {}", hash);
    hprintln!("heap_after_open: {} bytes", HEAP.inner.used());

    // 2. Boot and run: each round injects one envelope and steps the
    //    relay transaction; the delivery to the connector then opens
    //    its journaled call (round 1) or queues behind the claim.
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
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

    hprintln!(
        "heap_watermark: {} bytes of {} (allocator-tracked peak)",
        HEAP.peak.load(core::sync::atomic::Ordering::Relaxed),
        HEAP_SIZE
    );
    hprintln!("records: {}", journal.records().len());
    hprintln!("SPIKE_OK");
    debug::exit(debug::EXIT_SUCCESS);
    loop {}
}
