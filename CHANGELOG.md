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

## v0.1.1-alpha (18 November 2026)

A corrective release. It fixes what the first users of 0.1.0-alpha ran
into, moves several failures from execution to `rastok check`, and makes
the first hour easier: installation, help, editors and the debugger.

A program accepted by 0.1.0-alpha keeps its meaning, apart from the cases
listed under "Programs that `check` now refuses". Some golden traces saved
with 0.1.0-alpha must be saved again. The package format
remains 1, the diagnostic recording format remains 1, and all six standard
Connector Contracts remain at `@1`; no terminal case is added or renamed.

### In short

- **Errors come earlier.** Programs that passed `check` and then failed
  while running are now refused by `check`, with a message that says what
  to change.
- **The debugger keeps up with large graphs.** The graph is drawn once and
  reused; a step no longer redraws it. Links run at right angles, cross
  far less, and meet at the same level where they share a pin.
- **Lab time is consistent.** A mock that answers at once is seen at the
  instant of its request, whether a scenario advances time in one step or
  in several.
- **The language accepts what its types already allow.** Enum values where
  the type is known, empty lists and maps, optional values after a test.
- **Messages speak the language you write.** Types and operators appear in
  source notation, and more refusals say what to do next.
- **Installing is cleaner.** Each command has its own help, the examples
  come at the tag of your binary, and syntax highlighting installs in
  VS Code, Vim, Neovim and Emacs.

### Programs that `check` now refuses

Each of these passed `check` in 0.1.0-alpha and then failed while running.
`check` now refuses it and says why:

- **A mock that cannot be a valid answer.** A mock that ends a Call as
  completed without the answer its Contract requires is refused with
  `E-CONTRACT-REQUIRED`. When the way a mock ends is only known while
  running, it is still checked while running.
- **A route into an input that no handler reads.** An Operator handles
  `storage` only, and the program writes `in -> annunciator`. A bare Node
  name now always means its `out` port as a source and its `in` port as a
  destination, never another port picked from its declarations. The route
  is refused with `E-HANDLER-MISSING`, and the message suggests
  `annunciator.storage`. The same applies to an input read by no handler,
  alone or in a join.
- **A value that JSON cannot carry.** A request whose type has no JSON
  form, for example a struct with a timestamp field, sent to a Connector
  that speaks JSON, never left. It is refused with `E-TYPE-INVALID`, and
  the message gives the path of the field. This includes a tagged union
  inside a frame of a resident Process. A payload typed `variant` keeps
  each Contract's checks and refusals while running, and a request with a
  text-valued field behaves as before.
- **A WebSocket Session mock.** The Lab cannot play one yet, so its answer
  was never delivered. It is refused with `E-CHECK-DEFERRED`. A WebSocket
  declaration without a mock, and scripts of a resident Process, are still
  accepted.
- **A value used after its test was overwritten.** A handler tests a state
  cell, `id is text`, assigns that cell, then emits it where a `text` is
  required: the test no longer holds. This is refused with `E-PORT-TYPE`.
  After the assignment, reads use the cell's declared type again, including
  when it stays optional.

Two more refusals are new:

- **An enum symbol that is also a name in scope** is refused with `E-EXPR`,
  instead of silently meaning one or the other.
- **A very deep expression.** The conversions the compiler adds after an
  `is` test count in the depth limit, which is unchanged. An expression at
  that limit can now be refused with `E-PLAN-DEPTH`; it never surfaces as
  the internal error `E-INTERNAL-PLAN`.

### Golden traces saved with 0.1.0-alpha

Scenarios and their expectations pass as before. Two kinds of saved traces
no longer compare equal with `simulate --compare`:

- **A program that reads a value after an `is` test and uses Retry.** The
  compiler now adds a conversion where the value is read. This changes the
  semantic hash that `check` prints, and every Retry record carries that
  hash. The first divergence names a `retry_protocol` record whose expected
  and found values are the old and the new hash.
