# Contributing a Connector Driver

> Current contributor path through M1.6 H2. The normative contract lives in
> [`../spec/stdlib.md`](../spec/stdlib.md) and
> [D-2026-08-22-1](../spec/decisions.md). This guide explains how the pieces
> meet in the implementation; it does not define a second Connector model.

## The four objects

- A **Connector** is the Node written in an RKG graph.
- Its **Contract** is compiler data: stable id, required roles, ports, types,
  attributes, evaluation phases and lifecycle rules.
- A **Driver** is physical code that performs the external interaction.
- A **Binding** proves, before the VM and journal exist, which Driver object
  serves each active Connector Node.

No Rust crate name, Cargo feature, library path, Host, Worker or concrete
Driver version enters the semantic plan. The meeting key is the canonical
contract id, for example `rastok/file@1`.

```text
                         compile time

  RKG Connector ──symbol──▶ ContractCatalog ──check/lower──▶ validated plan
                                 │                    id + roles + exchange
                                 │
                                 └── declarative sheet; no Driver code

                         runtime assembly

  static factories ─┐
                    ├──▶ DriverRegistry ──bind exact id/roles──▶ RuntimeBindings
  future loader ────┘             ▲                                │
       │                          │                                │
       └─ DriverManifest           │                      ┌─────────┴─────────┐
          (data only)              │                      │                   │
                                                   inbound facts       outbound intents
                                                           │                   │
                                                           └──── VM + journal ┘

                         simulation

  Lab mock factory ──same id/roles/binding/protocol──▶ same VM + journal
                                      only the post-open world is scripted
```

## Two roles, three exchange shapes

`InboundConnectorDriver` retains and proposes one bounded external arrival at
a time. It does not mint root envelopes: sequencing, routing, admission,
identity and journal commit remain runtime authorities. The runtime answers
accepted only after commit, pending without a callback under pressure, or a
portable rejection.

`OutboundConnectorDriver` exposes exactly one exchange-specific outbound view:
`CallConnectorDriver` prepares/starts calls and reports correlated typed
completions; `SessionConnectorDriver` prepares/starts an opening and its
individual writes, then reports correlated open/write/terminal observations.
Neither writes records or manufactures scheduler outcomes such as timeouts.

A contract may require either role or both. A both-role factory returns one
bound object exposing both views over shared protocol state; registering two
unrelated objects is not equivalent. Event is exactly inbound; Session is
outbound or both-role because its open/write/terminal observations use the
exchange-specific outbound view.

Roles describe direction across the Rastok boundary. M1.6 adds an orthogonal
closed exchange fact describing lifecycle and cardinality:

```text
Event      : one spontaneous fact ──▶ one inbound root; no per-event exit
Call       : one admitted request ──▶ ordered outputs ──▶ one typed exit
Session    : one open ──▶ messages in both directions* ──▶ one typed exit
```

H1 makes all three exchange shapes explicit and executable at the generic
Driver/journal boundary. It replaces the pull-only inbound proposal with a
two-phase accepted/pending/rejected protocol before any live network Driver is
claimed. It does not add `DuplexConnectorDriver` or make the VM aware of
WebSocket.

The model covers the intended catalogue without new role kinds:

| Family | Typical role |
|---|---|
| `Process(job)`, `File(read/replace)`, HTTP client, LLM, database call | outbound call |
| HTTP server | inbound call |
| Clock, Calendar | inbound event |
| WebSocket client/server, queue consumer/producer, future `File.watch` plus writes | both-role session, when one contract really shares one protocol state |

Whether `File.watch` widens `rastok/file@1` or becomes a separate contract is
deliberately unresolved; the role model does not decide contract versioning.

## The contribution path

### 1. Publish the Contract

Add or extend one `ConnectorContract` sheet and give it one canonical
`ConnectorId`. Declare exact roles, exchange shape and all effective ports.
Attribute defaults, domains and their latest evaluation phase belong to the
sheet; a Driver never chooses a semantic default.

If decoded plans must be validated without the compiler catalogue, add the
small Registry-free core mirror and a product test proving every sheet row
agrees with it. Unsupported modes refuse by name rather than executing a
nearby behavior.

### 2. Separate pure exchange protocol from the effect

