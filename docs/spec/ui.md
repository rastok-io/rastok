# Visual simulation debugger

> **Status: projection of the [current format](identity.md), under D-2026-09-07-8, D-2026-09-08-1 and D-2026-09-10-1.**
> [R9 qualification](../architecture/rework-r9-qualification.md) records code
> and protocol evidence. Real-browser/keyboard qualification is tracked in
> [release qualification](../architecture/release-qualification.md).
> The [host-terminal ledger](../architecture/loop-host-terminal-v17.md) records
> the journal-derived Loop dial and command-disposition checks.

The debugger observes and controls the checked Lab simulation. It is not an
editor, a second runtime, a durable monitor or an alternative ingress.
`replay capture.rkrec --ui` additionally inspects a recorded run read-only
under D-2026-09-09-2; it cannot execute the embedded program.

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

For a diagnostic recording, `rastok replay capture.rkrec --ui [--ui-port PORT]`
opens at the empty prefix. Each subsequent transition selects one more recorded
record. Next transition/Finish navigate locally; Replay/Pause, Back and the
slider inspect the recorded timeline. Reset, retry, scenario actions and all
server execution commands refuse. The reader never constructs a physical
Driver. Checkpoint observation times are not exact record-commit timestamps.
The [recording contract](../architecture/diagnostic-recording.md) specifies
incomplete runs, privacy gates and the bounded snapshot history.
Tests: `recording::ui`, the UI protocol's read-only-command regression and
`browser.test.cjs`'s real-recording navigation/paused-playback case.

Every request, including assets, snapshots, timeline, SSE and controls,
requires the unpredictable per-process capability. The page removes it from
the address bar after loading. POST controls require the same origin and a
validated controller identity. One client owns control; observers may inspect
and explicitly take control. A stale controller cannot mutate the simulation.
Capability and ownership enter no graph, journal, diagnostic or accounting fact.

### Read-only Connector catalogue

`rastok connectors` and `rastok connectors --json` are CLI documentation views,
not debugger controls. They project the checked standard Contracts and modes,
resolved default port types and alternative type rules, attributes and phases,
terminal cases, linked implementations, Lab bindings and named conformance gaps.
They do not create a Driver, inspect a running deployment or execute a fixture.
No catalogue data is a new execution command or journal fact.

## 2. Stepping and playback

Every selection starts paused at B0, before admission. Controls are:

- **Next transition**: exactly the next authored action or ready Execution transition,
  not both (`advance one recorded transition`).
- **Next action**: one authored action to its effect, available when the
  previous action is quiescent (`run one authored action to its effect`).
  The status reports the number of recorded transitions advanced by this command.
  It reads `1 transition` for one, `n transitions` otherwise.
  Intermediate states are retained before the controller pauses at quiescence;
  executing an `advance` does not compress its ready transactions into one frame.
- **Run to expect**: stop before the next assertion.
- **Replay / Pause**: present the same recorded transitions at one or four events
  per second; Pause stops buffered playback even after the engine completes.
- **Finish**: run to completion without pedagogical pacing.
- **Retry**: available only when a refusal still owns retryable reserved work.
- **Reset**: fresh Execution, mock world, journal and clock for the selected scenario.
  Reloading the browser only reconnects to the recorded session; it is not Reset.

At B0 the status reads `Paused before admission · Next transition advances
one step`; the first navigation gesture dismisses it. There is no tour or modal.
Tooltips retain their action description and append a reason when disabled.

The server may run ahead while the browser reads recorded frames. Replay first
finishes the engine when necessary: `engine finishing, then replay`. The header
distinguishes engine state from buffered playback or historical inspection.
Pacing never changes logical time, scheduling or causal facts. Simulation time
advances through Lab actions; viewing a paused frame cannot age a timeout.
After completion or fault, history remains until Close or process termination.