- **A scenario where one `advance` crosses several deadlines of a mocked
  Connector.** Each answer is now recorded at the instant of its request,
  so the records come in another order. The first divergence names the
  `kind` of a record.

When the expectations pass and the divergences are of these two kinds,
save the traces again with `simulate --save`. The eight published
reference traces are unchanged.

### Debugger

- **The graph is drawn once.** Node placement and link routes are computed
  when the program opens and reused until its structure changes. Stepping,
  selecting and counters no longer recompute or move the drawing; queue
  counters reserve four digits from the start.
- **Links are easier to follow.** Every link runs along horizontal and
  vertical segments with rounded corners. Long links and return links are
  placed before their tracks are allocated, nested Graphs included, which
  leaves far fewer crossings on the published examples. Node placement
  differs from 0.1.0; ports and routes do not change.
- **Links that share a pin meet at its height**, without a small step
  beside the pin. Their vertical segments stay distinct.
- **No link runs through a Node.** Tracks stay clear of every Node of the
  neighbouring column, including wider Nodes and Nodes with no link there.
- **A queue counter keeps its place.** A link between two neighbouring
  columns keeps room for its counter between the two Nodes.
- **Column gaps follow what they carry.** Nested frames no longer repeat
  the App's outer margins. Text and Node sizes are unchanged.
- **Step always shows something.** A response already consumed by a time
  advance no longer counts as progress, so no click is empty. Batch
  results and reference traces are unchanged.
- **The source follows the current line**, including in optimized builds,
  where a second render used to stop the scroll. Scrolling by hand
  afterwards stays under your control. CI now runs the browser suite on
  both build profiles.
- **Readable size follows the current Node.** In the graph's Readable size
  view, the Node of the current transition is centered at the click and
  after each move along the timeline. Panning by hand holds until the next
  move; Fit graph is unchanged.

### Lab and time

- **A mock that answers at once is on time.** In 0.1.0, inside one
  `advance`, a mock answered only at the end of the advance: a Call could
  time out although its mock replied immediately. A ready answer is now
  delivered before time moves to the next deadline, in simulation and in
  hosted execution.
- **The same holds for a resident Process Session mock.** Its reply
  arrives at the instant of the input it answers. Handlers downstream read
  the same `now()` whether the scenario advances once or in several steps,
  in batch simulation and when stepping in the debugger.
- **`now()` inside a Retry mock** is the time at which that attempt
  started, the scenario's civil origin included, wherever the scenario's
  advances stop.
- **`expect` on a Connector's `exit` with Retry.** The documentation now
  says that it sees the end of every attempt, while the ports downstream
  receive only the final batch.
- **Secrets with a mocked Connector.** The Lab still asks for a synthetic
  secret binding when the Connector is mocked, and the documentation says
  why. A missing deployment environment variable has its own message,
  apart from a version mismatch; neither exposes secret material.

A scenario that passed only because a mock reply arrived at the end of a
large `advance` may now fail. A trace saved for a scenario whose advance
crosses several deadlines must be saved again, as described above. The
published reference traces changed: **none**.

### Language and expressions

Enum values:

- **You can write an enum value.** In 0.1.0, `status = Cancelled` was
  refused. A bare symbol now takes its type from the place it is written:
  a port, a field, a binding, a state cell, a Lab value, or the other side
  of an equality test. It also works where the enum is optional, inside
  lists, nested lists and Lab expectations included, and inside list and
  map literals written to an optional collection. A symbol that no enum
  declares, or that is also a name in scope, is still refused. A place
  that accepts several types does not pick the enum for you. In an
  expression, an enum value cannot yet be joined to a text or converted
  to one, and a text cannot be converted to an enum value. On a JSON
  line, an enum value travels as its name.

Lists and maps:

- **Empty lists and maps.** `[]` and `{:}` take the type of the place they
  go to: a port, a nested field, a state cell, a mock, a Lab assertion.
  Written with no type around, they hold `variant` elements, as in 0.1.0;
  sending that value later to a more precise type is refused with
  `E-PORT-TYPE`, and the message suggests an annotation such as
  `val members: array<text> = []`. `{}` is still the empty struct; where a
  map was expected, the message points to `{:}`. An empty map keeps its
  value and its declared type when a package or a journal is read back,
  whatever its key type; the encoding is unchanged.
