# R1 — Loop hosts and their Connector contracts

> **Status (2026-09-05): R1 frozen for review.** This document closes the
> contract questions which R0 deliberately left at the host boundary. It is a
> target contract for plan v16; current v15 code still admits only Graph Loop.

## 1. Result protected

Loop may wrap three different host shapes without inventing three meanings:

> one exact opening admission starts one finite application; the application
> closes its own obligations; exactly one recurrence-or-terminal verdict is
> retained; only then may the manager advance the run.

The host contract must answer, without consulting a Driver registry at runtime:

1. which input opens an application;
2. which facts belong to that application;
3. when the application is complete;
4. which outputs are frames and which facts are terminal;
5. how many values each port/case may produce;
6. whether repeating after a terminal fact is safe.

## 2. Contract vocabulary

```text
ApplicationProfile = {
  host_kind,
  opening_inputs,
  entry_rule,
  completion_rule,
  output_profiles,
  terminal_profile,
  recurrence_sources,
  proof_profile
}

OutputProfile = {
  port,
  cardinality,
  ordering,
  terminal_role,
  productive_cases?
}

Cardinality = Never | ZeroOrOne | ExactlyOne | ZeroOrMore | OneOrMore

TerminalVisibility = BeforeEffect | OutcomeUnknown | AfterEffect
```

`Cardinality` is per application, not per instance or run. It is validation
material for productibility and verdict uniqueness. It does not add a counter
to an envelope.

The three terminal-visibility classes mean:

- `BeforeEffect`: the contract proves no externally visible operation began;
- `OutcomeUnknown`: visibility may have occurred or the same typed case can be
  raised on both sides of the effect boundary;
- `AfterEffect`: the contract proves the operation became externally visible.

The ordering is a safety lattice, not a claim about business success. Both
`OutcomeUnknown` and `AfterEffect` require repeat-safe effect semantics.

## 3. Uniform Loop admission

### 3.1 Entry resolution

All hosts use the same source rule:

1. exactly one input means that input;
2. otherwise exactly one input named `in` means `in`;
3. otherwise `Loop.entry = <input>` is required;
4. zero inputs or a non-input name refuses `E-LOOP-ENTRY`.

Every ordinary admission to the resolved entry opens a distinct LoopRun. An
admission already carrying that run's recurrence reference re-enters it instead
of opening another run. Other host inputs do not implicitly open or join a run.

### 3.2 Application context

The exact `LoopContext` of the opening delivery identifies application work.
Only an activation carrying that context can:

- read the corresponding `Loop.state`;
- produce a command for the run;
- count toward that run's quiescence;
- cause a terminal output to become its implicit Done verdict.

An unrelated delivery to the same keyed instance remains serialized by the
instance mailbox but cannot accidentally close or advance the run. Multiple
runs may coexist in one instance; their private Loop state and manager commands
are distinct, while ordinary owner `var` state remains shared and therefore
observes the deterministic transaction serialization.

### 3.3 Lifecycle isolation

`on start` and `on stop` are instance lifecycle handlers, not Loop
applications. They cannot read `Loop.state` or issue `Loop.next`,
`Loop.again`, or `Loop.done`. Their ordinary output emissions retain their
normal lifecycle cause and do not vote in a run.

## 4. Operator host

| Contract item | Operator Loop |
|---|---|
| opening | ordinary delivery to resolved entry input |
| application transition | the one handler activation which consumes that delivery |
| completion | successful commit of the selected Dispatch transaction |
| frames | any non-terminal ordinary output emitted by that transaction |
| natural Done | one or more resolved terminal outputs in that transaction |
| explicit verdict | one manager command in that transaction |
| mutable state | owner `var`, shared by the instance and journaled as delta |
| proof | Dispatch leaves, closed cases, and supported integer measure |

The entry activation has exactly one handler. Every reachable Dispatch leaf
must select exactly one of:

- `Loop.next = value`;
- `Loop.again`;
- `Loop.done`;
- at least one `Loop.terminal` output;
- `stop`.

`nothing` is not a verdict. A terminal output and a command in one leaf is
`E-LOOP-CONTROL`. Several terminal outputs created in the same transaction,
including an ordered multi-emission, coalesce to one implicit Done verdict.

Other Operator handlers may exist for non-entry inputs. They are ordinary
instance transactions and affect a run only when their input delivery carries
that run's context. In v16 such a handler may emit frames but may not become a
second application entry. If topology could inject an uncorrelated delivery
into a verdict-producing auxiliary handler, checking refuses
`E-LOOP-CONTEXT`.

