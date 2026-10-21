# Contributing a Connector Driver

> Current implementation, updated 2026-10-07; version authority:
> [format registry](../spec/identity.md).
> D-2026-09-05-1 established the model implemented by R3–R9: Connector is
> one of the two Executor forms, its semantic transition comes from a
> versioned Contract, all productible outcomes have explicit
> dispositions. Process job/service use `std/process@4` / `@5`; File and
> network Contracts use `std/…@3`.
> Loop extends a Call's existing terminal type; it creates
> neither a new Contract id nor a contributed port.
> The normative contract lives in
> [`../spec/stdlib.md`](../spec/stdlib.md) and
> [D-2026-08-22-1 through D-2026-08-27-3](../spec/decisions.md). This guide
> explains how the pieces meet in the implementation; it does not define a
> second Connector model.

## The four objects

- A **Connector** is the Executor Node written in an RKG graph.
- Its **Contract** is compiler data: stable id, required roles, ports, types,
  attributes, evaluation phases and lifecycle rules.
- A **Driver** is physical code that performs the external interaction.
- A **Binding** proves, before the VM and journal exist, which real or
  effectless Driver object serves each active Connector Node.

An inbound binding may reserve every passive listener governed by its Contract so a
partial bind cannot expose a half-live service. This is fail-fast host
provisioning, not causal execution: it cannot acknowledge a request, identify a
fact, or perform an outbound effect. Any arrival in the short pre-Worker gap
remains pending under the Driver's ordinary finite capacities.

No Rust crate name, Cargo feature, library path, Host, Worker or concrete
Driver version enters the semantic plan. The meeting key is the canonical
Contract id, for example `std/file@3`.

```text
                         compile time

  RKG Connector ──symbol──▶ ContractCatalog ──check/lower──▶ validated plan
                                 │                    id + roles + exchange
                                 │
                                 └── declarative sheet; no Driver code

                         runtime assembly

  static factories ─┐
                    ├──▶ DriverRegistry ──bind exact id/roles──▶ RuntimeDrivers
  future loader ────┘             ▲                                │
       │                          │                                │
       └─ DriverManifest           │                      ┌─────────┴─────────┐
          (data only)              │                      │                   │
                                                   INGRESS facts       committed outbound intents
                                                           │                   │
                                                           └──── VM + journal ┘

                         simulation

  Lab mock factory ──same id/roles/binding/protocol──▶ same VM + journal
                                      only the external world is scripted
```

### Simulation does not require the physical implementation

The Contract and its pure protocol rules must be executable independently of
the physical Driver. `check` needs those rules; `simulate` binds a Lab model;
`run` binds real I/O. Request framing affecting a digest is shared pure code,
not reimplemented in the mock and not hidden behind a networking feature.
Port types alone do not describe an exchange's lifecycle or cardinality.

The HttpServer ack Lab model owns one bounded decoded-request proposal and one
acknowledgement slot. It uses `RuntimeDrivers::poll_inbound` and the generic
inbound Call terminal queue; the core still assigns the root identity, Call reference,
routes and journal records. The scenario selects `listener.out`, not an extra
test-only App input. Pending admission cannot acknowledge or overwrite the
request. HttpServer respond and Session source scripting remain check-time
gates; passive role binding is not proof of a complete simulation model.

