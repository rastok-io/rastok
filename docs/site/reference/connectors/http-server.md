---
title: HttpServer
description: Ports, terminal outcomes, settings and qualification evidence for the HttpServer connector.
order: 4
---

`std/http-server@3`. An inbound HTTP Call. `ack` sends 202 after admission; `respond` waits for the graph response. A malformed body or failed authentication can refuse before admission, without inventing a request envelope or an exit.

## Declaration

This declaration comes from `crates/rastok/src/lab.rs`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector network : HttpServer {
  listen = "127.0.0.1:8080"
  codec = json
}
```

The [contract index](../contracts.md) explains shared settings, build features
and the distinction between a Contract, a Lab model and an installed Driver.

Selector: `mode` defaults to `ack`.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `ack` | inbound | call | `lab-http-server` | Not compiled |
| `respond` | inbound | call | `lab-http-server` | Not compiled |

## Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| ack, respond | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| ack, respond | `out` | output | Same as `in` |
| ack, respond | `exit` | output | HttpServerExit, below |

| Mode | Output cardinality |
|---|---|
| ack | One admitted request `out`; exactly one request `exit`; no graph response |
| respond | One admitted request `out`; exactly one request `exit`; at most one response `in` |

## Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `Completed` | `{ status: int32 }` | Not an outbound Call |
| `OutcomeUnknown` | Empty | Not an outbound Call |
| `Overloaded` | Empty | Not an outbound Call |
| `PeerDisconnected` | Empty | Not an outbound Call |
| `RequestExpired` | Empty | Not an outbound Call |

## Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| ack, respond | `auth_bearer` | `text` | optional | instance |
| ack, respond | `behavior` | `deterministic` | `deterministic` | plan |
| ack, respond | `codec` | `bytes \| json \| text` | required | plan |
| ack, respond | `headers` | `HTTP headers` | optional | instance |
| ack, respond | `custom_headers` | `map<text,array<text>>` | optional | instance |
| ack, respond | `listen` | `text` | required | instance |
| ack, respond | `max_body` | `ByteSize` | `"8MiB"` | plan |
| ack, respond | `path` | `text` | `"/"` | plan |
| ack, respond | `tls_certificate` | `bytes` | optional | instance |
| ack, respond | `tls_private_key` | `bytes` | optional | instance |
| ack, respond | `versions` | `array<http/1.1 \| http/2 \| http/3>` | `["http/3", "http/2", "http/1.1"]` | plan |
| ack, respond | `zero_rtt` | `disabled` | `disabled` | plan |
| respond | `correlate` | `expression path` | optional | invocation |
| respond | `timeout` | `duration` | required | instance |

The Lab scripts decoded `ack` requests through `listener.out = value`, then a successful 202. It does not exercise wire decoding or authentication. A `respond` Node can bind silently beside another simulated flow; authored source injection and graph-response scripting for that mode are deferred.

## Qualification inventory

`ack`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/http_server.rs`, `crates/rastok/tests/lab_http_source.rs`.

`respond`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/http_server.rs`, `crates/rastok/tests/lab_http_source.rs`.
