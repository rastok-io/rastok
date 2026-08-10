# Rastok DSL — Examples

Polished `.rkg` snippets that exercise the syntax conventions
(`docs/spec/grammar.md` § *Syntax conventions*) and the formal
grammar (`docs/spec/grammar.md` § *Formal grammar*).

The model behind the syntax is laid out in [WHITEPAPER.md](../../WHITEPAPER.md) :
the two Node families (Operator vs Connector), the flat declarative
shape (`input`, `output`, `var`, `on`, with the default `in` / `out` /
`err` ports), the value-type tree (`variant`, scalars, `struct` /
`map` / `array`), the three dynamic dispatch primitives, and the
partition model (flat, keyed, with lifetime). Examples below exercise
these concepts concretely. Stdlib aliases (`HttpClient`, `Process`,
`File`, …) name native Connector types registered with the runtime ;
the primitive `operator` / `connector` keywords stand on their own
when no alias fits.

The parser does not exist yet ; these examples are the syntactic contract it
will honor. They are deliberately short,
single-purpose, and incremental.

## 1. Minimal — HTTP echo + log

Three nodes, one chain. Useful as a parser smoke test.

```rastok
graph hello_echo {
  connector inbound : HttpServer {
    listen = ":8080/echo"
    codec = "json"
  }

  operator tag_received {
    on in {
      jq {
        .meta.app.tag = "received"
        | .data   = { echo: .data }
      }
    }
  }

  connector log : File {
    path = "/var/log/rastok/echo.ndjson"
    codec = "ndjson"
    append = true
    rotate = "100 MiB"
  }

  inbound.out -> tag_received.in
  tag_received.out -> log.in
}
```

`tag_received` declares no ports : it uses the default `in` and `out`.
Its single output means the trailing `jq { ... }` in `on in` is the
**implicit yield** to `out` ; the explicit form is `out = jq { ... }`.

## 2. Codex session — one `Process` Connector, multi-port stdio bridge

A graph is itself a node when composed inside another graph, and — like an
Operator — it declares its own keying with `per <partition>`. Here
`sessions per session` means one `Process` instance per session key ; its
per-session paths are **graph-level `val`s** derived from `session.key` (CEL) and
referenced by name inside. A single-in/out graph just uses the default `in` /
`out` ports — no declaration needed. A `Process` Connector's `in` / `out` /
`err` ports *are* its stdin / stdout / stderr — likewise no declaration.

```rastok
graph sessions per session {   // declaration = instance
  max_instances = 1000         // Dispatch knob = body attribute
  // `per session` → one Process instance per session key (Dispatch).
  // Single in / out pass-through → default `in` / `out` ports (no declaration).
  partition session {
    key      = meta.app.session_key
    lifetime = duration("2h")                          // idle TTL → governs instance eviction
  }
  // Per-session config, derived from the key, as graph-level `val` :
  val codex_home = "/var/codex/" + session.key
  val cwd        = "/workspace/" + session.key

  connector codex : Process {
    command = "codex"
    args = ["app-server"]
    mode = service
    cwd = cwd                              // graph-level val, by name
    env {
      CODEX_HOME = codex_home
      RUST_LOG   = "info"
    }

    codec = "jsonl"
    protocol = "jsonrpc"
    effect = idempotent
    behavior = probabilistic
    // in = stdin, out = stdout, err = stderr (Process defaults)

    init {
      message = {
        method = "initialize"
        params = { clientInfo = { name = "rastok" } }
      }
      timeout = duration("15s")
    }
    kill = process_group
    restart = on_crash { backoff = "1s..30s", max_attempts = 5 }
  }

  connector codex_logs : File {
    path = "/var/codex/" + session.key + "/codex.stderr.log"
    codec = "text"
  }

  in         -> codex.in   { capacity = 1024, backpressure = block }
  codex.out  -> out
  codex.err  -> codex_logs.in
}
```

## 3. Per-key sub-graph composition

`sessions` (from Example 2) is declared `per session`, so it is a per-key
sub-graph : **its declaration is its instance** (`compose` is not a
declaration form). The runtime materializes one running copy per distinct session key ;
key, lifetime and per-instance `val`s all come from the graph's own
`per session`. It wires by name, like any node.

