# Annotation types — integration with the M1 close-out

> **Scope update — D-2026-09-16-7:** M1 retains individual measurements;
> reports, automatic pricing and generic subset arithmetic are out of scope.
> §15 is the reduced M1 scope; §19 is the final two-field schema inventory.
> Earlier dated sum/report and currency/basis evidence is historical, not
> the current signature or a remaining delivery requirement.

## Historical handoff and checkpoints — 15–18 September

The common [C4 qualification](retry-c4-qualification.md) supersedes the dated
candidate status below. The executable signature is exactly `id` / `unit`;
§20 records the completed local integration and the remaining opening gates.
The original handoff follows as historical evidence, not current schema rules.

15 September 2026. **Design handoff with a partial source checkpoint (§6).**
[D-2026-09-15-5](../spec/decisions.md#d-2026-09-15-5--define-flat-annotation-capable-structs-with-one-type-system)
and [annotations.md](../spec/annotations.md) settle the `@struct` model.
[D-2026-09-15-6](../spec/decisions.md#d-2026-09-15-6--hydrate-annotated-response-types-before-retry-disposition)
settles typed response hydration in place of a separate extraction mapping.
The repository contact below is against `58555a7` on `rework/retry`.
The tree was clean before the documentary pass. Subsequent source work is
recorded separately below. The later §4.1/§11 inventory adds application tag 6
to the unblessed v19 candidate; it claims no M1 closure or golden blessing.

## 1. What is settled, and what is not

Settled:

- `type Name = @struct { ... }`, one spelling, LL(1);
- flat fields using the existing logical scalar/enum types and optional forms;
- ordinary data use and annotation eligibility, without a new Value family;
- constant typed annotation arguments and explicit interpretation capability;
- stdlib definitions, protected core meanings and user metadata;
- generic numerical measurements, including exact decimal quantities;
- signed business quantities, including monetary refunds and inventory
  corrections; the declared field type still governs unsigned values
  (D-2026-09-16-6);
- M1 retains facts without aggregating them (D-2026-09-16-7); the existing
  `sum` argument encoding remains metadata, not an executable report policy;
- simple author-chosen measure ids, with no required OTel, `rastok.*` or
  `std.*` prefix (D-2026-09-16-3); quantity names do not confer interpretation;
- the response type itself maps JSON fields; hydration supplies annotated
  values on every attempt before Retry disposition, without an extractor DSL;
- monetary `@measure`, without a separate `@cost` spelling, preserving
  currency, evidence and exact arithmetic (D-2026-09-16-4);
- consumers read validated type metadata; Graph/Executor/Extension semantics
  and all existing privacy/secret guarantees remain unchanged.

The integration must still specify, before the affected code:

1. **Bindings and identity.** How a stdlib definition names its protected
   interpretation, how metadata-only use is explicitly represented, and how
   aliases/qualified declaration identities survive independent validation.
   Shape equality alone cannot authorize a policy.
2. **Arguments and profiles.** Canonical omission/default handling without
   changing ordinary optional-field semantics; a complete stdlib signature
   table; business-id consistency, exact unit representation and supported
   aggregation profiles. D-2026-09-16-2 selects only `sum` for M1; it is not
   yet generic decimal aggregation delivery. D-2026-09-16-3 drops the proposed
   `rastok.business.*` partition and the external-registry naming requirement.
3. **Monetary interpretation.** D-2026-09-16-4 and §10 settle the source
   replacement of `@cost`, scalar versus Money-shaped currency authority,
   sum and reported/calculated evidence. Other monetary ids now use the common
   binding inventoried in §4.1 and implemented in §14; they retain their identity.
4. **Hydration binding.** Subsequently settled by D-2026-09-16-1 and §7:
   explicit output refinement, a bounded HttpClient JSON profile and existing
   DecodeFailed behavior. No general serializer or new extraction mechanism.
5. **Transport.** Append the exact affected tables, tags and canonical hash
   contributions to C1's inventory before changing candidate v19/v3. This
   document allocates no key, semantic domain or Contract major.

These are design obligations, not reasons to reopen the chosen `@struct`
spelling or to implement a generic annotation language/plugin system first.

## 2. Repository contact

| Existing seam | Evidence | Integration consequence |
|---|---|---|
| Logical type/value algebra | [`datatype.rs`](../../crates/rastok-core/src/datatype.rs), `TypeField`, `StructType`, `DataType`; [types §2](../spec/types.md#2-the-logical-type-tree) | Reuse scalar/enum/decimal/struct validation; enforce flatness after aliases, no parallel quantity type syntax |
| Exact JSON numbers | [`json.rs`](../../crates/rastok-core/src/json.rs), `parse_value`; `datatype.rs::narrow_decimal` | The reader already retains exact integers/decimals and checked rescaling exists; hydration should reuse them, not add a parser or pass through floats |
| Type and annotation parser | [`parser.rs`](../../crates/rastok/src/compiler/parser.rs), `type_ref`, `base_type`, `schema_annotations` | Add the disjoint `@ struct` base-type form, retain the postfix application position and named arguments |
| Annotation resolution at the initial contact | [`check.rs`](../../crates/rastok/src/compiler/check.rs), `resolve_data_annotation` | The original five builtin names used closed checks, not stdlib `@struct` declarations; §10 subsequently unifies monetary source syntax without losing target/constant diagnostics |
| Sparse schema policy | [`schema_annotation.rs`](../../crates/rastok-core/src/schema_annotation.rs), `AnnotationTable`, `SchemaPath`, `DataAnnotation` | Reuse canonical target paths and compact resolved policy; a generic source declaration does not require a heap map per runtime value |
| Independent propagation | [`annotation_resolution.rs`](../../crates/rastok-core/src/plan/annotation_resolution.rs) | Keep re-derivation across copy, projection, calculation, state and control dependence; a serialized claim is not trusted |
| Measures | `measure_spec`, `MeasureCarrier` and `MeasureQuantity` | Today the field registry is closed and the fold only admits integer/duration quantities; custom decimal measurements are not delivered |
| Decimal costs | [`accounting.rs`](../../crates/rastok-core/src/accounting.rs), `ExactDecimal`, `Money`, `UnitPrice`, `CostReport` | Reuse exact arithmetic and currency/evidence checks; pricing currently takes an integral quantity and also needs an exact decimal path |
| Attempt observations | `accounting_observations` reads INBOUND and every TRANSACTION emission | Held emissions are already visible to the accounting reader, not only published graph deliveries |
| Dynamic response extraction at initial contact | Historical `agent-http-candidate.rkg`, `operator measure`; replaced by [agent-http.rkg](../../examples/agent-http.rkg) | That downstream Operator did not execute for a held attempt; the replacement hydrates annotated fields on the original Connector emission |
| Retry authority | [`retry-c1-algebra.md`](retry-c1-algebra.md#9-accounting-policy-and-inspection), [C3 §5](retry-c3-implementation.md#5-closing-clarification-and-remaining-qualification) | Keep exact operation/attempt/Call correlation; no observation can release a held result or become a second consumption |

Function names identify the intended seams, not a required public API.
No new `AnnotationInterpreter` trait, third-party dependency or dynamic
registry loader is justified by this contact.

## 3. Typed hydration, not a second extraction language

The [approved response example](../spec/annotations.md#61-hydration-supplies-the-values)
puts `@measure` on the fields of an ordinary response type, including amounts.
The codec hydrates that type on each attempt; the accounting reader observes
the typed fields already retained by the Connector transaction. Process,
File and HttpClient remain generic Contracts, without provider-specific
usage fields or a custom extractor executing business code.

Bind the response schema explicitly in the validated codec profile. Current
HTTP JSON output is `variant`: silently changing it from a downstream port
or a reporting request would violate the fixed Contract. Source syntax and
the versioned profile inventory must make the new binding explicit. No
additional source-to-target path map, field renaming or silent partial-record
projection is needed for the accepted same-shape hydration use case.

The required cases are known zero, unavailable expected value, invalid
present value and no observation required. A timeout can remain unknown even
with hydration; a missing price is different from a missing quantity.
An invalid native Contract payload remains a Contract violation, not a valid
sample manufactured as zero or as a successful decode. The typed codec must
preserve decimal lexemes exactly rather than recover a decimal from a binary
float. Payload conformance and annotation policy are checked before retention,
not by a reporting-only decoder after confidential data reaches the journal.

One response body per Call is not the same profile as a Process JSONL stream
with several usage lines. Incremental versus cumulative values and an exact
job/session association need explicit evidence; do not silently sum both.
The source and profile are retained/identity-bound so replay does not consult
new source, a latest provider schema or current prices.

Accounting identity must distinguish the original observation site from
copies. Existing per-flow rows are not a global consumption total. Attempt
Held, Consumed and Published events must not mint three samples for one
reported quantity; a second downstream decoding point is not another bill.
Independent reported cost and calculated price estimates remain alternatives,
not additive expenses.

## 4. Ordered gates

| Gate | Deliverable | Required evidence before closure |
|---|---|---|
| A0 — this handoff | Chosen syntax, consumer map, repository gaps, explicit pre-code decisions | Documentary review; no executable claim |
| A1 — definitions and authority | Flat type checking, annotation application/alias resolution, stdlib signatures and protected bindings | LL(1) success/recovery/rejection tests; alias-hidden composite refusal; ordinary data equivalence; spoofed semantic binding rejection |
| A2 — measurement retention | Simple declared ids, exact signed samples and canonical monetary metadata | Registry migration; exact original values; unit and basis separation; neutral monetary spelling migration; no duplicate sample or automatic total |
| A3 — response hydration | Explicit typed codec binding, exact materialization before disposition, attempt scope and coverage | Retained 5xx with usage, timeout without usage, final response with usage; exact decimal and conformance refusals; each prefix/replay identical; original Operator not executed; policy-safe retention/display |
| A4 — qualification and migration | Exact transport inventory, source gate opening, updated examples/docs, one reviewed blessing | Full Rust/core/no_std/UI matrix, hostile package checks, causal/identity witnesses, new embedded image and heap measurements |

A1 transport cannot be improvised while A2/A3 introduce more metadata later:
finish their common inventory first, then implement in the order above.
The ordinary source gate stays closed for an unsupported interpretation.
No tag is accepted and then silently ignored to obtain a green CLI check.

The M1 minimum is the existing security policies plus honest per-attempt
numerical/cost evidence for the gateway. Generic annotation execution,
user-written security interpreters, annotation inheritance, meta-annotation
programming, arbitrary dimensions, histograms, external metric registries,
FX, Budget and Saga do not become prerequisites through this decision.

### 4.1 Common binding inventory (16 September, before implementation)

The next slice uses one resolved application beside the existing compact
interpretations, not another type/value algebra or a per-envelope map:

| Authority | Candidate v19 movement |
|---|---|
| Annotation set | Append tag **6**: `[6, binding, definition, argument_type, argument_value]`; tags 1–5 keep their meaning at this checkpoint. §16 subsequently retires tag 3 by name, never reinterprets it |
| Binding | `1` = declared descriptive metadata; `2` = the protected `rastok/measure@2` interpretation; unknown bindings refuse |
| Definition | Descriptive definitions retain their declaring name and flat ordinary `DataType::Struct`; aliases resolve to that declaration. A protected binding requires its exact standard definition and argument schema, never a lookalike |
| Arguments | One ordinary `Value::Struct`, checked against that schema; all fields present. Omitted nullable metadata arguments normalize to explicit null. This does not permit missing fields in ordinary data |
| Measure signature | `id: text`, optional `unit: text`, `currency: text`, `basis: enum { provider_reported, pricebook_calculated }`, `aggregation: enum { sum }`, `subset_of: text`. Missing aggregation normalizes to `sum`; other omissions to null |
| Measure profiles | Quantities require an explicit unit and no currency/basis; money requires basis and its existing scalar/Money currency authority, without a second unit. Id spelling conveys no unit or subset relationship |
| Subsets | The nullable `subset_of` signature field keeps its candidate encoding; non-null generic relationships refuse and arithmetic is outside M1 under D-2026-09-16-7 |
| Compact kernels | Existing sensitive/secret/personal policies remain compact. The standard monetary `amount` form keeps tag 4; it is the same interpretation, not an additional sample. Other monetary ids retain their name in tag 6 |
| Identity | Append tag 6 to sparse `rastok.metering.r1` framing with binding, definition, argument type and canonical value bytes. Empty tables and unused flat data declarations contribute nothing new |
| Validation | Re-derive flatness, complete typed constants, protected definition/schema, supported profile and target. Include argument types/values in the existing package-depth budget |
| Propagation | Exact copies preserve applications. Calculations do not invent their meaning. Alternatives must agree per definition, and two applications of the same definition at one path refuse |
| Storage | Box the uncommon application payload; no additional field on every Value, envelope, journal record or empty annotation table. No new root table, Contract, recording field, domain, or package version |

This inventory includes the later business reader: exact integer/decimal
sums, custom units and monetary ids, and exact pricing without float or
rounding. D-2026-09-16-6 confirms signed business quantities, including monetary
refunds; unsigned field types, currency/evidence separation and non-negative
deployment tariffs keep their existing constraints. Signed aggregation,
pricing and subset handling require explicit integration rather than reusing
unsigned magnitudes silently. No observation can be admitted before
its accounting consumer is implemented. The old closed measure tag remains
the explicit migration baseline until its source/example migration, not a
default interpretation inferred for new ids.

Annotation definitions whose *arguments* themselves carry data annotations
remain a named unsupported application profile: this tuple transports their
ordinary argument schema, not a second nested policy table. Their use as
ordinary annotated data remains valid. No argument policy may be dropped to
admit such an application.

## 5. M1 is still open, independently of this syntax decision

The [C3 evidence](retry-c3-implementation.md) remains a staged checkpoint:
18 focused runtime tests after the mixed-deadline follow-up, not full Retry
closure. Loop/Retry/ordinary-Call deadlines now have a six-order resumption
probe. Remaining work includes other timer combinations, hostile
publication/commit capacity and closure races, accounting, inspection and
gallery migration, then the one explained candidate v19/v3 blessing. The earlier
complete candidate suite
had nine unblessed identity failures; the source slice does not bless them.

Embedded measurement is a hard prerequisite before growing the metadata path.
Before the descriptive-annotation slice, the [C3 footprint follow-up](retry-c3-implementation.md#42-shared-admission-and-bounded-collection-construction-16-september)
reduced Retry/full recovery to 1,040,824 B, 7,752 B below 1 MiB, with
unchanged 130,560 B peak heap. No gate has been raised. This closes the
measured code overflow, not qualification of additional metadata or every
retention profile. Section 11 records the subsequent annotation checkpoint.
Before the response-handoff/hydration follow-ups, the default echo image had
44,844 B of code margin; the extra Retry/full-recovery qualification image
exceeded 1 MiB by 70,924 B. These are different workloads,
not a single green result. The 16-frame-per-channel Retry workload has only
512 B heap margin under 128 KiB. Do not raise either bound or claim arbitrary
annotation tables are free without an explicit decision and new measurements.
The subsequent C3 response-handoff fix was measured separately: its unchanged
healthy heap profile is recorded alongside the still-over-budget Retry image
in [C3 §4](retry-c3-implementation.md#4-measured-cortex-m33-memory-not-an-extrapolation).
The later hydration measurements are retained in §7 below.

The [opening ledger](opening-showcase.md) also retains independent browser
qualification, two unaided quickstart trials, remaining distribution/remote
checks and release authorization. Public push, tags, crate publication and
paid benchmark runs still require Yoann's separate instruction. No external
reviewer is invoked by this handoff.

## 6. First source checkpoint

The `annotation_types` integration tests cover the LL(1) prefix, ordinary
data use, byte-identical plans/hashes against equivalent structs, scalar and
enum aliases, optional fields, preserved field policies, composite/open/cyclic
and duplicate-field refusals, and named staging of annotation applications.
A recovery regression first lost the following declaration; adding `type`
to declaration synchronization repairs that path without parser backtracking.
Protected stdlib names also refuse for user definitions and their aliases.

This is a partial A1, not annotation interpretation delivery. The AST gains
`FlatStruct`; HIR values and the runtime type/plan algebra do not. The existing
AST witnesses do not use the new form and no golden is rewritten. New
annotation uses still refuse by definition name until their validated binding
can be retained. No Connector port or codec changed in this first checkpoint.
Yoann subsequently approved `output out: ModelResponse` on 16 September;
the follow-up inventory is below. The alternative `codec = json(ModelResponse)`
is not an additional spelling.

Focused checks: eight `annotation_types` tests, the 48 existing parser tests,
and six `site_content` tests pass. Formatting, strict workspace Clippy and
rustdoc pass. Retry's separate storage-refusal follow-up is recorded in C3;
these results are not a blessing of candidate v19 or a claim of M1 closure.

## 7. Existing-output JSON hydration

16 September, D-2026-09-16-1. This inventory preceded implementation:

| Authority | Movement |
|---|---|
| Source/Contract sheet | `output out: T` refines an existing HttpClient JSON output; duplicate declarations, input declarations, new ports and non-admitted codecs refuse |
| Resolved port table | Existing `Node.ports` and total resolved pins carry `T`; no new `ConnectorSpec` field or schema lookup by name |
| Annotation table | Existing canonical paths beside that same output carry its M1.8 annotations; no per-value metadata copy |
| Independent validator | Re-derive the exact HttpClient/JSON capability and supported recursive schema; all other Contract pins remain exact |
| JSON materialization | Reuse the exact reader and decimal narrowing; recursively convert object members to canonical struct fields, enum strings to symbols and checked integer signedness |
| Failure | Missing/extra/wrong fields refuse by path without payload values; the physical HTTP codec returns existing `DecodeFailed`, without a partial `out` |
| Lab/native completion | Mocks emit the declared typed value; malformed native typed completions still fail the pre-journal gate |
| Identity | Refined output types/policies already enter canonical plan/hash tables; omission remains `variant`; same-shaped aliases are neutral |
| Transport | Amend candidate `http-client@3`; no key/tag/domain or version allocation, request descriptor and journal unchanged |

Nullable fields require explicit null, as ordinary type conformance does.
The first schema profile has no implicit encoding for bytes, temporal values
or floats, and no discriminator policy for tagged/non-nullable structural
unions; those targets refuse at check. Process JSONL and inbound/Session JSON
refinements remain named unsupported profiles, not silent fallbacks.

This is the typed-value prerequisite, not delivery of generic annotation
bindings, decimal business aggregation, or final Retry/M1 qualification.

Evidence for this bounded slice: five core hydration tests, seven source/plan/
Lab/gate tests and two physical-codec/binding tests pass. They cover member
order, exact decimals and integer bounds, nullable collections, malformed
JSON, missing/extra/wrong fields, unsupported encodings, forged codec pins,
identity-neutral omission/aliases and rejection before journal retention.
The old checker test now says that a Connector cannot add an input, rather
than prohibiting the approved refinement of an existing output; its named
Contract refusal still passes.

The staged Retry regression supplies 100 tokens / USD 0.01 in a held 503
response, then 200 tokens / USD 0.02 in the published 200 response. It observes
exactly two source samples, a public aggregate of 300 / USD 0.03, and only
one delivered response. Every journal prefix restores without re-preparing
or recalling the Driver. No downstream extractor runs. This is evidence for
existing M1.8 measures/costs, not generic decimal business measures.

The identical Cortex-M33 workloads were remeasured after hydration:

| Image | Heap peak | `.text + .rodata` | Code margin against 1 MiB |
|---|---:|---:|---:|
| Reference echo, 67 records | 112,656 B | 1,004,132 B | 44,444 B remaining |
| Retry/full recovery, 3 attempts, 16 frames/channel, 17 records | 130,560 B | 1,120,724 B | **72,148 B over** |

Both execute with `SPIKE_OK`; the Retry image is still not qualified against
the code gate. These Process workloads measure the effect on existing images,
not a maximum-size JSON hydration workload. No bound or golden was changed.

The final combined `cargo test --workspace --all-features --no-fail-fast`
run reports **1,475 passed, 9 failed, 0 ignored**, 52 groups including the two
empty doctest groups. The nine failures are the unblessed candidate changes
enumerated in [C3 §6](retry-c3-implementation.md#6-check-results-with-snapshot-boundaries),
not new hydration failures. Formatting, strict workspace Clippy, rustdoc,
the no-default-feature build and all three configured core no_std targets
pass. No JavaScript/browser or external protocol campaign was rerun.

## 8. Argument checking and standards contact (16 September)

A regression reproduced a malformed `@note(text = 1)` being reported only
as untransported metadata. Declared flat applications now check supplied
constants against the resolved ordinary `DataType` before that gate. They
name the offending argument and target path, without echoing its value.
Text escapes and scalar constructors use the existing constant evaluator;
decimal scale/precision and integer ranges remain logical-type checks.
Aliases follow the existing resolved declaration. This adds no core Value,
transport key, interpreter capability or semantic hash contribution.

The valid application still refuses at the binding gate. In particular this
slice neither grants user definitions protected meanings nor settles omission
defaults; nullable arguments that are absent are not synthesized. It does not
claim A1 complete or open generic business measurements.

Repository contact corrected two test assumptions: source currently exposes
`int`, `long` and `uint` through `parser.rs::simple_scalar`, not the logical
algebra's complete `int8`/`uint8` spelling set described by types §2. The
regressions use the executable `int` 32-bit boundary and unsigned `uint`;
this annotation slice does not broaden the scalar source grammar.

Qualification of this source-only follow-up: the ten `annotation_types`
integration tests and eleven existing annotation-filtered library tests pass.
Formatting, strict workspace Clippy and rustdoc pass. The first regression
failed on the generic binding diagnostic as intended before the fix; the
width-spelling assumptions above were then corrected in the fixtures. No
full workspace test, no_std image, embedded footprint or browser campaign was
rerun for this checker-only follow-up. The previous candidate qualification
and its nine unblessed identity failures remain separate.

Yoann's standards-first question also prompted an upstream check. OTel's
[cost proposal #443](https://github.com/open-telemetry/semantic-conventions-genai/pull/443)
was **open and unmerged** at head
`efc0f96f444397ab976a3987b6cf4db16fd7687b`, checked on 16 September. Its current
proposal is inference-only `gen_ai.usage.cost.amount` (double), `.currency`
(ISO 4217) and `.source` (`provider | local`), plus the histogram
`gen_ai.client.operation.cost`. These are not adopted registry entries and
are not a generic exact business-cost interpretation. The [published GenAI
metric list](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-metrics.md)
contains no cost instrument at this checkpoint. No automatic registry
update or adoption of a proposed name follows from this check.

For count units, the [OTel instrument-unit convention](https://opentelemetry.io/docs/specs/semconv/general/metrics/#instrument-units)
uses brace annotations such as `{item}`, not a bare `item` UCUM unit.
This is upstream contact, not a Rastok naming requirement: Yoann subsequently
chose not to adopt OTel vocabulary (§9). Exact retained decimal accounting
must not be replaced with a double.

## 9. Simple names replace standards-first naming (16 September)

D-2026-09-16-3 settles the naming question: no mandatory prefix, and authors
choose their measure ids. `amount` and `tokens` name quantities; `currency`
and `provider` are context. Their spelling alone grants no interpretation,
hydration, dynamic labels or aggregation. The annotation binding, ordinary
numeric type, explicit unit and sum-only profile remain the authorities.

Repository contact: `MeasureId::new` already accepts unprefixed lexical names,
but `schema_annotation.rs::measure_spec` still enforces the closed M1.8
registry. Source acceptance therefore requires more than deleting a prefix.
The common inventory must account for replacement bindings, token subset
relations, price-profile references and annotated-program identities. Existing
hydration and accounting evidence in §7 uses the old ids; it is not evidence
that new business ids are executable. No source, format or golden changes in
this documentary follow-up.

## 10. One monetary source spelling (16 September)

D-2026-09-16-4 removes `@cost` in favor of
`@measure(id = amount, currency = USD, basis = provider_reported)`. A
Money-shaped struct omits the annotation currency; `aggregation = sum` is
optional and normalizes away. `unit` and other aggregations refuse. The
retired spelling gives a migration diagnostic, including when a declaration
tries to reclaim it. No generic metadata binding is inferred from its shape.

Inventory before this source-only implementation:

| Authority | Movement |
|---|---|
| Source checker | Monetary arguments of `@measure` resolve the existing monetary interpretation; `@cost` refuses with the replacement |
| HIR / core policy | Unchanged `CostAnnotation` holds currency and basis; it denotes the monetary interpretation, not a second source annotation |
| Plan / semantic hash | Existing monetary tag 4 and its exact fields unchanged; omitted/explicit sum and migrated spelling have identical meaning |
| Journal / accounting / recording | Unchanged typed evidence, currency separation, per-attempt counting and replay |
| Examples / tests | Migrate executable monetary sites; pin an old-spelling witness before removal and prove the replacement unchanged |
| Generic binding gate | A custom monetary id must remain named-deferred, never lose its identity in the compact amount representation |

This is a neutral source migration inside the current candidate, not an
allocation for A1's generic metadata table or A2's decimal business sums.

The first regression rejected the new form with `unexpected argument basis`.
Before removal, the old spelling's relay witness measured semantic hash
`490b9e2bd295b8c438cb533665aa4bbd431f42d23a3084123ed3b096420f6cf6`
and BLAKE3 of the complete candidate package
`6ca1648297851e06dbd678bee3ddf0ad6f3ae79a727cce5ac8a410615751535a`.
The replacement matches both; independent decode and explicit/implicit sum
also match. Six focused monetary tests cover that neutrality, currency and
basis authority, malformed arguments, the retirement diagnostic and refusal
to collapse a custom id. Executable monetary sites are migrated, without
rewriting any golden.

The ten flat-annotation tests, seven typed-response tests, two accounting
tests, 103 checker tests and the retained-HTTP-attempt accounting regression
pass: **129 focused tests, zero failures**. The latter still observes exactly
100 + 200 tokens and USD 0.01 + 0.02, with no extra sample on Held/Consumed/
Published and identical reports after prefix recovery. Formatting, strict
all-target/all-feature Clippy and rustdoc pass. No full-workspace, embedded,
no_std or browser campaign was rerun for this source-only migration; the
candidate's earlier nine unblessed identity failures are not claimed closed.

The six site-content checks also pass, including the README quickstart copy
and gallery references (**135 focused tests in total**). The real CLI runs all
six `webhook-to-partner` scenarios successfully with the published synthetic
deployment fixture, without `--golden` or `--bless`. This confirms execution
of the migrated example; it does not bless the candidate gallery traces.

## 11. Declared descriptive applications (16 September)

The transport regression first failed with the previous named binding gate.
It now compiles, encodes and independently decodes `@note(...)` declared by
`type note = @struct { ... }`. The common inventory in §4.1 preceded this
code. Only binding 1 is admitted at this checkpoint; the future measure
binding is not accepted then ignored.

The source checker resolves declaration identity before structural alias
collapse. Typed arguments use existing scalar/enum/decimal constructors;
nullable omissions become explicit typed nulls. Core repeats flatness and
value conformance, rejects protected names and unsupported interpretation tags,
and checks that one declaring name has one schema throughout the plan.
Argument type/value nesting joins the existing package-depth budget. The
uncommon payload is boxed, and there is no global table or per-envelope field.

Seven new source/plan tests cover alias and omission neutrality, distinct
lookalikes, argument/declaration order, changed-value identity, exact-copy
retention without personal-data authority, duplicate/conflicting metadata,
fail-closed argument policies (including self-reference), and unused-definition
neutrality. Three core tests cover forged binding/name/schema/value tuples,
exact depth accounting and inconsistent same-name definitions across plan sites.
The ten earlier flat-type tests now assert retained applications rather than
the superseded staging refusal.

The full core suite passes **367 tests**; the five focused hosted suites pass
**32 tests** (declared annotations 7, flat types 10, monetary source 6, typed
responses 7, accounting 2). The monetary amount witness still has the exact
hash/package bytes recorded in §10. No golden is rewritten.

The final `cargo test --workspace --all-features --no-fail-fast -- --quiet`
campaign reports **1,497 passed, 9 failed, 0 ignored**. These are the same
candidate identity failures, not a green workspace: the library's semantic
hash and recording-byte witnesses; two Contract identity expectations; the
gallery golden; gauge's package; JSONL's hash and package; and echo's package.
They remain visible in six failing targets. This slice adds no blessing or
ignored test to conceal them.

`cargo fmt --all -- --check`, strict workspace/all-target/all-feature Clippy,
`cargo doc --workspace --no-deps`, and core checks with no default features
for `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf` pass. `git diff --check` is clean. No browser or
UI test campaign was rerun for this compiler/core change.

The same fixed Retry/full-recovery Cortex-M33 package, three Calls and 16
frames per channel, measures `.text` **963,692 B** + `.rodata` **82,752 B** =
**1,046,444 B**: 2,132 B remain under 1 MiB. Peak heap remains **130,560 B**,
512 B below 128 KiB, with 17 records and `SPIKE_OK`. These figures measure the
added decoder/validator on the existing qualification workload, **not** a
maximum-size metadata table. Subsequent business bindings still require their
own footprint proof. No memory/code gate was raised.

Generic measure normalization, decimal business aggregation, the final stdlib
binding migration and A4 remain open. Descriptive metadata has no accounting
power and does not substitute for that work. The later D-2026-09-16-6 settles
the sign question; this descriptive checkpoint does not implement a generic
signed business fold.

## 12. Signed quantities and monetary refunds (16 September)

Yoann confirms signed quantities, explicitly including refunds. This applies
to exact decimal amounts as well as non-monetary business corrections, without
an annotation-level signedness flag. The field type remains authoritative.
Both individual observations and zero/negative net totals must be retained;
missing evidence is not a known zero.

Repository contact corrects the assumption that money was restricted to
non-negative amounts: `ExactDecimal` carries `i128`, `Money::new` validates
precision/scale/currency without rejecting the sign, and `money_from` retains
it for scalar and Money-shaped observations. Non-negative `UnitPrice` is a
separate deployment tariff constraint. The unfinished generic measure fold,
not the existing monetary fold, still has the unsigned `MeasureQuantity`
limitation. No runtime, plan, journal or golden change is needed to preserve
already-supported monetary refunds.

The source-to-VM regression
`monetary_measure::signed_refunds_sum_exactly_without_discarding_original_samples`
covers both scalar and Money-shaped values. EUR 12.50 followed by refunds
of EUR -5.00, -12.50 and -20.00 gives respectively EUR 7.50, 0 and -7.50.
Each pair keeps distinct emission identities and provider-reported evidence;
one exact report row folds the shared flow point/currency/basis. The decoded
plan and source-compiled plan produce the same report from the retained
records. These six cases pass without a production-code or format change.

The monetary, accounting and typed-response suites pass **16 focused tests**;
formatting, strict workspace/all-target/all-feature Clippy, rustdoc and
`git diff --check` also pass. The full workspace, no_std and embedded campaigns
were not repeated for this specification/test-only follow-up; §11 remains the
latest full-campaign evidence, including its nine unblessed failures.

## 13. Additive business binding — implementation inventory (16 September)

This slice opens the quantity profile of the already inventoried binding 2,
`rastok/measure@2`: an explicit id and unit, signed integer or exact decimal
target, and sum only. The complete ordinary argument struct from §4.1 is
retained, including explicit nulls and normalized `aggregation = sum`.
Units are opaque, nonempty text of at most 255 UTF-8 bytes, without leading or
trailing whitespace or control characters; there is no unit conversion.
One id has one interpretation and unit throughout a validated plan. An old
registry binding and a generic binding cannot silently assign different
meanings to the same id.

The reader uses the existing exact decimal algebra for generic quantities;
the field's integer signedness and decimal bounds still govern source data.
Sums and deployment pricing are exact and checked, with signed unpriced
remainders. A known zero stays in the measure report. Price-profile ids may
reference these declared quantities; profiles still grant no observation or
annotation authority. Tariffs remain non-negative and currency/basis separate.

The generic `subset_of` profile stays named-deferred until its per-observation
validation and signed pricing rules are integrated. It is not accepted then
ignored. The old registry's inclusive-subtotal meaning remains unchanged.
Custom monetary ids likewise remain behind their existing named gate; the
compact `amount` profile stays byte-identical. These are staged consumers,
not a change to the approved target.

No additional format key, domain, Contract, recording field or version is
allocated. Binding 2 is accepted only together with its validator, propagation,
accounting and exact report projection. New tests must cover the source/plan/
journal path, forged bindings, conflicting units, overflow, signed pricing
and original retained Retry attempts. Footprint gates are unchanged.

The initial source/plan/journal tests pass for positive, zero and negative
integer totals, exact decimal quantities, absent units, unsupported profiles,
inconsistent id/unit definitions, overflow and signed deployment pricing.
Canonical argument order and omitted/explicit sum agree. A held HTTP attempt
contributes 1.25 kg and the published attempt -3.5 kg: two original samples,
one -2.25 kg aggregate and the same result at every recovered prefix, without
re-preparing or starting the Driver.

A regression also exposed two `@measure` profiles coexisting on one decimal
path through their different internal compact encodings. Money and quantity
now share the annotation's per-path uniqueness. Across paths the same id
cannot switch from money to an unrelated unit; currency and evidence remain
separate monetary dimensions. The compact monetary identity witness remains
the required unchanged proof. The untyped wire-shape test moves its money
annotation to a different path instead of constructing an invalid double
application; it changes no package golden.

Qualification on the final production code: **1,509 passed, 9 failed,
0 ignored** across all 53 Cargo-built workspace/all-feature test targets.
They were executed with at most six targets in parallel after the sequential
campaign was interrupted because executable startup was taking several
minutes. Both workspace doctest targets also complete, with zero doctests.
The nine failures are exactly the candidate identity witnesses listed in §11;
there is no additional functional failure and no versioned package or trace
golden is rewritten.

This includes all **369 core tests**, seven business-quantity tests, seven
monetary tests, seven typed-JSON tests and 19 Retry runtime tests. The monetary
hash/package witness from §10 remains identical. The UI projection regression
checks that `-1.250 kg` is rendered as the exact text `-1.25`, not an unsigned
number or a binary-float approximation. No JavaScript or browser campaign was
rerun for this projection-only change.

Formatting, strict workspace/all-target/all-feature Clippy, rustdoc,
`git diff --check` and core checks without default features on
`thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf` pass.

The unchanged fixed Retry/full-recovery Cortex-M33 workload now measures
`.text` **964,612 B** + `.rodata` **83,620 B** = **1,048,232 B**, leaving only
**344 B** under 1 MiB. Peak heap remains **130,560 B**, 512 B below 128 KiB;
QEMU retains 17 records and reports `SPIKE_OK`. Allocation-free checking of
the standard argument schema, shared map specializations and in-place
canonical annotation sorting kept this workload under the existing gates.
This is still an unannotated workload: it measures decoder/validator growth,
not a maximum-size business table or an embedded accounting-query workload.
That additional footprint qualification, the remaining bindings/migration and
A4 are open. This checkpoint does not bless v19 or close M1.

## 14. Named monetary quantities (16 September)

This slice opens the monetary profile of the already inventoried binding 2,
without another tuple, version, Contract or causal domain. A scalar such as
`decimal(38,18) @measure(id = refund, currency = EUR, basis = provider_reported)`
retains its id and ordinary typed arguments. Money-shaped values omit the
currency argument and retain sole currency authority in the value.
The standard `amount` form keeps compact tag 4; forging an alternative generic
monetary encoding of that id refuses. Omitted/explicit sum and argument order
remain identity-neutral, while changing the id changes semantic identity.

The common currency/basis/target checker serves both representations.
`CostSample` and `CostReportRow` now expose the monetary id; it is part of the
report grouping key, independent of currency, evidence and `calculated_from`.
Thus invoice EUR 12.50 and refund EUR -12.50 do not collapse into an invented
zero total. Two corrections under the same id still add exactly, including
zero and negative nets. A tariff calculation continues to produce `amount`
with its source quantity retained separately. The UI projects the same public
report, including the authored id, without another fold.

Focused evidence covers scalar/Money carriers, nominal identity, sum
normalization, independent decode, forged currency/target/profile refusals,
monetary-vs-quantity conflicts, distinct currencies and original samples.
The new Retry test retains a EUR -1.25 refund from the held attempt and
EUR -3.50 from the published attempt: exactly two samples and EUR -4.75,
including at every journal-prefix recovery with no new Driver start or
request preparation. No sample is manufactured for Held/Consumed/Published.

Before opening the reader, simplifying the definition-table walk saved
**576 B** in the fixed embedded image, preserving site order and all checks.
Read-only target validation now borrows the type and annotation table instead
of cloning them just to discard the validated pair. With the monetary profile,
the unchanged Retry/full-recovery workload measures `.text` **964,620 B** +
`.rodata` **83,724 B** = **1,048,344 B**: only **232 B** below 1 MiB.
Peak heap remains **130,560 B**, with the same 17 records, semantic hash and
`SPIKE_OK`. Neither threshold changed. Maximum annotation-table retention and
linked embedded accounting queries are still separate, unqualified workloads.

Qualification is in progress. The remaining generic-subset semantics, complete
registry/example migration and A4 are not closed by this monetary profile.
No golden has been blessed and Retry's source gate remains closed.
The 15 JavaScript graph-layout tests pass. The optional real-browser suite
cannot start in this environment (`Cannot find module 'playwright'`); its
absence is not a browser pass. The changed report projection is covered by
the Rust UI regression above, with no JavaScript change in this slice.

## 15. Retention without reports

D-2026-09-16-7 narrows the remaining M1 gate to individual measurement
retention. No aggregation, pricing, subset policy or debugger aggregate is
introduced. Earlier §§12–14 sum/report results remain historical evidence,
not requirements to restore their removed APIs.

The implementation removes `CostReport`, report rows, automatic tariff
application, subset arithmetic and `unpriced` results. It preserves exact
sample readers, typed hydration, annotation definitions and argument
encodings, deployment attestations and original journaled values. Existing
profile metadata can supply provenance/evidence, but is not a pricing engine.
No package/Contract/recording field, version, runtime command or golden changes
as a consequence of this reduction.

The UI no longer computes or serializes `cost_report`, including diagnostic
recording inspection. Its report panel is gone. Policy-aware original DATA,
META, instances, Calls and retained transitions remain.

The replacement qualification checks facts instead of totals:

- distinct positive, zero and negative original samples, ids, units, currency
  and evidence; no implicit conversion of a decimal quantity into money;
- readable independent values even when their sum would exceed the decimal
  domain, and no arithmetic enforcement of retained subset metadata;
- a deployment tariff does not manufacture a charge;
- typed token/amount, signed quantity and refund responses in a held attempt
  and in its published successor; exact attempt ordinals and batch membership;
- each original emission id occurs once; consuming a Held batch emits no
  second sample; recording encode/decode and every recovered/inspected prefix
  retain the original records without calling or preparing a Driver;
- absence of a report in Rust snapshots and in the flagship browser fixture.

The snapshot regression first failed because the old `cost_report` field
was present, then passed after removal. The core/Retry production journal and
state-machine paths are unchanged by this pass.

### Qualification — 17 September

The focused retention matrix passes **51 tests**: accounting (3), business
quantities (7), monetary quantities (9), typed JSON responses (7), Retry
runtime (20), the classified webhook (4), and the report-free snapshot (1).
The complete `cargo test --workspace --all-features --no-fail-fast` run has
**1,504 passed, 9 failed, 0 ignored**, across 55 groups. All nine failures are
the already inventoried candidate movements: the host semantic-hash witness,
echo recording bytes, two network Contract expectations, the HTTP-500 gallery
golden, gauge package, JSONL package/hash, and echo package. No expected value
or golden was changed to remove a failure. The workspace is not yet green.

Workspace all-target/all-feature checking, strict Clippy, formatting, rustdoc,
the no-default-feature build and all three configured core no_std targets
pass. JavaScript syntax, 15 layout tests and 3 editor grammar tests pass.
The two changed architecture diagrams were regenerated and their SVGs pass
XML validation.

The browser fixture now checks the absent report in LIVE/HISTORY/REPLAY, but
could not run: its installed-tool lookup returns `Cannot find module
'playwright'`. The Browser skill also reports no available browser backend.
Neither result is a visual pass. Existing policy-aware Rust snapshots cover
every flagship transition, not DOM geometry or browser interaction.

The unchanged Retry/full-recovery Cortex-M33 workload still reports 17 records,
the same semantic hash, and `SPIKE_OK`. Peak heap is **130,560 B**, leaving
**512 B** under 128 KiB. `.text` **964,620 B** + `.rodata` **83,724 B** totals
**1,048,344 B**, leaving **232 B** under 1 MiB. Report removal saves nothing
in this image because those queries were not linked. Maximum annotation-table
retention and linked individual-inspection workloads remain separate gates.
Neither threshold is raised. Local command transcripts are retained under
`/tmp/rastok-retention.lZJbj4/`.

The later [Retry scheduling qualification](retry-c3-implementation.md#8-mixed-timers-bounded-publication-and-exact-instances--17-september)
corrects same-instant recovery ordering and extends the runtime matrix. Its
updated footprint is 1,048,368 B of code (208 B margin), with unchanged peak
heap. The numbers above remain the report-removal checkpoint, not that later
measurement.

### Remaining M1 work

Reports, generic subsets and automatic pricing are **removed**, not unfinished
M1 tasks. Still open: complete annotation stdlib/registry migration and A4;
the remaining [C4 inspection and example migration](retry-c4-inspection.md)
and the single reviewed v19/v3 blessing; the typed HTTP gateway/gallery
migration; final workspace, footprint, browser/CI and release qualification.
C3's matrix and C4's public Retry admission are qualified in their linked
reports. M2 durable storage is unaffected.

## 16. Explicit-unit migration inventory — 17 September, before implementation

D-2026-09-16-3 removes the authority of a quantity name; D-2026-09-16-7
removes subset interpretation and reports. The remaining source fallback to
the M1.8 fixed registry must therefore retire, not infer a unit from a familiar
spelling. This completes that approved migration in the still-unblessed v19
candidate; no new version, field, binding or causal domain is allocated.

| Baseline | Replacement / retained rule |
|---|---|
| `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens` with an implicit unit | Explicit `@measure(id = input_tokens, unit = "token")` / `output_tokens`; authors may retain a dotted id only with explicit unit metadata, like any other id |
| Cache-read, cache-creation and reasoning token ids | Separate `cached_tokens`, `cache_creation_tokens`, `reasoning_tokens` samples with unit `"token"`; no implicit subset, subtraction or total |
| Names previously reserved to runtime instruments | No source authority or special meaning from the name; a numeric field may use any canonical id with its explicit unit. This does not synthesize a runtime instrument |
| Bare `@measure(id = ...)` | `E-DATA-ANNOTATION` explaining the missing unit or monetary currency/basis; no legacy alias |
| Compact registry annotation tag 3 | Retired, never reused; decoder refuses it by name. All numerical quantities use the already inventoried tag-6 protected measure binding with typed arguments |
| Monetary `amount`, security/privacy, descriptive metadata | Existing compact monetary/security encodings and tag-6 descriptive/named-money applications stay exact |
| Individual observation API | Exact signed decimal quantities and explicit unit text; no registry-derived integer/duration wrapper or subset field. No journal value is converted or rewritten |
| Deployment profile measure ids | Authored ids remain exact; migrate test profiles with their source ids. No tariff is applied and no alias matches an old id to a new one |
| Identity | Former registry-annotated programs change their metadata/hash explicitly; already explicit-unit programs, unannotated witnesses and their causal traces stay unchanged. No golden is blessed in this slice |

Migrate compiler, codec/hostile-plan and retention tests without losing their
path, propagation, confidentiality, exact-value or replay assertions. The
regressions must prove that even an old standard id requires a unit, an
explicit dotted id has no reserved carrier/unit, and retired tag 3 refuses.
Measure the same embedded Retry package before and after removing the
registry; do not weaken either footprint gate. The final A4 qualification and
common v19/v3 blessing still follow this inventory.

### Implementation and focused qualification

The source regression first accepted an old `gen_ai.usage.input_tokens` id
without a unit; the decoder regression first accepted retired tag 3. Both now
refuse by name. A dotted id with an explicit unit is accepted as an ordinary
author-chosen id, including a negative decimal value; it does not reserve a
carrier or unit. The source fallback, fixed lookup table, unsigned/duration
observation wrapper and implicit subset field are removed. Core re-derives
the protected typed binding and its numeric target independently.

The **362 core tests** pass. Seven focused hosted suites pass **65 tests**:
accounting 3, agent HTTP 3, business measures 8, monetary measures 9,
Retry runtime 25, typed JSON 10 and Loop policy 7. These retain copy/projection
and taint checks, distinct signed values/refunds, all original held attempts,
whole-batch disposition, every-prefix recovery and no calculated total.
The compiler subset has **210 passes and one failure**, the previously
inventoried M0 semantic-hash pin awaiting the common blessing, not a new
measure regression. No golden or expected hash is changed here.

The already-explicit Responses plan remains
`c247e9ee8888480fdf073918ed22255f9956dc9a16f2b33fa75fc1ed0a07a898`;
gauge remains
`0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e`.
The monetary neutrality regression preserves its pinned package/hash as well.

Strict workspace/all-target/all-feature Clippy, rustdoc and all three core
`no_std` checks pass. With the rebuilt CLI, **35 browser tests** and **29
JavaScript tests** pass; the Responses scenarios retain no bearer in LIVE,
HISTORY or REPLAY and expose no report. These local runs do not replace the
independent opening audit.

The identical 956-byte embedded Retry/recovery package measures **1,046,556 B**
of code (`.text` 963,420 + `.rodata` 83,136), saving **1,964 B** from the
projected-JSON checkpoint. **2,020 B** remain under 1 MiB; peak heap stays
**130,560 B**, **512 B** under 128 KiB. Package hash, three Calls, 16 frames
per channel, 17 records and `SPIKE_OK` are unchanged. This does not qualify
maximum annotation storage or linked individual inspection. Neither gate
has been raised. Logs: `/tmp/rastok-measure-migration.tZmKT9/`.

The final `cargo test --workspace --all-features --no-fail-fast` rerun reports
**1,527 passed, 9 failed, 0 ignored, 58 result groups**. The nine failures are
the unchanged candidate expectations: M0 semantic hash, echo recording bytes,
two network Contract/default pins, the gallery trace comparison, gauge package,
JSONL hash/package, and echo package. They remain red in six targets; no test
is ignored and no expectation is rewritten to conceal the common blessing.
See `workspace-warm.log` in the evidence directory.

The preceding run had **1,526 passes and 10 failures**: the same nine plus
the chat's 30-second listener-start deadline, with empty child stdout/stderr.
Its first isolated rerun also timed out. The loader diagnostic observed an
owned CLI process still at `_dyld_start`, before application entry. After an
ordinary CLI check completed on the rebuilt workspace binary, the unchanged
chat regression passed in **1.30 s**, then the entire workspace rerun passed
that test as well. No deadline, safety setting or assertion was relaxed.
The failed runs remain in `workspace-final.log` and `agent-chat-rerun.log`;
the successful replay is `agent-chat-warm-rerun.log`.

Final stdlib integration, A4 and the common candidate blessing remain open.

## 17. Standard definitions and aliases — 18 September, before implementation

Complete A1's already approved stdlib integration without another core binding
or package move. Ship the four standard signatures as ordinary `@struct`
declarations in the hosted stdlib and resolve them through the same type and
alias checker as authored definitions. Their names are protected at declaration,
including when a user attempts to replace them with an ordinary data alias.
The retired `cost` name remains a migration refusal, never an alias.

An alias of a standard definition retains that definition's interpretation:
`type Pii = personal` and `@Pii(identifier = true)` have exactly the same
policy, hash and package bytes as `@personal(identifier = true)`. A separately
declared lookalike remains descriptive metadata. Standard definitions also
work as ordinary data types; that use grants no policy to their values, and
ordinary data still requires every field. Annotation-only defaults remain
the normalization rules in [annotations §5.1](../spec/annotations.md#51-standard-signatures-and-normalization).

The checker resolves the standard measure argument schema from this stdlib
declaration; core independently verifies its existing protected schema.
Sensitive, secret, personal and compact monetary amount still lower to their
existing compact kernels. No root table, annotation tag, interpretation tag,
Contract, journal record, recording tuple, semantic domain or format version
changes. Unused definitions and aliases contribute no plan/hash bytes. The
embedded core does not load the hosted declaration source.

Required regressions: ordinary-data structural equivalence; alias-chain
identity for policy, quantity and monetary interpretations; default
normalization; duplicate-through-alias and replacement refusals; secret
admission and personal-data classification through aliases; and descriptive
lookalikes remaining powerless. This source integration closes neither A4's
footprint qualification nor the common v19/v3 blessing.

## 18. Generic measure target and OTLP boundary — 18 September

[D-2026-09-18-1](../spec/decisions.md#d-2026-09-18-1--separate-generic-measurements-otlp-export-and-budget-authority)
changes the intended standard measure signature to `id: text; unit: text`.
`unit = "USD"` describes a monetary denomination without a separate generic
`currency`, `basis`, `aggregation` or `subset_of` field. This supersedes the
final-signature expectation in section 17; its source-only alias integration
remains a checkpoint, not the completed measure migration.

Before resuming that migration, enumerate source diagnostics, normalized
arguments, annotation bindings, observation APIs, old monetary carriers,
evidence/profile handling and affected identity witnesses. Preserve exact
signed samples, original Retry emissions, privacy and causal provenance. Do
not claim that removing monetary-specific fields is byte-neutral, silently
drop evidence, allocate another format, or bless the candidate in this
documentation pass.

The separately planned `otlp` feature exports selected business measurements
and technical observations from the hosted layer. It introduces no dependency,
placeholder feature, core reporting engine or budget enforcement now. See
[observability §7.2](../spec/observability.md#72-planned-optional-otlp-export).

## 19. Two-field measure migration inventory — 18 September, before code

D-2026-09-18-1 now enters implementation on the existing unblessed v19/v3
candidate. D-2026-09-18-2 leaves Budget outside M1. This slice introduces no
OTLP feature, pricing, report, new journal fact or additional package version.

| Surface | Explicit movement / preserved evidence |
|---|---|
| Standard source definition | Exactly `id: text; unit: text`, both required. Aliases retain the protected definition. Currency units are ordinary nonempty unit text, not an ISO-only quantity class |
| Removed source arguments | `currency`, `basis`, `aggregation`, `subset_of` refuse with a migration explanation. A monetary scalar moves to an explicit unit; structured values annotate their numerical field, not a Money-shaped container |
| Protected definition identity | `rastok/measure@3` owns the two-field schema. The old `@2` definition/schema refuses, never acquiring a new meaning under the same identity. Existing tag-6 binding `Measure` remains the generic numerical interpretation |
| Compact monetary annotation | Retire tag 4 by name. Remove the special cost annotation, carrier inspection and separate cost-sample path. No retired tag is reused |
| Original evidence | Numeric DATA, declared types, original emissions, schema paths and sealed instance/Connector/exchange lineage remain. A value's unit supplies no claim that it was billed or calculated |
| Deployment evidence | Existing `CostProfile`, tariff metadata and run-header attestation keep their representation and digest. They remain unapplied; no migration erases an attested profile or invents a tariff/report consumer |
| Inspection | One signed exact `MeasureSample` path for quantities and amounts. No `CostSample`, inferred financial basis or special decimal(38,18) restriction on measured amounts. Ordinary type conformance remains authoritative |
| Identity | Annotated schemas change their semantic/package identity as inventoried. Unannotated bodies/hashes and unchanged DATA causal traces do not change. Current Retry/Contract/package changes still receive one common reviewed blessing |
| Source migration | Update executable examples, typed-response/Retry fixtures and active documentation; keep historical decisions/checkpoints explicitly dated. Reported-versus-calculated provenance is not inferred from the new annotation |

Required regressions: exact two-field ordinary data/aliases; signed integer and
decimal amounts without currency magic; rejected removed arguments and old
schema/tag; conflicting units; typed JSON hydration before Held disposition;
original samples after every recovery prefix without duplicates; privacy and
unchanged unattached-definition identity. Run the fixed embedded Retry workload
before final qualification: the last measured margins are only 2,020 B code
and 512 B heap. No gate may be raised to absorb this migration.

## 20. Local integration qualified — 18 September

The [common C4 ledger](retry-c4-qualification.md) closes A4's implementation,
fixture migration and local verification. The standard definition has exactly
`id: text` and `unit: text`; stdlib, source checking, independent package
validation, original-attempt hydration and inspection agree. `@cost` and
former currency/basis/aggregation/subset arguments refuse by name. There is
no monetary authority hidden in a unit and no M1 report or aggregate.

The final ordinary all-feature workspace run has **1,545 passed, 0 failed,
0 ignored**, including the retired-tag and generic signed/decimal tests.
Formatting, strict Clippy, rustdoc, three no_std targets, 35 browser tests,
30 other JavaScript tests and 3 actual-tokenizer tests pass. The ledger keeps
two earlier listener-start failures and their unchanged passing reruns visible.
The same Retry/recovery embedded image leaves **6,852 B code / 512 B heap**
margin; freshly blessed echo also passes. No limit is raised.

The shared blessing updates only the inventoried packages, hashes, recording
tuple digest and affected gallery traces. Echo/JSONL/gauge causal seals and
gauge's semantic hash stay unchanged. Six new Responses goldens complement
local physical hydration and no-secret-retention tests; no paid provider
qualification is implied. Earlier checkpoints above preserve their facts,
but pending bindings, example migration and blessing are no longer M1 work.

Independent opening review and distribution remain in
[opening-showcase.md](opening-showcase.md). Optional `otlp` and the future
Budget Extension remain documented directions, not implementations or M1 gates.