Put the family under `crates/rastok/src/connectors/<family>/`. Binding resolves
and validates every plan/boot/instance fact before external work. Call-time
settings arrive as typed values (the plan names this evaluation site
`Invocation`). Pure request preparation produces the exact
sealed request or a typed pre-open rejection.

Keep that protocol callable by both the real Driver and the Lab mock. For a
call it prepares one request and projects one completion. For a session
it prepares open/write facts, correlates inbound messages to one logical
`SessionRef`, and projects one terminal exit. Only the operation after the
journal boundary — filesystem access, process spawn, socket open/write —
differs between those worlds.

### 3. Implement the Driver roles

Implement only the required role interfaces and keep host-specific details at
their edge. Translate OS/library outcomes to the contract's portable tagged
union; do not leak raw errno, host paths or library error types into causal
values unless the contract explicitly declares them.

The Driver answers, proposes a session fact, or stays silent. The runtime owns
envelope birth, backpressure, journal ordering, timeout, close-once, bounded
Closing and typed port validation. An HTTP response, successful upgrade or
explicit application acknowledgement cannot precede the runtime's acceptance
callback. Transport ACK/window progress is only pacing and stays within the
Driver's bounded intake; WebSocket has no built-in per-message application
acknowledgement.

### 4. Register one factory

Implement `ConnectorDriverFactory` with the exact id and roles. `bind` may
validate plan facts and allocate local state, but it must not observe or mutate
the external world or launch work. `RuntimeBindings::try_new` calls each
required family factory once before execution and rejects missing, incomplete
or lying implementations.

Static builds explicitly construct the desired `DriverRegistry`. Minimal,
embedded and full standalone binaries may therefore link different families
without changing the plan model or adding VM branches.

The shipped standalone CLI is one such assembly: `rastok run` always registers
Process, and `--file-root <dir>` additionally registers File with an explicit
physical root. A graph never selects or discovers that root.

### 5. Supply a conformance profile

Run the real factory through the shared conformance battery. Select the
call or session profile and provide:

- one minimal source and happy terminal case;
- one deterministic provocation for every portably forceable terminal case;
- a written reason for every case that cannot be forced portably;
- a silence/timeout probe when applicable;
- an interleaving probe that makes crossed concurrent replies observable.

A session additionally proves open/close exactly once, bidirectional message
correlation, bounded pressure, post-exit silence, and recovery without blind
message replay. Fragmentation and control frames are family integration tests,
not application outputs. An event profile proves sequence/admission and has no
fabricated completion or per-event exit.

Add family-focused integration tests for physical behavior the generic battery
cannot force. A new family is not complete merely because its happy path runs.

### 6. Prove the Lab twin

Register a mock factory under the same id and roles. Reuse the family's pure
request preparation and scheduler-completion projection. Script only the
external outcome after the journal has opened the call. Prove malformed mock
outputs fail the ordinary resolved-port gate and that adding Lab source changes
neither semantic hash nor validated plan.

Put the pure scripted-protocol binder beside the real family module. The Lab
registry composes these family-owned binders; it must not grow a parallel
string match that re-implements the Connector catalogue.

### 7. Prove composition

Bind the family alone, in a reduced build that intentionally omits it, and
beside at least one unrelated family. The combined graph must need no concrete
family match in the VM. Replaying an already closed call must never repeat the
external observation or effect.

## Static builds and future plugins

Static registration is the only implementation shipped today. Cargo features
may offer build presets, but explicit registry contents remain the testable
authority.

`DriverManifest` is the data-only capability projection available to a future
plugin host: canonically ordered stable ids plus closed role claims, with
duplicates refused. A sidecar can be inspected before implementation code is
loaded. Its serialized file syntax, artifact locator, signature/trust policy,
loader protocol and native ABI are intentionally not frozen.

A future in-process loader or out-of-process host must ultimately provide an
ordinary `ConnectorDriverFactory` to the same `DriverRegistry`. Discovery is a
physical assembly choice, never an alternate VM, plan or simulation model.

## Review checklist

- No external observation occurs before the corresponding journal boundary.
- No Driver writes journal records or mints envelopes.
- No semantic default or type decision is duplicated in Driver code.
- No concrete Connector id is matched in the VM.
- Binding is complete before execution and registration order never selects.
- Real and mocked worlds share the same plan, request protocol and result gate.
- Every terminal case is proven or explicitly unprovable in conformance.
- Any semantic/package/causal identity move is declared before goldens change.