```rastok
graph teams_inbound {
  connector teams_in : HttpServer {
    listen = ":8080/adapters/teams/webhook"
    codec = "json"
    protocol = "teams_activity"
  }

  operator auth {
    on in where data.?from.aadObjectId.orValue("") != "" {
      meta.app.session_key = data.from.aadObjectId          // CEL — scalar write, no jq needed
      out = .
    }
    on in { out = . }                                // pass-through otherwise
  }

  // One running instance of `sessions` per distinct session key (declared in Example 2).
  // Key + lifetime come from its `per session` ; paths are its graph-level vals.

  teams_in.out -> auth.in
  auth.out        -> sessions.in
}
```

## 4. Router + keyed aggregator — fold with flush

Two custom Operators wired together : a Router that classifies
deltas vs flush triggers, and a stateful Aggregator partitioned
per turn.

```rastok
graph turn_stream {
  // public ports default to in / out (no declaration)

  partition turn {
    key = data.params.turn_id
    lifetime = duration("5m")
  }

  operator route {

    on in {
      delta = jq { select(.meta.app.method == "item/agentMessage/delta") }
      flush = jq { select(.meta.app.method == "turn/completed") }
    }
  }

  // One turn = one partition instance, ended by the flush event.
  operator turn_aggregator per turn {
    var buffer: text = ""

    on delta {
      buffer = jq { $buffer + (.data.params.delta // "") }
    }

    on flush {
      out = jq {
        .meta.app.kind     = "jsonrpc.notification"
        | .meta.app.method = "item/agentMessage"
        | .data        = { method: "item/agentMessage", params: { message: $buffer } }
      }
      stop
    }
  }

  in                       -> route.in
  route.delta              -> turn_aggregator.delta
  route.flush              -> turn_aggregator.flush
  turn_aggregator.out      -> out
}
```

The aggregator uses the declared `turn` partition keyed by `turn_id`
with a 5-minute fallback TTL. The `stop` statement inside `on flush`
explicitly ends the partition instance when the turn-completed event
arrives — no manual buffer reset needed. The slot is reclaimed ; a
future delta for a new `turn_id` starts a fresh aggregator instance.

`select(cond)` inside jq is the **emission gate** : when the
predicate is false the assigned port is not written, and no
envelope leaves on that port.

## 5. Outbound HTTP with OAuth2 + readable DLQ

Heavier Connector example : auth, retries, rate limit, dead-letter
queue, and an explicit snapshot port for operations tooling.

```rastok
graph teams_outbound {
  // single input → default `in`
  input read_dlq: struct { }
  output dlq_snapshot

  connector teams_out : HttpClient + Loop {
    base_url = env("TEAMS_SERVICE_URL")
    path = "/v3/conversations/" + meta.app.conversation_id + "/activities"   // per-call CEL
    method = "POST"
    codec = "json"
    protocol = "teams_activity"
    effect = irreversible
    behavior = deterministic
    idempotency_key = meta.app.delivery_id
    // in = request, out = response (HttpClient defaults)

    auth = oauth2_client_credentials {
      token_endpoint = env("TEAMS_TOKEN_ENDPOINT")
      client_id      = env("TEAMS_BOT_APP_ID")
      client_secret  = env("TEAMS_BOT_PASSWORD")
      scope          = "https://api.botframework.com/.default"
      refresh_before = duration("10m")
    }
    // retry is expressed by the Loop aspect
    Loop.variant = measure {
      initial = 3
      next = value - 1
      until = value <= 0
    }
    Loop.backoff = exponential { initial = duration("200ms") }
    on err where err.transient { }               // iterating case : runtime re-injects
    rate_limit = "10/s per meta.app.session_key"
  }

  operator delivery_dispatch {

    on in {
      delivered = jq { select(.data.delivered) }
      failed    = jq { select(.data.delivered | not) }
    }
  }

  connector dlq : Memory { }

  in                            -> teams_out.in
  teams_out.out                 -> delivery_dispatch.in
  delivery_dispatch.delivered   -> nothing
  delivery_dispatch.failed      -> dlq.in
  .read_dlq                     -> dlq.read
  dlq.out                       -> .dlq_snapshot
}
```

