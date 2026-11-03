---
title: A program
description: Read a complete Rastok program by separating its Graph, Operators, Connectors, wiring and Lab.
order: 1
---

After this chapter you can read a small `.rkg` file, identify where its
decisions and external work belong, and check its wiring without starting
the external program.

## Start with one boundary

Open `examples/echo.rkg` in the checkout. Its first declaration is a Graph
named `echo`. A Graph contains children and the connections between them.
It also defines the boundary through which the surrounding program can send
and receive data. Its children own the application handlers and private
`var` state.

The `: App` annotation attaches the App Extension. It marks this Graph as
the public root of the compiled unit. There must be exactly one such root
among the files compiled together. A unit can span several files; this
example fits in one.

This is the complete Graph, from `examples/echo.rkg`, lines 5–22:

```rkg
graph echo : App {
  connector shell : Process {
    mode = job ; command = "cat"
    codec = lines
    idempotent = true; behavior = deterministic
    timeout = duration("5s")
  }

  operator relay {
    on in where data != "" {
      out = data
    }
  }

  in -> relay -> shell -> out
  shell.err -> nothing
  shell.exit -> nothing
}
```

## Read declarations before connections

The `relay` Operator owns the decision. Its handler reacts to an envelope
on `in`; `data` refers to that envelope's payload. The guard rejects the
empty string. On the accepted path, assigning to `out` emits the payload.
Operators perform pure computation and may keep declared private state; this one
needs none.

The `shell` Connector owns the external boundary. `Process` names its
Contract, while `mode = job` selects a finite child-process invocation.
`command` and `codec` describe the requested external work and framing.
The timeout bounds waiting for that invocation. The compiler checks these
settings against the Process Contract.

A Driver implements the Contract. With `run`, the physical Process Driver
starts `cat`. With `simulate`, a Lab mock supplies its outcomes. The
Operator and the wiring remain the same.

## Account for every possible output

The final three lines are the wiring. `->` carries values in one direction;
the abbreviated chain uses the resolved default ports. The App input feeds
`relay`, its accepted output feeds `shell`, and the echoed text crosses the
App output boundary.

The two remaining outputs matter. Process `err` carries stderr, while
`exit` carries the typed terminal outcome of the invocation, including a
timeout. Both are explicitly sent to `nothing`. That is a deliberate
discard, not an unwired output that the runtime happens to ignore.

Every output the compiler considers possible needs a destination. A
statically proven impossible output is exempt. Removing either discard
from this Graph produces `E-UNWIRED-OUTPUT`. A discarded emission remains
visible in the journal and in Lab expectations.

## Check before running

From the repository root:

```sh
rastok check examples/echo.rkg
```

The command checks the source and reports its Node count and semantic hash.
It does not start `cat`, and it does not write a separate plan file that
you must pass to the next command. The accepted plan contains resolved
types, declarations and wiring for the runtime to interpret.

The rest of this source file is `lab smoke`. It supplies scenarios for the
same App, including the empty input and a silent process. Run those with
`rastok simulate examples/echo.rkg`. Each scenario records the graph's
transactions and the scripted boundary outcomes. Adding a scenario does
not change the program's semantic hash: the Lab is not deployed.

The normative sources are `docs/spec/grammar.md` for declarations and
`docs/spec/delivery-semantics.md` for output dispositions. The conceptual
model is also explained in `WHITEPAPER.md`, section 2. The current scope
remains an experimental alpha; read
[what is available today](../index.md#what-is-available-today) before treating
a successful example as a deployment guarantee.

## Summary

- A Graph contains Nodes and wiring; `App` marks the public root.
- An Operator decides; a Connector crosses a declared external boundary.
- Every possible output has an explicit destination.
- Checking, simulation and real execution are different commands over the
  same source program.

## Next

Continue with [Types and expressions](02-types-and-expressions.md) to read
the values carried by those connections. For the hands-on mirror, run
[Echo](../examples/01-echo.md), change its mock and inspect the first golden
difference.
