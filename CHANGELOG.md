# Changelog

## Unreleased — preparation for v0.1.0-alpha

This is not a released tag. Public release and crate publication await
5 November 2026 and Yoann's explicit instruction.

The current checkout uses **format 23 / diagnostic recording 4**, to be
labelled public format 1 at publication. The
[identity inventory](docs/architecture/lifecycle-registry-identity.md) records
the `l:loop` and `l:close` triggers, their causal identity changes and verification.
External opening gates remain open; these capabilities are not a release claim.

- Typed Graph scopes, Operator/Connector execution, explicit destinations,
  case routing and atomic multi-emission; exact-instance closure commands
  introduced in v18 are kept.
- Finite temporal Loop managers with exact causal run contexts and retained
  commands; exhaustion extends the host's failure terminal, never adds a port.
- Native outgoing-Call Retry, separate from Loop: the same prepared request,
  terminal-case policy, retained logical backoff, bounded whole-attempt
  disposition and exact-prefix recovery. Only the final attempt is published.
- Deterministic Lab scenarios, record-level golden comparison and a read-only
  simulation debugger with mailbox, instance, Retry and causal inspection.
- Explicit simulation reference modes: `--compare <dir>` reads existing
  traces without changing them; `--save <dir>` creates or replaces them.
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
- Diagnostic `run --record` and `replay [--ui]`, without repeated external
  effects; `rastok new` for an executable one-file starting point.
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
