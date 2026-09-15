# C2 — source and validated-plan implementation

15 September 2026. **In progress; Retry is not executable.** The
[C1 inventory](retry-c1-identity.md) owns the proposed v19/v3 cutover.
Preparatory changes below preserve executable v18 and recording v2; no
Contract major, journal tag or golden has moved.

## 1. Exact Session opening authority

Session direction is now a core Contract fact, used by both live execution
and prefix recovery. It is not inferred from `effect`, attribute names or
duplex roles. The internal closed query returns Peer for WebSocketServer,
Local for WebSocketClient and Process service, and no authority for an unknown
Contract or inconsistent roles/exchange. No transported field or public trait
is added.

A peer cannot open a client or resident process. An uncorrelated server write
is consumed without opening a client or guessing the sole live peer. Recovery
uses the same rule for openings and writes and does not re-offer old I/O.
The existing `effect` attribute/transport remains until the single cutover;
removing its Session-direction use is a prerequisite, not that cutover.

Evidence at this checkpoint:

- the core identity/roles/exchange matrix passes (four selected tests);
- twelve `machine_vm` Session tests pass, including new peer-root refusal
  before journaling and orphan-write parity with/without recovery;
- the five focused Contract, gauge, JSONL, resident Process and exchange
  recovery suites pass (101 tests, no blessing enabled);
- tests compare retained records, not only an in-memory resource count.

Full workspace, no_std, memory and witness checks are recorded after the
preparatory batch; the tests above do not qualify Retry scheduling/accounting.

## 2. Remaining C2 gates

- Canonical sets for response-producing terminal cases; HTTP ServerError
  becomes executable only with its declared Contract/type cutover.
- Source Retry settings and plan-owned idempotence; `W-LOOP-REPEAT` and its
  positive/negative/sibling/canonical-order tests.
- Remove global effect and Intent.effect without duplicating the guarantee.
- Carry all C1 packet, reference, closure-time and recording changes in the
  single cutover, with independent hostile validation.
- Keep named execution refusals for unfinished C3 paths; no check-green
  construct that every runtime must reject.

Runtime attempt publication, shared recurrence/recovery and accounting are
C3. Example/golden/UI migration and final qualification remain C4.
