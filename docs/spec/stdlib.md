# Standard library contracts

> **Status (2026-10-06): normative target, package v20.** Process job uses `@4`,
> Process service `@5`, and File/HTTP/WebSocket use `@3`. Their exact roles,
> cardinality, terminal visibility and idempotence profiles are independently
> checked. [C4 qualification](../architecture/retry-c4-qualification.md) records
> the single cutover from the earlier Contracts. HttpClient and
> HttpServer are live over HTTP/1.1–3, including bounded `ack` and `respond`;
> WebSocketClient and WebSocketServer are live over HTTP/1.1 Upgrade and
> HTTP/2–3 Extended CONNECT. M1.8 A4 adds versioned boot binding for
> HttpClient URL and bearer settings without changing the contract id.
> Completions carry ordered ordinary outputs and one final typed `exit`.
> Exact resolved port tables are independently checked in the package.
> Companion to [execution-model.md](execution-model.md) (`timeout` /
> `on_timeout`, the correlator duty), [delivery-semantics.md](delivery-semantics.md)
> (INGRESS / EGRESS records) and [simulation-semantics.md](simulation-semantics.md)
> (mocks, the coverage checklist these contracts feed).
>
> Guest-language placement follows the Operator/Connector boundary: a pure
> `GuestProgram` executes inside an Operator; a guest program that performs
> I/O executes behind a Connector.

The question this doc answers: **what exactly does each stdlib citizen
promise — so that the compiler, the simulator, the coverage checklist and the
future runtime all read the same contract?**


## 1. The Contract of a Connector

Every Connector has one Contract specified by the same template. No case is
optional — a Connector whose contract leaves a case blank is not spec'd:

Every admitted sheet also has one canonical versioned `ContractId` and a
closed required Driver-role fact. The source-facing contract name is ergonomic
syntax; the id and effective roles are the semantic facts transported by the
plan. Rust names this value `ContractId`. Physical crates, features, libraries, Hosts and Workers are never
contract identity.

| # | Case | Why it is mandatory |
|---|---|---|
| 1 | **Role** | what it bridges (a protocol, a process, time) |
| 2 | **Direction** | inbound source / outbound caller / both / sink |
| 3 | **Ports** | `in`/`out`/`err`/`exit` conventions + extras, with types |
| 4 | **Repetition guarantee** | plan-owned `idempotent` where the Call profile admits it; absent/false supplies no guarantee. Session opening follows roles, not an effect sentinel |
| 5 | **`behavior`** | deterministic / probabilistic — §2; pre-declared or per-instance |
| 6 | **Attributes** | the config surface with a contract-declared latest evaluation site (plan, boot, instance, or invocation); checking chooses the earliest legal actual site and transports it; `env()`/`secret()` are boot-only, and timeout/shutdown settings belong only to the profiles that declare them |
| 7 | **Correlator** | what one invocation or session is in its protocol, and how messages/answers find it (execution-model §6.1: the Connector matches) |
| 8 | **Outcome coverage** | the finite typed cases each output may emit and the completion rule the future `rastok simulate --coverage` enumerates |
| 9 | **Instances** | singleton / `instance.key` (each resource run may have a `run_id`) / per-invocation or per-session; Node-instance closing policy and each exchange's terminal exit |
| 10 | **Journal & sim** | which records it writes (INGRESS w/ `inbound_seq`; EGRESS open-closed); how it is mocked/injected |

Case 8 deserves the emphasis: coverage must enumerate the cases a
Connector can emit, but they are described by the ordinary types of its
output ports, not by a parallel `error_set`. Runtime-produced cases such
as timeout or unknown outcome must appear in the resolved port type just
like Driver-produced cases. A boundary completion is a separate control
fact: it closes one obligation and may carry several typed port outputs.
`--coverage` counts the cases and the completion paths.

The lifecycle scopes do not collapse into one another. A static Connector
Node may have several logical instances; each Node instance may admit several
events, invocations or sessions. An event is one spontaneous inbound root and
has no per-event terminal obligation. `stop` closes the Node instance, while
the conventional typed `exit` port closes one admitted invocation or session.
One instance can therefore observe many exits before it stops. Instance
closure refuses new admissions and later drains or cancels every open
invocation or session under a bounded policy; it never replaces their
individual terminal exits. An invocation correlates zero or more output
frames with one `exit`; a session correlates messages in both directions with
one session `exit`, never one exit per message (D-2026-08-25-1).

**Naming rule**: stdlib Contract names carry **no suffix** — the keyword already says
it (`connector c: Timer`). Applies to all future contract types too (`Postgres`,
`S3`, …).


**The contract is also the assurance boundary**: a future
`theorem` proves a graph **under** the Connector contracts declared here;
an `experiment` (simulation-semantics.md §10) checks empirically
that a real Driver and its external system honor them. Neither layer replaces the
other.

### 1.1 Application profile

Executable Contracts carry two mandatory kinds of evidence:

| Case | Required material |
|---|---|
| Application/cardinality | exchange profile, opening inputs, finite completion rule, per-port `Never | ZeroOrOne | ExactlyOne | ZeroOrMore | OneOrMore`, and one closed terminal fact where applicable |
| Repeat visibility | every terminal case classified `before_effect | outcome_unknown | after_effect` |

These facts are Registry-free validation material, productibility evidence
and Loop/Retry host qualification. The standard namespace is `std/`;
external namespaces must contain a dot. Current ids are
`std/process@4` (job), `std/process@5` (service), `std/file@3`,
`std/http-client@3`, `std/http-server@3`, `std/websocket-client@3`
and `std/websocket-server@3`. Only these exact Contracts execute in M1.
Source names such as `Process` remain unchanged.

## 2. `behavior` — determinism, separate from idempotence

The world's *timing* nondeterminism is already journal-absorbed
(journal-relative replay). `behavior` declares something else: **the
external system's semantic function** —

```
   same request, same world state  →  same answer ?

   GET /users/42                   →  yes            behavior = deterministic
   codex « think about … »         →  NO, by design  behavior = probabilistic
                                       (a distribution is sampled)
```

- **Pre-declared** where the Connector's Contract imposes it: `Timer`, `Calendar`,
  `Memory`, `HttpServer` = `deterministic`.
- **Per-instance, mandatory** on `HttpClient` and `Process` (only the dev
  knows who answers). Forgetting a required `behavior` = compile refusal. An ordinary Call needs no
  global `effect` declaration; the removed attribute refuses.
- **`seeded` = reserved, not spec'd** (deterministic given a recorded seed —
  LLMs at fixed temperature+seed, PRNGs).

**The derivation rule — the onion becomes computable**:

```
   Operators        : deterministic BY CONSTRUCTION (never declared)
   Pure guests (jq/sql) : deterministic BY CONTRACT (§5, clause ①)
   Connectors       : declared

   ⇒  behavior(graph) = deterministic  IFF  all its Connectors are

   the compiler can print :
   « probabilistic surface of the app = { codex } — everything else is provable »
```

