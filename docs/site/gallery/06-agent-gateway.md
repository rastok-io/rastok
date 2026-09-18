---
title: Agent gateway — keep a conversation isolated
description: Connect WebSocket conversations to resident JSONL processes, simulate interleaved app-server requests, and close one child without interrupting the other.
teaches: WebSocket and Process Sessions, exact instance ownership, transparent JSON-RPC, match mocks and child closure
graph: examples/agent-gateway.rkg
golden: crates/rastok/tests/golden/gallery/agent-gateway/app_server.two_conversations.trace
command: rastok simulate examples/agent-gateway.rkg --ui
expected: Two scenarios pass; Alice and Bob use separate children, streamed replies keep their socket address, and Bob continues after Alice closes.
break_it: 'Change the scripted delta "Hello from the simulated agent." to "A different answer." and rerun with the gallery golden directory without --bless.'
limits:
  - The Lab uses explicit peer inputs, not a simulated WebSocket handshake; real socket composition is a separate local integration test.
  - Replies are scripted app-server examples, not a paid model invocation or complete protocol qualification.
  - This trusted local bridge does not authenticate clients, redact arbitrary protocol payloads or price agent jobs.
  - The separate HTTP adapter uses an explicit projected Responses schema; local fixtures qualify it, not a live paid OpenAI call or arbitrary future API changes.
  - An unacknowledged system close becomes OutcomeUnknown, not a fabricated successful exit.
replaces: 'NiFi: keyed session state plus a custom resident-process processor; MuleSoft: a stateful process adapter behind an integration flow.'
order: 6
---

The sensor chapter kept state while time passed. A conversation adds state
outside the graph: the memory of a child process. Alice and Bob should each
keep their own child across several turns. Ending Alice's child must not
interrupt Bob, and sending a message must not be mistaken for receiving an
answer.

## Run

```sh
rastok simulate examples/agent-gateway.rkg --ui
```

The debugger starts paused. Follow one input, then inspect the instance and
Session tables. For the reproducible CLI check:

```sh
rastok simulate examples/agent-gateway.rkg --golden crates/rastok/tests/golden/gallery/agent-gateway
```

## Read the graph

Start at `clients.out`: each WebSocket message enters `conversation`, where
`instance = { key = meta.ext.session_id }` scopes its children. The first
message opens `Process(mode = service)`; later messages write to the same
child. `route_reply` remembers that socket address and restores it on child
responses. It does not share a global last-request variable between clients.

The real command is `codex app-server --listen stdio://`. The client, not
Process, speaks its JSON-RPC protocol: initialize, acknowledge initialization,
start a thread, send a turn. Requests, replies and notifications pass through
as JSON, including client replies without a `method` field. The Lab's `match`
replaces the child; it never launches Codex. Its `peer` input is an explicit
fixture seam, not an invented WebSocket upgrade.

Follow `finish.out -> model.instance.stop` last. An authored `close` input
requests that exact child generation's Closing, not every Process or the
whole conversation Graph. Socket closure has its own `session_exit` output.

## Expected result

Alice and Bob both initialize and start a thread. Both use request ids 0, 1
and 2: isolation cannot rely on globally unique RPC ids. Each scripted turn
returns an initial response, a text delta and a final notification. Two
resources serve the conversation, not one process per message. Alice's stop
settles as `OutcomeUnknown` because the mock supplies no system-shutdown
observation. Bob's next turn still uses his original child.

The second scenario returns a scripted unknown-method error. It is JSON data,
not a Process `exit`. The separate resident primer,
[agent-dialogue.rkg](../../../examples/agent-dialogue.rkg), shows an
application `quit`, observed resource exit and a fresh resource on new input.

## Add the small browser client

The next step is [Rastok Codex Gateway](../../../examples/agent-chat.rkg), with a
single [readable HTML file](../../../examples/agent-chat.html). It uses plain
HTML, CSS and JavaScript, with English comments: no framework, build step,
CDN or generated bundle. Start with its scripted protocol:

```sh
rastok simulate examples/agent-chat.rkg
```

Three scenarios cover interleaved conversations, a duplicate nickname and
invalid messages. The same app-server handshake is now wrapped in
`{ nickname, kind, rpc }`. `conversation` is keyed by the nickname and remembers
its socket; a second socket cannot take that name or close its child. The Lab
still enters through `peer`, without binding either network listener.

For the real local application, install and configure Codex first. This
command **does launch real processes** when a client connects, and sending a
message may use your configured model/account:

```sh
cargo run -p rastok --features hosted-websocket -- run examples/agent-chat.rkg
```

Open `http://127.0.0.1:8777/`, connect as Alice, and open another tab for Bob.
Send a message in each, then choose **Quit** for Alice and continue with Bob.
The page waits for `turn/completed`, not just the initial `turn/start` reply,
before enabling the next message. Model text is rendered literally, not as HTML.
An unsupported server-initiated request is explicitly declined.

The two flows share an App, not a transaction. `page` serves the embedded
`asset(...)` through HttpServer; `clients` carries chat over WebSocket at
`127.0.0.1:8778/chat`. Neither listener is inside the keyed conversation.
The current cooperative Worker can keep serving the page and other children
while external I/O waits; this does not claim parallel DSL execution.

