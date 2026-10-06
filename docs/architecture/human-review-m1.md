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
lazy cache keep their existing ownership. Neither embedded qualification
workload demonstrates a heap saving from that additional change. No saving
for that unimplemented variant is claimed.

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
`thumbv7m-none-eabi` pass for this action. The full workspace suite is still
running; final lot qualification will be recorded after all actions.

**A-T2-6(1) — Separate typed deadline validation from decoding** (the commit
introducing this paragraph). `timing::Error` names the acceptance failures;
only `timing::DecodeError` additionally wraps a canonical-byte error.
The cross-record checks now live in `journal::acceptance`, called by the
live journal, codec and recovery at their existing validation points.
Signed integer encoding/reading is shared with `journal::codec`.
No condition was widened or narrowed and the unsealed field bytes stay fixed.
The obsolete “timer schedule” diagnostic wording is gone from this path.
The journal-focused suite passes 74 tests, including each typed refusal,
and core all-target/all-feature Clippy passes. Whole-lot qualification
and embedded size measurement follow after the remaining actions.

## Landed changes

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

## Parked remarks (outside the current theme)

- **Animated PNG instead of GIF for the public tour** (raised 2026-09-25,
  during T1). Premise check: GIF's LZW patents expired in 2004, so the format
  is free; the remaining reasons to switch are technical — APNG keeps 24-bit
  colour and alpha where GIF is limited to 256 colours. Affects
  `docs/site/assets/webhook-tour.gif`, its references in `README.md:58` and
  `docs/site/gallery/04-webhook-to-partner.md:117`, the capture manifest and
  the `site_content.rs:482` check. Decision pending; to settle in T10.
