# Lifecycle registry — format-23 identity inventory

Date: 23 October 2026. Baseline: `d38ff9f` on `realign/node-ontology`.
Status: **approved by Yoann; regenerated once and locally qualified**.
This inventory was written before regeneration of any tracked witness.
Fable's independent replay remains separate.

## Approved movement

D-2026-10-23-1 makes the word after `l:` name the emitting mechanism.
Every Loop-manager transaction uses `l:loop`, including opening, admission,
advance, closure and expiry; an instance close request uses `l:close`.
Lifecycle bytes 3 and 7 do not move. The trigger text is sealed in
TRANSACTION key 19 and in the Emission identity recipe, so this is an
identity movement despite unchanged execution behavior and record shape.

Package format 23 records the movement once and refuses format 22. The
package body schema, semantic HIR and diagnostic-recording version 4 stay
unchanged. Artifacts are regenerated from source, never converted.
Debugger fact labels become `opened`, `admitted`, `advanced`, `closed`
and `expired`; these presentation names are not lifecycle triggers.

The public-format-1 TODO remains separate. No push, tag or publication is
authorized. The human-review registers remain owned by Yoann and Fable.

## Baseline and permitted witness changes

Reason **T**: the trigger text changes in a causal transaction and, wherever
that transaction emits, in its derived identities. No record is added,
removed or reordered. Reason **V**: only package version and checksum change;
the body and semantic hash stay byte-identical.

These are the only tracked traces permitted to move, below
`crates/rastok/tests/golden/gallery/`:

| Trace | Records | Old causal seal | Reason |
|---|---:|---|---|
| `agent-gateway/app_server.two_conversations.trace` | 89 | `b93a418f0f68e4d85d5627e652e11bd04254d9aca2465385839e86c963aeff23` | T: instance close requests |
| `agent-gateway/scripted_model.alice_and_bob.trace` | 50 | `3ff4aaeaa24d95e7a0a91fd3b1a9f67c258fd0ac612521da7439d585f997011b` | T: instance close requests |
| `two-tenants/ui_tour.lifetime.trace` | 30 | `38ebdd9a3109af18a9ba565de0a5f1a8b0aee7e4313f06fd02572814b618bbdb` | T: lifetime close request |
| `webhook-to-partner/webhook.duplicate.trace` | 35 | `e6eaa9fc680ca6fceb7c719fd379ce674ceb540ffeeb2c83172c13a259cf71e3` | T: instance close request |

The baseline package audit independently decodes and re-encodes each source
candidate. Every body below matched its tracked format-22 witness exactly.
The spike binary is the same package as the hex echo fixture.

| Package | Body bytes | Semantic hash, unchanged | Old package checksum | Reason |
|---|---:|---|---|---|
| `crates/rastok/tests/golden/echo.rkp` | 934 | `7106f3226fe8df3a74a453377d62e3a11e14e869ed6b2280fe95ccc7c7c94037` | `669d3e79dec2097567d15a706a0f5fc5db56f74f909714a200b7d6d10baa7df1` | V |
| `crates/rastok/tests/golden/gauge.rkp` | 230 | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `3f6cb93b512e68642942d9bd84d0feed9f2ee01e78e9fc05c5f83b8c9de08d10` | V |
| `crates/rastok/tests/golden/jsonl.rkp` | 979 | `6c58b687269df20e5bd5c468a88bf59eac4c67dda528f77890dc6937bd1dbc81` | `4b261c5c57ed625c133968be8f34306263ca17147c8cde9261bf6ef4d6309cf1` | V |
| `spikes/rp2350-emu/echo.rkp` | 934 | `7106f3226fe8df3a74a453377d62e3a11e14e869ed6b2280fe95ccc7c7c94037` | `669d3e79dec2097567d15a706a0f5fc5db56f74f909714a200b7d6d10baa7df1` | V |

