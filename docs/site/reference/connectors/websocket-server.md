---
title: WebSocketServer
description: Ports, terminal outcomes, settings and qualification evidence for the WebSocketServer connector.
order: 6
---

`std/websocket-server@3`. Accepts peer Sessions. Replies follow session lineage or an explicit `correlate`. A pre-admission handshake refusal creates no Session or exit.

## Declaration

This declaration comes from `examples/agent-gateway.rkg`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector clients : WebSocketServer {
  listen = "127.0.0.1:8443"
  path = "/agent"
  codec = json
  handshake_timeout = duration("5s")
  max_message = "1MiB"
  correlate = meta.ext.session_id
  versions = ["http/1.1"]
}
```

The [contract index](../contracts.md) explains shared settings, build features
and the distinction between a Contract, a Lab model and an installed Driver.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| No selector | inbound+outbound | session | `lab-websocket` | Not compiled |

## Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| all | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| all | `out` | output | Same as `in` |
| all | `exit` | output | WebSocketExit, below |

| Mode | Output cardinality |
|---|---|
| all | Zero or more frames and writes; exactly one `exit` per Session |

## Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `Closed` | `{ code: int32, reason: text }` | Not an outbound Call |
| `ConnectFailed` | Empty | Not an outbound Call |
| `DecodeFailed` | Empty | Not an outbound Call |
| `DnsFailed` | Empty | Not an outbound Call |
| `HandshakeRejected` | `{ status: int32 }` | Not an outbound Call |
| `MessageTooLarge` | `{ limit: uint64 }` | Not an outbound Call |
| `OutcomeUnknown` | Empty | Not an outbound Call |
| `ProtocolFailed` | Empty | Not an outbound Call |
| `TimeoutExpired` | Empty | Not an outbound Call |
| `TlsFailed` | Empty | Not an outbound Call |
| `TransportLost` | Empty | Not an outbound Call |

## Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| all | `allowed_origins` | `array<text>` | optional | instance |
| all | `auth_bearer` | `text` | optional | instance |
| all | `behavior` | `deterministic` | `deterministic` | plan |
| all | `codec` | `bytes \| json \| text` | required | plan |
| all | `correlate` | `text` | optional | invocation |
| all | `handshake_timeout` | `duration` | required | instance |
| all | `headers` | `HTTP headers` | optional | instance |
| all | `custom_headers` | `map<text,array<text>>` | optional | instance |
| all | `listen` | `text` | required | instance |
| all | `max_message` | `ByteSize` | `"8MiB"` | plan |
| all | `path` | `text` | `"/"` | plan |
| all | `subprotocols` | `array<text>` | optional | instance |
| all | `tls_certificate` | `bytes` | optional | instance |
| all | `tls_private_key` | `bytes` | optional | instance |
| all | `versions` | `array<http/1.1 \| http/2 \| http/3>` | `["http/3", "http/2", "http/1.1"]` | plan |
| all | `zero_rtt` | `disabled` | `disabled` | plan |

The Lab supplies session-open, frame and terminal observations through the same Session model. It tests graph behavior, not a physical listener, authentication or wire handshake. Explicit correlation never chooses another Connector's Session.

## Qualification inventory

No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/websocket.rs`.
