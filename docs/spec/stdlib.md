# Standard library contracts

> **Status: Normative — under review.**
> Companion to [execution-model.md](execution-model.md) (`timeout` /
> `on_timeout`, the correlator duty), [delivery-semantics.md](delivery-semantics.md)
> (INBOUND / OUTBOUND records) and [simulation-semantics.md](simulation-semantics.md)
> (mocks, the coverage checklist these contracts feed).
>
> Guest-language placement follows the Operator/Connector boundary: a pure
> `GuestProgram` executes inside an Operator; a guest program that performs
> I/O executes behind a Connector.

The question this doc answers: **what exactly does each stdlib citizen
promise — so that the compiler, the simulator, the coverage checklist and the
future runtime all read the same contract?**


## 1. The Connector contract — ten mandatory cases

Every Connector trait is specified by the same template. No case is
optional — a Connector whose contract leaves a case blank is not spec'd:

| # | Case | Why it is mandatory |
|---|---|---|
| 1 | **Role** | what it bridges (a protocol, a process, time) |
| 2 | **Direction** | inbound source / outbound caller / both / sink |
| 3 | **Ports** | `in`/`out`/`err` + extras, with types |
| 4 | **`effect`** | pre-declared by the trait, or per-instance (dev-declared) |
| 5 | **`behavior`** | deterministic / probabilistic — §2; pre-declared or per-instance |
| 6 | **Attributes** | the config surface (boot phase: `env()`/`secret()` legal); `timeout`/`on_timeout`/`restart` inherited where calls/instances exist |
| 7 | **Correlator** | what "a call" is in its protocol (execution-model §6.1: the Connector matches) |
| 8 | **Error set** | its typed errors — the finite list `rastok sim --coverage` enumerates |
| 9 | **Instances** | singleton / `per`-keyed (each with `run_id`) / per-call; the terminal envelope |
| 10 | **Journal & sim** | which records it writes (INBOUND w/ `inbound_seq`; OUTBOUND open-closed); how it is mocked/injected |

Case 8 deserves the emphasis: a Connector without a declared error set would
make the failure space non-enumerable — so the error set is **as mandatory as
`effect`**. The *effective* set =
the declared kinds **plus the runtime-common kinds** the machinery can emit
for any caller: `TimeoutExpired` (any budget), `OutcomeUnknown` (unkeyed
compensable, recovery), `OutputOverflow` (job mode) — `--coverage` counts
both halves.

**Naming rule**: stdlib traits carry **no suffix** — the keyword already says
it (`connector c: Clock`). Applies to all future traits too (`Postgres`,
`S3`, …).


**The contract is also the assurance boundary**: a future
`theorem` proves a graph **under** the Connector contracts declared here;
an `experiment` (simulation-semantics.md §10) checks empirically
that a real adapter and partner honor them. Neither layer replaces the
other.

## 2. `behavior` — the second axis, beside `effect`

The world's *timing* nondeterminism is already journal-absorbed
(journal-relative replay). `behavior` declares something else: **the
partner's semantic function** —

```
   same request, same world state  →  same answer ?

   GET /users/42                   →  yes            behavior = deterministic
   codex « think about … »         →  NO, by design  behavior = probabilistic
                                       (a distribution is sampled)
```

- **Pre-declared** where the trait imposes it: `Clock`, `Calendar`,
  `Memory`, `HttpServer` = `deterministic`.
- **Per-instance, mandatory** on `HttpClient` and `Process` (only the dev
  knows who answers). Forgetting it = compile refusal, like `effect`.
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


## 3. The six contracts

### 3.1 `HttpServer` — inbound HTTP, two modes

