---
title: The Lab
description: Executable Lab declarations, mock responses, scenario actions, observation windows and golden-trace commands.
order: 4
---

The real world (App) supplies external observations through Drivers. The
virtual world (Lab) supplies scripted observations and logical time to the
same graph. Operators and Contract checks execute in both.

## Declarations

The following notation describes syntax; angle brackets stand for authored
names or values, not literal characters.

```text
lab <name> {
  mock <connector-path> { <handlers> }
  scenario <name> { <actions and expectations> }
  simulation <name> {
    mock <connector-path> { <handlers> }
    scenario <name> { <actions and expectations> }
  }
}
```

| Declaration | Scope and rule |
|---|---|
| `lab <name>` | Verification corpus for the unit's App; same-named fragments merge across source files |
| Direct `scenario <name>` | Runs in the implicit simulation named after its Lab |
| `simulation <name>` | Optional group with shared mocks; omission of a target selects the App |
| `simulation <name> : App` | Explicit App target; `app` and the App graph's declared name also work |
| `mock <connector-path>` | Names an actual Connector: a child of the App root directly, or a top-level declaration by its name |
| `scenario <name>` | Independent run with its own mocks, origin and actions; the name carries its intent |
| Mock override | Replaces the whole mock for that Connector: scenario, then simulation, then Lab |
| Unit-level `val` | Available in mocks, injections, expectations and durations; Graph-local bindings do not escape into the Lab |

A unit has one logical Lab. Duplicate scenarios or mocks at one level refuse. An explicit simulation
cannot use its Lab's name when the Lab also contains direct scenarios.
The Lab is absent from the production package and semantic hash. Mocks do
not add ports, change Contracts or supply application metadata fields that
the graph never writes.

## Mock responses

| Form | Meaning |
|---|---|
| `on in where predicate { ... }` | First matching request clause in source order |
| `on in { ... }` | Catch-all request clause |
| Closed or predicate `match` | Select one checked response branch, using the same dispatch rules as the graph |
| Local `val` | Pure response computation |
| `out = value`, `err = value` | Emit a frame only on a port that the resolved Contract exposes |
| `exit = Case { ... }` | Emit the Contract's typed terminal; unique and last |
| `out = nothing` with no terminal | Explicit silence: leave the Call awaiting its deadline |

Each non-silent Call response has exactly one final `exit` and the frames
that terminal permits. A Session response can emit frames without ending
the resource; an optional exit remains last. For Process service, that
terminal closes the mock child, not an individual conversation turn.
An uncovered request or mock evaluation fault fails the scenario.

Mocks are request-driven and stateless. Call mocks can select responses from
the request and `now()`: [Two tenants](../examples/03-two-tenants.md) selects
silence for Alice at the epoch, then responds after the time advance.
There is no authored mock invocation counter.
`now()` is unavailable in Session mock bodies. Mock metadata writes,
multi-emission statements and Loop commands refuse.

## Actions

| Action | Effect |
|---|---|
| `app.in = value` | Admit an input through a declared App input |
| `listener.out = value` | Supply a decoded HttpServer `ack` request through its source model |
| Injection followed by `seq N` | Choose the explicit nonnegative ingress sequence instead of the fixture counter |
| `advance duration("5s")` | Move the logical timeline and process due obligations without sleeping five seconds |
| `origin = timestamp("2026-10-30T12:00:00Z")` | Set the immutable civil anchor for this scenario; it is a declaration, not an action |

Time starts at logical zero; the default civil origin is the Unix epoch.
The origin affects `now()` and conversion of epoch-aligned tumbling bounds,
not the graph hash. An advance uses a constant duration and cannot move time
backwards. Each action settles ready work before the next expectation.

Injection targets are not arbitrary intermediate ports. Source injection
models a decoded request, not malformed wire bytes or failed bearer
authentication. Those refusals need Driver tests. WebSocket and Process
Session fixtures use their declared Session model and correlation metadata;
they do not fabricate a Call per frame. See the capability distinctions in
[Standard contracts](contracts.md).

## Expectations and windows

| Form | Assertion |
|---|---|
| `expect target.port` | Exactly one observation |
| `expect N target.port` | Exactly `N` observations, including zero |
| `expect target.port == nothing` | No observations |
| `expect [N] target.port == value` | Exactly `N` observations equal to the value; default one |
| `expect [N] target.port where predicate` | Exactly `N` matching observations; default one |

Square brackets in this table denote an optional count, not literal syntax.
Counts are nonnegative integers. A nonzero count with `== nothing` refuses;
zero with any other equality value also refuses. Use
`expect 0 target.port where predicate` to require no matching observation.

Equality uses the ordinary value grammar: scalars, records, tagged cases,
maps, frontends and envelope fields. Values are checked against the observed
port type. A complete data value compares structurally; an envelope pattern
without a whole `data` assignment compares only its written fields.
Predicates are checked over the observed port type; narrow a `variant`
before reading its fields. An evaluation fault is a scenario fault, not a
non-match. A newline after the port ends the expectation.

An injection or time advance opens a new observation window. All following
expectations inspect that window without consuming it; earlier windows are
excluded. An emission appears once at its source, even if routed to
`nothing`. `app.<output>` observes an actual App-boundary crossing.
Expectations do not inspect private Operator state or a mailbox.

## Golden traces

```sh
rastok simulate examples/echo.rkg
rastok simulate examples/echo.rkg --compare examples/traces/echo
rastok simulate examples/echo.rkg --save examples/traces/echo
```

| Option | Result |
|---|---|
| Neither | Run scenario expectations without consulting reference traces |
| `--compare <dir>` | Compare with stored traces, without modifying them |
| `--save <dir>` | Write or replace reference traces for intentional review |
| Both | Usage error |
| `--ui` | Inspect simulation boundaries and run them step by step in the debugger (requires a `hosted-ui` build) |

Each file is `<simulation>.<scenario>.rkc`. The stored format is
deterministic CBOR rendered as one lowercase hexadecimal line per causal
record, followed by `seal <BLAKE3 digest>`. The seal covers concatenated
record bytes. It excludes unsealed observations, such as host timing, but
includes a handler's explicit observation of `now()`.

A comparison reports the first divergent record, expected and obtained
values, and causal context. For classified data, displayed details are
masked while the comparison still uses the canonical bytes. A golden trace
is distinct from the diagnostic recording produced by `run --record`:
`replay` reconstructs recorded facts without calling a Driver again.

## Specified, not shipped

| Item | Specification |
|---|---|
| Sub-graph simulation targets other than `app` | `docs/spec/simulation-semantics.md` §2 |
| Delayed mock `after` actions | `docs/spec/simulation-semantics.md` §§3, 5.1; `docs/spec/grammar.md`, mock grammar |
| Scenario `crash node` and `restart node` | `docs/spec/simulation-semantics.md` §5.4 |
| HttpServer `respond` source scripting; delayed/unsolicited Process service scripts | `docs/spec/simulation-semantics.md` §§2, 5.1 |
| Positive `expect fault` and assertions on private state | `docs/spec/simulation-semantics.md` §5.2 |
| Automatic outcome coverage, generated scenarios and tags | `docs/spec/simulation-semantics.md` §§7–8 |
| Production-to-scenario extraction | `docs/spec/simulation-semantics.md` §9 |
| Experiments and theorems | `docs/spec/simulation-semantics.md` §§10–11 |

Sources: `docs/spec/simulation-semantics.md`, `docs/spec/grammar.md`, the
checked scenario tree and the CLI trace writer. The guided walkthrough is
[The Lab: scenarios, mocks and time](../guide/04-the-lab.md).
