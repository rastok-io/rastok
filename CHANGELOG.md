# Changelog

## Unreleased — preparation for v0.1.0-alpha

This is not a released tag. Public release and crate publication await
15 October 2026 and Yoann's explicit instruction.

- Typed Graph scopes, Operator/Connector execution, explicit destinations,
  case routing and atomic multi-emission on the frozen v17 plan.
- Finite temporal Loop managers with exact causal run contexts and retained
  commands; exhaustion extends the host's failure terminal, never adds a port.
- Deterministic Lab scenarios, record-level golden comparison and a read-only
  simulation debugger with mailbox, instance, cost and causal inspection.
- Process, bounded File and hosted HTTP/WebSocket Contracts; secret bindings,
  data-policy annotations and journal-derived accounting.
- Diagnostic `run --record` and `replay [--ui]`, without repeated external
  effects; `rastok new` for an executable one-file starting point.
- Public Markdown content contract, initial gallery and editor highlighting.

Known limitations: the deployment journal is in memory; diagnostic files do
not provide durable restart or qualified personal-data retention. No Kafka,
SQL, Saga, debugger branch authoring, `run --ui` or public-response HTTP mode
is included in the opening showcase. HTTP/3-server ngtcp2, official Autobahn
and Connector implementation-cost measurements remain open, non-blocking
qualifications. See the [opening ledger](docs/architecture/opening-showcase.md)
for remaining blocking release work; no completion is implied by this draft.
