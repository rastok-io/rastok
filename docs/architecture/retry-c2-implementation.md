# C2 — source and validated-plan implementation

15 September 2026. **In progress; Retry is not executable.** The
[C1 inventory](retry-c1-identity.md) owns the proposed v19/v3 cutover.
The preparatory checkpoints (§1–§5) preserved executable v18/recording v2.
The current `rework/retry` worktree carries the **v19/v3 candidate**, not yet
blessed or qualified for release. §7 distinguishes that cutover from the
earlier green checkpoints. No golden has been re-blessed.

## 1. Exact Session opening authority

Session direction is now a core Contract fact, used by both live execution
and prefix recovery. It is not inferred from `effect`, attribute names or
duplex roles. The internal closed query returns Peer for WebSocketServer,
Local for WebSocketClient and Process service, and no authority for an unknown
Contract or inconsistent roles/exchange. No transported field or public trait
is added.

A peer cannot open a client or resident process. An uncorrelated server write
is consumed without opening a client or guessing the sole live peer. Recovery
uses the same rule for openings and writes and does not re-offer old I/O.
The existing `effect` attribute/transport remains until the single cutover;
removing its Session-direction use is a prerequisite, not that cutover.

Evidence at this checkpoint:

- the core identity/roles/exchange matrix passes (four selected tests);
- twelve `machine_vm` Session tests pass, including new peer-root refusal
  before journaling and orphan-write parity with/without recovery;
- the five focused Contract, gauge, JSONL, resident Process and exchange
  recovery suites pass (101 tests, no blessing enabled);
- tests compare retained records, not only an in-memory resource count.

These tests do not qualify Retry scheduling/accounting.

## 2. Contract-owned frame case sets

`FrameProduction::ExactlyOneOn` now carries a nonempty canonical set of
terminal names instead of one name. The VM checks each frame count against
the selected terminal; Call Loop and Graph Loop proofs use the same set when
combining frame and terminal outcomes. A frame is still externally visible
for the repetition check. The standard Contract tables remain the authority,
not a new serialized production claim.

The ten core Call-profile tests pass. Added tests exercise two response cases,
zero/one/extra frames, stream/never profiles and canonical/declared case sets.
Existing HttpClient and File read still select only `Completed`; Process job
retains its independently bounded streams. In particular, HTTP 500 is **still
Completed in the current Contract**. ServerError, its typed status checks and
two-case cardinality land with the declared Contract-major cutover, not here.

## 3. Preparatory qualification at `451dbb5`