- **`{:}` works as a function argument and inside another collection**;
  `size({:})` is zero.
- **A list is checked item by item** against the type it goes to, so a
  list that mixes null and text fits `array<text?>`. A list with no type
  around keeps `variant` elements and its representation.
- **A map with optional values** is built with its declared type even
  when the map itself is optional. At a port, a field, a binding, a state
  cell or a Lab value, a `map<text,int?>?` no longer passes `check` and
  then fails while running.
- **A struct written in place inside a list** is accepted where a value is
  emitted, mocked, injected or expected. A ternary between two such
  structs is still refused.
- **Semantic hashes.** The hash that `check` prints changes for a program
  whose typed list or map literals hold optional items or values, even if
  that program already ran. Its meaning does not change. The hashes of
  the published examples and the reference traces are unchanged.

Values that may be absent, and tagged unions:

- **After an `is` test, you get the tested value.** In 0.1.0,
  `id is text => out = { id = id }` passed `check` and failed while
  running, because `id` was still optional. The value now has the tested
  type inside the branch: in a conjunction, a conditional, a guarded
  handler or a guarded match arm. A second test builds on the first.
  Assigning a state cell ends the effect for the statements that follow.
  A program that reads such a value gets a new semantic hash; its saved
  traces compare equal unless it uses Retry, as described above.
- **Reading inside an input after its test.** In 0.1.0, a field of an
  optional or union input, read after its `is` test, passed `check` and
  failed while running. Fields, nested fields, indexes and `has` now read
  the tested value, also through a local copy, as they already did for a
  state cell. An input typed `variant` is not covered. Hashes and
  reference traces are unchanged.
- **Comparing an optional input to `null` gives the right answer.** In
  0.1.0 the comparison was false even when the input was null. The rule
  also applies to the values nested inside, as it does for optional fields
  and state cells. Hashes are unchanged.
- **An optional tagged union** is tested for presence with its type name,
  and for a case with `is`. A present case matches its union, null does
  not, and case payloads stay checked. Hashes are unchanged.
- **Inside a closed match with a single case, the fields of that case are
  readable.** Emitting the value itself, directly or unchanged, still
  sends the whole tagged value.
- **A struct inferred once and reused** gets the optional fields its typed
  destination requires, instead of failing when the value is checked
  while running.
- **A handler's input keeps its type** when the same expression also reads
  a state cell that can change. The separate check of the expressions a
  program carries is as strict as before.

Numbers and syntax:

- **An integer converts to a decimal** when it fits the declared precision
  and scale. Decimal arithmetic is still unavailable.
- **`%` on floating numbers evaluates** wherever `check` already accepted
  it, signed zero, infinities and NaN included. Mixing numeric families is
  still unavailable.
- **`=> stop` followed by another arm now parses**: a bare `stop` ends at
  its line, so the next match arm is not read as its value.

### Messages

- **Types and operators are printed the way you write them**,
  `array<text?>` and not an internal structure, optional collection
  elements included. Codes and diagnostic field names are unchanged.
- **A type conflict on a route names both declared types and their first
  structural difference.** A type without a name gets a bounded
  description, the same in the `expected` and `found` fields.
- **Reading a value that may be absent.** The message says which test is
  needed first, for an optional value and for a tagged union, and
  suggests a local binding for a field read on a computed value.
- **Loop verdicts.** A wrong verdict is located at the arm you wrote. A
  missing verdict and two competing verdicts are explained, outputs routed
  to `nothing` included.
- **`match` and line ends.** Messages explain where a final `match` may
  stand, how a case is tested, and where an expression ends. No syntax is
  added.
- **A join that can lose its pairs.** `check` warns with `W-JOIN-UNPAIRED`
  when a join can strand the frames or the end of a Call. Pairing by the
  Call's identity, with an explicit bound, avoids the warning.
