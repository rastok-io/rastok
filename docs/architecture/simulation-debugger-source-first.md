# Source-first simulation debugger

Status: specified, not shipped. Qualification is recorded in the external
`LOT-2026-11-02-debugger.md` work ledger.

## Motivation and invariant

A flow is a source file, versioned and reviewed with its scenarios. The debugger
will open that text as its primary view. Navigation and values will project the
selected recorded transition. Execution, the controller, plan identity and the
recording format will remain unchanged. No projection will execute a handler,
edit a value, author a branch or add a command.

## Views and physical state

The center pane will contain one tab per source unit and a secondary Graph tab.
The source pane will show line numbers, syntax coloring and a current-transition
marker. The existing inspector will remain on the right with Envelope, Mailbox,
Definition, Instances, History, Assertion and Compare. Graph will keep the
existing layout engine, ports, iteration dial and interaction. Selecting a graph
node will select its definition in source and its inspector context; the current
source marker will identify that node in Graph. The transport, future details
and fault panel will retain their behavior. Tab selection, scroll, pan, zoom and
the inspector splitter will remain physical UI state.

## Source and position model

A simulation snapshot will expose each source unit once, with source index,
display name, text and line count. The compiler will retain the original text
beside the checked plan; source names will come from the loading boundary.
Per-node `definition` fields will remain compatible. Classified programs will
withhold Lab source, including shared mocks, before serialization; line numbers
will remain stable. Original files will never be reconstructed from values.

A current position will have an explicit available or unavailable status,
source index, inclusive first/last line, column when retained, node path, optional
port and selected decision arm. Lines and columns will be one-based; columns
will count Unicode scalar values. Missing columns will be null. The existing
compiler lexer will locate identifier tokens in retained source. Scope will be
the checked enclosing node, with explicit instance references in value summaries.
Missing or ambiguous dynamic context will have an unavailable reason.

Recordings currently retain node excerpts only. Replay will use those excerpts
in the same pane, explicitly mark missing lines and report incomplete source.
It will not read a possibly changed file from disk or alter the recording format.
A future recording-format proposal would be needed to retain complete units.

## Stepping and marker

A step will continue to mean one recorded transition. The marker will land on
the transition's node, port or selected arm, using existing activity and decision
evidence. Scenario actions will use their retained action lines. A synthetic,
system or mock transition without an authored location will explicitly say that
source is unavailable. A redacted decision will fall back to its node or port,
without pretending to know its arm. Historical snapshots will use their own
retained prefix. The transport will keep Pause, Next transition, Finish, Reset
and the iteration dial unchanged. Scrolling will be confined to the source pane;
reduced motion will disable animated scrolling.

## Read-only hover values

Positioned, resolved identifiers will offer summaries for ports, state fields,
Loop/Retry counters, scenario inputs, DATA/META paths and pending deadlines.
A summary will have a stable kind, short rendering, redacted flag and inspector
link. It will name absent values and ambiguous contexts explicitly. Port values
will refer to the pending or latest recorded envelope in the selected prefix;
state will come from that snapshot's instance view, and counters and deadlines
from existing journal-derived views. DATA/META paths will traverse only existing
policy-filtered JSON. No hover will decode opaque payloads or expose prepared
request/settings. Classified scenario source will remain hidden.

Hover and keyboard focus will open the same tooltip. Escape will dismiss it;
its link will select the inspector tab containing the full recorded object.
The source and tooltip will contain no editable fields, inputs or forms.

## Limits and adapter follow-up

Breakpoints, value editing, source hot reload, branch authoring, a terminal
projection and a Debug Adapter Protocol adapter are out of scope. No POST route
or command will be added. A later adapter could reuse source indexes, positions,
node/instance scopes, variable summaries and the existing stepping controller.
It would still need a decision on variable-reference lifetimes, unavailable
source and ambiguous instance presentation, column encoding, adapter transport
and read-only capability negotiation. It must not equate an authored action,
recorded transition and journal record. These follow-ups are specified, not shipped.
