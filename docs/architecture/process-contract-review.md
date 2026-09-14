# Process — complete contract review

> Historical design inventory, 10 September. The subsequently implemented
> [resident Session contract](process-service.md) and D-2026-09-14-1 supersede
> this document's service-admission, cap and identity gates. Its remaining
> unsolicited Lab-world and gateway-adapter work is still named explicitly.

10 September 2026. **Protocol, fresh-opening and shutdown responsibilities accepted; remaining resource policies under review.**
Yoann has asked for one resident Codex process per conversation, with Alice
and Bob making several interleaved turns, and then requested that Process be
defined completely before implementing a reduced service profile. This
document covers both profiles and names the decisions required to open the
service gate. It does not change the current `rastok/process@2` validator,
job behavior, or any golden.

The normative authority remains [stdlib §3.3](../spec/stdlib.md#33-process--child-processes-two-modes),
[session delivery](../spec/delivery-semantics.md#45-sessions--one-resource-lifecycle-many-message-effects)
and [session supervision](../spec/execution-model.md#66-session-supervision-executable-through-m16-h6).
Recommendations below are proposals wherever those texts do not already
settle the question. An unresolved row is a review gate, never an implicit
implementation default.

**Subsequent lifecycle agreement (10 September):** D-2026-09-10-1 accepts
`instance.stop` for the current Graph and `child.instance.stop` for a direct
child, as commands rather than ports. This settles the direction of the
explicit closure request, not the service profile. The
[v18 implementation ledger](instance-stop-command.md) qualifies common
instance closure separately from the unresolved resource policies below.

**Subsequent protocol agreement (10 September):** D-2026-09-10-2 accepts a
generic JSONL Process with protocol interpretation in Operators. Section 5 is
now settled: the old `correlate`/Process-owned RPC promise is superseded.
Service framing bounds, resource selection, physical termination and
Lab world actions remain review gates; the profile still refuses at check.

**Subsequent reopening agreement (10 September):** D-2026-09-10-3 accepts no
automatic respawn. After child death, a new input may open a fresh child in the
still-open instance, with a new SessionRef, no resend of old writes and no
claim to restore child memory. This does not revive a stopped Node generation.

**Subsequent shutdown agreement (10 September):** D-2026-09-10-4 keeps an
application message such as `quit` on the ordinary data path. It does not
implicitly stop the Node. The Unix system fallback distinguishes `SIGTERM`
from forced `SIGKILL`; EOF is not a universal first shutdown action. The two
existing Closing budgets are accepted: cooperative shutdown at the drain
deadline, forced cleanup at the settlement deadline. No resident Driver is
claimed by that agreement.

## 1. Purpose and the three scopes

Process connects a graph to a child program over stdin, stdout and stderr.
It owns framing and resource lifecycle, not the program's business meaning.
Operators and Connectors still share the ordinary transaction, mailbox and
causal machinery. No Connector acquires DSL handlers.

Three scopes must remain distinct:

| Scope | Meaning | Identity |
|---|---|---|
| Application conversation | Alice's or Bob's dialogue; it may outlive a socket | Authored business key on a Graph or Node instance |
| Resident process | One physical child incarnation, modeled as one Session | Journal-derived `SessionRef`, owned by an exact `InstanceRef` and generation; never the OS PID |
| Application request or turn | One logical question, response or protocol operation inside that process | Protocol request id, distinct from the process SessionRef and from the message's EnvelopeId |

Several turns in Alice's conversation reuse her live process. Bob's turns
use another process. A websocket SessionRef is not automatically the process
SessionRef; forwarding a message must not confuse the two resources. A key is
also not an authorization check, a filesystem sandbox, a deduplication policy
or proof that a model remembered a previous turn.

## 2. Profile matrix

| Property | `mode = job` — executable, preserved | `mode = service` — target |
|---|---|---|
| Exchange | Outbound finite Call | Resident Session with both Driver roles |
| Child creation | One admitted input creates one child | First admitted input lazily opens one child for the instance |
| Further inputs | Further jobs, serialized within an instance | Further writes to that same live child |
| stdin | One framed request followed by EOF | Kept open across messages |
| stdout / stderr | Bounded capture, published on completion | Complete bounded frames admitted while the child lives |
| Terminal | One `exit` per job | One `exit` per child incarnation, not per message |
| State | No child state shared between jobs | Child may retain state across messages; that physical state is not itself journaled |
| Repetition | Explicit graph decision, possibly Loop | No automatic respawn; a new input may open a fresh child in the still-open instance, without retrying an earlier request |
| Loop host | Yes, subject to the resolved effect contract | No; service is Session-shaped |

The current Process job permits many `out` and `err` frames in its atomic
completion. A nonzero exit code is still `Exited`; the graph judges it.
No new streaming behavior is introduced into existing jobs.

## 3. Ports and framing

The ordinary interface remains `in`, `out`, `err`, `exit`:

- `in` carries an explicit graph-to-child message;
- `out` carries decoded stdout data, including protocol responses and notifications;
- `err` carries stderr data, never an implicit exception or process-death signal;
- `exit` describes the end or failed establishment of the child resource.

Every productible output needs a disposition. An unused stderr stream is
`err -> nothing`, not silent loss inside a Driver. Physical I/O errors are
not promoted to fabricated business responses.

### Existing job wire, unchanged

`lines` means UTF-8 lines, one text value per frame. `jsonl` means canonical
JSON requests and strict decoded JSON values on output; JSON numbers use the
existing exact representation. `bytes` means one bounded complete captured
channel, not a sequence of OS read chunks. stderr has its own `err_codec`.
The existing newline, empty-line, final-unterminated-line and decode-refusal
rules remain exactly those in the normative sheet.

### Service wire — proposal to seal

Reuse the line and JSON-line codecs, with frames visible before process exit.
A partial read is not a frame. Under pressure, the Driver retains a bounded
proposal or stops reading; it neither drops bytes nor invents a frame boundary.
At EOF, complete frames precede the terminal under a defined intake order;
the existing final-line rules apply. A malformed frame terminates the resource
with a typed failure; frames admitted before it cannot be rolled back.

Raw `bytes` on a resident stream needs a framing rule: the job's "whole
channel at EOF" rule cannot support conversational exchanges. **Recommend
named refusal for service `bytes` until an explicit bounded framing contract
exists**, for stdout and for independently configured stderr. Do not make OS
read chunk size observable as a DATA boundary.

stdout and stderr have per-stream order, not a portable cross-stream order.
The journal retains the admitted interleaving; replay uses that retained order.
Simulation equality requires the same supplied world events and logical time,
not a promise that two live process executions will interleave identically.

## 4. Lifecycle and delivery

Target state machine:

```text
Absent -> Opening -> Open -> Closing -> Terminal
              |          \------------> Terminal
              \-----------------------> Terminal
```

1. Resolve and validate settings and first-message framing before any effect.
2. Reserve the admission and commit the opening intent before spawning.
3. Driver acceptance establishes the child Session once. The first request
   must be written exactly once: it must not be duplicated as both opening
   data and an ordinary follow-up write.
4. Later messages reserve bounded capacity and commit write intents before
   bytes can reach stdin. A write settlement proves local acceptance, not
   completion of a model turn or successful business work.
5. stdout/stderr frames enter through the ordinary retained inbound proposal,
   admission and journal path. Pending proposals keep their identity.
6. Closing rejects new admission, drains within the authored policy, requests
   termination, settles or reports uncertainty, and emits one resource exit.
   No message or second terminal can be admitted after that terminal.

A preparation refusal must not invent an established Session or a spawned
child. Its typed refusal and causal handling must remain distinct from an
opened resource's terminal, including in the debugger's opening count.

The resource lives under the Node instance, not under the latest message's
root. A business turn ending does not implicitly close the process. Physical
handles remain Driver-owned and can never be recovered from an OS PID in a
recording.

### Closing — application request and system fallback

An application shutdown request is data. A graph may send `quit`, or a JSON
message defined by its protocol, to the ordinary input. Process writes it
without interpreting it. The child may reply, terminate, or ignore it; local
write settlement is evidence of none of these application outcomes. Only the
observed resource exit ends the Session. A normal resource exit leaves the
Node instance open unless graph/lifetime policy separately closes it.

Instance closure is the outer bounded guarantee. It closes admission to that
generation, drains already admitted work, requests system shutdown and settles
an observed terminal or uncertainty. The Unix service fallback uses `SIGTERM`
before forced `SIGKILL`, not a magic application message. EOF means only that
stdin is closed. It must not truncate an already admitted write or be described
as proof of shutdown. A protocol acknowledgement is likewise not an OS exit.

**Accepted timing, D-2026-09-10-4:** preserve the two existing budgets.
During `drain_for`, admitted messages (including an authored `quit`) may finish.
At the drain deadline, request `SIGTERM` for a surviving resource. During
`settle_for`, retain its observed exit normally. At the settlement deadline,
request forced cleanup for any survivor; if no terminal has been observed,
retain `OutcomeUnknown`, never a fabricated `Exited`. No third grace period
is added, no late OS exit rewrites the winner, and cleanup must follow its
retained authority rather than run inside a pre-commit projection. A zero
exit observed within the budget is different from a kill requested at its end.

The common VM now offers `close_session` at the drain deadline and
`abort_session` at the retained settlement deadline for a still-unsettled
Session. The latter is an idempotent, nonblocking handoff authorized by the
committed Closing request; it must not wait for terminal-route capacity or
storage. `release_session` follows the terminal commit and then releases
registry correlation. Neither callback manufactures an observed exit.
The [qualification ledger](instance-stop-command.md#9-two-budget-session-shutdown--10-september)
separates this common-boundary implementation from physical resource evidence.
There is still no resident Process Driver. Job `cancel` invokes forced
`kill_group` (Unix group, then direct-child kill/wait) and is unchanged.
The service gate must qualify nonblocking signal handoff, exact resource
fencing, bounded cleanup and recovery. A cross-platform child-only kill must
not be described as process-tree termination.

For the requested gallery, recommend explicitly coupling the conversation
owner's lifetime to its process. **Socket disconnect alone is not that policy.**
Decide whether it closes the conversation or permits reconnection; never hide
that choice in the Process Driver. Test Alice closing while Bob continues.

The accepted generic command gives the graph two explicit choices:
`decide.end_conversation -> instance.stop` closes the selected conversation
and its descendants; `decide.end_process -> codex.instance.stop` closes only
that direct child's instance and its resource. The second does not close the
parent conversation, reset sibling state or stop Bob's child. Neither command
restarts the process. Both retain an exact generation and invoke ordinary
bounded Closing; no Process-specific stop port or Connector handler is added.
Both spellings are now executable under v18 and qualified independently of
Process service. The remaining lifecycle gate concerns the resident resource,
not the ability to request instance closure.

## 5. Protocol ownership — accepted, D-2026-09-10-2

**Chosen: duplex Process, protocol adaptation in Operators.** Process owns
the child resource, framing, bounded writes/intake and one resource `exit`.
With `codec = jsonl`, it parses each complete stdout JSON line into the
existing `variant`. An Operator validates/narrows that value into application
messages and interprets request ids, responses, notifications, progress and
turn completion. A JSON syntax error is a Process framing failure; valid JSON
that violates the application schema is the adapter's concern.

There is no Process-owned per-request Call, RPC table or business timeout.
Local write settlement is not a reply. `Process.exit` ends a child incarnation,
not a turn; several turns reuse a live resource. An application's reply is
still ordinary stdout data at the Process boundary, not a special transport
event selected by inspecting its request id.

**Superseded:** stdlib §3.3/§9 previously promised both a pure stream and,
with `correlate`, request/reply Calls inside the resident process. The optional
path alone could not say:

- whether a message is a request, reply, notification or peer request;
- which response completes a multi-frame operation;
- whether a timeout terminates just that request or the process;
- whether an unmatched id is a late reply, an unknown reply or a new notification.

In particular, a reply to an expired request must not reappear as a spontaneous
event simply because its id no longer names an open request.

The decision rejects moving that application protocol into Process. Other
Contracts' correlators are unchanged. The deferred Process sheet still carries
its old attribute inventory; it cannot make that rejected RPC design
executable, and service admission must remove it rather than reinterpret it.

This boundary is not permission to leave correlation unspecified in the
gateway. The gateway still needs a tested Operator adaptation and must route
replies to the initiating conversation. Any pending application-request state,
timeout races and late-reply policy must be explicit and reconstructible, not
hidden in the physical Driver or only in a mock.

## 6. Attributes and bounds — complete review inventory

| Setting or policy | Current job rule | Service definition required |
|---|---|---|
| `mode` | Omission is exactly `job` | Explicit `service`; no fallback to jobs |
| `command`, `args`, `cwd`, `env` | Existing typed settings, resolved by the declared phase | Resolve once for a child incarnation; later messages cannot silently mutate its environment or directory |
| `codec`, `err_codec` | Existing choices and defaults | Closed admitted framing choices per §3 |
| `effect`, `behavior` | Authored, mandatory | Still authored; residency does not prove idempotence or determinism |
| `max_output` | Aggregate stdout + stderr capture, default 8 MiB | Not an unlimited lifetime counter; define finite frame and retained-queue byte caps separately |
| `timeout` | Optional whole-job budget; `on_timeout = kill` | No implicit per-request timer in Process. Seal service applicability/refusal; never reinterpret the job setting silently as idle lifetime or a different clock |
| Instance `lifetime` | Standard owner lifetime | Standard owner lifetime, not a request deadline |
| `closing` | Standard bounded drain/settle when required | Protocol request remains data; `SIGTERM` at the drain deadline and `SIGKILL` at the settlement deadline accepted in §4; physical qualification remains open |
| `correlate` | Not a job attribute | Retired by D-2026-09-10-2; application correlation belongs to the adapter |
| `restart` | Not a job attribute | Not in the service baseline (D-2026-09-10-3); a future automatic policy requires a separate decision |
| Capacity | Bounded VM/Driver admission | Also bound incomplete frames, pending writes, intake bytes and active processes; application request state belongs to the adapter |

Exact new spellings, defaults, units and the scope of every byte cap must be
written in the Contract sheet before admission is enabled. Saturation either
keeps admission pending under the same bounded reservation or produces a typed
failure; it is never unbounded allocation or silent truncation. Closing and
timer bookkeeping need enough reserved capacity to complete under pressure.

**Accepted fresh-open policy (D-2026-09-10-3):** child death emits the ordinary
typed `exit` but causes neither automatic respawn nor a Node stop. A new
eligible input may open a fresh child if that exact instance remains open
and no live child remains. The fresh opening gets a new SessionRef in the
same instance generation; subsequent inputs reuse it. Nothing implicitly
resends an earlier stdin write or restores the child's private memory.
Retained writes and late events for the dead Session are never retargeted.

Closing wins: an explicit stop prevents reopening in the target generation;
any successor generation follows the existing instance rules. The generic
client Session path already distinguishes a new open from a write to an exact
resource reference, but Process still needs qualified resource selection and
profile admission. This agreement does not turn the current job into a service.
An automatic `restart` policy is outside this baseline and would need its own
decision, not unchecked contents of the previously proposed struct.

## 7. Outcomes and time

The seven existing `ProcessExit` cases are retained as the job baseline:
`Exited`, `SpawnFailed`, `TimeoutExpired`, `DecodeFailed`, `CaptureFailed`,
`OutcomeUnknown`, `OutputOverflow`. Their payloads and old causal bytes do not
move to make a service implementation convenient.

For service, the review must pin which **resource** failures use these cases
and the bound referenced by `OutputOverflow`. A per-request error is not a
resource exit while the process remains alive. A stdout business error is
ordinary data; it does not automatically become stderr or an engine fault.

Every semantic deadline uses explicit logical time and retains its original
due instant. A physical wait does not choose a second semantic timeout.
Resource completion versus a resource deadline has one journaled winner.
Application request completion, request deadlines and late replies are the
adapter's separate policy, not Process supervision. Missing external evidence
means unknown, not successful cancellation.

## 8. Recovery, security and observation

- Diagnostic replay restores retained facts without spawning a child or
  replaying a stdin write. It is not durable deployment recovery.
- A dead runtime cannot assume a child, conversation or application memory
  survived. Unsettled externally visible writes are not blindly resent.
- Resume and start-fresh are distinct protocol decisions. Any resume token
  and relevant version must have an explicit authority and privacy policy;
  neither a mock map nor the current RAM process table is that authority.
- A process key isolates logical state, not the host filesystem, network or
  credentials. Real execution requires an explicit deployment security boundary.
- The existing `secret()` door is `HttpClient.auth_bearer`; arbitrary Process
  environment values are not a secret-safe reference channel. Do not add clear
  credentials to a gateway example or claim Process secret support before its
  own binding/redaction contract is decided and tested.
- Usage/cost belongs to qualified protocol data or explicit unknown evidence,
  attributed to a turn and conversation through retained causal references.
  Process presence or duration is not a token count, invoice or invented zero.
- UI observations show the selected journal prefix: open process incarnations,
  requests where modeled, and each terminal separately. No UI counter proves
  the number of process creations.

## 9. The Lab is a protocol implementation, not a physical dependency

Check needs the executable Contract. Simulation additionally needs its
effectless exchange model. Real execution additionally needs the physical
Driver. Missing Codex, credentials or a Process service Driver must not block
an otherwise complete, effectless service simulation.

Existing mock `where` and recursive `match` are reused. Pure scripts select
responses from explicit request data; no hidden turn counter is needed.
Session identity and write/terminal bookkeeping are supplied by the same core
as real execution, not manufactured by the mock.

Service requires a profile-aware response validator: a script may produce
zero or more frames without `exit`, or a terminal with no later frame. A job
still requires exactly one final `exit` for every non-silent completion.
An uncovered request still fails the scenario; `_ => nothing` is explicit
silence, not a catch-all successful answer.

The complete Lab design must also cover process death without a new input,
delayed/unsolicited stdout or stderr, bounded pending writes, rejected frames,
and a close acknowledgement that never arrives. The syntax and request/session
coordinates of those world actions must be decided; direct injection of a
forged mid-graph envelope or terminal is not an implementation shortcut.

If a scenario needs state in the *simulated external program*, define that
state's lifecycle, reset and reproducibility explicitly. Ordinary mocks are
stateless today; this document does not silently add `var` to them.

## 10. Reference dialogue and acceptance matrix

Handwritten world script, not a claim of executable source:

| Action | World response | Required resource evidence |
|---|---|---|
| Alice sends `alice-1`, about her garden | Tomatoes | Open process A once, accept the first message once |
| Bob sends `bob-1`, about his trip | Kyoto | Open process B once; A remains open |
| Alice sends `alice-2`, "how do I water them?" | Water tomatoes in the morning | Write to A; no third process opening |
| Bob sends `bob-2`, "how many days?" | Three days in Kyoto | Write to B; no fourth process opening |
| Alice's conversation closes under the chosen policy | A acknowledges termination | One A resource exit; B remains usable |
| Bob sends `bob-3` | A further reply | Still B; neither answer nor terminal is routed to Alice |

Scripted answers prove routing and lifecycle under that world; they do not
prove Codex memory. Reuse is established by retained opening/write references,
not just by counting business replies or observing no `exit`.

Additional reference scenario required by D-2026-09-10-3, still handwritten:

| Action | Required evidence |
|---|---|
| A and B are live; A dies | One A resource exit; A's Node instance and B's child remain open |
| Advance time with no new A input | No new A child opening and no resend |
| Alice sends a new request | Fresh A SessionRef under the same InstanceRef; only the new request is written |
| A late event or unsettled write names the old A Session | No delivery/write is retargeted to the fresh child |
| Explicitly stop A's Node instance | No fresh opening can revive that generation; B stays usable |

**Executable prerequisite evidence (10 September):** `machine_vm.rs` now tests
the fresh-opening scenario on the common Session engine with an effectless
WebSocket Contract, comparing uninterrupted and recovered causal histories.
A second test proves repeated child-only closure leaves Bob's resource usable
and does not renew Alice's Closing deadline. The
[closure follow-up](instance-stop-command.md#7-session-lifecycle-follow-up--10-september)
also records the direct-input scope refusal discovered by these probes.
These tests qualify reusable lifecycle machinery, not a resident Process,
JSONL service framing, Codex behavior, or executable Lab Session scripting.

The [shutdown-message follow-up](instance-stop-command.md#8-application-shutdown-is-data--10-september)
adds a common-Session probe for an accepted or ignored `quit`. A local write
settlement emits no exit and no implicit stop. Observed resource termination
leaves the Node open; separately requested bounded closure can instead end in
`OutcomeUnknown`. Both histories survive recovery without resending the write,
and neither affects Bob's live resource. This does not qualify physical signals.

Qualification must cover all of the following before calling Process complete:

1. Existing job framing, defaults, timeouts, error payloads and identity witnesses.
2. Two keyed resident processes, repeated turns, interleaved responses and stderr.
3. First-message once-only under pending open and write backpressure.
4. No cross-conversation/cross-resource SessionRef substitution.
5. Frame limits, malformed JSON/UTF-8, incomplete EOF and capacity saturation.
6. Process death before writing, after writing and with several requests pending.
7. Authored shutdown message, ignored message, cooperative system termination,
   forced cleanup, late frames, duplicate terminals and close uncertainty;
   write settlement and signal delivery must never prove resource exit.
8. Resource deadline races; separately, the gateway adapter's request timeout,
   late/unknown/notification classification and turn-completion behavior.
9. No automatic respawn without a new input; a fresh input-driven incarnation
   is fenced from old writes/events, without request retry or memory restoration.
10. Recovery of retained obligations without repeated external effects.
11. Lab/real same-shape conformance and registry admission without the physical Driver.
12. Hostile package validation, no_std and embedded size/heap evidence before qualification.

## 11. Repository contact — what is missing, exactly

| Evidence | Finding |
|---|---|
| `crates/rastok/src/stdlib.rs`, `PROCESS` | `service` is `ProfileSupport::Deferred`; the sheet currently fixes Outbound + Call |
| `crates/rastok/src/contract.rs`, `ConnectorContract` / `Profile` | Roles and exchange belong to the whole sheet, not the resolved profile; admitting Session service needs an explicit metamodel treatment |
| `crates/rastok-core/src/plan/validation.rs`, Process branch | Registry-free validation independently requires `mode = job`, Call and Outbound; changing only the checker cannot work |
| `crates/rastok-core/src/boundary.rs`, `SessionConnectorDriver` | Generic opening, write, close and inbound proposal seams already exist; write settlement is not a nested RPC response |
| `crates/rastok-core/src/runtime/vm.rs`, `select_session_ref` | A `meta.ext.session_id` currently selects a Session before instance-based opening. Chaining websocket and Process Sessions requires explicit resource scoping, not copying this token as though it named both |
| `crates/rastok-core/src/runtime/vm.rs`, `progress_session` | A client input without an explicit resource ref opens when no live Session remains. D-2026-09-10-3 accepts fresh input-driven opening for Process, separately from automatic restart; service still needs profile and resource-selection qualification |
| `crates/rastok/src/connectors/process/mod.rs`, `cancel` / `kill_group` | Current job cancellation forcibly kills the Unix process group and direct child; it does not implement the accepted cooperative service fallback |
| `crates/rastok-core/src/runtime/vm.rs`, `expire_closing` / `settle_session_next` | Generic cooperative close, forced abort at the retained settlement deadline and release after terminal commit are distinct handoffs; resident Process signal escalation still needs its physical Driver |
| `crates/rastok/src/compiler/check.rs`, `mocks` | Every non-silent mock leaf currently requires exactly one final `exit` |
| `crates/rastok/src/lab.rs`, `MockOutboundConnectorDriver` | Pure recursive dispatch already exists; execution is Call/completion-shaped, without resident-service world actions |
| `docs/spec/stdlib.md`, §3.3 and §9 | D-2026-09-10-2 supersedes Process-owned RPC and the nonexistent `instance.exit` endpoint; resource policies remain unsealed |
| `docs/examples/codex-websocket-gateway.rkg` | Current proof starts a Process job for each message; its success does not qualify one child per conversation |

## 12. Decision and identity gates

**Settled:** generic JSONL transport and resource lifecycle in Process,
application protocol in Operators (§5, D-2026-09-10-2), and input-driven fresh
opening without automatic restart (§6, D-2026-09-10-3), and application shutdown
messages separate from cooperative/forced system termination (§4,
D-2026-09-10-4). Exact-instance closure is separately executable under
D-2026-09-10-1. These decisions do not admit service.

Before admitting the resident service profile, still settle:

1. **Resource policy (§3/§4/§6):** admitted codecs and exact frame/queue caps;
   physical signal escalation and resource selection; conversation close versus socket
   disconnect. The latter is a gateway policy, not part of generic Process.
2. **Lab world actions (§9):** explicit resource coordinates and syntax for
   deferred/unsolicited frames, process death and close acknowledgement.
3. **Transport/identity:** resolved-profile role/exchange representation and the
   Process Contract version. New behavior cannot reinterpret an existing tag,
   turn a job into a Session or re-bless old witnesses merely to pass tests.

The current v18 package already transports an exchange kind, roles and generic
Session records, but that is not proof that it can represent the complete
chosen design. Enumerate required facts first, then decide whether the package
is unchanged or an explicit format amendment is needed. No id or tag is
reserved for service by this review document. The v18 closure allocation is
already settled separately. A Process Contract move and a package move are
separate decisions; unrelated contracts do not move automatically.

After review: amend the normative sheet/decision, implement independent
validation and the Lab model, qualify the dialogue, then implement/qualify the
real Driver and gateway protocol. This order does not require launching Codex
or a paid model to prove the service semantics.