A = authored action; B = recorded coherent transition, beginning at zero;
R = journal record index. These are different things. The interface spells out
`action · transition · record` and `LIVE · Main · transition 9`; B remains an
internal ordinal, not a public label. A transition may contain several
indivisible records. History shows `record 14 · transition 16` and the exact
record type, `INGRESS`, `TRANSACTION` or `EGRESS`. Transaction never means
an arbitrary debugger step. The word boundary is not used for time navigation.
Journal rows stay single-line; the table scrolls horizontally in a narrow
inspector instead of clipping record/transition positions. Summaries have full
tooltips and origin ids are abbreviated without repeating the column label.
Lifecycle facts remain stepable and labelled as lifecycle, not disguised as
application transactions or extra deliveries. Fan-out remains one atomic
transition. Replay, repeated Next transition and Finish retain the same execution.

A transaction refusal commits nothing. A physical boundary failure after a
previous commit must not claim rollback; its message says what remains
committed and whether retry is legal. An explicit pause requested while a Call
is open may retain its terminal boundary before pausing; a manually stepped
Call stays paused and later authored `advance` remains reachable.
An already paused simulation does not poll its scripted world spontaneously.
An in-progress logical-time advance can pause between its real transitions,
including while a scripted Call is open. Continuing resumes the same time
sweep; viewing the pause does not poll or advance it.
Discovering that no execution work is ready is not an extra user step: the same Step
request advances to the next authored action without publishing an empty frame.
Its status says `no execution work ready · advanced to A4` (using the actual action).

## 3. Read-only time navigation

Back, the transition slider and history rows select an immutable recorded frame.
They do not rerun a producer, change the live Execution, fork a journal or edit DATA.
Forward playback can revisit recorded frames. Return to live names the current
session/head. Mutating controls are disabled in history with a reason.

Reset and scenario selection increment a physical **session** ordinal and
select its B0 immediately. Old sessions remain inspectable. A session here
means one debugger scenario execution, not a Connector Session or a branch.
No branch command, authoring API or replay-to-mutate operation exists.

Every frame is retained under `(session, boundary)` before the replaceable
live notification is published. SSE is bounded and never blocks execution;
a missed notification creates no timeline hole. Reconnect obtains the current
index and can read every recorded frame. This complete in-memory history is
disposable debugger state, not another causal ledger or a durable backup.

## 4. Graph and inspector

Containment and actual ports come from the validated Node table, including
the App root and nested Graphs. Node bodies distinguish Graphs, Operators and
Connectors. The UI never infers execution from geometry.

- Graphs enclose children; Operator and Connector bodies are visually distinct.
  Titles show names, Contract and Extensions, without synthetic Node types.
- Hollow square pins are inputs; filled square pins are outputs. Graph pins
  belong to their real boundary. Disposed outputs remain local ports: neither
  `nothing` nor an unused output creates a root-level sink or a fake pipe.
  Clicking an App output pin, or pressing Enter while it is focused, inspects
  its latest recorded output. This references the original emission, not an
  extra observation or synthetic emission at the App boundary.
- Only physical edges are pipes. `Loop.next`, `Loop.again` and `Loop.done`
  are listed as manager commands in definitions and source-port details,
  never rendered as ports or back-edges. A command-only disposition labels
  its source `retry ⇢ Loop.next`. Exhaustion uses the host's resolved ordinary
  terminal; no `Loop.expired` pin or extra pipe exists.
- `≡ n` is current mailbox occupancy, not cumulative traffic. Selecting it
  opens the exact recorded FIFO with instance, sequence, META and DATA.
  A claimed Call head still occupies the mailbox until consumed. Large FIFOs
  render a bounded initial slice with access to all in-memory entries.
  Hover or keyboard focus previews the first five recorded envelopes with
  redacted DATA and abbreviated ids; it names the full occupancy and preview
  limit. The tab still exposes the complete in-memory FIFO in bounded slices.
- `⧉ n` counts currently opened instances, derived from lifecycle facts.
  Selecting it opens the filtered Instances tab; keys do not clutter the graph.
- Selecting a node/scope filters related panels without forcing Definition.
  Definition shows the exact authored source from the compiler debug table;
  scenario selection shows its DSL and highlights the action responsible for
  the selected transition (the first action before admission). Clicking
  the background clears selection. Replacing a graph frame preserves keyboard
  focus on the same surviving node, instance badge or mailbox.
