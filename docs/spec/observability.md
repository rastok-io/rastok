# Data policy, privacy, observations and cost

> **M1 scope amendment — D-2026-09-16-7:** retain individual typed
> measurements, including Retry's held attempts. No report engine, automatic
> pricing or subset arithmetic ships in M1. §6–7 describe that reduced scope;
> quantity ids have explicit units, not implicit registry meanings.

> **Format 20 target — 6 October 2026.** Security, privacy,
> propagation and late-bound secret guarantees remain in force, alongside [flat annotation types](annotations.md), typed response
> hydration and original signed measurements. Under D-2026-09-18-1 every
> measure has exactly `id` and `unit`: money is a quantity with a currency
> unit, not a second annotation profile. The [annotation inventory](../architecture/annotation-types-m1.md)
> and [common qualification](../architecture/retry-c4-qualification.md) track
> identity movements, unchanged witnesses and verification.
> Registry-free validation re-derives local claims and canonical propagation;
> presentation redacts before formatting. Raw executable `@secret` sites
> remain refused before effects.

This document owns data classification, typed runtime observations and their
accounting projections. Logical values and assignability remain owned by
[`types.md`](types.md); causal record structure remains owned by
[`delivery-semantics.md`](delivery-semantics.md) and
[`identity.md`](identity.md).

## 1. Two trees with different jobs

One resolved schema contains a logical type and a policy side-table:

```text
AnnotatedSchema
  type         : DataType
  annotations  : SchemaPath -> AnnotationSet
```

`DataType` answers whether a value conforms. `AnnotationSet` answers how a
conforming value is handled, observed and interpreted. An annotation does not
make two otherwise equal logical types unassignable. A policy-aware pass may
still refuse a sink or transformation independently of type conformance.

The side-table is sparse and canonical. An empty table has one representation
and triggers no runtime schema walk. Paths are sequences of typed segments:

```text
root
field(name)
array_element
map_key
map_value
structural_union_branch(index)
tagged_union_case(name)
```

Field and case names use the same canonical name rules as `DataType`.
Structural branch indexes refer to the canonical branch order, never source
order. A path must resolve in the adjacent type tree; dead or mistyped paths
refuse after decode.

The plan carries this sparse table locally beside every transported
schema that can own policy: raw and resolved port rows, nested-Graph ports,
owner values, Node state cells and handler-local typed bindings. Empty tables
are omitted. The decoder reconstructs the table from its structural wire form;
validation resolves every path against the adjacent `DataType` and verifies
its interpretation and typed arguments. Raw rows remain source declarations. The separately
transported resolved-port table is an effective verdict: A3 reconstructs it
from raw declarations and canonical operations, then requires exact equality.
A derived verdict is never copied back into a raw row and cannot masquerade as
source authority.

## 2. Source form

```ebnf
annotated_type       ::= type_ref data_annotation*
data_annotation      ::= "@" IDENT annotation_arguments?
annotation_arguments ::= "(" named_annotation_argument
                         ("," named_annotation_argument)* ")"
named_annotation_argument ::= IDENT "=" annotation_constant
```

Arguments are named plan constants. Duplicate names, positional arguments,
expressions, `env()`, `secret()`, `data`, `meta`, state and open maps refuse.
Annotation order is not semantic; canonical order follows annotation identity.

The executable source interpretations are:

```rastok
text @sensitive(level = confidential)
text @secret
text @personal(identifier = true)
uint @measure(id = input_tokens, unit = "token")
decimal(38,18) @measure(id = amount, unit = "USD")
```

There is no `@dimension` in M1.8. Provider, model, protocol, Node, port,
instance and lineage are typed context already known to the Driver or journal.
Arbitrary user-selected metric labels would make cardinality and policy an
unbounded source concern.

## 3. Annotation contracts

### 3.1 `@sensitive`

The levels are ordered `internal < confidential < restricted`. Exact copy,
projection and rename preserve the level. Combining values selects the maximum.
Derived opaque guest output is at least as sensitive as every input visible to
the guest. Logs, traces, diagnostics and Lab fixture projection consult this
policy before formatting or serialization.

### 3.2 `@secret`

Secret is a handling class stronger than `restricted`; it is not a scalar
type. It propagates through every computation that can depend on the value.
M1 permits one executable path:

```text
secret("path") -> SecretName -> secret-compatible Connector setting
               -> late Driver resolution
```

