# Relative logical time — format-24 identity inventory

Date: 23–24 October 2026. Baseline: `21eae53` on `realign/node-ontology`.
Status: **format 24 / recording 5 locally qualified; independent Fable replay
pending**.
The baseline below was measured before changing any time carrier. Candidate
values and diagnostic gate results were recorded before Fable's review.
On 24 October, Yoann selected option 1, preserving epoch-aligned tumbling,
and approved the inventoried witness regeneration. The final green gate below
qualifies that corrected implementation and its regenerated witnesses. It does
not authorize integration into `main` or close the independent opening gates.

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
box starts, aligned on the Unix epoch rather than boot or the first admission.
Yoann confirmed this existing rule on 24 October after Fable's review: the
origin is a conversion input, not a new window anchor.

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

## Measured candidate — 23 October, before review and witness approval

The candidate reads format 24 only and recording 5 only. All old artifacts
remain in place during qualification; version-refusal failures against those
artifacts are not treated as functional regressions or silently ignored.

### Packages: version/checksum movement only (V)

The semantic hashes and body sizes are the baseline values above. Independent
decode/re-encode confirms byte-identical bodies for all three source packages.
The spike echo candidate is the same binary as the echo hex witness.

The canonical integer 23 occupies one byte; 24 occupies two. Complete
package sizes therefore grow by exactly one byte: Echo 1,015 → 1,016 B,
JSONL 1,060 → 1,061 B, gauge 310 → 311 B, and spike Echo 1,015 → 1,016 B.
This is container overhead, not growth of any plan body.

| Witness | Format-24 checksum | Reason |
|---|---|---|
| `crates/rastok/tests/golden/echo.rkp` | `ebff76073949bb4180c43dbc9a7185e8628cc97d78af48d52efc8e5231b1e135` | V: version 23 → 24; body 934 B and hash unchanged |
| `crates/rastok/tests/golden/jsonl.rkp` | `8e56640f08894386037d722912f7c0788ceac5fa421c505c077048f0662c96fa` | V: version 23 → 24; body 979 B and hash unchanged |
| `crates/rastok/tests/golden/gauge.rkp` | `b79329986a10fc6c436058c5cd394286df84dd8e661288ef93f542ba23fe2e2d` | V: version 23 → 24; body 230 B and hash unchanged |
| `spikes/rp2350-emu/echo.rkp` | `ebff76073949bb4180c43dbc9a7185e8628cc97d78af48d52efc8e5231b1e135` | V: identical to the source-regenerated echo candidate |

The regenerated Retry qualification package keeps semantic hash
`3cad77eff37f7a658254c3855d92b084a12150a9cb3cd440051db864660f67e9`.
It is a scratch qualification input, not an additional tracked artifact.

### Gallery: no causal witness moves

All 28 scenarios run successfully with the candidate. In each row the old
and new seals are **equal**; the table records that shared value once.
These scenarios neither emit civil timestamps nor identify tumbling boxes,
so changing the civil carrier has no causal effect on them. Relative timeout
and Retry deadlines retain their existing values. No reference needs rewriting
for this group.

Paths below are relative to `crates/rastok/tests/golden/`.

