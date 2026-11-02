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

The center pane contains one tab per source unit and a secondary Graph tab.
The source pane shows line numbers, syntax coloring and a current-transition
marker. The inspector remains on the right with Envelope, Mailbox,
Definition, Instances, History, Assertion and Compare. Graph keeps the
existing layout engine, ports, iteration dial and interaction. Selecting a graph
node selects its definition for the source tab and its inspector context;
the chosen view stays open. The current source marker identifies that node in
Graph. The transport, future details and fault panel retain their behavior.
Tab selection, scroll, pan, zoom and the inspector splitter remain physical
UI state.

## Source and position model

A simulation snapshot exposes each source unit once, with source index,
display name, text and line count. The compiler retains the original text
beside the checked plan; source names come from the loading boundary.
Per-node `definition` fields remain compatible. Classified programs
withhold Lab source, including shared mocks, before serialization; line numbers
remain stable. Original files are never reconstructed from values.

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
have unavailable values; the projection does not evaluate arbitrary expressions.

Hover and keyboard focus open the same tooltip. Escape dismisses it;
its link selects the inspector tab containing the full recorded object.
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
