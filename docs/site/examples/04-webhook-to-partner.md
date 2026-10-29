---
title: 'Webhook to partner: put it together'
description: Admit a webhook, validate and deduplicate it, then exercise bounded partner retries with privacy, secret and cost evidence.
teaches: HttpServer ACK, typed validation, explicit deduplication state, HttpClient, finite retry, personal-data policy, secret binding, exact reported cost
graph: examples/webhook-to-partner.rkg
golden: crates/rastok/tests/golden/gallery/webhook-to-partner/webhook.nominal.trace
command: env RASTOK_SHOWCASE_PARTNER_TOKEN=lab-only-not-a-credential RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION=fixture-v1 rastok simulate examples/webhook-to-partner.rkg --deployment examples/webhook-to-partner.deployment.json
expected: Six scenarios pass; ACK precedes business completion, retries are bounded, duplicates are suppressed during retention, and 500 is handled as a response.
break_it: 'Change Retry.attempts = 2 to Retry.attempts = 1, then rerun the same command without --save.'
limits:
  - The Lab injects decoded HTTP requests; malformed-wire refusal is qualified separately by the real Driver test.
  - 202 means admitted, not processed; validation and deduplication occur after ACK.
  - Deduplication lasts until five minutes of inactivity and does not survive a restart.
  - The partner must implement idempotent upsert by event_id; a DSL declaration cannot establish that external guarantee.
  - Emails and charges are synthetic; the example privacy profile is not production authority, and no real credential is needed.
  - The journal is in memory; Kafka, SQL, durable delivery and diagnostic recording of personal-data plans are not demonstrated.
order: 4
---

You know how to replace an effect, route its outcomes and isolate state. Now
use those ideas for a familiar integration: receive a webhook and deliver it
to a partner that is not always available. Build the flow and its failure
scenarios together, before you need a live partner environment.

All six scenarios have versioned goldens. The captures below show native
Retry retaining a failed attempt and publishing the final outcome.

## Run

From the repository root:

```sh
env RASTOK_SHOWCASE_PARTNER_TOKEN=lab-only-not-a-credential \
    RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION=fixture-v1 \
    rastok simulate examples/webhook-to-partner.rkg \
      --deployment examples/webhook-to-partner.deployment.json
```

Add `--ui` to inspect the same graph. No HTTP listener, real partner or paid
API is started. The value above is a **fictional fixture**, not a credential.
For real deployments, supply secret material out of band, never in source,
profile JSON or shell command text. The profile contains only its coordinate
and version. The same Contract is used with a scripted or physical Driver.

## Read the graph

Follow the main path first: `listener` admits, `validate` checks the business
data, `deduplicate` owns a `seen` flag per event, and `deliver` contains the
bounded partner retry. Open that scope to find `request`,
`partner : HttpClient + Retry` and `decide`. The response type `Receipt`
annotates the charge where the Connector retains it, before any decision to
hold or publish the attempt.
`decide` classifies the final business outcome; it contains no retry logic.

Then read the declarations that constrain the path: the idle lifetime, the
partner's idempotency key, the exhaustive destinations, `@personal` and the
secret reference. These are reviewable parts of the program and deployment,
not separate conventions the reader must guess from the happy path.

## Expected result

In `webhook.nominal`, admission produces the ACK before business validation;
the scripted partner then completes the first attempt.

```mermaid
sequenceDiagram
    participant lab as Lab: webhook.nominal
    participant listener as listener: HttpServer
    participant validate as validate
    participant dedup as deduplicate
    participant request as deliver.request
    participant partner as deliver.partner: HttpClient + Retry
    participant decide as deliver.decide
    participant app as webhook_to_partner
    lab->>listener: Decoded request: event_id = ok
    Note over listener: Admission ACK: Completed(202), disposed to nothing
    listener->>validate: out = decoded payload
    validate->>dedup: out = validated Webhook
    dedup->>request: First event_id: set seen, forward
    request->>partner: Body and meta.app.event_id
    partner->>lab: Scripted POST with idempotency key
    lab-->>partner: Receipt charge 0.02 USD, then Completed(201)
    Note over partner: Receipt measured in journal, out disposed to nothing
    partner->>decide: exit = Completed(201)
    decide-->>app: done = event_id ok, status 201
```

`6 scenario(s), 0 failed` covers:

- `nominal`: one `202` ACK and one partner Call ending with `201`;
- `timeout_then_success`: one second to timeout, one second of backoff,
  then success on the second attempt;
