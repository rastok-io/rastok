# Rastok DSL — Examples

Polished `.rkg` snippets that exercise the syntax conventions
([`docs/spec/grammar.md`](../spec/grammar.md) § *Syntax conventions*)
and its formal grammar.

The model behind the syntax is laid out in [WHITEPAPER.md](../../WHITEPAPER.md) :
the two Node families (Operator vs Connector), the flat declarative
shape (`input`, `output`, `var`, `on`, with the default `in` / `out` /
`err` ports), the value-type tree (`variant`, scalars, `struct` /
`map` / `array`), the three dynamic dispatch primitives, and the
orthogonal instance-key and lifetime model. Examples below exercise
these concepts concretely. Stdlib aliases (`HttpClient`, `Process`,
`File`, …) name Connector types in the target catalogue;
the primitive `operator` / `connector` keywords stand on their own
when no alias fits.

The parser is a forward lexeme stream. Categorical plan-package v9 retains
the complete v7 M1.3 type/value/expression registry and now carries lexical
owners, Node-owned instance policy, scoped edges, and phased immutable
expressions. As of 2026-08-21,
`Process(mode = job)` with `lines`/`jsonl`/`bytes`, multiport completion,
the three metadata zones, `IDENT!` frontends, structural and tagged types,
typed narrowing, and keyed persistent/idle Node owners execute. The former
named-partition P0-P3 implementation is superseded by the
`instances`/`lifetime` model. Graph-owned and hierarchical routing,
`fixed`/`single_pass`/`tumbling` profiles, closing, HTTP/File adapters,
Dispatch, joins, Loop, Process service mode, and guest engines are still
pending or guarded. Thus
these are parser- and design-reference snippets, not a claim that every
section runs today. The small executable fixtures under
[`examples/`](../../examples/) are the run/simulate proofs.

## 1. Minimal — HTTP echo + log

Three nodes, one chain. Useful as a parser smoke test.

```rastok
graph hello_echo {
  connector inbound : HttpServer {
    listen = ":8080/echo"
    codec = json
  }

  operator tag_received {
    on in {
      out = jq! {
        .meta.app.tag = "received"
        | .data   = { echo: .data }
      }
    }
  }

  connector log : File {
    path = "/var/log/rastok/echo.ndjson"
    codec = ndjson
    append = true
    rotate = "100 MiB"
  }

  inbound.out -> tag_received.in
  tag_received.out -> log.in
}
```

`tag_received` declares no ports : it uses the default `in` and `out`,
and the handler writes it explicitly — `out = jq! { ... }` (there is no
implicit yield, D-2026-08-13-1).

## 2. Codex session — one `Process` Connector, multi-port stdio bridge

A graph is itself a Node when composed inside another graph, and — like an
Operator or Connector — it may select one logical instance with
`instances.key`. Here `sessions` selects one instance per session key; its
per-session paths are **graph-level `val`s** derived from `instance.key` (a
Rastok expression) and
referenced by name inside. A single-in/out graph just uses the default `in` /
`out` ports — no declaration needed. A `Process` Connector's `in` / `out` /
`err` ports *are* its stdin / stdout / stderr, while `exit` carries its
tagged lifecycle outcome — likewise no declaration.

```rastok
graph sessions {
  instances = { key = meta.app.session_key }
  lifetime = { kind = idle; after = duration("2h") }
  // One Graph instance per canonical session key. Physical capacity and
  // placement are deployment policy, not logical instance attributes.
  // Single in / out pass-through → default `in` / `out` ports (no declaration).
  // Per-session config, derived from the key, as graph-level `val` :
  val session_key = instance.key
  val codex_home = "/var/codex/" + instance.key
  val cwd        = "/workspace/" + instance.key

  connector codex : Process {
    command = "codex"
    args = ["app-server"]
    mode = service
    cwd = cwd                              // graph-level val, by name
    env = {
      "CODEX_HOME": codex_home
      "RUST_LOG": "info"
    }

    codec = jsonl
    protocol = "jsonrpc"
    effect = idempotent
    behavior = probabilistic
    // in = stdin, out = stdout, err = stderr, exit = invocation terminal (Process defaults)

    init = {
      message = {
        method = "initialize"
        params = { clientInfo = { name = "rastok" } }
      }
      timeout = duration("15s")
    }
    kill = process_group
    restart = { backoff = "1s..30s", max_attempts = 5 }
  }

  connector codex_logs : File {
    path = "/var/codex/" + session_key + "/codex.stderr.log"
    codec = text
  }

  in         -> codex.in   { capacity = 1024, backpressure = block }
  codex.out  -> out
  codex.err  -> codex_logs.in
}
```