The snapshot is oldest-first and non-destructive. Replaying selected
entries starts a new run and injects them as new roots; wiring
`dlq.out -> teams_out.in` here would create the static cycle that the
language deliberately rejects. The graph's input contract requires
`meta.app.delivery_id` to be assigned once, before this retrying
boundary, so every attempt carries the same business key.

## 6. Simulation — end-to-end deterministic test

Drives `teams_inbound` from Example 3 with a scripted Teams
message, asserts what the Codex session receives, and asserts the
outbound HTTP request shape.

```rastok
lab teams_inbound {    // binds to `graph teams_inbound : App` (declared in teams-codex-gateway.rkg)
  simulation teams_gateway on app {
    scenario plain_message "plain Teams message, agent replies" {

      mock sessions.codex {                                 // the composed Process, scripted
        on in where data.method == "turn/start" {
          out = {
            meta.app.kind   = "jsonrpc.notification"
            meta.app.method = "item/agentMessage"
            data = {
              method = "item/agentMessage"
              params = { message = "Bonjour" }
            }
          }
        }
      }

      teams_in.out = {
        meta.app.source = "teams"          // fixtures seed via meta.app — meta.sys is read-only, even in sim
        data = {
          type         = "message"
          from         = { aadObjectId = "abc-123" }
          conversation = { id = "conv-1" }
          text         = "Bonjour"
        }
      }

      expect "turn start"     sessions.codex.in where data.method == "turn/start" && data.params.prompt == "Bonjour"
      expect "Teams activity" teams_out.in where
        data.type == "message" && data.text.contains("Bonjour")
    }
  }
}
```

A fixture seeds inbound state through **`meta.app`** (writable) — never
by assigning `meta.sys.*`, which is read-only **absolutely, including in
simulation** (the fixture stands in for the runtime but goes through the
same braced envelope-form constructor, it does not bypass the invariant). See
[envelope.md §6](../spec/envelope.md).

## 7. Stateful Operator + Connector mock — weather lookup

Exercises three design-intent constructs : a stateful Operator
keyed by a partition, read-modify-write state semantics via
`var x` + bare-name reads (or `$x` inside jq) + `x = …`, and a
`mock` block scripting a Connector's behavior in a simulation.

```rastok
graph city_weather {
  // public ports default to in / out (no declaration)

  // Stateful Operator per session : counts how many requests this
  // session has issued, exposed in the meta of the outgoing request.
  operator request_counter per session {
    var count: int = 0

    on in {
      count = count + 1
      out = { meta.app.request_index = count }
    }
  }

  // Pure Operator : build the HTTP request shape, preserve the
  // user-facing city in meta so it survives the round-trip.
  operator build_request {
    on in {
      meta.app.request_city = data.city                     // scalar write = CEL
      data = jq {                                           // structural build = the jq guest
        .data | {
            url: "https://api.weather.example/v1/current",
            method: "GET",
            query:  { city: .city, country: (.country // "FR") }
          }
      }
    }
  }

  // Connector to the weather API.
  connector weather_api : HttpClient {
    timeout = duration("5s")
    effect = idempotent
    behavior = deterministic
    // in = request, out = response (HttpClient defaults)
  }

  // Pure Operator : extract the user-facing fields.
  operator format {
    on in {
      jq {
        .data = {
          city:        .meta.app.request_city,
          temperature: .data.temp_c,
          condition:   .data.condition
        }
      }
    }
  }

  in                        -> request_counter.in
  request_counter.out       -> build_request.in
  build_request.out         -> weather_api.in
  weather_api.out           -> format.in
  format.out                -> out
}

// Unit scope : injection targets inbound Connectors only (lineage stays true),
// so the scenario exercises the graph through a three-line wrapper app :
graph weather_demo : App {
  connector ask     : HttpServer { path = "/weather" }     // mode = ack (default)
  connector replies : Memory { }
  ask -> city_weather -> replies.in     // a declared graph wires BY NAME
}

lab weather_demo {
  simulation weather on app {
    scenario paris_sunny "user asks for Paris weather ; API answers clear sky" {

      mock city_weather.weather_api {                         // the scripted partner (root-relative qualified path)
        on in where data.query.city == "Paris" {
          out = { temp_c = 17.5, condition = "partly_cloudy" }   // the {…} envelope-data sugar
        }
      }

      ask.out = { city = "Paris", country = "FR" }            // injection = emission (+ sugar)

      // The intermediate contract (observation is legal on any port) :
      expect "request well-formed" city_weather.build_request.out where
        data.url == "https://api.weather.example/v1/current" && data.method == "GET" && data.query.city == "Paris"

      // The public outcome :
      expect "simplified response" replies.in where
        data.city == "Paris" && data.temperature == 17.5 && data.condition == "partly_cloudy"
    }
  }
}
```