| Witness | Old → new record count | Old = new causal seal |
|---|---:|---|
| `gallery/echo/smoke.roundtrip.trace` | 8 → 8 | `566aed6f75b16361988aae27ed8af674c2f8648cca11118829b96e8c1d65efff` |
| `gallery/echo/smoke.filtered.trace` | 4 → 4 | `7cd9e23c6af5c004268c7f590ea012d47d06b1d64949a333e8220c5db4b6978f` |
| `gallery/echo/smoke.silence_expires.trace` | 8 → 8 | `4a1c315c7132de01c8bf5b23dbf91cb17a88084a6f7db6030377675782eb7d99` |
| `gallery/file-pipeline/files.copy.trace` | 12 → 12 | `50b6357a920f562ed961f967f3fd8bc92b5e91b842e420393244291b24819721` |
| `gallery/file-pipeline/files.empty.trace` | 8 → 8 | `5c59610ead25df512f539866ee94ee9ce0139a1766a445d7442471b5731fe281` |
| `gallery/file-pipeline/files.utf8.trace` | 12 → 12 | `22d12627046efa6bd31def4160edbde5d69e24cce3758fd0afa9602c0a142b92` |
| `gallery/file-pipeline/files.directory.trace` | 6 → 6 | `e993283c1d730ef4d83b98c6a87debb5465b07a571577cde5b0e9088a59902d5` |
| `gallery/two-tenants/ui_tour.two_tenants.trace` | 29 → 29 | `2196320c0b04a809c649fae143c58e061402e5612134ecbf031e14a80378d67e` |
| `gallery/two-tenants/ui_tour.lifetime.trace` | 30 → 30 | `d985869ae3675376e4f9a458634786bd5f2af847edc396cdb0d82e137ecf51e5` |
| `gallery/webhook-to-partner/webhook.nominal.trace` | 18 → 18 | `d6dc13c583c51fe8274e2c49199b2850802d6bb95bc0e4ca69868ab73ee98c52` |
| `gallery/webhook-to-partner/webhook.timeout_then_success.trace` | 23 → 23 | `9b8e06771cdc344c7b2c8d9b586a81d3a1daa496dfe37abf7e203b5195b186fb` |
| `gallery/webhook-to-partner/webhook.partner_dead.trace` | 23 → 23 | `691fcc58e3911ec85cbab67b757d6db7c167293da68ed1face122d3123728d44` |
| `gallery/webhook-to-partner/webhook.duplicate.trace` | 35 → 35 | `818d9cca73bee51787c509bc032f14c81434926c2ca230e3d4a8918f56ac8980` |
| `gallery/webhook-to-partner/webhook.invalid_payload.trace` | 9 → 9 | `079544d241039cde9d6c2bd429f865dd0a96f0b9e0196f671791f6075a18a3fd` |
| `gallery/webhook-to-partner/webhook.http_500.trace` | 23 → 23 | `075f7304e3c4ace008d9a9820a747e62887eb580294f4e51bd29919ff248b6db` |
| `gallery/sensor-alarm/sensors.hysteresis.trace` | 11 → 11 | `3db3072dbbc8013cb3ae7292f674d51e7662337020a95f26554a49fd51fa7eb1` |
| `gallery/sensor-alarm/sensors.silent_sensor.trace` | 8 → 8 | `b273bd2353eeb7b8ad09b2b5a55556ac3f6a5b8d5e77717a36a63d9690658f04` |
| `gallery/sensor-alarm/sensors.virtual_day.trace` | 4 → 4 | `9de2f8f4a2c1ad627873679b88408b9d90f101d99ba146d4e2362cd30ae4af75` |
| `gallery/agent-gateway/scripted_model.alice_and_bob.trace` | 50 → 50 | `e16ea20a47b32fc7e21d22ef5d88d08e37305fb23d86b28772c0550defedec90` |
| `gallery/agent-gateway/scripted_model.fresh_child.trace` | 27 → 27 | `1e53edc0b605cf1b4da893d4373e7b0a0ad2dd580327a55acefb9852c93cc5fb` |
| `gallery/agent-gateway/app_server.two_conversations.trace` | 89 → 89 | `29b0c9ff02226074604d3b86f86f535d81d91d9e5edd1b6c857cecf45fdb0181` |
| `gallery/agent-gateway/app_server.unknown_method.trace` | 12 → 12 | `6f77b66a3598eb93ab531790abf971c3f9ec498cf6f4325af74d01f2a9ade2a5` |
| `gallery/agent-http/responses.two_sessions.trace` | 31 → 31 | `53fde566fb48ea019d3f0b337349482361780d44c0520e3714578b29dd0d4c72` |
| `gallery/agent-http/responses.missing_usage.trace` | 19 → 19 | `cc24772b564cca3294108068d6e7a9d08eb0e93113f88897e783649c6cd80774` |
| `gallery/agent-http/responses.observed_zero.trace` | 12 → 12 | `52832dc9b8508a8dd8676a4f11677bf042aa268058dc0566b8a36dab9ca8c480` |
| `gallery/agent-http/responses.schema_failure.trace` | 10 → 10 | `451fa758b3a2e61ca4566d5be0ead739b4986648d679a602d0130d54680efb9a` |
| `gallery/agent-http/responses.transport_failure.trace` | 10 → 10 | `fd8153806cc896e4c81321bb1ac6b021ac77f816f8552fdf9cee56b045214312` |
| `gallery/agent-http/responses.http_errors.trace` | 18 → 18 | `7b3107f2fe33c4ad15f6e42c7c7912e49ca6839c82a98acd325d732559dfaa33` |