## 3. Per-key sub-graph composition

`sessions` (from Example 2) owns `instances.key`, so it is a keyed sub-graph:
**its declaration owns its instances** (`compose` is not a declaration form).
The runtime materializes one logical instance per distinct canonical session
key; key, lifetime and graph-level `val`s all come from that declaration. It
wires by name like any Node.

```rastok
graph teams_inbound {
  connector teams_in : HttpServer {
    listen = ":8080/adapters/teams/webhook"
    codec = json
    protocol = "teams_activity"
  }

  operator auth {
    on in where data.?from.aadObjectId.orValue("") != "" {
      meta.app.session_key = data.from.aadObjectId          // Rastok expression — no jq needed
      out = .
    }
    on in { out = . }                                // pass-through otherwise
  }

  // One logical instance of `sessions` per distinct session key.
  // Its declaration owns both key and lifetime; paths are graph-level vals.

  teams_in.out -> auth.in
  auth.out        -> sessions.in
}
```

## 4. Router + keyed aggregator — fold with flush

Two custom Operators wired together : a Router that classifies
deltas vs flush triggers, and a stateful Aggregator keyed by turn.

```rastok
graph turn_stream {
  // public ports default to in / out (no declaration)

  operator route {

    on in {
      delta = jq! { select(.meta.app.method == "item/agentMessage/delta") }
      flush = jq! { select(.meta.app.method == "turn/completed") }
    }
  }

  // One turn = one Operator instance, ended by the flush event.
  operator turn_aggregator {
    instances = { key = data.params.turn_id }
    lifetime = { kind = idle; after = duration("5m") }
    var buffer: text = ""

    on delta {
      buffer = jq! { $buffer + (.data.params.delta // "") }
    }

    on flush {
      out = jq! {
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

The aggregator selects its own instance by `turn_id` and gives it a five-minute
idle lifetime. The `stop` statement inside `on flush` explicitly ends the
current Operator instance when the turn-completed event
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
    path = "/v3/conversations/" + meta.app.conversation_id + "/activities"   // per-call Rastok expression
    method = "POST"
    codec = json
    protocol = "teams_activity"
    effect = irreversible
    behavior = deterministic
    idempotency_key = meta.app.delivery_id
    // in = request, out = response (HttpClient defaults)

    auth = {
      scheme = oauth2_client_credentials
      token_endpoint = env("TEAMS_TOKEN_ENDPOINT")
      client_id      = env("TEAMS_BOT_APP_ID")
      client_secret  = env("TEAMS_BOT_PASSWORD")
      scope          = "https://api.botframework.com/.default"
      refresh_before = duration("10m")
    }
    // retry is expressed by the Loop aspect
    Loop.variant = {
      kind = measure
      initial = 3
      next = value - 1
      until = value <= 0
    }
    Loop.backoff = { curve = exponential, initial = duration("200ms") }
    on exit where data is ConnectFailed { }      // explicit iterating outcome
    rate_limit = "10/s per meta.app.session_key"
  }

  operator delivery_dispatch {

    on in {
      delivered = jq! { select(.data.delivered) }
      failed    = jq! { select(.data.delivered | not) }
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
        meta.ext.source = "teams"          // the world's half seeds meta.ext — meta.sys is read-only, even in sim
        data = {
          type         = "message"
          from         = { aadObjectId = "abc-123" }
          conversation = { id = "conv-1" }
          text         = "Bonjour"
        }
      }

      expect "turn start"     sessions.codex.in where data.method == "turn/start" && data.params.prompt == "Bonjour"
      expect "Teams activity" teams_out.in where (
        data.type == "message" && data.text.contains("Bonjour"))
    }
  }
}
```

A fixture seeds inbound state through **`meta.ext`** (the world's
zone, D-2026-08-10-1) — never
by assigning `meta.sys.*`, which is read-only **absolutely, including in
simulation** (the fixture stands in for the runtime but goes through the
same braced envelope-form constructor, it does not bypass the invariant). See
[envelope.md §6](../spec/envelope.md).

## 7. Stateful Operator + Connector mock — weather lookup

Exercises three design-intent constructs : a stateful Operator
keyed by its own instance selector, read-modify-write state semantics via
`var x` + bare-name reads (or `$x` inside jq) + `x = …`, and a
`mock` block scripting a Connector's behavior in a simulation.

