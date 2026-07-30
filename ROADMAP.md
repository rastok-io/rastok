# Rastok roadmap

This roadmap is organized by evidence gates, not by a speculative
sequence of product releases.

## Now — M0 mechanical proof

M0 answers one question: can the central technical loop work end to
end?

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
