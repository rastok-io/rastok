# Agent gateway — composition evidence and limits

14 September 2026. This slice finishes the resident WebSocket bridge and
publication documentation, not the paid-model, authentication or accounting
product. It changes no Contract, package version, journal key or language rule.

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
  `secret()` is supported on `HttpClient.auth_bearer`, but safe optional JSON
  member inspection (such as an absent `usage`) is not in the current CEL
  subset. `as struct` does not turn an open JSON map into a projection. Extending
  that subset requires Yoann's arbitration; a typed mock alone would hide this
  gap. No new `has(...)` or permissive cast is silently introduced here.

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