- Loop runs appear in Instances: exact run, owner, application, remaining fuel,
  phase, pending command, due instant and current state-envelope reference.
  That reference is a dependency, not a second unclassified DATA projection.
- Retry operations appear separately in Instances : exact
  operation and instance, admitted attempt/bound, phase, original due time,
  and one row per admitted attempt. Call, admission, closure, production and
  consumption links point to their original records. Held results are labelled
  `held by Retry`, final ones `published`; both link to the common redacted
  envelope inspector. Prepared request/settings never acquire another DATA view.
  Selecting an instance filters both recurrence tables. `+ Retry` explains the
  checked policy without adding a Loop dial, command, port or error wrapper.

Each Loop badge carries a journal-derived `⟳ n/N` dial with explicit fuel N,
or `⟳ n` when the budget is derived from a measure. It counts admitted
applications, not Advance facts: `0` before the first Admit, `1` during the
first application, `2` only after the second Admit. Close or Expired resets the
dial to `0`. With multiple active runs, `⟳ n ·k` shows the highest admitted
ordinal and number of active runs. Selecting an instance filters those runs.
A pending delay adds `⏱ 1.0 s` (time remaining at the selected logical instant)
without incrementing the dial. Hover exposes run ref,
remaining/initial fuel, due instant and resolved exhaustion target. History
and replay use the selected journal prefix, never the live head. Instances
renders runs as a table, not raw JSON.

History displays r2 facts as `l:open`, `l:admit`, `l:advance`, `l:close` and
`l:expired`. These are presentation labels, not changes to journal triggers.
Definition shows each Extension once, including the root App.
Retry history separately names admission, holding, scheduling, final publication
and stopping. A held batch can highlight its consumed input, never an outgoing
public/discard route inferred from static topology. The future strip shows a
pending Retry deadline from the accepted fold, not a new predicted action.
History counts envelope observations, not deliveries: a held source emission
remains observable without ever reaching a downstream mailbox.
Live, historical and recorded views use the same selected-prefix projection.
Evidence: `retry_inspection_agrees_in_steps_batch_and_every_recorded_prefix`
and the browser regression `Retry keeps held attempts inspectable…`; execution
status is explicit in [C4](../architecture/retry-c4-inspection.md).

The page has a compact transport row, a timeline row, a graph canvas and
Envelope / Mailbox / Definition / Instances / History / Assertion / Compare tabs.
Tabs wrap without overlapping their panel. Instances includes
exchanges and injection history. The canvas pans/zooms;
a splitter resizes the inspector. Initial fit uses the available space.
Zoom percentages are relative to Fit graph (100%), not an absolute SVG scale.
Toolbar zoom anchors at the canvas centre; wheel zoom anchors at the pointer.
The label exposes those conventions in its tooltip. Injection history names
the instances in the admission's sealed routes, never a guessed DATA field.
Observed-envelope rows initially fold META and DATA; opening a cell retains
the same structured inspector and redaction, without a full-height tree per row.
Key columns reserve fourteen monospace characters so ordinary META names such
as `graph_version` do not split across lines at the default inspector width.
Graph text stays at interface text size. Automatic fit has a readability floor;
larger drawings remain pannable instead of shrinking their boxes below the space
reserved for text. Separate vertical scope bands reuse horizontal rank columns.
Node sizes reserve title, the complete Contract/Extension subtitle, and
instance-badge space before admission; opening an instance does not move the
graph. The layout and SVG renderer use the same subtitle, so `HttpClient · + Retry`
does not escape into a port label or pipe. Theme follows the system.

Ports slide and reorder to reduce crossings. Smooth cubic paths have horizontal
port tangents; deterministic lane/label placement separates overlapping paths.
Crossings may remain, but geometry must not invent topology. Layout tests cover
label/pin clearance and manager-command exclusion. Visual qualification includes
1352×760 and a narrow viewport; automated geometry checks do not replace it.

An exact route at a recorded transition highlights its pipe; lifecycle highlights
the owner. A transition without a spatial route pulses its label rather than
inventing an envelope. Reduced-motion preference removes animation, not the
static evidence. Logical time is readable in fractional seconds/minutes, with
the exact nanoseconds retained in the read model.

