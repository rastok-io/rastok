---
title: File
description: Ports, terminal outcomes, settings and qualification evidence for the File connector.
order: 1
---

`std/file@1`. One file operation per Call. `read` publishes complete bytes; `replace` stages and atomically replaces one file. Directory operations are outside this Contract.

## Declaration

This declaration comes from `examples/file-pipeline.rkg`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector read_file : File {
  mode = read
  path = data as text
  max_read = "64KiB"
  timeout = duration("2s")
}
```

The [contract index](../contracts.md) explains shared settings, build features
and the distinction between a Contract, a Lab model and an installed Driver.

Selector: `mode` is required.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `read` | outbound | call | `lab-call` | `file` |
| `replace` | outbound | call | `lab-call` | `file` |

## Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| read | `in` | input | `variant` |
| read, replace | `out` | output | `bytes` |
| read, replace | `exit` | output | FileExit, below |
| replace | `in` | input | `bytes` |

| Mode | Output cardinality |
|---|---|
| read | `out`: exactly one on `Completed`; exactly one final `exit` per Call |
| replace | `out`: never; exactly one final `exit` per Call |

## Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `AtomicReplaceUnavailable` | Empty | before_effect |
| `Completed` | Empty | after_effect |
| `InvalidPath` | Empty | before_effect |
| `IoFailed` | Empty | outcome_unknown |
| `NotAFile` | Empty | before_effect |
| `NotFound` | Empty | before_effect |
| `OutcomeUnknown` | Empty | outcome_unknown |
| `PermissionDenied` | Empty | before_effect |
| `ReadLimitExceeded` | `{ limit: uint64 }` | after_effect |
| `TimeoutExpired` | Empty | outcome_unknown |

## Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| read, replace | `path` | `text` | required | invocation |
| read, replace | `idempotent` | `bool` | `true` | plan |
| read, replace | `behavior` | `deterministic` | `deterministic` | plan |
| read, replace | `timeout` | `duration` | optional | instance |
| read | `max_read` | `ByteSize` | `"8MiB"` | plan |

File paths are logical paths relative to the deployment root. A replacement has no truncate fallback and makes no crash-durability promise. The checker fixes File repetition to idempotent and behavior to deterministic.

## Qualification inventory

`read`: fixture `crates/rastok/tests/conformance_file.rs`. Cases without a portable forced fixture: `PermissionDenied`, `AtomicReplaceUnavailable`, `IoFailed`, `TimeoutExpired`, `OutcomeUnknown`. Related tests: `crates/rastok/tests/file_connector.rs`.

`replace`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/tests/file_connector.rs`.
