---
title: WebSocketClient
description: Ports, terminal outcomes, settings and qualification evidence for the WebSocketClient connector.
order: 5
---

`std/websocket-client@3`. Opens a peer Session. Each complete message is one value; each graph write is journaled before I/O. One terminal ends the Session, not a message.

## Declaration

This declaration comes from `crates/rastok/src/lab.rs`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector network : WebSocketClient {
  url = "wss://example.test/socket"
  handshake_timeout = duration("5s")
  codec = json
   behavior = deterministic
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
| all | `auth_bearer` | `text` | optional | instance |
| all | `behavior` | `deterministic \| probabilistic` | required | plan |
| all | `codec` | `bytes \| json \| text` | required | plan |
| all | `handshake_timeout` | `duration` | required | instance |
| all | `headers` | `HTTP headers` | optional | instance |
| all | `custom_headers` | `map<text,array<text>>` | optional | instance |
| all | `max_message` | `ByteSize` | `"8MiB"` | plan |
| all | `subprotocols` | `array<text>` | optional | instance |
| all | `tls_ca` | `bytes` | optional | instance |
| all | `tls_certificate` | `bytes` | optional | instance |
| all | `tls_private_key` | `bytes` | optional | instance |
| all | `url` | `text` | required | instance |
| all | `versions` | `array<http/1.1 \| http/2 \| http/3>` | `["http/3", "http/2", "http/1.1"]` | plan |
| all | `zero_rtt` | `disabled` | `disabled` | plan |

The Lab scripts Session opening, frames, writes and terminal outcomes through the Session model. This is protocol evidence without a socket, not physical handshake qualification. Input after a terminal may open a successor Session; old writes are not resent automatically.

## Qualification inventory

No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/websocket.rs`.
