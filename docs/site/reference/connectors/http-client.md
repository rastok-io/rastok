---
title: HttpClient
description: Ports, terminal outcomes, settings and qualification evidence for the HttpClient connector.
order: 3
---

`std/http-client@1`. One HTTP request per Call. Usable response bodies include non-success statuses. Decoded 500–599 responses use `ServerError`; other final statuses use `Completed`. A body decoding fault takes precedence over status classification.

## Declaration

This declaration comes from `crates/rastok/src/lab.rs`. Place the Connector in a Graph
and wire its ports; the excerpt alone is not an App.

```rkg
connector network : HttpClient {
  url = "https://example.test/v1"
  timeout = duration("5s")
  codec = json
  idempotent = true; behavior = deterministic
}
```

The [contract index](../contracts.md) explains shared settings, build features
and the distinction between a Contract, a Lab model and an installed Driver.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| No selector | outbound | call | `lab-call` | Not compiled |

## Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| all | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| all | `out` | output | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text`; explicit JSON response type admitted |
| all | `exit` | output | HttpClientExit, below |

| Mode | Output cardinality |
|---|---|
| all | `out`: exactly one on `Completed` or `ServerError`; exactly one final `exit` per Call |

## Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `BodyTooLarge` | `{ limit: uint64 }` | outcome_unknown |
| `Completed` | `{ status: int32 }` | after_effect |
| `ConnectFailed` | Empty | before_effect |
| `DecodeFailed` | Empty | outcome_unknown |
| `DnsFailed` | Empty | before_effect |
| `OutcomeUnknown` | Empty | outcome_unknown |
| `ProtocolFailed` | Empty | outcome_unknown |
| `ServerError` | `{ status: int32 }` | after_effect |
| `TimeoutExpired` | Empty | outcome_unknown |
| `TlsFailed` | Empty | before_effect |

## Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| all | `auth_bearer` | `text` | optional | invocation |
| all | `behavior` | `deterministic \| probabilistic` | required | plan |
| all | `codec` | `bytes \| json \| text` | required | plan |
| all | `json_profile` | `projected` | optional | plan |
| all | `idempotent` | `bool` | `false` | plan |
| all | `headers` | `HTTP headers` | optional | invocation |
| all | `custom_headers` | `map<text,array<text>>` | optional | invocation |
| all | `idempotency_key` | `text` | optional | invocation |
| all | `max_body` | `ByteSize` | `"8MiB"` | plan |
| all | `method` | `text` | `"GET"` | invocation |
| all | `timeout` | `duration` | required | instance |
| all | `tls_ca` | `bytes` | optional | instance |
| all | `tls_certificate` | `bytes` | optional | instance |
| all | `tls_private_key` | `bytes` | optional | instance |
| all | `url` | `text` | required | invocation |
| all | `versions` | `array<http/1.1 \| http/2 \| http/3>` | `["http/3", "http/2", "http/1.1"]` | plan |
| all | `zero_rtt` | `disabled` | `disabled` | plan |

`codec = json` permits an explicit `output out: Response`. `json_profile = projected` additionally skips extra members and fills absent nullable fields, while rejecting malformed required values. `auth_bearer = secret(...)` and `url = env(...)` use the declared boot-reference seam even though the table lists each setting's latest general phase. Those references are resolved from deployment configuration at startup, not from an input envelope. There is no automatic redirect or Retry: the graph declares its Retry policy.

## Qualification inventory

fixture `crates/rastok/tests/conformance_http_client.rs`. Cases without a portable forced fixture: `DnsFailed`, `OutcomeUnknown`. Related tests: `crates/rastok/src/connector/drivers/network/http_client.rs`.
