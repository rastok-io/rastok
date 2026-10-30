---
title: Standard contracts
description: Contract identities, modes, ports, terminal cases, settings and Lab support from the executable Connector catalogue.
order: 3
---

This inventory follows `rastok connectors --json` in its emitted order.
Contract availability, Lab models and installed physical Drivers are separate
facts. The captured local default-feature binary lists File and Process
Drivers; networking requires the hosted networking features. Run the command
on the binary you deploy to inspect that assembly.

A setting's latest phase is its evaluation ceiling: plan, boot, instance or
invocation. The compiler selects the earliest phase where its expression
closes. Omitted defaults become effective settings. Optional means absent,
not null. Byte sizes use `B`, `KiB`, `MiB` or `GiB`.

HTTP-derived Contracts share `headers` (a sparse typed record containing
`content_type: media_type`) and `custom_headers`
(`map<text,array<text>>`). Driver-owned headers cannot be overridden.
Certificate and private-key settings must be supplied together.
Declared version preferences are HTTP/3, HTTP/2, HTTP/1.1; the Driver uses
only compiled backends. `hosted-http12`, `hosted-http3` and
`hosted-websocket` select those capabilities.

`idempotent` is a repetition guarantee. `behavior` describes whether equal
requests in equal external state yield equal answers. They are distinct;
[Retry](../guide/07-loops-and-retries.md) also checks terminal visibility.
Sessions have no per-message Call exit or Retry attribute.

## File

`std/file@3`. One file operation per Call. `read` publishes complete bytes; `replace` stages and atomically replaces one file. Directory operations are outside this Contract.

Selector: `mode` is required.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `read` | outbound | call | `lab-call` | `file` |
| `replace` | outbound | call | `lab-call` | `file` |

### Ports and completion

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

### Exit union

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

### Settings

| Mode | Setting | Domain | Presence / default | Latest phase |
|---|---|---|---|---|
| read, replace | `path` | `text` | required | invocation |
| read, replace | `idempotent` | `bool` | `true` | plan |
| read, replace | `behavior` | `deterministic` | `deterministic` | plan |
| read, replace | `timeout` | `duration` | optional | instance |
| read | `max_read` | `ByteSize` | `"8MiB"` | plan |

File paths are logical paths relative to the deployment root. A replacement has no truncate fallback and makes no crash-durability promise. The checker fixes File repetition to idempotent and behavior to deterministic.

### Qualification inventory

`read`: fixture `crates/rastok/tests/conformance_file.rs`. Cases without a portable forced fixture: `PermissionDenied`, `AtomicReplaceUnavailable`, `IoFailed`, `TimeoutExpired`, `OutcomeUnknown`. Related tests: `crates/rastok/tests/file_connector.rs`.

`replace`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/tests/file_connector.rs`.

## HttpClient

`std/http-client@3`. One HTTP request per Call. Usable response bodies include non-success statuses. Decoded 500–599 responses use `ServerError`; other final statuses use `Completed`. A body decoding fault takes precedence over status classification.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| No selector | outbound | call | `lab-call` | Not compiled |

### Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| all | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| all | `out` | output | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text`; explicit JSON response type admitted |
| all | `exit` | output | HttpClientExit, below |

| Mode | Output cardinality |
|---|---|
| all | `out`: exactly one on `Completed` or `ServerError`; exactly one final `exit` per Call |

### Exit union

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

### Settings

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

`codec = json` permits an explicit `output out: Response`. `json_profile = projected` additionally skips extra members and fills absent nullable fields, while rejecting malformed required values. `auth_bearer = secret(...)` and `url = env(...)` use the declared boot-reference seam even though the table lists each setting's latest general phase. There is no automatic redirect or Retry: the graph declares its Retry policy.

### Qualification inventory

fixture `crates/rastok/tests/conformance_http_client.rs`. Cases without a portable forced fixture: `DnsFailed`, `OutcomeUnknown`. Related tests: `crates/rastok/src/connector/drivers/network/http_client.rs`.

## HttpServer

`std/http-server@3`. An inbound HTTP Call. `ack` sends 202 after admission; `respond` waits for the graph response. A malformed body or failed authentication can refuse before admission, without inventing a request envelope or an exit.

Selector: `mode` defaults to `ack`.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `ack` | inbound | call | `lab-http-server` | Not compiled |
| `respond` | inbound | call | `lab-http-server` | Not compiled |

### Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| ack, respond | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| ack, respond | `out` | output | Same as `in` |
| ack, respond | `exit` | output | HttpServerExit, below |

