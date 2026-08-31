# Hosted execution UI

> **Status: normative M1.9 target under D-2026-08-31-2.** The hosted UI is
> an optional build capability and an execution controller over the existing
> VM, journal, Lab and accounting folds. Until each M1.9 gate lands, `--ui`
> remains unavailable rather than approximating this contract.

The UI is a local pedagogical and debugging surface for `rastok run` and
`rastok simulate`. It is not a second runtime, a durable monitor, a remote
administration plane or a replacement for OpenTelemetry. Compilation remains
the ordinary CLI experience; `check --ui` is not an M1 command.

## 1. Static capability and command surface

`hosted-ui` is a Cargo feature, not a source branch or a runtime plugin. It
may be included in a normal hosted distribution and omitted from minimal and
embedded images without changing a graph, plan package or semantic identity.
A binary built without it refuses `--ui` by name.

```text
rastok run app.rkg --ui [--ui-port PORT]
rastok simulate app.rkg --ui [--ui-port PORT]
```

`--ui-port` belongs to `--ui`, occurs at most once and accepts a non-zero TCP
port. Without it, the OS chooses a free port. Binding failure occurs before
graph execution. The server listens only on `127.0.0.1`, prints its URL and
never opens a browser automatically.

The page is embedded in the binary as self-contained HTML, CSS and JavaScript.
M1 uses no Node/npm build and no filesystem asset lookup. A later `rastok-ui`
crate may own the same protocol and assets without changing this surface.

## 2. Capability-protected loopback server

Loopback is not an authorization boundary: another browser origin can target
a local port. Each UI run therefore obtains an unpredictable capability from
the host OS before binding. The capability appears in the printed URL and is
required by every HTML, API, SSE and control request. It never enters source,
plan, package, journal, observation, accounting, diagnostic or semantic
identity.

One authorized browser client owns control at a time. Other authorized clients
observe the same run read-only. `Take control` atomically replaces the current
controller; commands from a stale controller refuse without changing the
machine. Control ownership is physical UI state, never a causal fact.

## 3. Execution states and safe points

Every UI session starts `Paused` before the first semantic transaction. The
states are:

```text
Paused --Resume--> Running --Pause--> PauseRequested --> Paused
   |                    |                                  |
   `----Step------------'                                  |
                       Faulted <----------------------------'
                       Completed
