//! Optional stack watermark for the single-core qualification image. The
//! linker places all statics, including the heap arena, below `__sheap`.
//! Paint only unused RAM below MSP; later scans count overwritten words
//! from the initial stack top. No allocation or execution policy changes.

use core::sync::atomic::{AtomicUsize, Ordering};

const PATTERN: u32 = 0xA5C3_7E19;
const LIVE_STACK_GUARD_BYTES: usize = 32;
static PAINT_END: AtomicUsize = AtomicUsize::new(0);

extern "C" {
    static __sheap: u32;
    static _ram_start: u32;
    static _stack_start: u32;
}

#[inline(never)]
pub fn paint() {
    cortex_m::interrupt::free(|_| {
        let end = cortex_m::register::msp::read() as usize - LIVE_STACK_GUARD_BYTES;
        let start = &raw const __sheap as usize;
        assert!(start < end && start % 4 == 0 && end % 4 == 0);
        // SAFETY: the linker supplies word-aligned bounds in RAM. All live
        // stack frames are at or above MSP, all statics and the allocator
        // arena below __sheap. Interrupts are masked, only one core runs,
        // and volatile word stores cannot be removed as dead writes.
        unsafe {
            for address in (start..end).step_by(4) {
                (address as *mut u32).write_volatile(PATTERN);
            }
        }
        PAINT_END.store(end, Ordering::Relaxed);
    });
}

#[inline(never)]
fn high_water() -> usize {
    let start = &raw const __sheap as usize;
    let end = PAINT_END.load(Ordering::Relaxed);
    let top = &raw const _stack_start as usize;
    assert!(start < end && end <= top);
    let mut lowest = end;
    // SAFETY: this range was initialized by paint and remains valid RAM.
    // The spike has no concurrent core or interrupt writer. Volatile reads
    // observe stack writes made since painting; the scan's own stack use
    // is included, so it cannot conceal a deeper decoder/runtime peak.
    unsafe {
        for address in (start..end).step_by(4) {
            if (address as *const u32).read_volatile() != PATTERN {
                lowest = address;
                break;
            }
        }
    }
    assert!(lowest > start, "stack use reached the painted RAM floor");
    top - lowest
}

pub fn report(section: &str) {
    cortex_m_semihosting::hprintln!(
        "stack_{}: {} bytes (word watermark, since last paint)",
        section,
        high_water()
    );
    cortex_m_semihosting::hprintln!(
        "ram_static: {} bytes",
        (&raw const __sheap as usize) - (&raw const _ram_start as usize)
    );
}

/// Compare reader depths inside the same image and caller frame. The result
/// includes that frame; only the within-image difference measures one level.
pub fn reader_boundaries() {
    use rastok_core::canonical_cbor::{self, DecodeError, MAX_NESTING};
    let mut bytes = alloc::vec![0x81; MAX_NESTING];
    bytes.push(0);
    let mut measured = [0; 3];
    for (index, depth) in [1, MAX_NESTING - 1, MAX_NESTING].into_iter().enumerate() {
        paint();
        let result = canonical_cbor::decode(&bytes[MAX_NESTING - depth..]);
        measured[index] = high_water();
        if depth < MAX_NESTING {
            assert!(result.is_ok(), "the reader accepts every in-bound level");
        } else {
            assert_eq!(result, Err(DecodeError::NestingLimitExceeded));
        }
    }
    cortex_m_semihosting::hprintln!(
        "stack_reader: level 1 {} / level {} {} / level {} refused {} bytes; per level {} bytes",
        measured[0],
        MAX_NESTING - 1,
        measured[1],
        MAX_NESTING,
        measured[2],
        (measured[1] - measured[0]) / (MAX_NESTING - 2)
    );
}