An Operator can use `Loop.measure` without explicit fuel only when every
recurrent leaf lies in the checked proof fragment. Opaque helper calls, external
facts, or unsupported arithmetic require positive `Loop.fuel`.

## 5. Graph host

| Contract item | Graph Loop |
|---|---|
| opening | boundary admission at resolved entry input |
| application transition | ordinary contained Executor transactions under one LoopContext |
| completion | run-scoped causal quiescence of the contained subgraph |
| frames | non-terminal boundary outputs and any internal emissions |
| natural Done | first retained crossing of a resolved terminal boundary output |
| explicit verdict | child transaction routes an output to a manager command |
| mutable state | none on Graph; state belongs to contained Executors |
| proof | re-derived paths from entry through closed Dispatch/routes to commands/terminals |

The Graph itself has no application transaction. Its manager gates the opening
boundary, then admits the ordinary contained routes. A terminal boundary route
records an implicit Done command in the producing child's transaction. The
manager does not consume it until every obligation in the same LoopContext and
Graph scope is closed.

Only the first verdict command can occupy a run iteration's one-slot command
inbox. A later child transaction attempting another verdict cannot commit and
reports `LoopProtocolViolation`; when the topology and Dispatch tree prove this
possibility, checking rejects it as `E-LOOP-CONTROL`.

Routes which leave the Graph pop its LoopContext. Downstream work in the parent
does not delay Graph quiescence. Routes between descendants retain the context.
A nested Loop pushes its own run; its commands target the inner manager unless
a qualified child path explicitly targets another owner. Explicit access to an
outer Loop's state remains deferred.

`Loop.measure` without fuel is admitted only when the validator can follow
every recurrent path through pure, closed Operator Dispatch and prove the
supported literal decrease. A Connector result, open predicate, join with
external candidates, or other opaque step requires fuel.

## 6. outbound Connector Call host

| Contract item | Connector Call Loop |
|---|---|
| opening | request delivery to the Contract's sole outbound Call input |
| application transition | request preparation, one OUTBOUND Call, correlated Driver events, Connector transactions |
| completion | exactly one terminal `exit` after every application frame |
| frames | Contract-declared `out`/`err` values before the terminal exit |
| natural Done | terminal exit when no recurrence/explicit-Done command was selected |
| explicit verdict | a frame or exit case routes to a manager command |
| mutable state | core-owned Contract/exchange state; no DSL `var` |
| proof | positive fuel required in v16; opaque Driver outcomes cannot discharge a measure |

The resolved Contract owned by the Connector, not its source-facing name, determines eligibility:

```text
roles include Outbound
exchange == Call
profile is finite
exactly one request input
exactly one terminal exit per admitted request
all frame cardinalities and terminal cases are closed
```

An inbound Call, Event, Session, or resident service profile fails
`E-LOOP-HOST`. `Loop.entry` is derived from the request input and may only
restate it. `Loop.terminal` is inapplicable because the Contract owns the exit
terminal; writing it is `E-LOOP-TERMINAL`.

### 6.1 Frames and the terminal exit

A frame may be routed to `Loop.next`, `Loop.again`, or `Loop.done`, but the
manager waits for the Call's exit. If more than one frame selects a command,
the completion fails before commit. For a port with `ZeroOrMore` cardinality,
checking rejects an unconditional command route unless a closed Contract rule
proves at most one selected frame.

At exit:

1. an exit-case route may select a command;
2. an earlier frame command, if any, remains the sole verdict;
3. otherwise the terminal exit supplies natural Done;
4. `exit -> nothing` disposes the exit envelope but does not suppress natural
   Done;
5. routing the exit to an ordinary business destination also retains natural
   Done.

This separates output disposition from application control. `nothing` never
becomes a hidden Loop command.

### 6.2 Repetition safety

Every selected `Next` or `Again` is checked against the most conservative
visibility class reached by the application:

| terminal visibility | `effect = none` | `idempotent` | `compensable` | `irreversible` |
|---|---:|---:|---:|---:|
| `BeforeEffect` | allow | allow | allow | allow |
| `OutcomeUnknown` | allow | allow | `E-LOOP-EFFECT` | `E-LOOP-EFFECT` |
| `AfterEffect` | allow | allow | `E-LOOP-EFFECT` | `E-LOOP-EFFECT` |

`compensable` describes a separate undo protocol; it does not prove that
repeating the forward operation is safe. An idempotency header or Driver
feature is relevant only when the Contract makes it part of its
`idempotent` effect semantics.

