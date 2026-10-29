---
title: Connectors and outcomes
description: Read a Connector Contract, choose a supported mode, and route data and terminal outcomes explicitly.
order: 3
---

After this chapter you can distinguish a Connector declaration from its
Driver, find the supported modes in your binary, and route both useful
data and terminal outcomes. You can explain why admission, completion and
business success are different facts.

## Ask the catalogue first

Run this command from the checkout or an installed CLI:

```sh
rastok connectors
```

It lists compiled capabilities without starting a Driver. Read the exact
Contract id, mode, port types, required attributes, checker support, Lab
support and installed implementations separately. A Contract known to the
compiler does not imply that your binary includes its physical Driver.
Likewise, listing a conformance fixture does not mean the command ran it.

The executable standard families are Process, File, HttpClient, HttpServer,
WebSocketClient and WebSocketServer. The catalogue also describes deferred
families or modes. A File `watch` entry is not permission to use it today.
The [current status](../index.md#what-is-available-today) remains the starting
point for the alpha's limits.

## One Contract, different operations

The reader in `examples/file-pipeline.rkg`, lines 22–27, declares one
bounded operation per input:

```rkg
connector read_file : File {
  mode = read
  path = data as text
  max_read = "64KiB"
  timeout = duration("2s")
}
```

`path` is resolved for the invocation from its input. It is a logical path
relative to the root supplied to the physical File Driver, not permission
to read an arbitrary host path. That root is a resolution base, not a
portable security sandbox. The read publishes the complete bytes or no
`out` value; exceeding `max_read` produces `ReadLimitExceeded`.

The writer, lines 38–42, selects another mode of the same Contract:

```rkg
connector write_file : File {
  mode = replace
  path = "copy.txt"
  timeout = duration("2s")
}
```

It consumes bytes and reports a terminal outcome. Replacement is atomic
where the platform supports the promised operation; otherwise the result
is `AtomicReplaceUnavailable`, not an in-place fallback. Atomic visibility
does not promise crash durability. Directories are refused by name rather
than traversed as an implicit collection of files.

A mode resolves the operation's direction, exchange and port behavior.
It does not select a vendor or library. The physical Driver and the Lab
model implement the same checked Contract. Simulation substitutes the
external work; it does not bypass request validation or invent new ports.

## Data is not the terminal outcome

For File `read`, `out` carries bytes and `exit` closes the Call. For File
`replace`, successful `out` is impossible; completion is still reported
on `exit`. Process uses `err` for stderr. HTTP Contracts have no implicit
`err`. The names are conventions whose actual meaning comes from the
resolved Contract, not three universal success and fault channels.

The file Graph makes both final dispositions explicit, at lines 49–50:

```rkg
read_file.exit -> match { Completed -> nothing; _ -> failed }
write_file.exit -> match { Completed -> nothing; _ -> failed }
```

Successful terminals are deliberately discarded. Other cases cross the
App's `failed` output. These emissions remain observable in the journal
even when their destination is `nothing`. An invocation's `exit` does not
stop the whole Node instance; an instance may serve further invocations.

## Admission is not business success

The webhook listener, from `examples/webhook-to-partner.rkg`, lines 35–40,
uses the default `ack` mode:

```rkg
connector listener : HttpServer {
  listen = "127.0.0.1:8080"
  path = "/events"
  codec = json
  max_body = "16 KiB"
}
```

A successful accepting commit precedes the HTTP 202 response. That response
means admitted, not processed. Decoded but business-invalid data can then
reach the validation Operator and be rejected. Malformed JSON refused
before admission creates no invented graph Call or `exit` to assert on.
A decoded Lab injection therefore cannot qualify the physical JSON decoder.

For an outbound HttpClient, a usable decoded 500–599 response produces
`ServerError` and a response body, not a transport fault. Other decoded
statuses use `Completed`; the graph still decides which mean business
success. An incompatible response body produces `DecodeFailed`. A timeout
does not establish that the remote side did nothing.

Before asking an outbound Driver to act, the runtime commits the operation's
obligation. Its completion then returns through the typed boundary and
journal. In this alpha that journal is in memory; ordering before an effect
is not a claim of durable recovery after process loss.

The normative descriptions are `docs/spec/stdlib.md`,
`docs/spec/delivery-semantics.md` and `docs/spec/envelope.md`. Checking rejects
unknown settings, incompatible port types and missing possible-output
destinations before execution. Real-world compliance still needs separate
Driver and integration evidence.

## Summary

- Contract, mode and physical implementation answer different questions.
- Payload outputs and the terminal `exit` are independently typed.
- Explicit dispositions account for every possible result.
- Admission and a decoded HTTP response are not business success.

## Next

Continue with [The Lab](04-the-lab.md). Use
[File pipeline](../examples/02-file-pipeline.md) to inspect named file
refusals, then [Webhook to partner](../examples/04-webhook-to-partner.md)
to follow admission, validation and the partner's terminal outcome together.
