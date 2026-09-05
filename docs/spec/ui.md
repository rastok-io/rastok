# Hosted execution UI

> **Status: normative M1.9 target under D-2026-08-31-2 and
> D-2026-09-02-1.** The hosted UI is
> an optional build capability and an execution controller over the existing
> VM, journal, Lab and accounting folds. Until each M1.9 gate lands, `--ui`
> remains unavailable rather than approximating this contract.
>
> **Paused by D-2026-09-05-1.** This document describes the retained v15 UI
> work and intentionally still names projections such as Graph handlers. It is
> not authority for the v16 execution model. R9 will remove every vanished
> port/control projection and arbitrate the simulation/experiment command
> surface before UI work resumes.

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
The compact sticky bar names `Rastok UI`, shows `SIMULATION` or `RUN` beside the
execution state, and uses accessible icon transport controls. Its colors follow
the host `prefers-color-scheme`; light and dark are both first-class. The page
contains status and typed diagnostics, not generic operating instructions.

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
Paused --Play--> Running --Pause--> PauseRequested --> Paused
   |                    |                                  |
   `----Step in---------'                                  |
                       Faulted <----------------------------'
                       Completed
```

A `run` boundary is exactly one atomic VM transaction: one handler firing,
lifecycle transition, Loop continuation or Call settlement. It is never one
expression instruction, source line or arbitrary journal record.
Same-instant scheduler work which the execution model declares indivisible
remains one dated batch and is labelled as such.

Simulation has an additional authored-action boundary. Its primary `Step in`
advances exactly the next available boundary: one `inject`, `advance` or
`expect` while the VM is quiescent, otherwise one ready VM transaction. One
click never applies an action and executes its resulting transaction together.
The page names that last boundary explicitly. An authored action says that no
Node fired; a transaction names its exact journal owner and trigger, including
Graph or Node lifecycle transactions rather than skipping them for visual
convenience.

A manual `Step in` begins and ends in `Paused`, even when that boundary opens a Call.
The open Call remains visible and its Driver may retain a real completion, but
it does not turn the already-paused debugger into `PauseRequested`. This keeps
scripted silence and timeout tests usable: later Lab `expect` and `advance`
actions remain reachable one boundary at a time. `PauseRequested` belongs only to
an explicit `Pause` issued from `Running`; that command keeps its separate
promise to wait for an already-open Call's safe return boundary.

The UI uses three intentionally different ordinals:

- `A<n>` is the authored scenario-action cursor;
- `B<n>` is one retained atomic UI boundary, beginning at the initial `B0`;
- `R<n>` is a zero-based causal journal-record index.

Every authored action, scheduled VM transaction or retained Connector boundary
fact that changes the logical read model advances `B` once. Universal lazy
`l:start` transitions remain individually stepable boundaries; the UI does not
hide them to manufacture apparent movement. `Play`, repeated `Step in` and
`Continue` use the same execution transition and must finish with the same causal
records and delivered envelopes. One boundary may legitimately retain several
causal records atomically, so `B` and `R` are never presented as interchangeable.

`Pause` takes effect at a transaction boundary. No new inbound proposal is
admitted and no further handler starts. An outbound Call already opened may
continue in the external world; the boundary keeps polling it until its typed
terminal outcome is retained, then pauses before the corresponding handler
consumes that outcome. The UI displays `PauseRequested` and every outstanding
Call. The hosted wall-to-logical mapping freezes on both `Paused` and
`PauseRequested`: time spent inspecting the run does not age Call timeouts,
instance lifetimes or delayed Loop continuations. Resume continues from the
same logical instant. A real external outcome retained during the pause stays
retained; freezing time never fabricates, discards or rewrites it.

A Session is not a Call and may be intentionally long-lived. Pause never
closes it or waits for its terminal exit. Its transport may continue filling
the already-bounded Driver/mailbox path; the scheduler consumes no Session
message while paused. Backpressure remains the Connector contract.

If the one manually advanced transaction opens a Call, that Call is started
only after its OUTBOUND open is durable and the controller remains `Paused`
with the open Call visible. Only an explicit `Pause` requested from `Running`
uses `PauseRequested` and waits for the completion to become visible but
unconsumed.

## 4. Run controls

`run --ui` supports `Play`, `Pause`, `Step in`, `Retry` after a fault and
`Continue`. A fault leaves the failing work reserved and commits
nothing, exactly as the VM contract already promises. The UI reports the
typed error, owner/handler, triggering causal identity, structured redacted
context, the no-commit guarantee and the legal next actions.