```rastok
graph city_weather {
  // public ports default to in / out (no declaration)

  // Stateful Operator per session key: counts how many requests this
  // session has issued, exposed in the meta of the outgoing request.
  operator request_counter {
    instances = { key = meta.app.session_key }
    lifetime = { kind = idle; after = duration("2h") }
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
      meta.app.request_city = data.city                     // scalar Rastok expression
      data = jq! {                                           // structural build = the jq guest
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
      out = jq! {
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
      expect "request well-formed" city_weather.build_request.out where (
        data.url == "https://api.weather.example/v1/current" && data.method == "GET" && data.query.city == "Paris")

      // The public outcome :
      expect "simplified response" replies.in where (
        data.city == "Paris" && data.temperature == 17.5 && data.condition == "partly_cloudy")
    }
  }
}
```

Four things visible here :

- `request_counter.instances.key` keys the Operator's state by the incoming
  session key. The state survives across envelopes of the same canonical key
  and is reclaimed when that instance becomes idle. Composite identity is one
  expression, for example `key = { user = meta.app.user_id; workspace =
  meta.app.workspace_id }`.
- `var count : int = 0` declares the **typed state field**. It is
  read by bare `count` (or `$count` inside `jq! { … }` — jq's own
  syntax) and mutated **in place** — `count = count + 1` here — so the
  emit on the same transaction reads the new value (imperative
  intra-transaction semantics; the RHS is a Rastok expression, structural
  reshaping goes in a `jq! { … }` block).
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
instance selection, correlation and observability. The `data` remains the
payload. Constants declared with `val` make the defaults explicit
without creating mutable state.

```rastok
graph meta_control_plane {
  connector inbound : HttpServer {
    listen = ":8080/events"
    codec = json
  }

  operator normalize_meta {
    val SOURCE = "public_api"
    val DEFAULT_PRIORITY = "normal"

    on in {
      meta.app.source    = SOURCE                               // Rastok expression — vals by bare name
      meta.app.tenant_id = data.?tenant_id.orValue("public")    // absence explicit (the grid)
      meta.app.user_id   = data.user.id
      meta.app.priority  = data.?priority.orValue(DEFAULT_PRIORITY)
      data = jq! { .data.payload }                               // structural extraction = jq
    }
  }

  operator route_priority {

    on in {
      urgent = jq! { select(.meta.app.priority == "urgent") }
      normal = jq! { select(.meta.app.priority != "urgent") }
    }
  }

  connector urgent_log : File {
    path = "/var/log/rastok/urgent.ndjson"
    codec = ndjson
  }

  connector normal_log : File {
    path = "/var/log/rastok/normal.ndjson"
    codec = ndjson
  }

  inbound.out          -> normalize_meta.in
  normalize_meta.out       -> route_priority.in
  route_priority.urgent    -> urgent_log.in
  route_priority.normal    -> normal_log.in
}
```

The payload can change shape freely after normalization, but the
small scalar keys in `meta` are now stable enough to drive routing,
instance selection and audit.

## 9. Constants + keyed owners — tenant policy and per-user quota

This example combines three scopes :

- App-level constants (`DEFAULT_PLAN`, `FALLBACK_LIMIT`,
  `REJECT_REASON`) shared by the graph.
- A persistent `tenant_policy` Operator keyed by tenant.
- A `usage_gate` Operator keyed by the composite `(tenant, user)` identity.
- An explicit typed flow from policy state to quota state; no remote state read.