If a frame selects recurrence before the terminal case is known, the validator
uses `AfterEffect`: a response frame proves the external operation was visible.

## 7. Standard Contract v2 profiles

Removing implicit unwired policy and adding productibility/effect-stage facts
changes the six executable sheets once to `@2`. The tables below are the R1
contract source for R4's Registry-free mirrors.

### 7.1 Contracts for Loop-admissible Connectors

#### `rastok/process@2`, profile `job`

| Port/fact | Per application | Order/condition |
|---|---|---|
| `in` | exactly one | sealed request |
| `out` | zero or more | ordered stdout frames |
| `err` | zero or more | ordered stderr frames |
| `exit` | exactly one | after all committed frames; terminal |

The current completion publishes all frames and the exit atomically. A future
streaming profile must preserve the cardinality contract and cannot reuse this
profile silently.

| Exit case | visibility | rationale |
|---|---|---|
| `SpawnFailed` | `BeforeEffect` | OS refused child creation |
| `Exited` | `AfterEffect` | child was observed after execution |
| `TimeoutExpired` | `OutcomeUnknown` | child may have acted before timeout |
| `OutcomeUnknown` | `OutcomeUnknown` | definition |
| `DecodeFailed` | `OutcomeUnknown` | same case covers pre-open request encoding and post-run decoding |
| `CaptureFailed` | `OutcomeUnknown` | process may have run before capture failed |
| `OutputOverflow` | `OutcomeUnknown` | capture bound may win after the child acted |

Process requires authored `effect`; therefore only `idempotent` jobs may recur
after any case except `SpawnFailed`. `mode = service` remains deferred and is
not a Loop host.

#### `rastok/file@2`, profiles `read` and `replace`

| Port/fact | `read` | `replace` |
|---|---:|---:|
| `in` | exactly one | exactly one |
| `out` | exactly one iff exit is `Completed` | `Never` |
| `exit` | exactly one | exactly one |

| Exit case | visibility | rationale |
|---|---|---|
| `InvalidPath` | `BeforeEffect` | portable path validation precedes open |
| `NotFound`, `NotAFile`, `PermissionDenied` | `BeforeEffect` | requested logical operation did not commit |
| `AtomicReplaceUnavailable` | `BeforeEffect` | required target replacement did not occur |
| `Completed` | `AfterEffect` | read observation or replacement completed |
| `ReadLimitExceeded` | `AfterEffect` | bounded read work occurred; the contract remains idempotent |
| `IoFailed`, `TimeoutExpired`, `OutcomeUnknown` | `OutcomeUnknown` | target visibility cannot be proven uniformly |

Both executable File profiles pre-declare `effect = idempotent`, so every class
is repeat-safe. `watch` is a deferred inbound profile and not a v16 Loop host.

#### `rastok/http-client@2`

| Port/fact | Per application | Order/condition |
|---|---|---|
| `in` | exactly one | complete request body |
| `out` | exactly one iff exit is `Completed` | complete usable response, including non-2xx |
| `exit` | exactly one | after `out` when Completed; terminal |

| Exit case | visibility | rationale |
|---|---|---|
| `DnsFailed`, `ConnectFailed`, `TlsFailed` | `BeforeEffect` | zero-RTT is disabled and request bytes were not visible |
| `Completed` | `AfterEffect` | a complete response proves request execution reached the peer |
| `ProtocolFailed` | `OutcomeUnknown` | failure can follow request visibility |
| `TimeoutExpired`, `OutcomeUnknown` | `OutcomeUnknown` | terminal observation cannot prove visibility |
| `BodyTooLarge` | `OutcomeUnknown` | one case covers pre-open request bound and post-effect response bound |
| `DecodeFailed` | `OutcomeUnknown` | complete/partial response or local preparation may select the same portable case |

The node's required `effect` determines which recurrence routes are admitted.
Redirects and status retries remain graph policy; the Driver never performs a
hidden Loop.

### 7.2 Contracts for non-Loop Connectors which still move to `@2`

#### `rastok/http-server@2`

Inbound Call, therefore `E-LOOP-HOST`. Per admitted request, `out` publishes
exactly one decoded request; `in` accepts zero or one response according to
`ack`/`respond`; `exit` occurs exactly once for the admitted Call. Pre-admission
HTTP refusals mint no Call and no exit. All productible outputs require explicit
disposition in the containing Graph.

#### `rastok/websocket-client@2`

Outbound-opened Session, therefore `E-LOOP-HOST`. One session has zero or more
ordered `out` messages, zero or more graph-to-peer `in` writes, and exactly one
terminal `exit`. Session retry/reopen is not smuggled into Loop Call semantics.