```

A `run` step is exactly one atomic VM transaction: one handler firing,
lifecycle transition, Loop continuation or Call settlement. It is never one
expression instruction, source line or arbitrary journal record.
Same-instant scheduler work which the execution model declares indivisible
remains one dated batch and is labelled as such.

Simulation has an additional authored-action boundary. Its primary `Step`
advances exactly the next available boundary: one `inject`, `advance` or
`expect` while the VM is quiescent, otherwise one ready VM transaction. One
click never applies an action and executes its resulting transaction together.
The page names that last boundary explicitly. An authored action says that no
Node fired; a transaction names its exact journal owner and trigger, including
Graph or Node lifecycle transactions rather than skipping them for visual
convenience.

`Pause` takes effect at a transaction boundary. No new inbound proposal is
admitted and no further handler starts. An outbound Call already opened may
continue in the external world; the boundary keeps polling it until its typed
terminal outcome is retained, then pauses before the corresponding handler
consumes that outcome. The UI displays `PauseRequested` and every outstanding
Call. Semantic time does not advance merely to make a paused timeout happen.

A Session is not a Call and may be intentionally long-lived. Pause never
closes it or waits for its terminal exit. Its transport may continue filling
the already-bounded Driver/mailbox path; the scheduler consumes no Session
message while paused. Backpressure remains the Connector contract.

If the one stepped transaction opens a Call, that Call is started only after
its OUTBOUND open is durable. The controller then follows the same
`PauseRequested` rule and stops with the completion visible but unconsumed.

## 4. Run controls

`run --ui` supports `Resume`, `Pause`, `Step`, `Retry step` after a fault and
`Run to completion`. A fault leaves the failing work reserved and commits
nothing, exactly as the VM contract already promises. The UI reports the
typed error, owner/handler, triggering causal identity, structured redacted
context, the no-commit guarantee and the legal next actions.

A physical Driver/polling fault is not relabelled as a transaction fault: it
may be observed after an earlier transaction committed. In that state the UI
preserves the retained history, says that prior commits remain, disables
`Retry step` and requires an explicit restart. Only a VM refusal which still
owns the same reserved work may advertise retry.

There is no `Reset` for a real run: external effects already visible to the
world cannot be undone. Restarting production execution is an explicit new
CLI run with its own authority.

After completion or a fault the UI remains available until `Close` or process
termination, so the complete in-memory history stays inspectable.

## 5. Simulation controls

`simulate --ui` lists every checked simulation and scenario but activates one
scenario at a time. A fresh selection or `Reset` creates a fresh VM, journal,
mock world and virtual clock. The controls distinguish the two levels already
present in the Lab:

- `Step: next action` / `Step: next transaction` labels and advances the next
  available boundary, never both in one click;
- `Next scenario action` explicitly applies one `inject`, `advance` or
  `expect`, and refuses while VM work remains;
- `Run until next assertion` stops immediately before the next `expect`;
- `Run to completion` preserves the ordinary CLI verdict and golden rules.

The page labels the next `Step` as an action or transaction before it runs and
keeps the specialized controls available. An assertion never becomes a graph
transaction, and stepping never rewrites the authored scenario.

## 6. Read model

The journal remains the only causal authority. The UI subscribes to it and to
the existing retained observation and `CostReport` queries. It may keep
disposable indexes or rendered slices, but a reconnect or refresh derives the
same view from the complete journal still held in memory. UI loss, a slow SSE
client or a closed browser cannot delay, fail or alter execution.

The initial HTTP snapshot describes:

- the validated Graph/Node/edge topology as a compound graph: the root `App`
  contains its direct Nodes and nested Graph scopes, and each nested Graph
  recursively contains its own direct children. Compact Node cards expose only
  declaration name, effective NodeType and NodeAspects. A white disk denotes
  an input port and a black disk denotes an output port; exact edges attach to
  those disks and carry their cumulative envelope count. The Graph/Node owner
  of the last stepped transaction is highlighted; selecting a Graph frame or
  Node exposes its exact authored declaration, including ports, attributes,
  handlers and guard expressions;
- controller and execution state;
- current logical instances, lifecycle and Node-private `var` cells;
- retained causal observations with Call/Session correlation;
- the exact `CostReport`, including separate currencies and `unpriced` rows.

SSE then announces monotonically increasing view revisions. Reconnection asks
for the current snapshot; no wall-clock timestamp or exporter duration is
invented. The browser virtualizes large tables, but every record retained by
the in-memory journal remains addressable for that run.

Edge counters are causal projections, not browser counters. INBOUND and
TRANSACTION route facts increment their exact canonical edge; an emission to
the App boundary or explicit discard is derived from that same committed
transaction and the validated plan. A scenario `inject` may therefore light
an input edge while correctly reporting that no Node ran. The highlighted
record range is disposable controller state; cumulative counts and Node
transaction totals are reconstructed from the complete journal after every
refresh.

Source definitions are compiler debug side-table data, not plan data. The UI
may show the exact declaration span retained by the successful compilation,
but it never reverse-engineers source from the categorical plan and never
treats source text as execution authority. A deployment that has stripped
that physical side table still exposes topology and reports that source is
unavailable.

Containment is likewise read from the validated lexical owner table. The
browser does not infer nesting from dotted paths or edge geometry. Graph
frames are presentation of that existing authority and add no runtime scope.

## 7. Payload and state presentation

M1 displays payload content only when it is already materialized and its root
is a struct, map, array or union. Scalar leaves inside that structure remain
visible. A scalar root or encoded/opaque payload exposes only its logical kind
and known byte size; the UI never forces decoding or materialization merely
for presentation.

Node state uses the same structured-only rule. Presentation consults the exact
resolved annotation table before building JSON:

- `@secret` content is impossible on the executable source path and is never
  rendered for a forged package;
- `@sensitive` and `@personal` paths are redacted with no reveal control;
- unclassified structured descendants render through a typed JSON tree.

Redaction happens before serialization. String search or post-format masking
is not an acceptable implementation.

## 8. Identity and non-goals

The UI adds no package field, plan-package version, semantic hash extension or
causal record. Its port, capability, controller, browser connections,
presentation revision and physical timing are deliberately outside every
identity recipe.

M1.9 does not provide remote binding, TLS, user accounts, durable history,
cross-process attachment, source hot reload, breakpoints, state mutation,
manual envelope injection, `File.watch`, exact commit timestamps or an OTLP
exporter. Those are separate decisions, not hidden approximations.
