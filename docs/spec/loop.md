# Loop — finite temporal recursion

> **Status: normative; [current format](identity.md).** Loop contributes
> no port; exhaustion extends a resolved host terminal under D-2026-09-08-1.
> [The v17 ledger](../architecture/loop-host-terminal-v17.md) records that host-terminal qualification.
> Operator, outbound Call and Graph hosts execute and recover, including nested
> runs, annotated recurrence, entry-preserving Graph measures, boundary-only
> bodies and unconditional same-context joins. The control-proof limits below
> remain explicit. [R7 §10](../architecture/rework-r7-manager-recovery.md#10-context-composition-and-exchange-recovery--2026-09-07)
> records the host checkpoint; [R9](../architecture/rework-r9-qualification.md)
> records the final migration and qualification.

> **Loop/Retry distinction — D-2026-09-14-4:** Loop advances authored state;
> Retry repeats one prepared Call under its own hard safety gate. Global
> `effect` classification is removed. §§11–12 distinguish the advisory Loop
> warning from Retry admission. See the [refactor contract](../architecture/retry-extension-refactor.md)
> and [common qualification](../architecture/retry-c4-qualification.md).

`Loop` is a temporal Extension which applies one finite host application,
then either terminates or re-enters the same host input. Recurrence is a
journal-derived manager transition, never an edge in the spatial DAG.

## 1. Contract at a glance

A Loop Node has:

- the host's existing input and outputs;
- qualified attributes `Loop.fuel`, `Loop.delay`, `Loop.measure`,
  `Loop.entry`, `Loop.terminal`, and the optional `Loop.expired` target;
- the immutable binding `Loop.state`;
- three qualified manager commands: `Loop.next`, `Loop.again`, and
  `Loop.done`.

Loop contributes no port. It extends the host input with temporal admission,
ordinary terminal outputs with the Done role, and the resolved failure
terminal with the manager-produced `LoopExpired` case.

Commands are not ports. They have no `PhysicalEdge`, cannot be a source, and
do not reserve the host input while a delay is pending. The manager is neither
a Node nor a participant in the spatial DAG.

```text
opening delivery -> host application -> exactly one verdict
                                      |
                Done / Stopped ------+----> close run
                Next / Again --------+----> manager command
                                                |
                                         optional delay
                                                |
                                  journalled temporal re-admission
                                                |
                                         same host input
```

## 2. Hosts

Eligibility is derived from the resolved application profile in the Connector's
Contract, not a source-facing contract name.

| Host | Admission |
|---|---|
| Operator | admitted |
| Graph | admitted |
| finite outbound Connector Call | admitted |
| Event, Session, inbound Call | `E-LOOP-HOST` |
| Process service mode | `E-LOOP-HOST` |

The initial standard outbound Call hosts are Process job, File, and
HttpClient. A Connector's Contract must transport a closed finite profile before
it can host Loop.

A Graph remains a structural scope. Its Loop manager observes causal work in
the contained Executors; the Graph gains no handler, application mailbox,
mutable application state, or application transaction. Generic lifecycle
records attributed to a Graph remain valid.

## 3. Entry and run identity

The recurrent entry is resolved in this order:

1. the sole input, when exactly one exists;
2. the input named `in`, when it is unique;
3. the input named by `Loop.entry`;
4. otherwise `E-LOOP-ENTRY`.

`Loop.entry` must name an input and is rejected when it would contradict a
unique derived entry.

Every external admission to the entry starts a distinct run. Re-admission
carrying an existing recurrence reference continues that run. The stable
identity is:

```text
LoopRunId = blake3-derive( 0x08, instance::Generation ‖ OpeningAdmissionId )
```

`0x08` is `IdKind::LoopRun` in the closed identity registry (envelope.md §2);
the string `rastok.loop.r2` names the protocol in the plan stream, never an
identity domain. `OpeningAdmissionId` is the exact admitted delivery/cursor
occurrence, not only `origin_id`. This distinction permits two fan-out deliveries from one
business origin to open two runs in the same keyed instance without collision.

Every run carries an explicit `LoopContext` containing at least owner,
run, iteration, and opening admission. Spatial routes within a Loop Graph
preserve it; leaving the Graph pops it; a nested Loop pushes another context.
A join may combine candidates for Loop control only when they carry the same
context. An unrelated delivery to the same instance cannot advance a run.

Several runs may coexist in one instance. Their Loop state, fuel, wake-up, and
pending command are separate; ordinary owner `var` state remains shared and
observes normal deterministic instance serialization.

## 4. Loop.state

`Loop.state` is the complete envelope admitted at the beginning of the
current application. It is immutable and bound to one `LoopRunId`.

Advance selects the proposed state (Next) or retains the current state
(Again). Admit then creates the next application envelope: DATA and the
travelling `meta.app`/`meta.ext` zones are preserved, `origin_id` remains the
business lineage, and the system metadata receives the recurrence admission id
and the command's causal parent. The admitted envelope is retained in the
Admit fact. “Again retains state” never means reusing an application's
causal id: that would repeat emission and Call identities on identical retries.

It is visible:

- in a Loop Operator's application Dispatch;
- in Executors reached inside a Loop Graph while the same context is active;
- in the core-owned transition of a Loop Connector Call.

A nested Loop masks the outer binding. Explicit access to an outer Loop state
is deferred. Instance lifecycle handlers `on start` and `on stop` are not
Loop applications and cannot read `Loop.state` or issue Loop commands.

## 5. Commands and durable acceptance

| Command | Operator form | routed form | Meaning |
|---|---|---|---|
| Next | `Loop.next = value` | `source -> Loop.next` | use the produced envelope as the next state |
| Again | `Loop.again` | `source -> Loop.again` | retain `Loop.state`; routed value is only the cause |
| Done | `Loop.done` | `source -> Loop.done` | close the run without a business envelope |

Next data must be assignable to the resolved entry type. It creates an
envelope by the ordinary propagation rule. Again accepts any causal source
because it retains the previous state. Done is Control.

A manager has a bounded, one-verdict-per-iteration command inbox. Acceptance
uses two journal facts:

```text
CommandProduced  -- in the producer's atomic transaction
CommandConsumed  -- in a later manager transaction
```

Pending commands are re-derived as Produced minus Consumed. This split is
mandatory: a crash after the producer commits but before the manager runs must
neither lose the command nor replay the producer transaction.

Direct commands and routed commands have distinct canonical command-source
identities. Both include run, iteration, kind, and exact causal source.
Commands are destinations for routing and capacity accounting, but never
`PhysicalEdge` endpoints.

## 6. Terminal outputs

`Loop.terminal` names host outputs whose emission supplies the implicit Done
verdict:

- by convention, existing `out` and `err` are terminal outputs;
- every other output is a frame unless selected explicitly;
- `Loop.terminal = [done, failed]` replaces the convention, rather than adding to it;
- without terminal outputs, terminal paths must issue `Loop.done`;
- Connector Call: the Contract terminal exit owns application completion and
  `Loop.terminal` is inapplicable.

The resolved list is transported canonically. Every named item must be an
output of the owner. An empty list, duplicate, input or command is
`E-LOOP-TERMINAL`. The optional `Loop.expired = failed` selects an existing
terminal output as the exhaustion target; it creates no port. A Call cannot
override either terminal selection: its Contract owns that role.

Non-terminal outputs are frames. They may coexist with Next or Again.
Several terminal outputs emitted by one atomic transaction coalesce into one
Done verdict; they do not count as multiple verdicts.

## 7. Verdict and quiescence

Every application selects exactly one:

```text
Next(state') | Again | Done | Stopped
```

Done comes from a terminal output, `Loop.done`, or an unredirected terminal
Connector exit. Stopped comes from `stop` and closes the instance.
`nothing` is only an explicit output disposition and is never a verdict.

No verdict at quiescence is `LoopNoVerdict`. Competing commands, command plus
terminal output, or command plus stop are `LoopProtocolViolation`. When the
validated Dispatch/routes prove such a path, checking rejects it as
`E-LOOP-CONTROL`.

A produced command may wait before application quiescence. The manager
consumes it only after all obligations with the same run context have closed:
internal deliveries, application transactions, Calls, deadlines, and descendant
commands. Work for another run does not block it. No application overlaps the
next application of the same run.

Host completion is:

| Host | Completion |
|---|---|
| Operator | commit of the entry Dispatch transaction |
| Graph | causal quiescence of the contained subgraph |
| Connector Call | exactly one terminal exit after all frames and Call closure |

## 8. Fuel and expiry

`Loop.fuel = N` is a positive plan integer and permits at most N
applications, the first included:

```text
run(0, state) = Expired(state)
run(N + 1, state) =
  application(state), then
    Done | Stopped -> close
    Next(next)     -> run(N, next)
    Again          -> run(N, state)
```

Next and Again each consume one unit. If the final permitted application asks
to recur, no re-admission and no `l:next` are created. The manager emits:

```rastok
LoopExpired { last_state: EntryType }
```

For Next, `last_state` is the proposed next DATA. For Again, it is the retained
DATA. The emitted envelope inherits the command's causal provenance and policy.
The target is the existing `err` of an Operator/Graph, or the Contract's
terminal `exit` of an outbound Call. `Loop.expired = failed` overrides it only
for an Operator/Graph and requires `failed` in the resolved terminal set.

An Operator/Graph must declare that output with a tagged union accepting
`LoopExpired { last_state: EntryType }`. No output is synthesized. A Call's
existing terminal pin is extended by the checker; its base Driver Contract
still cannot produce the manager-only case. The plan carries the resolved
target and complete type, independently validated; the Execution never tests a port
spelling. Ordinary output disposition applies:

```rastok
worker.err -> failed
// or, when deliberately unused:
worker.err -> nothing
```

For a root `App + Loop`, the resolved terminal is already a public App output;
no parent disposition is needed. Exhaustion closes the run before this emission
is routed. It cannot command the exhausted run again. Internal fallback uses
a wrapping Graph.

An explicit fuel normally makes expiry productible. A valid measure-only Loop
proves expiry impossible: no target or port is required and no contributed-port
warning exists. `W-DEAD-OUTPUT` applies only to outputs declared by the author.
If fuel and measure coexist, expiry is impossible
only when the proof also establishes termination within the explicit fuel;
otherwise the checker remains conservative.

## 9. Delay

The accepted forms remain:

```rastok
Loop.delay = { kind = fixed; after = duration("5s") }
Loop.delay = {
  kind = exponential
  first = duration("100ms")
  ceiling = duration("2s")
}
```

The delay begins when the manager consumes Next or Again after quiescence.
It uses logical time and the recurrence deadline kind (byte 4). Arithmetic
saturates before overflow.
There is no jitter in the current subset.

No destination capacity is held during the wait. At the due instant, Admit
rechecks run and generation, proves the capacities selected from the
remembered entry port, commits `l:next` and the routes, then exposes the
deliveries. Closing makes stale wake-ups inert.

## 10. Proof of finitude

At least one of `Loop.fuel` or `Loop.measure` is required. The direct proof
fragment remains deliberately small:

- integer measure;
- direct path rooted at `Loop.state.data`;
- a positivity guard;
- literal strict decrease on every recurrent Dispatch leaf;
- closed Dispatch or predicates understood by the interval checker;
- certificate re-derived independently by the no-std validator.

The derived internal bound is `max(measure(initial), 0) + 1`. Opaque helper
calls, Connector results, external joins, or unsupported arithmetic require
fuel. Finitude bounds the number of applications; each application must also
terminate, for example through positive Call timeouts and bounded guests.

## 11. Connector Call rules

A Loop Connector must resolve to a finite outbound Call with one request input,
closed frame cardinalities, and exactly one terminal exit. Event, Session,
inbound Call, and resident-service modes refuse.

`out` and `err` are frames, not implicit terminals. A frame may select a
command, but the manager still waits for the Call exit. A Contract port with
zero-or-more frames cannot route unconditionally to a command unless the
contract proves that at most one matching frame can occur.

At terminal exit:

1. an exit-case route may select a command;
2. an earlier frame command remains the sole verdict;
3. otherwise the exit supplies natural Done;
4. disposing the exit with `nothing` does not suppress natural Done.

Every terminal case retains its Contract classification: `before_effect`,
`outcome_unknown`, or `after_effect`. Loop does not prove safe repetition of
external effects: an authored `Loop.again` path which may repeat a visible
Call without `idempotent = true` produces `W-LOOP-REPEAT`, including successful
sibling Calls in a Graph. A wholly BeforeEffect path does not warn. `Loop.next`
chooses a new state and is not diagnosed by this warning.

Protected repetition of the same operation belongs to `Retry`: selected
OutcomeUnknown/AfterEffect cases without the guarantee refuse with
`E-RETRY-EFFECT`. The former `E-LOOP-EFFECT` and global `effect` attribute are
retired. The [C1 Contract tables](../architecture/retry-c1-algebra.md)
retain conservative terminal visibility; no warning contributes to identity.

## 12. Operator and Graph control

For a Loop Operator, every reachable entry-Dispatch leaf must select one
verdict. Other handlers on the same owner may emit contextual frames, but
cannot become another application entry. An uncorrelated delivery cannot issue
a command for an active run.

For a Loop Graph, the opening boundary route pushes the context. Contained
Executors retain it; a terminal boundary crossing or routed command produces
the verdict in the child transaction. The manager waits for run-scoped
quiescence. A route leaving the Graph pops the context, so downstream parent
work does not delay the application.

A child Call's terminal settles that Call only. Unlike a Connector which
itself hosts Loop, it supplies no implicit Done to its containing Graph.
Its output routes participate in the Graph's verdict proof together with
every other child path. The proof preserves Contract response/terminal
correlation. The separate `W-LOOP-REPEAT` analysis covers Calls that `again`
could repeat, including sibling Calls which did not select the command;
this is an advisory check, not the Loop control proof.

A Graph has no `on in`, `on start`, or `on stop`. Window state and
lifecycle emissions belong in an Operator child. The generic lifecycle
controller still writes `l:start`, `l:closing`, and `l:stop` for the
Graph instance.

## 13. Closing and recovery

Closing wins over recurrence:

- no new re-admission;
- stale deadlines are inert;
- pending commands close with the run;
- nothing migrates to a successor generation;
- descendants close inner-first.

The r2 fold contains Open, CommandProduced, CommandConsumed, Next/Again, Admit,
Close, and Expired evidence. The current wire keys are specified in
[identity.md §3](identity.md#3-the-canonical-record-form).

Spatial recovery reconstructs instance generations, state deltas, unconsumed
deliveries and cursors, Calls, deadlines, runs, pending commands, and due
re-admissions. A produced but unconsumed command reappears; its producer is not
re-executed. RAM queues are caches over this fold, never authority.

The r2 packet is validated with its carrying record. Instance scope follows
exact parent-generation lifecycle links, routed contexts refer to actual
retained deliveries, and an Expired fact must publish the same ordinary
expiry envelope on the plan-resolved host terminal, with the emission id
derived from that actual port. Packet algebra alone does not establish these links
or prove run-scoped quiescence.

`rastok.loop.r2` identifies this protocol in the plan. Its ordinary expiry
emission uses the resolved host terminal; Loop contributes no port.

## 14. Examples

These are interface excerpts. Complete checked programs, including every
disposition and their Lab scenarios, are [countdown](../../examples/countdown.rkg)
and the [Loop debugger fixture](../../crates/rastok/tests/fixtures/ui-two-retries.rkg).
[Keyed retry](../../examples/keyed-retry.rkg) now teaches the distinct Retry
protocol, not authored Loop state progression.

```rastok
operator countdown : Loop {
  input in: struct { n: int }
  output out: text
  output err: union { LoopExpired { last_state: struct { n: int } } }
  Loop.fuel = 100

  on in match {
    data.n <= 0 => out = "liftoff"
    _           => Loop.next = { n = data.n - 1 }
  }
}
```

```rastok
graph refinement : Loop {
  input in: Draft
  output out: Answer
  output failed: RefinementFailure
  // RefinementFailure includes LoopExpired { last_state: Draft }.
  // It must also accept every model.exit case routed by `_` below;
  // otherwise route those cases through a typed error-adapter Operator.
  Loop.fuel = 5
  Loop.terminal = [out, failed]
  Loop.expired = failed

  in -> model
  model.out -> judge
  judge.accepted -> out
  judge.revised -> Loop.next
  model.exit -> match {
    Completed      -> nothing
    TimeoutExpired -> Loop.again
    _              -> failed
  }
}

refinement.out -> publish
refinement.failed -> alert
```

## 15. Named diagnostics

| Diagnostic | Meaning |
|---|---|
| `E-LOOP-HOST` | resolved host profile is not a finite application |
| `E-LOOP-ENTRY` | recurrent entry is absent or ambiguous |
| `E-LOOP-TERMINAL` | terminal set is absent, contradictory, or ill-typed |
| `E-LOOP-CONTEXT` | work cannot be associated with one exact run; currently a runtime refusal name, reserved as a dedicated tool code in the diagnostic registry |
| `E-LOOP-CONTROL` | a reachable path has zero or competing verdicts |
| `E-LOOP-CARDINALITY` | a Call output can issue more than one command per application |
| `W-LOOP-REPEAT` | authored Again may repeat a visible Call without an original-plan idempotence guarantee; advisory, not Retry's admission gate |
| `E-UNWIRED-OUTPUT` | productible exhaustion lacks a compatible host terminal or output disposition |
| `E-WIRING-MIGRATION` | removed `x.Loop.expired` wiring; use the resolved `x.err`, `x.exit`, or custom terminal |
| `LoopNoVerdict` | dynamic quiescence found no verdict |
| `LoopProtocolViolation` | dynamic execution attempted competing verdicts |

## 16. Deferred by name

Saga execution; Loop on Event, Session, inbound Call, and Process service;
several recurrent entry ports; internal fallback from exhaustion; explicit
outer-Loop state access; detachable obligations; affine, lexicographic, opaque
Connector, or Rocq proofs; jitter.

A join inside a Loop Graph cannot combine a candidate carrying the active
`LoopContext` with an unrelated delivery arriving from outside that context.
The runtime guard names `E-LOOP-CONTEXT` and consumes neither candidate; the
checker may refuse the surrounding application earlier through the bounded
control-proof gate. Data
needed by every application must enter through the opening state, be retained
in `Loop.state`, and travel through `Loop.next`; alternatively, perform the
join outside the Loop Graph. Cross-context joins are deferred until they have
an explicit run-association rule.

The spatial control proof admits an unconditional join only when each
application supplies exactly one candidate to each input. A join with `where`,
`within` or `last`, or a descendant lifecycle/lifetime-dependent application,
requires a stronger per-run completion proof and refuses at check today. The
proof also refuses beyond 1,024 distinct outcome combinations; it never admits
a program using approximate evidence. Joins and windows outside a Loop
application are unaffected.

The common lifecycle command `instance.stop` may close its own Loop Graph;
the run ends with `Stopped` and pending wakeups remain generation-fenced.
Child-only closure inside that application still requires the deferred
lifecycle-dependent proof. Request it from an enclosing non-Loop Graph or
close the Loop scope itself; no synthetic port or Loop command is introduced.

## 17. Related documents

- [extensions.md](extensions.md)
- [execution-model.md](execution-model.md)
- [delivery-semantics.md](delivery-semantics.md)
- [instances.md](instances.md)
- [identity.md](identity.md)
- [rework-r0-algebra.md](../architecture/rework-r0-algebra.md)
- [rework-r0-reference-traces.md](../architecture/rework-r0-reference-traces.md)
- [rework-r1-host-contracts.md](../architecture/rework-r1-host-contracts.md)
