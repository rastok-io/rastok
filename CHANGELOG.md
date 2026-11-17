# Changelog

## Versions

While the version carries the `-alpha` suffix:

- the third number changes for a corrective release. Every program the
  previous release accepted is still accepted with the same meaning,
  apart from the cases listed in the changelog, and no format changes
  its number;
- the second number changes when the language changes in a way that
  breaks programs, when the package format, the recording format or a
  standard Contract changes its number, or when a milestone opens a new
  capability;
- the first number stays at zero until stability is promised.

The package format, the recording format and each standard Contract
carry their own number. Every changelog entry states them.

## v0.1.1-alpha (unreleased)

Corrective release prepared for publication. The package format remains 1,
the diagnostic recording format remains 1, and all six standard Connector
Contracts remain at `@1`; no terminal case is added or renamed.

### Compatibility exceptions

These programs are now refused at check instead of failing during execution:

- a Call mock whose statically known terminal contradicts its Contract's
  frame requirements (`E-CONTRACT-REQUIRED`); computed terminals retain
  their runtime checks;
- a route to an Operator input without a single-port or join handler
  (`E-HANDLER-MISSING`). A bare Node name selects `out` as a source and
  `in` as a destination, never another port selected from its declarations;
- a typed request without a JSON form sent to a JSON codec
  (`E-TYPE-INVALID`), including a tagged union inside a resident Process
  frame. `variant` payloads retain each Contract's runtime checks and
  refusals; requests with a text-valued field retain their behavior;
- an authored WebSocket Session mock that the Lab cannot execute
  (`E-CHECK-DEFERRED`). WebSocket declarations without mocks and resident
  Process scripts remain admitted;
- an emission to a precise type that still relies on a state cell's
  earlier narrowing after assigning that cell (`E-PORT-TYPE`). Subsequent
  reads use the cell's declared type, including when it remains optional.

Two additional check-time refusals are explicit:

- a bare enum symbol that is also a visible binding is ambiguous
  (`E-EXPR`), rather than silently selecting either meaning;
- compiler-added conversions count in the unchanged expression depth
  budget. An expression at the limit that reads a narrowed value can now
  be refused with `E-PLAN-DEPTH`; excessive resolved depth never reports
  `E-INTERNAL-PLAN`.

### Lab and response timing

- Draw every debugger link with horizontal and vertical tracks and rounded
  corners. Clear spans and detours now share the same style, without changing
  node placement, ports or semantic routes.
- Place long links and return links with the port-constrained layered method,
  including nested Graphs, before allocating their routing tracks. The
  debugger keeps semantic endpoints, containment and mailbox identities while
  avoiding the previous per-link search through candidate detours.
- Reuse debugger node placement and edge routes until the validated topology
  changes. Mailbox badges reserve four digits before admission, so stepping
  and selection do not recalculate or move the drawing.
- Do not count responses already drained by a logical-time advance as new
  debugger progress. Step reaches the next inspectable change without an
  empty click; batch results and reference traces remain unchanged.
- Settle ready Call responses before advancing to the next logical deadline,
  in both simulation and hosted execution. Immediate mock replies no longer
  time out when one advance crosses several deadlines.
- Admit ready resident Process Session mock replies at the logical instant
  of the input they answer, before the next deadline. Downstream handlers
  observe the same `now()` with one large advance or several smaller advances,
  in batch simulation and debugger stepping.
- Keep `now()` in each Retry mock at that attempt's opening timestamp,
  including the scenario's civil origin, regardless of where advances stop.
- Explain that Lab expectations on a Connector's `exit` see every Retry
  attempt terminal, while downstream ports receive only the final batch.
- Explain why mocked Connectors still need synthetic secret bindings, and
  name a missing deployment environment variable separately from a version
  mismatch, without exposing secret material.

A scenario that passed only because a mock reply arrived at the end of a
large `advance` may now fail. Reference traces changed: **none**.

### Language and expressions

- Resolve enum symbols inside optional list and map literals, including
  nested collections, fields, state and Lab values. The optional collection
  supplies its element context without selecting a branch of a wider union.
- Construct optional map values using their declared type even when the map
  itself is optional. Ports, fields, bindings, state and Lab values no longer
  pass check only to fault on `map<text,int?>?` at execution.