- **Forms that are not available yet say so** with `E-PARSE-DEFERRED`:
  temporal type parameters, calendar intervals, fixed-length bytes and
  fixed-length arrays, delayed mock actions; reserved route words name
  their deferred purpose. The Reference tells the forms of the logical
  model from what a source file can declare, and text-key map literals
  from the wider logical key domain. No syntax is added.
- **Smaller fixes.** Advice on collection types applies to inline values
  and to bindings, without assuming an empty literal or an existing
  binding. Index and `size` refusals quote the rejected type. List
  concatenation is still unavailable. The unused `E-DUP-EDGE` is marked as
  superseded by the existing `E-ROUTE-DUPLICATE`.

### Command line, installation and editors

- **Every command has `--help` / `-h`**, printed on stdout with a
  successful exit status. A missing source file is reported as a usage
  error of that command, before anything is compiled or loaded.
- **`run` and `replay` print values you can read.** A struct or a number
  on an App output is printed as JSON, the same projection as the Lab,
  with protected fields masked. That projection stays explicit for an
  optional value, a decimal and a tagged union: each is printed as an
  object that names its `kind`. Text output and port labels are unchanged,
  and so are input decoding and end-of-input behavior.
- **The guide explains how `run` ends**: when it exits after stdin ends,
  why a listening Connector keeps it alive, that stdin submission is
  text-only, and why bounded structured logs are not a complete journal.
- **The quickstart leaves nothing behind.** Archives are verified and
  extracted in a system temporary directory, and the examples are fetched
  at the tag of the installed release.
- **Syntax highlighting installs as a package**, `rastok-dsl-0.1.1.vsix`
  for VS Code, from the release assets, instead of copying a directory by
  hand. JetBrains still imports the TextMate bundle.
- **Vim, Neovim and Emacs** get native file detection, highlighting and
  comment settings, with installation instructions and native tests. No
  language server or personal configuration change is included.
- **Windows** is built and tested natively in CI, without blocking, to
  gather evidence. This does not qualify or publish a Windows binary.

### Documentation and dependencies

- The published approval example retires its keyed instances after 48
  hours of inactivity, requests whose join expired included. Its semantic
  hash changes with that lifetime; no reference trace changes.
- The state reference is corrected: later statements in a handler read an
  assignment at once; the transaction commits its changes together, and a
  fault keeps the previous state.
- The outbound Call timeout's queued time is documented apart from inbound
  HTTP backpressure and Sessions: serialization is per Connector instance,
  not a limit of one Call across the execution.
- One reference table presents the DSL's numeric names and their fixed
  representations, apart from the logical widths used by schemas, with the
  operations and exact conversions each family supports.
- The pages tell `.rkg` programs from the runtime's implementation
  language, and Operators from Connectors, before the example sequence.
- The normative grammar gets a reading path, and the Lab reference's
  placeholder block becomes a complete program checked and simulated by a
  test.
- `chacha20` moves from the withdrawn 0.10.1 to 0.10.2; no other
  dependency changes.
- The opening ledgers are reconciled with the published binaries and
  crates, hosted CI, contribution settings and documentation reachability;
  manual trials not yet performed stay explicitly open.

### Compatibility evidence

All eight reference-trace comparisons pass without rewriting a trace.
The published examples keep their check results: fifteen accepted, two
already refused by the previous release. One published example changes its
semantic hash, `docs/examples/expense-approval.rkg`, because its keyed
instances now have the idle lifetime described above.

The embedded Retry/recovery workload passes the unchanged 1 MiB code-size
and 128 KiB heap gates. Temporary-set construction in plan validation
saves 16,200 B of linked code. After the later expression fixes, the
published revision keeps about 1.5 kB of code margin and 1,804 B of heap
margin. This reduction does not change execution, recovery, formats or
reference traces; the [qualification](docs/architecture/release-qualification.md#embedded-code-size-correction-16-november-2026)
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