| Case | Contract |
|---|---|
| Role / Direction | HTTP endpoint; **inbound source** |
| Ports | `out` (decoded requests); `in` (mode `respond`: the graph's answers); `err` |
| Populates | `meta.ext` : `content_type` (the request's full Content-Type), `request_id: text?`, `path: text`, `method: text` — written once at ingress; handlers may rewrite, never invent (D-2026-08-10-1) |
| effect / behavior | `none` / `deterministic` (pre-declared) |
| Attributes | `path`, `mode = ack (default) \| respond`, `codec`, `auth`, `timeout` (respond: the open-request budget), **`correlate = <CEL path on the response>`** (respond; default: lineage) |
| Correlator | mode `respond`: **default = lineage** (`origin_id == R.id`) — valid on join-free return paths ONLY. **A return path crossing a root-mixing join ⇒ explicit `correlate` required (compile error otherwise)**: the join output inherits the TRIGGER's origin, creating a race. The author carries the token (`meta.app.request_id = meta.sys.id`) and attributes it through joins with the explicit-attribution rule; matching = correlate-value == the open request's root id. An orphan response (no open request — it already expired, the 504 went out) is **discarded-and-journaled** (the call-closed rule's cousin). |
| Error set | `MalformedRequest` (400 to the client AND a journaled typed err), `AuthFailed` (401), `RequestExpired` (respond: 504, `l:timeout`, call-closed — a late graph answer is discarded-and-journaled, never two HTTP responses), `Overloaded` (respond: open requests are **bounded** → 503; composes with §3-delivery bounded lag) |
| Instances | singleton |
| Journal / sim | INBOUND records, `inbound_seq` atomic with the accept; fixture injects at `out` |

- **`ack`** (webhook): 202 immediately — "received", not "processed";
  replies to the user travel through an outbound Connector.
- **`respond`** (API): the request stays open; correlation by lineage;
  timeout → 504; bounded → 503. Exposing an API is first-class.
- The async HTTP library is the **runtime's** choice — the contract is
  lib-agnostic.

### 3.2 `HttpClient` — outbound HTTP

| Case | Contract |
|---|---|
| Role / Direction | HTTP calls; **outbound caller** |
| Ports | `in` (requests), `out` (responses), `err` |
| Populates | `meta.ext` : `content_type` (the response's Content-Type), `status: int` — on the reply envelope |
| effect / behavior | **per-instance, both mandatory** |
| Attributes | `url`/`base_url`+`path`, `method`, `codec`, `auth`, `tls`, `timeout`, `on_timeout = bypass (default) \| kill`, `idempotency_key` (mandatory when `irreversible`) |
| Correlator | protocol-native: HTTP request ↔ response |
| Error set | `ConnectFailed { transient = true }`, `TimeoutExpired { transient = true }`, `HttpStatus { status, body, transient }` — **default mapping: status ≥ 400 → err**, `transient = status ∈ {408, 429, 5xx}`. (`on err where err.transient` reads naturally; the mapping is stated, revisit only with evidence.) |
| Instances | singleton (`per` legal, rarely useful) |
| Journal / sim | OUTBOUND open (key, digest) / closed / discarded; mocked in scenarios |

### 3.3 `Process` — child processes, two modes

| Case | Contract |
|---|---|
| Role / Direction | child process over stdio; **both** (caller + source) |
| Ports | `in` → stdin; `out` ← stdout; `err` ← stderr + lifecycle |
| effect / behavior | **per-instance, both mandatory** |
| Attributes | the normative table below (D-2026-08-12-1) — every attribute is `name = value`, presence and defaults are the CONTRACT's, and the checker injects every omitted default into the effective HIR |
| Correlator | see modes below |
| Error set | `SpawnFailed`, `DecodeError` (the offending line in payload — never silent), `StderrLine` (stderr output; route it or write `-> nothing`), `TimeoutExpired`, `ExecFailed { exit_code, stderr }` (job), `OutputOverflow { size }` (job), `ProcessExited { exit_code, signal, run_id }` (service: the **terminal** error — terminal BY KIND; its data lives in the typed payload, `meta.sys` stays closed) |
| Instances | see modes below |
| Journal / sim | OUTBOUND records per call (job: per run); spontaneous emissions = INBOUND roots; mocked in scenarios |

**The attribute table (D-2026-08-12-1).** `mode` is the PROFILE
selector; its omission selects `job` and the checker materializes
`mode = job` into the effective HIR — omission and the exact
spelling of a default are one identity (hash included). The plan
carries the EFFECTIVE attributes; the adapter consumes resolved
values and never reintroduces a default.

| Attribute | Profile | Domain | Presence |
|---|---|---|---|
| `command` | common | CEL → `text` | **Required** |
| `args` | common | CEL → `array<text>` | Defaulted(`[]`) — omission ≡ `[]`; `null` refuses (no domain is implicitly nullable) |
| `codec` | common | choice `lines \| jsonl` (`bytes` **reserved**) | **Required** — the framing is never chosen silently (D-2026-08-11-1) |
| `effect` | common | choice, typed | **Required** |
| `behavior` | common | choice, typed | **Required** |
| `timeout` | common | CEL → `duration` | Optional — a true optional: no budget declared, no budget |
| `env` | common | composite map — **deferred** (named diagnostic until composite domains land; the canonical spelling will be `env = { "K": v }`) | — |
| `max_output` | job | CEL → `text` (a size string) | Defaulted(`"8MiB"`) |
| `on_timeout` | job | choice `kill` (one word — "fixed to `kill`") | Defaulted(`kill`) |
| `correlate` | service | CEL path over runtime data (deferred-only domain) | Optional |
| `restart` | service | struct (deferred-only domain) | Optional |

`correlate` and `restart` do NOT exist on the job profile: writing
one there refuses by NAME (« `restart` belongs to
`Process(mode = service)` and is not applicable to
`Process(mode = job)` ») — never a generic "forbidden".

**`mode = job` (default — the Unix one-shot; systemd `Type=oneshot`, k8s Job
lineage)**: one envelope = one spawn. stdin = the request (codec-framed);
stdout, complete at exit, = **one reply envelope**: `codec =
lines` → `data = text` (the whole stdout, one blob); `codec = jsonl` →
`data = array<variant>` (the decoded lines — possibly length 1). The buffer
is bounded: **`max_output = 8MiB` (default)**; exceeded → typed err
`OutputOverflow { size }` + the run is killed. exit ≠ 0 / signal →
`ExecFailed`. The **process boundary IS the correlation** — `correlate` is a
compile error here. `timeout` = the whole-run budget; `on_timeout = kill`
**forced** (bypassing a detached run is meaningless). `restart` is likewise
a compile error on a job (re-running a possibly-irreversible one-shot is a
graph decision — a Loop — never a supervisor reflex). `per` still
orders (alice's conversions run serially) — it shares nothing. The safe
default: zero hidden shared state.

**The `jsonl` wire (D-2026-08-11-1).** One JSON value per `\n`-framed
line, both directions, the JSON model exactly (`variant`).
OUTBOUND, the request value serializes CANONICALLY — these bytes are
`request_digest` material, so the writer is part of identity: object
keys sorted by their UTF-8 bytes (structs and maps alike — the
wire's own canonical order; the envelope seal orders map entries by
tagged CBOR bytes, which differs: `{"b", "aa"}` seals `b` first and
writes `aa` first),
numbers exact (`int` as the integer, `decimal` in plain decimal
notation at its sealed scale, `double` in shortest round-trip form),
strings JSON-escaped, one trailing `\n`. Unserializable requests
refuse TYPED as `DecodeError` before any open fact: `bytes` (no JSON
form — `lines` or a future `bytes` codec carry those), a
non-finite `double`, and any ENCODED payload (`jsonl` frames the
JSON model, not opaque representations). INBOUND, each line decodes
EAGERLY through the house JSON reader (D-2026-08-10-4/-7): exact
numbers, duplicate keys refused; a line that does not parse — an
interior empty line included — is `DecodeError` with the offending
line in the payload; the single trailing newline's empty tail is
framing, not a line; empty stdout is the empty array — but a LONE
`\n` is one empty line and refuses (the framing is judged before
the terminator is stripped). A line's padding is JSON's own
whitespace alphabet (space, tab, CR) and nothing wider; `null` is a
value like any other and decodes to the null element. The reply's
shape is a REAL pin: `Process(job, jsonl).out = array<variant>`,
consulted by the checker (a mock answering anything else refuses at
check) and by the machine (every settled outcome is judged against
it) — the Lab and the world see one shape.

**`mode = service` (the declared resident)**: long-running. `codec` frames
line by line. With `correlate = data.id`, **stdout splits**: lines whose id
matches an open call are its response (OUTBOUND closed, emitted on `out`,
trigger `p:in`, parent = the request); non-matching lines are **spontaneous
events** — INBOUND roots (the Process is also a source: codex
notifications). Without `correlate`: pure streams (in = stdin sink, out =
stdout source, no calls, no per-call timeout). `per` = one instance per key
(a codex **per session**), each with its `run_id`, terminal envelope and
`restart` policy; without `per` = the **explicit** shared singleton. Lazy
spawn (first delivery).

### 3.4 `Clock` — the metronome

| Case | Contract |
|---|---|
| Role / Direction | periodic physical time; **inbound source** |
| Ports | `out`: tick envelopes (`timestamp`) |
| effect / behavior | `none` / `deterministic` (pre-declared) |
| Attributes | `every = duration("…")` (a constant CEL duration) |
| Correlator | **none** (a pure source makes no calls) |
| Error set | **empty** (a metronome has no failure mode worth an envelope) |
| Instances | singleton per declaration; `system_clock` = the implicit instance the supervision plane already uses (lifetimes, `within`) |
| Journal / sim | ticks = INBOUND records; in sim, driven by `advance` |

### 3.5 `Calendar` — civil time

| Case | Contract |
|---|---|
| Role / Direction | calendar instants; **inbound source** |
| Ports | `out`: `data : struct { scheduled_at: timestamp }` |
| effect / behavior | `none` / `deterministic` (pre-declared) |
| Attributes | **either** `cron = <expr>` + `tz = <IANA zone>` (mandatory together — never the host's implicit zone) **or** `at = timestamp("…")` (fires **once**; the zone lives in the RFC-3339 string) |
| Correlator | **none** (a pure source makes no calls) |
| Error set | empty (misconfiguration is compile-time: bad cron/tz = refusal) |
| Instances | singleton per declaration |
| Journal / sim | ticks = INBOUND records; a scenario using `Calendar` should declare its `origin` (simulation-semantics §5: virtual t=0 ≡ a civil instant; absent → epoch + **warning**) |

**DST policy, stated**: a nonexistent civil time (spring forward) is
**skipped**; an ambiguous one (fall back) fires **once, first occurrence**.
The classic sane-cron behavior, written down.

*(Clock vs Calendar = the time literature's own pair: clock time — physical,
relative — vs calendar time — civil, absolute.)*

### 3.6 `Memory` — the bounded in-process store

| Case | Contract |
|---|---|
| Role / Direction | bounded, ordered, in-process value store; **append and snapshot read** |
| Ports | `in: T` appends `data`; `read: struct { }` requests a snapshot; `out: array<T>` returns it; `err` |
| effect / behavior | `idempotent` / `deterministic` (pre-declared) |
| Attributes | `cap = <n>` (default 10 000 — bounded **by construction**), `overflow = evict_oldest (default, evictions journaled) \| reject` |
| Correlator | a delivery to `read` is one call and produces exactly one `out` snapshot or one typed `err` |
| Error set | `CapacityExceeded` (mode `reject`) |
| Instances | one store per declared Node instance and therefore per partition key when the declaration uses `per` |
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

## 4. Standard vocabulary over core Operator semantics

These names expose common patterns without adding a second execution model:

| Name | Canonical core form |
|---|---|
| `Router` | one pure selector and a finite set of statically declared output ports |
| `Reducer` | an Operator with `var` state, normally keyed with `per` |
| `Aggregator` | a Reducer whose partition has a window lifetime |
| `Joiner` | a multi-input handler using the ordinary join and bound rules |
| `Dispatch` | the runtime role of a declared `graph ... per <partition>` |
| `Jaq` | a pure Operator whose computation is a pinned `jq { ... }` guest program |

They are deterministic by construction. Their names may improve diagnostics
and generated diagrams, but the semantics below reduce to the same handlers,
partitions, and edges as any other Operator or Graph.

```rastok
operator route : Router {
  select = data.kind
}

route.approval -> accept
route.rejection -> decline
```

The selector returns one declared port name, or a list of declared names for
a bounded multicast. An undeclared result is a typed routing error.

```rastok
partition account {
  key = meta.app.account_id
  lifetime = persistent
}

operator total : Reducer per account {
  var amount: decimal(18, 2) = 0

  on in {
    amount = amount + data.amount
    out = { data.total = amount }
  }
}
```

`Reducer` adds no fold callback abstraction: state changes remain visible in
the handler.

```rastok
partition request {
  key = meta.app.request_id
  lifetime = duration("48h")
}

operator decide : Joiner per request {
  on (expense, decision) within duration("48h") {
    approval = {
      meta.app.request_id = expense.meta.app.request_id   // per-field pick (D-2026-08-10-2: the merge is automatic, picks are explicit)
      data = {
        request_id = request.key
        approved = decision.data.approved
      }
    }
  }
}
```

The existing join rules still define correlation, bounds, and expiry. A
`Joiner` cannot hide an unbounded buffer.

```rastok
graph conversation per session {
  . -> classify -> answer -> .
}
```

This declaration is the Dispatch pattern: the graph type is static, while
the runtime creates bounded instances by `session` key. No dynamic graph
type is loaded.

```rastok
operator normalize : Jaq {
  program = jq {
    .data | with_entries(.key |= ascii_downcase)
  }
}
```

`Jaq` is available only when the jq guest is installed and pinned. It remains
subject to the pure guest contract in §5.

## 5. The pure guest-program contract — six clauses

The **Rastok DSL is the host language**. CEL is its integrated core expression
language, not a guest module. A tagged block carries source in a **guest
language** — for example `jq { }` and `sql { }` — which the build
turns into a versioned **`GuestProgram`**. A **`GuestExecutor`** executes a
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
   row/memory limits); a blown budget = typed err `GuestBudgetExceeded`,
   **never a hang**.
5. **One grain, one grammar** — one block position, one guest language; the tag
   is the name (`jq { }`, `sql { }`); no `cel { }` block; a guest absent
   from the build (Cargo feature) = a compile error naming the missing
   feature.
6. **Typed errors, parsed early** — block syntax is parsed **at compile
   time** (no runtime syntax surprises); runtime eval failure = typed err
   `GuestError { language, message }` on `err` — it enters the coverage
   checklist like everything else.

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

The parallel plane's contract is set (real times = journal `committed_at` +
spans; nothing wall-clock in the data plane). This note pins four things:

1. **Normative metric names** — per partition: `mailbox_depth`, `lag` (per
   edge — *the* backpressure signal), `instances`, `evictions`; per
   Connector: `calls`, `timeouts`, `retries`, `outbound_open` (in-flight),
   `terminals`; per app: `journal_size` vs `low_water_mark`. Dashboards
   converge on names, not guesses.
2. **Inactivity & liveness** (deferred from execution-model §6.1):
   observation-plane watches (*"nothing emitted for 5min"*, *"is the process
   alive?"*) producing **alerts, never envelopes** — zero determinism
   impact.
3. **OTel export** — `origin_id` = trace id; one span per transaction (real
   durations); outbound calls as child spans. **The journal is the truth;
   spans are derived** — never the reverse.
4. **Per-session cost attribution** — OUTBOUND records MAY carry a `cost`
   map declared by the Connector (tokens, calls, bytes); aggregated by
   partition key / `origin_id` → per-session spend. With §2: the
   probabilistic surface is *where* the cost lives; the journal *prices*
   it.


## 7. The MCP appendix (non-normative design direction)

The surface reserves the following mapping:

- **An exposed graph = an MCP tool**: typed input ports → `inputSchema`
  (the Arrow-anchored types project to JSON Schema); output → result; the
  typed error set → tool errors.
- **`effect` → the existing MCP annotations**, derived (the max over the
  graph's Connectors): `idempotent` → `idempotentHint`, `irreversible` →
  `destructiveHint`, `none` → `readOnlyHint`.
- **`behavior` → an annotation MCP does not define**: exported as a
  documented `_meta` extension.
- **Prompt confidentiality**: internal prompts are hidden from the
  *caller* (a tool exposes its contract, not its insides) — never from
  *audit* (the journal sees everything). The onion rule.
- **`McpClient` / `McpServer` are deferred first-class traits**. Plain REST
  needs no dedicated trait; it lives in `HttpClient`/`HttpServer`.


## 8. Non-goals of this doc (deferred, with homes)

- **Additional Connector traits**: `Postgres`, `File`, `Kafka`, `S3`, … —
  same template, no suffixes. Their order follows executable use cases;
  runtime durability does not depend on an application Connector.
- **`codec = bytes`** on Process (stream framing question).
- **`seeded`** behavior value.
- **HttpServer `respond` streaming** (SSE/chunked).
- **The health-check protocol details**.
- **The SEP text** for the MCP determinism annotation — its own deliverable.


## 9. Process service conformance obligations

- A correlated request/reply round-trip closes its OUTBOUND call.
- An uncorrelated service notification becomes a spontaneous INBOUND root.
- A protocol error is returned as a correlated typed error.
- A reply after `TimeoutExpired` is discarded-and-counted, never delivered.
- Process death emits `ProcessExited`; the restart policy may respawn the
  service and invoke its protocol-specific resume operation.

A partner may persist per activity rather than per creation, so an empty
session can legitimately be unresumable. The recovery recipe for such
partners is **resume-or-start-fresh, routed on the typed error** — graph-side
wiring, no new mechanism.


## 10. Related docs

- [execution-model.md](execution-model.md) — `timeout`/`on_timeout`, the
  correlator duty, dated events, windows.
- [delivery-semantics.md](delivery-semantics.md) — INBOUND/OUTBOUND records,
  the key-stability rule these contracts obey.
- [simulation-semantics.md](simulation-semantics.md) — mocks, the coverage
  checklist fed by the error sets, `origin` for `Calendar` scenarios.
- [envelope.md](envelope.md) — `inbound_seq`, terminal envelopes.
- [effects-and-sagas.md](effects-and-sagas.md) — the `effect` axis;
  `behavior` is its sibling.
