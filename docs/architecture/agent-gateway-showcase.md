# Agent gateway — composition evidence and limits

14 September 2026. This slice finishes the resident WebSocket bridge and
publication documentation, not the paid-model, authentication or accounting
product. It changes no Contract, package version, journal key or language rule.

## Readable browser continuation — 17 September

`examples/agent-chat.rkg` continues the simulated app-server bridge with
Rastok Codex Gateway. `examples/agent-chat.html` is a single, unminified
HTML file with inline CSS and vanilla JavaScript, commented in English. No
frontend dependency, generated bundle, API key or model setting is embedded.

An HttpServer in `respond` mode returns the compiled `asset(...)` with its
HTML content type. A separate shared WebSocketServer carries
`{ nickname, kind, rpc }` messages. Only `conversation` is keyed, by nickname;
it owns its resident Process and socket-return state. A second socket claiming
an occupied nickname is refused by explicit instance state. This is local
ownership logic, not authentication or a public security perimeter.

The page sequences initialize/initialized/thread/start/turn/start, appends
text deltas and waits for `turn/completed` to accept another turn. Unexpected
server requests receive an explicit JSON-RPC error; tools and approval prompts
have no UI here. The protocol was rechecked against the official app-server
documentation on 17 September. No actual Codex invocation was used for tests.

`kind = "quit"` routes to the current conversation's `instance.stop`, so its
children close but both listeners and Bob's conversation remain available.
The reply acknowledges a closure request, not an invented successful child
exit. Cancelling a connection before admission sends no initialization;
rejecting an occupied nickname sends no Quit to its existing owner. A lost
socket is not converted into a closure command. The idle lifetime bounds
abandoned conversations to five minutes; reconnect/resume and persistence
remain outside this example. Codex may retain its own local thread history;
the page's Quit requests process closure, not deletion of those files.

Qualification:

- Three Lab scenarios pass: Alice/Bob, duplicate nickname and invalid input.
  No Driver implementation, network listener or child is used by the Lab;
  the explicit `peer` input is its decoded-message seam.
- Four Rust tests pass: the exact HTML survives plan encode/decode, scripted
  runs are deterministic with exactly two reused children, stepping matches
  batch, and real loopback HTTP/WebSocket traffic coexists with two harmless
  shell JSONL processes. The latter observes Alice's child terminal, Bob's
  unchanged PID and a still-working HTTP asset response after Quit.
- Ten dependency-free Node tests pass against the inline script: page identity, handshake,
  streaming/final-item handling, literal text, tab isolation, cancellation,
  duplicate-name recovery, explicit unsupported-request refusal and errors.
  They are wired into CI; a remote CI run is not claimed.
- The adjacent gateway and site-contract suites also pass: 13 targeted Rust
  tests in total. The client, layout and editor lexical suites total 28 passing
  JavaScript tests. Workspace formatting, strict all-target/all-feature Clippy
  and rustdoc pass. The full workspace test suite was not rerun for this
  example/documentation-only pass; the unblessed v19 inventory stays open.
- Yoann subsequently opened three conversations in the real application and
  reported correct operation. This is a user-reported live smoke check, not
  an automated model test. The full browser visual qualification remains
  **open**: the agent's browser runtime reported no available browser, and
  DOM/socket doubles do not replace that audit.

```sh
rastok simulate examples/agent-chat.rkg
cargo test -p rastok --all-features --test agent_chat
node --test examples/agent-chat.test.cjs
# Optional real local demo: requires an installed/configured Codex.
cargo run -p rastok --features hosted-websocket -- run examples/agent-chat.rkg
# Then open http://127.0.0.1:8777/ in two tabs.
```

No runtime, journal, plan format or execution command changed for this
composition; no golden was blessed. The asset belongs to this new program's
identity, not the existing gateway's golden. Future task-per-instance,
multicore and independent-flow placement choices are documented in
[parallelism-notes.md](parallelism-notes.md), explicitly post-M1. One Worker
already permits this I/O concurrency; no parallel Operator execution is claimed.

## Two independently runnable layers

- `examples/agent-dialogue.rkg` teaches a typed conversation adapter with an
  imagined model and a generic resident Process. Its real command is `cat`;
  it does not speak Codex's protocol.
- `examples/agent-gateway.rkg` composes the actual WebSocketServer and resident
  Process contracts. The `conversation` Graph is keyed by the incoming socket's
  `meta.ext.session_id`. Each conversation owns its `model` and `route_reply`
  instances. The same JSON object may use the same request id in two sockets
  without sharing the child or the return address.