The clear value is never a `Value` and never enters source, plan, package,
journal, diagnostic, log or trace. Provider, logical path and version are
attested in the run header before use. An unused named alias may describe
`@secret` data, but attaching it to any source-executable schema site refuses
at check with `E-CHECK-DEFERRED`. A later externalization protocol may replace
clear input with a reference before the causal commit and thereby open that
gate. VM boot retains the same refusal for foreign or forged packages; it is
defense in depth, not a delayed source-language judgement.

### 3.3 `@personal`

`@personal` marks personal data in the sense of Regulation (EU) 2016/679,
not merely the narrower security acronym PII. It is independent of
`@sensitive` and accepts three optional boolean plan constants:

```rastok
text @personal
text @personal(identifier = true)
text @personal(special = true)
text @personal(criminal = true)
```

`special` identifies Article 9 special-category data; `criminal` keeps the
distinct Article 10 regime; `identifier` marks an exact value used to identify
or index the data subject. The flags may be combined. Exact copy and projection
preserve all three. A calculation retains `special`/`criminal` taint but clears
`identifier`; alternative producers retain that role only when every
alternative is an identifier. All personal data is redacted before
presentation by default, regardless of its confidentiality level.

This annotation is a technical policy input, not a compliance certificate.
Purpose, legal basis, recipients, transfer safeguards and retention belong to
the attested deployment privacy profile. A4 must receive that profile before a
personal-data plan can boot. The M1 hosted runner then uses only its run-scoped
in-memory journal; a future durable journal must expose an approved erasure or
field-externalization capability before accepting clear personal data.

These rules implement technical support for privacy by design and default,
data minimisation, storage limitation, security and accountability. The
controller remains responsible for the legal and organisational judgements
required by Articles 5, 25, 30 and 32, including any erasure request under
Article 17.

The resulting PII/personal-data classes are therefore:

| Source | Technical class | Additional deployment evidence |
|---|---|---|
| `@personal` | ordinary personal data | purpose and Article 6 legal basis |
| `@personal(identifier = true)` | ordinary personal data used as a subject identifier | same; identifier semantics survive only exact transformations |
| `@personal(special = true)` | Article 9 special-category personal data | explicit Article 9 condition |
| `@personal(criminal = true)` | Article 10 criminal-offence personal data | controller-declared Article 10 authority |

This is one composable classification, not a sensitivity hierarchy: the
Article 9, Article 10 and identifier flags can coexist on one path.

### 3.4 `@measure`

A numerical quantity declares its id and unit explicitly:

```rkg
uint @measure(id = input_tokens, unit = "token")
int @measure(id = stock_correction, unit = "item")
decimal(38,18) @measure(id = weight, unit = "kg")
```

An id has no built-in unit, carrier, subset or exporter meaning. Canonical
simple or dotted names are author-chosen; neither OTel names nor a reserved
prefix confer authority. Even a former standard name requires an explicit
unit. The target must be an integer or exact decimal; signed field types
retain negative corrections, and zero is an observation, not missing evidence.
The unit is opaque text: no conversion is implicit.

The protected `std/measure@3` binding carries the same ordinary flat
argument schema as other annotation definitions. Complete arguments, numeric
targets, consistent units per id and interpretation authority are checked
independently when opening a package. The schema has exactly two required
text fields; there is no aggregation argument or M1 aggregate.

Input, output, cache and reasoning counts may be separate original samples.
M1 derives no relationship or total from their names. Generic `subset_of`
refuses by name. Runtime instruments and future exporter mappings are separate
consumers; a field annotation neither produces an instrument nor adopts one.

The protected definition is `std/measure@3`, represented as a typed declared
annotation (application tag 4, numerical binding 2). The format's active
registry is [identity.md](identity.md); no parallel compact cost/measure tag
or implicit standard quantity registry exists.

### 3.5 Amounts are ordinary quantities

`decimal(18,6) @measure(id = amount, unit = "USD")` uses exactly the same
binding as tokens or stock corrections. The field supplies the signed exact
value. The id remains author-chosen; the unit is opaque text, not an ISO
registry lookup or a permission to charge. A negative value can describe a
refund. Original values remain separate facts; M1 computes no net amount.

If a response contains `{ amount, currency }`, annotate the numerical
`amount` field with its statically declared unit. Annotating the whole struct,
deriving a unit from a runtime field, or silently converting currencies is
not admitted. One id cannot carry different units in the same validated plan.

`@cost` is retired. Former `currency`, `basis`, `aggregation` and
`subset_of` arguments also refuse with a migration explanation. A consumer
that values tokens, compares an estimate to an invoice, or enforces a budget
needs an explicit policy; those operations are outside the M1 annotation.

The existing exact `Money` and pinned ISO currency checks remain limited to
retained deployment tariff metadata, not measured values. M1 does not apply
those tariffs.