Four things visible here :

- `operator request_counter per session` keys the Operator's
  state by the `session` partition defined in the stdlib. The state
  survives across envelopes of the same session and is
  garbage-collected when the session key idle-outs. For multi-key
  partitioning, use `per user, workspace`, etc.
- `var count : int = 0` declares the **typed state field**. It is
  read by bare `count` (or `$count` inside `jq { … }` — jq's own
  syntax) and mutated **in place** — `count = count + 1` here — so the
  emit on the same transaction reads the new value (imperative
  intra-transaction semantics ; the RHS is plain CEL, structural
  reshaping goes in a `jq { … }` block).
- `on in { … }` is the single-port handler. The handler forms
  (`on start`, `on <port>`, the `on (a, b)` join with optional
  `where` / `having` / `within` / `last` clauses, `on stop`) cover
  everything. No `on tick` — time-based behavior wires a
  `Clock` to an input.
- `mock weather_api { on in where … { out = … } }` replaces the real
  `HttpClient` Connector at simulation time, without touching the
  graph itself (mock bodies are ordinary handler grammar —
  simulation-semantics §3).

## 8. Meta as the control plane

`meta` carries small typed scalar facts used for routing,
partitioning, correlation and observability. The `data` remains the
payload. Constants declared with `val` make the defaults explicit
without creating mutable state.

```rastok
graph meta_control_plane {
  connector inbound : HttpServer {
    listen = ":8080/events"
    codec = "json"
  }

  operator normalize_meta {
    val SOURCE = "public_api"
    val DEFAULT_PRIORITY = "normal"

    on in {
      meta.app.source    = SOURCE                               // CEL — vals by bare name
      meta.app.tenant_id = data.?tenant_id.orValue("public")    // absence explicit (the grid)
      meta.app.user_id   = data.user.id
      meta.app.priority  = data.?priority.orValue(DEFAULT_PRIORITY)
      data = jq { .data.payload }                               // structural extraction = jq
    }
  }

  operator route_priority {

    on in {
      urgent = jq { select(.meta.app.priority == "urgent") }
      normal = jq { select(.meta.app.priority != "urgent") }
    }
  }

  connector urgent_log : File {
    path = "/var/log/rastok/urgent.ndjson"
    codec = "ndjson"
  }

  connector normal_log : File {
    path = "/var/log/rastok/normal.ndjson"
    codec = "ndjson"
  }

  inbound.out          -> normalize_meta.in
  normalize_meta.out       -> route_priority.in
  route_priority.urgent    -> urgent_log.in
  route_priority.normal    -> normal_log.in
}
```

The payload can change shape freely after normalization, but the
small scalar keys in `meta` are now stable enough to drive routing,
state partitioning and audit.

## 9. Constants + partitions — tenant policy and per-user quota

This example combines three scopes :

- App-level constants (`DEFAULT_PLAN`, `FALLBACK_LIMIT`,
  `REJECT_REASON`) shared by the graph.
- A persistent `tenant` partition that stores policy.
- A `user, tenant` composite partition that stores usage counters.