The generated three-attempt Retry spike package is not tracked. It is
regenerated at a new path using `retry_spike_package`; its semantic hash
remains `3cad77eff37f7a658254c3855d92b084a12150a9cb3cd440051db864660f67e9`.
QEMU prints that semantic hash, not the version-sensitive package checksum.
Thus its printed hash must not change merely because the header changes.

Version pins in `plan_fields.rs`, `retry_plan.rs` and `instance_stop.rs`
follow 22 → 23. `plan_package.rs` pins the corresponding canonical header
byte, `0x16` → `0x17`, and its deliberately non-minimal spelling. These
are reason V, not changes to any hostile-package assertion. Its existing
refusal loop covers every older version, now including 22. No 64-hex
semantic-hash pin is expected to change.

## Embedded baseline

Rust 1.97.1, repository release profile, QEMU `mps2-an505`, Cortex-M33.
Code means `.text + .rodata`. Both workloads finish with `SPIKE_OK`.

| Workload | `.text` | `.rodata` | Code | Decode peak heap | Total peak heap | Records |
|---|---:|---:|---:|---:|---:|---:|
| Echo, 32 iterations | 897,420 B | 74,008 B | 971,428 B | 36,160 B | 111,404 B | 67 |
| Retry, including recovery | 958,676 B | 84,816 B | 1,043,492 B | 30,684 B | 129,188 B | 17 |

Retry uses one operation, three Calls, 16 frames per channel and four bytes
per frame. Heap limits remain 128 KiB and code limits remain 1 MiB. Baseline
margins are 19,668/77,148 B for echo and 1,884/5,084 B for Retry (heap/code).

## Candidate measurements — before regeneration

The source-to-Lab audit passed all 28 gallery scenarios before replacing
any witness. All record counts are unchanged. Exactly the four previously
listed traces change their seal; the other 24 seals match their baseline.
Each changed trace contains one close request and no Loop-manager record.
Comparing the generated candidates record by record finds only the UTF-8
trigger field changed, at zero-based records 73, 36, 12 and 22 respectively.
These close requests emit no envelope, so no envelope id changes in these
four witnesses; their causal seals change because key 19 is sealed.
Loop-trigger identity changes are exercised by the protocol/runtime tests,
not by a new gallery fixture.

| Trace, same paths as above | New causal seal | Reason |
|---|---|---|
| `app_server.two_conversations.trace` | `29b0c9ff02226074604d3b86f86f535d81d91d9e5edd1b6c857cecf45fdb0181` | T |
| `scripted_model.alice_and_bob.trace` | `e16ea20a47b32fc7e21d22ef5d88d08e37305fb23d86b28772c0550defedec90` | T |
| `ui_tour.lifetime.trace` | `d985869ae3675376e4f9a458634786bd5f2af847edc396cdb0d82e137ecf51e5` | T |
| `webhook.duplicate.trace` | `818d9cca73bee51787c509bc032f14c81434926c2ca230e3d4a8918f56ac8980` | T |

All three compiled package candidates independently decode and re-encode.
Their complete bodies and semantic hashes equal the baseline, as do the
echo, JSONL and gauge causal seals. Only version and checksum change:

| Package, same paths as above | New package checksum | Reason |
|---|---|---|
| `tests/golden/echo.rkp` | `cc1ef5faabba4fe968047fb16a48c51559fd4d4182ea215c7ff702e33890c342` | V |
| `tests/golden/gauge.rkp` | `1874f13837d62b61a16fa544282ba40acd953a6eee66120e4e5d45665219600a` | V |
| `tests/golden/jsonl.rkp` | `e4521331d5346aa8782ee03dd599e4284f6d0f358a4de149166fe70f9333bbe7` | V |
| `spikes/rp2350-emu/echo.rkp` | `cc1ef5faabba4fe968047fb16a48c51559fd4d4182ea215c7ff702e33890c342` | V |

The regenerated Retry candidate preserves its baseline semantic hash.
The gallery audit used only the public test token
`lab-only-not-a-credential` with version `fixture-v1`, never a real secret
or external model call.

## Applied qualification — 23 October 2026