## 4. Propagation

| Operation | Sensitive / secret / personal | Measure |
|---|---|---|
| exact copy, projection, rename | preserve | preserve |
| calculation | maximum taint | drop unless a registered rule proves meaning |
| merge/fusion | maximum taint | explicit rule or refusal |
| opaque guest | conservative contamination | drop |
| codec round-trip | preserve only through a policy-compatible boundary | preserve only with exact logical meaning |

The hosted checker derives the output table. Categorical validation derives it
again from canonical operations and requires equality. The VM never repairs,
guesses or reinterprets an annotation name.

A3 covers boundary and Node ports, nested-Graph boundaries, owner `val`s,
Node-owned `var`s, handler-local `let`s, guards, field writes, Loop recurrence
and `LoopExpired.last_state`. Control dependence carries the strongest
security taint. Alternative producers must agree exactly on measure
meaning; disagreement is `E-DATA-ANNOTATION`, never a first-wins choice.
Metadata has no data-policy schema in M1, so writing classified data into
`meta.app` or `meta.ext` refuses rather than losing its policy.

Short-circuit `&&` carries the left operand's proven type/case into the
right operand's policy derivation, just as expression typing does. Both
operands still contribute control taint; narrowing never declassifies the
decision. The `short_circuit_and_projects_policy_through_the_proven_case`
regression covers ordinary and join-qualified paths.

Opaque guest execution remains a named gate. No guest form can execute around
this judgement; a future guest boundary must use the same conservative
security-taint rule and discard accounting meaning before admission.

If the raw plan has no annotation declaration, validation returns the ordinary
resolved table without walking operation trees. Annotated plans pay the fixed
derivation once at validation; transactions do no policy reconstruction. VM
boot caches whether a raw clear-secret site remains in a decoded or forged
plan and refuses it with `UnsupportedDataPolicy` before journal or state
mutation. The source checker has already refused that executable use; named
aliases which are never used remain harmless structural vocabulary. A personal-data site
is admitted only when the booted profile covers every ordinary/Article 9/
Article 10 requirement and the journal exposes the exact same deployment
attestation. Sensitivity and measure policy otherwise execute. CLI and
Lab fault paths never format a runtime value before consulting policy or
reducing the message to a value category.

## 5. Deployment bindings and secrets

Human configuration is a hosted concern:

```text
closed deployment JSON / `--deployment` path / provider environment
                         |
                         v
                 DeploymentProfile
                         |
                         v
                    BootBindings
                         |
                         v
                  no-std semantic core
```

The core accepts typed values and provider interfaces; it parses no human
format. Duplicate physical sources for one binding refuse rather than obeying
a hidden precedence rule. Secrets are never accepted as CLI argument values
or JSON fields. Embedded applications may construct the same typed bindings
directly or load a separately specified canonical binary profile.

The hosted M1 profile is closed JSON. Non-secret environment values are
explicit name/value rows. A secret row carries a logical reference, the name
of an environment-provider coordinate and an immutable version; the actual
bytes come only from that environment variable, whose `<NAME>_VERSION`
companion must match. The executable source forms are exact root references:

```rastok
connector model : HttpClient {
  url = env("MODEL_URL")
  auth_bearer = secret("providers/model/token")
  // ...
}
```

In M1 only `HttpClient.url` admits `env()` and
`HttpClient.auth_bearer` admits `secret()`. A concatenation, the wrong
reference kind, any other setting and all unit/App boot `val` spellings refuse
by name until their concrete consumers land. Both real and scripted Drivers
bind the same logical request descriptor. The real Driver alone overlays
secret material inside its boundary; package, request digest, journal,
diagnostics and Lab expectations retain only the logical reference.

The same profile carries a versioned privacy attestation whenever the plan
contains `@personal`. Its stable processing-activity reference and digest bind
the declared purposes, legal-basis attestation, retention rule, recipient
classes and any transfer safeguard without placing those deployment facts in
graph identity. The run header records the reference and digest before the
first personal value is admitted. Rastok verifies that the profile covers the
plan and selected boundary; it does not adjudicate the controller's legal
basis. Missing or contradictory coverage refuses before any effect.
If personal data reaches an input of an external Connector, `recipients` must
name at least one stable recipient class; declaring no recipient is accepted
only when the validated flow does not disclose personal data across such a
boundary. `transfer_safeguards = []` is an explicit controller attestation
that this processing has no international transfer, not a value inferred from
a URL.

The same immutable header stores each non-secret boot binding as
`(EnvironmentName, canonical tagged-value digest)` and each secret binding as
logical reference/provider coordinate/version. A VM with an absent or
type-incompatible binding, or a journal carrying another header, refuses
before mutation even through the low-level Rust API.