The independent source-package runs also preserve the JSONL and gauge traces:
`jsonl-roundtrip.trace`, 8 records,
`7afbcaaba2f085995ac7262e256058c2f3ed459fce10a2ce8f5dd6a3617dc099`;
`gauge-roundtrip.trace`, 4 records,
`0f1dd432e3e4e5f4b3bc20a2f86e024a67e620db51f8e41575eb7ae3cd2a9528`.
`echo-roundtrip.trace` shares the gallery echo roundtrip above.

### Carrier and recording pins

- **T:** canonical timestamp value `[10, epoch_ns]` becomes
  `[22, [seconds, fraction]]`; timestamp_ntz uses 23 rather than 11.
  Type tags do not change. HIR timestamp constants hash the normalized
  seconds/fraction pair rather than the narrow integer. Source expressions
  that contain no timestamp constants retain their semantic hash.
- **O:** transaction key 39 and its lossless record field carry a bare civil
  pair, not a logical integer. For the epoch origin and logical instant
  17 ns, the old value `17` becomes `[0, 17]`. The existing canonical-form
  assertion in `golden.rs` follows this declared representation.
- **B:** processing-time-box segment kind 2 carries a civil pair.
  Its start remains an integral civil period from the Unix epoch;
  the recorded origin converts that boundary to a relative logical due instant. Instance
  round-trip and recovery tests exercise this without replacing a trace pin.
- **R:** the recording header changes from five entries at version 4 to
  six entries at version 5, adding the normalized origin. The current
  version assertion in `recording::tests::incomplete_corrupt_and_wrong_version_files_refuse`
  and the recording-format assertions in `site_content` must follow this
  version at approval. Neither is permission to accept a preceding reader.
  Checkpoint instants remain relative signed integers.
  Public status headings remain explicit candidates until that approval
  and the final gate; their qualified spellings are not restored prematurely.
  Diagnostic recordings are generated from source by the tests, not kept as
  `.rkrec` witnesses. The independent echo record-tuple digest remains
  `6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d`
  before and after C: the recording header is outside that tuple projection.
- **V, hostile-header pin:** `plan_package::a_non_canonical_envelope_refuses`
  currently pins version 23 as `a4 01 17`. The format-24 head is
  `a4 01 18 18`; the following hash entry moves by one byte. Its reordered-key
  and nonminimal-integer cases must still refuse as `NonCanonical`; the
  nonminimal spelling of 24 is `19 00 18`, not the minimal `18 18`.
  These exact-byte assertions follow the container movement at approval.
- **V, closure-proof version pin:**
  `instance_stop::forged_packages_cannot_omit_the_lexical_scope_proof`
  first verifies that a forged closure scope refuses, then independently
  asserts package version 23. The refusal still passes on the candidate;
  only the final version assertion becomes 24 at approval.
- **V, round-trip version pins:** `plan_fields` and
  `retry_plan::retry_roundtrips_without_any_command_port_or_expired_wrapper`
  also assert version 23 independently of their round-trip properties.
  Those literals become 24; the semantic assertions and execution refusals
  remain unchanged.
- The full diagnostic suite reports nine failing tests against these
  preceding-version pins and qualified-status assertions. Their update
  remains subject to review and approval; no assertion has been weakened.

### Candidate embedded measurement

Same commands, target, allocator and code convention as the baseline, with
scratch-generated format-24 packages supplied through `RASTOK_SPIKE_PACKAGE`
for both builds. Both images finish with `SPIKE_OK`; Echo remains the final
default-feature build. No tracked spike package has been replaced.

| Workload | Code before → candidate | Peak heap before → candidate | Decode heap | Remaining heap / code |
|---|---:|---:|---:|---:|
| Echo, 32 iterations | 971,432 → 973,664 B | 111,404 → 111,684 B | 36,160 B | 19,388 / 74,912 B |
| Retry including recovery | 1,043,488 → 1,046,400 B | 129,188 → 129,268 B | 30,684 B | 1,804 / 2,176 B |

Candidate `.text/.rodata`: Echo 899,468/74,196 B, Retry 961,284/85,116 B.
The transaction record carrier grows from 224 to 232 B on this target;
record/ingress/egress sizes remain 8/216/8 B. Retry live/recovered used heap
is 93,332/93,196 B; both retain the same 129,268 B peak. Record counts stay
67 (Echo) and 17 (Retry). Any subsequent semantic-core change requires
remeasurement before final qualification.

### Diagnostic qualification before approval — 23 October