Replay does not need this attribute (the journal records sampled answers like
any others); **composition, audit and cost do**: the probabilistic surface
is the token-spend surface (§6.4), the MCP tool annotation derives from it
(§7), and hardening has a metric — replacing an LLM call with an Operator
*visibly shrinks* the probabilistic surface.


## 3. The nine contracts

### HTTP field and media-type vocabulary

All four HTTP-derived sheets expose the same two configuration attributes:

```rastok
headers = {
  content_type = media_type("application/json")
}
custom_headers = {
  "X-Trace-Id": ["one"]
}
```

`headers` is a sparse typed struct. Its current field is
`content_type: media_type`; future well-known fields join only with their own
logical type. `custom_headers` is `map<text,array<text>>`, preserving an
author's conventional HTTP casing at the source boundary and explicit field
line order. Before an external open, names normalize to lowercase and values
become the lossless byte-header descriptor. Case-insensitive collisions and
attempts to override Driver-owned fields (`Host`, `Content-Length`,
authorization/idempotency machinery, hop-by-hop or WebSocket protocol fields,
and typed `Content-Type`) refuse. `Accept` remains custom until Rastok owns a
distinct media-range/quality type.

`media_type` is open. Tooling offers a completion catalogue for common
`application/*`, `text/*` and image values, but any canonical concrete media
type is admitted; the catalogue is not an enum and never gates compilation.
There is no parallel `mime_type` spelling.

### 3.1 `HttpServer` — inbound HTTP, two modes