M1 accepts only `run_scoped` retention and only the in-memory journal for a
personal-data run. A special-category path additionally requires a declared
Article 9 condition; a criminal-offence path requires a declared Article 10
authority. A future durable journal is not implicitly authorized by this
profile: the journal capability seam defaults to refusal, and an implementation
must first expose an independently reviewed erasure or externalization
capability.

One hosted profile therefore has this closed shape (references are
controller-owned stable identifiers, not free-form legal prose):

```json
{
  "environment": [
    { "name": "MODEL_URL", "value": "https://api.example.test" }
  ],
  "secrets": [
    {
      "reference": "providers/model/token",
      "environment": "MODEL_TOKEN",
      "version": "release-7"
    }
  ],
  "privacy": {
    "reference": "privacy.example/support@1",
    "processing_activity": "ropa/support",
    "purposes": ["purpose/customer-support"],
    "legal_bases": ["basis/contract"],
    "special_category_condition": null,
    "criminal_offence_authority": null,
    "recipients": ["recipient/model-provider"],
    "transfer_safeguards": ["transfer/scc-2021"],
    "retention": "run_scoped"
  },
  "cost_profile": {
    "reference": "pricing.example/model-api@2026-08-30",
    "prices": [
      {
        "connector": "model",
        "provider": "example-ai",
        "model": "large-v1",
        "measure": "input_tokens",
        "units": 1000000,
        "amount": "2.50",
        "currency": "USD"
      }
    ]
  }
}
```

For this example the host supplies `MODEL_TOKEN` and
`MODEL_TOKEN_VERSION=release-7` out of band. A graph carrying
`@personal(special = true)` must replace the null Article 9 condition with a
stable attestation reference; the analogous rule applies to Article 10.

## 6. Record observations

A causal journal record has one typed structural observation, and accounting
may enrich an annotated envelope lazily:

```text
RecordObservation     = record transition + typed correlation
AccountingObservation = RecordObservation + envelope id
                       + exact measure/cost samples + flow scope
```

Both are excluded from the causal seal. Exact committed time, duration,
runtime/host facts and late diagnostics remain future typed extensions; M1.8
does not synthesize them at projection time. If an amount can affect graph
control, it must instead enter as ordinary typed envelope data and therefore
becomes causal. Observation-only cost can never drive a branch.

The structural observation of a committed record carries correlation already
owned by causal history: `origin_id`, owner/instance, Connector and a typed
inbound/outbound Call or Session/write reference. Stream order associates it
with record order; no second ordinal or placement fact is invented. The VM's
payload-bearing `EnvelopeView` delivery log is a distinct Lab/debugger surface,
not this retained accounting plane.

The journal and its associated validated plan are the measurement authority.
Values remain in the original admitted or emitted envelopes; annotations
describe their meaning. They are not duplicated into a measurement ledger.
D-2026-09-16-7 removes the report engine from M1: no grouping, total, automatic
price calculation, subset subtraction or `unpriced` result is produced.
A missing value stays missing; a known zero remains an original value.

## 7. Projection

Read-only inspection derives individual observations from the journal prefix
and resolved schemas. Structured logging independently uses a bounded
never-block queue: saturation drops a projection and increments its local
typed drop counter, without delaying or failing a transaction. Redaction runs
before formatting; payload content is omitted by default.

The hosted runner exposes structural logs through `--json-logs` on a
separate thread. Its terminal I/O never runs in the VM producer. Event names
remain `rastok.ingress.committed`, `rastok.transaction.committed`,
`rastok.call.*` and `rastok.session.*`. The later OpenTelemetry exporter
still requires exact unsealed timing evidence; it cannot invent span times.
Neither that exporter nor a measurement report is an M1 completion gate.

### 7.1 Individual measurement inspection

`accounting_observations` reads INGRESS and each original TRANSACTION
emission. Each observation retains the record correlation, envelope id,
canonical annotation path, exact value and provenance: origin, instance
generation, owner, Node, Connector, port and typed exchange. It does not
combine copies of a value at different flow points into a consumption total.

A typed decoding Operator can expose annotations from a dynamically typed
response. The Connector provenance is then the nearest producer in the
envelope's sealed parent chain; Node and port still name the current point.
The lineage walk stops at the current record, so a later record cannot
explain an earlier sample. No attribution is guessed from topology.

A Connector with a declared response type hydrates the fields before
retention. Retry Held and Published batches retain those exact original
emissions, including signed quantities and refunds. Consuming a Held batch
does not produce another measurement-bearing emission. Recovery and diagnostic
recording preserve the records and plan; neither re-runs an extractor nor
recalculates a quantity.