#### `rastok/websocket-server@2`

Inbound-and-outbound accepted Session, therefore `E-LOOP-HOST`. One session has
zero or more ordered peer messages on `out`, zero or more writes on `in`, and
exactly one terminal `exit`. A handshake rejected before session admission
creates no fake session.

## 8. Productibility contributed by Contracts

Each v2 contract mirror exposes, per resolved profile:

```text
PortProduction {
  direction,
  cardinality,
  cases?,
  order_group?,
  terminal?
}
```

This replaces `UnwiredPolicy`; it does not choose a destination. Examples:

- File replace `out = Never`, so no destination is required and the checker
  emits `W-DEAD-OUTPUT` if the port is explicitly declared/exposed;
- Process job `err = ZeroOrMore`, so it must be routed or explicitly discarded;
- each closed exit case is possible unless the resolved profile proves
  otherwise;
- HttpClient Completed couples one `out` before the one `exit`.

Core validation compares the transported port table, production profile,
terminal cases, roles, exchange, effect domain, and behavior against the exact
Registry-free mirror for all six standard identities.

## 9. Loop terminal resolution by host

| Host | `Loop.entry` | `Loop.terminal` | natural terminal |
|---|---|---|---|
| Operator | derived or authored input | one output default; list required for several | terminal output in the application transaction |
| Graph | derived or authored boundary input | one output default; list required for several | terminal boundary crossing under the run context |
| outbound Connector Call | Contract request input; restatement only | forbidden | exactly one terminal exit not overridden by a command |

`Loop.expired` is never a member of `Loop.terminal`. It is produced after a
recurrence verdict exhausts fuel and belongs to the parent routing scope.
On a root `App + Loop` Graph, it is a public App-boundary output because no
lexical parent exists; public termination satisfies the disposition rule.

The manager retains the complete state envelope. The public payload is:

```rastok
LoopExpired { last_state: EntryType }
```

`last_state` is the retained/proposed envelope's `data`; the new expired
envelope receives ordinary manager-emission metadata from the command cause.

## 10. Quiescence details

The run-scoped obligation set is closed and enumerable:

```text
pending ordinary deliveries whose LoopContext contains run
active Executor transaction for run
open outbound Call / Session write opened by a transaction for run
dated application obligation tagged with run
pending child-manager command whose context contains run
pending terminal boundary crossing inside the host scope
```

The selected command is excluded so it cannot wait on itself. A lifecycle
Closing obligation is not ignored: Closing wins, invalidates the run, and then
drains/cancels under the existing inner-first protocol.

For a Connector Call, terminal exit is required before quiescence even when an
earlier response frame selected a command. For a Graph, obligations past its
boundary are excluded. For an Operator, a successful transaction normally
reaches quiescence immediately.

## 11. Named rejections

| Code | Condition |
|---|---|
| `E-LOOP-HOST` | Event, Session, inbound Call, service profile, or otherwise non-finite host |
| `E-LOOP-ENTRY` | no unique recurrent input |
| `E-LOOP-TERMINAL` | ambiguous/mistyped terminal list or any list on Connector Call |
| `E-LOOP-CONTROL` | a reachable application leaf has zero or multiple verdicts |
| `E-LOOP-CONTEXT` | a route/join can mix run contexts or an uncorrelated activation can vote |
| `E-LOOP-CARDINALITY` | a possibly repeated frame can create more than one command |
| `E-LOOP-EFFECT` | recurrence can repeat an unsafe visible or ambiguous effect |
| `E-UNWIRED-OUTPUT` | a possible port/case has no disposition |

Runtime defense uses `LoopNoVerdict`, `LoopProtocolViolation`,
`LoopContextMismatch`, and `ContractViolation`; none silently drops a value or
repeats an effect.

## 12. R1 closure matrix

| Required question | Closed answer |
|---|---|
| admissible inputs | exact entry resolution in §3.1 |
| Operator completion | one Dispatch commit |
| Graph completion | contained run-context quiescence |
| Connector completion | exactly one outbound Call exit |
| frame cardinality | standard v2 tables in §7 |
| terminal classification | conservative visibility tables in §7.1 |
| `effect` crossing | matrix in §6.2 |
| opaque termination | positive fuel required for Connector Call |
| several runs per instance | exact admission identity + LoopContext; tx serialized |
| false external root claim | corrected: correlated outbound outcome is not INBOUND |

R1 adds no executable code and allocates no wire tag. R2 incorporates these
answers into the live specification and design-decision ledger, then stops for
review before R3.