```rastok
graph quota_control : App {
  val DEFAULT_PLAN = "free"
  val FALLBACK_LIMIT = 1000
  val REJECT_REASON = "quota_exceeded"

  graph quota_gateway {
    // public interface declared by name — a multi-port composable graph's contract
    input  admin
    input  usage
    output policy_updated
    output approval
    output rejection

    operator tenant_policy {
      instances = { key = meta.app.tenant_id }
      lifetime = { kind = persistent }

      val PLAN_LIMITS = {
        free = 1000,
        pro = 10000,
        enterprise = 100000
      }

      var plan: text = DEFAULT_PLAN
      var monthly_limit: int = FALLBACK_LIMIT


      on update {
        plan = jq! { .data.plan // $plan }
        monthly_limit = jq! { $PLAN_LIMITS[$plan] // $FALLBACK_LIMIT }   // reads the just-written plan

        updated = {
          meta.app.tenant_id = instance.key
          meta.app.plan = plan
          meta.app.monthly_limit = monthly_limit
        }
      }

      // The policy crosses the Node boundary as data, not as a remote var read.
      on lookup {
        policy = {
          meta.app.plan = plan
          meta.app.monthly_limit = monthly_limit
        }
      }
    }

    operator usage_gate {
      instances = {
        key = {
          tenant = meta.app.tenant_id
          user = meta.app.user_id
        }
      }
      lifetime = { kind = idle; after = duration("24h") }

      var used_tokens: int = 0


      on event {
        val cost = jq! { (.data.tokens // 0) | tonumber }
        val limit = meta.app.?monthly_limit.orValue(FALLBACK_LIMIT)

        used_tokens = used_tokens + cost           // jq below reads the new $used_tokens

        approval = jq! {
          select($used_tokens <= $limit)
          | .meta.app.quota_status = "approved"
          | .meta.app.tokens_used  = $used_tokens
          | .meta.app.tokens_limit = $limit
        }

        rejection = jq! {
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
    usage -> tenant_policy.lookup
    tenant_policy.policy -> usage_gate.event
    usage_gate.approval -> approval
    usage_gate.rejection -> rejection
  }
}
```

The important part is the state boundary: each `var` is private to the Node
that declares it. `tenant_policy` carries the selected policy across its typed
`policy` output; `usage_gate` then combines that immutable envelope fact with
its own private counter. If no tenant policy exists yet, the optional metadata
read falls back to `FALLBACK_LIMIT` via `.?` + `.orValue()`.

## Conventions visible everywhere

| Construction | Form |
|---|---|
| Node instance selection | ordinary composite attribute: `instances = { key = <expr> }`; omission means one singleton in the enclosing Graph instance |
| Lifetime | independent ordinary attribute: `lifetime = { kind = persistent }`, `idle`, `fixed`, `single_pass`, or `tumbling` with the profile's duration field |
| Operator declaration | `operator <name> { ... }` |
| Connector declaration | `connector <name> : <NativeType> { ... }` |
| Graph declaration | `graph <name> { ... }`; a Graph may own `instances` and `lifetime` like any Node |
| Sub-graph instance | declaration = owner; a keyed Graph is wired by name (`compose` is not a declaration form) |
| Operator port | **Inferred from use** : `on <name>` ⇒ input, `<name> = …` ⇒ output. Declare `input`/`output <name>[: T]` only to pin a type/schema or document. `in`/`out`/`err` are conventional spellings; unused ones do not exist, and `err: E` is an explicit domain result rather than a machine-fault trap. |
| Public graph port | single-in/out → defaults `in`/`out` ; a multi-port graph declares its interface by name (`input admin`, …), optionally typed |
| Connector port | never redeclared — the Connector contract + attributes resolve its complete typed port table (`in`/`out`/`err`/`exit` for Process) |
| Type | `variant`, scalar (`text`/`int`/`bytes`/…), `struct { … }`, `map<text, V>`, `array<T>`, `enum`, `union`, `T?` |
| State field | `var <name>: <type> [= <default>]`; private to its declaring Node and serialized by that Node's logical instance mailbox |
| Connector attribute | `<name> = <value>` |
| Composite attribute | `<name> = { ... }` (`restart = { ... }`, `Loop.backoff = { curve = exponential, ... }`) |
| Handler | `on <trigger> [where … having … within … last …] { ... }` (`on (a, b)` join, `on start`, `on stop`) |
| State read (inside jq) | `$<name>` (bare `<name>` in DSL) |
| State write | `<name> = <expr>` |
| Local binding | `val <name> = <expr>` |
| Port emission | `<out_port> = <expr>` (always explicit); atomic multi-emit `(a, b) = (e1, e2)` |
| Narrowing | `<expr> as <type>` (checked; no implicit dynamic cast or privileged `err` route) |
| List / array | `[a, b, c]` |
| References | boot: `env("VAR")` / `secret("path")` in unit/App boot vals and contract-marked Connector fields; instance: local `instance.key` and enclosing lexical vals; transaction: `meta.app.x`, `data.y`, `"a" + b` — the `${…}` interpolation is gone |
| Edge | `a -> b` |
| Port-qualified edge | `a.<out_port> -> b.<in_port>` |
| Edge with config | `a -> b { capacity = 1024, backpressure = block }` |

The full grammar — including the LL(1) decision table — lives in
`docs/spec/grammar.md` § *Formal grammar*.
