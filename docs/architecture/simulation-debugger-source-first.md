# Source-first simulation debugger

Status: shipped, with the recording and resolution limits below.
Qualification is recorded in the external
`LOT-2026-11-02-debugger.md` work ledger.

## Motivation and invariant

A flow is a source file, versioned and reviewed with its scenarios. The debugger
opens that text as its primary view. Navigation and values project the
selected recorded transition. Execution, the controller, plan identity and the
recording format are unchanged. The projection does not execute a handler,
edit a value, author a branch or add a command.

## Views and physical state

The left pane contains one tab per source unit. The right side contains the
inspector above the complementary Graph pane. The columns start at equal widths
and the right panes at equal heights.
The source pane shows line numbers, syntax coloring and a current-transition
marker. The inspector remains on the right with Envelope, Mailbox,
Instances, History, Assertion and Compare. Graph keeps the
existing layout engine, ports, iteration dial and interaction. Selecting a graph
node scrolls to its definition in the source tab and selects its inspector context.
The current source marker identifies that node in Graph. The transport,
future details and fault panel retain their behavior.
Source tab selection, scroll, pan, zoom and both splitter proportions remain
physical UI state. Pointer dragging and axis-aligned arrows resize the panes;
Home/End reach their size limits. Stored proportions survive reload and viewport
changes. Narrow screens stack source above the right panes within the viewport,
with both dividers resizing heights and a separate stored source proportion.

Fit graph centers the complete drawing in its canvas. Text scales with that
overview; Readable size restores interface-sized text and panning. This replaces
the former automatic readability floor. The Graph title shares its activity row.
Command status shares the source header with the current position. Replay and
Pause share one visible control, and Retry transaction appears only with a
retryable fault. Scenario changes take effect from the selector; its progress
button selects the scenario source. The history-session selector appears when
more than one session exists.

Selecting a graph node reveals and highlights its source block without switching
inspector tabs. Instances retains decision evidence, node information and manager
commands in a collapsible section for the selected node. There is no Definition
tab; source text appears once, in the primary pane. Recorded scenario actions
are selectable there without switching inspector tabs. A history record selects
the same source definition and graph node. Instance sections show counts and
start folded when empty; a user's fold persists while a section remains populated.

Source typography uses embedded JetBrains Mono regular and semibold faces with
ligatures. Same-origin font routes retain capability authorization and the
existing content security policy. The font license is included in the binary
and available through a static route. No installed font or external service is
required. The source string and Unicode columns are unchanged. Multiline brace
regions come from retained lexer tokens, excluding string and comment contents. A region
crossing unavailable recording lines has no folding control. A closing line
shared with another statement stays visible. Nested folds persist per source
within the tab, separately from the execution prefix. Transition and definition
navigation reveal their containing regions. A manually folded current position
has a marker on the folded header; its description and source status retain the
actual line. Enter and Space activate gutter controls without starting Replay.

## Source and position model

A simulation snapshot exposes each source unit once, with source index,
display name, text and line count. The compiler retains the original text
beside the checked plan; source names come from the loading boundary.
Per-node `definition` fields remain compatible. Classified programs
withhold Lab source, including shared mocks, before serialization; line numbers
remain stable. Original files are never reconstructed from values.

The existing scenario-definition projection also supplies declaration locations
for source-visible scenarios: source index, inclusive line range, simulation
name and scenario name. The compiler's retained declarations supply these
fields without adding plan fields. The existing classified-source guard
withholds both scenario text and this location list.

A current position has an explicit available or unavailable status,
source index, inclusive first/last line, column when retained, node path, optional
port and selected decision arm. Lines and columns are one-based; columns
count Unicode scalar values. Decision arms retain line ranges only; missing
columns are null. The existing compiler lexer locates identifier tokens in
retained source. Scopes follow checked enclosing nodes; the inspector retains
recorded instance references. Missing or ambiguous dynamic context has an
unavailable summary. Identical retained declarations on the same line do not
acquire a guessed scope.

Recordings retain node excerpts only. Replay uses those excerpts
in the same pane, explicitly marks missing lines and reports incomplete source.
It does not read a possibly changed file from disk or alter the recording format.
A future recording-format proposal would be needed to retain complete units.

## Stepping and marker

A step continues to mean one recorded transition. The marker lands on
the transition's node, port or selected arm, using existing activity and decision
evidence. Scenario actions use their retained action lines when policy permits.
A synthetic, system or mock transition without an authored location explicitly
says that source is unavailable. A redacted decision falls back to its node or
port, without pretending to know its arm. Historical snapshots use their own
retained prefix. The transport keeps Pause, Next transition, Finish, Reset
and the iteration dial unchanged. Scrolling is confined to the source pane;
reduced motion disables animated scrolling.

A green scenario gutter triangle composes existing commands. For an inactive
scenario it sends select-scenario, observes the matching fresh session at its
initial boundary, and sends one step. For the active scenario it sends one step.
The pending intent disables execution controls and repeated gutter activation.
Historical navigation, a disconnected controller, a refused selection or a
different observed session cancels the pending step. Historical inspection,
read-only recording playback and observer mode cannot use the gutter to execute
work. The ordinary selector remains available when scenario source is withheld.

F8 toggles Replay/Pause; F10 advances one recorded transition. These are aliases
for the existing controls, with Space and `n` retained when function keys are
reserved by the browser or keyboard. Inputs, composition and modified function
keys keep native behavior. The toolbar and accessible shortcut metadata expose
the bindings. No function-call stepping model is implied.

## Read-only hover values

Positioned, resolved identifiers offer summaries for ports, state fields,
Loop/Retry counters, scenario inputs, DATA/META paths and pending deadlines.
A summary has a stable kind, short rendering, redacted flag and inspector
link. It names absent values and ambiguous contexts explicitly. Port values
refer to the pending or latest recorded envelope in the selected prefix;
state comes from that snapshot's instance view, and counters and deadlines
from existing journal-derived views. DATA/META paths traverse only existing
policy-filtered JSON. No hover decodes opaque payloads or exposes prepared
request/settings. Classified scenario source remains hidden. Port summaries
show DATA and application META, with transport identifiers available through
the inspector link. Unsupported identifiers and ambiguous instance selection
have unavailable values in the read model; the projection does not evaluate
arbitrary expressions. Only resolved inspection symbols enter the source's Tab
order or open tooltips. Known runtime symbols with absent values remain
inspectable. Available values have a dotted underline. Counter summaries use
labeled text; complete objects in the existing bounded summary use the DATA
tree. The UI does not request an additional value payload. The shared comment
recognizer colors lexer trivia without mistaking string contents for comments;
annotations and primitive types have their own syntax color.

Hover and keyboard focus open the same tooltip. Escape dismisses it;
its link selects the inspector tab containing the full recorded object.
Tab traverses a structured summary and its inspector link, then continues through
source and into the other panes. Shift+Tab returns through the tooltip to its
identifier.
The source and tooltip contain no editable fields, inputs or forms.

## Limits and adapter follow-up

Breakpoints, value editing, source hot reload, branch authoring, a terminal
projection and a Debug Adapter Protocol adapter are out of scope. No POST route
or command was added. A later adapter could reuse source indexes, positions,
node/instance scopes, variable summaries and the existing stepping controller.
It would still need a decision on variable-reference lifetimes, unavailable
source and ambiguous instance presentation, column encoding, adapter transport
and read-only capability negotiation. It must not equate an authored action,
recorded transition and journal record. These follow-ups are specified, not shipped.