| Mode | Output cardinality |
|---|---|
| ack | One admitted request `out`; exactly one request `exit`; no graph response |
| respond | One admitted request `out`; exactly one request `exit`; at most one response `in` |

### Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `Completed` | `{ status: int32 }` | Not an outbound Call |
| `OutcomeUnknown` | Empty | Not an outbound Call |
| `Overloaded` | Empty | Not an outbound Call |
| `PeerDisconnected` | Empty | Not an outbound Call |
| `RequestExpired` | Empty | Not an outbound Call |

### Settings

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

### Qualification inventory

`ack`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/http_server.rs`, `crates/rastok/tests/lab_http_source.rs`.

`respond`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/http_server.rs`, `crates/rastok/tests/lab_http_source.rs`.

## Process

`std/process@6`. `job` starts one child per Call and publishes its ordered frames with the exit. `service` opens a resident child per Node instance on first input and reuses it. One exit ends that child, not each application request.

Selector: `mode` defaults to `job`.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| `job` | outbound | call | `lab-call` | `process-job` |
| `service` | inbound+outbound | session | `lab-process-service` | `process-service` |

### Ports and completion

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

### Exit union

| Case | Payload | Call visibility |
|---|---|---|
| `CaptureFailed` | Empty | outcome_unknown |
| `DecodeFailed` | `{ line: text?, message: text }` | outcome_unknown |
| `Exited` | `{ exit_code: int32, signal: int32? }` | after_effect |
| `OutcomeUnknown` | Empty | outcome_unknown |
| `OutputOverflow` | `{ limit: uint64, reason: enum { budget, capacity } }` | outcome_unknown |
| `SpawnFailed` | `{ message: text }` | before_effect |
| `TimeoutExpired` | Empty | outcome_unknown |

### Settings

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

### Qualification inventory

`job`: fixture `crates/rastok/tests/conformance_process.rs`. Cases without a portable forced fixture: `CaptureFailed`, `OutcomeUnknown`. Related tests: `crates/rastok/tests/process_service.rs`, `crates/rastok/src/connector/drivers/process/mod.rs`.

`service`: No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/tests/process_service.rs`, `crates/rastok/src/connector/drivers/process/mod.rs`.

## WebSocketClient

`std/websocket-client@3`. Opens a peer Session. Each complete message is one value; each graph write is journaled before I/O. One terminal ends the Session, not a message.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| No selector | inbound+outbound | session | `lab-websocket` | Not compiled |

### Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| all | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| all | `out` | output | Same as `in` |
| all | `exit` | output | WebSocketExit, below |

| Mode | Output cardinality |
|---|---|
| all | Zero or more frames and writes; exactly one `exit` per Session |

### Exit union

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

### Settings

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

### Qualification inventory

No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/websocket.rs`.

## WebSocketServer

`std/websocket-server@3`. Accepts peer Sessions. Replies follow session lineage or an explicit `correlate`. A pre-admission handshake refusal creates no Session or exit.

| Mode | Driver direction | Exchange | Lab model | Installed in this binary |
|---|---|---|---|---|
| No selector | inbound+outbound | session | `lab-websocket` | Not compiled |

### Ports and completion

| Mode | Port | Direction | Type rule |
|---|---|---|---|
| all | `in` | input | `codec = bytes` → `bytes`; `codec = json` → `variant`; `codec = text` → `text` |
| all | `out` | output | Same as `in` |
| all | `exit` | output | WebSocketExit, below |

| Mode | Output cardinality |
|---|---|
| all | Zero or more frames and writes; exactly one `exit` per Session |

### Exit union

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

### Settings

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

### Qualification inventory

No dedicated full-Contract fixture is declared. Related tests: `crates/rastok/src/connector/drivers/network/websocket.rs`.

## Specified, not shipped

| Item | Specification |
|---|---|
| File `watch`: listed as unavailable by the checker, with no ports or implementation | `docs/spec/stdlib.md` §3.7 |
| Timer: elapsed-time source | `docs/spec/stdlib.md` §3.4 |
| Calendar: civil schedule source | `docs/spec/stdlib.md` §3.5 |
| Memory: bounded append/snapshot store | `docs/spec/stdlib.md` §3.6 |
| HttpServer `respond` source scripting in the Lab | `docs/spec/simulation-semantics.md` §2 |
| Streaming HttpServer responses and seeded behavior | `docs/spec/stdlib.md` §8 |

Fixture names describe the evidence corpus, not checks performed by the
catalogue command. Source authority: `docs/spec/stdlib.md`, resolved
standard definitions and the current binary's catalogue. For authoring,
see [Connectors and outcomes](../guide/03-connectors-and-outcomes.md).