## 5. DATA, META and policy

The next source-authored injection is read-only. Its resolved type and
materialized DATA may be displayed before admission; `meta.sys` is pending,
not a fabricated id. To change it, edit the RKG on disk and restart the CLI.
The App input pin is topology, not an Inject button.
For a source-Connector fixture, the inspector names `listener.out`, not
`App.out`, and uses that source's resolved type and policy. Once admitted,
the root is inspected from INGRESS (`history.sources`), without pretending it
was another Execution delivery or emission transaction. Tests: the pending-injection
projection test and the HttpServer source case in `browser.test.cjs`.
If the plan contains classified data, the raw source DATA is withheld in both
views: a generic Contract pin may precede the first classified schema. This is
a conservative presentation policy, like scenario-source suppression, not
backward propagation of annotations or journal redaction. The regression
`a_source_copy_cannot_bypass_classification_further_inside_the_graph` covers
the pending and admitted copies.
The same conservative rule applies to mailbox and delivery views at a
`variant` pin, or one without a resolved schema, in a classified plan: an
untyped copy before validation is not evidence of public data. Typed pins
keep their field-level redaction. The webhook showcase regression inspects
every recorded transition, including the mailbox before its first typed
Operator, and refuses any visible email or physical credential.
The browser test `the webhook gallery remains classified in LIVE, HISTORY
and REPLAY without a measurement report` covers all six flagship scenarios with a synthetic
deployment profile, including the first admission, history and paused replay.
The browser receives no raw email or physical bearer through the snapshot;
the snapshot and page contain no measurement report.

Committed DATA may be scalar or structured. A tree separates technical
`kind`/`materialized` metadata from `content`. Encoded, unmaterialized DATA
stays encoded; viewing it does not trigger decoding. META is separate and
contains the exact sys/app/ext zones after admission.

Resolved policy is consulted **before** serialization and DOM construction:
classified sensitive/personal paths are redacted, with no reveal control.
Raw secret payloads remain impossible on the admitted execution path. State
uses the same policy boundary. No regex masking or string-built HTML is used.
D-2026-09-16-7 removes the CostReport pane and snapshot field. Original
measurement-bearing DATA remains inspectable under the same policy; no UI
aggregate has been decided. `view::tests::snapshots_do_not_compute_a_measurement_report`
checks the absence of a report without changing journal facts. The flagship
browser regression checks the same absence in LIVE, HISTORY and REPLAY.

Scenario source is a physical compiler side table, not a reconstruction from
values. Until source-level redaction exists, a program with classified ports,
state or owner bindings exposes no raw scenario text: an injection, mock or
assertion literal must not bypass DATA redaction through Definition. Unclassified
scenario source retains comments and original line numbers without changing
the plan or semantic hash.

Instance headings use the key decoded from the canonical address and generation,
for example `alice · gen 0`, never a guess from DATA. The full address requires
opening a chevron and is copiable. Envelope, origin and Loop-run ids show eight
characters with the exact value on hover and copy; META links use the same
rule. Classified key labels stay hidden. This is display folding, not a new id
recipe or permission to reveal a redacted value.

## 6. Explanatory projection

All evidence below is disposable and scoped to the selected snapshot and journal
prefix. Inspection does not execute a handler, call a Driver, decode opaque DATA,
append a record or modify the live machine.

- **Graph explanations.** Hover and keyboard focus show port direction, resolved
  type and declared dispositions (including case sets, manager commands and
  `nothing`); edge occupation, selected cases and capacity; node Kind, Contract,
  resolved exchange profile and open-instance count; Extension contributions;
  and the Loop run/application/fuel/due instant/resolved exhaustion terminal.
  Capacity is the actual shared instance-mailbox capacity, not an invented
  independent budget per pipe. Missing resolved type information says unavailable.