- Read fields, nested fields, indexes and `has` through the selected branch
  of a lazily loaded input, including a local copy, just as for a state cell.
  Carrying positions remain lazy, type tests keep their branch witnesses,
  and semantic program hashes and reference traces are unchanged.
- Test optional tagged-union presence with its type name and select its
  cases with `is`: present cases match their union, null does not, and
  case payloads remain checked. Existing program hashes are unchanged.
- Compare optional input payloads to `null` using the selected value, as
  for optional fields and state cells. Payloads loaded during a comparison
  also apply this rule recursively to nested values; retained branch
  witnesses and semantic program hashes are unchanged.
- Accept the existing empty map spelling `{:}` inside expression arguments
  and nested collections; `size({:})` evaluates to zero.
- Materialize values narrowed by positive `is` tests in conjunctions,
  conditionals, guarded handlers and guarded match arms, so execution yields
  the value whose type was checked. Assigning a state cell ends its narrowing
  facts for subsequent statements. The inserted conversion changes the semantic
  hash of a program that reads a narrowed value; existing causal reference traces
  remain comparable.
- Resolve bare enum symbols from their expected type or the opposite enum
  operand of an equality test; ambiguous symbols refuse with `E-EXPR`.
- Propagate the expected enum element type into list literals, including
  nested lists and Lab expectations, without making symbols visible names.
- Resolve bare enum symbols in optional enum positions, including state,
  ports, collection values, comparisons and Lab values. Visible-name conflicts
  and unknown symbols still refuse; wider unions do not choose an enum branch.
- Check nonempty list literals element by element in typed positions,
  including mixed null and text elements for `array<text?>`. Unconstrained
  mixed lists retain `variant` elements and their existing representation.
- Typed collection literals with optional elements or values change semantic
  hashes, even when the program already passed: lists since element-by-element
  checking, maps since typed construction in 0.1.1. Their meaning is unchanged;
  published example hashes and reference traces are unchanged.
- Type empty arrays and maps from their declared destination, including
  nested fields, state, mocks and Lab assertions. Without an expected type,
  empty collections retain `variant` elements, as in 0.1.0. A later typed
  destination refuses a more precise requirement with `E-PORT-TYPE` and
  suggests an annotation such as `val members: array<text> = []`.
  `{}` remains a struct; a map mismatch points to `{:}`. An empty map keeps
  its value and declared type through package and journal decoding regardless
  of its key type, using the unchanged encoding.
- Expose case payload fields inside a single-case closed match while keeping
  the complete tagged value for direct and identity emissions.
- Resolve successive positive type tests using the preceding facts. Reused
  inferred structs also construct optional fields required at their typed
  destination, instead of failing when execution checks the value.
- Preserve typed handler inputs when an expression also reads mutable state,
  without weakening the independent check of transported expressions.
- Allow exact integer-to-decimal conversions, subject to the declared
  precision and scale; decimal arithmetic remains unavailable.
- Accept inline struct values inside arrays at emission, mock, injection
  and expectation sites; a ternary between inline structs remains refused.
- End a bare `stop` at its line boundary so the next match arm is not parsed
  as its value.
- Evaluate floating `%` expressions already accepted by the checker,
  including signed zero, infinities and NaN; decimal arithmetic and mixed
  numeric families remain unavailable.

### Diagnostics

- Keep collection type advice applicable to inline values and bindings,
  without assuming an empty literal or an existing binding.
- Name deferred temporal type parameters, calendar intervals, fixed-length
  bytes and fixed-length arrays with `E-PARSE-DEFERRED`. The Reference
  distinguishes logical model forms from source declarations, and text-key
  map literals from the wider logical key domain; no syntax is added.
- Render expression operators and types in source notation in refusals,
  including optional collection elements, instead of Rust debug structures.
  Index and `size` refusals quote the rejected type without an article.
  Codes and diagnostic field names are unchanged; list concatenation is
  still unavailable.
- Warn with `W-JOIN-UNPAIRED` when a join can strand Call frames or terminals;
  pairing by Call identity with an explicit bound avoids the warning.
- Name conflicting declared types and their first structural difference in
  route diagnostics; anonymous types use bounded descriptions with the same
  text in the `expected` and `found` fields.
- Locate invalid Loop verdicts at the authored arm and explain missing or
  competing verdicts, including outputs routed to `nothing`.
- Explain the tests needed before optional or tagged-union member access,
  and suggest a local binding for a field read on a computed value.