This model scripts a successful fixed ACK after a decoded request. It does not
claim to test the wire decoder, TLS, auth, body limits or real 202 delivery.
Those are physical Driver conformance obligations. See
[simulation-semantics §2](../spec/simulation-semantics.md#2-the-fixture-is-the-journals-world-half)
and D-2026-09-09-3 for the separation.

## Two roles, three exchange shapes

`Ingress` retains and proposes one bounded external arrival at
a time. This world-originated fact enters the Connector instance's ordinary
mailbox only after INGRESS admission. The Driver does not assign root identities:
sequencing, routing, admission, identity and journal commit remain runtime authorities. The runtime answers
accepted only after commit, pending without a callback under pressure, or a
portable rejection. A proposal also names the exact Connector Node that owns
its source output through its single `IngressProvenance` authority.
Registry binding proves that path belongs to the Contract; the VM proves the
output row. An App-boundary injection remains a separate explicit API
category, never a provenance-string heuristic.

For a Call-shaped inbound contract, the same bound object also exposes
`InboundCall`. The accepting INGRESS fact returns an
`InboundCallId` only after it has committed the root and exact source
instance. For fixed acknowledgement, the terminal view later reports one
correlated `Completion`; the VM validates it and commits one zero-cursor
`l:reply` Connector transaction with exactly one final `exit`.

For a graph response, the same view first receives an `InboundCallResponse`
through `start_response`. Its envelope and destination were already committed
by the producing graph transaction. `Pending` means no protocol bytes were
written and leaves the exact response behind the Connector's ordinary claimed
mailbox; `Started` transfers the bounded attempt and obliges the Driver to
report its one terminal event through `events`. The resulting `l:reply`
consumes that real delivery cursor. Neither form is an outbound Call, and the
Driver never manufactures a mailbox delivery or writes a causal record.

`Outbound` exposes exactly one exchange-specific outbound view:
`OutboundCall` prepares/starts calls and reports correlated typed
completions; `Session` prepares/starts an opening and its
individual writes, then reports correlated open/write/terminal observations.
A correlated result for an existing outbound exchange is not a second
world-originated INGRESS root: it settles the already committed obligation and
then enables the Connector's ordinary core-owned transition. Neither Driver
writes records or manufactures scheduler outcomes such as timeouts.

Session shutdown has three separate handoffs: `close_session` requests
cooperative protocol closure during bounded Closing; `abort_session` requests
forced local cleanup at its retained settlement deadline; `release_session`
notifies the Driver after the sole terminal transaction has committed. The
last two must not wait for peer cooperation, allocate an unbounded cleanup
queue or start another grace period. They are fenced by the exact `SessionId`
and cannot affect its successor. Aborting does not fabricate a normal exit or
forget correlation early. Resource-free Lab Drivers may use the no-op defaults;
physical shutdown needs separate qualification. In particular, these common
seams alone do not qualify physical signals or a hosted WebSocket abort.
The [resident Process tests](process-service.md) separately qualify Unix
signal handoff, frame ordering and cleanup under backpressure.
The hosted WebSocket Driver still uses its existing protocol-close path and
the new callbacks' no-op defaults, not a newly implemented forced-close path.

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

H1 makes all three exchange shapes explicit and lands the generic Session
fold plus two-phase accepted/pending/rejected admission. H5 completes the
generic inbound Call half with its source-instance opening and close-once
terminal transaction, then attaches ack/respond H1/H2/H3 Drivers. H5.2's
direct-lineage response return includes exact pre-commit selection for a keyed
source and consume-only orphan handling. H6 attaches both WebSocket Contracts
to the existing Session view. None adds `DuplexConnectorDriver` or makes the
VM aware of HTTP or WebSocket.

## The hosted network seam

Network Drivers do not each choose a runtime or expose an async interface to
the VM. H3 gives client and server implementations one private physical pump
interface; each bound engine owns its current-thread runtime:

```text
 synchronous Driver / VM
          │
          ├── bounded work queue ───────────────┐
          ├── bounded control queue (priority)  │
          │                                     ▼
          │                          current-thread Tokio pump
          │                           ┌────────┼─────────┐
          │                        Hyper H1  Hyper H2  quiche H3
          │                           └────────┼─────────┘
          │                                  sockets
          │                                     │
          └── bounded fact queue ◀───────────────┘
                    shared byte budget
```

The control lane is separate so saturation by calls/messages cannot starve a
cancel. Queue slots and bytes are both finite. Work bytes remain charged while
queued and while the runtime owns the work; fact bytes remain charged until
the synchronous side drains them. An inbound Driver that retains a proposal
after that drain must move the reservation into the retained proposal, as
HttpServer does, so a pending causal admission cannot make memory appear free.
Protocol receive windows must not exceed the same per-engine physical budget.
Dropping the synchronous handle cancels and joins its pump. These rules apply
equally to inbound and outbound Drivers; they do not change the two semantic
roles.

For Call Drivers, finite outbound pressure is expressed by the closed
`CallStart::Pending` verdict. The already-journaled intent stays open, no
effect starts and the DriverPoller offers it again after capacity returns.
`Started` transfers ownership exactly once. A Driver must never translate a
full local queue or byte semaphore into a domain result such as
`ConnectFailed`; backpressure changes pacing only.

Static features select physical code only: `hosted-http12` supplies
Hyper/Rustls, `hosted-http3` supplies quiche/BoringSSL,
`hosted-websocket` supplies Tungstenite plus its H1/H2 transport, and
`hosted-network` combines them. `connector::drivers::assembly::hosted_registry` is the
one shipped composition function: Process is always present, File requires an
explicit physical root, and each network Contract follows those linked
capabilities. A missing Contract refuses during global binding before VM or
journal construction. No library type leaves `connector/drivers/network`, enters a
plan, or enters `rastok-core`. HTTP/1.1 Upgrade and HTTP/2–3 Extended CONNECT
are separately proved physical adapters over the same Session protocol; the
shared message codec alone is never transport evidence.

H4 is the first live consumer of this seam. The hosted HttpClient uses H1
origin-form requests, a pooled multiplexed H2 sender and verified quiche H3
while preserving one semantic request descriptor and one Call completion
shape. Request/response bodies and headers are finite, and the selected wire
version appears only in `meta.ext.http_version`. The flat contract-specific module
`connector::drivers::http_client` owns semantic request binding; the private
`connector::drivers::network::http_client` owns sockets and protocol engines.

H5.1 is the first live inbound consumer. The flat
`connector::drivers::http_server` module resolves the complete ack-mode listener and
route contract before a socket opens; private `network::http_server` owns Hyper,
Rustls, connection/stream permits and byte reservations. HTTP/1.1 and HTTP/2
both retain the request until the VM answers the same proposal. Only an
accepted-after-commit decision permits 202, and the fixed terminal observation
returns through the bounded fact lane rather than a detached task. Final H5
adds HTTP/3 and the live graph-driven `respond` profile above H5.2's generic
direct-lineage response handoff. All three versions retain the same Call and
bounded close-once rules.

H6 is the live Session consumer. WebSocketClient and WebSocketServer use the
same `Session` fold over H1 Upgrade, RFC 8441 and RFC 9220.
Opening, each write and terminal publication are separately correlated by
journal-derived `SessionId`/`SessionWriteId`; pending work stays within the
ordinary mailbox bound. A server object owns a multi-session table, while a
client Node instance owns at most one active connection. The contract-specific modules
own settings and portable outcomes; private `network::websocket` owns sockets,
handshakes, fragmentation, control frames and transport progress.

H4.1 makes the header boundary contributor-facing and typed. A Contract sheet
declares well-known request fields in the sparse `headers` struct and leaves
extensions in `custom_headers: map<text,array<text>>`; the Contract binder must
combine and validate them before EGRESS open. Exact peer response bytes stay
in `meta.ext.http_headers`, while valid standard observations such as
`content_type: media_type` receive their typed projection. Shared modules are
named after the responsibility they own (`websocket_protocol`, `network`), not
after the vague fact that several Contracts happen to use them.

The model covers the intended catalogue without new role kinds:

| Contract | Typical role |
|---|---|
| `Process(job)`, `File(read/replace)`, HTTP client, LLM, database call | outbound call |
| HTTP server | inbound call |
| Timer, Calendar | inbound event |
| Process service, WebSocket client/server, queue consumer/producer, future `File.watch` plus writes | both-role session, when one contract really shares one protocol state |

Whether `File.watch` widens File's Contract again or becomes a separate
contract is
deliberately unresolved; the role model does not decide contract versioning.

## The contribution path

### 1. Publish the Contract

Add or extend one `ConnectorContract` sheet and give it one canonical
`ContractId`.
Declare exact roles, exchange shape and all effective ports.
Also declare application profile, frame cardinality, terminal cases,
effect-visibility stage, and productibility evidence. These fields are what
move the six shipped Contracts to `@2`; removing implicit `unwired` policy does
not erase the old `@1` meaning.
Attribute defaults, domains and their latest evaluation phase belong to the
sheet; a Driver never chooses a semantic default.

If decoded plans must be validated without the compiler catalogue, add the
small Registry-free core mirror and a product test proving every sheet row
agrees with it. Unsupported modes refuse by name rather than executing a
nearby behavior.

When a boundary returns a provider quantity or amount, expose it as ordinary
typed payload data and annotate the exact field with `@measure`. A monetary
amount carries currency and basis, for example `@measure(id = amount,
currency = USD, basis = provider_reported)`; `@cost` is retired. The approved
business model uses simple author-chosen ids, not an OTel naming requirement;
the [annotation handoff](annotation-types-m1.md) distinguishes that target
from the still-closed numerical registry. Cache/reasoning token subsets must
remain separate fields with their inclusion relation, never a second charge.
Never put a credential value in such a payload: Connector
authentication consumes a `secret()` reference at its declared setting, and
raw `@secret` envelope fields refuse at source check and remain fail-closed at
VM boot for foreign packages. Physical price rows belong
to the deployment `PriceBook`, keyed by the exact Connector path, rather
than to Driver code or graph identity.

### 2. Separate pure exchange protocol from the effect

Put the Connector implementation under `crates/rastok/src/connector/drivers/<connector>/`. Binding resolves
and validates every plan/boot/instance fact before external work. Call-time
settings arrive as typed values (the plan names this evaluation site
`Invocation`). Pure request preparation produces the exact
sealed request or a typed pre-open rejection.

Keep that protocol callable by both the real Driver and the Lab mock. For a
call it prepares one request and projects one completion. For a session
it prepares open/write facts, correlates inbound messages to one logical
`SessionId`, and projects one terminal exit. Only the operation after the
journal boundary — filesystem access, process spawn, socket open/write —
differs between those worlds.

### 3. Implement the Driver roles

Implement only the required role interfaces and keep host-specific details at
their edge. Translate OS/library outcomes to the contract's portable tagged
union; do not leak raw errno, host paths or library error types into causal
values unless the contract explicitly declares them.

A hosted network implementation uses the shared bounded pump. Async work,
socket handles, Hyper/quiche/Tungstenite values and TLS contexts remain below
that module. The Driver-facing side is synchronous and exchanges owned Rastok
facts because queued or asynchronous boundaries must own their data.

The Driver answers an outbound Call, proposes an INGRESS fact, reports an
inbound Call terminal or Session observation, or stays silent. The runtime
owns envelope birth, backpressure, journal ordering, timeout, close-once,
bounded Closing and typed port validation. An HTTP response, successful
upgrade or explicit application acknowledgement cannot precede the runtime's
acceptance callback. Transport ACK/window progress is only pacing and stays
within the Driver's bounded intake; WebSocket has no built-in per-message
application acknowledgement.

### 4. Register one factory

Implement `DriverFactory` with the exact id and roles. `bind` validates
settings and may reserve passive listeners as described above; it must not
admit an external fact, acknowledge a request or launch an outbound effect.
`RuntimeDrivers::try_new` calls each
required Contract factory once before execution and rejects missing, incomplete
or lying implementations.

Static builds explicitly construct the desired `DriverRegistry`. Minimal,
embedded and full standalone binaries may therefore link different Connector implementations
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
message replay. Fragmentation and control frames are Contract integration tests,
not application outputs. An event profile proves sequence/admission and has no
fabricated completion or per-event exit.

Add Contract-focused integration tests for physical behavior the generic battery
cannot force. A new Connector and its Contract are not complete merely because their happy path runs.

### 6. Prove the Lab twin

Register a mock factory under the same id and roles. Reuse the Contract's pure
request preparation and scheduler-completion projection. Script only the
external outcome after the journal has opened the call. Prove malformed mock
outputs fail the ordinary resolved-port gate and that adding Lab source changes
neither semantic hash nor validated plan.

Put the pure scripted-protocol binder beside the real contract-specific module. The Lab
registry composes these Contract-owned binders; it must not grow a parallel
string match that re-implements the Connector catalogue.

### 7. Prove composition

Bind the Contract alone, in a reduced build that intentionally omits it, and
beside at least one unrelated Contract. The combined graph must need no concrete
Contract match in the VM. Replaying an already closed call must never repeat the
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
ordinary `DriverFactory` to the same `DriverRegistry`. Discovery is a
physical assembly choice, never an alternate VM, plan or simulation model.

## Review checklist

- No external observation occurs before the corresponding journal boundary.
- No Driver writes journal records or assigns envelope identities.
- No semantic default or type decision is duplicated in Driver code.
- No concrete Connector id is matched in the VM.
- Binding is complete before execution and registration order never selects.
- Real and mocked worlds share the same plan, request protocol and result gate.
- Every terminal case is proven or explicitly untestable in conformance.
- Any semantic/package/causal identity move is declared before goldens change.
