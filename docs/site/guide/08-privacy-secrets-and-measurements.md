---
title: Privacy, secrets and measurements
description: Classify typed data, bind credentials outside the graph, and retain exact business quantities on their original responses.
order: 8
---

After this chapter you can distinguish a data annotation from its runtime
interpretation, prepare the supported secret and privacy bindings, and
locate a measurement in the original response. A value's type answers what
it contains; its annotations state how that value must be handled or read.

## Declare metadata on a type

An annotation-capable type uses `type Name = @struct { ... }`. Its argument
fields are flat: scalars, enums and their nullable forms, rather than nested
records or collections. The same declaration can describe ordinary data or
typed metadata. Annotation arguments are checked plan constants.

A user-declared annotation supplies descriptive metadata, not a handler or a
security hook. Standard annotations have registered consumers in the checker,
runtime and inspection tools. Reproducing a standard annotation's field shape
does not acquire that authority. See `docs/spec/annotations.md` §§1–5.

## Classify data where it enters the schema

`examples/webhook-to-partner.rkg`, lines 3–7, identifies the email explicitly:

```rkg
type Webhook = struct {
  event_id: text
  email: text @personal(identifier = true)
  amount: int
}
```

`@personal` is a classification, independent of confidentiality. Its optional
`identifier`, `special` and `criminal` flags describe the field's role. A
personal-data plan requires an attested deployment profile covering its
processing and recipients before execution. The supplied webhook profile is
synthetic evidence for the demonstration, not authority for real processing.

`@sensitive(level = ...)` orders `internal`, `confidential` and `restricted`.
Copies preserve policy; derived values conservatively carry relevant taint.
The CLI and debugger redact before formatting. Redaction of a screen is not
an erasure policy for stored data, nor a promise that arbitrary unannotated
payloads are safe to share.

## Keep credentials outside DATA

`@secret` describes a stronger handling class. The executable credential path
is reference-based: a logical `secret("path")` at an admitted Connector setting
is bound by the deployment, then resolved at the physical Driver boundary.
The clear value is not an ordinary graph `Value`. Raw secret-bearing schemas
on executable ports are refused; annotating a string is not a way to persist
it safely in the journal.

The current sites are specific: `HttpClient.url` accepts an exact `env()`
reference and `HttpClient.auth_bearer` an exact `secret()` reference. Read
`examples/agent-http.rkg`, lines 56–66:

```rkg
connector model : HttpClient {
  url = env("MODEL_URL")
  auth_bearer = secret("showcase/model/token")
  method = "POST"
  codec = json
  json_profile = projected
  output out: ModelResponse
  timeout = duration("10s")
  max_body = "256 KiB"
  behavior = probabilistic
}
```

The deployment JSON names the logical secret, environment-provider coordinate
and immutable version. Supply the value out of band; its `<NAME>_VERSION`
companion must match. The graph, request digest and journal keep references,
not the bearer bytes. `--deployment` supplies the same binding contract to
real and simulated execution. General `env()` expressions at other sites are
not part of this profile. See `docs/spec/observability.md` §5.

## Read quantities from original responses

The adapter's response schema maps JSON members by name before committing the
typed response. `examples/agent-http.rkg`, lines 8–14, supplies its measurement
meaning alongside the numerical types:

```rkg
type ResponseUsage = struct {
  input_tokens: uint @measure(id = input_tokens, unit = "token")
  output_tokens: uint @measure(id = output_tokens, unit = "token")
  total_tokens: uint @measure(id = total_tokens, unit = "token")
  input_tokens_details: JsonObject?
  output_tokens_details: JsonObject?
}
```

`@measure` has exactly `id` and `unit`. The integer or exact decimal field
supplies the quantity; signed types can retain refunds or corrections. A
currency is an ordinary unit. Each id has one unit throughout the plan.
Missing usage supplies no sample; an observed zero remains evidence.

The journal retains these values in their original envelopes. Read-only
inspection pairs them with validated annotations and causal provenance.
Retry's held attempts keep their own samples, even when no downstream
Operator sees them. Consuming a held batch creates no second sample. Treat
input, output and inclusive totals as separate observations, not as three
quantities to add blindly.

## Generate types from an API description

`rastok --resolve-open-api <project-dir> [--check]` discovers static HTTP
endpoints, selects matching OpenAPI 3.0/3.1 documents and writes deterministic
`openapi.generated.rkg` aliases. It leaves authored files untouched.
`--check` writes nothing and refuses missing or stale generated source.
Dynamic or ambiguous endpoints and schemas outside the lossless subset
refuse by name. Choose where the generated types constrain ports; every
actual response still has to conform at runtime.

## Summary

Annotations classify typed data; deployment bindings supply external authority;
measurements preserve original quantities. Reporting, automatic pricing,
budgets and durable personal-data storage remain outside the
[current alpha scope](../index.md#what-is-available-today). Normative details
are in `docs/spec/annotations.md`, `docs/spec/observability.md` and
`docs/spec/schema-projections.md` §5.

## Next

Continue with [Command line](09-command-line.md). Practice with the deployment profile in
[Webhook to partner](../examples/04-webhook-to-partner.md), then compare absent
and zero usage in the [HTTP model adapter](../examples/08-model-adapter.md).