- Explain final-match placement, case tests and expression line boundaries
  without accepting new syntax. Delayed mock actions now report the existing
  `E-PARSE-DEFERRED` code; reserved route words name their deferred purpose.
- Mark the unused `E-DUP-EDGE` diagnostic as superseded by the existing
  `E-ROUTE-DUPLICATE` refusal.

### Command line, installation and editors

- Report missing source files as command-specific usage errors before
  compilation or deployment loading, and provide `--help` / `-h` for each
  command on stdout with a successful exit status.
- Render non-text App output values with the Lab's policy-safe JSON
  projection in `run` and `replay`, preserving protected-field masking,
  text output and port labels. Explain the existing text-only stdin
  submission; input decoding and EOF behavior are unchanged.
- Retrieve examples at the installed release tag in the quickstart, and
  verify and extract downloaded archives in a system temporary directory
  without adding files to the caller's working directory.
- Package the data-only editor grammar as `rastok-dsl-0.1.1.vsix` for the
  release artifacts; install through the editor rather than manual directory
  copying. JetBrains still imports the TextMate bundle.
- Add a non-blocking native Windows build and test job to gather evidence;
  this does not qualify or publish a Windows binary.
- Document when `run` exits after stdin ends, why a listening Connector keeps
  it alive, and why bounded structured logs are not a complete journal.
- Add native Vim and Neovim file detection, syntax highlighting and
  buffer-local comment settings, with installation instructions and native
  tests. No language server or personal configuration changes are included.
- Add an Emacs major mode for `.rkg` highlighting and comment commands,
  with native tests for source boundaries and edits to reader delimiters.

### Documentation and dependencies

- Retire the published approval example's keyed instances after 48 hours
  of inactivity, including requests whose join expired. Its semantic hash
  changes with that lifetime; no reference trace changes.
- Correct the state reference: later statements in a handler read an
  assignment immediately; the transaction commits its changes together,
  and a fault retains the previous state.
- Document the outbound Call timeout's queued time separately from inbound
  HTTP backpressure and Sessions; serialization is per Connector instance,
  not a limit of one Call across the execution.
- Present the DSL's numeric names and their fixed representations in one
  reference table, separate from the logical widths used by schemas. State
  which operations and exact conversions each family supports.
- Distinguish `.rkg` programs from the runtime's implementation language,
  and Operators from Connectors before the example sequence.
- Add a reading path to the normative grammar and replace the Lab reference's
  placeholder block with a complete program checked and simulated by a test.
- Update only `chacha20` from withdrawn version 0.10.1 to 0.10.2.
- Reconcile the opening ledgers with published binaries and crates, hosted
  CI, contribution settings and documentation reachability; keep unperformed
  manual trials explicitly open.

### Compatibility evidence and remaining qualification

All eight reference-trace comparisons pass without rewriting a trace.
The published examples retain their check results: fifteen accepted, two
already refused by the previous release. The only published example with
a changed semantic hash is `docs/examples/expense-approval.rkg`, because
its keyed instances now have the idle lifetime described above.