```rastok
graph quota_control : App {
  val DEFAULT_PLAN = "free"
  val FALLBACK_LIMIT = 1000
  val REJECT_REASON = "quota_exceeded"

  partition tenant {
    key = meta.app.tenant_id
    lifetime = persistent
  }

  partition user {
    key = meta.app.user_id
    lifetime = duration("24h")
  }

  graph quota_gateway {
    // public interface declared by name — a multi-port composable graph's contract
    input  admin
    input  usage
    output policy_updated
    output approval
    output rejection

    operator tenant_policy per tenant {
      val PLAN_LIMITS = {
        free = 1000,
        pro = 10000,
        enterprise = 100000
      }

      var plan: text = DEFAULT_PLAN
      var monthly_limit: int = FALLBACK_LIMIT


      on update {
        plan = jq { .data.plan // $plan }
        monthly_limit = jq { $PLAN_LIMITS[$plan] // $FALLBACK_LIMIT }   // reads the just-written plan

        updated = {
          meta.app.tenant_id = tenant.key
          meta.app.plan = plan
          meta.app.monthly_limit = monthly_limit
        }
      }
    }

    operator usage_gate per user, tenant {
      var used_tokens: int = 0


      on event {
        val cost = jq { (.data.tokens // 0) | tonumber }
        val limit = tenant.?monthly_limit.orValue(FALLBACK_LIMIT)

        used_tokens = used_tokens + cost           // jq below reads the new $used_tokens

        approval = jq {
          select($used_tokens <= $limit)
          | .meta.app.quota_status = "approved"
          | .meta.app.tokens_used  = $used_tokens
          | .meta.app.tokens_limit = $limit
        }

        rejection = jq {
          select($used_tokens > $limit)
          | .meta.app.quota_status = "rejected"
          | .meta.app.tokens_used  = $used_tokens
          | .meta.app.tokens_limit = $limit
          | .data = {
              reason: $REJECT_REASON,
              tokens_used: $used_tokens,
              tokens_limit: $limit
            }
        }
      }
    }

    admin -> tenant_policy.update
    tenant_policy.updated -> policy_updated
    usage -> usage_gate.event
    usage_gate.approval -> approval
    usage_gate.rejection -> rejection
  }
}
```

The important part is the read boundary : `usage_gate` writes only
its own `used_tokens`, but it can read `tenant.monthly_limit` as a
read-only cross-partition value. If no tenant policy exists yet,
the optional read falls back to `FALLBACK_LIMIT` via `.?` + `.orValue()`.

## Conventions visible everywhere

| Construction | Form |
|---|---|
| Operator declaration | `operator <name> [per <partition>] { ... }` |
| Connector declaration | `connector <name> : <NativeType> [per <partition>] { ... }` |
| Graph declaration | `graph <name> [per <partition>] { ... }` (keying on the graph, like an Operator) |
| Sub-graph instance | declaration = instance : `graph <name> per <partition> { max_instances = … ; … }`, wired by name (`compose` is not a declaration form) |
| Operator port | **Implicit** : `on <name>` ⇒ input, `<name> = …` ⇒ output. Declare `input`/`output <name>[: T]` only to pin a type/schema or document. `in`/`out`/`err` are defaults. |
| Public graph port | single-in/out → defaults `in`/`out` ; a multi-port graph declares its interface by name (`input admin`, …), optionally typed |
| Connector port | never declared — the Connector type + `codec` define `in`/`out`/`err` |
| Type | `variant`, scalar (`text`/`int`/`bytes`/…), `struct { … }`, `map<text, V>`, `array<T>`, `enum`, `union`, `T?` |
| State field | `var <name>: <type> [= <default>]` |
| Connector attribute | `<name> = <value>` |
| Tagged-value attribute | `<name> = <tag> { ... }` (`restart = on_crash { ... }`, `Loop.backoff = exponential { ... }`) |
| Handler | `on <trigger> [where … having … within … last …] { ... }` (`on (a, b)` join, `on start`, `on stop`) |
| State read (inside jq) | `$<name>` (bare `<name>` in DSL) |
| State write | `<name> = <expr>` |
| Local binding | `val <name> = <expr>` |
| Port emission | `<out_port> = <expr>` (trailing expression yields to `out`) ; atomic multi-emit `(a, b) = (e1, e2)` |
| Narrowing | `<expr> as <type>` (fallible → `err`) |
| List / array | `[a, b, c]` |
| References | boot : `env("VAR")` / `secret("path")` (initializers only) ; runtime : plain CEL (`session.key`, `meta.app.x`, `"a" + b`) — the `${…}` interp is gone |
| Edge | `a -> b` |
| Port-qualified edge | `a.<out_port> -> b.<in_port>` |
| Edge with config | `a -> b { capacity = 1024, backpressure = block }` |

The full grammar — including the LL(1) decision table — lives in
`docs/spec/grammar.md` § *Formal grammar*.
