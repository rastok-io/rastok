# Rastok roadmap

This page says what is done, what comes next, and in what order. Only the
releases already shipped carry a date. What each release changed is in the
[changelog](CHANGELOG.md); the evidence behind each milestone is in
[`docs/architecture/`](docs/architecture/). The detailed ledger kept until
0.1.1 stays readable in the
[roadmap of v0.1.1-alpha](https://github.com/rastok-io/rastok/blob/v0.1.1-alpha/ROADMAP.md).

## Done

### M0, the mechanical proof (4 August 2026)

The central loop works end to end: parse and check a small `.rkg` subset,
lower it to an executable plan, run it with one Worker, run one scenario
with a mock, virtual time and a golden trace, cross one real `Process`
boundary, show a useful diff at the first divergence, and reproduce the
result on two machines.
[Exit report](docs/architecture/m0-exit-report.md).

### M0.5, the engine consolidation (6 August 2026)

A `no_std`-ready core split from the hosted layer, the executable plan
carried as a versioned and verifiable package, the Connector conformance
harness, and a first embedded run under emulation.
[Slice](docs/architecture/m0.5-slice.md).

### M1, the alpha: v0.1.0-alpha (5 November 2026)

An asynchronous integration, with a request, an external partner and a late
callback, built, simulated and shown end to end before its real systems
exist. It brings the language, the compiler, the runtime, the standard
Connectors, the Lab with virtual time and injected faults, the journal kept
in memory, recorded runs and their replay, golden traces, the visual
debugger, and eight examples, from echo to agents.
[Slice](docs/architecture/m1-slice.md).

### v0.1.1-alpha (18 November 2026)

A corrective release. It fixes what the first users of 0.1.0-alpha ran into,
moves several failures from execution to `rastok check`, makes the first
hour easier, and lets the debugger keep up with large graphs.
[Changelog](CHANGELOG.md#v011-alpha-18-november-2026).

## Next: v0.2.0-alpha, the language refuses earlier

More mistakes are caught by `rastok check` instead of at execution, and a
few words are renamed while there are still few programs to migrate. The
planned changes include:

- `text` becomes `string`;
- lifetimes get explicit names;
- an instance with a key must say when it ends.

A program written for 0.1 will need edits. No old spelling is kept as a
synonym, and the changelog will list every change. The scope is still being
settled, and the release has no date yet.

## Then, in an order that users can change

- **Nothing is lost on failure.** A durable local journal, restart after a
  stop, and `crash` and `restart` inside a scenario. Planned as 0.3.0-alpha.
- **An incident becomes a test.** A real failure turned into a replayable
  scenario in the Lab, with its evidence.
- **The classic building blocks** of an information system's integrations.
- **Agents under control**: approval, refusal and budgets, down to embedded
  targets.
- Later: assurance and compliance, then scale and enterprise operation.

What comes after 0.2.0 is decided from what early users and their use cases
ask for most. To weigh in, open an
[issue](https://github.com/rastok-io/rastok/issues).

## The evidence we look for

- Someone we do not know installs Rastok from the documentation alone.
- That person writes or changes a graph of their own.
- A real incident becomes a replayable scenario.
- A team uses Rastok in its own technical evaluation.

This evidence, more than any plan, decides the next product scope.

## Toward 1.0

Stability is promised at 1.0, once everything the specification defers is
settled. Until then every version carries `-alpha`, and the second number
changes when the language breaks programs: see
[the versioning rules](CHANGELOG.md#versions).