A physical Driver/polling fault is not relabelled as a transaction fault: it
may be observed after an earlier transaction committed. In that state the UI
preserves the retained history, says that prior commits remain, disables
`Retry` and requires an explicit restart. Only a VM refusal which still
owns the same reserved work may advertise retry.

There is no `Reset` for a real run: external effects already visible to the
world cannot be undone. Restarting production execution is an explicit new
CLI run with its own authority.

The retained `B` timeline is nevertheless navigable during a real run. Selecting
an earlier boundary changes only the browser projection: the execution owner
continues at the live head, while the page reconstructs and shows the exact
retained journal prefix, instances, private state, mailboxes, Calls/Sessions and
CostReport captured at that boundary. It does not rerun a Driver, undo an effect
or move the live machine. Historical DATA/state mutation, retry, boundary advance
and resume are disabled; the global safe `Pause` and `Close` controls remain
available. `Return live` restores the current head. The header continues to show
the live execution state while the timeline labels the displayed frame
`HISTORY · B<n>`.

After completion or a fault the UI remains available until `Close` or process
termination, so the complete in-memory history stays inspectable.

## 5. Simulation controls

`simulate --ui` lists every checked simulation and scenario but activates one
scenario at a time. A fresh selection or `Reset` creates a fresh VM, journal,
mock world and virtual clock. The controls distinguish the two levels already
present in the Lab:

- `Step in` labels and advances the next
  available boundary, never both in one click;
- `Step over` explicitly applies one complete `inject`, `advance` or
  `expect`, and refuses while VM work remains;
- `Run to expect` stops immediately before the next `expect`;
- `Continue` preserves the ordinary CLI verdict and golden rules.

The page labels the next boundary as an action or transaction before it runs and
keeps the specialized controls available. An assertion never becomes a graph
transaction. Highlighting follows the exact committed edge or edge set from
one boundary to the next, but fan-out and every other atomic transaction stay
one boundary rather than being fragmented for animation.

Selecting a previous `B` first enters the same read-only historical view as a
real run; it does not silently mutate the current scenario. `Branch from here`
is the explicit reverse-debug operation. It creates a fresh scenario world,
replays the selected branch's recorded authored actions, VM
transactions and Driver polls through the selected boundary, then compares the
complete journal, VM inspection value, observations, logical time, Lab cursor,
quiescence and failures with the retained checkpoint. Divergence refuses and
leaves the current branch untouched. Exact agreement creates a new branch at
that boundary. The browser names branch zero `Main` and later branches
`Branch <n>`; their numeric identifiers remain protocol details. The source
branch remains immutable, and all later advances belong only to the new branch.
A compact branch selector keeps every
retained source and successor branch inspectable. Thus simulation can go back
and take a different path without truncating or rewriting any earlier journal.

Immediately before a next authored `inject`, the page exposes its pending
position as `before App.<port>`. It displays the future envelope zones as
`META` and `DATA`: `meta.ext` is the authored world context, `meta.app` is
empty, and `meta.sys` is explicitly pending because admission has not minted
it. This is a read-only projection of the checked scenario on disk, including
its resolved App input type and redacted value. The browser cannot replace the
action, DATA, META, port, sequence or provenance. The ordinary Lab and VM
admission remains the only path which mints the immutable envelope and journal
fact. To change an input, the author changes the `.rkg` source and starts a new
CLI session; source hot reload is not an M1 feature.

`Reset` and selecting another scenario likewise create a new branch with its
own `B0`; already retained branches remain in memory for the UI session.

The root `App.in` pin is topology, not an editor or alternate ingress. The
Instances view may derive a clickable history from already-retained root
envelopes and boundary frames; missing logical time is read from the existing
retained frame rather than invented.

## 6. Read model

The journal remains the only causal authority. The UI subscribes to it and to
the existing retained observation and `CostReport` queries. It may keep
disposable indexes or rendered slices, but a reconnect or refresh derives the
same view from the complete journal still held in memory. UI loss, a slow SSE
client or a closed browser cannot delay, fail or alter execution.

The execution owner synchronously retains one complete, already-redacted
projection for every `B` before publishing the replaceable newest-live update.
The browser may miss SSE notifications but cannot create a hole in the server's
in-memory timeline. A frame is keyed by `(branch, B)` and carries the causal record
prefix through `R`, VM inspection, observations, scenario position and
accounting projection for that boundary. These frames are disposable debugger
state, not another causal ledger; M1 neither persists them nor reconstructs an
external world's physical state. They are retained until the UI process exits.

The initial HTTP snapshot describes:

