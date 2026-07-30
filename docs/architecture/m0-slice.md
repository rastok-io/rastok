# M0 end-to-end slice

> Implementation slice — non-normative and under review.

M0 proves that one Rastok program can cross the whole mechanical chain
without inventing a second engine for simulation.

## Outcome

Given a small `.rkg` program, Rastok must:

1. parse it with source spans and useful diagnostics;
2. reject invalid names, types, ports, or graph structure;
3. lower the accepted graph into a canonical immutable plan;
4. execute that plan in a single-worker, in-memory runtime;
5. turn a Process connector request into an explicit effect intent;
6. resolve the intent once through a real App handler;
7. resolve the same intent through a controlled Lab handler;
8. record typed inputs and outcomes in their causal order;
9. compare the Lab result with a stable golden artifact.

The proof is complete only when the result is reproducible from a clean
checkout and a failed expectation produces a diff that identifies the
meaningful semantic mismatch.

## Slice boundaries

| Stage | Input | Output | Primary responsibility |
|---|---|---|---|
| Parse | UTF-8 `.rkg` source | syntax tree plus source spans | source front end |
| Check | syntax tree | checked graph or typed diagnostics | semantic checker |
| Plan | checked graph | canonical executable plan | planner and canonicalizer |
| Step | plan, state, input event | new state plus effect intents | runtime core |
| Handle | Process effect intent | typed outcome event | selected App or Lab handler |
| Record | input and outcome events | ordered in-memory journal | journal boundary |
| Compare | completed Lab run | match or structured golden diff | Lab evidence |

These are logical seams inside the bootstrap crate. They may begin as
modules or private types. M0 does not require a crate per row.

## App and Lab

The execution path up to an effect intent is identical:

```text
checked plan + state + event
              │
              ▼
       deterministic step
              │
              ▼
        Process intent
          ┌───┴───┐
          │       │
     App handler  Lab handler
          │       │
          └───┬───┘
              ▼
       typed outcome event
              │
              ▼
             step
```

The App handler may observe a real child process. The Lab handler must
consume controlled scenario data and must not accidentally fall through
to the host. Both return through the same typed outcome boundary.

## Minimal evidence

M0 should leave behind a small set of executable evidence:

- one successful parse/check/plan fixture;
- one invalid program whose stable diagnostic covers a real semantic
  rule rather than a parser accident;
- one App integration test using a harmless deterministic child
  process;
- one Lab scenario for the same graph with a scripted Process outcome;
- one golden trace and one intentional mismatch test for the diff;
- unit tests around canonical identity and ordering used by the slice.

The normative expectations come from [`../spec/`](../spec/). The
working coverage map is [`spec-to-code.md`](spec-to-code.md).

## Explicit non-goals

M0 does not claim:

- a production durability format or crash-safe storage engine;
- multiple workers, placement, replication, or cluster membership;
- live Saga compensation or Loop recovery managers;
- complete standard-library coverage;
- network Process execution or a stable public wire protocol;
- backwards-compatible public Rust APIs;
- production security, tenancy, packaging, or operations.

The in-memory implementation must nevertheless preserve the logical
boundaries needed by later durability. For example, journal APIs expose
typed logical records and ordering, not assumptions about a future file
layout.

## Exit rule

M0 exits when the chain is continuous and testable, not when every
documented feature has a stub. The next slice is selected from observed
pressure in the implementation and the evidence gates in
[`ROADMAP.md`](../../ROADMAP.md).