The eight listed witnesses were regenerated once from source after the
candidate measurements above. The final audit compares 73 protected
artifacts against `d38ff9f`: exactly these eight change; the other 65 are
byte-identical. The package bodies, semantic hashes, record counts and
record order are unchanged. Previous decisions and both human-review
registers are untouched.

Both Lifecycle variants were renamed through RustRover. The codec
accepts the ten current words and refuses the removed spellings and the
former UI pseudo-triggers. The future Loop-admission label also changes
from its former pseudo-trigger to `admitted`, matching the history labels;
`l:timeout` and the instance phase labels are unchanged.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --workspace --all-features --no-fail-fast -- --test-threads=1` | 61 groups, 1,624 passed, 0 failed, 0 ignored |
| `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps` | Passed, no warning |
| `cargo test --locked -p rastok --test site_content` | 12 passed |
| `cargo check --locked -p rastok-core --no-default-features --target <target>` | Passed for `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` |
| Browser tests, after the all-feature CLI build | 35 passed, 0 failed, 0 skipped |
| Echo and Retry under QEMU | Both `SPIKE_OK`; stdout byte-identical to the baseline |

The additional Rust test checks lifecycle words through the record codec;
the existing wire-registry test now pins their display spelling as well
as all ten bytes and rejection of the remaining 246 bytes. The initial
focused run exposed an incomplete Retry carrier in the new test fixture;
the fixture was corrected, not the decoder. The subsequent focused run
and complete qualification passed. No functional assertion was weakened,
no timeout changed and no test ignored. Some macOS executable launches
waited before entering Rust; the complete gate finished without a retry.

### Embedded result and reproduction

The same Rust 1.97.1 toolchain, release profile and commands were used before
and after. No spike source changes. Code remains `.text + .rodata`, with
the existing 1 MiB code and 128 KiB heap limits:

| Workload | `.text` | `.rodata` | Code | Decode peak heap | Total peak heap | Heap margin | Code margin |
|---|---:|---:|---:|---:|---:|---:|---:|
| Echo, 32 iterations | 897,420 B | 74,012 B | 971,432 B | 36,160 B | 111,404 B | 19,668 B | 77,144 B |
| Retry, including recovery | 958,676 B | 84,812 B | 1,043,488 B | 30,684 B | 129,188 B | 1,884 B | 5,088 B |

`.text` is unchanged. Only `.rodata` moves: +4 B for echo and −4 B for
Retry. The 1,984 B vector table is also unchanged. Full QEMU stdout is
byte-identical, including both semantic hashes, decode ticks, record
layouts, record counts, heap peaks and echo execution timings. Retry's
live/recovered used heap stays 93,252/93,116 B, each with a 129,188 B peak.
No other measured variation occurred.

From the repository root, the Retry package is generated at a fresh path:

```sh
lifecycle_probe_dir="$(mktemp -d)"
cargo run --locked -p rastok --example retry_spike_package -- "$lifecycle_probe_dir/retry.rkp" 3
cd spikes/rp2350-emu
env -u RASTOK_SPIKE_PACKAGE -u RASTOK_RETRY_OPERATIONS -u RASTOK_RETRY_FRAMES -u RASTOK_RETRY_FRAME_BYTES \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0
env RASTOK_SPIKE_PACKAGE="$lifecycle_probe_dir/retry.rkp" \
  RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' \
  cargo build --locked --release --features retry-qualification
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0
```

The spike's Cargo configuration selects `thumbv8m.main-none-eabihf`.
Browser qualification used `node --test crates/rastok/src/ui/browser.test.cjs`
after `cargo build --locked -p rastok --all-features`, with the installed
Playwright and Chrome paths supplied by `RASTOK_PLAYWRIGHT` and
`RASTOK_CHROME`. The qualification runs builds and network tests
sequentially. Local measurement logs are under `/tmp/rastok-lifecycle-v23.JSj1ey/`.

No push, tag or publication was performed. Fable records the independent
replay in the human-review registers.
