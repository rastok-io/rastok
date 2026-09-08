# Visual simulation debugger

> **Status (2026-09-08): v17 projection under D-2026-09-07-8 and D-2026-09-08-1.**
> [R9 qualification](../architecture/rework-r9-qualification.md) records code
> and protocol evidence. The separate real-browser/keyboard gate remains in
> [release qualification](../architecture/release-qualification.md).
> The [host-terminal ledger](../architecture/loop-host-terminal-v17.md) records
> the journal-derived Loop dial and command-disposition checks.

The debugger observes and controls the checked Lab simulation. It is not an
editor, a second runtime, a durable monitor or an alternative ingress.

## 1. Capability and CLI

`hosted-ui` is an optional Cargo feature, omitted from reduced and embedded
images without changing program identity. The command is:

```text
rastok simulate app.rkg [lab.rkg] --ui [--ui-port PORT]
```

The port is nonzero, specified at most once and requires `--ui`; otherwise
the OS chooses a free loopback port. The server binds only `127.0.0.1`,
prints its capability-bearing URL and does not open a browser automatically.
The page is embedded HTML/CSS/JS: no frontend dependency, asset filesystem
lookup or build service. `run --ui` and `check --ui` refuse before effects.
A build without the capability refuses by name.

Every request, including assets, snapshots, timeline, SSE and controls,
requires the unpredictable per-process capability. The page removes it from
the address bar after loading. POST controls require the same origin and a
validated controller identity. One client owns control; observers may inspect
and explicitly take control. A stale controller cannot mutate the simulation.
Capability and ownership enter no graph, journal, diagnostic or accounting fact.

## 2. Stepping and playback

Every selection starts paused at B0, before admission. Controls are:

- **Step in**: exactly the next authored action or ready VM boundary, not both.
- **Step over**: one complete authored action, available at an action boundary.
- **Run to expect**: stop before the next assertion.
- **Play / Pause**: present the same retained boundaries at one or four events
  per second; Pause stops buffered playback even after the engine completes.
- **Continue**: finish without pedagogical pacing.
- **Retry**: available only when a refusal still owns retryable reserved work.
- **Reset**: fresh VM, mock world, journal and clock for the selected scenario.

The server may run ahead while the browser reads retained frames. The header
distinguishes engine state from buffered playback or historical inspection.
Pacing never changes logical time, scheduling or causal facts. Simulation time
advances through Lab actions; viewing a paused frame cannot age a timeout.
After completion or fault, history remains until Close or process termination.

A = authored action; B = retained atomic boundary, beginning at zero;
R = causal record index. These are different things. A boundary may contain
several indivisible records. History shows both R and its containing B.
Lifecycle facts remain stepable and labelled as lifecycle, not disguised as
application transactions or extra deliveries. Fan-out remains one atomic
boundary. Play, repeated Step in and Continue retain the same execution.

A transaction refusal commits nothing. A physical boundary failure after a
previous commit must not claim rollback; its message says what remains
committed and whether retry is legal. An explicit pause requested while a Call
is open may retain its terminal boundary before pausing; a manually stepped
Call stays paused and later authored `advance` remains reachable.
An already paused simulation does not poll its scripted world spontaneously.
Discovering that no VM work is ready is not an extra user step: the same Step
request advances to the next authored action without publishing an empty frame.

## 3. Read-only time navigation

Back, the boundary slider and history rows select an immutable retained frame.
They do not rerun a producer, change the live VM, fork a journal or edit DATA.
Forward playback can revisit retained frames. Return to live names the current
session/head. Mutating controls are disabled in history with a reason.

Reset and scenario selection increment a physical **session** ordinal and
select its B0 immediately. Old sessions remain inspectable. A session here
means one debugger scenario execution, not a Connector Session or a branch.
No branch command, authoring API or replay-to-mutate operation exists.

Every frame is retained under `(session, boundary)` before the replaceable
live notification is published. SSE is bounded and never blocks execution;
a missed notification creates no timeline hole. Reconnect obtains the current
index and can read every retained frame. This complete in-memory history is
disposable debugger state, not another causal ledger or a durable backup.

## 4. Graph and inspector

Containment comes from the validated owner table. Executable nodes and actual
boundary ports come from the plan. The UI never infers execution from geometry.

- Graphs enclose children; Operator and Connector bodies are visually distinct.
  Titles show names, Contract and Extensions, without synthetic Node types.
- Hollow square pins are inputs; filled square pins are outputs. Graph pins
  belong to their real boundary. Disposed outputs remain local ports: neither
  `nothing` nor an unused output creates a root-level sink or a fake pipe.
- Only physical edges are pipes. `Loop.next`, `Loop.again` and `Loop.done`
  are listed as manager commands in definitions and source-port details,
  never rendered as ports or back-edges. A command-only disposition labels
  its source `retry ⇢ Loop.next`. Exhaustion uses the host's resolved ordinary
  terminal; no `Loop.expired` pin or extra pipe exists.
