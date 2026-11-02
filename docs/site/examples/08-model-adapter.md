---
title: 'HTTP model adapter: typed responses and token metering'
description: Select a typed view of an OpenAI Responses result, retain original token measurements by job and session, and distinguish absent usage from an observed zero.
teaches: Typed JSON responses, projected fields, original @measure samples, boot-bound secrets and inference without Retry
graph: examples/agent-http.rkg
golden: examples/traces/agent-http/responses.two_sessions.trace
command: RASTOK_SHOWCASE_MODEL_TOKEN=lab-only-not-a-credential RASTOK_SHOWCASE_MODEL_TOKEN_VERSION=fixture-v1 rastok simulate examples/agent-http.rkg --deployment examples/agent-http.deployment.json --compare examples/traces/agent-http
expected: Six scenarios pass; three jobs retain their session and usage, missing usage stays unknown, and observed zero stays zero.
break_it: 'Change Hello from the simulated model. to A different model answer. in the mock and rerun the comparison; responses.two_sessions diverges at record 6.'
limits:
  - Mocks supply typed outcomes; wire JSON deserialization is qualified separately by local tests, not by a live paid model call.
  - Token quantities are original observations, not a price calculation or a billing report.
  - A model inference can be charged again; this example supplies no idempotence guarantee and has no Retry.
  - This boot-dependent deployment profile refuses diagnostic recording; its bearer is not copied into a recording.
order: 8
---

A bounded loop tells you how much work may repeat. At a model boundary,
another question matters: what usage did the partner actually report?
Keep that evidence on the original typed response, without turning an absent
field into a zero or counting a downstream copy again.

## Run

From the repository root, use the synthetic fixture bindings and the checked
[deployment profile](../../../examples/agent-http.deployment.json):

```sh
RASTOK_SHOWCASE_MODEL_TOKEN=lab-only-not-a-credential RASTOK_SHOWCASE_MODEL_TOKEN_VERSION=fixture-v1 rastok simulate examples/agent-http.rkg --deployment examples/agent-http.deployment.json --compare examples/traces/agent-http
```

Add `--ui` for the paused debugger. The Lab never contacts the profile's
configured model URL. These fixture values are not credentials.

## Read the program

Start with `ResponseUsage` and `ModelResponse`. The former marks the three
original quantities with `@measure(id = ..., unit = "token")`. The latter
is an explicit selected view of an external JSON object: response identity,
output, status, usage and error.

The `model` Connector refines its existing port with `output out: ModelResponse`.
Under `codec = json` and `json_profile = projected`, deserialization keeps
declared members, omits additional members and turns an absent nullable
member into null. A member with the wrong type still produces `DecodeFailed`.
Ordinary structs and strict JSON deserialization remain closed.

`request` carries `session` and `job` into META. `explain_usage` emits an
explanation when usage is absent; it neither extracts nor sums measurements.
`terminal` separates a usable 2xx response from other HTTP outcomes. One job
means one non-streaming inference, not a resident conversation.

The bearer is a logical `secret("showcase/model/token")` reference. Its value
is bound at boot and supplied only at the real HttpClient Driver boundary.
`behavior = probabilistic` describes the partner's outputs. It does not mean
that repeating a request is safe: there is no Retry on this inference.

## Expected result

The CLI ends with `6 scenario(s), 0 failed`:

| Scenario | Evidence |
|---|---|
| `two_sessions` | Alice's two jobs and Bob's job retain their own session and original response; each fixture reports 12 input, 4 output and 16 total tokens |
| `missing_usage` | Absent/null usage produces an explanation and no measurement sample |
| `observed_zero` | Three explicitly reported zero quantities remain observations |
| `schema_failure` | `DecodeFailed` supplies no partially usable `model.out` |
| `transport_failure` | A ten-second logical timeout produces no usage and no repeated inference |
| `http_errors` | A decoded 500 is `ServerError`; an incompatible error body is `DecodeFailed` |

The three token quantities are separate facts, not three numbers to add
together. Input plus output already accounts for the reported total in this
fixture; the graph invents neither a monetary amount nor a missing quantity.

## Break it

In the `simulated_message` constant, change `Hello from the simulated model.`
to `A different model answer.`. Keep the expected trace unchanged and rerun
the comparison command. The counts still match, but `responses.two_sessions`
fails its golden comparison. The other five scenarios still pass.

## Read the golden difference

The measured first divergence is `record 6: closure(32)[1][0][1][1]`:
the expected encoded value has 399 bytes and the changed one 393 bytes.
The cause is `EGRESS "model"`, with no Call key. The differing external
response is recorded before downstream interpretation, even though the
scenario's count assertions still hold. Restore the text; do not save over
the reference to hide the changed answer.

## Carry forward

Previous: [Countdown](07-countdown.md). Use an explicit response type when
a partner supplies usage or another business quantity. Keep each original
sample and its provenance, including an honest absence. Compare this short
adapter with the resident [Agent gateway](06-agent-gateway.md): the same
typed-boundary discipline applies to two different interaction lifetimes.

## Limits

The Lab scripts typed responses. Separate local wire tests, described in
the [projected JSON qualification](../../architecture/openai-responses-hydration.md),
exercise full JSON deserialization; no paid OpenAI invocation is claimed.
An API can change its payload and fail deserialization at runtime.

No reporting engine, pricing rule or automatic inference retry is supplied.
Missing usage remains unknown. The in-memory journal is not durable recovery.
The boot-dependent profile also refuses `run --record`; use these versioned
Lab traces rather than treating a deployment credential as a replay input.
