---
title: Command line
description: Choose a Rastok command, supply its options, and distinguish read-only checks from real execution and file writes.
order: 9
---

After this chapter you can choose commands and interpret their exit status.
Run examples from the checkout root.

## Help and version

```text
rastok --help | -h | help
rastok --version
```

The help aliases print usage on stdout and exit successfully.
`--version` prints `rastok` and the package version. Neither accepts additional
arguments. Help is global, not command-specific.

Below, square brackets mean optional, `|` separates alternatives, and `...`
means one or more files. These markers are not literal arguments.
Multiple source paths form one compilation unit.

## Create a program: new

```text
rastok new <name>
```

`rastok new hello` creates `hello.rkg`: an App, a typed Operator and a Lab
scenario. Use a bare name, without a path or extension. Existing files are
never overwritten. Printed follow-up commands are not executed.

## Check a program: check

```text
rastok check <file.rkg>...
```

```sh
rastok check examples/echo.rkg
```

This checks types, wiring and Contracts, then prints the Node count and
semantic hash. It neither executes the App nor writes a plan file. Success
does not prove partner availability or compatible physical Drivers.

## Run scenarios: simulate

```text
rastok simulate <file.rkg>... [--deployment <profile.json>] [--compare <dir> | --save <dir>] [--ui [--ui-port <port>]]
```

```sh
rastok simulate examples/echo.rkg --compare examples/traces/echo
```

Simulation uses Lab mocks and virtual time, not physical Drivers. Without
`--ui`, it runs every scenario, printing `PASS` or `FAIL` followed by
`simulation.scenario` and a final count. Direct scenarios use their Lab's name.
A unit without a Lab refuses.

- `--compare` reads reviewed traces and reports the first difference in
  each differing scenario. A missing reference fails the comparison.
- `--save` creates or replaces reference files named
  `<simulation>.<scenario>.trace`. Review the semantic reason for the change;
  saving is not proof that expectations pass.
- These two options are mutually exclusive. With neither, only scenario
  expectations are checked; no reference trace is read or written.
- `--deployment` supplies boot values and attestations needed by the graph,
  even when the external responses are mocked.

`--ui` requires `hosted-ui`, prints a token-bearing loopback URL and starts
paused. `--ui-port` requires `--ui` and selects a nonzero TCP port;
otherwise the OS chooses one.

## Execute the App: run

```text
rastok run <file.rkg>... [--file-root <dir>] [--deployment <profile.json>] [--json-logs] [--record <file.rkrec>]
```

```sh
printf 'hello\n' | rastok run examples/echo.rkg --record echo.rkrec
```

This uses physical Drivers and can start processes, write files or contact
services. Each stdin line is a text submission; public App outputs go to
stdout. With one declared App output port, each envelope prints its value alone.
With several output ports, each line keeps the `port: value` prefix, even if
only one port emits. Lab scenarios are not executed.

- `--file-root` enables the physical File Driver within an explicit root.
- `--deployment` reads typed boot settings, secret coordinates, attestations
  and any Driver selection from a deployment JSON file. Secret material is
  supplied out of band, never as a CLI argument.
- `--json-logs` exports structured events on stderr; outputs stay on stdout.
  Other diagnostics may also use stderr, so it is not an exclusively JSON stream.
- `--record` writes a new diagnostic recording and refuses an existing path.
  The recorder also refuses personal-data plans, nonempty deployment
  attestations and boot-dependent Connector settings. Treat recordings as
  sensitive local evidence, not durable restart storage.

`run --ui` is not supported. Inspect a supported recording afterward instead.

## Inspect a recording: replay

```text
rastok replay <file.rkrec> [--ui [--ui-port <port>]]
```

`rastok replay echo.rkrec` prints App outputs on stdout and a record-count/seal
summary on stderr, without Driver I/O. It uses the same output format as `run`:
values alone for a single App output, `port: value` for several outputs.
`--ui` uses simulation's port rules and opens read-only recorded history.
Missing source lines are explicit. Replay needs no original source, ignores
source files on disk, and cannot execute the program or create a Lab scenario.

## Inspect capabilities: connectors

```text
rastok connectors [--json]
```

The human view lists Contracts, modes, ports, settings, installed
implementations, Lab bindings and untested cases. `--json` produces the machine-readable
catalogue on stdout. Neither constructs a Driver nor probes a service.
Checker support, a compiled implementation and successful deployment remain
different questions.

## Resolve project schemas: --resolve-open-api

```text
rastok --resolve-open-api <project-dir> [--check]
```

This matches static HTTP endpoints to OpenAPI 3.0/3.1 documents and writes
`openapi.generated.rkg` types, leaving authored sources untouched. `--check`
writes nothing and refuses missing or stale output. Unsupported or ambiguous
schemas refuse. Generation cannot prove a live API honors its schema.
The leading double dash is required.

## Exit status and streams

- `0`: the requested operation succeeded, including explicit help or version.
- `1`: compilation, execution, scenario, comparison or file operation failed.
- `2`: a recognized usage error, such as a missing command or invalid option combination.

Usage errors go to stderr, including a bare `rastok` invocation. Scenario
failure details remain in the simulation report on stdout; do not detect
success solely by whether stderr is empty.

## File formats

`.rkg` holds the program and its Lab. `.trace` holds one scenario's reviewed
causal reference, read by `simulate --compare`. `.rkrec` records a real run
for inspection by `replay`, without repeating its external effects.

## Summary

Check before executing, simulate before contacting partners, and save only
an intentional new baseline. Success is not a deployment guarantee; read the
[alpha limits](../index.md#what-is-available-today). Normative sources include
`docs/spec/ui.md` and `docs/spec/schema-projections.md` §5.

## Next

Try [Echo](../examples/01-echo.md) through simulation and replay.
Previous: [Privacy, secrets and measurements](08-privacy-secrets-and-measurements.md).
Deployment: [Running for real](05-running-for-real.md).