All required workspace checks pass on this batch:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
env -u RASTOK_BLESS cargo test --workspace --all-features --quiet
cargo doc --workspace --no-deps
```

The full run reports **1,398 passed, zero failed, zero ignored** across 47
test groups (including the empty binary/doc groups). Existing semantic hashes,
package fixtures and causal seals pass without re-blessing. Core also checks
on `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf`. Local documentation links/fences and
`git diff --check` pass. Browser/JavaScript and independent protocol campaigns
were not rerun; no UI or physical Driver code changed.

The Cortex-M33 spike was rebuilt from `spikes/rp2350-emu` with
`cargo build --release --locked`, then run using the
[QEMU recipe](spike-rp2350-emulated.md#reproduction). `llvm-size -A` measured
the resulting release ELF:

| Measurement | Result | Compared with pre-C2 baseline |
|---|---|---|
| peak heap | 111,732 B; 19,340 B below 128 KiB | unchanged |
| `.text` + `.rodata` | 882,788 + 69,872 = 952,660 B | +632 B; 95,916 B below 1 MiB |
| Record / Inbound / Transaction / Outbound layouts | 8 / 216 / 224 / 8 B | unchanged |
| relay ticks, average / minimum / maximum | 1,907 / 1,858 / 2,092 | unchanged |
| retained records | 67, `SPIKE_OK` | echo semantic hash unchanged |

This is the existing non-Retry echo workload. It proves neither held-attempt
memory nor new closure-field overhead; those measurements remain cutover/C3
gates. No gate, package fixture or golden was changed.

## 4. Retry source policy gate

The source checker now recognizes candidate Retry policies before applying
the executable v18 sheets. It checks the exact resolved outbound Call profile,
same-owner Loop/Retry conflict, duplicate/missing attributes, positive attempts
and constant timeout, fixed/exponential delay, canonical terminal selection
and plan-constant boolean idempotence. Unit, Graph and earlier Node `val`s use
the existing expression/type/constant evaluator; no invocation state becomes
a policy constant. The delay checker is shared with Loop without changing
Loop's accepted forms, HIR or identity.

Process job and HttpClient omission/false supply no guarantee in this gate;
File read/replace fixes true and rejects false. The historical `effect` word
never substitutes for that boolean. `idempotent` without Retry is also a
candidate surface, not a v18 effect alias. Session, Event, inbound Call, Graph
and Operator hosts fail by name. The structured Extension conflict carries
the complete owner path and canonical Loop/Retry names.

Core's Contract-owned Call facts independently check canonical selected cases
and the conservative visibility/guarantee matrix. They neither import a
Driver registry nor create a runtime obligation. Unknown and duplicate cases
cannot manufacture a guarantee. An empty case set selects no recurrence.

**This is not complete candidate HIR/plan validation or Retry execution.**
An otherwise accepted policy still gets `E-CHECK-DEFERRED`; invalid policies
retain their specific refusals. Neither produces HIR/package. Ordinary
attributes, wiring, instance-bound timeout
proofs and the complete target Contract mirrors still require the cutover.
In particular, `ServerError` is recognized only to name its pending HttpClient
major change; it is not inserted into current @2 types or published by its
Driver. Policy order tests are not yet hash-neutrality evidence for Retry.

The eleven focused source tests exercise these refusals through public
compilation; two new core matrices cover every current Call terminal and both
guarantee values, noncanonical sets and unknown cases. Their initial regression
run failed with generic unknown-Extension/attribute diagnostics; the dedicated
policy gate replaces that cascade with named refusals.
The first test attempt was blocked by an unaccepted license for the active
Xcode installation. The already installed Command Line Tools worked using a
per-command `DEVELOPER_DIR=/Library/Developer/CommandLineTools`; no license was
accepted and no global tool selection was changed.

Qualification of this source checkpoint:

- the all-features workspace run reports **1,411 passed, zero failed, zero
  ignored**; the final eleven policy tests were also rerun after tightening
  diagnostic payloads and annotation refusals;
- fmt, all-target/all-feature clippy with warnings denied, rustdoc and all
  three core `no_std` targets pass;
- local documentation link targets and `git diff --check` pass;
- the rebuilt Cortex-M33/QEMU spike reports `SPIKE_OK`, 67 records and the
  unchanged echo semantic hash; peak heap is **111,732 B** (19,340 B margin),
  `.text + .rodata` is **882,780 + 69,872 = 952,652 B** (95,924 B margin);
- compared with §3, heap and record layouts are unchanged and code is 8 B
  smaller. This still does not measure retained Retry batches, which do not
  exist in the executable runtime yet.

No package version, recording version, Contract major, hash pin or causal
golden was changed. Browser/JavaScript and independent protocol campaigns were
not rerun; no UI or physical Driver implementation changed in this checkpoint.

## 5. Retained attempt algebra and shared codecs at `a574895`

Historical checkpoint: [C3 §2](retry-c3-implementation.md#2-corrections-found-by-executable-tests)
corrects the pinned-Call-clock claim below and the later carrier's fake manager
`now()` read. These corrections reuse already inventoried fields, not new keys.

The core now has typed Retry operations and a pure protocol fold, together
with the candidate key-58 retention and causal projections from C1. This is
**not journal carrier admission, spatial recovery or executable Retry**. No
v18 record writes a Retry packet and no v2 recording accepts a new row shape.
The current source gate still refuses an otherwise valid Retry policy before
HIR/package emission. The implementation work continues on `rework/retry`.

The fold binds an opening to the exact delivery, instance generation, original
plan, Contract, admitted envelope and Contract-checked policy. Later Calls
keep the original prepared bytes, reduced settings, partner key and logical
opening time. They use new admission and Call identities; DATA and travelling
metadata are retained, with the previous terminal as parent. It checks the
winning closure and the original arm before any late manager receipt can
advance. An active Closing generation suppresses further admission; a held
batch is consumed without publication when stopped in backoff.

`Held`, `Consumed + Advance`, `Published + Close` and `Consumed + Close(stopped)`
are distinct atomic shapes. Producer emission validation checks the entire
ordered batch and its unique terminal; manager receipt must match the pending
batch exactly. The fold retains only the current pending batch, not one batch
per configured attempt. This does not replace the C3 measurement of raw
capture, typed emissions and retained journal history.

Lossless record encoding moved from hosted `recording/codec.rs` to core
`journal/retention.rs`. The hosted layer still owns file framing, versions,
checksums, source/package attachment and persistence policy. It reuses this
reader instead of maintaining another journal interpretation. Envelope/data
causal projections likewise moved intact to `identity/encoding.rs`; the
golden writer and Retry packet use the same value/spine registry. No new
dependency, clock or I/O enters core.

The byte-neutral move was checked against the **old hosted implementation**:
the canonical retained echo record rows hash to
`e0688e724608dd1d9686e86d09148da3fdc3caaf40eeca32d26c9010af5234fa`
before and after the move. That is a retention regression, not a causal
golden or a recording-file checksum. The declared v19/v3 cutover will change
the Call/Intent row shape; that future change must explain this pin too.

Focused evidence at this point: twelve protocol tests and five codec tests
pass, including malformed packets, exact effect preparation, original
deadlines, whole-batch order and reconstruction of every algebra prefix.
All ten existing/new diagnostic recording tests pass. The prefix test supplies
explicit typed events; it is **not** a full decoded-journal RecoveryFold or
Driver replay proof. Record-carrier/plan checks and runtime scheduling remain
required below.

Travelling metadata uses the existing canonical identity rule, not IEEE
scalar equality: unchanged NaN metadata must remain admissible and signed
zero must not silently change. The new regression initially failed at the
first admission with `AdmissionIdentity`; the fold compares canonical
metadata and leaves the metadata value/evaluator semantics unchanged.

## 6. C2 acceptance checklist

- HTTP ServerError and its declared Contract/type cutover, including decoded
  status boundaries and frame cardinality in Driver, Lab and core validation.
- Carry checked Retry settings and plan-owned idempotence into complete HIR
  and independently validated transport; `W-LOOP-REPEAT` and its
  positive/negative/sibling/canonical-order tests remain required.
- Remove global effect and Intent.effect without duplicating the guarantee.
- Carry all C1 packet, reference, closure-time and recording changes in the
  single cutover, with independent hostile validation.
- Keep named execution refusals for unfinished C3 paths; no check-green
  construct that every runtime must reject.

Runtime attempt publication, shared recurrence/recovery and accounting are
C3. Example/golden/UI migration and final qualification remain C4.

## 7. v19/v3 candidate — not blessed

The candidate now removes `Effect` from core and `Intent`, moves the exact
Contract majors declared in C1, and carries the original plan-constant
idempotence guarantee under Connector key 8. Non-Call profiles have no such
guarantee. The independent validator re-evaluates its closed expression and
checks its agreement with the effective attribute; File cannot override true.

Retry is checked through the normal Node/HIR/lowering pipeline, not a separate
partial source checker. Its owner-ordered `rastok.retry.r1` contribution and
tag-4 transport carry attempts, canonical cases, delay, entry and terminal.
Public compilation still refuses at the named manager execution gate, and a
raw validated Retry plan also refuses before VM mutation. This is intentional
staging until C3, not executable Retry support.

Call closures retain their original logical instant uniformly, including
timeout and abandonment. Openings cannot carry it; missing/backdated clocks,
changed attempt context, and an arm for the wrong next admission refuse.
Recovery reads the retained closure instant. Recording v3 drops the old Intent
effect word and retains these fields; v2 is not an alias. The causal projection
adds only sparse Retry context/facts (58) and arm identity (37), not the
retention clock/deadline (59/56). Manager/producer carrier checks reject empty
packets, torn atomic shapes, mixed operations, hidden application state, and
a Held batch that also claims spatial delivery. Full original-plan/prefix
binding and publication remain C3 obligations.

HttpClient publishes `ServerError` only for a usable decoded 500–599 response.
Both response cases require one out frame; decoding failure stays prior to
status classification. Source fixtures with explicit HTTP terminal unions must
add this case. Loop's advisory `W-LOOP-REPEAT` uses the original guarantee,
including sibling effects; it is not Retry's hard safety rule.

Focused evidence: **355 core tests pass**; eleven source-policy, six raw/HIR
Retry-plan, eight causal-golden and two HTTP conformance tests pass. A further
VM/package/Graph-Call run passes 81 + 11 + 22 + 9 tests; its sole failure is
the deliberately unblessed echo package. The Loop warning names the exact
owner, Again source and repeated Connector in canonical order. The nine
Graph-Call tests also pass with this richer diagnostic evidence.

The latest hosted library run passes **379 tests**, with two expected
unblessed pins: the M0 compiler hash and the v3 retained-row encoding.
Strict workspace clippy, fmt, rustdoc and all three no_std checks pass.
The full binary matrix is still being collected; it is not called green.
A parallel real-gateway probe failed its 30-second listener startup bound
with no child output and must be rerun separately, not have its bound raised.
No test is ignored and no golden has changed. The byte-neutral codec
checkpoint in §5 is not the v3 tuple, which intentionally changes.

Candidate package audit (not a blessing):

| Witness | v18 body | v19 body | Identity |
|---|---:|---:|---|
| echo | 939 B | 964 B | Contract/guarantee movement; candidate hash `878a34b7…` |
| JSONL | 983 B | 1,008 B | same movement; candidate hash `3f1b40ec…` |
| gauge | 249 B | 249 B | body, `0d3ad43d…` semantic hash and causal trace byte-identical |

The package audit runs all five witness scenarios successfully; the existing
causal-golden tests remain green. All container versions/checksums move as
declared. The sparse Retry causal tests independently verify key 58 context
and key 37 arm identity; changed key 59/56 retention time does not change the
causal seal.

The Cortex-M33 spike was rebuilt using a separately generated candidate echo
package (`RASTOK_SPIKE_PACKAGE`); the tracked fixture remains untouched.
Its final measured candidate result is:

| Measurement | Candidate | Remaining gate margin |
|---|---:|---:|
| peak heap | 112,656 B | 18,416 B below 128 KiB |
| `.text` + `.rodata` | 904,316 + 71,220 = 975,536 B | 73,040 B below 1 MiB |
| Record / Inbound / Transaction / Outbound | 8 / 216 / 224 / 8 B | unchanged layouts; owned retention still costs memory |
| relay average / minimum / maximum ticks | 1,913 / 1,862 / 2,095 | 67 records, `SPIKE_OK` |

Heap grew 924 B from §5. This remains the **non-Retry echo workload**, not a
claim about held batches or recovery peaks. C3's Process capture/retention
matrix remains required. No dependency or embedded gate was changed.
