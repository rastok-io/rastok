# Data policy, privacy, observations and cost

> **Status: normative and executable through closed M1.8 under
> D-2026-08-30-1 through D-2026-08-30-6 and D-2026-08-31-1.** Current package v17 retains the M1.8 policy transport as
> canonical side-tables. Registry-free validation re-derives every local
> claim and the effective policy of canonical operations; presentation
> redacts before formatting. A4 admits personal data only under an attested
> run-scoped privacy profile and resolves reference-origin credentials only at
> declared Driver settings. Executable raw `@secret` sites defer at source
> check, and VM boot keeps foreign packages fail-closed.

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

Categorical v17 retains this sparse table locally beside every transported
schema that can own policy: raw and resolved port rows, nested-Graph ports,
owner values, Node state cells and handler-local typed bindings. Empty tables
are omitted. The decoder reconstructs the table from its structural wire form;
validation resolves every path against the adjacent `DataType` and verifies
the closed registry. Raw rows remain source declarations. The separately
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

The M1.8 set is closed:

```rastok
text @sensitive(level = confidential)
text @secret
text @personal(identifier = true)
uint @measure(id = gen_ai.usage.input_tokens)
decimal(38,18) @cost(currency = USD, basis = provider_reported)
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
secret("path") -> SecretRef -> secret-compatible Connector setting
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

`id` selects one entry from the pinned semantic-measure registry. The entry
fixes carrier, unit, aggregation, subset relations and legal sources; source
does not repeat them. Unknown ids or incompatible field types refuse.

OpenTelemetry names have priority when their definitions match exactly. The
registry distinguishes an OTel attribute from an OTel instrument even when
both describe the same quantity. `@measure` binds a field to the former; the
runtime projects the latter without inventing another semantic name.

The initial standard field identities are:

- `gen_ai.usage.input_tokens`;
- `gen_ai.usage.output_tokens`;
- `gen_ai.usage.cache_read.input_tokens`;
- `gen_ai.usage.cache_creation.input_tokens`;
- `gen_ai.usage.reasoning.output_tokens`;

The following standard instruments are pinned for the later hosted projection;
they are not field annotations and A6 remains explicitly deferred:

- `gen_ai.client.token.usage`;
- `gen_ai.client.operation.duration`;
- `gen_ai.client.operation.time_to_first_chunk`;
- `gen_ai.client.operation.time_per_output_chunk`;
- `http.client.request.duration`, `http.client.request.body.size` and
  `http.client.response.body.size`;
- `http.server.request.duration`, `http.server.request.body.size` and
  `http.server.response.body.size`.

Cache-read and cache-creation counts are subsets of input tokens. Reasoning
tokens are a subset of output tokens. Reports retain all observations but do
not add a subset to its total. When a provider distinguishes consumed from
billable token counts, the pinned OTel rule preferring billable usage applies;
the observation retains its provider-reported provenance.

The registry owns an exact internal unit as well as the carrier. Token fields
use `token`; HTTP body sizes use bytes; retained durations use integer
nanoseconds and are rescaled only by an exporter. Field measures are
non-negative additive samples. A negative value, carrier mismatch or overflow
refuses the report rather than being skipped.

Rastok-specific operational instruments will use `rastok.*`, including flow
envelope count/payload size, generic Connector calls and duration, mailbox
depth, edge lag, instance count/eviction, journal size/low-water mark and
dropped observation projections. Their final spellings are allocated only
when their retained facts and exporter land; A7 does not pretend that an
exporter counter is an annotated field measure. A future OTel convention that
exactly covers one requires an explicit registry migration; two aliases are
never emitted silently.

### 3.5 `@cost`

`basis` is initially `provider_reported` or `pricebook_calculated`. A decimal
field requires a constant ISO currency. A canonical Money-shaped struct
contains `amount: decimal(38,18)` and `currency: text`; its currency value is
validated and a redundant constant argument refuses. Costs of different basis
or currency remain separate observations.

The source compiler checks membership against the ISO 4217 Maintenance
Agency's current List One snapshot fetched on 2026-08-30 (SHA-256
`838dfb991648cf36df939edd5fe3811737962b75a32252847d239cedd1e291c9`). A
registry update is an explicit semantic migration; it is never inherited from
the host or a dependency.

`Money` is a carrier convention, not a `DataType` constructor. Arithmetic is
checked exact decimal arithmetic. Overflow, excess precision, a malformed
currency and any required rounding refuse; float conversion is never implicit.

## 4. Propagation

| Operation | Sensitive / secret / personal | Measure / cost |
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
security taint. Alternative producers must agree exactly on measure and cost
meaning; disagreement is `E-DATA-ANNOTATION`, never a first-wins choice.
Metadata has no data-policy schema in M1, so writing classified data into
`meta.app` or `meta.ext` refuses rather than losing its policy.

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
attestation. Sensitivity, measure and cost policy otherwise execute. CLI and
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
    "reference": "privacy/support@1",
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
    "reference": "pricing/model-api@2026-08-30",
    "prices": [
      {
        "connector": "model",
        "provider": "example-ai",
        "model": "large-v1",
        "measure": "gen_ai.usage.input_tokens",
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
payload-bearing `Observed` delivery log is a distinct Lab/debugger surface,
not this retained accounting plane.

The journal is the accounting authority. `CostReport` is a deterministic fold
over retained observations, grouped by `origin_id` and optionally instance,
Node, Connector, provider/model, basis and currency. A missing price produces
an explicit `unpriced` item. No cross-currency total exists without a later
explicit FX authority.

## 7. Projection

```text
journal + retained observation stream
       +-- deterministic fold ------> CostReport
       `-- bounded try-send --------> logs / OpenTelemetry
```