- `partner_dead`: two timed-out Calls, then the final `TimeoutExpired`
  inside the business `PartnerFailed` response, with
  no further Call even after another ten seconds;
- `duplicate`: an explicit `seen` state suppresses a second request with
  the same event id; after five minutes idle, a fresh instance admits it again;
- `invalid_payload`: decoded JSON with the wrong shape, then a negative
  amount, both rejected after ACK and without contacting the partner;
- `http_500`: `ServerError { status: 500 }` is a decoded response; the
  static `Retry.on` policy selects it, and the second response is `201`.

The email field is `@personal`. The deployment must attest the in-memory
processing before admission; having a valid secret binding alone is not
enough. Debugger views redact before formatting. Personal data can remain in
the in-memory causal journal: presentation masking is not erasure. The bearer
is different: its clear bytes never enter the journal at all.

The declared `output out: Receipt` interprets the fictional partner's `charge`
field as an exact amount with `@measure(id = amount, unit = "USD")`.
That declaration names a quantity and its unit, not a billing policy. No downstream accounting
Operator is run or replayed to extract it. The nominal reply reports
`0.02`; the `500` and successful replies retain two separate samples, `0.01`
and `0.02`, with their causal references. The first is Held by Retry and
never reaches `decide`; it remains inspectable in the journal. M1 computes no
total or price and the debugger displays the original DATA, not a cost report. A timed-out Call
supplies no charge: missing evidence is **unknown cost, not a zero bill**.

## Break it

Change the single `Retry.attempts = 2` to `Retry.attempts = 1`. The timeout and
`500` scenarios can no longer succeed on a second attempt. Rerun the command
above, without `--save`, then restore the source.

## Read the golden difference

Add
`--compare crates/rastok/tests/golden/gallery/webhook-to-partner`.
The CLI names the first differing record and field, expected versus actual,
and recorded causal context. This classified graph deliberately redacts
values in divergence messages; it does not expose an email to explain a
failure. Recorded operation facts can differ before the final answer does.
The versioned goldens contain **only the synthetic fixture data**; they are
not sanitised exports suitable for real personal data.

The following 30-second sequence visits recorded states of
`webhook.timeout_then_success`: admission, timeout, backoff and successful
second attempt. During the backoff, Retry retains the failed batch without
delivering it to `decide`; only the final result is published. These are real
debugger views of a simulation, not live HTTP requests or an edited branch.

![Thirty seconds through the recorded timeout and retry transitions](../assets/webhook-tour.gif)

Three stills make the checkpoints easier to inspect:

- [Before admission, transition 0](../assets/webhook-before.png).
- [Retry backoff scheduled, transition 20](../assets/webhook-backoff.png).
- [Success after two attempts, transition 34](../assets/webhook-completed.png).

## Carry forward

Read [Connectors and outcomes](../guide/03-connectors-and-outcomes.md) for the
admission boundary and [Running for real](../guide/05-running-for-real.md)
for deployment settings, secrets and diagnostic evidence.

This is the central exercise: one integration, several possible worlds, and
a recorded explanation when a revision changes behavior. Keep the timeout,
duplicate and 500 scenarios alongside the nominal one. Each describes a case
the next change must still handle; passing them is evidence under their stated
world assumptions, not proof that every real partner will behave that way.

Previous: [Two tenants](03-two-tenants.md). Next:
[Sensor alarm](05-sensor-alarm.md). The same approach can test an event that
never arrives, without making the test wait in real time.

## Limits

Malformed JSON, failed authentication and an excessive wire body are refused
before a root or ACK exists. A decoded Lab injection cannot exercise that
decoder. The separate physical loopback proof is:

```sh
cargo test -p rastok --features hosted-http12 auth_codec_and_body_refusals_never_invent_an_inbound_call
```

An instance key isolates state; `seen` does the deduplication. This is
accept-once within an **idle** retention window: duplicates extend that
window, and a permanently failed job is still considered seen until expiry.
It is not exactly-once delivery, a durable outbox, or automatic resubmission
after failure. A restart loses that state. The partner must honour the stable
idempotency key, upsert by `event_id`; the request and key remain identical
between attempts. Both retry paths rely on that external property. The example
does not test a real partner's idempotence.

The supplied privacy profile describes a local synthetic demonstration, not
legal authority for production processing. Real purposes, recipients,
transfers and retention require deployment-specific review. No persistent
personal-data journal or `run --record` is admitted by this example. Kafka
and SQL are not shown. Modify the scenario on disk to try another world;
there is no debugger branch or payload editor.
