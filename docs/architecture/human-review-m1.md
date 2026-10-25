# Human review — M1 general review ledger

> Opened 2026-09-23 on `rework/retry` at `beaeb67`. This is the maintainer's
> line-by-line review of the M1 checkout, guided by Anthropic Claude Fable 5.1.
> Fable briefs each file (what it protects, where to look, known findings);
> Yoann reads it in the IDE and comments; Fable verifies each comment against
> the specification and the code and records the outcome here. Nothing is
> changed in the repository during the review; actions are listed per theme
> and executed afterwards. The Fable → Astra pre-review of 18 September is a
> separate document outside the repository.

## Reading order: from the promise outward

**Current schedule — 5 October amendment.** The opening is **5 November
2026**, with a real freeze on **24 October**. The consolidated T1/T2 brief
supersedes the earlier v20/v21 split below: byte-neutral vocabulary first,
then byte-neutral structure, then one reviewed v20 cutover and one bless.
Lot 1 stops for Fable's review; the cutover stops for Fable and Yoann before
any witness blessing. The older dated paragraphs remain review evidence.
The release-number TODO is recorded with the no-legacy decision below.

**Current implementation — 7 October.** The consolidated T1/T2 cutover is
qualified as package 20 / recording 4: **1,571 tests pass, zero fail or are
ignored**, with the complete gate in the [identity inventory](human-review-v20-identity.md).
The [consolidated action disposition](#consolidated-action-disposition--7-october)
below names landed commits and the M2-only remainders. Current paths are
`instance.rs`, `canonical.rs` and `journal/codec.rs`; the dated reading map
below preserves the paths present when that reading was planned. T2 resumes
at `journal/loop_protocol.rs`. This closes the approved implementation brief,
not M1, the rest of the human review or the external opening gates.

**Reading — 8 October.** `fedada7` was deposited on 7 October and no longer
moves; the review continues on working branches under GitHub flow. T2 is read
to its end (Loop protocol and fold, Retry protocol, carriers, causal projection
and fold, explicit instance closing) and T4 has its first pass. Each finding
below carries its lot: zero-byte corrections before the freeze, v21 after the
opening, or M2. The priorities for the external Rust review of 26–30 October
are in their own section before the parked remarks.

**Reading — 10 October.** Preparing the T4 second pass raised a T1 question
on temporal carriers: dates are bounded to 1678–2262 and the two time planes
meet in `now()` and the hosted boot instant. Yoann settled it as
D-2026-10-10-1; it is recorded as A-T1-12 below, lot v21. The T4 second pass
itself has not started.

**Reading — 11 October.** Astra proposed realigning the plan on the Node
ontology of extensions.md §1 (one Node table, one typed index, one reference
wiring with derived indexes, inspection out of `vm.rs`); Fable verified it
against the code and placed it in M2 as a format movement. Yoann decided the
same day that it is an **opening condition** (D-2026-10-11-1). The section
[Realignment on the Node ontology](#realignment-on-the-node-ontology--decided-2026-10-11)
separates what is decided, what is to qualify and the calendar impact
submitted to Yoann. The T4 second pass started the same evening with block
1, `inject_submission`: A-T4-4 below, two of its four questions settled.
The same evening a [vocabulary ledger](human-review-vocabulary.md) was
drafted at Yoann's request: every public type of both crates by family, the
overlapping terms first, 42 questions for his validation.

**Reading — 12 October.** The vocabulary ledger was validated line by line
with Yoann and closed: every **⟂** is settled, and its
[Outcome](human-review-vocabulary.md#outcome--the-rename-lot-closed-12-october)
section is the rename lot, about forty renames and zero journal bytes. Yoann
then decided to **stop the review here** and start the work it produced: the
realignment of D-2026-10-11-1 (lots 1 and 2), A-T4-4 inside lot 1, and the
rename lot with them, all before the opening. His reason: the rest of the
reading is much easier on code that carries the validated names, for him and
for the external reviewer alike; every later block is read once, on
`TransactionPreparer`, `DriverPoller`, `Obligation` and `EnvelopeDraft`
rather than on the words the ledger retired. The T4 second pass is suspended
after block 1; blocks 2–7 are read on the realigned code. Astra's
cross-check of the three documents found two stale sentences, the ledger
header still saying "nothing here is decided" and the disposition row below
placing lot 1 "after the opening"; both are corrected in this touch.

**Reading — 23 October.** The review resumed on the reorganized tree with
Fable's reading of the `l:` registry, from Yoann's three questions at midday
(what `l` stands for, whether `l:next` serves Loop and Retry alike, how Next
differs from Again). The reading found the trigger of every Loop manager
transaction spelled after one command, a `l:retry` row allocated but never
specified, two stale "tag" numbers in dated texts, and debugger labels
imitating the registry. Yoann decided one rule for the word after `l:` and
ratified the findings as an immediate lot, A-T2-13, with package format 23
because the word is sealed in TRANSACTION key 19 and in the Emission recipe:
[Decided — 23 October midday](human-review-vocabulary.md#decided--23-october-midday-yoann-the-l-registry).
Astra applied it in `0413fdf` the same afternoon, with D-2026-10-23-1 and the
[lifecycle identity inventory](lifecycle-registry-identity.md); Fable replayed
the gate and the byte audit the same day:
[Applied and replayed — 23 October afternoon](human-review-vocabulary.md#applied-and-replayed--23-october-afternoon-astra-0413fdf-fable).
Exactly eight tracked witnesses move: the four traces that embed a close
request, by one record and their seal; the four packages, by header byte and
checksum. Semantic hashes, record counts and record order are unchanged. Two
precision remarks on the new §3.1 cause column are recorded there for a
documentation touch. The human reading continues where the 12 October entry
left it, on a tree whose words are now the specification's: the reorganized
T5 skeleton, then T4 blocks 2, 3, 5, 6 and 7, then the T10 first hour.

**Reading — 23–24 October.** The reading did not resume; the pre-opening
lot Yoann ordered on 23 October took the two days and closed three items
that were "to Yoann": the word `Host` (vocabulary ⟂ 7e), the `retry.md`
left open by A-T2-12, and the timing of A-T1-12, moved from the
post-opening lot to before the opening together with the `LogicalTime(i64)`
carrier of 15 October. Fable briefed Astra in three lots: A, the Worker's
services renamed `worker::Supervision` (zero bytes); B, the Retry contract
gathered in `docs/spec/retry.md` (zero bytes); C, the relative logical
timeline of D-2026-10-10-1 under package format 24 and diagnostic recording
5, inventory first. Astra delivered A as `e9492ea`, B as `21eae53`, the C
value types as `10f91f5` and the C wire change as an uncommitted candidate
with its [inventory](time-carriers-identity.md). Fable reviewed the
candidate reading-only and blocked it on one point: its tumbling boxes
aligned on `origin + k·every`, where the specification since v0.0, and
D-2026-10-10-1's own rejection paragraph, align them on `epoch + k·every`;
the brief had copied the ambiguous formula. Yoann kept the specification's
rule on 24 October; Astra corrected code, test, D-2026-10-23-2 point 3 and
the documents, regenerated the four packages, updated the nine inventoried
pins and committed C as `9fbe8d2`. Fable replayed the whole tree the same
afternoon:
[Pre-opening lot — 23 and 24 October](human-review-vocabulary.md#pre-opening-lot--23-and-24-october-astra-four-commits-fable).
Exactly four tracked witnesses move, the packages, by one version byte and
their checksum; no trace moves. The 13 October item "bless and the missed
derivatives" is closed as stale in the same record: `2259fd4` had replaced
`--bless` by `simulate --compare | --save` at Yoann's decision and `beabf68`
had applied the derivatives. The external brief below is re-anchored on
`9fbe8d2`. The human reading continues where the 12 October entry left it,
once `realign/node-ontology` is integrated into `main` on Yoann's order: the
T5 skeleton, then T4 blocks 2, 3, 5, 6 and 7, then the T10 first hour.

**Reading — 24 October, evening.** Yoann asked whether the compiler was
mostly what remained of the reading. Not only: T3 has never been read (its
files are now `plan.rs`, `node.rs`, the single Node table of the
realignment, and `plan/`, about 15 000 lines before selection), T4 blocks 2,
3, 5, 6 and 7 remain, the T5 skeleton is to be reread on the reorganized
tree, then tiers B and C. He set the compiler's check as tutorials before
the opening rather than reading, T8 the same way; Fable ran twelve broken
variants of the `rastok new` graph through `check` and `simulate` and records
the surface findings under
[Parked remarks](#surface-findings-from-the-compiler-tutorials--24-october-to-yoann).
Reading `Driver`, Yoann also saw `inbound()` return `Ingress` while
`outbound()` returned `Outbound`, set the rule that direction words qualify
a noun, and Astra applied it the same evening as `9844a19`
(`InboundRole`, `OutboundRole`, `inbound_role()`, `outbound_role()`):
[Driver roles](human-review-vocabulary.md#driver-roles--24-october-evening-astra-9844a19-fable).

**Reading — 25 October.** Yoann read `lab.rs` diagonally (T8, first
contact) and asked three questions. Whether `Simulator` should be `Lab`: no.
`hir::Lab` is the DSL block; `Simulator` is one scenario in flight, built
once per scenario by `run_inner`; the name is the specification's
(simulation-semantics §1) and was settled on 12 October (vocabulary ledger
⟂ 11b). The `session` locals and two doc comments (`lab.rs:179`, 676) still
carry the old name: cosmetic, after the opening. Why HTTP in the Lab: the §2
rule "injection at inbound only" needs a Lab twin of every injectable source
Driver, and §2.1 models one source today, HttpServer `ack`;
`HttpServerLabInput` is that twin's world-side slot, one retained proposal
and one terminal, no socket. Whether the simulator should know the mocks at
all: the inversion exists for everything that passes through the Worker
(`Driver`, `InboundRole`, `OutboundRole`, `DriverFactory`, `RuntimeDrivers`)
and leaks through two shared `Rc<RefCell<…>>` back-channels, the mock fault
vector and the HttpServer source slot. Yoann judged `lab.rs` acceptable for
now and parked the second leak as A-T8-1 under
[Parked remarks](#lab-world-face--25-october-yoann-to-the-post-opening-queue).
He also asked where the validated plan lives: `ValidatedExecutionPlan` is
`plan/validation.rs:979`, its constructor `try_new` (line 1021) runs the
validation passes in a fixed order; the raw `ExecutionPlan` and `Node` are
`plan.rs:357` and 603.

Rastok promises that the same validated plan and the same journaled inputs
produce the same execution, identities and trace. The review reads what
carries that promise first (identity, journal, validated plan, VM), then what
feeds it (effect boundary, expressions, compiler), then what consumes it
(Lab, Drivers, CLI/UI). The workspace has ~181 k lines of Rust; the plan
targets ~45 k in selective reading. Tests are never read as such; they are
cited as evidence of an invariant when a brief needs one.

Tiers: **A** required before opening; **B** read, partial coverage accepted;
**C** skimmed, relying on tests.

**Schedule (revised 2026-09-26, opening 15 October 2026).** The human review
continues but does not gate the date. Before the opening Yoann reads what
carries the promise: T2 (journal, until 3 October) then T4 (VM, step and
recovery, 3–10 October). T3, T5 and tiers B/C are read after the opening,
with this ledger public. Actions from the review enter before the 15th only
if they are visible in the gallery or the quickstart, change a public
format, or touch security; otherwise they become M2 issues. The T1 actions
that move bytes and are due before the freeze of 3 October land as **one
format change, v20** (A-T1-1 shared payload bytes, A-T1-1b materialization
by the reading port's type, A-T1-4 `IdKind`, A-T1-2 role removal), with a
single re-blessing of goldens and recordings. A-T1-5 (`instance` module)
was M2; on 2026-09-28 Yoann pulled it entirely before v20, together with
the rename residues, with this clarification: a consistent public release
on 15 October prevails over byte neutrality — a witness that moves is
re-blessed with its reason, not avoided. A-T1-6, A-T1-7 and A-T1-9 also move sealed bytes; they are
deliberately not in v20. Yoann's rule (2026-09-26): exactly one byte
movement before the freeze (v20, the quick T1 actions) and exactly one after
the 15th (**v21**, carrying those three plus whatever T2 and T4 reveal), with
the constraint that v20 fixes nothing that would make v21 more expensive.
v21 is the first public M2 milestone, before the durable journal; alpha
formats may change until 0.2, each with its CHANGELOG note. Written safety
valve: if v20 is not blessed on 3 October, the opening moves to 5 November.

| # | Theme | Tier | Files (reading order) | Lines ≈ |
|---|---|---|---|---:|
| T1 | Identity and canonical form | A | `envelope.rs`, `identity.rs`, `identity/encoding.rs`, `canonical_cbor.rs`, `value.rs`, `datatype.rs` (selective) | 3 500 |
| T2 | The journal, replayable truth | A | `journal.rs` (selective: Record, MemJournal, close-once), `journal/retention.rs`, `timing.rs`, `admission.rs`, `loop_protocol.rs` + `fold.rs`, `retry_protocol.rs` + `fold.rs`, `instance_stop.rs` | 6 000 |
| T3 | The validated plan, independent authority | A | `plan.rs`, `plan/validation.rs` (selective), `serialization.rs` (selective), `composition.rs`, `routing_validation.rs`, `dispatch_validation.rs`, `case_partition.rs`, `productibility.rs`, `graph_loop.rs`, `call_loop.rs`, `retry.rs`, `type_resolution.rs`, `annotation_resolution.rs` | 7 000 |
| T4 | The VM, the deterministic step | A | `runtime.rs`, `runtime/vm.rs` (selective: step, transaction, mailboxes, advance), `vm/timing.rs`, `closing.rs`, `call_completion.rs`, `instance_stop.rs`, `loop_manager.rs`, `graph_loop.rs`, `retry_manager.rs`, `advance.rs`, `recovery.rs` + `recovery/*`, `dispatch.rs` | 8 000 |
| T5 | Effect boundary and Contracts | A | `boundary.rs` (selective), `connector.rs`, `connector/call.rs`, `deployment.rs`, `data_policy.rs`, `accounting.rs`, `observation.rs`, `schema_annotation.rs` | 4 000 |
| T6 | Expressions and types | B | `expr.rs` (selective), `json.rs`, `json/hydration.rs`, `loop_proof.rs`, `trivia.rs` | 3 000 |
| T7 | The host compiler | B | `lexer.rs`, `syntax.rs`, `parser.rs` (selective), `hir.rs`, `check.rs` (selective) + `check/{routing,dispatch,loop_control,retry_control,extensions,annotation_arguments}`, `lower.rs`, `compiler.rs`, `constants.rs`, `diagnostics.rs` | 9 000 |
| T8 | Lab, goldens, conformance, recording | B | `lab.rs`, `lab/process_service.rs`, `golden.rs`, `conformance.rs`, `recording/*` | 5 000 |
| T9 | Physical Drivers | C | `connectors/registry.rs`, `assembly.rs`, `manifest.rs`, `process/{mod,service}.rs`, `file/mod.rs`, `http_client.rs`, `http_server.rs`, `network/{pump,http_client,http_server,websocket,tls}.rs`, `websocket_protocol.rs` | 6 000 |
| T10 | Surfaces: CLI, stdlib, UI, projections | C | `main.rs`, `stdlib.rs` + `stdlib/annotations.rkg` + `starter.rkg`, `contract.rs`, `deployment_config.rs`, `observability.rs`, `render.rs`, `ui/*`, `schema_projection/*`, `openapi.rs` | 5 000 |
| T11 | Outside the workspace | C | `spikes/rp2350-emu/src/*`, `bench/vibe/runner`, `.github/workflows/*`, `tools/ci/*` | 1 500 |

## Protocol per file

Fable briefs the file; Yoann reads it in the IDE and comments; Fable
verifies each comment against the specification and the code. Only two
kinds of outcome are recorded here: a **confirmed defect** and a **decision
with its action**. Questions answered as "conforms" are not recorded; each
theme ends with a one-line list of what was read and found conforming.

Actions are numbered `A-Tn-k`, carry an owner and stay open until the
change lands with its regression.

## Implementation follow-up — 2026-09-27, vocabulary before v20

At Yoann's request, the naming subset below is implemented together before
v20 so the next T2/T4 readings use the agreed words. This dated follow-up
supersedes the later scheduling of these renames in the original entries;
it does not close the remaining design or format work. Yoann requested that
this ledger and the complete rename worktree be committed together, with
his DCO sign-off. The human review was conducted by Yoann with Fable 5.1;
Astra implemented its requested follow-up.

| Review action | Disposition in this change | Still open |
|---|---|---|
| A-T1-4 | **Partial:** `Id::derive` takes a parameter named `id_kind`. | The closed `IdKind` enum belongs to v20; domain bytes and recipes are unchanged here. |
| A-T1-5 | **Partial:** `InstanceAddress/Frame/Ref/AddressError` become `InstanceIdentity/Segment/Generation/IdentityError`; the three designated `owner` fields become `scope`; the identity doc includes the worked scope/key/box example. | The `instance` module and moves of policy, state, live instances, `OwnerPath` and encoding remain M2. |
| A-T1-8 | **Requested code pass done:** `canonical_envelope`, `canonical_data`, `canonical_value`, `canonical_meta_value`, `decode_canonical_value`, and `completion_form`; redundant golden aliases removed and affected code comments clarified. | The separate specification vocabulary pass remains open; this change does not rewrite `docs/spec`. |
| A-T1-11 | **Done:** `check_consistency`, `check_value_type`, `check_value_type_at`, `check_bounds`, `NestingExceeded`, `NodesExceeded`, `MembersExceeded`, and `bound`; public conformance uses `Result<(), ConformanceViolation>`. | None for this action. `accepts`, `MAX_TYPE_*` and `E-TYPE-BUDGET` are preserved. |
| A-T2-1 | **Requested rename set done:** derived Call/Session/Loop/Retry identities use `Id`, declaration references use `Name`, the inbound proposal uses `Handle`, and `EndpointRef` becomes `EdgeEnd`; the designated id functions, fields and locals follow. | No instance-module move is implied. |
| A-T2-2(a) | **Done:** `RECURRENCE_DELAY` keeps byte 5; the unused `SAGA_TIMEOUT` constant is removed, with byte 4 explicitly reserved. | A-T2-2(b), extension-neutral journal facts, remains M2 design work. |
| A-T2-3 | **Done:** `MemJournal` documents its deliberately linear M1 lookups/full scans and the atomic-index path for M2. | No index or storage implementation was added. |

Yoann also confirmed that the pre-public JSON logs have no consumers:
their keys now use `call_id`, `session_id` and `session_write_id`, without
compatibility aliases. A regression exercises inbound/outbound Calls,
Sessions and writes through the existing Lab scenarios and the real JSON
projection. No live workload was rerun for this change.

Primary symbol renames used RustRover's semantic refactoring. References
missed through wildcard imports, async/test bodies and the two out-of-workspace
spikes were completed with targeted edits after Yoann's approval. The
explicitly requested signature/comment changes used the same scoped fallback.

**Evidence.** Each of the five rename groups passed
`cargo check --workspace --all-features`. The full workspace suite then
passed **1,551 tests, zero failures/ignored, 60 result groups**, including
`canonical_parameter_order_is_pinned` and the echo/JSONL/gauge witnesses.
The six tests above the earlier 1,545 baseline were already in HEAD.
`cargo fmt --all -- --check`, workspace all-targets/all-features Clippy with
`-D warnings`, and `cargo doc --workspace --no-deps` passed. After the final
JSON-key alignment, its regression was observed failing on the old keys and
passing on the new ones; all **three observability tests**, fmt, workspace
Clippy and `cargo build --workspace --all-features` passed again. The full
workspace test suite was not rerun after that isolated export change.
RustRover's build check passed for the rename pass; the subsequent JSON-only
build request timed out, while the final Cargo build completed successfully.
The two embedded spikes were not separately rebuilt in this pass.

**Identity boundary.** Package v19 and diagnostic recording v3 remain in
place. No recipe, encoded tag, golden, `.rkp`, `.trace`, fixture or example
was changed or re-blessed. The echo semantic hash remains
`878a34b77a7ac6500b6452b903f633ba39848c47ed0ac387ce1597f78a785699`.
Payload sharing, type-directed raw materialization, role removal and the
other v20/v21 work remain open. This follow-up neither publishes anything
nor declares M1 or the human review complete.

## T1 — Identity and canonical form

### `envelope.rs` — read 2026-09-23

**A-T1-1 — Share payload bytes instead of copying them.** Confirmed:
`Data::Encoded` owns `bytes: Vec<u8>`, `Envelope` derives `Clone`, and the
runtime clones the envelope per fan-out route (`vm.rs:6698`), into the
journal, on every Retry/Loop re-admission (`retry_manager.rs:528`,
`loop_manager.rs:832`) and on every handler read (`runtime.rs:2400`).
Decision: fix before opening. Share the bytes (`Arc<[u8]>` or equivalent,
alloc-only) and consider `Arc<Value>` for materialized values; equality and
identity are unchanged since both derive from the bytes.
`InstanceIdentity` already uses this pattern (`identity.rs:123`).
Spec: envelope.md §5.1 states byte sharing as an implementation obligation.
Owner: Astra. The lazy-decode cache (`OnceCell`) is validated as is.

**A-T1-1b — Materialize by the reading port's declared type, not by
content.** Confirmed defect: `runtime.rs:2403-2411` materializes a `raw`
payload as `Text` when the bytes are valid UTF-8, else `Bytes`; `size()`
counts characters on text and bytes on bytes (`expr.rs:2708-2713`). In the
public gallery, `examples/file-pipeline.rkg:29-34` declares `input in: bytes`
and emits `bytes_read = size(data)`: the ASCII Lab payload hides it, a file
containing `é\n` (3 bytes) would report 2. Fix: a `bytes` port always yields
`Value::Bytes`; a `text` port yields `Value::Text` and refuses invalid UTF-8
as a typed error. Regression with a non-ASCII payload. Spec: envelope.md
§5.1 rule 2. Owner: Astra.

**A-T1-2 — Remove `meta.sys.role`.** `Role::Compensate` is never produced,
only decoded (`retention.rs:606`); envelope.md §1.2 now makes Saga context
an instance context. Decision: remove. Astra proposes the depth: drop the
dead variant and keep the wire slot reserved (no format movement), or drop
the field from `meta.sys`, the spine (`identity/encoding.rs:30`) and the
recording-v3 retention tuple (`retention.rs:228`), with envelope.md §1.1
amended.

**A-T1-4 — `domain_tag` becomes `id_kind`, a closed enum.** Id derivation
bytes are bare `u8` constants spread over four files (`journal.rs:60-75`,
`loop_protocol.rs:280`, `retry_protocol.rs:259`, `instance_stop.rs:16`);
only discipline prevents a duplicate. Decision: `#[repr(u8)] enum IdKind`
owned by `envelope.rs`, one variant per envelope.md §2 registry row,
`Id::derive(id_kind, fields)`; `derive` stays (the spec's word). No byte
moves. Spec: envelope.md §2 renames `domain_tag` to `id_kind`. Owner: Astra.

### `identity.rs` — read 2026-09-24

**A-T1-5 — Create the `instance` module; retire `identity.rs`.** The
purpose is only to identify one instance uniquely and carry its policy and
state; today the concept is spread over five files with no map
(`plan::InstanceSpec`, `identity::InstanceAddress/Ref/Frame`,
`runtime::InstanceState`, the private `vm::Instance`,
`journal::instance_stop`), "address" suggests a location and "frame" a call
stack. Decision: one module `crates/rastok-core/src/instance.rs`, on the
model of `envelope.rs`, types named by role inside it:
`instance::Identity` (was `InstanceIdentity`; not `Id`, which already names
the 128-bit envelope id), `instance::Segment` (was `InstanceSegment`, variants
`SelectedKey`/`ProcessingTimeBox` kept), `instance::Generation` (was
`InstanceGeneration`), `instance::IdentityError`, `instance::Spec`, `Lifetime`,
`Closing` (from `plan.rs`), `instance::State` (from `runtime.rs`), and
`instance::Live` (the VM's private record, with `Phase`) once `Delivery`
(`runtime.rs:200`, pure data) moves below it; otherwise `Live` stays in
the VM. The scope field is named `scope` ("the Node or Graph that declared
`instance = { … }`"). `identity::encoding` (the tagged-value registry) is
not about instances and moves next to `canonical_cbor.rs`;
`OwnerPath` moves to `node.rs` beside `node::Path` so `instance` never
imports `plan`. Each type opens with a three-line worked example. No byte
moves; spec: instances.md adopts "segment" for "frame". Owner: Astra;
Yoann validates the final words.

### `identity/encoding.rs` — read 2026-09-25

**A-T1-6 — One canonical form: ccbor for metadata and graph-born values;
free payloads stay bytes plus a named decoder.** Decision (supersedes the
Avro storage plan of envelope.md §5 and §5.1 rule 3): `meta.sys`, `app` and
`ext` are persisted in the canonical CBOR profile of D-2026-08-03-1 ("ccbor"),
the same bytes that are sealed, so integrity is a hash of the stored bytes.
`data` stays `(encoding, bytes)`: the encoding tag names the decoder
(`raw`, `ccbor`, and later `json`, `arrow-ipc`, …), bytes are sealed as
received and never re-encoded. A value born in the graph has ccbor as its
byte form under a distinct `Encoding::Ccbor`; `raw` means external bytes
only. This makes `canonical_data` injective at the `Data` level (today a
materialized value seals under the `raw` tag, `identity/encoding.rs:71-90`).
Avro is demoted to a schema projection for export (`schema_projection/avro.rs`
already exists); Arrow IPC keeps its role for tables. Reasons recorded:
`Value ↔ canonical bytes` is a bijection (deterministic, injective,
`decode_canonical_value` inverse, non-canonical bytes refused by re-encoding),
which no schema-dependent or whitespace-tolerant codec offers; the kernel
already writes it in `no_std`; recording v3 already persists this way.
Spec: envelope.md §5/§5.1 amended; identity.md §3 unchanged. Code:
`Encoding::Ccbor` is a sealed-tag movement for witnesses carrying graph-born
values, to be scheduled in the grouped format cutover with A-T1-2 (b), not
alone. Owner: Astra.

**A-T1-7 — Decoders are build facts.** The identity of a free payload is its
bytes; its *meaning* is `decoder(bytes)`, a function only once the decoder
is pinned by id and version, and neither injective nor surjective in general.
A decoder change (a JSON parser, an Arrow/SQL engine under `sql { }`) changes
materialized values, hence transactions, hence every downstream id.
Decision: the build attests a **decoder registry** — an extensible decoder
id plus its version — next to the plan hash and the format version; replay
and golden comparison against a different registry refuse by name (the
reserved `E-REPLAY-VERSION-MISMATCH` or a sibling code), never diverge
silently; a library upgrade that moves a golden is re-blessed with its
semantic reason. ccbor itself is an entry of that registry as the decoder
named by `Encoding::Ccbor`, but its version is not free: every other entry,
and the registry's own attestation, are sealed through the canonical
profile, so a change to ccbor is a format-version change (as for blake3,
envelope.md §2.0) and the `ccbor` entry's version is the format version.
Refinements of 2026-09-25: (a) decoder families are named by a
vendor-qualified text id in the exact Contract-id grammar
(`rastok/raw@1`, `rastok/ccbor@1`, `acme/exif@1`), so third parties add
decoders without a central allocator; the sealed data form carries a
**binary decoder id**, proposed as the ordinary 16-byte derivation
`Id::derive(IdKind::Decoder, [text id])` — fixed width, domain-separated,
injective over the text, no registry lookup needed to mint it — with the
text retained in the registry (definition to confirm by Yoann; a
build-local table index was rejected as fragile across builds). (b) Decided:
the attested version is a **behaviour hash**, a conformance fingerprint =
blake3 over the corpus hash and the canonical encoding of the decoder's
outputs and refusals on a fixed, versioned corpus, computed at build; the
exact implementation (crate, version) is attested beside it for provenance.
Replay and golden comparison refuse on any difference; accepting a moved
fingerprint is an explicit re-bless. The fingerprint proves equivalence on
the corpus only, which the spec must say. Proposed home: the manifest row
of each `graph_version` (identity.md §2 already retains "the schemas
needed to decode any historical zone" there); confirmed with M2. Spec: envelope.md §5.1 (today's "this build's materialization
rule" comment on `canonical_data` becomes the rule). Owner: Astra for the spec
text; implementation with M2.

**A-T1-8 — Retire the word "spine".** It carries three meanings at once:
the five reserved `meta.sys` fields (envelope.md §1.1 "Tier 1 — the spine",
`envelope.rs:250-414`, `check.rs:6740`), the canonical CBOR form of a whole
envelope including data and zones (`canonical_envelope`, `canonical_data`,
identity.md §3 "the envelope spine, keys 1–9"), and the common part of every
causal record and Loop fact (identity.md §3 "shared spine + one extension
table per kind", `state_spine`, `admitted_spine`, `completion_spine` in
`golden.rs`). Decision (words validated by Yoann 2026-09-25): one plain word per
meaning, spec and code together, no byte moves. The reserved fields are
"the `sys` fields" (envelope.md §5 already calls `meta.sys` "the header");
the canonical CBOR form is "the canonical form" — `canonical_envelope`,
`canonical_data`, `canonical_value` once `identity::encoding` sits next to
`canonical_cbor.rs` (A-T1-5), and `golden.rs:51` already aliases
`data_spine as data_form`; the record layout is "common keys plus one
extension table per kind", Loop fact forms `state_form`, `admitted_form`,
`completion_form`. Surface: 35 code sites, 14 spec sites in identity.md,
3 in envelope.md, a dozen elsewhere. Owner: Astra; Yoann validates the words.

**A-T1-9 — Contract and decoder ids live in the `std` namespace; third-party
namespaces are dotted.** A Contract id denotes a semantics ("the Process
call, major 4"), not a publisher; carrying the product name in every sealed
id ties technical identity to the trademark that TRADEMARKS.md protects, and
a compatible fork would have to carry that name in every package hash.
Decision (2026-09-26): the standard library mints `std/<name>@<major>`
(`std/process@4`, `std/http-client@3`, `std/measure@3`, `std/ccbor@1`,
`std/raw@1`, …); the bare namespace `std` is reserved to it and refused
elsewhere; every other namespace has at least one dot and is expected to
be a domain the publisher controls in reverse order (`com.acme/exif@1`),
never resolved, only compared. Precedent: `System.*`, `java.*`, Rust
`std`/`core`. Rust itself has no reverse-DNS convention (crates.io is a
flat registry), so the rule is stated by Rastok's own grammar, one check in
`ContractId::parse`. Cost: seven constants in `connector.rs:19-36`, 167
occurrences in 50 files, and every witness moves because ids enter the plan
hash and goldens — scheduled in the grouped format cutover with A-T1-2 (b),
A-T1-6 and A-T1-7, never alone. Spec: identity.md gains the namespace rule;
stdlib.md and the ledgers update their spellings. Owner: Astra.
Refinement (2026-09-26): no `core` namespace for `no_std` decoders. The
portability tier of an implementation (kernel = `rastok-core`, `no_std` +
alloc; hosted = needs `std`) is not part of the id: the same `std/ccbor@1`
must seal identically on the host and on the embedded kernel, which is
exactly what the QEMU spike proves, and mirroring Rust's `core`/`std`
re-exports would create aliases, which `ContractId` forbids. The tier is a
column of the registry row and a build-profile fact: a plan that needs a
decoder absent from this assembly refuses by name at binding, the mechanism
already used for Contracts (`contract::Availability`,
`E-CONNECTOR-UNAVAILABLE`; a sibling `E-DECODER-UNAVAILABLE`). Decoders
declared "kernel" live in `rastok-core` and pass the `no-std-frontier` CI
job; today that covers `raw`, `ccbor` and the house JSON reader.

### `canonical_cbor.rs` — read 2026-09-26

**A-T1-10 — Self-contained module documentation for the canonical form,
and one convention for citing the specification from code.** The module is
validated as read: a strict reader (minimal heads, strictly increasing map
keys, one NaN, bounded nesting) whose success means the bytes were
canonical, so `Value ↔ bytes` is a bijection without re-encoding. What is
missing is the story a newcomer needs before the code: the head doc of
`canonical_cbor.rs` (and `identity/encoding.rs`) must say, standing alone,
what CBOR is (a binary item model with a head byte and a shortest-form
argument), why Rastok uses it (a standard, schema-free, auditable substrate
for everything that is hashed), and why the codec is hand-written — the
bijection is the stability guarantee behind every id, seal, plan hash and
golden, and a third-party encoder's legal choices can change under a
version bump, each change a moved seal (D-2026-08-03-1) — keeping the
references to the specification. Citation convention, since three forms
coexist today (14 absolute GitHub URLs in rustdoc reference links, 16
relative paths, 59 bare "`identity.md` §3" mentions): a comment carries the
short bare form inline (`identity.md §3`) so it reads without a browser,
and the module's rustdoc ends with reference-style links to the confirmed
public URL (`https://github.com/rastok-io/rastok/blob/main/docs/spec/…`,
to be switched to `docs.rastok.com` once it resolves), never a relative
path: relative links resolve in the IDE's source tree but break in rustdoc,
docs.rs and GitHub's source view, where only absolute URLs survive. Applies
to every comment in the workspace. Owner: Astra; checked by a small lint
in CI or a test that refuses relative `.md` links in doc comments.
Priority: not urgent, after the opening.

### `value.rs` and `datatype.rs` — read 2026-09-26

**A-T1-11 — Name the three checks by their question.** `Value::validate`
and `DataType::violation` sound alike but ask different questions, and
`violation` returns `Option` with `None` meaning success, the inverse of the
`Result` idiom. They are layered, not overlapping: `violation` first calls
`validate` (`datatype.rs:1220-1231`), then walks the type. Decision
(2026-09-26, words settled): `Value::validate()` becomes
`Value::check_consistency()` (the value respects its own invariants: no
duplicate struct field, admissible and unique map keys, decimal128 budget,
non-empty discriminants); `DataType::violation(name, &v)` becomes
`DataType::check_value_type(name, &v) -> Result<(), ConformanceViolation>`
(the value is of this type), `violation_at` → `check_value_type_at`;
`DataType::validate()` (`datatype.rs:1150`, the global structural bounds
of a type tree: nesting 128, nodes 4 096, members 256) becomes
`DataType::check_bounds()`, and its errors follow: `TypeError::NestingBudget`
/ `NodeBudget` / `MemberBudget` become `NestingExceeded` / `NodesExceeded` /
`MembersExceeded`, field `budget` → `bound` (`datatype.rs:2061-2085`);
the constants `MAX_TYPE_*` and the published diagnostic code
`E-TYPE-BUDGET` stay (diagnostics.md rule 1: a published code is never
renamed), the diagnostic's prose may say "bound". Retiring "budget" from
the type vocabulary also keeps the word free for the future `Budget`
Extension (D-2026-09-18-2), which governs money and admission, not type
shapes. `accepts` unchanged as the boolean form. The verb is `check`, not `validate`, to keep "validation" for the
plan (`ValidatedExecutionPlan`). About twenty callers in the kernel and two
in the host change with the return type; no byte moves. Owner: Astra.
Not urgent, after the opening.

T1 close-out (2026-09-26): eleven actions, all owned by Astra, none
blocking the opening by itself. Scheduling per the revised calendar: v20
before 3 October = A-T1-1, A-T1-1b, A-T1-2, A-T1-4 (one re-blessing);
v21 after the opening, first public M2 milestone = A-T1-6, A-T1-7,
A-T1-9 plus what T2/T4 reveal; M2 later = A-T1-5; not urgent = A-T1-8, A-T1-10, A-T1-11. Read and found
conforming: three metadata zones (D-2026-08-10-1); `canonical_cbor.rs`
(strict reader, bijection without re-encoding); `identity/encoding.rs`
tag registry against identity.md §3; `value.rs` equality and consistency
layering; `datatype.rs` §1 bounds and the `DataType` enum — its composite
constructors, `TypeOrderKey` and the key-admission rules were not read in
this pass;
`Value` versus `DataType` is one type system with a checked runtime
conformance relation, not a second type system.

### Temporal carriers and the logical timeline — decided 2026-10-10

**A-T1-12 — Widen the instant carriers and make the logical timeline
explicitly relative (v21, bytes move).** `Value::Timestamp` and
`Value::TimestampNtz` are signed i64 nanoseconds (`value.rs:33-41`; tags 10
and 11 in identity.md §3), so every date is bounded to 1678–2262 whatever the
port's unit, because the unit is normalized away before the value is carried
(types.md, "V1 value bound"). Business sentinels such as 9999-12-31, pre-1678
history and in-range Arrow or Postgres instants refuse at the door; the
refusal itself is correct and checked (`datatype.rs:1906-1913`). At the same
time the hosted runner boots the VM at the civil instant (`main.rs:389-391`)
and `now()` exposes the raw logical instant as a `timestamp`
(`runtime.rs:736-738`), fusing the two planes of execution-model §5 that the
Lab keeps apart: it boots at 0, and the scenario `origin` is parsed
(`parser.rs:720`) but not applied. Decision D-2026-10-10-1: a relative i64
timeline from `t₀ = 0`, a civil `origin` as an unsealed retained boot input,
`now() = origin + instant`, tumbling boxes on `origin + k·every`, and
`timestamp`/`timestamp_ntz` carried as `[seconds, nanoseconds]` under the new
tags 22 and 23, tags 10 and 11 retired. Scope: `value.rs`, `canonical.rs`,
`datatype.rs` conformance, `expr.rs` temporal parsing and arithmetic,
`instance.rs` box segment, the `now` binding in `runtime.rs`, the hosted
time source, Lab `origin`, the recording header; identity.md §3, types.md,
execution-model §5, simulation-semantics §5.3 and the `Timer` payload in
stdlib.md. Every envelope carrying an instant and every box address
re-blesses with its reason. Owner: Astra. Lot: v21, with A-T1-6, A-T1-7 and
A-T1-9. Before the opening, zero bytes: README "Status and known limits"
states the V1 bound and the relative timeline, and types.md points to the
decision [landed 10 October by Fable on the review branch].

**Moved before the opening by Yoann on 23 October; applied by Astra and
replayed by Fable on 24 October.** Value types in `10f91f5`
(`rastok_core::time`: `LogicalTime`, signed `Duration`, `Timestamp` as
signed seconds and a `u32` fraction; saturating instant arithmetic, checked
civil arithmetic, no instant + instant), the wire change in `9fbe8d2` under
package format 24 and diagnostic recording 5 (tags 22/23 as
`[seconds, nanoseconds]`, 10/11 refused; the origin in the journal's run
header and in the recording header; TRANSACTION key 39 `observed_time` as a
civil pair; box segment kind 2 as a civil pair; `OriginMismatch` and
`CivilTimeOutOfRange`; `now()` bound lazily so only the path that reads it
refuses; Lab `origin` applied; the hosted time source split into origin and
elapsed). One correction from Fable's review, decided by Yoann on
24 October: tumbling boxes stay epoch-aligned and the origin only converts;
the formula "origin + k·every" above was the ambiguity, settled by
D-2026-10-23-2 point 3. Replay on `9fbe8d2`: four checks, 1 649 tests in
62 groups, `site_content` 13/13, three `no_std` targets, browser
35/35, QEMU echo and Retry `SPIKE_OK` with code margins
75 072 B and 2 296 B and heap margins 19 388 B and
1 804 B; exactly the four packages move, by one version byte and
their checksum, bodies and semantic hashes unchanged; no trace moves.
Record:
[Pre-opening lot — 23 and 24 October](human-review-vocabulary.md#pre-opening-lot--23-and-24-october-astra-four-commits-fable).
Status: applied before the opening, reviewed; no longer in the v21 lot.

## T2 — The journal, replayable truth

### `journal.rs` — read 2026-09-26/27

The id recipes follow A-T1-4 (`IdKind`) when v20 lands; nothing else to
change in `ids`.

**A-T2-1 — Identity newtypes are `*Id`, not `*Ref`.** `Ref` collides with
Rust references and hides that most of these wrap a derived `envelope::Id`.
Decision (2026-09-27), rule: a derived identity of a journal fact is an
`Id`; a reference to a declared thing by its name is a `Name`; a runtime
index that identifies nothing durable is a `Handle`. Proposed application,
Astra to confirm case by case: `CallId`, `InboundCallId`, `SessionId`,
`SessionWriteId` (`boundary.rs`), `LoopRunId` (`journal.rs:646`),
`OperationId` (`retry_protocol.rs:29`), `CommandId`
(`loop_protocol.rs:29`) → `…Id`; the composite `AttemptRef { operation,
ordinal }` → `AttemptId`; `InstanceGeneration` → `instance::Generation` (A-T1-5);
`SecretName`, `PrivacyProfileName`, `PolicyName` (`deployment.rs`),
`CostProfileName` (`accounting.rs`) → `…Name`; `InboundProposalRef(u64)`
(`boundary.rs:1789`) → `InboundProposalHandle`; `EdgeEnd` (`plan.rs:1095`)
to be classified. No byte moves. v21 with the other renames.

**A-T2-2 — Extension-owned deadlines and facts in a core journal.** The timer
kind registry (`journal.rs:251-270`, sealed at causal key 37, kinds 1–9)
names Extensions: `LOOP_DELAY = 5` is in fact the recurrence-manager delay
already reused by Retry (`retry_manager.rs`, `retry_protocol/fold.rs`), and
`SAGA_TIMEOUT = 4` is a reserved, unused byte. Likewise the record carries
one sparse packet per std Extension (keys 55 Loop, 57 instance commands,
58 Retry) with a fold each, so the journal core knows every Extension by
name. Acceptable while every Extension is std, which is M1. Decision
(2026-09-27): (a) v21 — rename `LOOP_DELAY` to `RECURRENCE_DELAY` (byte 5
unchanged) and drop the dead `SAGA_TIMEOUT` constant while keeping byte 4
reserved in the spec; (b) M2 design, before the durable journal or with
the Extension SDK, whichever comes first — extension-neutral journal
facts: an Extension registers an id in the `std/<name>@<major>` grammar
(A-T1-9), its facts travel as one opaque canonical packet under a single
sparse key `[extension_id, bytes]`, its deadlines as one generic
`EXTENSION_DEADLINE` kind whose reference names the extension and its arm,
the core validates correlation and arity, the Extension's own fold
validates content. This moves sealed bytes and is the natural moment for
the byte-4 question. Owner: Astra for (a); design note for (b).

**Naming question, open — "deployment".** Yoann finds the word jarring and
suggests "environment". `deployment.rs` carries typed boot inputs: boot
values, secret references and privacy attestations, and says of itself
"no environment lookup"; "environment" already means the process
environment in the DSL (`env()`, "environment-compatible Connector
setting"), so the rename would give one word two meanings. Public surface:
the `--deployment` CLI flag and `*.deployment.json` profiles (nine public
sites), plus about fifty spec mentions. Alternatives with a single
meaning: "site", "binding", "profile" (already "deployment profile").
Decision (2026-09-27): left as is for now; may be reopened after the opening.

Known and accepted for M1 (pre-review F-05): `MemJournal` lookups are
linear (`find_call`, `find_envelope`, `statuses_of`; `journal.rs:2644-2657`)
and the VM scans the whole journal per step for Retry. Harmless for
in-memory runs of hundreds of records; O(J²) over a long-lived run.
Fix path for M2: indexes kept atomically with the records, as the
lifecycle/session folds already are, and a durable backend indexed by
call id. Not v20, not v21. **Action A-T2-3** (Yoann, 2026-09-27): the
`MemJournal` doc says so — a deliberately simple, linear M1 implementation,
with the M2 fix path — so no reader mistakes it for the intended design.
Comment only; with v20 or any later touch of the file.

**A-T2-4 — The three record kinds are nouns: INGRESS, TRANSACTION, EGRESS.**
Decision (Yoann, 2026-10-02): INGRESS replaces INBOUND and EGRESS replaces
OUTBOUND, because TRANSACTION is a noun and the other two were adjectives.
Rule: the noun names a journal record kind and the pipeline that writes it;
the adjectives `inbound` / `outbound` keep naming the direction of an
exchange or a Connector role. Verified: "ingress" already means admission
into the graph (the doc of `Record::Inbound` says "born at ingress";
`VmError::UnknownIngress`, `ingress_port`, `SessionIngressRecord`,
`InboundCallIngressRecord`), so the word gains no second meaning; "egress"
is unused. No byte moves: no golden, fixture, `.rkp` or `.trace` spells
either word, and the gallery pages use them only as adjectives. Visible
changes: the debugger's History record-type column, about 110 uppercase
mentions over eight specification files, and log keys that name the record
kind (not `call_direction`, which is a direction). Proposed application,
Astra to confirm case by case: `Record::{Inbound, Outbound}` →
`{Ingress, Egress}`; `InboundRecord` → `IngressRecord`; `OutboundRecord` →
`EgressRecord`; `prepare_inbound` / `commit_inbound` / `append_inbound` →
`…_ingress`; `open_outbound` / `close_outbound` / `abandon_outbound` →
`…_egress`; the admission pipeline `InboundSubmission`, `InboundDecision`,
`InboundAdmission`, `InboundAcceptance`, `InboundRejection`,
`InboundProposal`, `InboundProposalHandle`, `InboundEnvelopeProvenance`,
`PreparedInbound` → `Ingress…`. Kept as adjectives: `InboundCall…`,
`InboundSession`, `InboundExchange…`, `InboundConnectorDriver`,
`OutboundConnectorDriver`, `OutboundConnectorBindings`,
`ConnectorRoles::InboundAndOutbound`, `has_inbound` / `has_outbound`,
`outbound_call_semantics`, and "inbound Call" / "outbound Call" in prose.
To classify: `OutboundStatus` (it is the status of one Call fact) and
`UnboundOutbound`. One sentence owed to delivery-semantics §6: EGRESS means
an effect the graph initiated; the response of an inbound Call also leaves
the graph, yet it is retained as the `exchange_exit` of a TRANSACTION
(`journal.rs:703`), not as an EGRESS record. Owner: Astra, with the
RustRover rename refactoring. It is visible in the debugger and the
specification, so it lands before the opening, in the same pass as the
remaining vocabulary residues.

**A-T2-5 — The journal names deadlines, not timers: `Deadline`, `Deadlines`,
`DeadlineKind`, `deadline_ids`, `created_at`.** Decisions (Yoann, 2026-10-05).
(a) The timer-kind registry of `journal.rs:250-268` becomes a closed enum, the
same action as A-T1-4 for the second byte registry: every other sealed tag in
the core is a Rust enum projected to its number at the byte boundary
(`Lifecycle` with `lifecycle_tag`, `serialization.rs:728`; `Record` with kind
1/2/3), and AGENTS.md asks for algebraic types and exhaustive matches. Verified
consequences of the constants: `TimerSchedule::try_new` accepts kinds 1 to 9 as
one range (`journal/timing.rs:101`), so the reserved, unallocated byte 4 passes
retention validation; `recovery.rs` compares raw kinds at 23 sites, where a
forgotten branch stays silent today. (b) The vocabulary follows the
specification, which says "dated obligation" eleven times and "deadline"
sixty-four times against four "timer schedule": the journal records what is
owed and when; "timer" and "scheduler" stay on the VM side, which is a
disposable projection. Proposed application, Astra, in the vocabulary pass
that already carries `IdKind`:

| Today | Target | Meaning |
|---|---|---|
| `timer_kind` constants | `enum DeadlineKind`, nine variants, `TryFrom<u8>` refusing byte 4 and any unknown byte by name, explicit `deadline_kind_tag` for the projection, never the variant order; no `Unknown(u8)` variant | the closed registry |
| `TimerArm { kind, reference, due_at }` | `Deadline` | one dated obligation: kind, exact obligation id, due instant |
| `TimerSchedule { armed_at, arms }` | `Deadlines { created_at, items }` | the deadlines one commit creates together; one creation instant, order kept for equal-date ties |
| `timers: Vec<(u8, Id)>`, sealed key 37 | `deadline_ids: Vec<(DeadlineKind, Id)>` | the identities of the deadlines this record creates; `(kind, id)` identifies a deadline because one subject has at most one per kind |
| `timer_schedule`, retained key 56 | `deadlines` | the same deadlines with their instants |
| `armed_at` | `created_at` | the logical commit instant, documented as logical nanoseconds |

Reading sentence: a record seals its deadline ids and retains its deadlines;
`require_timers` becomes "every sealed deadline id has its retained deadline".
`identified_deadlines` was considered for key 37 and set aside for the
existing `*_ids` convention (`envelope_ids`, `reply_ids`, `input_ids`).
No byte moves: the numbers, the `identity.md` registry labels (not sealed)
and every golden stay. About 250 mechanical sites in the core, the debugger
explanation projection and the tests.

**A-T2-6 — One layer per file in the journal's timing and admission code.**
Confirmed defects (Yoann, 2026-10-05), from reading `journal/timing.rs` and
`journal/admission.rs`:

1. `timing.rs` mixes three layers and borrows the codec's error for all of
   them. It is the only journal submodule without its own `Error`
   (`instance_stop`, `retention`, `loop_protocol/fold`, `retry_protocol/fold`
   each define one): 24 refusals are `canonical_cbor::DecodeError::Malformed`,
   of which about fifteen are not about bytes — the constructor `try_new`
   (`:94-110`) and the cross-record validators `validate_call`,
   `validate_transaction`, `require_timers` (`:18-66`, `:176-290`). The VM
   producers `loop_manager.rs:728`, `retry_manager.rs:692` and
   `vm/timing.rs:293` therefore receive a "decode" error for a value they
   just computed, and `Malformed(&'static str)` erases the distinction into a
   string, which AGENTS.md forbids. Target: a typed `timing::Error`
   (`EmptySchedule`, `DeadlinePrecedesCreation`, `MissingClosureInstant`,
   `ClosurePrecedesOpening`, `ArmWithoutRetryContext`, `ArmMismatch`,
   `SealedDeadlineWithoutInstant`, `FiringTimeDisagrees`,
   `LoopDeadlineMismatch`; the unknown-kind case disappears with A-T2-5);
   only `decode` additionally wraps a `DecodeError`. The three cross-record
   validators move to journal acceptance, next to the Loop and Retry folds
   they already name, so `timing.rs` is what its header says: the type and
   its codec. `integer`/`signed` join their twins in `retention.rs:55,97`.
2. The admission-generation rule is written twice with two state
   representations: `AdmissionPrefix` (`admission.rs:31-110`) and
   `MemJournal::instance_generation_for_admission` (`journal.rs:2397`), both
   deciding "current generation if active and not closing, next otherwise".
   Target: one fold owns the rule; the live journal keeps it as its index and
   recovery feeds it a prefix, so the two callers cannot diverge.
3. Residue, cosmetic: `impl InstanceAdmissions for AdmissionPrefix`
   (`admission.rs:112`) lists its members in the opposite order of the trait
   (`:10-15`); the IDE flags it, clippy does not.

No byte moves, no golden. Not a pure rename, so by the entry rule of
2026-09-26 it is not pre-opening work: v21 pass, after 15 October. Owner:
Astra; regression for each typed error and one prefix-versus-live
equivalence test for the generation rule.

**A-T2-7 — A field is sealed or unsealed; `journal::retention` becomes
`journal::codec`.** Decision (Yoann, 2026-10-05). The pair "sealed / retained"
confused readers because "retained" named three things at once: a category of
fields outside the causal seal (keys 56 and 59), the ordinary verb "kept"
(`retain_emission_routes`, `retained_observations`, "the journal retains the
complete envelope"), and the module `journal/retention.rs`, whose encoding
contains sealed and unsealed fields alike. Rule: a journal field has one
property, it enters the causal seal or it does not; the words are **sealed**
and **unsealed**, as signed and unsigned headers, and the repository already
uses them (`golden_trace.rs`: "retained call time is unsealed …";
decisions.md: "outside the seal"). No noun is needed for the unsealed
category: today it holds exactly two fields, both instants, named by what
they are (`deadlines`, `closed_at`). Astra's "trace / context" was set aside:
"trace" already names the sealed projection (`.trace` goldens), and "context"
already carries four meanings in the code (`RoutedContext`, `ContextOwner`,
Retry attempt context, `ServerContext`). Proposed application, Astra, in the
v21 vocabulary pass with A-T2-5 and A-T2-6:

| Today | Target |
|---|---|
| `journal::retention` | `journal::codec` — the complete, lossless encoding of a `Record`, sealed and unsealed fields together |
| `retention::record(&Record) -> Cbor` | `codec::encode` |
| `retention::Reader` | `codec::Decoder` (stateful: parents and command producers must precede their references) |
| `retention::Error` | `codec::Error` |
| "retained field / schedule / instant / time" in `identity.md`, `decisions.md`, `delivery-semantics.md`, code comments | "unsealed …" |
| "retained" as the verb *kept* | unchanged |

The sweep must distinguish the category noun from the verb: 204 spec
mentions and about 600 code mentions of "retain" are mostly the verb. The
sentence for a newcomer: the journal seals what happened; the instants stay
unsealed so a resume lands on the exact second. No byte moves: the codec's
bytes, the registry numbers and every golden stay.

**Decision — keep v20 during the work; format 1 before opening, no legacy.** (Yoann,
2026-10-05.) The written valve applied: v20 was not blessed on 3 October, so
the opening moves to **5 November 2026**, real freeze on 24 October. Every
approved action of T1 and T2-so-far lands before that freeze in **one** byte
movement. The current chantier keeps the name and planned number **v20**.
**Release TODO:** renumber the public format to **1** before publication;
apply the corresponding recording-version choice in that same release step,
not in the byte-neutral lots. The first public format carries the first number.
The executable package stays v19 until the reviewed cutover. In that cutover no
registry keeps a retired, reserved or historical row — envelope key 6, key 53,
deadline-kind byte 4, the `role` slot, retired `ExtensionSpec` tags, the
`rastok.loop.r1` and `rastok.aspects.r1` domains all disappear and the numbers
compact; no reader for an earlier version, no alias for an old spelling; old
packages, recordings and goldens are regenerated once with their reasons, never
converted. The extend-only discipline (append, never renumber, never reuse)
starts at format 1. Dated reports under `docs/architecture/` stay as evidence;
no live specification describes an earlier format. The consolidated brief
handed to Astra on 2026-10-05 carries the lots and the cutover protocol.

### `loop_protocol.rs` and `loop_protocol/fold.rs` — read 2026-10-08

Read in full by Fable (brief for Yoann); the reading resumed on the working
branch after the 7 October deposit of `fedada7`. The protocol is small and
closed: three derived identities (`run_ref`, `command_ref`, `admission_ref`),
five facts, one sparse packet; the fold refuses every contradiction it can
see without the plan and clones itself before each application so a refusal
changes nothing. Verified conforming: `Fact::Advance.due_at` is retained by
the lossless codec (`codec.rs:552-561`) and excluded from the key-40 causal
form (`identity.md:300-303`, `golden.rs:532-543`), and journal acceptance
checks it against the key-41 deadline (`acceptance.rs:78-120`), so the
field's comment "outside causal identity" is exact; the identity recipes
frame every part with its length (`envelope.rs:127-139`); a command of an
inner run names the enclosing runs in its context and the fold requires them
to be active and strictly nested (`fold.rs:258-282`).

**A-T2-8 — Stale identity-domain labels (zero bytes, before the freeze).**
The three recipe comments still name the string domains of the pre-`IdKind`
cutover: `rastok.loop-run.r2`, `rastok.loop-command.r1`,
`rastok.loop-admission.r1` (`loop_protocol.rs:279,298,316`), while the code
derives with `IdKind::LoopRun`/`LoopCommand`/`LoopAdmission` and the
specification says "Domains are run `0x08`, command `0x09` and re-admission
`0x03`" (`identity.md:302`). The live Loop specification carries the same
residue in its recipe block, `derive(rastok.loop.r2, instance::Generation,
OpeningAdmissionId)` (`loop.md:93-98`), which contradicts `identity.md`; the
protocol domain `rastok.loop.r2` of the plan stream (`identity.md:106`,
`lower.rs:134`) is not an identity domain and stays. Residue of the accepted
7 October sweep; comment and specification text only. **Landed 8 October by
Fable on the review branch**, with the key-24 precision below: the three
comments name `IdKind::LoopRun`/`LoopCommand`/`LoopAdmission` with their
bytes, `loop.md` writes `blake3-derive( 0x08, … )` in the envelope.md §2
notation and says that `rastok.loop.r2` names the protocol, never an identity
domain. Verified: fmt, strict Clippy and rustdoc on the core, the rustdoc
link checker, and the spec-reading suites (`site_content`, `golden_trace`,
`plan_package`, `jsonl_fixture`, `plan_validation`) pass; no witness moves.

**A-T2-9 — The Loop fold accepts `Close { Stopped }` without closing
evidence (v21).** `fold.rs:486-488` closes a run on a `Stopped` reason on any
transaction of the owner instance, dropping a pending command, because the
fold tracks only `l:start`/`l:stop` links (`parents`, `fold.rs:114`) and not
the closing scope. The VM produces that fact on the owner's stop record
(`loop_manager.rs:51-78`), on a handler transaction carrying bare `stop`
(`loop_manager.rs:314-319`) and on a standalone `l:next` manager record once
the instance has left `Open` (`loop_manager.rs:482-504`); recovery refuses a
manager stop whose instance is still `Open` (`recovery/loops.rs:162-175`),
but journal acceptance runs the fold alone (`journal.rs:2062-2072`). The
instance-closing fold already derives the closing scope from
`close_scope_generation` (`instance_stop.rs:463-488`); the Loop fold can do
the same and require, for `Stopped`, that the run's instance is within a
retained closing scope. Not visible, not a format: v21, with the
regression "a `Stopped` close on a live instance refuses". Owner: Astra.

**A-T2-10 — The key-40 causal projection lives outside the core (v21,
zero bytes).** The sealed form of the Loop packet is encoded by
`rastok::golden::loop_packet_tree` (`golden.rs:428-596`, inserted at
`golden.rs:719`), whereas the instance-closing and Retry packets project
themselves (`instance_stop::Packet::tree`, `instance_stop.rs:94`;
`retry_protocol::Packet::tree`, `retry_protocol.rs:244`, sharing
`encoding::packet_tree` with the lossless carrier). The rule that `due_at`
is excluded from the seal is therefore enforced two crates away from the
type that documents it. Target: `loop_protocol::Packet::tree`, shared with
the codec the way Retry's `encoding.rs` is; `golden.rs` calls it. Bytes
unchanged; the trace goldens are the regression. Owner: Astra.

### `retry_protocol.rs`, `carriers.rs`, `encoding.rs` and `retry_protocol/fold.rs` — read 2026-10-08

Verified conforming: the pin's `opened_at` and the prepared request bytes
stay out of the causal form (`encoding.rs:14-30`, `identity.md:326-328`);
the fold re-derives the operation identity from the opening authority and
never from a fact (`fold.rs:325-338`); every attempt's Call must repeat the
pinned request, settings and key (`check_intent`, `fold.rs:466-479`); the
winning closure fixes `withhold` before any later Closing (`fold.rs:516-517`)
and a held batch is consumed or published exactly once; the recurrence
deadline is checked against the policy and the closure instant
(`fold.rs:525-544`). Journal acceptance runs only the carrier check
(`carriers::validate_transaction`, `journal.rs:2055`); the fold needs the
original plan (`Opening`), so it runs in the VM and in recovery
(`retry_manager.rs`, `recovery/retries.rs`). Accepted, with the reason
recorded here.

**A-T2-11 — Retry's carrier matrix and completion checks are written
several times (v21, zero bytes).** The admissible packet shapes are listed
twice, in `carriers.rs:29-84` and in the fold's `apply_inner`
(`fold.rs:242-307`); the "batch equals the ordered emissions" check exists in
`carriers::producer` (`carriers.rs:129-137`) and `Fold::validate_emissions`
(`fold.rs:626-633`); the terminal case of a completion is read two ways,
from the last output's union case in `closure_schedule`
(`retry_manager.rs:679-690`) and from the terminal port through the Contract
in `closure_case` (`retry_manager.rs:721-727`) and `validate_emissions`
(`fold.rs:640-649`), beside the Contract check of `call_completion.rs:22-61`.
Same class as A-T2-6(2): one owner per rule. Also `fold.rs:334` refuses an
opening dated after `at` as `OriginalBinding` where `admit` classifies the
same contradiction as `Timing` (`fold.rs:407-409`). Owner: Astra.

### `journal/instance_stop.rs` — read 2026-10-08

Verified conforming: the command identity covers source, scope, target and
cause (`instance_stop.rs:60-79`) and is re-derived at decode
(`instance_stop.rs:182-191`); the target is resolved only along the exact
parent-generation chain and never by choosing a newest generation
(`resolve`, `instance_stop.rs:364-419`); consumption follows production
order across targets, requires the winning `l:closing` of the command's own
target and settles redundant requests by that same close
(`instance_stop.rs:456-498, 571-610`); the VM queue is a projection with the
same order (`vm/instance_stop.rs:143-213`) and recovery rebuilds it from the
fold (`recovery.rs:120-130`). Nothing to change. One cost note joins A-T2-3:
`MemJournal::instance_commands` re-folds the whole journal on every call
(`journal.rs:1888-1890`), on each accepted transaction that carries commands
(`journal.rs:2058`) and twice per producing transaction in the VM
(`vm/instance_stop.rs:95,128`), where `loop_protocol()` returns the fold the
journal already maintains (`journal.rs:1879-1885`). M2 with the indexes.

**T2 is read.** Conforming and not recorded elsewhere: `deadline.rs`,
`acceptance.rs` and `codec.rs` were read with the lot-3 complements. Found
across the three folds, M2 design note next to A-T2-2(b): each fold rebuilds
its own lifecycle tree — Loop from `l:start`/`l:stop` (`fold.rs:114,172-184`),
instance closing from start, close scope and stop (`instance_stop.rs:256-260`),
Retry from a set the VM feeds (`begin_closing`, `fold.rs:208-210`,
`vm.rs:3850-3852`) — and each `apply` clones the whole fold, envelopes and
pinned request bytes included (`fold.rs:149-154`, `retry fold.rs:230-234,
488`, `instance_stop.rs:338-343`). One lifecycle fold consulted by the three
protocols is the natural shape for the durable journal; not before.

## T4 — The VM, the deterministic step

### First pass — read 2026-10-08

Read in full: `vm/advance.rs`, `vm/closing.rs`, `vm/call_completion.rs`,
`vm/instance_stop.rs`, `vm/timing.rs`, `vm/loop_manager.rs`,
`vm/retry_manager.rs`, `vm/recovery.rs`, `recovery/{loops,retries,calls,
ingress}.rs`, `runtime/dispatch.rs`, the mailboxes (`runtime.rs:205-432`).
Read selectively in `vm.rs`: the scheduler (`:916-958`), `step_with_bindings`
and the join scan (`:2108-2319`), `advance`/`peek_due`/`expire`
(`:2718-3103`), the session handshake and lifetime expiries (`:3105-3293`),
`request_close` through `fire_lifecycle_next` and `promote_waiting`
(`:3786-4362`), `settle_next` (`:4367-4455`), `operator_work`,
`fire_operator`, `fire_connector` (`:4573-4849`), `open_call` and
`retain_call_open` (`:5476-5605`), `validate_and_prepare_emissions`
(`:5630-5690`), `check_room_for_routes` through `install_target`
(`:6598-6877`); in `graph_loop.rs`, `restore_graph_manager`
(`:1003-1233`). Not yet read: `inject_submission` (`vm.rs:1673-2107`),
`prepare_routes`/`prepare_owner_target`/`candidate_owner`
(`vm.rs:5695-6330`), `recovery/exchanges.rs`, the rest of `graph_loop.rs`,
`boot_at_inner`, `runtime.rs` outside the mailboxes.

Verified conforming, the promise's load-bearing parts:

- **One step order, journal first.** `step_with_bindings` tries, in a fixed
  order, retry-closure synchronization, instance commands, single-pass
  close, closures, lifecycle work, Graph Loop openings, the dated recurrence
  head, the Loop and Retry managers, settled inbound Calls, Sessions and
  Calls, then the first runnable mailbox in canonical `(slot, instance)`
  order (`vm.rs:2118-2223`, `runtime.rs:289-336`). Every transaction commits
  its record before any mailbox, state or scheduler mutation, and those
  mutations are infallible by prior reservation (`fire_operator`,
  `vm.rs:4793-4812`; `settle_next`, `:4434-4453`; `retain_call_open`,
  `:5586-5604`; `route_prepared`, `:6742-6754`).
- **Expiry is reserved, not consumed.** `peek_due` keeps the deadline until
  the expiry commits (`vm.rs:943-957`, `advance.rs:137-149`); an open Call
  closes once with its Timeout fact and its Driver cancel is proved before
  the close-once mutation (`vm.rs:2855-2890`); a Call that expires in its
  queue still opens and closes with the same causality (`vm.rs:2906-3100`);
  a ready mailbox whose budget is already due is expired before any intent
  reaches a Driver (`open_call`, `vm.rs:5484-5495`).
- **Closing is inner-first and deterministic.** `progress_closures` stops one
  instance per call, deepest owner path first then identity order, only when
  its scope is quiescent, it has no descendants and no stop product can still
  reach it (`vm.rs:3890-3933`, `closing.rs`); a Graph request marks the
  subtree and later children inherit the close (`activate_close`,
  `vm.rs:3836-3888`; `install_target`, `:6821-6863`).
- **Recovery re-derives and compares; it never re-runs a producer.** Retained
  instants must be monotone and not after the resume instant
  (`recovery.rs:64-98`); routes, routed contexts, deadlines and instance
  commands are recomputed from the plan and the admission prefix and must
  equal the record (`recovery.rs:441-521`, `recovery/ingress.rs:212-279`,
  `recovery/retries.rs:145-198`); an open Call is rebuilt as pending without
  restarting the effect and a closure needs its retained opening
  (`recovery/calls.rs:38-118, 120-186`); every manager transition is checked
  against the fold and the deployed Loop, including the stop authority
  (`recovery/loops.rs`, `graph_loop.rs:1003-1233`); Loop deadlines come from
  retained `Advance` facts, never from the boot instant
  (`recovery.rs:530-552`).
- **The cursor key is `(edge, source instance)`** as key 20 seals it
  (`runtime.rs:262-266`); a successor generation starts its own sequence.
- **Dispatch is a pure selection** with a depth budget, closed matches
  requiring exactly one arm and open matches taking the first true
  predicate (`dispatch.rs`).
- **Typed and untyped materialization agree** where the Loop expiry needs
  them to: the VM builds `LoopExpired.last_state` through the entry type
  (`loop_manager.rs:699-708`) and the fold checks a `Text` payload through
  the typed reading and anything else through `data_value`
  (`fold.rs:534-546`); `DataSlot::materialize` differs from `data_value` only
  for raw bytes read as `Text` (`runtime.rs:2065-2093, 2479-2498`).

**A-T4-1 — The drain expiry mutates before its fallible Driver cancel
(v21).** `expire_closing` for `ClosingDeadline::Drain` calls
`begin_settlement` (`vm.rs:4004`), which moves every instance of the scope
to `Settling` and residualizes mailboxes, before cancelling the pending Calls
through the Driver (`vm.rs:4005-4021`). A cancel refusal leaves the deadline
reserved, as designed, but the replay finds `stage == Settling`, so the
guarded arm (`vm.rs:4003`) no longer matches and the empty `Drain` arm
(`vm.rs:4148`) acknowledges the deadline: the remaining Calls are never
cancelled and wait for the settlement deadline, which abandons them
(`vm.rs:4051-4086`). The journal stays consistent, so this is robustness,
not correctness. Target: prove every cancel before `begin_settlement`, as
`expire` already does for one Call (`vm.rs:2855-2868`), or make the
`Settling` replay re-issue the cancels. The settlement arm is already
replayable (`abandon_egress` answers `AlreadyClosed`). Owner: Astra;
regression with a Driver whose first cancel refuses.

**A-T4-2 — Fuel resolution is written three times (v21, zero bytes).**
`loop_manager.rs:159-174`, `recovery/loops.rs:75-90` and
`graph_loop.rs:1078-1093` each resolve the Loop fuel from `spec.fuel` or the
proof's measure; the second spells the conversion `*n as u64` where the
others use `u64::try_from`. One `fn resolved_fuel(spec, envelope, instance)`.
Cosmetic beside it: `loop_manager.rs:341-350` assigns `context: vec![run]`
and overwrites it at `:349`; `loop_manager.rs:748,781-782` recompute
`current.iteration + 1` unchecked after the checked addition at `:643-646`.
Owner: Astra.

**A-T4-3 — Whole-journal scans on the step path (A-T2-3, M2).** Recorded so
the M2 index design has its list: `next_dated_recurrence`
(`retry_manager.rs:353-413`) runs on every step once a Loop or Retry exists
(`vm.rs:2143`), `synchronize_retry_closures` (`retry_manager.rs:188-197`) on
every step (`vm.rs:2124`), `step_retry_manager` (`retry_manager.rs:433-473`),
`step_instance_commands` (`vm/instance_stop.rs:182-198`), `expire`
(`vm.rs:2840-2851`) and `restore_instance_closure` (`recovery.rs:742-763`)
each scan the records, besides the `find_call`/`find_envelope` lookups
already named by A-T2-3. The mailbox sequence table `edge_seq`
(`runtime.rs:215, 262-266`) is never pruned for stopped generations. Not
v21: the behaviour is the deliberately linear M1 implementation.

**Specification precision, before the freeze (zero bytes).** Key 24 lists
kind 3 among the sealed deadline ids and says "Kind 3 uses InstanceLifetime"
(`identity.md:256`); the VM retains lifetime deadlines at key 41 only
(`vm/timing.rs:9-75`, `retain_deadlines`) and never seals one, and recovery
refuses a sealed kind 3 (`recovery.rs:223-233`). One sentence: lifetime
deadlines are retained, never sealed. Landed with A-T2-8 (8 October).

### Second pass — read 2026-10-11

Block 1 of the planned second pass, read in full by Fable (brief for
Yoann) on the current code: `inject_submission` (`vm.rs:1673-2107`), its
two public faces `inject` and `admit_ingress` (`vm.rs:1439-1492`), the
journal's preparation contract (`journal.rs:1236-1273`) and the Driver side
of the door (`RuntimeBindings::pump_inbound`, `registry.rs:614-640`;
`IngressProposal`, `boundary.rs:1808`). Blocks 2–7 are not yet read.

Verified conforming:

- **One door, one order.** Every root entering the graph — boundary
  injection or a Connector-origin fact — passes the same transaction:
  preflight, exact endpoint from the validated plan, `meta.ext` judged
  against the travelling schema, pins judged for the ingress port and every
  destination, root minted by the journal, source instance selected for a
  Call or Session, routes prepared outermost-to-innermost as pure
  candidates, room checked, instance commands prepared, deadlines
  collected, `commit_ingress`, then install and `route_prepared`. Nothing
  executes; the fact waits in a mailbox for `step`.
- **Preparation is read-only, by signature.** `prepare_ingress`,
  `prepare_connector_ingress` and `prepare_session_ingress` take `&self`
  and the contract says "only `commit_ingress` consumes the sequence". A
  `MailboxFull` (`:1821`, `:1881`) or `PlanInvariant` refusal after the
  root is minted (`:1776`) therefore leaves no journal trace, for any
  implementation. Block-1 question 1 is settled.
- **Backpressure without loss.** `admit_ingress` maps `MailboxFull` and
  `InstanceClosing` to `Pending`, the four peer/data refusals to a typed
  `Rejected`; `pump_inbound` retains both the proposal and the Driver's
  transport unit while `Pending`, and the Driver learns `Accepted` only
  after the commit. External priority 8 holds on this door.
- **The peer's `session_id` never reaches a key.** The `meta.ext` loop
  skips `session_id` when framing is present (`:1735-1737`); the journal
  derives or verifies the logical ref and overwrites the projection inside
  `prepare_session_ingress`; routes and keys are then prepared from
  `inbound.envelope()`. Block-1 question 4 is settled.

Still open from block 1: whether a Session message's ancestor-scope
preservation (`:1944-1950`) selects the same generation as `restore_session`
at recovery, and where recovery reconstructs the handshake seed that
`opening_only` deliberately routes nowhere (`:1934-1941`) — both for
block 3.

**A-T4-4 — `inject_submission` is one 435-line transaction, with a rule
written twice and one fallible branch after its commit (realignment,
lot 1).** The function is thirteen commented paragraphs without function
names. Three things the length hides. (1) The source-instance reservation
— first owner of the chain, `prepare_target`, held obligations counted
against `WORKER_MAILBOX_CAPACITY`, `MailboxFull` — is written twice, for
the inbound Call (`:1800-1825`) and for the Session opening
(`:1860-1885`); family A-T4-2. (2) `source_target` and `session_target`
are `Option<(Slot, PreparedTarget, bool)>` whose boolean means "opening":
an unwritten four-case enum (none, Call, Session opening, Session message)
whose constructor is the five-arm framing matrix (`:1833-1925`) and its
four `PlanInvariant` refusals. (3) After `commit_ingress` (`:2009-2011`)
one `PlanInvariant` can still return (`:2073`, "a committed Session opening
lost its logical reference"): by then `enqueue_instance_commands` and
`install_target` have run, `sessions.insert` and `route_prepared` have
not. The committed exchange derives from the prepared Session record, so
the branch is unreachable by construction — but it is exactly the shape
external priority 2 hunts, and the only post-commit refusal on this path.
Target: the `fire_operator` pattern, prepare (pure, fallible) → commit →
install (infallible): `resolve_ingress_endpoint`, `judge_ingress_metadata`,
`judge_ingress_pins`, `mint_root`, `admit_inbound_exchange` returning the
enum, `prepare_root_routes`, `reserve_root_admission`,
`attach_ingress_deadlines`, then `install_inbound_exchange` after the
commit; one `reserve_source_instance` shared by Call and Session; the
exchange id derived before the commit, or exposed typed by
`CommittedIngress`, so nothing after the commit can refuse. Zero bytes;
no semantic change. Owner: Astra. Lot: realignment lot 1 rather than lot 2,
because the endpoint resolution (`EdgeEnd { node: None }` versus a
Connector slot, `:1683-1727`) is what the single typed index rewrites. Not
before the freeze: this is the most sensitive core path and the external
brief references it by line.

**Block 4 — the Graph-hosted Loop manager, `graph_loop.rs` — read from
2026-10-15.** Fable's brief (a ten-step reading order, eight observations
to confirm or refute, the Rust forms a C#/Scala reader meets) was given in
session; Yoann reads in the IDE and comments; only confirmed items are
recorded here, as they are confirmed.

Yoann relayed Fable's eight observations to Astra for an independent
check (on `f5738b2`, 76 targeted Loop, composition, Graph Call and Retry
tests passed, full gate not rerun, nothing modified); Fable then verified
Astra's answer against the code. Astra confirms no functional defect on the
paths examined. Two of Fable's observations were wrong in their
explanation and are corrected below; the others stand as structure
findings.

Confirmed:

- **Neither Loop-manager file says what it is (Yoann, first comment).**
  The specification has one Loop manager whatever the host (`loop.md` §2:
  Operator, Graph, finite outbound Call). The code splits it by host.
  `loop_manager.rs` holds the protocol shared by every host — the fold
  installed from committed journal facts (`refresh_loop_manager`),
  opening, command admission, fuel, verdict, closing — and serves
  Operator- and Call-hosted Loops entirely. `graph_loop.rs` holds what
  exists only because the host is a scope without handler, mailbox or
  application state: the admission retained at the Graph boundary before
  fan-out, the entry routes deployed into the contained Executors, the run
  context pushed on each delivery, the verdicts collected from the
  children's transactions, quiescence decided by scanning their
  obligations — §2's "Its Loop manager observes causal work in the
  contained Executors". The two `//!` headers say none of this:
  `graph_loop.rs` opens on "Retained Loop Graph admissions stop boundary
  expansion before fan-out. The attached manager owns these bounded
  deliveries; Graphs stay scopes", `loop_manager.rs` on "Command-driven
  Loop applications". A reader arriving from the specification cannot
  tell, from either, that they are two halves of one manager, nor which
  half they hold.
- **Fuel is resolved a fourth time, and both resolutions refuse the same
  inputs.** `open_graph_loop` errors on a non-positive or non-integer fuel;
  `restore_graph_manager` maps the same arm to `None` and then refuses
  because `expected != Some(fuel)`. Same outcome, two spellings — family
  A-T4-2, whose count becomes four (`open_graph_loop`,
  `restore_graph_manager`, `loop_manager.rs`, `recovery/loops.rs`).
- **The admission derivation is written twice.** `admit_graph_loop`
  derives the consumed boundary position, the expected routes, the
  contexts and the deadlines, commits, then applies; the `Admit` arm of
  `restore_graph_manager` derives the same from the plan, the previous run
  state and the journal prefix, then compares exactly. Astra's reading,
  which Fable adopts: share the *preparation*, not the controls — the live
  path asks for capacity and commits, recovery asks for an exact match and
  never trusts the recorded fields; what means "wait for room" live must
  refuse at recovery.
- **`deliver_graph_admission` mutates before its only `false`.**
  `apply_instance_admissions` and the route interning precede the
  `graph_loops` presence test; `false` means "continue ordinary routing",
  not "undo". The presence is guaranteed at construction whenever the plan
  holds a Loop Graph (`Execution::new` builds `graph_loops` from the
  plan), so the branch is unreachable in a valid execution — the shape
  external priority 2 hunts, as in A-T4-4 (3).
- **Fifteen `graph_error` refusals, one variant.** All are
  `InvalidLoopDecision`; several are plan invariants the validator already
  proved ("lost its Loop specification", "no declared owner"), one is dead
  ("a case route has no value": `value` was just set in the branch above).
  Three categories to separate site by site: invariant guaranteed by the
  plan → eliminate the case or `PlanInvariant`; genuine Loop protocol
  fault → keep `InvalidLoopDecision`; inconsistent journal → recovery
  error. No blanket replacement, no `unwrap` to hide a case.
- **The producer is found by a linear path search** in
  `retain_graph_verdicts` and `graph_entry_verdict` while the realigned
  caller holds the `NodeIndex`. Paths stay the reference for journaled
  identities; the lookup can take the index.
- **The two sorts in `select_graph_routes` / `prepare_graph_routes` are
  not redundant, and not interchangeable (Astra's nuance, verified).** The
  first sort, by `destination_order`, runs before the traversal pushes
  commands, instance stops and pending crossings, so it fixes the
  traversal of passive Graphs and the discovery order of commands; the
  `BTreeSet<Endpoint>` deduplicates; the sort in `prepare_graph_routes` by
  `endpoint_order` restores the canonical delivery order. The comment above
  the first sort ("Keep the canonical path order used by source routing and
  identities") describes the second sort's job. Replacing either by
  Node-index order would be wrong.
- **Two sequence tables, one mechanism, two authorities (Astra's reading,
  verified).** `GraphLoops::sequences` and `Mailboxes::edge_seq`
  (`runtime.rs`) share the key `(EdgeIndex, Generation)`; routing uses one
  or the other for a given delivery, never both, so no delivery is counted
  twice. A common allocator is conceivable; merging the queues is not — it
  would give the Graph the application mailbox §2 denies it.

Corrected from Fable's brief:

- **`refresh_loop_manager` is not a journal scan.** `MemJournal`, the only
  production `Journal` (the other implementor is a test double in
  `execution.rs`), keeps `command_loops: Option<Box<LoopView>>`; at commit
  it applies the transaction to a clone of that view *before* pushing the
  record and installs the new view with it; `Journal::loop_protocol`
  returns a clone of the maintained view and folds the records only when
  no view exists yet. So the refresh after each manager commit cannot
  refuse with `MemJournal`. Two residues stay: the trait signature is
  `Result`, a fallible interface after the commit (external priority 2,
  interface level); and the refresh clones the whole view at each of its
  fourteen call sites, a cost to measure (A-T4-3, M2), not a scan.
- **Quiescence (§7) covers the deadlines of today's admitted perimeter,
  but not through join windows.** The validator (`plan/graph_loop.rs`)
  refuses, inside a Loop Graph, any join with a guard, a `within` or a
  `last` ("a retained or expiring join needs a finite per-run completion
  proof") and requires a positive plan-constant timeout on every outbound
  Call. Coverage therefore holds through pending Calls, open Retry
  operations and nested runs awaiting a verdict; the join-window
  transitivity Fable offered does not apply. `graph_run_has_work`,
  `update_graph_loop_contexts` (rebuilding the live set each call) and the
  worst-case quadratic `next_graph_admission` join A-T4-3's list.

**A-T4-5 — The two Loop-manager module headers name the split, and three
comments say what the code does not (before the freeze, zero bytes).**
Rewrite both `//!` headers so that each states: that the specification's
Loop manager is one and the code splits it by host; what this file holds
and for which hosts; where the other half is; the `loop.md` section that
grounds the split (§2 for the Graph half). The invariant sentence each
header already carries (boundary expansion stopped before fan-out;
preparation never consumes a command or a cursor) stays. Three comments:
the two sequence tables and why they are not merged
(`GraphLoops::sequences`); the two sorts and their distinct jobs
(`select_graph_routes`); the deadline coverage of quiescence by the
validator's restrictions (`graph_run_has_work`). Owner: Astra. Lot: before
the 23 October freeze, with the documentation touch; no code moves.

**A-T4-6 — One fuel resolution and one admission preparation shared by
the live path and recovery (v21, zero bytes).** A pure fuel resolution
replacing the four spellings (closes the count of A-T4-2); one admission
preparation — consumed boundary position, routes, contexts, deadlines —
called by `admit_graph_loop` (capacity, then commit, then apply) and by the
`Admit` arm of `restore_graph_manager` (exact comparison with the record).
Recovery keeps recomputing from the plan and the previous state and keeps
refusing what the live path would merely defer. No new manager object.
Owner: Astra. Lot: v21, after the opening.

**A-T4-7 — No renunciation after mutation in `deliver_graph_admission`
(v21, zero bytes).** Recognise the destination and check every
precondition, `graph_loops` included, before `apply_instance_admissions`
and the route interning; then apply without a refusal path. Family A-T4-4
(3), external priority 2. Owner: Astra. Lot: v21, after the opening.

**A-T4-8 — Classify the `graph_error` refusals site by site, remove the
dead branch, look the producer up by index (v21).** The three categories
above; "a case route has no value" removed; `retain_graph_verdicts` and
`graph_entry_verdict` take the `NodeIndex` the caller holds. Owner: Astra.
Lot: v21, after the opening; the lot placements of A-T4-6 to A-T4-8 are
Fable's proposal, to be confirmed by Yoann.

**Applied 17 October (`c57fbd7`, Astra), reviewed by Fable.** Yoann asked
for a light evening rework; Fable briefed A-T4-5, A-T4-7 and the dead
branch of A-T4-8 only, A-T4-6 and the rest of A-T4-8 staying in v21. Astra
landed the three in one commit touching `graph_loop.rs` and
`loop_manager.rs` alone: both `//!` headers now name the split by host and
point at each other, the invariant sentences kept; the three comments sit
on `GraphLoops::sequences`, on the sort in `select_graph_routes` and in
`graph_run_has_work`; `deliver_graph_admission` binds `graph_loops` and
computes the opening before `routes.insert`, `sequences`, `admissions` and
a final `apply_instance_admissions`, so no refusal follows a mutation;
the case value is bound with `Option::insert` and the "a case route has no
value" refusal is gone. Fable reproduced the four completion checks on the
commit: fmt, clippy `-D warnings`, rustdoc, and 1 598 tests in 61 groups,
0 failed, 0 ignored; no `.rkp` or golden moved; `unreachable!` counts
unchanged (0 in `graph_loop.rs`, 32 in `execution.rs`). Block 4 is closed
for the human review; A-T4-6 and the remainder of A-T4-8 wait for v21.

### Disposition of the 8 October findings

| Lot | Items |
|---|---|
| Before the 23/24 October freeze, zero bytes | A-T2-8 (three comments, `loop.md` recipe); key-24 precision in `identity.md` — both landed 8 October |
| v21, after the opening | A-T2-9 (Loop fold learns the closing scope), A-T2-10 (`loop_protocol::Packet::tree`), A-T2-11 (one Retry carrier matrix, one terminal-case reading, `Timing` classification), A-T4-1 (drain cancels before `begin_settlement`), A-T4-2 (one fuel resolution, two cosmetic residues) |
| M2, with the durable journal | A-T4-3 and the `instance_commands` re-fold (A-T2-3), one lifecycle fold for the three protocols (A-T2-2(b)) |
| Realignment lot 1 (D-2026-10-11-1), before the opening | A-T4-4 (11 October) |

Yoann's 8 October brief dates the real freeze **Friday 23 October**; this
ledger, `ROADMAP.md:59`, `opening-showcase.md:3` and `decisions.md:137` say
24 October, a Saturday. To align in the next documentation touch; the brief
is taken as the intent.

**Still to read in T4** before the external review: `inject_submission`,
`prepare_routes` through `candidate_owner`, `recovery/exchanges.rs`, the
Graph Loop manager (`graph_loop.rs` except its recovery check), and the
`unreachable!`/`unwrap_or_else` sites of `vm.rs` one by one.
Block 1, `inject_submission`, was read on 11 October: see the second pass
above; blocks 2–7 remain. **12 October:** suspended here by Yoann's
decision, so that the realignment, A-T4-4 and the rename lot start; blocks
2–7 are read on the realigned code.
**13 October:** the rename lot is applied by Astra (ten commits, `32bc654`
to `5bc7f0a`, branch `realign/node-ontology` from `4b4a4a4`) and reviewed by
Fable: every decided name in place, no witness moved, the four completion
checks and the rp2350 spike check green (1 572 passed in 61 groups, 0 failed, 0 ignored). The review, one
correction (bless is the specification's word, `--bless`) and the missed
derivatives are in the ledger's *Applied — 13 October*. `inject_submission`
now reads `admit_proposal`; A-T4-4 keeps its decomposition. Realignment
lot 1 starts on this base.
**15 October:** lots 1 and 2 of the realignment landed and were reviewed
(see *Applied and reviewed — 14–15 October* in the realignment section);
blocks 2–7 are read on the realigned code, `runtime/execution.rs` and its
`inspection` and `admission` children. Block 4 (`graph_loop.rs`) started
the same evening and closed on 17 October with its applied rework: see
*Block 4* above. Blocks 2, 3, 5, 6 and 7 remain.

## T5 — The effect boundary and the Driver seam

### `boundary.rs`, `connector.rs`, `connectors/registry.rs` — read from 2026-10-17

**Decisions of 17 October (Yoann).** The lot placements of A-T4-6 and of
the remainder of A-T4-8 are confirmed: v21, after the opening. The human
review changes mode for what remains before the freeze: Yoann reads
skeletons (headers, types, signatures) and judges names, placement of
concerns and the contributor's path; the bodies are cross-checked by Fable
and Astra and read line by line by the external Rust reviewer (26–30
October); the remaining T4 blocks (2, 3, 5, 6, 7) follow that mode. The
pre-freeze priority is T5 with one T9 Driver as the example, because a
contributor who wants to add a Connector meets this seam first; the
"first hour as a stranger" path (T10) follows.

Fable's T5 brief laid out the seam: Contract as compiler data
(`rastok/src/contract.rs`, the seven standard sheets in `stdlib.rs`),
semantic ids in core (`connector.rs`: `ContractId`, `ConnectorDirection`,
`ConnectorExchange`; `connector/call.rs`: `CallSemantics`), the seam itself
in `boundary.rs` (identities, journaled intents, observations and
admission, obligations, eleven traits, `DriverPoller`, and the
standard-library mirror), assembly in the host (`connectors/registry.rs`:
`ConnectorDriverFactory`, `BoundConnectorDriver`, `DriverRegistry`,
`RuntimeBindings`), the Drivers of T9 and the Lab mocks, and the two
consumers (`Execution::step_with_bindings` over
`&mut dyn OutboundConnectorBindings`; `Worker`'s `Host::poll_input` fed by
`RuntimeBindings::pump_inbound`). Four structure observations are left to
confirm during the reading, none a defect: a contributed Connector touches
the `no_std` core through the mirror the guide's step 1 documents ("the
small Registry-free core mirror": fourteen functions, five `*Exit` enums,
seven id constants), so adding a Contract today means a sheet in
`stdlib.rs`, a mirror in `rastok-core`, a Driver under `connectors/` and a
conformance profile; `OutboundConnectorBindings` and
`SessionConnectorBindings` overlap, `RuntimeBindings` implementing both
with the former delegating to the latter method by method, while a blanket
impl makes any `CallConnectorDriver` an `OutboundConnectorBindings` for
tests and replay; five words name one family (`*ConnectorDriver`,
`*ConnectorBindings`, `BoundConnectorDriver`, `RuntimeBindings`,
`ConnectorDriverFactory`); the guide's review checklist is the one a
contributor must be able to tick alone.

Yoann's first comments are names, recorded and verified in the
[vocabulary ledger](human-review-vocabulary.md) as ⟂ 8a to 8d: `boundary`
as a catch-all module, the repeated suffix `ConnectorDriver` as a missing
`driver` module, `*Bindings` reopened against its 12 October settlement
(Fable recommends keeping the specification's word and stating the sense
in each trait's first doc line), and "VM" surviving in prose.

**A-T5-1 — "VM" becomes `Execution` in prose, specification first (before
the freeze, zero bytes).** 77 code comments and 29 specification sentences
outside `decisions.md` still say "the VM" after lot 1 renamed the type.
Rewrite the 29 specification sentences and the 77 comments together, with
the sentence Yoann gave as the model: "consumed by the `Execution` of the
`ValidatedExecutionPlan`". `decisions.md` is dated and stays. Owner: Astra.
Lot: before the 23 October freeze, with the documentation touch.

### Connector rework — requested before the freeze, 21 October

**Status: implemented on 22 October (eight commits, `491e397` to `974f85f`)
and reviewed by Fable the same day; see the Applied and Reviewed entries at
the end of this section.** Originally recorded on 21 October as follows. Yoann
asks that the complete Connector proposal discussed with Astra enter the
pre-freeze rework, followed by Fable's review for possible simplifications.
This scheduling supersedes the 17 October deferral of the **T5 layout**
questions; it does not reopen the separate post-opening A-T4-6/A-T4-8 work.
The detailed before/after tables are maintained once in the
[vocabulary ledger](human-review-vocabulary.md#connector-rework--21-october-before-the-freeze).
Owner: Astra. Review: Fable, then Yoann for unresolved choices. No calendar
extension, implementation result or new format number is implied here.

**Protected result.** A contributor can find the standard Connectors, their
exact Contracts and supported modes, the interfaces their Driver must
implement, the Lab substitute and the conformance tests without reading a
mixed execution/Driver module. The DSL author sees a Contract, not the
libraries or provider selected to implement it. The compiler, independent
core validator, runtime and generated documentation must agree on the same
resolved ports, attributes and exchange obligations.

**Agreed direction.** Organize this code under `connector`: physical Drivers
are under hosted `rastok::connector::drivers`, never top-level `drivers`;
standard Connector definitions use `connector::stdlib`, not `standard` or
Rust's `std` module name. The semantic namespace `std/…` is unchanged. Core
exchange data, Driver interfaces and portable validation stay `no_std`;
physical I/O, assembly and descriptive documentation stay hosted. One DSL
Connector name should designate one versioned Contract with checked modes;
several physical implementations may satisfy that Contract. The exact
rename tables and implementation choices below remain reviewable before
coding; no new normative rule is silently written by this ledger.

| Action | Work to do before the freeze | Boundary and acceptance evidence |
|---|---|---|
| **A-T5-2 — coherent modules and names** | Split `boundary.rs` into Connector exchange data, Driver interfaces, standard Contract rules and execution-owned polling/obligations. Place hosted implementations under `connector/drivers/`. Apply the vocabulary table and align live prose with A-T5-1. | Mechanical portion: no changed behavior, identity, package, recording or causal reference. No compatibility aliases. Public API remains small and usable from both crates. |
| **A-T5-3 — one runtime Driver association** | Use `RuntimeDrivers` for the prepared association; consolidate the duplicated outbound Session routing interface. An implementer supplies the relevant per-Driver interfaces, not the global routing table. | Preserve commit-before-effect, admission acknowledgements, backpressure, exact Call/Session ownership, shared duplex state, close-once and no-recall recovery. Prove equivalence before removing the duplicated trait. |
| **A-T5-4 — one Contract per DSL name, checked modes** | Specify the replacement of Process's two exact ids by one Contract with statically resolved job/Call and service/Session modes. Remove the mode's ContractId override only after that specification is reviewed. | This is **not** a rename: `stdlib.md` §1.1/§3.3 currently require `std/process@4` and `std/process@5`. Inventory all identity/artifact consequences before assigning an id or saving any reference; independently validate the resolved mode and preserve its existing lifecycle and Loop/Retry restrictions. |
| **A-T5-5 — several implementations of one Contract** | Separate Contract identity from implementation selection and exchange ownership; let the hosted registry select a compatible implementation from explicit configuration, independently for each planned Connector. | Selection is explicit or uniquely determined, never registration order. Qualify two implementations of one Contract in a local test, correlation and instance isolation, and refusal of absent, ambiguous or incompatible implementations before effects. No cloud Driver or plugin loader is part of this action. |
| **A-T5-6 — documentation from checked definitions** | Expose a standard catalogue and resolved-mode descriptions for humans and LLMs, using the existing Contract/type metadata, concise explanations and executable Lab examples. Update the contributor guide to the new paths and contribution sequence. | Distinguish known Contract, executable mode, Lab support and physical Driver availability. Terminal cases come from resolved output types; generated views do not become a second schema or semantic authority. State the remaining core contribution required for a new Contract honestly. |

**Execution order and review stops.** First review the tables and settle the
remaining implementation questions in the vocabulary ledger. Then do
A-T5-2 with A-T5-1, followed by the interface consolidation A-T5-3. Prepare
and review A-T5-4's specification/identity inventory before implementing its
Contract movement; A-T5-5 must also remove the current assumption that one
ContractId implies one Driver object and one exchange kind. Finish A-T5-6
against the actual qualified interfaces, rather than documenting a target
as already available. Keep byte-neutral changes separate from any
identity-bearing Contract change. If cost or evidence threatens the
freeze, report the remaining action to Yoann; neither silently defer it
nor weaken the gate.

**Qualification required, not executed by this documentation entry.** Run
the four AGENTS.md checks, the relevant Driver/Lab/recovery/conformance
tests and the existing `no_std` and embedded qualification. Keep the
128 KiB heap and 1 MiB code gates and measure before accepting core growth.
For the mechanical changes, all current witnesses must remain identical.
For A-T5-4, enumerate the actual semantic hash, package, recording and
causal-trace consequences before any reference update; a contract-id
change cannot be labelled byte-neutral. No test is ignored or weakened.

**Explicit exclusions.** No S3/Azure/GCP implementation: ObjectStorage is
the long-term substitutability example, not a newly admitted Contract.
No dynamic plugin loader, native ABI, new parallelism, changes to
Event/Call/Session semantics, automatic provider fallback or new Driver
selection syntax is authorized by this entry. No push, tag or publication.

**Fable's review — 21 October, and Yoann's decision.** Fable verified the
six actions against the code (`c57fbd7`) and the specification. Nothing in
A-T5-2, A-T5-3, A-T5-5 or A-T5-6 moves a byte: no Rust type or module
name reaches an identity path (no `type_name`, `stringify!` or
`module_path!`; no `{:?}` in `canonical.rs`, `plan/serialization.rs`,
`journal/codec.rs`, `golden.rs`, `recording/codec.rs`); the public paths of
both crates change, without aliases. A-T5-4 is the identity movement the
ledger already names: the Contract id is key 1 of the connector node in
the plan, hence the semantic hash; D-2026-09-14-1 made the two Process ids
normative; the spike witness `spikes/rp2350-emu/echo.rkp` carries
`std/process@4` in clear; Retry records seal `plan: SemanticHash`, so the
gallery traces of a Retry-and-Process plan move; `contracts.rs` and
`literals.rs` pin hashes of Process plans; the three package goldens are to
be checked by decoding. Complements Fable adds for the implementation:
the core mirror is larger than `boundary.rs` — `ContractShape` in
`plan/validation.rs` (roles, exchange, required and allowed attributes,
ports, one arm per id, plus id-conditioned rules) is the other half, so
`connector/stdlib/` should hold one value per standard Contract that
`validation.rs` consumes, the three existing parity tests proving the
merge; `CompletionCheck` belongs with the poller (its only core
implementor is `PlanCompletionCheck`, its callers are `execution.rs` and
recovery), not with the Driver interfaces; `Supervisor` and
`LifecycleObservation` have no caller in either crate or the spikes and can
go in the same lot; `SessionConnectorBindings` has one implementor and one
use, the delegation inside `RuntimeBindings`' own `OutboundConnectorBindings`
impl, so its removal is a fold, not an equivalence proof; the blanket
`impl<T: CallConnectorDriver> OutboundConnectorBindings for T` is to be
checked for test-only use; `CallProtocol` may be a supertrait of
`OutboundCall` if the two signatures coincide. Fable also proposed folding
`driver::Outbound` into `driver::Driver` (four optional views, five
implementer traits) and deferring A-T5-4 and A-T5-5 past the freeze.
**Yoann's decision (21 October): the rework is taken in full as Astra
recorded it**, actions A-T5-2 to A-T5-6 with their gates and order; Fable's
complements are implementation notes for Astra, the view fusion and the
re-ordering are not taken.

**Review pause and the T9 scope — 21 October, afternoon (Yoann).** The
human review is paused until the Connector rework lands; Yoann resumes his
reading on the reorganized tree. The rework covers T9 as well as T5: the
hosted Drivers implement the renamed interfaces, move under
`connector/drivers/`, and the File Driver becomes the worked example of the
rewritten guide; the network Drivers are moved and renamed, not reworked
inside. Astra runs the series without intermediate validation except one
hard stop, the A-T5-4 identity inventory and specification amendment, which
Yoann validates before any Contract movement; the byte-moving commit is
isolated at the end of the code series, and the series stands without it.
The documentation (README, WHITEPAPER, living architecture documents,
specification, diagrams, contributor guide) lands in the same series, with
the dated reports left intact. Fable's brief to Astra is dated 21 October.
Fable replays the four checks, the full suite, the witness identity and the
`no_std` and QEMU gates on each landing and records the result here.

### Applied — 22 October (Astra), byte-neutral T5/T9 series

| Action | Commit | Applied result |
|---|---|---|
| A-T5-1 | `491e397` | Execution terminology in specification, code comments and UI. |
| A-T5-2 | `55155ca`, `e2d980c` | Shared portable Contract definitions; exchange and Driver interfaces; execution-owned polling; hosted `connector/drivers` modules and split registry/runtime responsibilities. |
| A-T5-3 | `958d0bb` | One outbound routing interface; per-object Call/Session views retained; no global blanket Call-to-routing implementation. |
| A-T5-4, preparation only | `eabdee9` | Decoded identity inventory and proposed amendment; current Process ids and package format unchanged. |
| A-T5-5 | `0b0ad9d` | Named implementations selected per Node, unique-compatible default, exact object ownership and routing reconstruction from a recovered prefix. |
| A-T5-6 | This catalogue/documentation commit | Human/JSON catalogue from checked definitions and manifests, File contributor guide, live-document and diagram updates. |

The final gate records **1,620 passed, 0 failed, 0 ignored in
61 groups**; fmt, strict Clippy and rustdoc pass. Site: 12/12;
browser: 35/35; all three no_std targets and both QEMU workloads pass.
Echo peak heap/code: 111,404/971,716 B. Retry with
recovery: 129,188/1,043,716 B, leaving
1,884 B heap and 4,860 B code.
All 73 protected artifacts remain byte-identical. Commands, intermediate
failures and exact measurements are in
[connector-rework-qualification.md](connector-rework-qualification.md).

**Open, not implemented:** A-T5-4's Process Contract id, proposed decision
and package-version choice require Yoann's approval of
[the identity inventory](connector-rework-identity.md). No identity-moving
commit, compatibility alias, public push, tag or publication is included.
No new Fable review is claimed by this Applied entry; the existing review
and Yoann's 21 October decisions above remain intact.

### Applied — 22 October (Astra), approved Process identity amendment

Yoann approved A-T5-4's inventory (`eabdee9`), `std/process@6`, package
format 22 and the proposed amendment after the neutral series (`3ca0867`).
D-2026-10-22-1 records that decision. This identity commit implements it:
one Contract, with job/Call and service/Session resolved from the shared
mode definition by both the compiler and independent core validator.
Implementation selection still distinguishes the two physical resources;
neither mode gains new ports, codecs or Loop/Retry behavior.

The [identity inventory](connector-rework-identity.md) records measured
old/new values and reasons before the single regeneration pass. Six of
73 protected artifacts move; 67 remain identical. Gauge's body and semantic
hash are unchanged. The only causal trace changes are the two Process
Retry scenarios, whose opening facts bind the originating plan and Contract.
Recording remains version 4. There is no old-id alias or package-21 reader.

The complete Rust suite passes **1,623 tests in 61 groups, zero failures and
zero ignored**; fmt, strict Clippy and rustdoc pass. Site: 12/12; browser:
35/35; three no_std targets and both QEMU workloads pass. Echo peak heap/code:
111,404/971,428 B. Retry with recovery: 129,188/1,043,452 B, leaving
1,884 B heap and 5,124 B code. Commands, intermediate failures and final
measurements are in the [qualification](connector-rework-qualification.md).
This entry supersedes the neutral checkpoint's pending approval, not its
historical measurements. It claims no new Fable replay and authorizes no
push, tag or publication.

### Reviewed — 22 October (Fable)

**Gates reproduced on `974f85f`.** fmt, Clippy `-D warnings` on all targets
and features, rustdoc; 61 groups, 1 623 tests passed, 0 failed, 0 ignored
(baseline 1 598); `site_content` 12/12; the three `no_std` targets
(`thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`,
`riscv32imac-unknown-none-elf`); both QEMU workloads `SPIKE_OK` with the
same measurements as Astra: echo 111 404 B peak heap, 971 428 B code
(`.text` 897 420 + `.rodata` 74 008), semantic hash equal to the CI pin;
Retry with recovery 129 188 B peak heap, 1 043 452 B code, package hash
equal to the commit's. Against A-T5-1 (`491e397`) the code grows by 2 576 B
(echo) and 2 716 B (Retry) while heap is unchanged; the Retry code margin
goes from 7 840 B to 5 124 B. The growth comes from the shared standard
definitions now compiled into the core (A-T5-2, +3 016 B on echo), partly
reclaimed by the later commits. The gates did not move.

**Byte neutrality verified.** Across `491e397..3ca0867` no package, trace,
recording or spike witness changed, and no 64-hex literal appears on one side
only of the diff; the edits to `golden_trace.rs` are renames. The identity
commit `974f85f` moves exactly the six artifacts the inventory names (echo,
jsonl and gauge packages, the spike package, the two `two-tenants` Retry
seals) and the pins the inventory lists; `keyed-retry` has no stored gallery
trace, so its movement is a package hash only, as the inventory's table
states. The spike package carries `std/process@6` in clear; the test
packages were checked by decoding (`package_ledger` example), not by text
search. The public narrative paragraph is verbatim in `README.md`,
`WHITEPAPER.md` and `crates/rastok/README.md` against its source. Both
registers kept every earlier paragraph; Astra added only the Applied entries.

**Structure, as the brief asked.** `boundary` is gone from the core and
`connectors` from the hosted crate; no alias or re-export of an old path
remains; `Supervisor`, `LifecycleObservation` and `SessionConnectorBindings`
are deleted; `ContractShape` has left `plan/validation.rs`, which reads the
same `connector::stdlib` definitions as the compiler; `RuntimeDrivers` owns
Calls, inbound Calls and Sessions by a private implementation index, as the
implementation notes suggested; the hosted registry refuses missing,
unknown, incompatible and ambiguous selections before any factory runs; the
Lab mock factory implements `DriverFactory` and is not re-modelled as a
selectable implementation. The registry file of 3 112 lines became
`registry.rs` (536) and `runtime.rs` (1 252) with their tests apart.

**Findings, none blocking.**

- A-T5-7 (zero bytes, owner Astra): the words derived from the renamed
  types were not renamed with them. Yoann asked for one complete pass on
  22 October; its result is the
  [residue sweep](human-review-vocabulary.md#residue-sweep--22-october-fable-the-finishing-lot-for-astra)
  in the vocabulary ledger: `vm` naming `Execution` values (1 678 lines,
  210 in `src`), `bindings` naming `RuntimeDrivers` values (349 lines) and
  the `*_with_bindings` methods (240 call lines), `NoOutboundBindings`,
  the two `bind` helpers, `ProfileProtocol` and the `profile` accessors and
  locals (about 200 lines), `profile` naming test fixtures in
  `conformance.rs` (56), `decide_inbound`, `MockOutboundConnectorDriver`,
  one `connector_id` parameter; plus a dozen specification and living-document
  sentences. **Applied by Astra in `819a2ff` on 22 October and replayed by
  Fable the same evening**: four checks, 1 623 tests in 61 groups,
  `site_content` 12/12, three `no_std` targets, QEMU echo and Retry (code
  margin 5 116 B, heap 1 884 B), only `.rs` and `.md` files touched, no hash
  literal in the diff, the token-normalized diff reduced to rustfmt moves,
  diagnostic wording and the inventory's renames; the sweep re-run is at
  zero on every listed term. Record:
  [Applied and replayed — 22 October evening](human-review-vocabulary.md#applied-and-replayed--22-october-evening-astra-819a2ff-fable).
  Status: applied, reviewed.
- A-T5-8 (zero bytes, owner to assign): a second layer found at the
  replay and absent from the 22 October inventory, Fable's gap. Five
  `bindings` fields and parameters that hold `*RequestSettings` values
  (`drivers/file`, `drivers/process`, `drivers/network/http_client`, about
  40 lines, proposed `settings`), one panic text in
  `tests/connector_composition.rs`, and eleven sentences where "profile"
  still means a Contract mode: `identity.md`, `delivery-semantics.md`,
  `types.md`, `simulation-semantics.md`, `WHITEPAPER.md` (body, twice),
  `components.md`, `plan-package.md`, `opening-showcase.md`,
  `process-service.md`. The table with lines and proposed wording is in the
  record above. Lot before the freeze or v21: Yoann decides. **Yoann chose
  before the freeze; applied by Astra in `ff0227d` on 22 October evening and
  replayed by Fable on 23 October**: four checks, 1 623 tests in 61 groups,
  `site_content` 12/12, browser 35/35, three `no_std` targets, QEMU output
  identical; the 39 renamed lines are the inventory's and the eleven
  sentences read "mode". The fields are named `request_settings`, not
  `settings`, because `prepare` in `drivers/process/service.rs` already takes
  a `settings` parameter. Record:
  [Applied and replayed — 23 October](human-review-vocabulary.md#applied-and-replayed--23-october-astra-three-commits-fable).
  Status: applied, reviewed.
- A-T2-12 (zero bytes, owner Astra; numbered under T2 because the journal
  protocols own the facts): Loop/Retry naming symmetry, from Yoann's
  22 October question. Facts in the past participle in both protocols
  (`Opened`, `Advanced`, `Admitted`, `Closed`), `loop_view::RunState` →
  `Run`, `check_node_manager` → `check_loop_manager`, `retry_admission_load`
  beside `loop_admission_load`, `is_retry_attribute` beside
  `is_loop_attribute`, the specification's words in `identity.md`, `loop.md`
  and `ui.md`; and, at Yoann's own decision, `E-RETRY-ATTRIBUTE` → `E-RETRY`
  beside `E-LOOP`. **Applied by Astra in `2a1cd11` and `3736817`, replayed
  by Fable on 23 October** with the same gate; QEMU Retry `.rodata` +32 B of
  `Debug` strings, hash and heap unchanged, code margin 5 084 B. Left to
  Yoann: `l:next` against `l:retry` (the spelling is in the Emission recipe,
  so not zero bytes), a `retry.md`, the tense of the UI
  transport labels, the placements and test names listed in the record.
  Of these, `l:next` closed the same day (A-T2-13) and `retry.md` landed
  in `21eae53` on 23 October (the pre-opening lot, see A-T1-12 and the
  23–24 October reading note); the UI tense, placements and test names
  remain for T10. Status: applied, reviewed.
- A-T2-13 (format change, owner Astra; numbered under T2 because the `l:`
  registry lives in the TRANSACTION record's key 19 and in the Emission
  recipe): the lifecycle registry, decided by Yoann on 23 October at midday
  from Fable's reading, closing the `l:next` item A-T2-12 left open. One
  rule: the word after `l:` names the emitting mechanism as the specification
  names it, in one bare word (the handler for `start` and `stop`, the
  Extension for `loop` and `retry`, the event for `timeout`, `reply`, `exit`,
  `evict`, `recover` and `close`); never a command, a fact or a phase;
  `parent_id` carries the cause; the prefix is reserved to the registry.
  `l:next` → `l:loop`, because the reading found it on every Loop manager
  transaction, not on the re-admission alone: the eight `manager_record`
  calls carry Opened, Admitted, Advanced, Closed and Expired, and the
  `LoopExpired` emission id uses it (`loop_manager.rs:734`), where
  `envelope.md:356` and `loop.md:233` say "re-admission" only. `l:closing`
  → `l:close`, the registry's only participle. Bytes 3 and 7 do not move.
  `envelope.md` §3.1 becomes the single authority, with a byte column and
  the `l:retry` row that `retry-c1-identity.md` allocated but no
  specification table carried; the stale "tag 10" (D-2026-08-27-2) and
  "tag 11" (`retry-c1-identity.md`) stay as history and are named by the
  decision. Debugger labels for Loop facts drop their `l:` (`ui/view.rs:1046`
  already spelled `l:close` for `Closed`, a collision with the new word) and
  show `opened`, `admitted`, `advanced`, `closed`, `expired`. The spelling is
  sealed material, so the lot carries package format 23 and D-2026-10-23-1,
  written by Astra; diagnostic recording stays 4. The only artifacts whose
  content moves are the four gallery traces that embed either word
  (`app_server.two_conversations`, `scripted_model.alice_and_bob`,
  `ui_tour.lifetime`, `webhook.duplicate`); the four `.rkp` move by header
  only. Not in the lot: the seven other words, `l:stop`'s two causes, the
  Closing-named Rust identifiers (`instance::Phase::Closing`,
  `ClosingStage`, `instance_closing_id`: the phase, not the trigger), the
  dated documents, `retry.md`, `UiCommand::RetryStep`. Brief handed to Astra
  on 23 October. **Applied by Astra in `0413fdf` the same afternoon and
  replayed by Fable**: four checks, 1 624 tests in 61 groups (one codec test
  added), `site_content` 12/12, three `no_std` targets, browser 35/35,
  QEMU echo and Retry outputs identical to `3736817` with `.rodata` +4 B and
  −4 B; exactly the eight named witnesses move, the traces by one trigger
  field and their seal, the packages by header byte and checksum. Two
  precision remarks on the §3.1 cause column of the Loop row are open for a
  documentation touch. Record:
  [Applied and replayed — 23 October afternoon](human-review-vocabulary.md#applied-and-replayed--23-october-afternoon-astra-0413fdf-fable).
  Status: applied, reviewed.
- Four public names are outside the vocabulary tables and are recorded there:
  `RuntimeDrivers::prepare_with_selection`, `RuntimeDrivers::restore_ownership`,
  `OutboundDrivers::forget_session`, `DriverRecoveryError`. The names are
  apt; `forget_session` is a fourth verb beside `close`, `abort` and
  `release`, on the routing interface only, releasing routing state after a
  terminal commit.
- Design observation, not a defect today: the core keeps a blanket
  `impl<T: OutboundCall> Outbound for T`, so an object implementing
  `OutboundCall` cannot also write its own `Outbound` to expose a Session
  view; a future Call-and-Session object would delegate through an inner
  object. No shipped Driver is in that case.
- For T10, the catalogue's human view embeds raw JSON for interaction,
  cardinality and attribute domains, and the deferred reason for File
  `watch` reads as a fragment; the CLI presentation was not frozen by this
  entry, so this goes to the first-hour review.
- `spec-to-code.md` keeps the retired names only in its dated M1.5 rows,
  consistent with the rule for historical sections.
- On disk only: the emptied `crates/rastok/src/connectors/` directory
  survives the move and is untracked; it can be removed locally.

**Status.** The Connector rework is applied and reviewed. The human review
resumes on this tree. The external brief's and this ledger's line references
anchored on `fedada7` remain Fable's re-anchoring debt.

## Realignment on the Node ontology — decided 2026-10-11

Astra's 11 October proposal, verified by Fable against the code, and
Yoann's decision the same day ([D-2026-10-11-1](../spec/decisions.md)).
As the decision asks, this section separates what is decided, what remains
to qualify and what is submitted to Yoann.

**Decided — an opening condition, not M2 by default.** The plan and the
runtime are brought back to extensions.md §1 (Node = Graph | Executor;
Executor = Operator | Connector) before the opening. Today `plan::Node` is
narrower than the specification's Node; an Executor is an `Owner` row and a
`Node` row pointing at each other, and `OwnerNodeMismatch` checks the
reciprocity; handlers form a third table with an `owner` back-reference,
and `NonOperatorHandler` keeps them on Operators; ports live in `boundary`,
`Owner::ports` and `Node::ports`; the wiring exists as `edges`, `routes`
and `boundary_routes`, and `plan/composition.rs` recomposes the inventory
whenever a nested Graph, a Graph Loop or an `InstanceStop` destination is
present, to check that the three agree. Scope, as the decision states it:
one Node table with a body by nature and handlers inside their Operator;
one typed index and explicit destinations; one reference wiring per Graph
with derived, private execution indexes; execution, read-only inspection
and debugger presentation in separate modules (`CallInspection::direction`
restates its `reference` variant); the specification's words for business
concepts. Not in scope: removing small types for their own sake or
replacing typed indexes with bare integers. No new semantics, and
identities cannot move: `instance.rs` seals an Identity from an `OwnerPath`
and its segments; `envelope.rs`, `canonical.rs` and `journal/codec.rs`
never read a `Slot` or `OwnerSlot`; the semantic hash seals the canonical
HIR. The only `OwnerSlot` outside the plan is `instance::Live::owner`,
live state. Owner: Astra. Review: Fable, then Yoann.

**To qualify — layout and cost.** Heterogeneous table, boxed bodies or
compact side tables, measured against the M2 entry constraint recorded
below (512 B of heap and 7,136 B of code under the gates): the rp2350
spike decodes the package at boot (`pkg_decode`), so routes derived at
load are paid there, where transported routes cost package bytes and no
heap. Peak at load, resident memory and code size, plus the differential
comparison (same admissions, emissions, causal identities, dispositions,
recovery) on the existing goldens, decide the layout, not the current
model's incumbency. One invariant to write before coding: a Graph boundary
port's face is given by the Graph whose wiring names it (outer face in the
parent's wiring, inner face in its own), or the two meanings of
`EdgeEnd::node == None` come back under another name.

**Submitted to Yoann — calendar impact.** Measured surface on `fedada7`:
65 files reference the plan model (`Slot`, `OwnerSlot`, `owners`,
`handlers`, `boundary_routes`, `OwnerKind`), 45 sources and 20 test files.
The densest are `runtime/vm.rs` (103 sites, 6,884 lines),
`plan/validation.rs` (95 sites, 5,032 lines), `plan/annotation_resolution.rs`
(59), `plan/serialization.rs` (52 sites, 4,482 lines), `runtime.rs` (52),
`compiler/lower.rs` (23) and the debugger views (`ui/view.rs`,
`ui/view/explanation.rs`). Precedents on this code with a settled design:
the v16 cutover, including the plan change that removed Graph handler
states and realigned the debugger, landed 5–7 September after the rework;
v19 landed 15–18 September with documentation on the 22nd; the v20
candidate 5–6 October, blessed on the 7th. This work is semantics-free but
broader than any single cutover since v16: it changes the model every
module reads and rewrites the package layout. Fable's estimate, to be
corrected by Astra's own: lot 1 (plan model, serialization, lowering,
validator, VM access paths, tests, witness inventory and bless) six to
eight working days from Monday 13 October, landing 21–23 October at best;
lot 2 (runtime split, inspection and debugger separation, naming) two to
three days after lot 1, since both touch `vm.rs`; one day for Fable to
re-anchor this ledger and the external brief. Median landing of both lots:
around 28 October, inside the external review week. Options, Fable's
recommendation first:

1. Keep 23 October and 5 November as targets, lot 1 then lot 2, with a
   checkpoint on **Monday 20 October**: lot 1 in Fable's review by then,
   or Yoann chooses between options 2 and 3 that day. Ask the external
   reviewer this week whether 2–6 November is possible, so that option 2
   exists.
2. Shift one week: freeze Friday 30 October, external review 2–6 November,
   opening around Thursday 12 November. The reviewer then reads the
   realigned code, which is the point of the decision.
3. Keep the dates; land lot 1 only before the freeze and lot 2 after the
   external review, before the opening. The reviewer reads the new plan
   and the old runtime organization. This narrows the decision, so only
   Yoann can choose it.

Consequences already visible: this is a package movement before
publication, so it takes the next package number and becomes what the
public-format-1 TODO relabels; D-2026-10-07-1's "bless once" is superseded
for the package witnesses, the embedded echo package and the recording pin
(regenerated once, with an inventory), not for traces, whose identities
must not move. D-2026-10-10-1's lot keeps its definition, "the first
post-opening format", whatever its number; nothing from it rides this
movement unless Yoann says so. The line references of this ledger and of
the external brief are anchored on `fedada7` until the realignment lands.

**Where the review feeds it.** The T4 second pass continues on the current
code; its findings name functions and invariants, not positions, so they
survive the move. Block 4 (`plan/composition.rs`, Graph Loop admission) and
the owner/node reciprocity checks are exactly what the realignment makes
structural; reading them with that lens is useful, widening the scope is
not.
Point 5 of the decision, the specification's words for business concepts,
is prepared by the [vocabulary ledger](human-review-vocabulary.md).

**Applied and reviewed — 14–15 October.** Both lots landed on
`realign/node-ontology` and were reviewed by Fable on the landed commits;
the registers record what landed, what was reproduced and what is
submitted to Yoann. Nothing below reopens the decision.

*Lot 1* (`2259fd4` to `54c2b3e`, then `032656a`, `17981f2`, `35d3072`;
complement `05b67a6`). One `plan::Node` table with a `NodeKind` body by
nature, handlers inside their Operator and no `Handler::owner`; one
`NodeIndex`; `Endpoint` and `Destination` in each Graph's reference wiring;
private execution routes derived at validation; queued deliveries carry an
`EdgeIndex` resolved by `runtime::DeliveryRoutes`, never a journal identity.
A-T4-4 is applied as prepare → commit → apply. Layout qualified: boxed
bodies, measured on both Cortex-M33 workloads (echo 36,160 B at load and
111,404 B peak; Retry with recovery 30,684 B and 129,188 B, leaving 1,884 B
of heap and 7,664 B of code under the gates, against the 512 B and 7,136 B
recorded below as the M2 entry constraint). Package 21: body bytes echo
961 → 934, JSONL 1,005 → 979, gauge 249 → 230, spike binary 1,042 → 1,015;
semantic hashes, causal seals and the recording-4 tuple digest unchanged;
the inventory preceded the regeneration (`54c2b3e`). The one failure the
realignment produced was the depth boundary: containing handlers in the
Operator body adds three CBOR levels, so `the_depth_boundaries_are_exact`
refused at level 130 what format 20 accepted at 127. Yoann decided
([D-2026-10-14-1](../spec/decisions.md)) that format 21 fixes its own reader
budget: `canonical_cbor::MAX_NESTING` 256 (root at level 0, consumers
`decode_at`, both `NestingExceeded` checks and `encode_field`),
`expr::MAX_EXPRESSION_DEPTH` 128 (`Parser::descend`, the post-parse
`structural_depth`, `dispatch_validation::tree_at`), `MAX_TYPE_NESTING` 128
and 32 decisions unchanged; no compatibility framing, no 128 + 3. Stack
measured by Fable on QEMU `mps2-an505` with a paint-and-scan probe, then
by Astra with the same method rewritten as the spike's optional
`stack-qualification` feature: `decode_at` costs 240 B per CBOR level;
the complete `plan::decode` of the deepest admissible package (2,193 B,
expression depth 128, serialized level 255) uses 79,168 B of stack and
107,328 B of heap. On this target the heap binds before the stack at 256
levels; the 128 KiB heap gate leaves 23,744 B for that package before any
execution. Fable reproduced on `35d3072`: fmt, Clippy `-D warnings`,
rustdoc, 1,591 tests in 61 groups (0 failed, 0 ignored), the three stack
images byte for byte on the reported figures. Three findings, corrected in
`05b67a6`: the opening checklist and the recording contract still called
format 20 current (now checked by `site_content`); no test reached the
Plan-phase `E-PLAN-DEPTH` from source any more (regression added: `size`
of 119 nested lists under two additions reads back, 120 refuse at Plan with
depth 256 while expression and type depths stay under 128); the Plan
message read as if it allowed the refused value (now names the level and
the exclusive bound). Fable's normal images measure 184 B (echo) and 176 B
(Retry) less code than Astra's; cause not established, gates unaffected,
recorded in the [qualification](node-realignment-qualification.md#fable-review-follow-up--15-october).

*Lot 2* (`ab0ae1c`, `b7670ff`, `78cf345`). Inspection types moved from
`runtime/execution.rs` to the child module `runtime::execution::inspection`
without a new mutation surface; `CallDirectionInspection` dropped because
`CallReferenceInspection` already states it, `SessionDirectionInspection`
kept because a `SessionId` does not say who opened; a regression proves an
owned snapshot neither tracks later steps nor writes the journal. The
hosted `rastok::worker::Worker` owns one `Execution`, its `MemJournal` and
its `RuntimeBindings`, with `run(&mut impl Host)`, `run_until_idle`, `step`,
`shutdown`, `accept_input` and `advance_to` public and the Lab face
crate-private (`settle_action`, `poll`, `poll_inbound`, `advance_once`).
Fable compared the former CLI loop with `Worker::run` and `process_ready`
step by step: same order (time advance, input, at most 64 admissions, ready
work, responses, timeouts, terminal events, outbound polling, report, ready
work, checkpoint, completion rule, 10 ms wait), same error paths including
the recording checkpoint on a processing fault; the Lab's `settle` and
`poll_inbound_world` are folded with the same 64-pass bound and order. The
core is untouched apart from the move; every `Execution` method the Worker
calls predates the lot. Fable reproduced on `78cf345`: fmt, Clippy,
rustdoc, 1,598 tests in 61 groups (0 failed, 0 ignored), the three `no_std`
targets, both embedded images with unchanged heap peaks and Fable's own
code figures of lot 1. Remarks, none blocking: the trait `worker::Host`
reuses the specification's word for the clustered model's machine
([vocabulary ⟂ 7e](human-review-vocabulary.md), to Yoann); `run_until_idle`
is public with no caller besides its tests (keep as the embedder's verb or
narrow until one exists, Yoann); two Lab failure sentences changed wording
with the Worker while `conformance.rs` keeps the former ("the machine did
not settle within 64 boundary rounds", "the boundary round faulted"), no
test pins them; the two qualification reports quote machine paths
(`/Users/…`, the Codex runtime's Playwright, `/opt/homebrew`, `/tmp/…`) to
replace by placeholders before publication; the conformance harness keeps
a third small loop with a wall-clock deadline, outside the "two faces"
target; `Worker::new` takes `Option<SemanticHash>` and lets a Retry plan
without hash fail at admission, the pre-existing `Execution` behavior.
Astra records, in the roadmap, Yoann's 15 October decision to carry the
`LogicalTime(i64)` value type with A-T1-12; Fable did not witness it and
did not rerun the browser suite (35 tests reported by Astra). **24 October:** carried in
`10f91f5` and `9fbe8d2`, replayed by Fable; see A-T1-12.

*Calendar.* Option 1 above was taken: lot 1 landed 14–15 October, lot 2 on
15 October, both before the 20 October checkpoint and the 23 October
freeze. Astra's own estimate, delivered on 14 October in the lot 1
handoff: lot 0 two to three days, lot 1 six to nine days, lot 2 three to
five days plus one review day. The line references of this ledger and of
the external brief are still anchored on `fedada7`; re-anchoring them on
the realigned code is the one-day item owed by Fable, together with the
T4 second pass, blocks 2–7, which now reads the realigned code. **24 October:**
the external brief is re-anchored on `9fbe8d2`; the dated reading evidence
keeps its `fedada7` anchors by the header's rule. The 13 October item "bless
and the missed rename derivatives" was already closed when written here:
`2259fd4` (13 October, Yoann's decision) had replaced `--bless` and
`RASTOK_BLESS` by `simulate --compare <dir> | --save <dir>`, and `beabf68` had
applied the derivatives; recorded as closed on 24 October.

## Implementation follow-up — 2026-10-05, lot 1

The consolidated brief's vocabulary lot lands on `eb3f396` in the commit
**Align identity, journal and deadline vocabulary**, introducing this entry.
[Its report](human-review-lot1.md) records the exact scope and qualification:
1,554 tests pass with no failures or ignored tests, plus fmt, strict Clippy,
rustdoc and the `thumbv7m-none-eabi` core check. This is the Fable review stop
before lot 2. The dated entries above keep the original review evidence,
not the current scheduling of unfinished work.

| Action | Current disposition | Named remainder |
|---|---|---|
| A-T1-4 | Landed in this lot: closed `IdKind`, explicit byte projection and named unknown-byte refusal; fourteen existing recipes, unchanged bytes | Pre-public compaction and the future decoder-domain allocation belong to the cutover |
| A-T2-4 | Landed in this lot: journal nouns, admission pipeline, History record types and structured log event; `CallStatus`; outbound binding names remain directional | No compatibility aliases |
| A-T2-5 | Landed in this lot: typed deadline kinds, dated obligations, `created_at`, sealed ids and unsealed deadlines | Eight allocated kinds, not nine; byte 4 refuses. Compaction waits for the cutover |
| A-T2-7 | Landed in this lot: lossless `journal::codec`, `encode`, `Decoder`, sealed/unsealed field terminology | Retained prefixes, envelopes and observations remain ordinary kept evidence, not an unsealed-field category |
| A-T2-6(3) | Landed in this lot: admission-prefix members follow trait order | A-T2-6(1)/(2), typed timing validation and a single admission fold, remain lot 2 |
| A-T1-5 / A-T1-8 residues | Prior `eb3f396` code spellings verified; remaining UI local and live prose aligned | No repeat of the already-landed instance/canonical module moves |

No payload sharing, role removal, materialization change, `std/` migration
or numerical renumbering is part of this lot. The package remains v19 and
recording v3. The **public-format-1 TODO** is retained above, not implemented
early. Nothing is pushed and no external reviewer is invoked.

## Implementation follow-up — 2026-10-06, lot 2

Fable accepted lot 1 at `8fa9331`: 1,554 passing tests and no remaining
arbitration. Lot 2 is structural only: package v19, recording v3 and all
witness bytes stay unchanged. The v20 cutover has not started.

**A-T1-1 — Share encoded envelope bytes**, landed in `28d9f0d`.
`Data::Encoded` now owns an `Arc<[u8]>`; cloning a journal payload,
delivery or recurrent admission shares those immutable original bytes.
Equality and canonical encoding remain content-based. The pointer-sharing
regression failed before the change and also checks materialization and
survival after the original envelope data is dropped.

`Arc<Value>` was considered but not introduced: materialized trees and the
lazy cache keep their existing ownership. Only the encoded-byte change was
implemented and measured; no `Arc<Value>` prototype or benchmark was run.
The unchanged peak heap below is not evidence about what that additional
change might save.

Cortex-M33 QEMU measurements, release builds from the same tree before and
after this action, using the same three-attempt Retry package:

| Workload | Peak heap, before / after | Code + read-only data, before / after |
|---|---:|---:|
| echo | 112,656 / 112,656 B | 971,468 / 971,132 B |
| Retry, including full recovery | 130,560 / 130,560 B | 1,039,572 / 1,039,380 B |

Both runs report `SPIKE_OK`; the 128 KiB heap and 1 MiB code gates hold.
These workloads use materialized values, so their unchanged peak heap is
expected. The encoded-byte regression proves sharing, not a measured gain on
these workloads. Format check, strict workspace Clippy, rustdoc and
`thumbv7m-none-eabi` passed for this action. At that checkpoint the full
workspace suite was still running; final lot qualification is recorded below.

**A-T2-6(1) — Separate typed deadline validation from decoding**, landed in
`387f39c`. `timing::Error` names the acceptance failures;
only `timing::DecodeError` additionally wraps a canonical-byte error.
The cross-record checks now live in `journal::acceptance`, called by the
live journal, codec and recovery at their existing validation points.
Signed integer encoding/reading is shared with `journal::codec`.
No condition was widened or narrowed and the unsealed field bytes stay fixed.
The obsolete “timer schedule” diagnostic wording is gone from this path.
The journal-focused suite passed 74 tests, including each typed refusal,
and core all-target/all-feature Clippy passed. The hosted recording reader
preserves these typed failures too; its focused regression distinguishes
malformed bytes from deadline and Call acceptance failures. That bridge was
folded into this same action commit after workspace Clippy exposed the
missing exhaustive match, without moving the recording tuple.

**A-T1-10 — Enforce the rustdoc citation convention**, landed in `9ef4865`.
The self-contained `canonical` and `canonical_cbor`
headers already landed in `eb3f396`; they describe CBOR, the byte bijection,
the handwritten codec and the separate tag registry, with short in-text
citations and absolute reference links. No rewrite was needed.
CI now checks all tracked Rust files for relative Markdown links in rustdoc
comments, without a new dependency. Four focused tests cover line and block
docs, inline/reference/HTML links, absolute links and ordinary non-doc text.
The check passes on 208 tracked Rust files.

**A-T2-6(2) — Share the admission-generation fold**, landed in `cdc4350`.
`MemJournal` and `AdmissionPrefix` use the same lifecycle
fold, including active close roots, parent-generation traversal and the
current-versus-successor admission rule. Generation validation remains
before the journal's other lifecycle checks; the index changes only after
all rejection gates, with the committed record. This consolidates an
existing index, not the deferred M2 general journal indexing.
The new regression compares independently specified generations after
each of thirteen records: parent/child starts, inner-first Closing,
successor generations and an unaffected sibling. It also checks that a
rejected parent stop changes neither the journal nor admission queries,
then reconstructs every prefix and the complete live index.
The journal-focused suite passes 75 tests; core strict Clippy passes.

**Lot-1 residue — Keep wire tags separate from enum representation**, landed
in `4dee690`. `IdKind` and `DeadlineKind` no longer
repeat wire numbers as Rust discriminants. Their explicit tag projections
and strict readers are unchanged, as is declaration/sort order. No call
site casts either enum to a wire byte. The existing tests pin all allocated
bytes and reject every other byte; whole-lot qualification reruns them.

### Final lot-2 qualification

Qualified on 2026-10-06, against the lot-1 base `8fa9331`, after the five
scoped commits above. The final standard workspace run, not an aggregate of
partial runs, passes **1,560 tests, zero failed, zero ignored**, in 60 result
groups including the two empty rustdoc groups.

| Check actually executed | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --workspace --all-features` | Passed: 1,560 tests |
| `cargo doc --locked --workspace --no-deps` | Passed |
| `cargo check --locked -p rastok-core --no-default-features --target thumbv7m-none-eabi` | Passed |
| Same core check, `thumbv8m.main-none-eabihf` | Passed |
| Same core check, `riscv32imac-unknown-none-elf` | Passed |
| `node --test tools/ci/check-dco.test.mjs tools/ci/check-rustdoc-links.test.mjs` | Passed: 14 tests |
| Rustdoc-link lint | Passed: 208 tracked Rust files, no relative Markdown link |
| `git diff --check` | Passed |

An auxiliary run of the test executables in four parallel workers hit the
30-second listener-start deadline in
`agent_chat::http_remains_available_while_two_children_talk_and_one_quits`
(`Connection refused`, no application output). Its cause was not established.
The subsequent standard full Cargo run above passed that unchanged test;
no timeout was relaxed and no failure was ignored.

Final Cortex-M33 measurements, with the same workloads and packages as the
baseline, after all lot-2 changes:

| Workload | Peak heap, baseline / final | `.text + .rodata`, baseline / final | Code delta |
|---|---:|---:|---:|
| echo | 112,656 / 112,656 B | 971,468 / 971,024 B | -444 B |
| Retry, including full recovery | 130,560 / 130,560 B | 1,039,572 / 1,039,012 B | -560 B |

Both final runs report `SPIKE_OK`. The Retry workload leaves **512 B**
under the 128 KiB peak-heap gate and **9,564 B** under the 1 MiB code gate.
There is no claimed heap saving or throughput improvement. Echo keeps
67 records and semantic hash
`878a34b77a7ac6500b6452b903f633ba39848c47ed0ac387ce1597f78a785699`;
the Retry package keeps hash
`62d07d3b8e10ea45daf1260d9c3b993fb5046b22edfbb78e66ddaf34e1dce132`
and produces 17 records for one operation, three Calls, sixteen four-byte
frames per channel per attempt.

No golden, fixture, example, `.rkp` or `.trace` changed relative to `8fa9331`;
there was no re-blessing. Package v19 and recording v3 remain unchanged.
The Loop and Retry protocol files being read by the human reviewer, their
Loop fold, and `instance_stop.rs` are untouched by this lot.

**Review stop:** lot 2 is ready for Fable's review; that review has not been
run by Astra. No lot-3 cutover, v20 format change, push, tag or publication
has begun. `Arc<Value>` remains an unimplemented option, not an unfinished
promise of this encoded-byte change.

## Implementation follow-up — 2026-10-06, lot 3 candidate

Yoann qualified lot 2 and opened lot 3 from `3bdd807`. The candidate is
isolated on `rework/v20`. It implements the consolidated cutover, not a
second vocabulary pass. The no-legacy amendment compacts the active
registries before their first public blessing; it introduces no reader,
converter or alias for an earlier package or recording.

| Action | Candidate implementation | Review / release boundary |
|---|---|---|
| A-T1-2 | Four `meta.sys` identity fields; no envelope role, role write/read, or role in the lossless tuple | Changed canonical envelope/record bytes require the single bless |
| A-T1-1b | Raw payload materialization follows the reading port: bytes stay bytes; text requires valid UTF-8, including keys, invocation expressions and Lab observations | Focused non-ASCII and shared-payload regressions; the Loop expiry path is qualified too |
| A-T1-6 | Graph-born DATA is ccbor; external raw bytes cannot impersonate a canonical logical value; metadata is canonical CBOR | Plan/journal readers reject noncanonical logical values; Avro remains a schema import/export projection |
| A-T1-9 | Standard Contract and protected annotation identities use `std/`; other Contract namespaces require a dot | No alias from the former namespace; seven Contract pins change |
| A-T1-7 | Decoder name, derived id, attestation/fingerprint and unavailable-build refusal specified; `IdKind::Decoder` allocated | Decoder attestation/availability implementation remains M2 |
| Pre-public compaction | Envelope/record keys, identity domains, deadlines, Extensions, annotations, lifecycle and value tags have a compact active registry | Package 20 and recording 4 only; format-1 public label remains a pre-publication TODO |

The exact old/new allocations, witness hashes, body sizes, checksums,
causal seals and embedded measurements are recorded in
[the v20 identity inventory](human-review-v20-identity.md). The gauge's
semantic hash stays fixed; its compact package and causal representation
are measured separately rather than described as byte-neutral.

The complete candidate suite has run: **1,543 passed, 25 failed, 0 ignored**
(61 groups). The 25 observed failures are unchanged byte/identity pins,
enumerated by target in the inventory; this is deliberately not a green
release gate before the approved blessing. All 28 gallery scenarios pass
in a separate read-only run. Fmt, strict Clippy, rustdoc, the three `no_std`
targets and both Cortex-M33 workloads pass. The final Retry spike keeps
512 B of heap margin and 6,872 B of code margin. JavaScript projection,
SVG layout and CI helpers pass (32, 16 and 14 tests respectively).
The optional Playwright suite was not run: its local dependency is absent.

No moved witness is accepted merely because it is produced by the new code,
no functional assertion is weakened, and no test timeout is relaxed. The
full gate must be rerun after blessing, including assertions beyond the
currently failing pins. The inventory distinguishes the initial macOS
process-startup failures from the final passing chat/gateway integration run.

**Review stop:** this is not a blessed format or a release qualification.
Fable's review and Yoann's explicit approval must precede the single
re-blessing. No push, merge, tag, publication or reviewer invocation has
been performed.

**A-T2-6 (4) — Deadlines travel inline in the record codec; `timing.rs` keeps
only the types.** Found 2026-10-07 while re-reading `timing.rs` after the
v20 candidate; a miss of the 2 October reading, which had kept "the type and
its codec" together. Verified: `Deadlines::encode` / `decode` have exactly one
production consumer, `codec.rs:252-260`, and that consumer nests them —
record key 41 holds a CBOR byte string that itself contains the map
`{ 41: [created_at, [[kind, id, due_at]…]] }`, the number written twice and
a CBOR inside a CBOR, where every other field (emissions, routes, Loop and
Retry packets) is an inline array. The nesting is the heritage of the former
stand-alone retention field; it is also the only reason `timing::DecodeError`
(`timing.rs:76-104`) exists, duplicating the `Read` / `Invalid` distinction
that `codec::Error` already makes. Target: key 41 becomes the inline array
`[created_at, [[kind, id, due_at]…]]` written and read by `journal::codec`,
whose reader calls `Deadlines::try_new`; `codec::Error::Deadlines(timing::Error)`;
`timing::DecodeError`, `DEADLINES_KEY` and `CALL_CLOSED_AT_KEY` leave
`timing.rs`, which keeps `Deadline`, `Deadlines`, `try_new` and `Error`, with
a header saying "dated obligations accepted with one record"; the recording
bridge `recording/mod.rs:84-88` simplifies; the existing round-trip tests move
to the codec. Moves recording bytes, never a causal seal, so it belongs in the
v20 candidate before its single blessing. Owner: Astra.

**Decision pending Yoann — namespace of the remaining `namespace/name@major`
ids.** A-T1-9 landed for Contracts and decoders, but `SecretProviderId` still
mints `rastok/env@1` (`deployment_config.rs:27`) and tests use `rastok/test@1`;
cost and privacy profiles use `pricing/…` and `privacy/…`. Proposed rule, the
consistency reading of A-T1-9: every id of this grammar minted by the standard
library uses `std/` (`std/env@1`), every other publisher a dotted namespace,
including test fixtures (`test.local/…`); `identity.md` states the rule once
for the whole grammar. The alternative — the rule applies only to ids that
enter the plan hash — must then be written explicitly. Last byte movement
before extend-only, so it is settled in the v20 candidate.

Yoann confirmed the general rule on 7 October: standard-library identities
use `std/`; other qualified identities use dotted publisher namespaces,
including fixtures and deployment profiles. Existing opaque profile-version
syntax (including dates) is unchanged. The candidate follow-up implements
`std/env@1`, `test.local/test@1` and dotted example profile publishers without
aliases. This resolves the decision above; it does not admit a new profile or
alter the graph's causal identity.

**A-T2-5 residue — closed byte enums use the standard Rust idiom.** Decision
(Yoann, 2026-10-07). Lot 2 (`4dee690`) removed the explicit discriminants and
kept a separate `*_tag` projection function per enum, as the brief literally
asked. The idiomatic form is the opposite: `#[repr(u8)]` with the wire value
on each variant, `impl From<Kind> for u8` (`kind as u8`, defined and stable
under `repr(u8)`) and `impl TryFrom<u8> for Kind` with an exhaustive `match`
and a named refusal for every other byte; no `Unknown` variant, no derive
crate for eight arms. Applies to the three registries — `IdKind`,
`DeadlineKind`, `Lifecycle` — and retires `id_kind_tag`, `deadline_kind_tag`
and `lifecycle_tag`. The number lives once, on the variant, where a reader
looks for it. Mechanical; no byte moves. Owner: Astra, in the v20 candidate.

### Candidate complements — 7 October

The three additions above are implemented together before witness replacement.
`timing.rs` owns only dated-obligation types and acceptance errors; the record
codec writes the optional deadline array directly and calls `Deadlines::try_new`
on reading. Its error carries `timing::Error`, without another decoding layer.
**Contact correction:** the lossless record is a positional tuple, not a map
whose outer key is 41. The removed byte string contained that redundant map;
the tuple now contains the array itself. `closed_at` was already inline and
does not change. Key 41 remains the field's name in the identity registry.

The namespace decision applies to provider and profile references as well as
Contracts; their existing reference/version syntax is not widened. All three
byte enums have explicit discriminants, `From` and rejecting `TryFrom`, with
every possible byte exercised. The helpers and the stand-alone timing codec
are gone, without an old-spelling alias. Package/trace identity and the sole
additional recording-byte movement are measured in the final section of the
[v20 inventory](human-review-v20-identity.md) before blessing.

**Documentation sweep — 7 October (Fable): residues of A-T2-4, A-T2-5,
A-T2-7 and of the format-20 renumbering in live prose.** Method: grep over
every tracked text surface except dated evidence (review registers,
decisions, errata, milestone ledgers, bannered historical simulations), each
hit checked against `identity.md` §3 and the three byte enums. Confirmed,
owner Astra, zero bytes, before the 24 October freeze:

1. **Numbers not renumbered.** `delivery-semantics.md:73` (routes "INGRESS
   key 25 / TRANSACTION key 38" → 13 / 25); `execution-model.md:575` (join
   tuple "key 34" → 21) and `:502` ("timer kind 9" → `SessionHandshake` = 8);
   `simulation-semantics.md:367-368` ("key 31 … key 34" → 18 / 21);
   `stdlib.md:206` ("Timer kind 8" → `InboundCallTimeout` = 7);
   `boundary.rs:1945,1954` ("timer kind 5" → `RecurrenceDelay` = 4);
   `journal.rs:554` ("keys 20–28" → 8–16), `:687` ("keys 30–39 and 48–55" →
   17–26 and 34–40), `:567` ("key 24" → 12); `tests/multiport.rs:6` ("key 24"
   → 12, contradicted by its own line 260).
2. **Record kinds.** `delivery-semantics.md` §4 (lines 159, 178, 181, 185,
   200, 309, 313) still says "outbound record" for the EGRESS record while
   its line 73 already says INGRESS/TRANSACTION; rustdoc and user-visible
   strings `journal.rs:1142` ("no outbound record for this call"),
   `boundary.rs:1523,1988,2154`, `vm.rs:1450,4461,5934-5935`,
   `golden.rs:1088`, `tests/golden_trace.rs:363`; `ARCHITECTURE.md:76`.
   Adjectives (`InboundCallId`, `open_outbound_session`, `UnboundOutbound`)
   stay, per A-T2-4.
3. **Deadline vocabulary on the journal side.** `identity.md:341,344,365`,
   `envelope.md:210`, `execution-model.md:361`, `diagnostic-recording.md:46,51`
   ("retained timer schedules", "non-causal retained fields" → unsealed
   deadlines / unsealed fields), `components.md:389`; `journal.rs:131,144,
   560,603,697,1031`, `retry_protocol/fold.rs:94`; "domain tag" → `IdKind`
   in `envelope.rs:114` and `journal.rs:50`; locals and parameters named
   `timers` in `acceptance.rs:50`, `codec.rs:698-850`, `golden.rs:795-800`.
   VM-side "timer" and "scheduler" stay (A-T2-5 b).
4. **Renamed ids still spelled `*Ref`** in spec and examples: `SessionRef`
   (`stdlib.md:458,463,633,648`, `connector-drivers.md:127,235,308`,
   `examples/agent-chat.rkg:123`, `docs/examples/codex-websocket-gateway.rkg:6`),
   `SessionWriteRef` (`connector-drivers.md:235`), `SecretRef`
   (`delivery-semantics.md:204`, `observability.md:115`, `stdlib.md:248`),
   `InboundCallRef` (`simulation-semantics.md:182`), `DeliveryRef`
   (`grammar.md:823`).
5. **"Current" format labels and `rastok/` Contract ids in live docs.**
   `ARCHITECTURE.md:5` (v19), `docs/architecture/README.md:7` (v19 /
   recording v3), `components.md:4,432-436` (v17, `rastok/process@2`…),
   `connector-drivers.md:3,34`, `spec-to-code.md:3` (v18),
   `process-service.md:12-13`, `docs/examples/dsl-examples.md:3,59`,
   `dsl-showcase-complex.rkg:32`, `teams-codex-gateway.rkg:67`,
   `editors/vscode/rastok-dsl/{README.md:1,16,93, package.json:4,
   grammar.test.cjs:42}` (v19), `release-qualification.md:257` ("Current v19
   measurement" below a 7 October "current representation"). Target: the
   current format is stated once (`identity.md` header); every other live
   surface points to it or says "format 20, public format 1 at publication";
   the 7 October embedded figures sit in `release-qualification.md`.
6. **UML sources** `03`, `04`, `05`, `06`, `07` and their SVG carry
   `InboundRecord`/`OutboundRecord`, INBOUND/OUTBOUND, "six-field spine",
   "Tier 1", `CallRef`/`SessionRef`/`SecretRef`/`InstanceRef`; regenerated at
   v18 per `docs/architecture/README.md:148`. Refresh for format 20 or label
   the views as a v18 snapshot.

Decisions (Yoann, 2026-10-07): "arm / armed / arms" leaves the deadline
vocabulary entirely, verb included — a deadline is *created* at
`created_at` (`timing.rs:2,93,107`, `journal.rs:309,561,602,795`,
`identity.md:256,338`, `runtime.rs:1265 armed_budgets`, `retry_manager.rs`,
`recovery.rs:550 expected_arms`, `release-qualification.md:241,249`; about
166 lines outside the `match` sense). "Timer" is reserved for the mechanism
that fires an action after a delay or periodically — the `Timer` Contract,
the host's monotonic timer, the VM's wake-up queue; every mention of a
journal deadline says deadline (`loop.md:207,373,383`, `ui.md:330`,
`simulation-debugger-explanations.md:17,49,143`, locals `timers` in
`runtime.rs`, `vm.rs`, `golden.rs`, `acceptance.rs`, `codec.rs`, "stale /
message / handshake / delivery timer" in `vm.rs:3177-3178`, `recovery.rs`,
tests), and `journal/timing.rs` is renamed after what it holds
(`journal/deadline.rs`, `journal::deadline::{Deadline, Deadlines, Error}`).
The A-T2-1 rule applies to the remaining code types: `InboundExchangeRef` →
`InboundExchangeId`, `OpeningAdmissionRef` → `OpeningAdmissionId`; the
non-identities lose `Ref` too — `PayloadRef` → `PayloadSource`, `PortRef` →
`PortPath` — and `TypeRef` keeps the grammar's own name (`type_ref`,
grammar.md:1545). User-facing "retained" (debugger labels, `docs/site`,
README/WHITEPAPER prose, `diagnostic-recording.md`) becomes "recorded",
including the snapshot field `completion_retained` (`app.js:2130`) without
alias; the GDPR sense of "retention" and code locals stay. Owner: Astra,
one zero-byte lot before the freeze.
Verified clean: no envelope `role`, no
`*_tag` helper, no `timing::DecodeError`, no `journal::retention`, no
non-dotted third-party namespace in code; `LoopDelay` is the Loop `delay`
setting, `Timer`/`Calendar` are Contract names, "retention" in privacy prose
is the GDPR sense.

## Landed changes

### Documentation sweep — 7 October (implementation)

The commit containing this entry closes the seven-point documentation brief
above without another format movement. Live key/kind/domain references were
checked against `identity.md` and the byte enums. Record nouns are
INGRESS/EGRESS; inbound/outbound remain directions. Journal obligations use
deadline/creation/unsealed vocabulary, while timers remain mechanisms and
match arms remain decision branches.

`journal::deadline` replaces `journal::timing`, without an alias. RustRover
performed the requested identity/selector/path renames and the remaining
deadline-helper renames. Moving the module and fixing imports used ordinary
edits; eleven cfg-gated WebSocket test references missed by the IDE were
updated explicitly. `TypeRef` keeps its grammar meaning. The inspection
snapshot now publishes only `completion_recorded`, with a dedicated
Call/Session regression. Displayed English uses recorded; privacy retention
and intentionally retained requests are different concepts and keep their words.

UML 03–07 and all five SVGs were regenerated and visually checked; diagram 05
is a current selected-type view, not the old pre-build inventory. Current
architecture headers point to the format registry, editor metadata no longer
embeds a package version, and the 7 October embedded measurements are labelled
current without presenting them as a new measurement of this documentation pass.
The dated review entries, including Fable's uncommitted sweep, are preserved.

The 48 tracked packages, traces, recording and fixture files were compared
byte-for-byte with the parent. The only permitted difference is the separately
approved AST witness: three `PortRef` names become `PortPath` in `minimal.ast`.
No source program, causal golden, package or recording is re-blessed; the
`.rkg` changes are comments only. The public-format-1 TODO and the human T2
reading at `loop_protocol.rs` remain; this pass authorizes no push or publication.

**Qualification.** `cargo fmt --all -- --check`, strict workspace Clippy
(`--all-targets --all-features -- -D warnings`), `cargo doc --workspace --no-deps`
and `cargo test --workspace --all-features --no-fail-fast` pass: **1,572 tests,
zero failures or ignored tests, 61 result groups**. After the final assertion-wording
corrections, `meta_zones` and `multiport` pass again (**77 tests**), with fmt,
strict Clippy and rustdoc rerun. The layout/editor/link-check unit tests pass
**26 tests**; the rustdoc-link scan checks **210 Rust files**, with no relative
Markdown link. The real-browser suite passes **35 tests**, with no skipped
test or application console error, using the CI's Playwright **1.58.2** and
local Chrome. Initial browser runs exposed an outdated local Playwright/CSP
combination and two stale `rastok/process@4` expectations; the expectations
now name `std/process@4`, without weakening CSP or changing a timeout.
The CLI still reports echo `562396de…`, JSONL `0aeff047…` and gauge `0d3ad43d…`.
No `no_std` target or embedded workload was rerun in this documentation pass;
their qualification remains the separately dated cutover evidence.

**Verified by Fable — 7 October, on `7947b53`.** The gate was rerun in
session: fmt, strict Clippy, rustdoc with `-D warnings` and the full workspace
suite — **1,572 tests in 61 groups, zero failed or ignored**. The sweep above
was replayed on the live surfaces, every live key/kind/domain numeral was
checked against `identity.md` §3 and the byte enums, and the 154 tracked
Markdown files were link-checked. Five zero-byte residues escaped the lot and
are corrected by the commit carrying this paragraph: value tags still numbered
17/18 for struct/map in `expr.rs:84`, `tests/literals.rs:164` and
`tests/multiport.rs:1870` (registry: 15/16); the `02` container view title and
SVG ("package v19"); two dated links in `retry-c1-algebra.md` and
`retry-c1-identity.md` retargeted from the removed `journal/timing.rs` to
`journal/deadline.rs`, their reviewed words kept; the webhook gallery caption
("Armed Retry backoff"); and `parallelism-notes.md:172` (the journal seals
deadlines, not timers). The same commit corrects four stale current-state
facts outside the sweep's surfaces: `CHANGELOG.md` still said "v19 /
recording v3" and 15 October, and `crates/rastok-core/README.md`,
`opening-showcase.md` and `opening-tooling.md` still dated the opening
15 October (5 November since the 5 October amendment). The lot is accepted;
T2 reading resumes at `loop_protocol.rs`.

### Consolidated action disposition — 7 October

The table below supersedes the scheduling and "still open" columns in the
dated notes above. Implementation commits are listed independently of the
single witness blessing, **`18a0d44`**, following the committed final inventory
and complements in **`8bc54fd`**. Its complete qualification is recorded in the
[v20 identity inventory](human-review-v20-identity.md).
The final ordinary workspace run passes 1,571 tests in 61 groups with no
failures or ignored tests. Fmt, strict Clippy, rustdoc, the three `no_std`
targets and both fixed embedded workloads pass as well.

| Action | Landed implementation | Named remainder |
|---|---|---|
| A-T1-1 | `28d9f0d`: immutable encoded payloads share `Arc<[u8]>`, with measured embedded bounds | `Arc<Value>` was considered, not promised; no further payload-sharing work is required by this action |
| A-T1-1b | `2de737a`: materialize raw DATA by the reader type, including UTF-8 refusal and the three-byte `é\n` gallery regression | Its witness is blessed in `18a0d44` |
| A-T1-2 | `2de737a`: remove role from the envelope, canonical form and recording; compact the registry | No role slot or compatibility reader remains |
| A-T1-4 | `8fa9331`, `2de737a`, `8bc54fd`: closed `IdKind`, compact explicit discriminants, standard Rust conversions and exhaustive byte refusals | Decoder attestation itself remains M2 under A-T1-7 |
| A-T1-5 | `f72454a`, `eb3f396`, `8fa9331`: instance/canonical modules, dependency direction and all listed rename residues | No deferred instance-module move remains |
| A-T1-6 | `2de737a`: graph-born ccbor, raw/ccbor distinction and canonical metadata | Avro remains an export/schema projection, not another identity encoding |
| A-T1-7 | `2de737a`: decoder identity and conformance/availability specification; `IdKind::Decoder` allocated | Attested decoder registry and build-availability implementation remain M2, explicitly outside this brief |
| A-T1-8 | `f72454a`, `eb3f396`, `8fa9331`: canonical-form and sys-field vocabulary in code and live prose | Dated review evidence keeps the words it reviewed |
| A-T1-9 | `2de737a`, `8bc54fd`: standard `std/` identities, dotted publishers, provider/profile examples and no aliases | Existing opaque profile-version syntax is unchanged |
| A-T1-10 | `eb3f396`, `9ef4865`: self-contained CBOR headers, absolute reference links and CI lint | None |
| A-T1-11 | `f72454a`, `eb3f396`: consistency/type/bounds checks, `Result` and named exceeded bounds | Published diagnostic codes remain unchanged |
| A-T2-1 | `f72454a`, `eb3f396`: Id/Name/Handle vocabulary and `EdgeEnd` | None |
| A-T2-2(a) | `f72454a`, `8fa9331`, `2de737a`: recurrence deadline vocabulary and compact active allocation | A-T2-2(b), Extension-neutral journal facts, remains M2 design; it is not claimed landed |
| A-T2-3 | `f72454a`: explicit linear-M1 journal lookup/scan limits and atomic-index direction | General journal indexes remain M2; this cutover does not implement them |
| A-T2-4 | `8fa9331`: Ingress/Transaction/Egress records and admission pipeline, directional adjectives preserved | None |
| A-T2-5 | `8fa9331`, `2de737a`, `8bc54fd`: typed deadlines, compact bytes and idiomatic rejecting enum conversions | None |
| A-T2-6(1) | `387f39c`: typed timing errors and journal-owned acceptance checks | The final codec separation is A-T2-6(4) |
| A-T2-6(2) | `cdc4350`: one generation-admission fold for live and prefix replay | None |
| A-T2-6(3) | `8fa9331`: admission implementation follows trait order | None |
| A-T2-6(4) | `8bc54fd`: inline deadline arrays in the record codec; timing owns types and acceptance errors only | `closed_at` was already inline; no causal-seal movement from this complement |
| A-T2-7 | `8fa9331`: `journal::codec` and sealed/unsealed terminology | Ordinary retained evidence remains "retained" as a verb, not a second field category |

**M2 entry constraint.** The final Retry/recovery workload uses 130,560 B of
the 128 KiB heap gate and 1,041,440 B of the 1 MiB code gate: **512 B heap and
7,136 B code remain**. No core growth is accepted without reclaiming bytes
first. The workload, thresholds and functional assertions were not reduced.

The human review itself is not closed by these changes. T2 resumes at
`loop_protocol.rs`, then its fold, Retry and explicit instance closing; T4
remains a separate reading. No external reviewer was invoked. The
**public-format-1 TODO**, 24 October freeze and 5 November opening remain;
no push, tag or publication is authorized by this disposition.

**`f72454a` — Rename identity, canonical-form and check vocabulary
(2026-09-28, Astra with the RustRover rename refactoring, signed by Yoann).**
Implements the pure-rename part of A-T1-4 (`id_kind` parameter), A-T1-5
(`InstanceIdentity`, `InstanceSegment`, `InstanceGeneration`, `scope`),
A-T1-8 (`canonical_*`, code comments), A-T1-11 (`check_consistency`,
`check_value_type` returning `Result`, `check_bounds`, `…Exceeded`/`bound`),
A-T2-1 (`…Id`/`…Name`/`…Handle`, `ids::*_id`, `EdgeEnd`), A-T2-2 (a)
(`RECURRENCE_DELAY`, `SAGA_TIMEOUT` removed) and A-T2-3 (the `MemJournal`
comment). Also aligned the pre-public JSON log keys of the observability
output with the same vocabulary, without legacy aliases (a visible output
change, acceptable before any release; noted here so it is not mistaken
for a rename). Verified by Fable on the commit: no golden, fixture,
example, `.rkp` or `.trace` changed; full gate rerun by Fable on
`f72454a` (2026-09-28): fmt, strict Clippy and rustdoc pass, 1552 tests
pass, 0 fail.
Residual vocabulary left for the v21 naming sweep, none blocking reading:
`TypeRefusal::NodeBudget` (`compiler/check.rs:124`) and
`PlanValidationError::NestingBudgetExceeded { budget }` keep the diagnostic
vocabulary of `E-TYPE-BUDGET`/`E-PLAN-DEPTH`; `DeploymentError::InvalidSecretRef`
→ `InvalidSecretName`; `vm.rs::select_session_ref` → `select_session_id`;
`encoding.rs::map_value_from_tagged` → `map_from_canonical`; two test names
still say "spine" (`machine_executor.rs:109,166`). Still open from those
actions: the `instance` module move (M2), the `IdKind` enum (v20), the
specification prose (v21 docs sweep).

## External Rust review (26–30 October) — priorities

The reviewer reads the frozen branch. Line references in this section are
anchored on `9fbe8d2` (24 October), the tree integrated into `main` for the
review; the earlier `fedada7` anchors were retired with the realignment
(`runtime/vm.rs` is now `runtime/execution.rs` and `runtime/execution/*`,
`Vm` is `Execution`, the Loop and Retry folds are `loop_view.rs` and
`retry_view.rs`). In order of value for the promise
(same plan, same inputs, same execution, identities and trace), with the
question each item should answer:

1. **Same-instant ordering, live and recovered.** The scheduler breaks ties
   with an Execution-local `creation_seq` that is not journaled
   (`execution.rs:816-860`, `Scheduler`). Loop and Retry re-derive their
   dated head from the record order (`retry_manager.rs:369-430`,
   `next_dated_recurrence`); every other deadline kind relies on recovery
   re-scheduling in the same order the live Execution did (`recovery.rs`,
   `route_prepared` at `execution.rs:6237-6301`, `apply_instance_admissions`
   at `:6302-6410`). Question: can two deadlines of different kinds, due at
   the same instant, expire in a different order after recovery, and would
   that change the record sequence? A targeted test with a lifetime and a
   Call timeout due together would settle it.
2. **Facts first, then mutation — every path.** The discipline is explicit
   in `expire` for an open Call (`execution.rs:2267-2651`) and broken once in
   the drain expiry (A-T4-1, open, lot v21). Question: is there any other
   place where a fallible Driver or journal call follows an Execution
   mutation, so a refusal leaves the Execution and the journal disagreeing?
   Candidates: `expire_closing` both arms (`:3406-3580`),
   `expire_session_handshake` (`:2652-2809`), `settle_next` (`:3803-3907`),
   `step_retry_manager` (`retry_manager.rs:431-661`, the commit at `:624`).
3. **Every `unreachable!` and `unwrap_or_else(|| unreachable!)`.**
   `execution.rs` uses them for reserved capacity and planned paths
   (`:2422, :2452, :3812, :3975, :5003, :5119, :5139-5141, :6267, :6281,
   :6330`; the "a literal path is valid" sites parse constant paths).
   Question: is each one proved by a reservation made in the same step, or
   could a recovered or concurrently-mutated state reach it? A panic in the
   core is a correctness bug under AGENTS.md.
4. **Recovery re-derivation has no hidden input.** Recovery recomputes
   routes, contexts, settings and deadlines and compares them with the
   record (`recovery.rs:424-541`, `recovery/retries.rs:75-91`,
   `recovery/calls.rs:85-101`). Question: does any recomputation read
   `self.now`, the civil origin, environment or an unjournaled value, so
   that a correct journal could be refused or a wrong one accepted? Note the
   instant handling at `recovery.rs:67-121`: since format 24 the TRANSACTION
   record carries an observed civil time (key 39) that recovery converts
   back through the journal header's origin, and `preflight`
   (`execution.rs:1126-1145`) refuses an Execution whose origin differs from
   the journal's.
5. **The Loop view's blind spots.** A-T2-9 (`Stopped` without closing
   evidence) and the invariant that every enclosing run named in a manager
   record's context is still active when that record commits
   (`loop_view.rs:260-284` refuses otherwise; producers at
   `loop_manager.rs:193, 244, 510-514, 541-545, 638, 764`). Question: can an
   outer run close by a natural or explicit verdict while an inner run still
   owes a manager record, so the journal refuses the Execution's own record?
6. **Retry: three readers of one rule.** A-T2-11. Question: is there an
   input accepted by the carrier matrix (`retry_protocol/carriers.rs`) and
   refused by the view (`retry_view.rs`), or a completion whose terminal
   case reads differently in `closure_schedule` and `closure_case`
   (`retry_manager.rs:684-752`)?
7. **Arithmetic at the edges.** `u64` ordinals, `LogicalTime` and `Duration`
   as signed i64 nanoseconds, `Timestamp` as signed seconds and a `u32`
   fraction: saturating instant + duration (`time.rs:70-84`, behind every
   due instant in `timing.rs` and `loop_manager.rs:588, 621, 671`), checked
   civil arithmetic (`time.rs:160-195`; `civil_time`, `execution.rs:2851-2857`),
   the modular reduction of the epoch-aligned box (`time_box_elapsed`,
   `timing.rs:15-30`), `entry_measure_budget` (`loop_manager.rs:815-853`),
   the unchecked `+ 1` of A-T4-2, `creation_seq.wrapping_add`
   (`execution.rs:843`). Question: does any saturation or wrap silently
   change a journaled value, and does any civil refusal reach the journal
   after a mutation?
8. **Backpressure accounting.** `check_room_for_routes` and the admission
   loads (`execution.rs:6125-6201`, `retry_manager.rs:335-368`,
   `loop_manager.rs:23-59`) count reservations against
   `WORKER_MAILBOX_CAPACITY` (`execution.rs:816`). Question: can a batch pass
   the pre-commit check and still hit a full mailbox after commit (the
   `unreachable!` at `:6281`)?

Out of scope for that week: tiers T3 and T5–T11, vocabulary, the dated
review evidence, and the M2 index design (A-T4-3). Tests are evidence, not
reading matter; the two views' test modules name the invariants if a brief
is needed.

## Parked remarks (outside the current theme)

### Execution interface — decided 13 October (Yoann)

During realignment lot 1, Yoann approves the following addition to the
rename lot: `Vm` becomes `Execution`, `inject` becomes `accept_input`, and
`admit_ingress` becomes `accept_connector_output`. Related public types
follow (`ExecutionFault`, `ExecutionInspection`), as does the module name
(`runtime::execution`). This supersedes the earlier decision to keep “VM”
below; the hosted `Worker` remains separate.

The two acceptance methods are alternative entry points, not successive
stages. `accept_input` admits an App input; `accept_connector_output`
admits a world-originated proposal from a source Connector. A correlated
outbound Call result still settles its existing exchange through
`poll_drivers`; it is not another root admission. Pending, rejection and
commit-before-accept behavior are unchanged.

The admission sequence is **prepare → commit → apply**. Preparation can
refuse without changing live state. The journal commits the record and its
routes. Applying that committed admission updates instances, exchanges and
mailboxes; it is not a second transaction. `install` is replaced by `apply`
where it names that post-commit update. Graph Loop context maintenance uses
`update`, because it also removes contexts that are no longer live.
Recovery reconstructs the same state if execution stops after commit and
before application. No journal field, identity or execution rule changes.

Status: implemented; the [naming checkpoint](node-realignment-qualification.md#execution-interface-naming-checkpoint)
records the checks actually run. The full lot 1 qualification remains open.
Other constructor, clock and readiness method names are outside this naming
decision.

**Follow-up, 13 October:** Yoann subsequently authorizes finishing those
methods: `boot*` becomes `new*`, `advance` becomes `advance_to`, `now()`
becomes `logical_time()`, and `has_ready_transaction()` becomes
`has_ready_work()`. The DSL and scenario names are unchanged. The precise
mapping and narrow exception for RustRover's ambiguous-name selector are
recorded in the [vocabulary follow-up](human-review-vocabulary.md#execution-methods--follow-up-approved-13-october-yoann).
No clock representation or execution semantics changes.

- **The hosted Worker has no name and no type** (raised 2026-10-11, during
  T4, by Yoann: "le main paraît un peu monolithique"). execution-model.md
  names the Worker that takes one mailbox and processes one transaction
  (`:119`) and says the M1 machine is its one-worker physical case
  (`:143`); the code has it twice, unnamed: `main.rs` `run` (`:397`, about
  185 lines: time source, stdin thread, the advance/inject/drain/pump/drain
  round, recorder checkpoint, log projection, 10 ms idle sleep) with
  `drive_boundary` (`:665`), and the Lab's scenario round (`lab.rs:835-880`)
  with the same sequence and the same 64-round bound. The Lab's interactive
  session already exposes the high-level verbs (`step_transaction`,
  `poll_world`, `settle_current_action`); the CLI has the loop bare. Target,
  for realignment lot 2: a `Worker` type in the `rastok` crate — never in
  the core, which stays loop-free — owning the round with two faces, hosted
  and Lab; host verbs outside the instance-lifecycle vocabulary (`start`,
  `stop`, `close` are spec words): `run`, `round`, `step`, `shutdown`. The
  10 ms idle poll is acceptable for M1 and to revisit for the resident
  service. Same reading, zero bytes, any time: the `Vm` doc comment should
  say which kind of machine it is — state as a projection of the journal,
  each step a transaction proposed to the journal, the host driving. Names
  weighed and set aside for `Vm`: Monitor (a lock to C#/Java readers,
  supervision, hypervisor), Mediator (one role, ESB vocabulary); `Machine`
  is the specification's own word if it is ever renamed; public surfaces
  keep "VM".
  **15 October:** done in lot 2 (`b7670ff`) as `rastok::worker::Worker`,
  owning the `Execution`, the `MemJournal` and the `RuntimeBindings`, with
  the two faces: hosted (`run` over a `worker::Host`, `run_until_idle`,
  `step`, `shutdown`) and Lab (crate-private `settle_action`, `poll`,
  `poll_inbound`, `advance_once`). `round` became the private
  `process_ready`; the public verb in its place is `run_until_idle`, which
  makes "idle is not complete" explicit and tested. The `Execution` doc
  comment now says which machine it is: journal-projected state, each
  step a transaction, the host driving. The 10 ms idle poll stays with the
  CLI's `Host` implementation. Open: the word `Host` (vocabulary ⟂ 7e).
  **23 October:** `worker::Supervision`, `e9492ea`; ⟂ 7e decided by Yoann.

- **Animated PNG instead of GIF for the public tour** (raised 2026-09-25,
  during T1). Premise check: GIF's LZW patents expired in 2004, so the format
  is free; the remaining reasons to switch are technical — APNG keeps 24-bit
  colour and alpha where GIF is limited to 256 colours. Affects
  `docs/site/assets/webhook-tour.gif`, its references in `README.md:58` and
  `docs/site/gallery/04-webhook-to-partner.md:117`, the capture manifest and
  the `site_content.rs:482` check. Decision pending; to settle in T10.

### Surface findings from the compiler tutorials — 24 October (to Yoann)

Not a reading. Fable broke the `rastok new hello` graph twelve ways and ran
`rastok check` and `rastok simulate` on the night build of `9fbe8d2`
(sheet and files in Fable's scratchpad). Exit codes are right (1 on error
or FAIL, 0 otherwise). Seven messages are good: unknown type, misspelled
Node or port in the wiring, deleted wiring, wrong emitted type, unknown name
in an expression, unknown wiring port. The specification lists 82 `E-*`
codes; two `.diag` goldens pin a rendering, both `E-PARSE-EXPECTED`. Items,
by surface:

- **`lab::check_expectation`** (`lab.rs:931-933`, T8): an expectation
  without a label renders its target with `{:?}`, so the first break a
  newcomer makes in the quickstart (`count == 2`) prints `FAIL smoke.roundtrip
  … expect Boundary(Port { name: "out" }): 1 match(es) in a window of 1
  deliveries`. The `rastok new` template has no label. A `Display` for the
  target (`app.out`) and a test on the FAIL line. Fable recommends it before
  the opening; Yoann decides the window.
- **`render::render_all`** (`render.rs:98`, T10): diagnostics are rendered
  in emission order, and expression diagnostics are emitted after the host
  parse. An unterminated string inside a handler body (`out = "Hello }`)
  prints three cascaded `E-PARSE-EXPECTED` at lines 11, 14 and 22 before its
  `E-EXPR: unterminated string` at line 8; the same fault in a Lab
  assignment prints the one right message. Order by source and offset at
  render time, or carry the expression's lexical error in the parse pass.
  After the opening.
- **`check::infer_ports`** (`check.rs:4113`, T7): `on inn` declares an
  inferred input port, as `grammar.md` §"Port naming and default edges"
  allows, so a typo on the trigger surfaces as `E-PORT-TYPE` on
  `out` ("this value is variant") or, without `data`, as "`greet.inn` has
  no effective payload type … found: an unsolved inference variable".
  Conformant; the wording should say the port exists only through this
  handler and name the near declared port. After the opening.
- **Parser, missing colon** (T7): `input in text` is reported at the next
  line, `E-PARSE-EXPECTED` with expected `=` or a wiring arrow and found
  `output`,
  because `text` was read as a bare statement. A hint at the port
  declaration. After the opening.
- **`E-SIM-TARGET` on a typo** (T8): `simulation smoke on apx` answers "a
  simulation closes `app`; sub-graph targets are a deferred capability".
  Name the typo before the deferred capability. After the opening.

### Lab world face — 25 October (Yoann, to the post-opening queue)

From Yoann's diagonal reading of `lab.rs`, judged acceptable for now.

- **A-T8-1** (zero bytes, owner to assign, after the opening): `Simulator`
  depends on one stdlib type. Its field `source_input:
  Rc<RefCell<HttpServerLabInput>>` (`lab.rs:161`) is used at eleven sites
  (`offer`, `is_pending`, `take_fault`); `injection_port` (`lab.rs:846-856`)
  repeats at run time the HttpServer + `ack` + `out` gate that
  `check.rs:1178-1195` already enforces as deferred; and the injection's
  `target.connector()` path is ignored in favour of the single slot.
  `InboundRole` is pulled (`poll_proposal`), so nothing in the Driver traits
  lets a scenario push a scripted arrival or ask whether it is still
  pending: the Lab needed a world-side face and took the concrete one.
  Shape: a Lab-side trait for that face with the three methods already
  used, implemented by `HttpServerLabInput`, held by `node::Path`; the gate
  becomes "a scripted source is registered for this path", and the
  capability moves to the Lab factory that owns the twin
  (`inbound_lab_factories`, `drivers/mod.rs:95`). The mock fault vector
  (`MockFaults`, `lab.rs:49`) is the lesser leak: the simulator depends on
  `Vec<String>`, not on a mock. AGENTS.md asks for a trait only at a real
  second implementation; `check.rs:1180` says "currently HttpServer ack
  only" and the WebSocket server twin already binds without injection, so
  the second implementation is announced. Not before the opening: the tree
  is frozen for the external reviewer.