A and B are closed: `e9492ea` and `21eae53`; their full gates passed.
The independent time value types landed in `10f91f5`. C is not closed.
Formatting, workspace/all-target compilation, strict Clippy, rustdoc with
warnings denied, all three no_std checks (Cortex-M33, Cortex-M3 and RISC-V)
and the 35 browser probes have passed on the candidate. The 385 core unit
tests and nine cross-layer time-carrier tests pass. The latter cover the
Lab's declared/default origins, invalid origin declarations, wide literals,
recording/recovery, tumbling origin conversion, lazy civil-overflow refusal,
origin mismatch before admission, unchanged seals for an unobserved origin,
and civil time in a Call mock.

The complete diagnostic command was
`cargo test --locked --workspace --all-features --no-fail-fast -- --test-threads=1`:
62 groups, 1,633 passed, 11 failed, zero ignored, exit 101. Nine failures are
the inventoried version pins and qualified-status assertions above; two are
physical gateway startup failures described below. This is not a green gate.
The command compiled its test binaries before the final wide-literal and
UI-projection regressions were added: its `time_carriers` group has eight
tests. The subsequent focused run passes all nine time-carrier tests, and
the new UI-projection test passes separately. These two additional passes
are not included in the full-suite tally. Final qualification must rerun
the whole suite after the approved reference update.

The separate documentation gate reports 12 passed and one failure:
`current_public_status_matches_the_package_and_contracts`
first rejects the explicit candidate heading in the whitepaper. Its subsequent
recording-4 assertions also await the inventoried version update above.
The browser's first invocation failed to resolve its
optional external Playwright module; the configured isolated tool path then
ran all 35 probes successfully. No dependency was added to the repository.
The UI projection test also preserves both extreme civil-second values
exactly as decimal strings, with their separate nanosecond fraction.
The current-core diagram (05) and its SVG now distinguish logical deadlines,
the unsealed civil origin and an observed civil timestamp. PlantUML 1.2026.9
regenerated the SVG locally; its raster rendering was visually checked.
Several native test executables stopped in macOS `_dyld_start`, before Rust
code. Only the affected owned commands were restarted; no system setting or
test deadline was changed. Two physical gateway probes initially expired
waiting for the CLI to start. Both groups subsequently passed unchanged:
chat 4/4 and gateway 3/3. Their successful reruns do not replace the two
failures in the full-suite tally. Expected failures against the preceding
format's pins remain pending the approval above.
Seventy-three protected artifacts and both human-review registers remain
byte-identical to the pre-C baseline. No artifact regeneration, main
integration, tag, push or publication has occurred.

### Review and approved correction — 24 October

Fable approved A and B and the C carriers, origin retention, lazy `now()` and
recording shape. The review identified that the candidate's origin-aligned
tumbling boxes changed the existing epoch-alignment rule. Yoann selected
option 1: preserve that rule, then regenerate the four package witnesses and
update the nine inventoried failing pins/status assertions. No causal trace
is authorized to change.

Address selection and lifetime deadlines now derive the same elapsed offset
within an epoch-aligned box from the recorded origin and logical instant.
The calculation reduces the wide civil pair modulo the validated positive
period; it requires neither i128 nor a new runtime cache. Civil box starts
remain checked, while logical deadline addition retains its saturation rule.
The regression first failed on the candidate's box address, then passed with
two different boots in the same civil window, a pre-epoch origin, year 1600,
year 9999 and recovery at the original window deadline. Separate tests cover
the arithmetic bounds and refusal of an unrepresentable civil start before
admission.

The review's small corrections are included: the Retry specification cites
the diagnostic's retirement date rather than a commit hash; the diagnostic
registry explicitly covers repeated scenario origins; the Calendar warning's
normative text remains intact with its M1 implementation status stated
separately; the parser's duplicate restart fallback is removed. The two
human-review registers remain Fable's responsibility.

### Final embedded measurement — 24 October

The epoch-alignment correction and its final comments were rebuilt with the
same release commands, allocator and QEMU target. Retry still has the same
semantic hash and 17 records; Echo has the same semantic hash and 67 records.
Both finish with `SPIKE_OK`. The limits remain 128 KiB heap and 1 MiB code.

| Workload | `.text` | `.rodata` | Code | Decode peak heap | Total peak heap | Remaining heap / code |
|---|---:|---:|---:|---:|---:|---:|
| Echo, 32 iterations | 899,364 B | 74,140 B | 973,504 B | 36,160 B | 111,684 B | 19,388 / 75,072 B |
| Retry including recovery | 961,220 B | 85,060 B | 1,046,280 B | 30,684 B | 129,268 B | 1,804 / 2,296 B |

