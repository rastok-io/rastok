# Relative logical time — format-24 identity inventory

Date: 23 October 2026. Baseline: `21eae53` on `realign/node-ontology`.
Status: **implementation in progress; no witness regeneration approved**.
The baseline below was measured before changing any time carrier. Candidate
values and final qualification must be completed before Fable's review and
Yoann's approval. This document does not claim that format 24 is qualified.

## Movement and staging

[D-2026-10-23-2](../spec/decisions.md#d-2026-10-23-2--implement-the-relative-logical-timeline-before-opening-in-format-24)
separates the relative logical line from its immutable civil origin. The
first implementation stage adds independent Rust value types without
changing any existing encoder. The wire-changing candidate must advertise
format 24, never a changed representation under format 23. Diagnostic
recording 5 carries the origin in its integrity-protected header.

The journal already has an immutable unsealed run header. Its new origin
is not a fourth record kind and does not belong in a static plan package.
An observed `now()` seals the resulting civil pair; a pure run does not
seal an otherwise unobserved origin. Tumbling addresses seal their civil
box starts, aligned on the fixed origin rather than the first admission.

The candidate inventory must distinguish these reasons:

- **V**: package version and checksum only; body and semantic hash unchanged.
- **T**: a timestamp value uses tag 22 or 23 and a normalized seconds/fraction
  pair, including its semantic-hash projection when present in the program.
- **O**: a transaction observes civil `now()` under its recorded origin.
- **B**: a processing-time-box segment seals its wide civil start.
- **R**: the recording header carries origin and recording version 5.

No record may be added, removed or reordered merely to accept new pins.
Whether a witness actually moves is established by comparison, not inferred
from the presence of a timestamp in a mock. Old artifacts are regenerated
from source, never converted. Review registers and historical reports remain
unchanged. No push, tag, publication or modification of `main` is authorized.

## Package baseline

The independent source-to-package audit decoded and re-encoded all three
candidates. Their bodies, semantic hashes and bytes matched the tracked
format-23 witnesses. The spike binary equals the hex echo package.

| Package | Body bytes | Semantic hash | Format-23 checksum |
|---|---:|---|---|
| `crates/rastok/tests/golden/echo.rkp` | 934 | `7106f3226fe8df3a74a453377d62e3a11e14e869ed6b2280fe95ccc7c7c94037` | `cc1ef5faabba4fe968047fb16a48c51559fd4d4182ea215c7ff702e33890c342` |
| `crates/rastok/tests/golden/jsonl.rkp` | 979 | `6c58b687269df20e5bd5c468a88bf59eac4c67dda528f77890dc6937bd1dbc81` | `e4521331d5346aa8782ee03dd599e4284f6d0f358a4de149166fe70f9333bbe7` |
| `crates/rastok/tests/golden/gauge.rkp` | 230 | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `1874f13837d62b61a16fa544282ba40acd953a6eee66120e4e5d45665219600a` |
| `spikes/rp2350-emu/echo.rkp` | 934 | `7106f3226fe8df3a74a453377d62e3a11e14e869ed6b2280fe95ccc7c7c94037` | `cc1ef5faabba4fe968047fb16a48c51559fd4d4182ea215c7ff702e33890c342` |

All 28 gallery scenarios passed against their existing record counts and
causal seals before the carrier changes. Only the public Lab fixture token
`lab-only-not-a-credential`, version `fixture-v1`, was supplied; no physical
Driver or model was called. A comparison of the 73 protected artifacts and
the two review registers found no change after A and B.

## Embedded baseline

Rust 1.97.1, repository release profile, QEMU `mps2-an505`, Cortex-M33.
Code is `.text + .rodata`; the 1,984-byte vector table is reported separately
by `llvm-size -A` and unchanged. Both workloads finish with `SPIKE_OK`.

| Workload | `.text` | `.rodata` | Code | Decode peak heap | Total peak heap | Records |
|---|---:|---:|---:|---:|---:|---:|
| Echo, 32 iterations | 897,420 B | 74,012 B | 971,432 B | 36,160 B | 111,404 B | 67 |
| Retry, including recovery | 958,676 B | 84,812 B | 1,043,488 B | 30,684 B | 129,188 B | 17 |

Retry uses one operation, three Calls, 16 frames per channel and four bytes
per frame. Its semantic hash is
`3cad77eff37f7a658254c3855d92b084a12150a9cb3cd440051db864660f67e9`.
The heap gate is 128 KiB and the code gate is 1 MiB: baseline margins are
19,668/77,144 B for Echo and 1,884/5,088 B for Retry (heap/code). A negative
candidate margin stops qualification; neither gate may be relaxed.

Commands, from the repository root, with a fresh scratch directory:

```sh
qualification_dir=$(mktemp -d)
cargo run --locked -p rastok --example retry_spike_package -- "$qualification_dir/retry.rkp" 3
cd spikes/rp2350-emu
RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 -nographic -monitor none -serial none -semihosting-config enable=on,target=native -icount shift=0
RASTOK_SPIKE_PACKAGE="$qualification_dir/retry.rkp" RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release --features retry-qualification
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 -nographic -monitor none -serial none -semihosting-config enable=on,target=native -icount shift=0
RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
```

These are baseline commands with the tracked format-23 echo package. Before
approval, candidate measurements must supply scratch-generated packages for
both workloads; they must not overwrite the tracked spike package.
