---
title: Process
description: Ports, terminal outcomes, settings and qualification evidence for the Process connector.
order: 2
---

`std/process@6`. `job` starts one child per Call and publishes its ordered frames with the exit. `service` opens a resident child per Node instance on first input and reuses it. One exit ends that child, not each application request.

## Declaration

This declaration comes from `examples/echo.rkg`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector shell : Process {
  mode = job ; command = "cat"
  codec = lines
  idempotent = true; behavior = deterministic
  timeout = duration("5s")
}
```

The [contract index](../contracts.md) explains shared settings, build features
and the distinction between a Contract, a Lab model and an installed Driver.

Selector: `mode` defaults to `job`.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `job` | outbound | call | `lab-call` | `process-job` |
| `service` | inbound+outbound | session | `lab-process-service` | `process-service` |

## Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| job, service | `in` | input | `codec = lines` → `text`; `codec = jsonl` → `variant`; `codec = bytes` → `bytes` |
| job, service | `out` | output | `codec = lines` → `text`; `codec = jsonl` → `variant`; `codec = bytes` → `bytes` |
| job, service | `err` | output | `err_codec = lines` → `text`; `err_codec = bytes` → `bytes` |
| job, service | `exit` | output | ProcessExit, below |

| Mode | Output cardinality |
|---|---|
| job | `err`: zero or more; `out`: zero or more; exactly one final `exit` per Call |
| service | Zero or more frames and writes; exactly one `exit` per Session |

## Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `CaptureFailed` | Empty | outcome_unknown |
| `DecodeFailed` | `{ line: text?, message: text }` | outcome_unknown |
| `Exited` | `{ exit_code: int32, signal: int32? }` | after_effect |
| `OutcomeUnknown` | Empty | outcome_unknown |
| `OutputOverflow` | `{ limit: uint64, reason: enum { budget, capacity } }` | outcome_unknown |
| `SpawnFailed` | `{ message: text }` | before_effect |
| `TimeoutExpired` | Empty | outcome_unknown |

## Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| job, service | `args` | `array<text>` | `[]` | instance |
| job, service | `behavior` | `deterministic \| probabilistic` | required | plan |
| job, service | `codec` | `bytes \| jsonl \| lines` | required | plan |
| job, service | `err_codec` | `bytes \| lines` | `lines` | plan |
| job, service | `command` | `text` | required | instance |
| job, service | `cwd` | `text` | optional | instance |
| job, service | `env` | `map<text,text>` | optional | instance |
| job | `idempotent` | `bool` | `false` | plan |
| job | `timeout` | `duration` | optional | instance |
| job | `max_output` | `ByteSize` | `"8MiB"` | plan |
| job | `on_timeout` | `kill` | `kill` | plan |
| service | `max_frame` | `ByteSize` | `"1MiB"` | plan |

The shared catalogue lists framing choices for both modes; service validation further restricts `codec` to `lines | jsonl` and `err_codec` to `lines`. Service has no per-turn timeout or idempotence setting. Its mock supports input-driven `match` responses and an optional terminal; delayed/unsolicited mock frames, `now()` in Session mocks and system-close acknowledgements are not scripted. Physical unsolicited output is supported. Job mocks retain the ordinary Call timeout and terminal rules.

## Qualification inventory

`job`: fixture `crates/rastok/tests/conformance_process.rs`. Cases without a portable forced fixture: `CaptureFailed`, `OutcomeUnknown`. Related tests: `crates/rastok/tests/process_service.rs`, `crates/rastok/src/connector/drivers/process/mod.rs`.

`service`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/tests/process_service.rs`, `crates/rastok/src/connector/drivers/process/mod.rs`.