- **Decisions.** Selecting an Operator transaction in history and opening
  Definition highlights its chosen authored arm. The read model resolves consumed
  ids from journal routes/cursors, folds private state up to but **not including**
  this transaction, and reconstructs recorded META, logical time, immutable
  bindings, named join inputs and Loop state. The core's pure Dispatch evaluator
  selects the arm; the explanation shows only open-match predicates evaluated
  before and including the first true one. Closed arms use the actual typed tag.
  Compiler source spans are a debug side table, never part of plan identity.
  A chosen outer arm does not highlight unchosen nested arms.
  A case-routed pipe highlights the case of its selected/latest source envelope.
- **Causal links.** Envelope selection highlights the recorded origin lineage in
  pipes, nodes and journal rows. Origin and parent links navigate to recorded
  envelopes; system metadata without a DATA observation says so. Command links explicitly
  connect `CommandProduced`, `CommandConsumed`, and the resulting `Admit` through
  the r2 command reference and `(run, iteration)`, not by matching a convenient
  business key. A Done command has no Admit link. Nothing beyond the selected
  prefix is offered as already happened.
- **Scenario and timeline.** Action source lines are keyboard/click targets for
  their first recorded effect transition. Selecting a transition highlights its
  responsible action in the authored DSL. Unexecuted actions have no invented
  position. This uses the controller's existing action index, not source parsing
  in JavaScript.
- **Pending work.** The dotted Pending control beside the timeline head lists
  queued deliveries, their unsealed deadline facts, active lifetime/Closing
  deadlines, and Loop readmissions waiting on a recorded due instant. Call
  closures remove cancelled timeouts even while the claimed head awaits
  consumption. Times come from `deadlines`, never timeout-duration arithmetic
  performed at inspection time. The list is an inventory, not a predicted order
  of future transactions. A Session handshake already removed from its mailbox
  is not projected until its precise opening-delivery correlation is exposed;
  later message deadlines cannot masquerade as new handshake obligations.
- **Assertion evidence.** A failed expect automatically selects Assertion with
  the authored assertion, exact Lab target window, observed/matching counts and
  recorded envelopes. Direct field comparisons show observed/expected values;
  `expect nothing` shows its offenders. Evidence is re-derived by the existing
  assertion evaluator, not a second definition of success. Unsupported comparison
  shapes retain the assertion and envelope evidence, without an invented field
  diff. Unknown predicate evaluation is unknown, never zero matches.
- **Keyboard.** Left/right select transitions, `n` advances one transition, `a`
  one authored action, Space toggles Replay/Pause, Enter opens the selected
  envelope, Escape dismisses an overlay/selection, and `/` opens node/port/id
  search. Native inputs and tab controls keep their own keys. Focus is visible;
  redraw preserves focus on surviving graph targets. Escape restores an enabled
  focus target even at completion, so `/` works after leaving the timeline input.
- **Comparison.** Compare shows two keyed Loop instances at the selected
  transition, or two recorded simulation sessions at the same transition ordinal.
  The shared graph remains selected; two state columns show logical time, private
  state, application, remaining fuel, phase and due instant, with value differences
  highlighted. Missing transitions say not recorded. Alignment by ordinal is not
  an assertion of semantic equivalence and no equality is called a replay proof.

Classified programs suppress predicate values and assertion/source evidence
conservatively: these must not become side channels around DATA redaction. Opaque
inputs or an unavailable instance binding produce an explicit explanation limit;
no arm is guessed. Lifecycle decisions without an exact consumed context also
remain unavailable. The journal stays the sole semantic authority.

### Evidence for this presentation pass

Every item has a browser or projection regression; browser tests are in
`crates/rastok/src/ui/browser.test.cjs`, projection tests in `ui/view.rs` and
`ui/view/{explanation,expectation}.rs`.