- `≡ n` is current mailbox occupancy, not cumulative traffic. Selecting it
  opens the exact retained FIFO with instance, sequence, META and DATA.
  A claimed Call head still occupies the mailbox until consumed. Large FIFOs
  render a bounded initial slice with access to all in-memory entries.
- `⧉ n` counts currently opened instances, derived from lifecycle facts.
  Selecting it opens the filtered Instances tab; keys do not clutter the graph.
- Selecting a node/scope filters related panels without forcing Definition.
  Definition shows the exact authored source from the compiler debug table;
  scenario selection shows its DSL and highlights the next action. Clicking
  the background clears selection. Replacing a graph frame preserves keyboard
  focus on the same surviving node, instance badge or mailbox.
- Loop runs appear in Instances: exact run, owner, application, remaining fuel,
  phase, pending command, due instant and current state-envelope reference.
  That reference is a dependency, not a second unclassified DATA projection.

Each Loop badge carries a journal-derived `⟳ n` dial. It counts admitted
applications, not Advance facts: `0` before the first Admit, `1` during the
first application, `2` only after the second Admit. Close or Expired resets the
dial to `0`. With multiple active runs, `⟳ n ·k` shows the highest admitted
ordinal and number of active runs. Selecting an instance filters those runs.
An armed delay adds `⏱` without incrementing the dial. Hover exposes run ref,
remaining/initial fuel, due instant and resolved exhaustion target. History
and replay use the selected journal prefix, never the live head. Instances
renders runs as a table, not raw JSON.

History displays r2 facts as `l:open`, `l:admit`, `l:advance`, `l:close` and
`l:expired`. These are presentation labels, not changes to journal triggers.
Definition shows each Extension once, including the root App.

The page has a compact transport row, a timeline row, a graph canvas and
Envelope / Mailbox / Definition / Instances / History tabs. Instances includes
exchanges, injection history and the public CostReport. The canvas pans/zooms;
a splitter resizes the inspector. Initial fit uses the available space.
Graph text stays at interface text size. Theme follows the system.

Ports slide and reorder to reduce crossings. Smooth cubic paths have horizontal
port tangents; deterministic lane/label placement separates overlapping paths.
Crossings may remain, but geometry must not invent topology. Layout tests cover
label/pin clearance and manager-command exclusion. Visual qualification includes
1352×760 and a narrow viewport; automated geometry checks do not replace it.

An exact route at a retained boundary highlights its pipe; lifecycle highlights
the owner. A boundary without a spatial route pulses its label rather than
inventing an envelope. Reduced-motion preference removes animation, not the
static evidence. Logical time is readable in fractional seconds/minutes, with
the exact nanoseconds retained in the read model.

## 5. DATA, META and policy

The next source-authored injection is read-only. Its resolved type and
materialized DATA may be displayed before admission; `meta.sys` is pending,
not a fabricated id. To change it, edit the RKG on disk and restart the CLI.
The App input pin is topology, not an Inject button.

Committed DATA may be scalar or structured. A tree separates technical
`kind`/`materialized` metadata from `content`. Encoded, unmaterialized DATA
stays encoded; viewing it does not trigger decoding. META is separate and
contains the exact sys/app/ext zones after admission.

Resolved policy is consulted **before** serialization and DOM construction:
classified sensitive/personal paths are redacted, with no reveal control.
Raw secret payloads remain impossible on the admitted execution path. State
uses the same policy boundary. No regex masking, string-built HTML or second
accounting fold: CostReport comes from its existing public journal query.

Scenario source is a physical compiler side table, not a reconstruction from
values. Until source-level redaction exists, a program with classified ports,
state or owner bindings exposes no raw scenario text: an injection, mock or
assertion literal must not bypass DATA redaction through Definition. Unclassified
scenario source retains comments and original line numbers without changing
the plan or semantic hash.

## 6. Identity and named limits

UI state adds no plan field, semantic contribution or causal record. Page
capabilities, controller identity, session/boundary counters, view revisions
and playback speed are physical.

Known qualification gap: an authored `advance` currently invokes `vm.advance`
as one action and can retain several VM transactions/lifecycle facts inside
one B frame. This does not yet satisfy the individually stepable VM/lifecycle
intent of section 2. Equal Step/Continue journals do not close that gap;
intermediate frames must be retained by the controller/Lab seam, never invented
by the renderer. See the [independent audit](../architecture/m1.9-fable-ux-audit.md).

Deferred: real-I/O experiments and harness Connectors, live-run debugging,
branch authoring, source hot reload, input/state editing, breakpoints,
cross-process attachment, durable UI history, remote access, user accounts,
exact commit timestamps and OTLP. A terminal debugger is a later consumer of
the same controller/read model, with spelling and interaction still undecided.
