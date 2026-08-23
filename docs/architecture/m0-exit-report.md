# M0 exit report

> Evidence record — non-normative. M0 ran 2026-07-31 → 2026-08-04.
> Scope: [`m0-slice.md`](m0-slice.md); build sequence:
> [`build-order.md`](build-order.md); assumed deviations:
> [`m0-exit-notes.md`](m0-exit-notes.md).

## Verdict

M0 exits under its own rule — the chain is continuous and testable.
One `.rkg` unit parses, checks, and lowers to a hashed plan; the plan
executes on a journaled, transactional VM; a real child process
answers across the Process boundary; the same graph runs against its
scripted lab; the journal projects to a canonical sealed trace whose
first divergence is named, not dumped. Every stage of the build order
exited through its tests, and the final state was reproduced from a
clean checkout on a second machine and operating system.

## The evidence, by claim

| Claim | Evidence |
|---|---|
| The compiler refuses wrong programs with stable diagnostics | registry-driven codes grown change-by-change ([diagnostics.md](../spec/diagnostics.md)); parse/check fixtures with blessed goldens; a semantic refusal rendered with source line and underline, exercised through the CLI |
| The machine is atomic and loses nothing | reserve → evaluate → reserve-capacity → commit → apply protocol; fault-injection tests replay refused commits, opens, closures, and expiries without loss or duplication |
| Effects are journaled before they happen | OUTBOUND open precedes every start through the then-named `Partner` seam (now `OutboundConnectorDriver`); closures are gated against the then-current contract error set BEFORE becoming causal facts; close-once arbitrates every race |
| Time is the machine's, not the world's | budgets arm at delivery (queue time counts), only the scheduler mints a timeout, a cancelled call kills its process group; the response/expiry race is genuine and journaled |
| Behavior is sealed at the byte | deterministic CBOR projection per [identity.md §3](../spec/identity.md), blake3 seals, golden traces with a structural diff against the STORED golden; the encoder is pinned to RFC 8949 test vectors |
| The lab proves the machine, not a double | scenarios run on the production `Vm` behind the same then-named `Partner` seam as the real boundary implementation; misspelled targets, non-boolean predicates, uncovered calls, and unsettled machines are all named failures |
| Determinism holds across machines | 2026-08-04, commit `a05b77e`: clean clone on Debian 13 x86_64, throwaway toolchain 1.97.1 — full gate green, and the semantic hash and canonical seals byte-identical to the macOS development machine |

Final state: 209 tests (golden traces and CLI drives included), zero
clippy warnings under `-D warnings`, one external dependency
(`blake3`), toolchain pinned, CI guarding the full gate on Linux and
macOS for every future push.

## What the reviews changed

Eleven adversarial review passes ran against the slice; the last
declared the series closed. Their material effect, visible in the
history: the lab was made incapable of false greens; envelope writes
became checked targets instead of runtime string interpretation; the
Process boundary was rebuilt around prepared bytes, an asynchronous external
responder, and protocol-terminated pipes; the timeout moved wholly into
the scheduler with delivery-armed budgets; identity gained the
resolved contract, the budget reference, and sugar-insensitive write
hashing. Where implementation exposed a contradiction inside the
frozen specification, the boundary rule held: nine errata record the
conflicts ([errata.md](../spec/errata.md)) rather than silent
arbitration in code.

## What M0 deliberately did not prove

The assumed scope deviations live in
[`m0-exit-notes.md`](m0-exit-notes.md) with their normative homes:
the sim-05 ten-record reference trace, `StderrLine` on a successful
job, the `PreparedRequest` boundary carriage, and the
DeliveryRef → `call_ref` derivation — the spec-first prerequisite of
any multiport connector. The forward-looking concurrency model and
its seven open decisions are in
[`parallelism-notes.md`](parallelism-notes.md).

## Pressure observed → next slice

The pressures that accumulated during the build, in order of force:

1. **Identity wants the delivery reference.** Three separate findings
   (fan-out budgets, queue lookups, `call_ref` collision) converged on
   the same missing concept. Spec work, and it blocks multiport.
2. **The core is separable now.** The module boundary audit found the
   no_std frontier already clean — one inverted dependency
   (`vm → connectors::prepare`, the PreparedRequest debt) and zero
   forbidden types. Splitting while the surface is small is cheap;
   later it will not be.
3. **The plan wants to travel.** Boot-from-artifact (and any embedded
   target) needs a canonical, verifiable serialization of the
   `ExecutionPlan` under the same byte discipline the golden traces
   proved out.
4. **Connectors are the velocity bottleneck of the next milestone.**
   The contract sheets exist; a reusable conformance harness over the
   then-named `Partner` seam was expected to turn each new adapter into a
   half-day task. M1.5 C5 later separated the reusable registry/binder tax from
   File-specific work and found no trustworthy elapsed-time evidence; the
   half-day wording remains a historical hypothesis, not a measured claim.

These pressures are the M0.5 scope ([`m0.5-slice.md`](m0.5-slice.md)).
