---
title: Running for real
description: Move from scripted outcomes to physical Drivers while keeping deployment settings, secrets, evidence and alpha limits explicit.
order: 5
---

After this chapter you can choose between simulation, a real run and
diagnostic replay. You can identify the deployment information that a
graph needs, keep credentials out of source, and distinguish a reviewed
golden from a recording of one real execution.

## Change the world, not the graph

`rastok run` compiles the App and uses physical Drivers. Its inputs and
external responses are no longer supplied by Lab scenarios. Before trying
a network integration, inspect `rastok connectors` in the binary you will
use: checker support and an installed physical implementation are separate
requirements. Getting Started installs the documented hosted features.

Echo is a small local first run. From the repository root, with a new
recording filename:

```sh
printf 'hello\n' | rastok run examples/echo.rkg --record echo.rkrec
rastok replay echo.rkrec
```

The first command starts the real `cat` process and prints its echoed
line. Replay prints the recorded App output and the same causal seal
without starting `cat` again. The original source file is not needed by
the recording reader. An existing recording path is refused rather than
overwritten.

## Supply deployment facts explicitly

The webhook example needs more than a command name. Its partner Connector,
from `examples/webhook-to-partner.rkg`, lines 90–106, separates a URL
binding and a secret reference from the application logic:

```rkg
connector partner : HttpClient + Retry {
  url = env("PARTNER_URL")
  auth_bearer = secret("showcase/partner/token")
  method = "POST"
  codec = json
  output out: Receipt
  timeout = duration("1s")
  max_body = "16 KiB"
  idempotency_key = meta.app.event_id
  // Required partner contract: honour this key for the identical request.
  // This declaration does not prove that a real service honours that contract.
  idempotent = true
  behavior = deterministic
  Retry.attempts = 2
  Retry.on = [ServerError, TimeoutExpired, DnsFailed, ConnectFailed]
  Retry.delay = { kind = fixed; after = duration("1s") }
}
```

In the alpha, `env()` is admitted at `HttpClient.url`, and `secret()` at
`HttpClient.auth_bearer`. They are not general environment-reading functions
available in every expression. The deployment JSON names non-secret values
and the secret's provider coordinate and version. Actual secret material
is supplied out of band; the provider version must match the attestation.
Do not paste a credential into the graph, profile JSON or a shell command.

`examples/webhook-to-partner.deployment.json` is a synthetic Lab profile,
not production authorization. The example's Run section supplies an
explicitly fictional token and its matching version. The same profile also
attests the personal-data policy required by the annotated email field.
Missing or contradictory coverage refuses before effects. Redaction is
not permission to persist that data indefinitely.

## Repeat only under a real guarantee

`Retry.attempts = 2` includes the first attempt. The manager retains the
same request, waits the declared delay after a selected terminal, then
tries again within that bound. Intermediate attempt facts remain in the
journal; only the final attempt is delivered downstream.

The partner must actually honor the idempotency key for the identical
operation. Declaring `idempotent = true` cannot make an arbitrary remote
action safe to repeat. A timeout alone does not prove that it did nothing.
`Loop` instead advances application state, and keyed instances own isolated
state and lifetime. Their detailed protocols belong to later chapters;
[Two tenants](../examples/03-two-tenants.md) already demonstrates isolation.

## Choose the evidence you need

A golden is a reviewed canonical trace of a Lab scenario. `simulate
--compare` checks stored references without changing them and names the
first divergence in each differing scenario. `--save` creates or replaces
references, so review the semantic reason before saving. Neither option
supplies mock responses; those remain in the Lab.

A diagnostic recording contains the real run's plan and journal evidence.
Replay reconstructs that evidence, not a fresh execution or a generated
scenario. The journal is currently in memory. A recording is bounded local
diagnostic storage, not a durable deployment journal, automatic restart
mechanism or share-safe export.

Do not apply the Echo recording command indiscriminately to the webhook
graph. The current recorder refuses personal-data plans, nonempty deployment
attestations and boot-dependent Connector settings before file creation or
effects. Ordinary unclassified payloads can still be sensitive. Protect the
file rather than assuming the debugger's masking rewrites its stored bytes.

`simulate --ui` opens paused and executes the scenario forward under your
control. Its history controls inspect recorded transitions. `replay --ui`
only navigates recorded prefixes, with no Driver I/O. Neither edits the
graph or forks a new world, and `run --ui` is not supported.

The webhook's typed response also retains original annotated measurements
before Retry decides whether to hold or publish an attempt. Missing usage
is not zero. The alpha does not calculate a bill, aggregate costs or enforce
a Budget Extension. These limits are listed with the other
[current capabilities](../index.md#what-is-available-today).

The normative reading is `docs/spec/execution-model.md`,
`docs/spec/observability.md`, `docs/spec/annotations.md` and `docs/spec/ui.md`.
The recording contract is `docs/architecture/diagnostic-recording.md`.

## Summary

- Real execution needs compatible physical Drivers and explicit deployment facts.
- A secret reference is not its material; an attestation is not a source literal.
- Goldens compare scenarios; recordings reconstruct one captured run.
- Retry safety and privacy still require real external guarantees.

## Next

Continue by example with [Webhook to partner](../examples/04-webhook-to-partner.md).
Run its synthetic scenarios, inspect the bounded retry, then review what a
real partner and deployment must supply. Return to
[Getting Started](../getting-started.md#two-ways-to-learn) to choose another
example; this is the last chapter of the opening guide.