- the validated Graph/Node/edge topology as a compound graph: the root `App`
  contains its direct Nodes and nested Graph scopes, and each nested Graph
  recursively contains its own direct children. Compact Node cards expose only
  declaration name, effective NodeType and NodeAspects. A hollow square denotes
  an input port and a filled square denotes an output port; exact edges attach
  to those pins. Each routed Node input has one compact mailbox badge near its
  receiving pin containing
  only its current occupancy. Selecting it opens the exact current FIFO in the
  inspector; no cumulative history is disguised as queue state. Graph ports live in a
  side-constrained group on the corresponding Graph-frame side, including the
  root App input and output ports. Each pin may slide vertically and exchange
  display order with peers toward the median of its connected endpoints.
  Deterministic layer sweeps minimize crossings jointly across Node order and
  Graph-frame pins; an interval-track pass separates coincident routes and
  assigns distinct fan-in/fan-out approaches. A
  cross-hierarchy route traverses a nested Graph pin only when its retained
  port identity is unique; an ambiguous flattened seam is never guessed.
  The collision-reduced route remains the placement authority, while the
  rendered path is a smooth cubic spline through its required Graph portals,
  with horizontal port tangents and compact arrowheads. Distinct routes may
  cross but never share a visible curve segment. Geometry never becomes
  execution authority. Mailbox badges are placed toward the receiving pin on a sampled curve by
  a deterministic obstacle scorer: candidates are rejected when they intersect
  a Node, Graph header, edge, port label or earlier badge, with sixteen drawing
  units reserved before a qualified result label. Port
  names render once at their owning pin with an opaque text halo rather than
  being repeated on every edge. Only retained topology edges with an
  inspectable destination become pipes. An unwired Node output and an explicit
  `-> nothing` drain remain visible as local contract ports and gain no
  synthetic route, terminal cap, counter or root-level sink. The port tooltip
  distinguishes a declared drain from an ordinary output. Separately, a thin
  dotted control relation derived from each validated Graph-owned handler joins
  its activation port to the Graph output(s) that handler may emit. It carries
  no mailbox, route fact or envelope motion. `next` uses a temporal-control
  style; `expired` is connected from an explicit Loop-manager source. Its
  compact `on` marker and keyboard-selectable path open the owning Graph
  definition, so guarded choices stay inspectable without inventing spatial
  edges. The Graph/Node owner
  of the displayed boundary is highlighted. Selecting a box lightens only its
  body and border; port colors remain unchanged. An owner with live instances shows
  an `⧉ n` count. Selecting that
  control opens the filtered Instances panel; keys, addresses and generations
  never clutter the graph itself. Selecting a Graph frame or
  Node exposes its exact authored declaration, including ports, attributes,
  handlers and guard expressions;
- controller and execution state;
- current logical instances, lifecycle and Node-private `var` cells;
- retained causal observations with Call/Session correlation;
- the next authored input or latest observed envelope, with `META`
  (`sys`, `app`, `ext`) and `DATA` shown as distinct normative zones;
- the exact `CostReport`, including separate currencies and `unpriced` rows.

The embedded page presents this model as one debugging instrument rather than
independent cards. At the wide reference viewport its first row is one 32-pixel
transport strip, followed by one 28-pixel timeline strip. The scenario is
collapsed by default. The remaining height belongs to a compound-graph canvas
and an inspector with `Envelope`, `Mailbox`, `Definition`, `Instances` and
`History` tabs. A draggable splitter resizes graph and inspector. Calls,
Sessions, injection history and CostReport remain
available inside the Instances runtime tab; moving them does not merge their
authorities. The reference 1352×760 viewport must not require page-level
vertical scrolling to inspect the complete graph and either the current
envelope or at least ten compact causal rows.

The SVG initially fits the available canvas with a 24-pixel viewport margin.
Explicit zoom controls and wheel zoom change scale, while pointer dragging pans
the drawing without changing its geometry or execution selection. The root
Graph therefore grows by at least 140 drawing units for its result gutter and
by enough additional units for its output labels and their clearances; a nested
Graph never grows merely because its descendant is drained.

Presentation uses square, one-pixel-bounded surfaces, a single strong text
weight, a compact mono face for identities and transport facts, and the host
light/dark preference. Only Operator/Connector bodies and badges may have a
small radius. SVG fit scaling does not enlarge its text: Graph names, port
labels, counters and badges retain the same screen-space scale as the rest of
the interface. There are no decorative cards, shadows or pill controls. Amber
is reserved for historical/branch state; the active Boundary uses the green
execution accent.

SSE then announces monotonically increasing view revisions. Reconnection asks
for the current snapshot and timeline index; any retained frame remains
addressable by `(branch, B)`. No wall-clock timestamp or exporter duration is
invented. The browser initially renders bounded table/FIFO slices and can reveal
more on demand; every record retained by the in-memory journal remains
addressable for that run.