Quit is an application message routed to `instance.stop` **inside that
conversation**. It closes the conversation and its children, not the App or
Bob. It is never a magic line sent to Codex's stdin. The acknowledgement means
“closure requested”, not “the child exited successfully”. Closing a browser
tab without Quit is different: the abandoned conversation has a five-minute
idle lifetime. This page has no reconnect/resume feature or saved history.
Codex may keep its own local thread history; Quit is not a deletion request.

The page selects the app-server read-only sandbox and no-approval policy;
this is a trusted local example, not a security boundary. Do not expose the
listeners publicly or send credentials through the chat. Editing the HTML
changes its embedded asset: restart `run` to load the new file. The browser
does not edit the graph, the Lab or retained data.

## Inspect a typed model response

The smaller [HTTP variant](../../../examples/agent-http.rkg) makes one
non-streaming OpenAI Responses request per job. Unlike the resident bridge,
it does not keep a conversation inside a child process. The input carries
`session`, `job` and `prompt`; the first two remain in the response's META.
Try its six scenarios without a real credential or model call:

```sh
RASTOK_SHOWCASE_MODEL_TOKEN=lab-only-not-a-credential RASTOK_SHOWCASE_MODEL_TOKEN_VERSION=fixture-v1 rastok simulate examples/agent-http.rkg --deployment examples/agent-http.deployment.json --ui
```

Read `ModelResponse`, then `model`: `output out: ModelResponse` and
`json_profile = projected` hydrate the declared fields before an emission is
retained. Additional JSON members are omitted; absent nullable fields become
null; a wrong type still gives `DecodeFailed`. This is an explicit selected
view of Responses, not a new rule that makes every struct open.

Inspect `model.out`. Its original `input_tokens`, `output_tokens` and
`total_tokens` carry `@measure` annotations; no downstream Operator extracts
or counts them again. Three scripted jobs retain their own measurements and
session association. An absent `usage` produces no sample; an observed zero
remains zero. The three quantities are separate evidence, **not three numbers
to add together**. No token price or currency amount is invented.

The other scenarios cover incompatible schema, a timeout, and HTTP failures.
A decoded 500 is `ServerError`; an error body incompatible with the selected
schema is `DecodeFailed` instead. There is no automatic Retry or invented
idempotence guarantee for an inference that could be charged twice.

The Lab supplies typed outcomes; separate tests exercise complete wire JSON
and a real local HTTP server. No live OpenAI invocation was performed for this
qualification. To compare all six scenarios, use the same command without
`--ui` and add `--golden crates/rastok/tests/golden/gallery/agent-http`.
Change `Hello from the simulated model.` to `A different model answer.`
in that example's mock: its count assertions still pass, but the golden
names the changed record. Restore the reply afterward; do not use `--bless`.

## Break it

Change only `Hello from the simulated agent.` to `A different answer.` in
the mock. Leave the golden unchanged and run the comparison command again.
The graph still compiles, and the shape/count assertions still pass. The
changed answer nevertheless differs in the retained evidence.

## Read the golden difference

The diff names the first changed record, expected and actual data, and the
retained cause. Here the changed evidence is an inbound service frame, not
an invented per-turn Call completion. Do not use `--bless` to hide the change.
Restore the answer before continuing.

## Carry forward

Previous: [Sensor alarm](05-sensor-alarm.md). You now have the pieces to
isolate an agent conversation: typed Contracts, instance-owned state, a
resident resource, explicit shutdown, and a controlled external world. Try
the standalone browser client above, or the smaller resident primer to add
your own typed adapter. Model-specific
usage evidence belongs at that adapter boundary, not in a protocol hidden
inside Process. The HTTP variant above shows the typed boundary; it makes
no claim of a billing report or measured usage from arbitrary app-server JSON.

## Limits

This is a trusted **local** bridge, not a public authenticated AI service.
A real client owns handshake sequencing, request ids, approvals and the
read-only tool policy. The teaching page requests read-only execution; that
is not an authentication boundary or permission to expose it publicly.
`behavior = probabilistic` describes
the external boundary; the Lab remains deterministic. Tests compose real
sockets with harmless shell children, not a model or a credential.

Process has no `secret()` binding. An installed Codex can use its existing
local authentication; do not transmit credentials through this gateway.
Unannotated protocol payloads remain visible in the in-memory journal.
Neither secret-free arbitrary payloads nor legal compliance are claimed.
No token count or cost is invented from missing usage, message counts or
Session writes. The HTTP adapter targets OpenAI Responses, not a surrogate
model API. Its [projected profile](../../architecture/openai-responses-hydration.md)
is qualified locally, including original samples from a held Retry attempt
on an inert test endpoint. This does not authorize retrying model inference.
`secret()` is bound only at the HttpClient Driver boundary; its bearer is not
retained or displayed. That boot-dependent profile still refuses diagnostic
recording, rather than persisting deployment secrets or inventing replay inputs.

Disconnecting a socket is not the same as cancelling an agent turn. The idle
lifetime bounds abandoned conversations but does not restore child memory
after restart. Details and the exact test commands live in the
[composition evidence](../../architecture/agent-gateway-showcase.md).

The service mock responds to inputs with frames and an optional terminal.
Unsolicited/delayed scripted frames, system-close acknowledgements and `now()`
in Session mocks remain named limits. The deployment journal is not durable.

## NiFi / MuleSoft equivalent

A keyed stateful flow around a resident-process adapter. The distinctive
combination here is the typed DSL, independently checked contract, replaceable
effect boundary and deterministic Lab scenarios—not a claim that other tools
cannot host stateful processes.
