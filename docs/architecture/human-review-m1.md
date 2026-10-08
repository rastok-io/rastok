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
7 October sweep; comment and specification text only. Owner: Astra.

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
deadlines are retained, never sealed. Owner: Astra, with A-T2-8.

### Disposition of the 8 October findings

| Lot | Items |
|---|---|
| Before the 23/24 October freeze, zero bytes | A-T2-8 (three comments, `loop.md` recipe); key-24 precision in `identity.md` |
| v21, after the opening | A-T2-9 (Loop fold learns the closing scope), A-T2-10 (`loop_protocol::Packet::tree`), A-T2-11 (one Retry carrier matrix, one terminal-case reading, `Timing` classification), A-T4-1 (drain cancels before `begin_settlement`), A-T4-2 (one fuel resolution, two cosmetic residues) |
| M2, with the durable journal | A-T4-3 and the `instance_commands` re-fold (A-T2-3), one lifecycle fold for the three protocols (A-T2-2(b)) |

Yoann's 8 October brief dates the real freeze **Friday 23 October**; this
ledger, `ROADMAP.md:59`, `opening-showcase.md:3` and `decisions.md:137` say
24 October, a Saturday. To align in the next documentation touch; the brief
is taken as the intent.

**Still to read in T4** before the external review: `inject_submission`,
`prepare_routes` through `candidate_owner`, `recovery/exchanges.rs`, the
Graph Loop manager (`graph_loop.rs` except its recovery check), and the
`unreachable!`/`unwrap_or_else` sites of `vm.rs` one by one.

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

The reviewer reads the frozen branch. In order of value for the promise
(same plan, same inputs, same execution, identities and trace), with the
question each item should answer:

1. **Same-instant ordering, live and recovered.** The scheduler breaks ties
   with a VM-local `creation_seq` that is not journaled (`vm.rs:916-937`).
   Loop and Retry re-derive their dated head from the record order
   (`retry_manager.rs:350-413`); every other deadline kind relies on
   recovery re-scheduling in the same order the live VM did
   (`recovery.rs`, `route_prepared`, `install_target`). Question: can two
   deadlines of different kinds, due at the same instant, expire in a
   different order after recovery, and would that change the record
   sequence? A targeted test with a lifetime and a Call timeout due together
   would settle it.
2. **Facts first, then mutation — every path.** The discipline is explicit
   in `expire` for an open Call (`vm.rs:2855-2868`) and broken once in the
   drain expiry (A-T4-1). Question: is there any other place where a
   fallible Driver or journal call follows a VM mutation, so a refusal
   leaves the VM and the journal disagreeing? Candidates: `expire_closing`
   both arms, `expire_session_handshake` (`vm.rs:3105-3261`), `settle_next`,
   `step_retry_manager` (`retry_manager.rs:608-640`).
3. **Every `unreachable!` and `unwrap_or_else(|| unreachable!)`.** `vm.rs`
   uses them for reserved capacity and planned paths (`:2894, :2924, :5594,
   :5614-5616, :6739, :6752-6754`). Question: is each one proved by a
   reservation made in the same step, or could a recovered or
   concurrently-mutated state reach it? A panic in the core is a correctness
   bug under AGENTS.md.
4. **Recovery re-derivation has no hidden input.** Recovery recomputes
   routes, settings and deadlines and compares them with the record
   (`recovery.rs:441-521`, `recovery/retries.rs:80-91`,
   `recovery/calls.rs:90-101`). Question: does any recomputation read
   `self.now`, the boot instant, environment or an unjournaled value, so
   that a correct journal could be refused or a wrong one accepted? Note the
   instant handling at `recovery.rs:64-98` and `recovery/retries.rs:74`.
5. **The Loop fold's blind spots.** A-T2-9 (`Stopped` without closing
   evidence) and the invariant that every enclosing run named in a manager
   record's context is still active when that record commits
   (`fold.rs:269-282` refuses otherwise; producers at
   `loop_manager.rs:488-504, 519-532, 615-619`). Question: can an outer run
   close by a natural or explicit verdict while an inner run still owes a
   manager record, so the journal refuses the VM's own record?
6. **Retry: three readers of one rule.** A-T2-11. Question: is there an
   input accepted by the carrier matrix and refused by the fold, or a
   completion whose terminal case reads differently in `closure_schedule`
   and `closure_case`?
7. **Arithmetic at the edges.** `u64` ordinals and `i64` nanosecond instants:
   `saturating_add` on due instants (`vm/timing.rs`, `loop_manager.rs:649`),
   `entry_measure_budget` (`loop_manager.rs:791-828`), the unchecked `+ 1`
   of A-T4-2, `creation_seq.wrapping_add`. Question: does any saturation or
   wrap silently change a journaled value?
8. **Backpressure accounting.** `check_room_for_routes` and
   `recurrence_admission_load` (`vm.rs:6598-6670`, `retry_manager.rs:319-348`,
   `loop_manager.rs:16-49`) count reservations against
   `WORKER_MAILBOX_CAPACITY`. Question: can a batch pass the pre-commit check
   and still hit a full mailbox after commit (the `unreachable!` of item 3)?

Out of scope for that week: tiers T3 and T5–T11, vocabulary, the dated
review evidence, and the M2 index design (A-T4-3). Tests are evidence, not
reading matter; the two folds' test modules name the invariants if a brief
is needed.

## Parked remarks (outside the current theme)

- **Animated PNG instead of GIF for the public tour** (raised 2026-09-25,
  during T1). Premise check: GIF's LZW patents expired in 2004, so the format
  is free; the remaining reasons to switch are technical — APNG keeps 24-bit
  colour and alpha where GIF is limited to 256 colours. Affects
  `docs/site/assets/webhook-tour.gif`, its references in `README.md:58` and
  `docs/site/gallery/04-webhook-to-partner.md:117`, the capture manifest and
  the `site_content.rs:482` check. Decision pending; to settle in T10.
