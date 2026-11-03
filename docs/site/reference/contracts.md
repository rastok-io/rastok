---
title: Standard contracts
description: Contract identities, modes, ports, terminal cases, settings and Lab support from the executable Connector catalogue.
order: 3
---

This inventory follows `rastok connectors --json` in its emitted order.
Contract availability, Lab models and installed physical Drivers are separate
facts. A binary built with default features (no networking) lists File and Process
Drivers; networking requires the hosted networking features. Run the command
on the binary you deploy to inspect that assembly.

The [standard exit types](stdlib.md#types) are available
by name without imports. The physical Driver selection does not change those
type definitions.

A setting's latest phase is its evaluation ceiling: plan, boot, instance or
invocation. A setting may depend on values available by that phase, but
not on values that become available later. The compiler selects the earliest phase where its expression
closes. Omitted defaults become effective settings. Optional means absent,
not null. Byte sizes use `B`, `KiB`, `MiB` or `GiB`.

HTTP-derived Contracts share `headers` (a sparse typed record containing
`content_type: media_type`) and `custom_headers`
(`map<text,array<text>>`). Driver-owned headers cannot be overridden.
Certificate and private-key settings must be supplied together.
Declared version preferences are HTTP/3, HTTP/2, HTTP/1.1; the Driver uses
only compiled backends. `hosted-http12`, `hosted-http3` and
`hosted-websocket` select those capabilities. `hosted-network` enables the
hosted networking group.

`idempotent` is a repetition guarantee. `behavior` describes whether equal
requests in equal external state yield equal answers. They are distinct;
[Retry](../guide/07-loops-and-retries.md) also checks terminal visibility.
Sessions have no per-message Call exit or Retry attribute.

## Connectors

- [File](connectors/file.md): Read or atomically replace a file.
- [Process](connectors/process.md): Start a job or keep a resident process.
- [HttpClient](connectors/http-client.md): Send a request and receive its response and terminal outcome.
- [HttpServer](connectors/http-server.md): Admit a request and acknowledge it or await the graph's response.
- [WebSocketClient](connectors/websocket-client.md): Open a peer Session and exchange messages.
- [WebSocketServer](connectors/websocket-server.md): Accept peer Sessions and route their messages.

## Specified, not shipped

| Item | Specification |
|---|---|
| File `watch`: listed as checker deferred; attributes only, no Driver, Lab model, fixture or example | `docs/spec/stdlib.md` §3.7 |
| Timer: elapsed-time source | `docs/spec/stdlib.md` §3.4 |
| Calendar: civil schedule source | `docs/spec/stdlib.md` §3.5 |
| Memory: bounded append/snapshot store | `docs/spec/stdlib.md` §3.6 |
| HttpServer `respond` source scripting in the Lab | `docs/spec/simulation-semantics.md` §2 |
| Streaming HttpServer responses and seeded behavior | `docs/spec/stdlib.md` §8 |

Fixture names describe the evidence corpus, not checks performed by the
catalogue command. Source authority: `docs/spec/stdlib.md`, resolved
standard definitions and the current binary's catalogue. For authoring,
see [Connectors and outcomes](../guide/03-connectors-and-outcomes.md).