| Brief item | Regression |
| --- | --- |
| Transports, transition vocabulary, first contact, action count | `transitions have explicit transport names…` |
| Recorded-completion snapshot field, with no old-name alias | `exchange_snapshots_name_recorded_completion_without_an_alias` |
| Folded ids across five examples and three viewing states | `echo/jsonl/gauge/countdown/keyed-retry: live, history and replay…` |
| Dial and delayed recurrences, stable selected-prefix values | `transitions have explicit transport names…`; `Loop dials follow historical admissions…`. The original B21–B22 positions predate fine-grained advance; tests now select the actual waiting folds rather than accidental ordinals |
| Every graph hover, mailbox head preview by pointer and keyboard without navigation, exact future timeout and cancellation | `explanatory hovers expose types…` |
| Readable compound layout at 1352×760 and narrow width; text/curve clearance; HISTORY clears zoom; four-digit mailbox badges | `the flagship keeps readable graph text…`; `vertically stacked scopes reuse horizontal ranks…`; `a four-digit mailbox count fits…` |
| Closed/open decisions, selected route case, prestate and joined inputs | `Operator decisions highlight…`; `open_match_explains_only_evaluated_predicates_using_prestate`; `join_decisions_use_named_inputs_and_the_retained_completing_port` |
| Causal links without forward leakage | `causal selection links opening admission…`; `decisions_use_consumed_envelopes_and_the_selected_prefix` |
| Scenario/action links, `a` and `n` on a fresh LIVE session, keyboard navigation | `scenario actions select their recorded transition…`; `the explanatory path is reachable with Tab and keyboard alone`; `keyboard node selection retains focus…` |
| Assertion diff, expect-nothing offender and redaction | `failed expect shows…`; `failure_explanation_uses_the_exact_assertion_window_and_field`; `classified_predicates_and_assertion_values_are_not_explanation_channels` |
| Aligned comparisons, separate read-only sessions | `comparison aligns two keyed Loop instances…` |
| Resident Process gateway, no child in simulation, correct scripted reply counts in LIVE/HISTORY/REPLAY | `the resident gateway explains independent Process Sessions…`; `resident_gateway_steps_deliver_opening_before_frames_like_the_batch_lab` compares the complete batch/step trace |
| Typed Responses values remain inspectable without the deployment bearer or a measurement report | `the Responses adapter preserves typed evidence…`; `responses_debugger_projects_each_transition_without_materializing_the_bearer` covers every scenario prefix |
| Complete Contract/Extension subtitles fit their node before and after admission | `Connector subtitles and Extension labels fit…`; `the flagship keeps readable graph text…` checks real SVG labels and pipes |
| Individually inspectable advance transactions, original App output, folded history cells | `an authored advance retains each transaction…`; Rust `advancing_time_keeps_each_ready_transaction_individually_inspectable` compares the complete batch/step result |
| Admission keys from recorded routes, including a misleading `tenant` field | `injection history names the recorded routing key…` |
| Toolbar zoom keeps the canvas centre fixed, Fit is 100%, splitter remains usable | `the complete tour stays inspectable by mouse and keyboard` |

The browser harness checks both uncaught exceptions and console errors on every
page. SVG sizing uses the CSSOM rather than a blocked inline style attribute;
the authenticated favicon request returns an empty success. The strict existing
Content Security Policy is unchanged, including the absence of `unsafe-inline`.

`keyed-retry` actually wires `model.exit -> decide`, not an edge `match`; its
Operator demonstrates TimeoutExpired/Exited arms. The separate
`tests/fixtures/ui-explanation.rkg` fixture demonstrates a closed case-routed
edge and intentionally failing assertions. The UI does not invent the edge
match named in the brief.

## 7. Identity and named limits

UI state adds no plan field, semantic contribution or causal record. Page
capabilities, controller identity, session/boundary counters, view revisions
and playback speed are physical.

The authored-`advance` gap from the [independent audit](../architecture/m1.9-fable-ux-audit.md)
is corrected at the controller/Lab seam. A caller-owned time-sweep cursor
retains real intermediate states; batch `Execution::advance_to` drives the same cursor
to completion. This is neither a new timer queue nor a journal authority.
Same-instant expiry ordering and settlement polling remain unchanged. The
regressions check both individual transaction visibility and identical final
canonical journals. The independent visual retest remains a separate gate.

Deferred: real-I/O experiments and harness Connectors, live-run debugging,
branch authoring, source hot reload, input/state editing, breakpoints,
cross-process attachment, durable UI history, remote access, user accounts,
exact commit timestamps and OTLP. A terminal debugger is a later consumer of
the same controller/read model, with spelling and interaction still undecided.
