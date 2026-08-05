# Rastok roadmap

This roadmap is organized by evidence gates, not by a speculative
sequence of product releases.

## Done — M0 mechanical proof (exited 2026-08-04)

M0 answered one question: can the central technical loop work end to
end? It can — the evidence is recorded in
[`docs/architecture/m0-exit-report.md`](docs/architecture/m0-exit-report.md).

- Parse and check the smallest useful `.rkg` subset.
- Lower it to an executable plan.
- Execute it in a one-worker in-memory VM.
- Run one scenario with a mock, virtual time, and a golden trace.
- Cross one real `Process` boundary.
- Produce a useful diff at the first divergence.
- Reproduce the result on two machines.

Out of scope: a complete connector catalogue, live Saga or Loop
managers, durable deployment, enterprise security, UI, registry, and
cluster operation.

## Now — M0.5 engine consolidation

Consolidate what M0 proved before widening it
([`docs/architecture/m0.5-slice.md`](docs/architecture/m0.5-slice.md)):

- Specify the delivery reference and derive the call identity from
  it — the multiport prerequisite, spec-first.
- Split the no_std-ready core (`rastok-core`) from the std layer,
  with CI guarding the frontier; prove neutrality by the unchanged
  semantic hash.
- Seal the executable plan as a versioned, verifiable blob.
- Time-boxed embedded spike against a written go/no-go.
- A connector conformance harness over the `Partner` seam — a new
  adapter in half a day.
- First LLM-authorship benchmark wave: generate → check → retry →
  simulate, measured.

## Next — incident lab and safe dogfood

- Turn one real internal failure into a redacted fixture and scenario.
- Run in a trusted, non-critical environment with a kill switch.
- Measure installation time, diagnosis time, cost per session, and
  maintenance effort.
- Test the same proof with selected design partners.

## Then — transferability

- A third party installs Rastok from the documentation.
- That party changes or writes a graph.
- A third-party incident becomes a replayable regression scenario.
- At least one partner uses the proof in its own technical evaluation.

Only evidence from these gates determines the next product scope.