The export path is never-block. A full queue drops the projection and
saturating-increments its local typed drop counter; it cannot delay or fail a
transaction. A future A6 exporter allocates the final `rastok.*` instrument
name only with its retained fact and projection. Redaction runs before output
formatting. Payload content is omitted by default.

`meta.sys.origin_id` supplies the OpenTelemetry trace id. One logical
transaction becomes one span; an outbound Call becomes its child only when the
Call actually opens. A budget refusal or pre-open validation failure must never
fabricate a network span. Sampling affects only export. The journal and report
remain complete.

High-cardinality refs such as origin, instance, Call and Session are available
on spans/logs and in reports. They are not metric labels by default. Standard
OTel instrument names and attribute definitions are used verbatim when
applicable; only uncovered project concepts use `rastok.*`.

The hosted runner exposes this projection through `--json-logs`. Export occurs
on a separate thread; its blocking terminal I/O is never executed by the VM
producer. Event names are Rastok-specific (`rastok.inbound.committed`,
`rastok.transaction.committed`, `rastok.call.*`, `rastok.session.*`) while
standard measure instruments remain governed by the registry in §3.4.

### 7.1 Exact report fold

A7 enriches a structural `RecordObservation` lazily at each annotated envelope
and port. The typed accounting observation carries the structural observation,
the envelope id, canonical annotation path, measure/cost value and exact flow
scope. The complete grouping key is origin, instance generation, owner, Node,
Connector, port, typed exchange and optional deployment provider/model. This
is deliberately a per-flow report: copying one measured value through three
ports yields three clearly different rows, never one graph-wide total which a
caller might mistake for new consumption.

For a dynamically typed boundary, the field annotation can become visible
only after an ordinary typed decoding Operator. The Connector dimension is
then the nearest Connector producer in the envelope's sealed parent chain,
while Node and port continue to name the current flow point. Exact
copy/projection is the only way accounting meaning reaches that point;
calculations drop it. The lineage walk ends at the current record, so a future
fact can never explain an earlier observation. Attribution never guesses from
graph topology or a Connector name.

`CostReport::from_journal` folds those observations directly from causal
records and the validated plan while checking any supplied price profile
against the immutable run header. The lower-level `from_records` form is for
already authenticated record sets: its caller owns that external provenance.
Provider-reported fields retain their exact basis and
currency. An optional deployment `CostProfile` calculates additional costs
from registered measures. It is a closed canonical table keyed by Connector
path and measure; every rule fixes provider, model/SKU, a positive power-of-ten
unit quantity, exact decimal price and ISO currency. Its stable reference and
digest are recorded in the immutable run header. The profile is not graph
identity and cannot change causal bytes.
An annotated `pricebook_calculated` field requires at least one profile row
for its sealed causal Connector; a merely present but unrelated profile is not
evidence.

Subset-aware pricing prevents token double counting. If cache tokens have a
special price, they are subtracted from inclusive input tokens and priced
separately; otherwise the inclusive input rule covers them. The same rule
holds for reasoning/output tokens. A subset larger than its inclusive parent
is invalid within each observed envelope and cannot be hidden by aggregation
with a later valid sample. Every positive remainder without a matching rule is
emitted as `unpriced`. An explicit zero price is a real price and remains
distinguishable from that state.

Amounts normalize as exact decimal values. Addition and multiplication are
checked; division by a unit bundle changes scale only. Any required rounding,
precision overflow or scale beyond canonical Money refuses. Cost rows with
different provider/pricebook evidence, currency or source measure never merge.
No API exposes a cross-currency total.

An annotated envelope retained only as encoded bytes cannot be interpreted by
a reporting-only decoder and therefore refuses until its normal codec has
materialized the logical value. This is fail-closed accounting, not missing
data disguised as zero.

## 8. Named later work

- Budget/quota Extensions and pre-effect reservation;
- FX, billing authority, FOCUS conformance and OpenCost allocation;
- arbitrary user-defined measures or metric dimensions;
- sampled cost calibration and automatic model selection;
- field-level externalization of incoming secret payloads;
- hot secret rotation and encrypted durable-journal storage;
- durable personal-data journaling until erasure/externalization is proved;
- provider-specific exporter plugins.
- exact retained commit time and duration, followed by the qualified hosted
  OpenTelemetry transport and independent Collector evidence.

The non-normative reservation exploration lives in
[`budget-governance-draft.md`](../architecture/budget-governance-draft.md).