An edge mailbox badge reads `≡ n`, where `n` is a read-only inspection of actual `Delivery` values
currently retained in VM mailboxes, grouped by their canonical incoming edge;
a claimed Call head still occupies its mailbox, and consuming a delivery
decrements the value. Selecting the number exposes the exact FIFO order,
instance, sequence, redacted META and redacted DATA. The inspector initially
renders a bounded slice but can reveal every in-memory entry. Graph outputs and
`nothing` drains have no downstream mailbox and therefore receive no badge or
outgoing presentation pipe. A
scenario `inject` may add one queued delivery while correctly reporting that no
Node ran. Causal traversal history remains in journal/history views rather than
being conflated with present queue contents.

The displayed boundary also projects its exact admitted, consumed and emitted routes.
Those routes receive a short moving-envelope animation, while the firing Node
or lifecycle owner pulses. A boundary with no spatial route (for example an
assertion) pulses the boundary banner and never invents an envelope. A Call or
Session boundary fact names and pulses its Connector, and a small marker moves
outward for an opening/write or inward for a retained terminal outcome.
Animation is disposable presentation state, restarts from
the retained boundary projection after refresh, and is disabled under
`prefers-reduced-motion` without hiding the static highlight.

`Play` does not skip the pedagogical read model merely because the engine can
finish quickly. The server executes the ordinary transitions, while the browser
walks every retained `B` at either one or four events per second. `Continue` is
the explicit non-animated fast path. Buffered playback never alters scheduling,
logical time or journal facts. Logical time is rendered as fractional seconds,
then minutes and seconds (and hours when needed), while exact nanoseconds remain
available as machine-readable detail.

Source definitions are compiler debug side-table data, not plan data. The UI
may show the exact declaration span retained by the successful compilation,
but it never reverse-engineers source from the categorical plan and never
treats source text as execution authority. A deployment that has stripped
that physical side table still exposes topology and reports that source is
unavailable.

Containment is likewise read from the validated lexical owner table. The
browser does not infer nesting from dotted paths or edge geometry. Graph
frames are presentation of that existing authority and add no runtime scope.

Every causal-history row retains its generic observation event and also
projects the exact trigger/kind plus a human action summary. Thus `l:start`
renders as `Instance opened · <owner> · gen <n>`, while a port firing renders
as `Port transaction · <owner> · <port>`; neither is folded away or counted as another
delivery. Selecting a Node filters history, instances, exchanges and cost to
that exact path. Selecting a Graph filters the same panels to its lexical
subtree. Selection does not force an inspector-tab change; only explicit
mailbox and instance controls open their corresponding tabs. This filter is
disposable browser state; clearing it restores every row retained by the
in-memory authorities.

## 7. DATA, META and state presentation

M1 displays committed `DATA` content whenever it is already materialized,
whether its root is scalar, struct, map, array or union. The presentation keeps
the technical `kind` and `materialized` fields beside one `content` branch, so
plain text remains visibly text rather than becoming an opaque placeholder.
An unmaterialized encoded DATA value exposes only representation metadata and
known byte size; the UI never forces decoding or materialization merely for
presentation. A still-uncommitted simulation input uses its resolved type in a
read-only tree because the authored action already materialized it and no
causal envelope exists yet.

`META` is displayed separately. After admission it includes the exact
read-only `meta.sys` spine and the `app`/`ext` namespaces. Before admission it
shows only authored `meta.ext`, an empty `meta.app`, and a pending marker in
place of `meta.sys`; the UI never fabricates an id for a draft.

Node state remains structured-only. DATA and state presentation consult the
exact resolved annotation table before building their read model:

- `@secret` content is impossible on the executable source path and is never
  rendered for a forged package;
- `@sensitive` and `@personal` paths are redacted with no reveal control;
- unclassified DATA renders as a compact DOM tree rather than formatted JSON;
  structured state descendants retain their typed projection.

Redaction happens before serialization. String search or post-format masking
is not an acceptable implementation.

## 8. Identity and non-goals

The UI adds no package field, plan-package version, semantic hash extension or
causal record. Its port, capability, controller, browser connections,
presentation revision and physical timing are deliberately outside every
identity recipe.

M1.9 does not provide remote binding, TLS, user accounts, durable history,
cross-process attachment, source hot reload, breakpoints, state mutation,
ad-hoc envelope injection, editing authored actions, META or committed
envelopes, `File.watch`, exact commit timestamps or an OTLP exporter. Those are
separate decisions, not hidden approximations.

A rich terminal debugger is a named later projection over the same controller
commands and read model. Its CLI spelling and terminal interaction model remain
unsettled; M1.9 does not duplicate runtime semantics to provide it.