Publication is pending. The embedded Retry/recovery workload now passes the
unchanged 1 MiB code-size and 128 KiB heap gates. Temporary-set construction
in plan validation saves 16,200 B of linked code, leaving 2,516 B of code
and 1,804 B of heap margin. This reduction does not change execution, recovery,
formats or reference traces; the [qualification](docs/architecture/release-qualification.md#embedded-code-size-correction-16-november-2026)
records the fixed workloads and reproduction commands.

## v0.1.0-alpha (5 November 2026)

First public alpha, under the Apache 2.0 license. The package format is 1,
the diagnostic recording format is 1, and the six standard Connector
Contracts are at `@1` (D-2026-11-03-4); the representations are the ones
qualified in October under the former internal numbers, and every reference
trace, test golden and package witness was regenerated from source and
compared record by record. The
[identity inventory](docs/architecture/time-carriers-identity.md) records the
relative logical timeline and the wide civil carriers. The alpha scope stated
on the site's status block is the release claim; nothing else is.

Since the October review:

- The debugger shows the program source as its main view, with a gutter
  marker on the selected transition, identifier values on hover and keyboard
  focus (read only, redaction kept), the graph as a synchronized complementary
  view, a Play control with playback speed, and a guided tour page. JetBrains Mono is
  embedded under the SIL Open Font License for the source pane; the
  Definition inspector tab is removed.
- Lab scenarios are written directly inside `lab`, without string labels;
  `expect target.port` means exactly one observation, with `expect N`,
  `== nothing`, `== value` and `where` forms; `simulation <name> { }` groups
  shared mocks; expectations no longer consume the observation window.
- Rastok artifacts use one filename family: `.rkg` source, `.rkp` plan
  package, `.rkc` causal reference trace, `.rkr` diagnostic recording.
- A failed scenario is explained from its source line and the recorded
  evidence; `run` and `replay` print the bare value for single-output
  applications; the debugger URL is a terminal link only; reference traces
  live beside their examples under `examples/traces/`.
- Standard Contract exit types (`FileExit`, `ProcessExit`, `HttpClientExit`,
  `HttpServerExit`, `WebSocketExit`) resolve without source declarations
  (D-2026-11-03-3).
- The documentation gains a nine-chapter guide, eight example chapters that
  form one learning path (countdown and HTTP model adapter added), one
  Reference page per Connector plus the execution and standard library pages,
  an Overview that opens with the product positioning, and a CLI that answers
  every information request.
- The README, the whitepaper and the release prose follow the October public
  narrative, without named product comparisons.

From the October qualification:

- `worker::Supervision` supplies the Worker's input, time and checkpoint services;
  `Host` retains its clustered-machine meaning, without changing execution.
- `LogicalTime` and signed `Duration` distinguish execution instants from civil
  timestamps. Journal and recording headers retain the immutable civil origin;
  Lab scenarios apply `origin` and `now()` observes it; tumbling boxes retain
  their epoch alignment. Wide civil values no longer share the i64-nanosecond date bound.
- Typed Graph scopes, Operator/Connector execution, explicit destinations,
  case routing and atomic multi-emission; exact-instance closure commands
  introduced in v18 are kept.
- Finite temporal Loop managers with exact causal run contexts and retained
  commands; exhaustion extends the host's failure terminal, never adds a port.
- Native outgoing-Call Retry, separate from Loop: the same prepared request,
  terminal-case policy, retained logical backoff, bounded whole-attempt
  disposition and exact-prefix recovery. Only the final attempt is published.
- Deterministic Lab scenarios, record-level golden trace comparison and a read-only
  simulation debugger with mailbox, instance, Retry and causal inspection.
- Explicit simulation reference modes: `--compare <dir>` reads existing
  golden traces without changing them; `--save <dir>` creates or replaces them.
  The modes are mutually exclusive, and neither supplies the Lab responses.
- Process, bounded File and hosted HTTP/WebSocket Contracts; secret bindings,
  data-policy annotations and individual measurement inspection. M1 has no
  measurement report, automatic pricing or subset arithmetic.
- Flat annotation types, author-chosen measurement ids and explicit units;
  signed integer/decimal samples and monetary refunds retain original values.
  `@cost` and the implicit-unit registry refuse with migration diagnostics.
- Typed HttpClient JSON responses, strict by default, with an explicit
  projected profile for nullable omissions and extra response members.
  Mistyped present values still refuse; missing usage is never invented as zero.
- Resident Process Sessions, exact-child shutdown and a native JSONL
  WebSocket agent bridge. Both finite jobs and resident services share
  `std/process@6`; their modes retain distinct Call and Session lifecycles.
- Unit-global and lexical `val` constants; pure `has(object.member)` tests
  optional JSON members without conflating absence, null and observed zero.
- Independently inspectable logical-time advances, original App-output
  inspection, compact envelope history and routing-derived instance labels.
- Recorded runs with `run --record` and their diagnostic `replay [--ui]`,
  without repeated external effects; `rastok new` for an executable one-file
  starting point.
- Public Markdown content contract, six-chapter learning path, captures/GIF
  and editor highlighting. The Responses HTTP adapter has six Lab scenarios
  and local wire/browser qualification; no paid provider call is claimed.

Known limitations: the deployment journal is in memory; diagnostic files do
not provide durable restart or qualified personal-data retention. No Kafka,
SQL, Saga, debugger branch authoring, `run --ui` or public-response HTTP mode
is included in the opening showcase. HTTP/3-server ngtcp2, official Autobahn
and Connector implementation-cost measurements remain open, non-blocking
qualifications. See the [opening ledger](docs/architecture/opening-showcase.md)
for remaining blocking release work; no completion is implied by this draft.