The bridge forwards JSON unchanged. A client negotiates
`initialize`, `initialized`, `thread/start`, then `turn/start` and consumes
replies plus notifications. Client replies to server requests have `id` and
`result` rather than `method`; the bridge does not reject that half of the
protocol. The pinned source of this description is the
[official Codex app-server documentation](https://learn.chatgpt.com/docs/app-server),
consulted on 14 September. The subprocess command is
`codex app-server --listen stdio://`; this is JSONL, not `codex exec` per turn.
No model invocation was performed to qualify this slice.

The Lab uses App `peer` and `close` inputs as explicit test seams. It replaces
the Process with a script and checks forwarding, multiple frames, exact
closure and Bob's continued use after Alice closes. It does **not** pretend
to script the WebSocket upgrade or unsolicited peer events. A separate marked
loopback test opens two real WebSocket connections to the same graph, substitutes
a harmless shell JSONL child, checks distinct/reused PIDs and unchanged JSON,
including a client reply without `method`.

## Ownership, shutdown and the journal

A child frame is a new Session ingress root, not a per-job Call response.
Its `session_id` identifies the Process resource, not the WebSocket. The
`route_reply` Operator retains the original socket coordinate in private
per-conversation state and restores it only on the outward envelope. No
global last-request slot, topology guess or mutable correlation registry is
introduced. Replies still retain the Process's causal root in the journal.

`conversation.finish.out -> model.instance.stop` requests the exact child
instance's Closing. The authored mock provides no system-shutdown observation,
so the deadline truthfully produces `OutcomeUnknown`. Socket closure separately
appears on `session_exit`; it is not proof that an agent job was cancelled.
The five-minute idle lifetime bounds the conversation; it is not durable memory,
a rate limit or a guarantee that the last turn finished. A trusted caller must
not send new work to a conversation it has explicitly stopped.

## Composition corrections

1. Session message routing preserves the installed source instance's ancestor
   scope. Re-evaluating the conversation key from child-frame metadata created
   a different conversation and lost the retained socket coordinate.
   `tests/agent_gateway.rs` checks that replying and remembering use exactly
   the same two instances, and that repeated Lab runs retain identical records.
2. A Process reader may publish `Opened` and its first frame in the same poll.
   The Driver must hand `Opened` to the core first. Otherwise the first frame
   is correctly rejected as belonging to an unopened Session. A deterministic
   channel test supplies both events together, without an OS timing assumption.
   The scripted Driver follows the same rule: stepping must not expose a
   mock's immediate response before its opening is retained. Interactive and
   batch scenarios are compared, not merely assumed equivalent.
3. The interactive Lab must drain ready transactions before offering another
   scripted frame, and retain a preceding write acknowledgement first. A
   just-progressed world is not yet quiescent merely because no Operator is
   ready. Otherwise `expect` can run too early, or stepping changes the journal's
   interleaving. The regression compares the entire canonical batch/step trace,
   including multi-frame turns, rather than comparing final counts alone.

These are existing ownership/order invariants, not new semantics. The first
changes formerly incorrect routes for nested keyed Session frames; the second
removes a physical scheduling race; the third restores the batch runner's
established cadence in stepping. The non-Session causal witnesses remain the
regression gate. No golden is blessed merely to cover a failure.

## Explicit limits

- This is a loopback, trusted-client demonstration, not a public authenticated
  service. Clients are responsible for the app-server handshake, IDs, approvals
  and read-only tool policy. The example's repeat-safe effect declaration must
  not be reused to authorize arbitrary non-repeatable agent actions. There is
  no automatic retry or restoration of a process's private memory.
- Process has no `secret()` binding. Use an already configured local Codex
  account; do not send credentials through the gateway. Unannotated protocol
  payloads are retained in the in-memory journal. This bridge does **not**
  claim that arbitrary protocol contents are secret-free or GDPR-certified.
- Scripted deltas and final events demonstrate transport, not model quality,
  complete protocol conformance, approvals, or live model usage accounting.
- Session writes are not agent jobs. Request IDs/turn IDs are application
  correlation; Session ingress roots must not be relabelled as their request's
  `origin_id`. No per-job or per-session cost is invented from byte counts.
- The one-model HTTP usage/credential adapter remains a separate open item.
  `secret()` is supported on `HttpClient.auth_bearer`. Yoann subsequently
  authorized minimal `has(object.member)` presence inspection, implemented
  under D-2026-09-14-3. It does not turn `as struct` into an open-map projection.
  The [candidate adapter](../examples/agent-http-candidate.rkg) distinguishes
  missing/null/invalid usage from reported zero. It deliberately remains outside
  the executable gallery: `effect = irreversible` is still refused in M1.
  D-2026-09-14-4 resolves that design question through the
  [Loop / Retry / Saga refactor](retry-extension-refactor.md): an ordinary
  Call needs no Saga classification or fabricated idempotence guarantee.
  Implementing that cutover and qualifying the adapter remain open; neither
  paid invocation nor adapter execution is claimed. The proposed wire shape
  follows the
  [official Responses request/response schema](https://developers.openai.com/api/reference/cli/resources/responses/methods/create),
  consulted on 14 September. Companion prices are fictional demonstration rates,
  never OpenAI tariffs.

## Reproduce

```sh
cargo test -p rastok --all-features --test agent_gateway
cargo test -p rastok --all-features --lib an_immediate_frame_cannot_overtake
rastok simulate examples/agent-gateway.rkg --ui
```

The first command includes a bounded local-network/Unix-child integration;
it never starts the `codex` command in the example. The last command runs
only the Lab and never spawns a child. Real `run` requires an installed Codex,
explicitly trusted clients and a separately reviewed deployment; it is not
part of the default gallery command.
