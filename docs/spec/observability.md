# Data policy, observations and cost

> **Status: normative M1.8 target under D-2026-08-30-1.** A1 is open but no
> annotation is executable and categorical package v14 remains current. The
> host retains recognized syntax and the core owns its canonical side-table;
> every use refuses by name until the complete resolver and v15 transport are
> independently validated.

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
attested in the run header before use. A raw input schema field carrying
`@secret` refuses at its decode boundary until a later externalization protocol
can replace clear input with a reference before the causal commit.

### 3.3 `@measure`

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

The initial standard instruments are:

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

Rastok-specific operational measures use `rastok.*`, including flow envelope
count/payload size, generic Connector calls and duration, mailbox depth, edge
lag, instance count/eviction, journal size/low-water mark and dropped
observation projections. A future OTel convention that exactly covers one of
these requires an explicit registry migration; two aliases are never emitted
silently.

### 3.4 `@cost`

`basis` is initially `provider_reported` or `pricebook_calculated`. A decimal
field requires a constant ISO currency. A canonical Money-shaped struct
contains `amount: decimal(38,18)` and `currency: text`; its currency value is
validated and a redundant constant argument refuses. Costs of different basis
or currency remain separate observations.

`Money` is a carrier convention, not a `DataType` constructor. Arithmetic is
checked exact decimal arithmetic. Overflow, excess precision, a malformed
currency and any required rounding refuse; float conversion is never implicit.

## 4. Propagation

| Operation | Sensitive / secret | Measure / cost |
|---|---|---|
| exact copy, projection, rename | preserve | preserve |
| calculation | maximum taint | drop unless a registered rule proves meaning |
| merge/fusion | maximum taint | explicit rule or refusal |
| opaque guest | conservative contamination | drop |
| codec round-trip | preserve only through a policy-compatible boundary | preserve only with exact logical meaning |

The hosted checker derives the output table. Categorical validation derives it
again from canonical operations and requires equality. The VM never repairs,
guesses or reinterprets an annotation name.

## 5. Deployment bindings and secrets

Human configuration is a hosted concern:

```text
deployment JSON / explicit CLI setting / explicit env binding
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
a hidden precedence rule. Secrets are never accepted as CLI argument values.
Embedded applications may construct the same typed bindings directly or load
a separately specified canonical binary profile.

## 6. Record observations

A causal journal record may carry a typed, non-causal observation ring:

```text
RecordObservation = committed_at
                  | duration
                  | measure
                  | cost
                  | runtime/host fact
                  | dropped/late diagnostic fact
```

These facts are recorded when useful but excluded from the causal seal. If an
amount can affect graph control, it must instead enter as ordinary typed
envelope data and therefore becomes causal. Observation-only cost can never
drive a branch.

The journal is the accounting authority. `CostReport` is a deterministic fold
over retained observations, grouped by `origin_id` and optionally instance,
Node, Connector, provider/model, basis and currency. A missing price produces
an explicit `unpriced` item. No cross-currency total exists without a later
explicit FX authority.

## 7. Projection

```text
journal + observation ring
       +-- deterministic fold ------> CostReport
       `-- bounded try-send --------> logs / OpenTelemetry
```

The export path is never-block. A full queue drops the projection and increments
`rastok.observation.dropped`; it cannot delay or fail a transaction. Redaction
runs before output formatting. Payload content is omitted by default.

`meta.sys.origin_id` supplies the OpenTelemetry trace id. One logical
transaction becomes one span; an outbound Call becomes its child only when the
Call actually opens. A budget refusal or pre-open validation failure must never
fabricate a network span. Sampling affects only export. The journal and report
remain complete.

High-cardinality refs such as origin, instance, Call and Session are available
on spans/logs and in reports. They are not metric labels by default. Standard
OTel instrument names and attribute definitions are used verbatim when
applicable; only uncovered project concepts use `rastok.*`.

## 8. Named later work

- Budget/quota aspects and pre-effect reservation;
- FX, billing authority, FOCUS conformance and OpenCost allocation;
- arbitrary user-defined measures or metric dimensions;
- sampled cost calibration and automatic model selection;
- field-level externalization of incoming secret payloads;
- hot secret rotation and encrypted durable-journal storage;
- provider-specific exporter plugins.

The non-normative reservation exploration lives in
[`budget-governance-draft.md`](../architecture/budget-governance-draft.md).