Numerical inspection preserves exact integer/decimal values, ids, units and
sealed causal provenance. An id does not imply a unit or a tariff.
Independent valid samples remain readable even if their sum would overflow.
An annotated value retained only as bytes cannot be decoded by this inspection
helper; its normal codec must have materialized it. Missing data is never zero.

The existing deployment `PriceBook` and its run-header attestation keep
their encoded representation for compatibility. They may supply declared
provider/model context for the matching causal Connector; M1 does not apply
their tariffs or infer a financial basis from a sample. These metadata must
not be advertised as an executable reporting engine.

The debugger keeps its ordinary policy-aware DATA and META inspectors. It
does not emit a `cost_report` snapshot or display a report panel. A local UI
aggregate remains an undecided presentation feature, not a runtime policy.

### 7.2 Planned optional OTLP export

**Approved architecture, not implemented — D-2026-09-18-1.** Export is a
separate consumer of the executable two-field `@measure` signature. A currency
used as a unit requires no special exporter hook in the semantic core.

The future Cargo feature is `otlp`, in hosted `rastok`, separately enabled by
deployment configuration. An image without that capability must reject an
explicit export request. No feature placeholder or dependency is added now.
The semantic core neither imports OTel nor requires an exporter to retain,
inspect or replay its measurements.

One observation adapter can select two sources:

- business quantities from original typed envelopes plus their validated
  annotations and sealed causal provenance, including held Retry attempts;
- technical facts from retained transitions, such as Call/Retry events, or
  explicitly identified hosted observations, such as current queue occupancy.
  A live host sample must not be presented as replayable journal evidence.

The adapter does not decode a response again or rerun a downstream Operator.
Existing `RecordObservation` and individual accounting inspection supply
starting read seams, not an already implemented incremental OTLP subscription.
The present Driver registry registers effect implementations; it is not an
observer registry. A static optional hosted adapter needs no dynamic plugin
loader or speculative general-purpose plugin ABI.

An export mapping chooses source paths, metric names, units, allowed attributes,
representation and temporality. It lives outside the DSL and cannot change a
source quantity's meaning. The
[OTel metrics model](https://opentelemetry.io/docs/specs/otel/metrics/data-model/)
carries metric streams with aggregation semantics, not arbitrary exact raw
measurement events. The adapter must therefore define any local reduction
needed before a Collector/backend can aggregate further; `@measure(id, unit)`
alone does not select a sum, gauge or histogram. Signed observations cannot
silently become a monotonic counter. The numerical conversion policy must
make loss of exact decimal precision explicit; journal values remain exact.

The handoff is bounded and never blocks the semantic producer. Export runs
outside the VM, with bounded buffering/retries, observable loss and a qualified
shutdown policy. Re-export/restart cursor behavior must be defined before
shipping, not advertised as exactly-once delivery. Diagnostic replay must not
silently add old measurements to live production series.

Redaction and destination policy precede serialization. An exporter does not
bypass `@secret`, `@personal` or `@sensitive`; arbitrary payload fields and
high-cardinality instance/envelope ids are not metric attributes by default.
Durations and spans require their actual time evidence. Collection time must
not masquerade as transaction commit time. No hidden clock enters the core.

This is a projection, like structural logs: its absence or failure cannot
change routing, Retry admission or a future budget verdict. A read-only cost
consumer may later value retained quantities under an explicit tariff or
select a reported amount. Enforcing a budget instead requires authority
**before** the effect; the
[current budget proposal](../architecture/budget-governance-draft.md#current-proposal--18-september-2026)
keeps that protocol distinct. Neither consumer is implemented by this decision.

## 8. Named later work

- Budget/quota Extensions and pre-effect reservation;
- FX, billing authority, FOCUS conformance and OpenCost allocation;
- measurement reports, automatic pricing and any debugger-only aggregate;
- generic subset relations and complete registry migration;
  simple additive integer/decimal business quantities are executable under
  [D-2026-09-15-5](annotations.md); arbitrary metric dimensions remain deferred;
- sampled cost calibration and automatic model selection;
- field-level externalization of incoming secret payloads;
- hot secret rotation and encrypted durable-journal storage;
- durable personal-data journaling until erasure/externalization is proved;
- provider-specific exporter plugins.
- exact retained commit time and duration, followed by the qualified hosted
  OpenTelemetry transport and independent Collector evidence.

The non-normative reservation exploration lives in
[`budget-governance-draft.md`](../architecture/budget-governance-draft.md).
