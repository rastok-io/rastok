# Changelog

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