Relative to the pre-C baseline, Echo uses 2,072 B more code and 280 B more
peak heap; Retry uses 2,792 B more code and 80 B more peak heap. The correction
does not add a per-owner alignment cache. These margins remain a constraint
for later core changes, not permission to increase either limit.

The final default-feature Echo image was rebuilt from the regenerated tracked
package without a package override, then executed successfully. Its code and
heap measurements are the Echo row above.

### Interrupted qualification attempt — 24 October

The approved four packages have been regenerated from source. Their bodies
and semantic hashes are unchanged; each container grows by one byte for V.
The other 69 protected artifacts remain byte-identical, including every
causal trace. Both human-review registers and `main` remain untouched.
All 28 gallery scenarios again match their existing record counts and seals.

Completed successfully on the corrected candidate:

- `cargo fmt --all -- --check`;
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps`;
- the three `cargo check --locked -p rastok-core --no-default-features --target …`
  checks: `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, and
  `riscv32imac-unknown-none-elf`;
- compilation of the full test suite with `--no-run`;
- the ten cross-layer `time_carriers` tests, zero failures or ignored tests;
- the final Echo and Retry QEMU workloads above.

The full workspace command did **not** complete successfully. Its hosted
library group passed all 412 tests, then the main test executable remained
in macOS `_dyld_start` before entering Rust. Restarting only the affected
processes, including a direct terminal invocation, did not unblock the gate;
the next integration executable also stalled at startup. The standalone
`site_content` executable likewise did not start on its final retry. An
earlier run exposed one remaining recording-4 status assertion; it has been
updated to the approved version 5, retaining the old inventory-link assertion
and adding the new inventory link. Its final 13-test run is still unqualified.

The final browser attempt had four UI-server startup timeouts before it was
stopped; it is **not** a 35/35 result. Chrome itself started, but the native
Rastok children did not publish a server URL. The successful browser run in
the diagnostic section predates the approved correction and does not replace
this final gate. Only owned qualification processes were stopped; no system
security setting or test timeout was changed. These startup failures leave
the complete Rust suite, standalone site-content gate and final browser
gate pending at this checkpoint. C had not been closed or committed, and
nothing had been pushed, tagged, published or merged into `main`.

### Final qualification — 24 October

The subsequent complete run of
`cargo test --locked --workspace --all-features --no-fail-fast -- --test-threads=1`
finished with exit 0: **62 groups, 1,649 passed, zero failed, zero ignored**.
This includes the ten cross-layer time-carrier tests, 387 core unit tests and
the negative compile test for adding two logical instants. No test was
filtered out, ignored or given a longer timeout. Read-only `--list` probes
used during startup diagnosis are not included in this result.

| Final check | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Pass |
| `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps` | Pass, no warnings |
| `cargo test --locked -p rastok --all-features --test site_content` | 13 passed, zero failed or ignored |
| Browser suite, existing Playwright and Chrome installation | 35 passed, zero failed, skipped or cancelled |
| Three `no_std` targets listed above | Pass on all three |
| QEMU Echo and Retry, including recovery | Both `SPIKE_OK`; measurements above |
| Source-generated gallery comparison | 28 scenarios, unchanged record counts and causal seals |
| Protected-artifact and review-register audit | Four approved V movements; 69 other artifacts and both review registers unchanged |
| `git diff --check` | Pass |

The final documentation checks also found inventory links lost when the live
status headings were updated. The Connector and lifecycle inventory links
were restored in the example catalogue, opening checklist and recording
document; the existing assertions remain, and the catalogue additionally
checks the time-carrier inventory link. Neither the public narrative nor a
functional assertion was weakened. The earlier startup interruptions and
status-assertion failures above are superseded by this complete green run,
not counted as passing attempts.

The four approved packages are the only changed tracked witnesses. Their
container version and checksum move for reason V; their body bytes, semantic
hashes and all causal traces remain unchanged. Recordings are regenerated by
their tests under version 5, with the required immutable origin and explicit
refusal of the preceding version. The two fixed embedded workloads remain
under the unchanged gates; Retry leaves **1,804 B of heap and 2,296 B of code**.

The implementation is ready for Fable's independent replay on the committed
work branch. Review registers remain Fable's; `main` stays at `fedada7`.
No push, tag, publication or main integration is part of this qualification.