| Case | Contract |
|---|---|
| Stable identity | `std/http-server@3` (exact Contract) |
| Role / Direction | HTTP endpoint; **inbound** Driver role |
| Exchange | call — one admitted request, at most one protocol response, one typed request-lifecycle `exit` |
| Ports | `out: T` decoded request; `in: T` graph response in `respond`; `exit: HttpServerExit`; no implicit `err` |
| Payload type | required `codec`: `json → variant`, `text → text`, `bytes → bytes`; one complete body is one value |
| Populates | canonical `meta.ext.content_type: media_type`, lossless `http_headers`, `http_method`, `http_uri`, `http_version`; a response carries `http_status` when applicable |
| behavior | `deterministic` (pre-declared); no repetition guarantee for this inbound/event profile |
| Attributes | common: `listen: text` required (instance), `path: text = "/"` (plan), required `codec` (plan), `max_body: ByteSize = "8MiB"` (plan), optional typed `headers` and open `custom_headers` plus `auth_bearer: text` (instance), optional paired `tls_certificate`/`tls_private_key: bytes` (instance), ordered `versions = ["http/3", "http/2", "http/1.1"]` (plan), `zero_rtt = disabled` (plan); `mode = ack` default, while `respond` adds optional invocation `correlate` and required positive instance `timeout` |
| Correlator | mode `respond`: **default = lineage** (`origin_id == R.id`) — valid on join-free return paths ONLY. **A return path crossing a root-mixing join ⇒ explicit `correlate` required (compile error otherwise)**: the join output inherits the TRIGGER's origin, creating a race. The author carries the token (`meta.app.request_id = meta.sys.id`) and attributes it through joins with the explicit-attribution rule; matching = correlate-value == the open request's root id. An orphan response (no open request — it already expired, the 504 went out) is **discarded-and-journaled** (the call-closed rule's cousin). |
| Outcome cases | `Completed { status: int }`, `RequestExpired`, `Overloaded`, `PeerDisconnected`, `OutcomeUnknown` |
| Application profile | inbound Call, therefore not a Loop host; each admitted request produces exactly one decoded `out`, accepts zero or one response `in` according to mode, and produces exactly one terminal `exit`; a pre-admission refusal produces none of these |
| Instances | singleton |
| Journal / sim | two-phase INGRESS proposal; commit precedes 202/response; Lab `listener.out = value` scripts a decoded request and successful fixed ACK, without a socket; source `respond` scripting remains deferred |

- **`ack`** (webhook): 202 immediately after the accepting commit —
  "received", not "processed";
  replies to the user travel through an outbound Connector.
- **`respond`** (API): the request stays open; correlation by lineage;
  timeout → 504; bounded → 503. Exposing an API is first-class.
- The async HTTP library is the **runtime's** choice — the contract is
  lib-agnostic. The H4 hosted HttpClient uses Hyper for HTTP/1.1-2 and quiche
  for HTTP/3 behind one bounded private pump. H5.1's hosted HttpServer uses
  Hyper for HTTP/1.1-2 `ack` on that same pump shape: connection, stream,
  request, fact and byte capacities are finite; request bytes remain charged
  while the proposal awaits causal admission; and 202 follows the accepted
  commit. These are Driver choices and are absent from plan identity. H5 now
  executes the same bounded protocol over HTTP/3 and executes `respond`: the
  producing transaction seals the exact opening instance, Driver saturation
  leaves the real response pending without bytes, and `l:reply` consumes its
  cursor. Deadline kind 7 (`InboundCallTimeout`) makes the positive request deadline part of the
  accepting fact; response and 504 race through one terminal protocol action.

Malformed framing, failed bearer authentication and body refusal can occur
before a request has a typed `out` value. H5 returns an HTTP refusal without
minting a root or fake Call. Such pre-admission refusals are deliberately
absent from `HttpServerExit`; an `l:reply` is legal only for an admitted
request.

### 3.2 `HttpClient` — outbound HTTP

> **Current @3 Contract:** the amendment to
> [D-2026-09-14-4](decisions.md#pre-c1-review-amendments--14-september)
> separates usable 500–599 responses as `ServerError { status }`; other
> admitted final statuses remain `Completed { status }`. Both produce `out`
> and are `after_effect`. The table below describes `http-client@3`;
> [C1](../architecture/retry-c1-identity.md) inventories the version,
> cardinality, validation and golden changes. D-2026-09-16-1 also admits
> `output out: Response` under `codec = json`, using the existing port table.
> The body is hydrated by field name before retention; a mismatching actual
> API response is `DecodeFailed`, not a partially usable value. See
> [the exact M1 schema profile](types.md#41-explicit-json-response-types-in-m1).
> D-2026-09-17-1 additionally defines an opt-in `json_profile = projected` for
> optional/additional JSON members, executable in the `@3` Contract.
> It requires `codec = json` and a declared struct output. Omission keeps strict
> hydration; it adds no default attribute, port or request field.
> See its [normalization and qualification contract](../architecture/openai-responses-hydration.md#4-limited-projected-profile).

| Case | Contract |
|---|---|
| Stable identity | `std/http-client@3` (exact Contract) |
| Role / Direction | HTTP calls; **outbound caller** |
| Exchange | call — one request, response outputs, one typed call `exit` |
| Ports | `in: T` request body, `out: T` every usable response body including non-2xx, `exit: HttpClientExit`; no implicit `err` |
| Payload type | required `codec`: `json → variant`, `text → text`, `bytes → bytes`; no line delimiter is added |
| Populates | response canonical `meta.ext.content_type: media_type` when valid, lossless `http_headers`, `http_status: int`, and `http_version` |
| idempotent / behavior | `idempotent: bool = false`; `behavior` required; both plan-evaluated |
| Attributes | required invocation `url: text` (M1.8 A4 additionally admits the exact boot reference `env("NAME")`); invocation `method: text = "GET"`, optional typed `headers`, open `custom_headers`, `auth_bearer` (A4 admits only `secret("logical/path")` at boot), `idempotency_key`; required plan `codec`; plan `max_body: ByteSize = "8MiB"` bounds request and response bodies; required positive instance `timeout`; optional instance `tls_ca`, plus paired `tls_certificate`/`tls_private_key`; ordered plan `versions = ["http/3", "http/2", "http/1.1"]`; `zero_rtt = disabled` |
| Correlator | protocol-native: HTTP request ↔ response |
| Outcome cases | `Completed { status: int }`, `ServerError { status: int }`, `DnsFailed`, `ConnectFailed`, `TlsFailed`, `ProtocolFailed`, `TimeoutExpired`, `BodyTooLarge { limit: uint }`, `DecodeFailed`, `OutcomeUnknown` |
| Application profile | outbound finite Call and Loop-admissible; exactly one request `in`, exactly one `out` iff the exit is `Completed` or `ServerError`, then exactly one terminal `exit`; terminal visibility is the conservative [C1 table](../architecture/retry-c1-algebra.md) |
| Instances | singleton by default; the common Node attribute `instance.key` may key the Connector |
| Journal / sim | method, resolved URL, canonical ordinary headers, exact body, idempotency material and a logical bearer `SecretName` seal before EGRESS open; physical bearer bytes are fixed by the run header and overlaid only in the real Driver; the contract-specific Lab twin scripts only the post-open outcome |

The hosted H4 Driver attempts declared versions in order, skipping backends
not compiled into the runtime. It may fall back only before request bytes can
have become visible. Once visible, failure is terminal for that call; redirects remain outside the Driver, and retries follow the validated static
`Retry.on` policy when that Extension is present. Local work/byte saturation leaves the
open call pending and starts no effect. HTTP/1.1 uses origin-form plus a
Driver-owned Host field, HTTP/2 multiplexes by origin/TLS identity, and HTTP/3
requires verified HTTPS. Every valid status, including non-2xx, publishes its
body on `out`; the wire version is observation in `meta.ext.http_version`, not
payload or identity.

A4's boot references are exact roots, not general evaluator calls. A profile
must supply the URL value and resolve the versioned bearer before Driver
binding. The logical request seen by real and Lab Drivers contains the secret
reference only. The real Driver may validate the bearer before opening an
effect and may put it on the wire only after the EGRESS record; neither
material nor its command-line spelling exists in graph or causal artifacts.

For hosted clients, absent `tls_ca` selects Rastok's compiled exact Mozilla
root corpus. Present `tls_ca` replaces it with a PEM certificate bundle or one
DER certificate. `tls_certificate` accepts a PEM chain or one DER leaf, and
`tls_private_key` accepts an unencrypted PEM PKCS#8/PKCS#1/SEC1 key or an
auto-detected unencrypted DER key. Both H1-H2's Rustls stack and H3's
BoringSSL stack consume these same bytes and validate a certificate/key pair
before socket I/O. A deployment may select another Driver, but it may not
weaken verification or reinterpret the plan bytes.

### 3.3 `Process` — child processes, two modes

| Case | Contract |
|---|---|
| Stable identity | `job`: `std/process@4`; `service`: `std/process@5`; both use package v20 |
| Role / Direction | Job: outbound Call. Service: both roles, resident Session. Selecting a profile resolves its exact Contract before lowering. |
| Ports | `in` → stdin; `out` ← decoded/framed stdout; `err` ← framed stderr data; `exit` ← `ProcessExit` |
| idempotent / behavior | job: `idempotent: bool = false`; service has no idempotence attribute. `behavior` required for both profiles, plan-evaluated |
| Attributes | the normative table below (D-2026-08-12-1, amended by D-2026-08-14-1) — every attribute is `name = value`, presence and defaults are the CONTRACT's, and the checker injects every omitted default into the effective HIR |
| Correlation | Job: the Call itself. Service: the exact Node instance selects its resource Session; application request/reply correlation belongs to Operators. An upstream socket's SessionId cannot select a child process. |
| Outcome cases | `ProcessExit` below; exactly one terminal per job invocation or resident child incarnation, never per service message. stderr never carries lifecycle data. |
| Application profile | `job`: one request, ordered frames, one atomic terminal completion; Loop-admissible under [R1](../architecture/rework-r1-host-contracts.md#71-contracts-for-loop-admissible-connectors). `service`: first input opens a child, subsequent inputs reuse it, inbound frames stream while it lives; not a Loop host. |
| Instances | see modes below |
| Journal / sim | EGRESS records per call (job: per run); spontaneous emissions = INGRESS roots; mocked in scenarios |

**The attribute table (D-2026-08-12-1).** `mode` is the PROFILE
selector; its omission selects `job` and the checker materializes
`mode = job` into the effective HIR — omission and the exact
spelling of a default are one identity (hash included). The plan
carries the EFFECTIVE attributes; each contract row declares the latest
permitted site, checking chooses the earliest site where the expression
closes, and the Driver consumes resolved values or an explicitly transported
boot/instance/invocation expression and never reintroduces a default. M1.4 R1
now carries this ceiling in the executable sheet and rejects a non-plan port
type selector. Plan-resolved `cwd` and `env` already bind through the real
Driver; transport of their boot/instance expressions remains the categorical
R2 step, never a Driver fallback.

| Attribute | Profile | Domain | Latest evaluation site | Presence |
|---|---|---|---|---|
| `command` | common | Rastok expression → `text` | instance | **Required** |
| `args` | common | Rastok expression → `array<text>` | instance | Defaulted(`[]`) — omission ≡ `[]`; `null` refuses (no domain is implicitly nullable) |
| `cwd` | common | Rastok expression → `text` | instance | Optional |
| `codec` | common | choice `lines \| jsonl \| bytes` for `in` and `out` | plan | **Required** — request/reply framing is never chosen silently (D-2026-08-11-1) |
| `err_codec` | common | choice `lines \| bytes` for stderr only | plan | Defaulted(`lines`) — diagnostics never inherit `jsonl` from `codec`; omission ≡ `lines` |
| `idempotent` | job only | bool | plan | `false`, no guarantee |
| `behavior` | common | choice, typed | plan | **Required** |
| `timeout` | job | Rastok expression → `duration` | instance | Optional — a true optional: no budget declared, no budget; refused on service |
| `env` | common | `map<text,text>` | instance | Optional; canonical spelling `env = { "K": v }`; written entries override inherited host entries and unspecified entries remain inherited |
| `max_output` | job | byte size: `<digits><unit>`, unit ∈ `B \| KiB \| MiB \| GiB` (explicit, required), ASCII whitespace tolerated around and between, ≤ 4 GiB (syntax bound; the Driver binding additionally proves the budget is REPRESENTABLE on this host — at most `isize::MAX` — while actual allocation is judged chunk by chunk at intake via `try_reserve`, surfacing as the typed overflow, never an abort) — one parser (`contract::byte_size`) judges at check AND enforces at run | plan | Defaulted(`"8MiB"`) |
| `on_timeout` | job | choice `kill` (one word — "fixed to `kill`") | plan | Defaulted(`kill`) |
| `max_frame` | service | positive byte size, same parser and host-representability check as `max_output`; bytes excluding newline, per stdin/stdout/stderr frame | plan | Defaulted(`"1MiB"`) |

Service restricts `codec` to `lines | jsonl` and `err_codec` to `lines`.
Raw `bytes` has no resident-frame delimiter and refuses. A service stdin write
contains one frame: an embedded newline refuses before any write. `correlate`
and `restart` are retired, not executable attributes. Request ids belong to
Operators; fresh input may open a child after death, but never resends old
writes. Service has no per-turn timeout: an application can express its own
bounded rendezvous; instance lifetime and Closing bound resource lifetime.

`ProcessExit` is an ordinary tagged union on the ordinary `exit`
port:

```rastok
type ProcessExit = union {
  Exited         { exit_code: int32, signal: int32? }
  SpawnFailed    { message: text }
  TimeoutExpired
  DecodeFailed   { message: text, line: text? }
  CaptureFailed
  OutcomeUnknown
  OutputOverflow { limit: uint64, reason: enum { budget, capacity } }
}
```

For a job, one boundary completion carries exactly one terminal
`ProcessExit` and zero or more ordered `out` and `err` emissions. `err`
contains only stderr data observed for that run under its resolved framing;
it never carries an invented lifecycle failure. A non-zero exit is still
`Exited`, not an automatically classified failure. A graph narrows/tests
the exit case and decides whether to retry, alert, discard or continue.
`run_id`, call identity and causal lineage remain runtime
identity/provenance rather than fields invented inside stderr or every exit
payload.

`OutcomeUnknown` is empty and closed. It is emitted only by supervision when
no terminal external outcome can be established: recovery triage of an open
EGRESS record, or expiry of an instance's explicit Closing settlement budget. In
both cases the EGRESS closes `abandoned`, the envelope uses `l:recover`, and
causal provenance explains the reason instead of a racing string payload. A
confirmed close-time kill remains the observed `Exited` result. A late Driver
answer after abandonment is a discarded observation and cannot create a
second exit.

Framing and publication are orthogonal. `codec` frames stdin/stdout;
`err_codec` frames stderr independently. `lines` yields one `text` value per
line, `jsonl` yields one `variant` per JSON value, and `bytes` yields one
bounded byte value for the captured channel. The M1.3 job implementation
keeps bounded on-exit publication: all ordered frame values and the one
`exit` are committed as one completion, so a retry cannot expose a partial
attempt. **On-exit does not aggregate frames**: line-framed stderr may yield
two hundred `err` envelopes in that transaction. An explicit bounded
collector may instead aggregate them. The resident Session profile
publishes frames while the resource lives: it preserves Session identity,
emits no frame after that resource's `exit`, and keeps intake/backpressure
bounded. It does not change job completion or introduce per-message Calls.

The frame boundaries are exact. For `lines`, a request is one `text` value;
the Driver appends one `\n` when absent. On output, `\n` is removed from
each UTF-8 line, an empty line is the empty text value, a final non-empty
unterminated line is accepted at EOF, and an empty channel emits nothing.
For `jsonl`, the same newline boundaries contain JSON values; an empty line
refuses. For `bytes`, request bytes pass unchanged and a non-empty captured
channel produces one value. A framing or UTF-8 failure yields `DecodeFailed`;
no partial frame is emitted.

**`mode = job` (default — the Unix one-shot; systemd `Type=oneshot`, k8s Job
lineage)**: one envelope = one spawn. stdin = the request (codec-framed);
when stdout decodes successfully the completion carries zero or more `out`
frames: `codec = lines` → one `text` per line; `codec = jsonl` → one
`variant` per decoded line; `codec = bytes` → one bounded `bytes` value
when non-empty. Independently, `err_codec = lines` yields one `text` per
stderr line and `err_codec = bytes` yields one bounded `bytes` value when
non-empty. The completion always carries one `exit`. The CAPTURE is
bounded: **`max_output = 8MiB` (default)** is the AGGREGATE budget
across stdout and stderr (D-2026-08-12-1, review round) — the total
is judged before any byte is stored, so neither stream ever loses
bytes silently; crossing it → exit
`OutputOverflow { limit, reason }` and the run is killed. `reason`
distinguishes `budget` (the declared cap) from `capacity` (the
host's capture allocation refused — judged chunk by chunk via
`try_reserve`; the pump's own bounded 8 KiB chunk copies remain
ordinary allocations). The payload carries NO stream and NO exact
size: the two capture pumps have no total order, and the journal
records no thread race. A pipe READ error is its own fact — the
completion reports `CaptureFailed` (closed, empty exit payload: the OS
detail and the failing stream would race between two pumps),
truncated bytes never presented as the whole. Every observed exit emits
`Exited { exit_code, signal }`, including exit ≠ 0. The **process boundary IS the correlation** — `correlate` is a
compile error here. `timeout` = the whole-run budget; `on_timeout = kill`
**forced** (bypassing a detached run is meaningless). `restart` is likewise
a compile error on a job (re-running a possibly-irreversible one-shot is a
graph decision — a Loop — never a supervisor reflex). `instance.key` still
orders one customer's conversions serially — it shares nothing. The safe
default: zero hidden shared state.

**The `jsonl` wire (D-2026-08-11-1).** One JSON value per `\n`-framed
line, both directions, the JSON model exactly (`variant`).
Outbound, the request value serializes CANONICALLY — these bytes are
`request_digest` material, so the writer is part of identity: object
keys sorted by their UTF-8 bytes (structs and maps alike — the
wire's own canonical order; the envelope seal orders map entries by
tagged CBOR bytes, which differs: `{"b", "aa"}` seals `b` first and
writes `aa` first),
numbers exact (`int` as the integer, `decimal` in plain decimal
notation at its sealed scale, `double` in shortest round-trip form),
strings JSON-escaped, one trailing `\n`. Unserializable requests
refuse as `DecodeFailed` before any open fact: `bytes` (no JSON
form — `lines` or a future `bytes` codec carry those), a
non-finite `double`, and any ENCODED payload (`jsonl` frames the
JSON model, not opaque representations). Inbound, each line decodes
EAGERLY through the house JSON reader (D-2026-08-10-4/-7): exact
numbers, duplicate keys refused; a line that does not parse — an
interior empty line included — is `DecodeFailed` with the offending
line in the payload; the single trailing newline's empty tail is
framing, not a line; empty stdout emits no `out`, while a LONE `\n`
is one empty line and refuses (the framing is judged before the
terminator is stripped). A line's padding is JSON's own whitespace
alphabet (space, tab, CR) and nothing wider; `null` is a value like
any other and decodes to one null-valued envelope. The reply's real
pin is `Process(job, jsonl).out = variant`; multiplicity belongs to
the ordered completion, not to an artificial `array<variant>` wrapper.
The checker and machine judge every emitted frame against that pin, so
Lab and world see one shape.

**`mode = service` (resident Session, D-2026-09-14-1).**
D-2026-09-10-2 settles the responsibility boundary: Process is a generic
duplex stdio resource. With `codec = jsonl`, a complete stdout JSON line
becomes one `variant`; an Operator validates/narrows that value and interprets
the application protocol. Syntax decoding belongs to Process; response ids,
notifications, progress, business errors and turn completion belong to the
graph. Valid JSON with an unexpected domain shape is not a Process framing
failure. No hidden RPC table classifies stdout or matches it to the most
recent stdin message. `correlate` is not part of this profile.

The profile uses D-2026-08-25-1's generic `Session` exchange: the first input
lazily opens a child for the exact Node instance; later inputs write to that
live child. Several turns share one resource. stdout/stderr frames and the
single resource `ProcessExit` carry its Session context. Local write settlement
does not complete a turn, and `exit` ends the child resource, not one request.
There is no fake Call or Process job per line, nor a WebSocket-specific runtime
path. A websocket SessionId is not the resident child's SessionId.

D-2026-09-10-3 fixes fresh opening after child death: retain the resource's
typed `exit`, do not respawn automatically, and leave the Node instance alive
unless its own lifecycle closes it. A new eligible input can open a fresh
child in that still-open instance, with a new `SessionId` and no change to
the instance generation. Earlier writes are never replayed or retargeted;
the child's private memory is not restored. Closing or stopped generations
cannot be revived by this rule. Any successor follows ordinary Node-instance
admission, independently of resource reopening.

D-2026-09-10-4 separates application shutdown from instance closure. A protocol
message such as `quit` is ordinary input, not a word interpreted by Process.
Its write settlement proves neither application completion nor resource exit.
If the child exits, Process publishes its terminal without implicitly stopping
the Node instance. `instance.stop` separately bounds resource closure after
admitted work drains. The resident Unix Driver's system fallback distinguishes
`SIGTERM` from forced `SIGKILL`; EOF is not a universal shutdown request.
At the drain deadline the service requests `SIGTERM`; at the settlement
deadline it requests forced cleanup, with `OutcomeUnknown` unless a terminal
was observed. There is no third grace period. Signal handoff runs on the OS
worker, never the semantic path. Existing job cancellation is unchanged.

The Unix Driver retains at most 64 resources per bound Driver, eight queued
writes per resource, bounded eight-slot reader/publication queues and a
reserved shutdown control slot. Full intake applies backpressure; it never
drops a frame. Frames already admitted before a decode/overflow failure stay
visible, followed by one typed exit. The same seven exit cases are retained;
`TimeoutExpired` is not synthesized by a per-turn service timer. A failed
spawn yields `SpawnFailed`, pipe failure `CaptureFailed`, invalid JSON/UTF-8
`DecodeFailed`, and an exceeded frame bound `OutputOverflow`.

The [implementation ledger](../architecture/process-service.md) links tests
and the [resident dialogue](../../examples/agent-dialogue.rkg). The Lab runs
`match` mocks without any physical Driver: each input may yield frames and an
optional final exit. An authored `quit` response can terminate the mock
resource; silence during system close yields `OutcomeUnknown`. Delayed or
unsolicited scripted frames and system-close acknowledgements remain named
Lab limits, as does `now()` in Session mocks. Physical unsolicited stdout is
supported. Automatic restart, protocol-specific resume, Process credentials,
and non-Unix physical supervision are not claimed.

### 3.4 `Timer` — one-shot or periodic time events

| Case | Contract |
|---|---|
| Exchange | event — each firing is one spontaneous inbound root and has no per-event `exit` |
| Role / Direction | one-shot or periodic logical-time event; **inbound source** |
| Ports | `out`: timestamp envelopes |
| behavior | `deterministic` (pre-declared); no repetition guarantee for this inbound/event profile |
| Attributes | exactly one of `after = duration("…")` (one firing) or `every = duration("…")` (periodic); positive plan constants |
| Correlator | **none** (a pure source makes no calls) |
| Outcome cases | none (a timer event has no runtime outcome worth another envelope) |
| Instances | one source per declared Node instance; no implicit `system_clock` Node exists |
| Journal / sim | firings = INGRESS records; in simulation, driven by `advance` |

`Timer` is not the VM's clock and is not required for `now()`, timeouts,
lifetimes, join eviction or Loop delay. Those mechanisms consume the explicit
logical timeline and generic dated-event queue directly. Declare a `Timer`
only when the graph itself needs an event because time passed. An embedded
host may drive both from a monotonic hardware timer; an RTC or another
civil-time source is needed only to anchor `Calendar` semantics.

### 3.5 `Calendar` — civil time

| Case | Contract |
|---|---|
| Exchange | event — each civil instant is one spontaneous inbound root and has no per-event `exit` |
| Role / Direction | calendar instants; **inbound source** |
| Ports | `out`: `data : struct { scheduled_at: timestamp }` |
| behavior | `deterministic` (pre-declared); no repetition guarantee for this inbound/event profile |
| Attributes | **either** `cron = <expr>` + `tz = <IANA zone>` (mandatory together — never the host's implicit zone) **or** `at = timestamp("…")` (fires **once**; the zone lives in the RFC-3339 string) |
| Correlator | **none** (a pure source makes no calls) |
| Outcome cases | none (misconfiguration is compile-time: bad cron/tz = refusal) |
| Instances | singleton per declaration |
| Journal / sim | ticks = INGRESS records; a scenario using `Calendar` should declare its `origin` (simulation-semantics §5: virtual t=0 ≡ a civil instant; absent → epoch + **warning**) |

**DST policy, stated**: a nonexistent civil time (spring forward) is
**skipped**; an ambiguous one (fall back) fires **once, first occurrence**.
The classic sane-cron behavior, written down.

*(`Timer` schedules elapsed-time events; `Calendar` schedules civil instants.)*

### 3.6 `Memory` — the bounded in-process store

| Case | Contract |
|---|---|
| Role / Direction | bounded, ordered, in-process value store; **append and snapshot read** |
| Ports | `in: T` appends `data`; `read: struct { }` requests a snapshot; `out: array<T>` returns it; `exit: MemoryExit` terminates each admitted operation |
| behavior | `deterministic`; this future Memory sheet grants no admitted Retry profile |
| Attributes | `cap = <n>` (default 10 000 — bounded **by construction**), `overflow = evict_oldest (default, evictions journaled) \| reject` |
| Correlator | each delivery to `in` or `read` is one operation; a read also completes with an `out` snapshot, and every operation has exactly one ordinary typed exit |
| Outcome cases | every admitted append or read emits exactly one `MemoryExit`: `Completed` (with an `out` snapshot for a read), or `CapacityExceeded` when `overflow = reject` |
| Instances | one store per declared Node instance and therefore per canonical key when the declaration uses `instance.key` |
| Journal / sim | content is Node state, recovered by folding appends and evictions at the Node's effective durability scope; reads and their snapshots are journaled and deterministic |

`Memory` stores the incoming `data` value, not a magical copy of the whole
envelope. A read returns one fresh envelope whose `data` is an
`array<T>` ordered from oldest to newest. The snapshot is taken at the
`read` transaction's journal position: later appends are not included.
Reading does not consume entries.

Because `cap` bounds the store, every snapshot is bounded too. Filtering,
random access, destructive reads, and durable database queries are outside
this minimal contract. A graph can apply a pure Operator to the returned
array. A DLQ that needs replay metadata stores an explicit typed record
containing that metadata; `Memory` does not invent a hidden record shape.

`Memory.out` follows ordinary DAG edges. Reading it never grants permission
to wire a static back-edge to an upstream Node. A replay tool or a new graph
run can inject selected values as new roots.

### 3.7 `File` — one logical file per invocation

| Case | Contract |
|---|---|
| Stable identity | `std/file@3` (exact Contract) |
| Role / Direction | outbound one-file operation; `mode = read \| replace` is required; `watch` is a known deferred spelling whose inbound role requires a later contract-version decision |
| Ports | read: `in: variant`, one `out: bytes` on success, then `exit: FileExit`; replace: `in: bytes`, no success `out`, then `exit: FileExit` |
| idempotent / behavior | `true` / `deterministic` (pre-declared by the File Contract) |
| Common attributes | `path: text` Required, latest site **invocation**; `timeout: duration` Optional, latest site instance |
| Read attributes | `max_read: ByteSize = "8MiB"` (plan; portable maximum 4 GiB) |
| Outcome cases | `Completed`; `NotFound`; `NotAFile`; `PermissionDenied`; `InvalidPath`; `ReadLimitExceeded { limit: uint }`; `AtomicReplaceUnavailable`; `IoFailed`; scheduler `TimeoutExpired`; bounded-Closing `OutcomeUnknown` |
| Application profile | outbound finite Call and Loop-admissible: both profiles consume exactly one `in` and produce exactly one terminal `exit`; `read` produces exactly one `out` iff Completed, while `replace` can never produce `out`; terminal visibility is the conservative `@2` table in [R1](../architecture/rework-r1-host-contracts.md#71-contracts-for-loop-admissible-connectors) |
| Instances | ordinary Node instances; a dynamic `path` is evaluated for each invocation and is not instance addressing |
| Journal / sim | request preparation and typed rejection occur before EGRESS open; C4's real Driver executes only the sealed operation after open; replay never recalls it without recovery triage; C5's mock factory reuses the same pure File request binding and only replaces the post-open effect |

`path` is a logical UTF-8 path relative to the physical root supplied to the
bound Driver. The physical root is deployment data, not plan identity. `/` is
the only logical separator; `\` refuses rather than changing meaning across
hosts. Empty or native-absolute paths, NUL, empty segments, `.` and `..`, and
a spelling the host cannot represent yield `InvalidPath`; parents are never
created implicitly.
The path spelling is not case-folded, Unicode-normalized or canonicalized.
The root is a resolution base rather than a portable security sandbox: native
symlink behavior belongs to the external world or to a stricter deployment
Driver.

A read publishes the complete bytes or no `out` at all. Crossing `max_read`
returns `ReadLimitExceeded` with the declared limit. A replace stages in the
target directory and uses one native atomic replacement. If that visibility
guarantee is unavailable, `AtomicReplaceUnavailable` is terminal; there is no
truncate/in-place fallback. The contract does not promise crash durability or
preservation of permissions, ownership, xattrs or timestamps. Existing
directory segments are required, while the final target may be created.

The sealed request descriptor carries mode, exact logical path, and the read
bound or exact replacement bytes. It is prepared before EGRESS open and
executed without recomputation afterwards. Cancellation before the read or
rename commit point is silent at the Driver; scheduler timeout and bounded
Closing own the terminal fact. A completion that loses the close race is an
ordinary late observation, never a second graph event.

The exit union is deliberately portable and mostly empty. The outbound
intent already seals the operation and logical path; host error numbers,
unstable messages and repeated paths are observations, not causal payload.
Directory listing, recursion, removal and parent creation are not hidden File
modes. A later watch contract must separately settle snapshot, replacement,
deletion, overflow, resynchronization and acknowledgement semantics.

### 3.8 `WebSocketServer` — accepted duplex sessions

| Case | Contract |
|---|---|
| Stable identity | `std/websocket-server@3` (exact Contract) |
| Role / Direction | both roles over one shared bound listener/session table; accepts remote sessions, receives messages and writes graph replies |
| Exchange | session — many messages in either direction, exactly one terminal `WebSocketExit` per accepted session |
| Ports | `out: T` receives complete peer messages; `in: T` sends complete messages; `exit: WebSocketExit` terminates a session; no implicit `err` |
| Payload type | `codec = json` → `variant`; `text` → `text`; `bytes` → `bytes`; text/binary mismatch is a protocol refusal, never coercion |
| Populates | `meta.ext.session_id: text` on every `out` and `exit`, plus `http_headers`, `http_uri`, `http_version`, `websocket_origin`, and selected `websocket_subprotocol` from the admitted handshake |
| behavior | `deterministic` for admission; graph-to-peer effects remain journaled Session writes. No idempotence attribute |
| Attributes | required instance `listen: text` and positive `handshake_timeout: duration`; plan `path: text = "/"`, required `codec`, `max_message: ByteSize = "8MiB"`, ordered `versions = ["http/3", "http/2", "http/1.1"]`, `zero_rtt = disabled`; optional instance typed `headers`, open `custom_headers`, `auth_bearer`, `allowed_origins: array<text>`, `subprotocols: array<text>`, paired certificate/key bytes; optional invocation `correlate: text` |
| Correlator | direct descendants reply by session lineage; root-mixing, stored or spontaneous paths use explicit `correlate`, normally `meta.ext.session_id` |
| Outcome cases | `WebSocketExit`: `Closed { code: int, reason: text }`, `DnsFailed`, `ConnectFailed`, `TlsFailed`, `HandshakeRejected { status: int }`, `ProtocolFailed`, `TransportLost`, `TimeoutExpired`, `MessageTooLarge { limit: uint }`, `DecodeFailed`, `OutcomeUnknown` |
| Application profile | accepted Session, therefore not a Loop host; zero or more ordered peer `out` messages, zero or more graph-to-peer `in` writes, then exactly one terminal `exit`; a pre-admission refusal creates no Session or exit |
| Instances | one listener per Node instance may own many sessions; each session has one placement-neutral logical `SessionId` |
| Journal / sim | session open commits before upgrade success; each message remains bounded/pending until runtime acceptance; graph writes are sealed before bytes; Lab scripts the same open/messages/exit protocol |

An HTTP/1.1 Upgrade, HTTP/2 Extended CONNECT and HTTP/3 Extended CONNECT are
three physical bindings of this one contract. One complete WebSocket message
is one envelope. Fragmentation, ping/pong and close-handshake frames remain
inside the Driver. A message does not manufacture an invocation `exit`; only
the session terminal does. No `out` may follow that terminal.

H3 proved the bounded Tungstenite message machine on a synthetic stream and
through Hyper's HTTP/1.1 Upgrade. H6 separately proves RFC 8441 and RFC 9220;
all three handshakes feed the same generic Session fold, but no shared codec
test is counted as transport evidence.

For an established session, `meta.ext.session_id` is the textual projection
of the journal-derived logical `SessionId`, not a socket descriptor, address,
Worker id or library handle. A pre-open failure has no invented session id: no
Session existed. The field allows a graph to key a child scope or explicitly
correlate a response without exposing physical placement. It is
external-context data and may be copied deliberately; the runtime's causal
route remains authoritative.
The Driver overwrites any peer-supplied field at ingress. Correlation is scoped
to this Connector's live session table; an unknown or closed target is an
orphan write, discarded-and-journaled without misrouting or a second exit.

### 3.9 `WebSocketClient` — opened duplex sessions

| Case | Contract |
|---|---|
| Stable identity | `std/websocket-client@3` (exact Contract) |
| Role / Direction | both roles over one shared connection state; opens a remote session, writes graph messages and receives peer messages |
| Exchange | session — many messages in either direction, exactly one terminal `WebSocketExit` per opened session |
| Ports | `in: T`, `out: T`, `exit: WebSocketExit`; types follow the same required codec table as WebSocketServer; no implicit `err` |
| Populates | `meta.ext.session_id: text`, `http_headers`, `http_uri`, `http_version`, and selected `websocket_subprotocol` |
| behavior | required on each node, plan-evaluated; no Session idempotence attribute |
| Attributes | required instance `url: text` and positive `handshake_timeout: duration`; required plan `codec`; plan `max_message: ByteSize = "8MiB"`, ordered `versions = ["http/3", "http/2", "http/1.1"]`, `zero_rtt = disabled`; optional instance typed `headers`, open `custom_headers`, `auth_bearer`, `subprotocols: array<text>`, `tls_ca`, and paired certificate/key bytes |
| Correlator | v1 permits at most one live remote session per logical Node instance; no message-level request/reply correlation is invented |
| Outcome cases | the same closed `WebSocketExit` authority as the server; an opening failure is the sole terminal of the attempted session |
| Application profile | outbound-opened Session, therefore not a Loop host; zero or more ordered peer `out` messages, zero or more graph-to-peer `in` writes, then exactly one terminal `exit` |
| Instances | singleton by default; `instance.key` creates independent client sessions; input after a terminal opens the successor session under explicit lifecycle policy |
| Journal / sim | open and each write are journal-before-effect; ambiguous writes are not blindly replayed; Lab scripts peer messages and terminal races |

Reconnection is not a Driver retry. A graph may route selected exit cases
through `Loop` to open a new session and may retransmit only under its
application acknowledgement/idempotency protocol. The Driver never promises
that a transport-level write means the peer application processed a message.

## 4. Guide patterns over core Operator semantics

These names describe common patterns without adding another Node category:

| Name | Canonical core form |
|---|---|
| `Reducer` | an Operator with Node-local `var` state |
| `Aggregator` | a Reducer with `instance.key` |
| `Joiner` | a multi-input handler using the ordinary join and bound rules |

They are prose, not legal annotations. Routing is expressed by an Operator's
`Dispatch` or by the containing Graph's closed edge `match`; a keyed Graph is
just a scope with `instance.key`, not a `Dispatch` archetype. A future pure
guest-backed Operator may use a pinned `GuestProgram`, but it cannot reintroduce
the removed `node::Type` axis or masquerade as an Extension.

```rastok
operator total {
  instance = { key = meta.app.account_id }
  var amount: decimal(18, 2) = 0

  on in {
    amount = amount + data.amount
    out = { data.total = amount }
  }
}
```

The Reducer pattern adds no fold callback abstraction: state changes remain
visible in the handler.

```rastok
type JoinExpiry = union { JoinExpired {} }

operator decide {
  instance = { key = meta.app.request_id }
  lifetime = { kind = idle ; after = duration("48h") }
  output expired: JoinExpiry

  on (expense, decision) within duration("48h") {
    approval = {
      meta.app.request_id = expense.meta.app.request_id   // per-field pick (D-2026-08-10-2: the merge is automatic, picks are explicit)
      data = {
        request_id = instance.key
        approved = decision.data.approved
      }
    }
  }
}
```

The ordinary R6 join rules define correlation, bounds, and expiry. A bounded
join declares the exact one-case `JoinExpiry` output shown above; retained
facts leave through that ordinary typed port rather than disappearing. A
The Joiner pattern cannot hide an unbounded buffer.

The syntax for selecting a future guest-backed transition remains deferred.
`jq! { ... }` alone does not create a Node kind, Connector, or Extension. Any
eventual form remains subject to the pure guest contract in §5.

## 5. The pure guest-program contract — six clauses

The **Rastok DSL is the host language**. Rastok expressions are its
intrinsic computation language; their compatible core is differentially
tested against CEL, but CEL owns neither Rastok payload types nor the full
language. Expressions are not a guest module. A
tagged frontend carries source in a **guest language** — for example
`jq! { }` and `sql! { }` — which the build
turns into a versioned **`GuestProgram`**. A **`GuestRunner`** executes a
pure `GuestProgram` inside the enclosing Operator's node transaction. A pure
guest exists only by signing these six clauses:

1. **Pure determinism** — same input bytes → same output bytes, every host,
   every run. No `now`, no env, no randomness, no I/O: the guest sees only
   the value it is handed (zero ambient authority). **Set-producing guests
   must define a canonical order** (an unordered SQL result is a dead golden
   trace).
2. **Version pinned** — guest language, frontend/compiler/lowering profile,
   ABI and executor identities are canonical-HIR fields and therefore part of
   the semantic hash resolved by the `graph_version` manifest
   (trace-stability condition 4); a change to any of them = a version bump =
   an explicit re-bless.
3. **Typed boundary** — pin-or-variant at both block edges; inside, the
   guest language's own law applies (jq's *missing ≡ null* survives only there).
4. **Bounded cost** — the guest declares its bound mechanism (jq fuel, SQL
   row/memory limits); a blown budget emits its declared typed outcome
   case `GuestBudgetExceeded`, **never a hang**.
5. **One grain, one grammar** — one block position, one guest language;
   `IDENT!` is the common host form (`jq! { }`, `sql! { }`); there is no
   `cel!` block; a guest absent
   from the build (Cargo feature) = a compile error naming the missing
   feature.
6. **Typed outcomes, parsed early** — block syntax is parsed **at compile
   time** (no runtime syntax surprises); runtime evaluation failure is a
   declared tagged case such as `GuestError { language, message }` on the
   frontend's typed output contract — it enters coverage like everything
   else.

The core knows no guest language by name — it knows `GuestProgram` and the
contract.

**Placement is determined by effects, not by source language or execution
format.** A guest program satisfying the contract above belongs in an
Operator. A guest program that imports or performs network, filesystem,
process, clock or other external I/O belongs behind a Connector and is governed
by the Connector contract (§1) and outbound WAL. Wasm may implement either
case: it is an execution format and ABI, not a source language and not a
purity guarantee. JavaScript, Rust or another guest language may later lower
to a pure Wasm `GuestProgram`; a Wasm program with I/O imports remains
Connector material. The compiler/loader MUST reject an Operator
`GuestProgram` whose declared imports exceed the pure host ABI; the program
must instead be owned by a Connector. This rule reserves no new syntax.


## 6. The observability note (minimal — names and principles)

[observability.md](observability.md) owns the current contract: original typed
measurements stay in journaled envelopes, with exactly `id` and `unit` in
their annotation. M1 has no report engine, automatic price calculation,
subset arithmetic, metric instrument registry or OTLP exporter.

The planned optional hosted `otlp` feature observes runtime facts and can
export explicit business measurements alongside technical telemetry.
Aggregation and visualization happen downstream. Exporter time is never
invented commit time; exact temporal export and metric mapping need their
own qualification. The future Budget Extension consumes authoritative
retained facts for admission policy; it is not controlled by that exporter.
No exporter, instrument or Budget setting becomes executable through this note.


## 7. The MCP appendix (non-normative design direction)

The surface reserves the following mapping:

- **An exposed graph = an MCP tool**: typed input ports → `inputSchema`
  (the Arrow-anchored types project to JSON Schema); typed output ports and
  tagged exit cases → result/tool errors according to the export profile.
- **Operation guarantees → MCP hints** need an explicit future export
  contract. A plan-owned idempotence guarantee is potential input; no global
  effect enum or maximum-over-Connectors calculation supplies read-only,
  destructive or whole-graph repetition claims.
- **`behavior` → an annotation MCP does not define**: exported as a
  documented `_meta` extension.
- **Prompt confidentiality**: internal prompts are hidden from the
  *caller* (a tool exposes its contract, not its insides) — never from
  *authorized audit*. Secret credential bytes never enter the journal, and
  classified data remains subject to the deployment's privacy policy.
- **`McpClient` / `McpServer` are deferred Contracts for Connectors**.
  Plain REST needs no dedicated type; it lives in `HttpClient`/`HttpServer`.


## 8. Non-goals of this doc (deferred, with homes)

- **Additional Connector contracts**: `Postgres`, `Kafka`, `S3`, … —
  same template, no suffixes. Their order follows executable use cases;
  runtime durability does not depend on an application Connector.
- **`seeded`** behavior value.
- **HttpServer `respond` streaming** (SSE/chunked).
- **The health-check protocol details**.
- **The SEP text** for the MCP determinism annotation — its own deliverable.


## 9. Process service conformance obligations

D-2026-09-10-2 separates transport proof from gateway protocol proof.
D-2026-09-14-1 admits the resident transport; these remain its conformance
obligations, not a claim that a Codex-specific adapter is qualified:

- Several inputs to one live instance reuse its child; another keyed instance
  owns another child. First-message writes are not duplicated by opening.
- Every stdin write follows its retained intent; settlement proves local
  acceptance only, without a per-message Call or `exit`.
- A shutdown message such as `quit` follows the same write path. It neither
  closes an instance nor publishes an exit before the resource reports one.
  An ignored request remains ordinary data; a separately requested instance
  close is bounded and cannot fabricate a successful resource termination.
- Complete JSONL stdout frames become `variant` values under bounded intake.
  Process does not inspect application request ids or message kinds.
- An Operator adapter validates/classifies responses, notifications, progress
  and business errors, and owns any request correlation or deadline policy.
  Late-reply handling is explicit protocol logic, never a Process guess.
- An observed child termination yields `Exited` on the ordinary `exit` port;
  all resource terminals are close-once and prohibit later frames. Finishing
  a turn emits neither a resource exit nor a Node stop by implication.
- Exact-instance closure cannot terminate another conversation's child.
  Repeated closure does not repeat cancellation or touch a successor.
- Application shutdown, cooperative system termination and forced termination
  have distinct qualification cases. A late terminal after `OutcomeUnknown`
  cannot publish a second exit, and recovery never resends a shutdown message.
- Child death without a new input causes no new opening, including after an
  advance of logical time. A later input to the still-open instance may open
  one fresh Session; old writes, events and child memory do not migrate to it.
  A stopped generation stays stopped, and other instances keep their children.
- Simulation uses the same exchange model without a physical child. Replay
  never repeats a spawn or stdin write. Child-memory restoration and protocol
  resume cannot be claimed without a separately qualified explicit policy.

The [implementation ledger](../architecture/process-service.md) links the
transport tests and named limits. The earlier [review](../architecture/process-contract-review.md)
preserves the design inventory, including future Lab world actions.


## 10. Related docs

- [execution-model.md](execution-model.md) — `timeout`/`on_timeout`, the
  correlator duty, dated events, windows.
- [delivery-semantics.md](delivery-semantics.md) — INGRESS/EGRESS records,
  the key-stability rule these contracts obey.
- [simulation-semantics.md](simulation-semantics.md) — mocks, the coverage
  checklist fed by resolved port cases, `origin` for `Calendar` scenarios.
- [envelope.md](envelope.md) — `inbound_seq`, terminal envelopes.
- [effects-and-sagas.md](effects-and-sagas.md) — operation guarantees,
  journal-before-effect and future Saga compensation.
- [M1.6 HTTP/WebSocket backlog](../architecture/m1.6-http-websocket-backlog.md)
  — ordered gates from generic sessions through the hosted network proof.
