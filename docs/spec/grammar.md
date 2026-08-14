# Rastok DSL — Grammar notes

> **Status: Normative M1.3 target.** The current v6 parser still accepts
> the superseded bare frontend spellings while M1.3 lands; forms not yet
> executable remain marked or refused with named deferrals
> (`E-CHECK-DEFERRED`). This document defines the destination, not a
> compatibility requirement for the old surface.
>
> **Conceptual basis** : this document assumes the model defined in [WHITEPAPER.md](../../WHITEPAPER.md). Concepts like `Operator`, `Connector`, event-driven transaction, the parameterization grid behind the named archetypes (Reducer, Aggregator, Window, Joiner), the dynamic dispatch primitives, and the partition model are introduced there. This document is the grammar ; foundations is the constitution.
>
> The `simulation` DSL is covered in this document. Formal verification,
> multi-tenancy and runtime deployment remain outside the current normative
> surface.

## File extension

`.rkg` — short for "rastok graph". Examples in `docs/*.rkg`.

## Syntax conventions — two configuration forms, no exceptions

Configuration and block declarations use two syntactic forms and
only two (D-2026-08-12-1). (Edges, `val`/`var` bindings, port and
type declarations have their own productions below — this rule
governs how a node is CONFIGURED.)

| Form | Shape | Meaning |
|---|---|---|
| **Attribute** | `name = value` | any value — scalar, composite, or tagged — assigned to a key |
| **Labeled block** | `keyword <label> [: <Type>] { ... }` | reserved-keyword declaration (`operator`, `connector`, `graph`, `simulation`, `defaults`, `on`) |

The former unlabeled block (`name { … }`, the named sub-object) is a
DEAD form: the parser still recognizes it to refuse with the
migration « write `name = { … }` ».

Rules :

- **Attributes always use `=`.** `command = "codex"`, `args = ["app-server"]`, `timeout = duration("5s")`, `env = { "HOME": "/tmp" }`. No exceptions — there are NO configuration sub-blocks (`env { … }`, `init { … }`, `tls { … }`, `auth oauth2_client_credentials { … }` are dead forms; they refuse with a migration hint, D-2026-08-12-1).
- **Type annotations always use `:`.** `input msg: struct`, `var count: int`, `connector X : HttpClient`. Never `=`.
- **An attribute value belongs to one of FOUR families** (D-2026-08-12-1,
  amended by D-2026-08-14-1): a BARE WORD from the contract's closed
  choice (`mode = job`); a Rastok EXPRESSION (`duration("5s")`,
  `asset("logo.png")`); a BRACED LITERAL — `=` entries build a struct,
  `:` entries a map (D-2026-08-10-6), replaced atomically, never
  patched; or a TAGGED FRONTEND `name!(args?) payload` whose payload
  belongs to that reader/compiler (`sql!(data) { … }`, `text! { … }`,
  `base64!(ct) { … }`, `jq!(data) { … }`, `json! <json-value>`).
  Tagged INITIALIZERS (`on_crash { … }`, `measure { … }`,
  `exponential { … }`) remain dead: a plain struct plus the attribute's
  name suffices, and a former discriminant is a closed-choice FIELD
  (`Loop.backoff = { curve = exponential, … }`).
- **Dotted attribute names are owner qualification ONLY** (`Loop.backoff`, `Saga.timeout`, `HttpClient.timeout`) — never a patch path into a composite value.
- **The host grammar is LL(1) after left-factoring the identifier
  prefixes** (D-2026-08-13-1): classification observes the form (one
  raw significant byte, or one peeked token) without irreversible
  effect; once chosen, ONE branch consumes and the cursor never moves
  backward. Sub-languages are selected by an explicit syntactic form,
  then read by their dedicated reader — each byte has one lexical
  authority, and host trivia (comments included) never changes an
  AST, a plan, or a hash. See `Formal grammar` for the decision
  table.

Example mixing all forms :

```rastok
connector codex : Process {        // type annotation with ":"
  command  = "codex"               // attribute "="
  args     = ["app-server"]        // list value
  mode     = service
  codec    = jsonl
  protocol = "jsonrpc"
  effect   = idempotent
  behavior = probabilistic
  // in / out / err / status = stdin / stdout / stderr / lifecycle (Process defaults)

  init = {                         // composite attribute : startup handshake
    message = { method = "initialize" }
    timeout = duration("15s")
  }
  restart  = {                     // struct value ; the attribute names it
    backoff = "1s..30s"
    max_attempts = 5
  }
  kill     = process_group         // enum-like ident value
}
```

## Single composite concept : `graph` — and the `: App` entry point

The `app` and `compose` keywords are not part of the grammar. A graph role
marks the deployment entry point instead.

There is **one** keyword for composite units. No `pipeline`, `flow`,
`session`, `template` — and no `app` :

```rastok
graph teams_gateway : App {          // the DEPLOYMENT ENTRY POINT — exactly one per deployed unit
  durability = local                 // app-level attributes live here
  partition session { key = meta.app.session_key ; lifetime = duration("2h") }

  connector inbound : HttpServer { path = "/teams/webhook" ; codec = json }

  graph sessions per session {       // a graph DECLARES → it IS an instance (a composite Node),
    max_instances = 1000             //   keyed by `per` like any node ; Dispatch knobs are
    spawn_policy  = lazy             //   ordinary body attributes
    connector codex : Process { mode = service ; command = "codex" ; args = ["app-server"]
                                codec = jsonl ; correlate = data.id
                                effect = idempotent ; behavior = probabilistic }
    in -> codex.in
    codex.out -> out
  }

  connector outbound : HttpClient { url = env("TEAMS_URL") ; codec = json
                                    effect = idempotent ; behavior = deterministic }

  inbound.out -> sessions.in         // graphs WIRE BY NAME, like every node
  sessions.out -> outbound.in
}
```

The rules :

- **A project is many files, many graphs.** A file's top level holds
  declarations (`graph`, `lab`, `val`) — there is no implicit per-file
  root graph.
- **Unit formation** : a
  **deployed unit** is the set of source files the compile command selects
  together — a project may hold several units. The exactly-one-`: App` rule
  below is checked per unit. The unit's simulations, experiments and
  theorems live in the **`lab`**
  whose name matches the App's ; each `simulation` names its closed graph
  explicitly (`on app` today — sub-graph targets are deferred, E-SIM-TARGET)
  ([simulation-semantics.md](simulation-semantics.md) §2, Boundaries).
- **`: App` is a graph ROLE** (the third annotation category beside traits
  and aspects) : it marks the deployment entry point. A deployed unit has
  **exactly one** `: App` graph (0 or ≥2 = build error). App-level concerns —
  `durability`, retention overrides, shared partitions — are its attributes
  and body. `app` is no longer a keyword; it survives as a **pre-bound
  identifier** aliasing the `: App` graph, so a declared App boundary port
  remains addressable as `app.<port>`. M1.3 removes the special `app.err`
  root sink: observation and bubbling are resolved per output contract.
- **Durability is lexical.** A stateful Node's effective value is its
  explicit `durability`, otherwise the closest enclosing Graph's explicit
  value, otherwise the App's value, otherwise `none`. A child may strengthen
  or weaken the inherited value; weakening emits
  `W-DURABILITY-WEAKENED` and is included in coverage output.
- **Declaration = instantiation, for every family.** A declared graph is a
  composite Node in its enclosing scope, wired **by name** — exactly like an
  Operator or Connector. A declared graph that is never wired = dead-code
  warning (the unfed-input family).
- **`per` is uniform** : it keys the instances of any node — operator,
  connector, or graph (`graph sessions per session` = one instance per key ;
  Dispatch, unchanged semantics). The Dispatch knobs (`max_instances`,
  `spawn_policy`) are body attributes of the keyed graph.
- **Identity guard** : the `: App` graph is the composition **root** ;
  `node_path` is relative to the root and the root's own name is
  **excluded** ([envelope.md §2](envelope.md)).
- **Reuse note** : instantiating one graph *type* at several sites is
  deferred. If introduced, it can use
  `graph <name> : <GraphType>` — the annotation slot and the symbol
  classifier already accommodate it. `compose` and `app` are
  reserved-but-not-used.

## Vocabulary : Node, Operator, Connector, Graph, inputs/outputs, state

Rastok recognizes exactly **three families of node** ([WHITEPAPER.md §2](../../WHITEPAPER.md)) :

- **Operator** — computation internal to the graph. Reacts to envelope events, optionally reads/updates state keyed by a partition, emits zero or more envelopes.
- **Connector** — same graph shape as Operator, but it is allowed to talk to an external system (HTTP, processes, files, databases, clocks, brokers, etc.). Its external configuration is expressed as flat attributes and typed ports, not as mandatory subsections.
- **Graph** — composite Node. Its body declares Nodes and edges ; the graph declaration itself is the instance wired by name.

A concrete Node is declared with `operator`, `connector`, or `graph`. By
convention formal implementation types end in `-Operator`, `-Connector`, or
the canonical `Graph` type :

```rastok
JaqOperator, WasmOperator, ReducerOperator,
HttpClient, HttpServerConnector, ProcessConnector,
File, Postgres, Kafka, Clock, MCPConnector, Graph, …
```

The stdlib exposes shorter aliases for everyday writing : `Jaq`, `Reducer`, `Aggregator`, `HttpClient`, `Process`, `File`, etc. They desugar to the formal type names.

Every other concept (Reducer, Aggregator, Window, Joiner) is just an Operator parameterized along three orthogonal axes : presence of `var` state, presence of `per`, and the handler trigger (`on <port>` vs the `on (a, b)` join). See [Operator parameterization](#operator-parameterization-state--per--transaction) below.

The word **Processor** is not part of the canonical vocabulary. Runtime traits
such as `Reader`, `Transformer`, and `Writer` may exist in an implementation,
but the DSL exposes Nodes, `input` / `output` endpoints (with the default
`in` / `out` / `err` ports), and (for Connectors) external configuration.

Supporting concepts :

- **Node** — any element of a graph : Operator, Connector, or Graph.
- **input / output** — typed envelope endpoints declared with `input` /
  `output` inside a Node body (plus the default `in` / `out` / `err`
  ports). Connected to other Nodes via DAG edges.
- **state binding** — a Node's typed access to a partition's key instance. Declared via `per <partition>` on the Node. *Not* an arrow in the DAG ; it is a binding to a partition-keyed state cell held by the runtime.
- **external configuration** — (Connector only) attributes such as URL,
  path, command, codec, protocol, timeout, retry policy, etc. A grouped
  configuration is a COMPOSITE attribute (`env = { … }`,
  `jwt = { cache_ttl = … }`, `tls = { … }`) — one value, replaced
  atomically; there is no namespace sugar and no flat-prefix
  flattening (D-2026-08-12-1).

This keeps protocol layers separate :

```text
transport = stdio | http | websocket | tcp | file | memory
codec     = jsonl | json | sse | text | bytes | multipart
protocol  = jsonrpc | teams_activity | mcp | log | raw
```

HTTP and JSON-RPC are therefore not the same layer : HTTP is a transport/application binding, JSON-RPC is a message protocol that can run over `stdio+jsonl`, WebSocket text, HTTP JSON, etc.

Example : a `Process` node for `codex app-server` owns one child process
address, binds `stdin/stdout/stderr` to `in`/`out`/`err`, and exposes an
additional typed `status` output. The compiler lowers those ports to
runtime Reader/Writer halves that share one process handle and one
correlator.

`HttpServer` and `HttpClient` are deliberately separate node types because
they have different external roles even though both can have read and write
directions :

```text
HttpServer
  world -> connector.out              // inbound HTTP request → envelope (out = graph receives)
  connector.in -> world               // envelope → HTTP response (in = graph sends)

HttpClient
  connector.in -> world               // envelope → outbound HTTP request
  world -> connector.out              // HTTP response → envelope
```

### Cardinality is declared on the graph, like an Operator's `per`

A graph is a Node, so — exactly like an Operator — it declares its own keying
with `per <partition>` on the **definition**. Cardinality is intrinsic to the
graph, not re-decided at every use site :

- **`graph X { … }`** (no `per`) → **singleton**. One running instance (the
  whole-service top-level graph, or one shared sub-graph instance fed by all callers).
- **`graph X per <partition> { … }`** → **per-key** (the `Dispatch` primitive).
  The runtime materializes one running instance per distinct partition key ; the
  partition's `lifetime` governs eviction.

This mirrors `operator counter per session` : every Node (operator, connector,
graph) declares its keying the same way. A graph keyed `per session` carries its
per-session data as **graph-level `val`** (immutable, derived from
`session.key`) ; mutable per-session state stays in an operator `var per
session` inside it. (A *shared* graph-level `var` across operators is deferred —
it depends on the per-key concurrency model.)

> Trade-off (accepted) : a `per`-graph is no longer reusable as a singleton at a
> different site — but neither is an `operator per …`. Keying is a property of
> the Node, decided once. A genuinely cardinality-agnostic graph omits `per`
> (singleton) ; the rare both-ways case uses a thin wrapper.

### Graph composability

A graph references another graph **by name, as a node** — the third node
family, not a special construct :

```rastok
graph sessions per session {
  // `per session` → one instance per session key (Dispatch). Single in/out → default ports.
  // Per-session config lives here as graph-level `val`, derived from the key :
  partition session {
    key = meta.app.session_key
    lifetime = duration("2h")
  }
  val codex_home = "/var/codex/" + session.key
  val cwd        = "/workspace/" + session.key

  connector codex : Process {
    command  = "codex"
    args     = ["app-server"]
    mode     = service
    cwd      = cwd                        // graph-level val, by name
    env = { "CODEX_HOME": codex_home }

    codec    = jsonl
    protocol = "jsonrpc"
    effect   = idempotent
    behavior = probabilistic
    // in / out / err / status = stdin / stdout / stderr / lifecycle (Process defaults)

    init = {
      message = { method = "initialize" }
      timeout = duration("10s")
    }
    kill  = process_group
  }

  in        -> codex.in
  codex.out -> out
}

graph teams_inbound {
  // Top-level → implicitly singleton.
  connector inbound : HttpServer {
    listen = ":8080"
    codec  = json
  }

  // `sessions` — declared above, at the unit's top level — is
  // wired BY NAME : the declaration IS the instance ; Dispatch knobs
  // (max_instances, spawn_policy) are its body attributes.
  inbound.out -> sessions.in
  sessions.out  -> ...
}
```

A declared graph is an instance ; its key, cardinality and lifetime come from
its own `per <partition>` ; the **Dispatch knobs** are body attributes :

| Field | Required | Effect |
|---|---|---|
| `max_instances` | optional | Cap on concurrent per-key instances. Default applied otherwise (e.g., 1000). |
| `spawn_policy` | optional | `warm_per_key` (default) / `lazy` / `pre_spawn` / `pool` |

Composition rules :
- A graph's `input <name>: <type>` declarations become inbound ports on the composed graph node.
- A graph's `output <name>: <type>` declarations become outbound ports on the composed graph node.
- A composed graph is still a Node — of the **Graph family** (never reclassified as an Operator or Connector). Effect-wise it is *pure* when it contains no Connector and *effectful* when it wraps a boundary ; that property propagates to analyses, not to its family.
- A graph that contains a node binding a unique resource (HTTP listener, TCP listen, etc.) **cannot** be composed with `key` : the validator rejects "this graph cannot be replicated, port :X". Such a graph is forced top-level (singleton).

DAG validation runs **flat** : after expansion, the runtime composes one giant graph and Kahn rejects cycles across composition boundaries too.

### Graph scope, visibility and liveness

Braces and names carry ALL the semantics — indentation carries none.

- **Placement.** The `: App` graph is the unit's **composition root**. Every
  top-level `graph` declaration of the unit is instantiated and belongs to
  the deployment **as a child of that root** for pathing purposes. `node_path`
  is root-relative with the root's name excluded — so a top-level graph's
  path is its own name (`sessions.codex`), and a nested graph's path is
  `outer.inner.node`.
- **Visibility.** The unit's top level is ONE namespace : top-level names
  are wireable from every graph body of the unit (the gateway wires the
  sibling `sessions` from inside its App). A *nested* graph's name is
  lexical : visible only inside the body that declares it. Name shadowing
  of a top-level name by a nested declaration is a compile error (no
  silent capture).
- **Liveness.** A graph with **no inbound path** — no edge reaches it or
  its children from outside, and it contains no source Connector (a
  Connector producing INBOUND records) — is **dead** : diagnostic
  `W-DEAD-GRAPH` (warning, not an error — work-in-progress graphs are
  legitimate), and its boundary-port wiring obligations are suspended.
  Any partial wiring, or any source Connector inside, re-arms the normal
  resolved `UnwiredPolicy` of every output (`require_wire`, `journal`,
  `bubble(port)`, or explicit discard with `-> nothing`).

## Operator parameterization : state + per + transaction

Per [WHITEPAPER.md §2.3](../../WHITEPAPER.md), what informal vocabulary called "Reducer", "Aggregator", "Window", "Joiner" are **the same Operator** parameterized along three orthogonal axes :

| Axis            | Values                                | Effect                                          |
|-----------------|---------------------------------------|-------------------------------------------------|
| `var` state     | absent / typed `S`                    | Operator becomes stateful                       |
| `per`           | absent / one key / multiple keys      | State is partitioned per partition key instance |
| handler trigger | `on <port>` / `on (a, b)` (join)      | Determines when the Operator fires (§2.3 guards) |

The named archetype aliases map mechanically to combinations of these axes :

| Alias          | Equivalent Operator parameterization                                       |
|----------------|----------------------------------------------------------------------------|
| Pure transform | no `var`, no `per`                                                         |
| Reducer        | `var S`, no `per`                                                          |
| Aggregator     | `var S`, `per K`                                                           |
| Window         | `var S`, `per K`, partition has `lifetime = window D mode`                 |
| Joiner         | `var S`, `per K`, an `on (a, b)` join handler (bounded by `last`/`within`) |

These names are kept in the stdlib **as aliases** for the corresponding `Operator + parameters` combinations. The compiler lowers them all to the same underlying trait. Example aliases :

```
alias Jaq        = JaqOperator
alias Reducer    = JaqOperator                 // alias used when state is present
alias Aggregator = JaqOperator                 // alias used when state + per are present
```

Less frequent patterns (filter-with-dedup, buffer-by-size, rate-limit, debouncer, memoize-cache, …) are expressed as the appropriate Operator parameterization plus jaq logic — they do not need their own DSL keyword.

### State semantics — read-modify-write per transaction

State is **read-modify-write, imperatively, within a transaction**. A persistent field `var x: T = default` is read by its bare name `x` (or `$x` inside a `jq! { … }` block — jq's own convention). At the *start* of a transaction `x` holds the value the previous transaction committed (or the default if uninitialized) ; assignments mutate it **in place**, so a later read in the same transaction sees the new value, like an imperative function. The transaction commits atomically (state + emissions + audit) at its end.

```rastok
operator counter {
  var count: int = 0                          // default 0 on first transaction
  // no `per` → single state cell shared across all envelopes (graph-scoped)

  on msg {
    count = count + 1                         // Rastok expression; later reads see the new value
    out = { meta.app.count = count }          // default 'out' port
  }
}
```

**State expressions are Rastok expressions**: `count = count + 1`, `x = x * 2 + data.n`.
There are no mutation shortcuts (`x++` / `x += v`) ; they are redundant with
ordinary expressions. Structural reshaping may use tagged frontends
(`jq! { … }` / `sql! { … }`).

**Emitting** : a braced value is classified ONCE by its top-level roots (D-2026-08-09-1). With only the reserved roots `data` and `meta` it is the ENVELOPE form : `out = { meta.app.count = count }` inherits the input envelope (meta + data) and patches the listed fields, and a whole-`data` assignment makes the data fresh. With no reserved root, `out = { total = n }` builds a fresh data record (a `struct`), keeping the input's heritable metadata per the uniform propagation rule (D-2026-08-10-2 ; `content_type` excluded — D-2026-08-10-4). Mixing the two refuses with `E-BRACE-MIXED` — business fields named `data` or `meta` take the explicit escape `data = { … }`. `out = .` re-emits the input envelope unchanged ; `out = nothing` emits nothing on that port this transaction. There is no `previous(x)` operator — a transaction reads the previous transaction's committed state at its start, then proceeds imperatively. The synchronous-dataflow `pre` of Lustre/SCADE applies at the **inter-transaction** boundary (each transaction is one cycle), not inside a transaction. Wall-clock time enters only through a `Clock` observed via `on <clock_port>` (see [WHITEPAPER.md §5](../../WHITEPAPER.md)).

## Dynamic dispatch primitives (Router, Dispatch)

The graph topology is statically declared (DAG, no cycles, every edge visible in source). Dynamism is recovered through two primitives, each of which preserves static analyzability : *what* happens at runtime is dynamic, *where* it can possibly happen is statically declared.

| Primitive  | Selection mode                    | Topology contract                                  |
|------------|-----------------------------------|----------------------------------------------------|
| **Router** | Runtime choice 1-of-N or list-of-N | All N output ports are declared statically         |
| **Dispatch** | Per-key sub-graph instantiation | The sub-graph type is declared ; instances per key |

These are the only sanctioned ways to express runtime decisions about destination. A node that "decides who to message" outside of these primitives is rejected by the validator. This is what keeps the topology analyzable, the path coverage finite, and LLM-driven graph generation tractable.

### Boundary sugar `.` and `.foo` for graph wiring

The wiring grammar accepts a dot-prefixed `port_ref` to refer to the **current graph's boundary** :

| Form | Meaning | Position |
|---|---|---|
| `.` | the graph's default boundary port — `in` when used as edge source, `out` when used as edge target | leftmost / rightmost of a chain, or as a single edge endpoint |
| `.foo` | the graph's explicitly declared port `foo` (must match an `input foo` or `output foo` declaration in the body) | anywhere a `port_ref` is legal |

This mirrors the **jq convention** : in jq, `.` is "the current envelope" and `.field` accesses a property ; in wiring, `.` is "the current graph" and `.foo` accesses a named boundary port. Same mental model, different scope.

```rastok
// without sugar
graph payment : Saga {
  . -> reserve -> authorize -> pay -> .
}

// with `.` sugar — boundary inferred by position
graph payment : Saga {
  . -> reserve -> authorize -> pay -> .
}

// with `.foo` sugar — explicit named boundary ports
graph webhook {
  input  request  : HttpRequest
  output response : HttpResponse
  output error    : ErrorReport

  .request -> handler -> .response
  handler.err -> .error
}
```

**Static rule** : `.` (bare) is **only** legal at the leftmost or rightmost position of a chain edge, or as the source / target of a one-hop edge. A `.` in an intermediate position (`a -> . -> b`) is a compile error — the boundary cannot be re-entered mid-flow. `.foo` (named) has no such restriction ; it is a regular port reference.

**Why this is safe LL(1)** : at the start of a `top_decl` or `graph_body` item, seeing `.` immediately identifies an `edge_decl` whose source is a boundary `port_ref`. The parser does not need to peek further than the next token to disambiguate from value-position `.` (which only appears after `=`).

---

Broadcast to multiple outputs is not a separate primitive — it is the
broadcast form of the atomic multi-emit statement :

```rastok
operator notify {
  on in {
    (sms, email, push, slack) = in       // same envelope to every declared output
  }
}
```

This subsumes the `Fanout` alias entirely and composes naturally with conditional logic (broadcast only some envelopes, not others) without needing an upstream `Router` to split the cases.

The "switch on a value" pattern (forward an envelope to one of a declared set of targets based on a field value) is expressed as a degenerate `Router` whose selector returns that field :

```rastok
operator forward : Router {
  select = jq! { .meta.app.target }
}
forward.a -> a_handler
forward.b -> b_handler
forward.c -> c_handler
```

The detailed syntax for each primitive is documented in the existing sections on multi-port nodes ([Router](#port-naming-and-default-edges)) and composed graphs ([Dispatch](#graph-composability)).

## Partitions in the DSL

The model defined in [WHITEPAPER.md §3](../../WHITEPAPER.md) is exposed to the DSL through a small stdlib of pre-declared partitions plus the ability to declare custom ones inline. There is no parent-child relationship between partitions ; each partition is a flat `(key, lifetime)` declaration. A Node attaches its state to a partition with the `per` keyword :

```rastok
operator session_counter per session {
  var count: int = 0

  on msg {
    count = count + 1
    out = { meta.app.count = count }
  }
}
```

The `per` keyword takes one identifier (single-key partition) or several comma-separated identifiers (composite-key partition) :

```rastok
operator user_workspace_quota per user, workspace {
  var quota: int = 0

  on msg {
    quota = jq! { $quota + .data.cost }
  }
}
```

Without `per`, the Node has a single state cell shared across all envelopes (graph-scoped).

Standard partitions (provided by the stdlib) include `tenant`, `session`, `request`, `user`, `workspace`. Runtime-managed envelope metadata (the 5-field `meta.sys` Tier 1 : `id`, `parent_id`, `origin_id`, `role`, `graph_version` — see [envelope.md](envelope.md)) is read-only under `meta.sys.<field>` — the envelope is not a partition.

Custom partitions are first-class :

```rastok
partition workspace {
  key      = meta.app.workspace_id
  lifetime = duration("4h")
}
```

A bare duration value as a lifetime is interpreted as `idle_ttl`. For other lifetime kinds, use the explicit form : `persistent`, `single_pass`, `fixed duration("…")`, `window duration("…") <mode>` — the structural words are DSL, the durations are ordinary Rastok expression values.

## Naming convention

| Element | Convention | Example |
|---|---|---|
| Declaration keyword | `graph` / `operator` / `connector` | `graph teams_inbound { ... }`, `operator classify { ... }`, `connector inbound : HttpServer { ... }` |
| Graph names | snake_case (graphs are named like modules, not like node types) | `teams_inbound`, `codex_session`, `api_to_s3` |
| Node types | PascalCase | `HttpServer`, `HttpClient`, `Jaq`, `Process` |
| Node instances | snake_case | `inbound`, `to_codex`, `client_ws`, `sessions` |
| Ports | snake_case, role-named, **no direction suffix** (direction from `input`/`output`) ; `in`/`out`/`err` are defaults | `event`, `delta`, `flush`, `matched` |
| Config keys | snake_case | `listen`, `nonce_ttl`, `jwks_url` |

Node type names describe a stable external role or a pure graph primitive.
No `Reader`, `Writer`, or `Endpoint` suffix is needed in the DSL :

| Type | Role is inferred from | Examples |
|---|---|---|
| `HttpServer` | server address/binding | inbound request on `out`, optional response on `in` (default ports) |
| `HttpClient` | client address/binding | outbound POST, GET download, delivery/report output |
| `Process` | typed stdio + lifecycle ports | default `in`/`out`/`err`/`status` = `stdin`/`stdout`/`stderr`/tagged lifecycle, one child-process lifecycle |
| `Jaq` | its transformer contract | pure `in -> out` envelope transform |
| `Memory` | its fixed store contract | `in` appends; `read` requests a bounded snapshot on `out` |
| `File` (deferred) | its future ports and configuration | read side, write side, or both |

Rust implementation structs may use explicit names (`FileReader`,
`JsonRpcWriter`, etc.). The NodeRegistry exposes suffix-free DSL type names
and chooses the runtime halves from the node's ports and Connector attributes.
Role and duplex behavior are properties of ports and traits, not of the
user-facing node type.

## Port naming and default edges

**Ports may be inferred from the handler.** `on <name>` contributes an input ;
assigning `<name> = …` in a handler declares an output. So Operators
usually declare *no* ports at all — the `input` / `output` keywords are
optional, needed only to **pin** a type/schema (`output x: T`), or to
document a node with no handler (e.g. a saga) or a graph's public
interface. Direction is carried by the keyword, not the name. The names
**`in` / `out` / `err`** are conventional default ports — pre-bound but
typed by the same mechanism as every other port. A `Process` maps them
exactly to stdin, decoded stdout and raw stderr; its ordinary `status`
output carries the tagged lifecycle/outcome value. Conventions :

- ports are named for their role, **no direction suffix** (`event`, `delta`, `flush`)
- a single-input / single-output node just uses the defaults `in` and `out`
- a port's **type is inferred** ; annotate (`output x: T`) only to pin / force
- Connector contracts declare fixed or parametric port rules; the
  instantiated contract plus codec resolves their effective types
- `err` conventionally carries diagnostic bytes/data; it has no special
  grammar, type or transaction semantics

Every output is wired explicitly or sent to its resolved sink policy:

```rastok
client.out -> codex.in
codex.out      -> client.in
codex.err      -> logs.in
codex.status   -> lifecycle.in
```

Multi-port nodes (`Router`, `Aggregator`, ...) expose additional named ports
via the same `.<name>` syntax — no separate `ports { ... }` declaration block
needed :

```rastok
operator route : Router {
  select = jq! {
    if .data.kind == "invoice" then ["audit", "invoice"]
    elif .data.kind == "refund" then "refund"
    else []
    end
  }
}

route.audit   -> audit_log.in
route.invoice -> invoice_handler.in
route.refund  -> refund_handler.in
```

The available port names come from the node contract (`Router` documents what `select` outputs and how it maps to ports). A `Router` selector may return :

- a string : one route (`"refund"`)
- an array of strings : fan-out to several routes (`["audit", "invoice"]`)
- an empty array : explicit drop

The envelope is never transformed by `Router`; every selected route receives
the same envelope. If several upstream edges target the same input port, the
runtime treats that as an implicit funnel / fan-in into the port. That default
merge carries no guard or ordering policy. A guarded fan-in such as "wait for
`a`, `b` and `c` for the same key before emitting" is expressed by an
`on (a, b, c)` join handler (§2.3), bounded by `last` (and optionally
`within`) — not the default funnel.

`Aggregator` is the stateful counterpart for keyed folds : fragments arrive
on a declared input such as `.delta`, flush signals arrive on `.flush`,
and the aggregate is emitted on a declared output such as `.aggregate`.

The explicit `Router` node is the canonical form. The parser may also accept
an inline sugar for local routing :

```rastok
codex.out -> route jq! {
  if .meta.app.method == "item/agentMessage/delta" then ["aggregate", "typing"]
  elif .meta.app.method == "turn/completed" then "flush"
  else []
  end
} {
  aggregate -> aggregate_reply.delta
  typing    -> typing_feedback.in
  flush     -> aggregate_reply.flush
}
```

This desugars to an anonymous `Router` plus normal edges. A future structural
pattern form can share the same lowering :

```rastok
codex.out -> {
  { meta: { method: "item/agentMessage/delta" } } -> aggregate_reply.delta, typing_feedback.in
  { meta: { method: "turn/completed" } }          -> aggregate_reply.flush
  _                                               -> nothing
}
```

## Edges

### `->` — unidirectional

```
a -> b
```

Equivalent to the source's single declared output port wired to the target's
single declared input port. This is a convenience for unambiguous nodes, not an
edge between ports literally named `out` and `in`.

### Discarding output : `-> nothing`

There is **no `Drop` node**. To discard a stream, wire it to the built-in sink
`nothing` :

```
codex.err            -> nothing       // explicit discard (non-blocking drain)
delivery.delivered   -> nothing
```

`nothing` is the edge-level twin of `out = nothing` in a handler : the handler
form means *"this transaction emits nothing"* (source side) ; `-> nothing` means
*"this output stream goes to /dev/null"* (sink side). The drain **always
consumes**, so it never applies back-pressure.

**An emitted output port that is left unwired is a compile warning** — it is
ambiguous and, with bounded queues, dangerous : under `backpressure = block` an
unconsumed port would stall (deadlock) its producer ; under a drop policy it
would lose data silently. Resolve it explicitly — wire it to a consumer, or to
`nothing`. (The runtime default for an ignored warning is specified in
`docs/spec/delivery-semantics.md` ; the recommendation is *drop*, since blocking a
whole partition on a forgotten wire is the worse failure.)

### Bidirectional wiring

Write the two directions explicitly :

```rastok
client.out -> agent.in
agent.out    -> client.in
agent.err    -> logs.in
```

These are two independent envelope streams. They do not mean "the same envelope loops back"; each direction starts at an envelope output port and ends at an envelope input port. The compiler can lower a multi-role node into separate runtime Reader/Writer halves for validation, while still keeping one user-facing node identity for config, address, lifecycle and observability.

### Chains

```rastok
a -> b -> c             // a -> b ; b -> c
client.out -> agent.in
agent.out    -> client.in
```

For complex graphs, prefer one edge per line.

### Explicit ports

```rastok
agent.err    -> logs.in
client.out -> agent.in
agent.out    -> client.in
```

Ambiguity-free — every edge says exactly which port on each side.

### Branchements (fan-out / fan-in)

Real DSL primitives, not just sugar :

```
parse -> {
  store
  audit
  metrics
}

# == parse -> store ; parse -> audit ; parse -> metrics
```

```
{
  api
  cron
  worker
} -> sink

# == api -> sink ; cron -> sink ; worker -> sink
```

Used for fan-out (one source to many destinations) and fan-in (many sources to one destination — funnel). The mpsc multi-producer semantic makes fan-in cost-free.

### Trace by branchement

No `tap` / `observe` keyword. To trace a flow, just branch to a Writer :

```
parse -> {
  store
  traces
}
```

For a duplex bridge, write the two arrows and branch each :

```
client.out -> {
  agent.in
  traces.in
}
agent.out -> {
  client.in
  traces.in
}
```

The runtime can attach `meta.ext.direction = client_to_agent | agent_to_client` to differentiate (a sheet-declared ext field).

### Edge config

A `{ ... }` block after an edge configures graph-local flow control. It does
not describe wire serialization ; `transport`, `codec`, and `protocol` belong
to node `binding` sections.

```rastok
client.out -> agent.in { capacity = bounded(1024), backpressure = block, close = cascade }
agent.out -> client.in { capacity = bounded(1024), backpressure = drop_oldest }
```

Edge config keys (target schema) :

| Key | Values | Default | Purpose |
|---|---|---|---|
| `capacity` | `bounded(N)`, `unbounded` | `bounded(64)` | mpsc queue size |
| `backpressure` | `block`, `drop_oldest`, `drop_newest`, `error` | `block` | what happens when downstream can't keep up |
| `close` | `cascade`, `mirror`, `none` | `cascade` | propagation of EOF/close signal |
| `trace` | `off`, `summary`, `full` | `off` | observability granularity |

graph-level defaults can be set once :

```rastok
graph foo {
  defaults {
    capacity     = bounded(64)
    backpressure = block
  }
  ...
}
```

## DAG strict — cycles are a compile error

A `graph` **must** be a DAG. Writing a cycle :

```
a -> b
b -> a
```

is a **compile error** from the DSL parser. Same for indirect cycles. The parser runs Kahn's algorithm at parse time and refuses to emit a runnable graph if a cycle is detected.

Bidirectional communication is modeled as two independent streams between explicit ports. The validator reasons over the lowered Reader/Writer halves, not over the external resource as a single cyclic node.

### What we gain by being DAG-strict

- **Finite path length:** topology alone cannot make an envelope circle;
  handler and external-call termination remain separate obligations.
- **Provenance and replay:** causal paths have a topological order, while
  the journal records arrival races.
- **No cyclic wait introduced by a back-edge:** joins, bounded queues, and
  Connector timeouts still require validation.
- **Bounded queued flow:** `Σ edge.capacity` bounds edge slots, excluding
  declared Node state and Connector-owned buffers.
- **Static visualization:** layered layouts remain readable.

### Idiomatic patterns for things that look like cycles

Writing two arrows in opposite directions gives a compile error. Express the same intent without a cycle :

| Need | Pattern |
|---|---|
| Retry after a selected outcome | The `: Loop` aspect on the Connector, with an explicit handler/guard over the connector's typed outcome port (for Process, normally `status`) — see [loop.md](loop.md) |
| DLQ + replay | Failed → `Memory`; operations tooling reads a bounded snapshot, then injects selected values as roots of a new run |
| Iterative refinement, fixed N | Static unfolding : `step -> step -> step` |
| Iterative refinement, dynamic N | the bounded `: Loop` aspect |
| Backpressure feedback | Already implicit via bounded mpsc — no edge needed |
| Saga state machine | Internal state in a SagaCoordinator node, or unfolded chain |

These idioms are **the** way to express the corresponding patterns. They're not workarounds — they're the expected vocabulary.

## Tagged frontends and guest languages

The Rastok DSL is the **host language**. `IDENT!` selects a dedicated
reader/compiler; a guest-language body is compiled or validated into a
`GuestProgram`. Rastok expressions are intrinsic and do not use this
mechanism. Placement follows effects ([stdlib.md](stdlib.md) §5) :
a pure guest program executes inside an Operator ; a guest program with I/O
executes behind a Connector. Wasm is an execution format for either case, not
by itself a guest language or a purity guarantee. An Operator guest whose
imports exceed the pure host ABI is a compile error.

Guest-language source uses the **tagged frontend** syntax:

```rastok
operator to_codex : Jaq {
  program = jq! {
    if .data.type == "invoke" then
      .data = { decision: .data.value.verb }
    else . end
  }
}
```

Rules:
- Frontend name and bang (`jq!`, `bash!`, `sql!`, `python!`, …) appear
  before the frontend-owned payload
- The selected frontend owns the source region (no `${...}` interpolation,
  no host-language comments inside it)
- A braced frontend may use balanced `{}` only when its registered framer
  declares that contract; guest-specific strings and comments belong to
  the guest's framer, never to a universal host scanner
- Tooling hint : `.rkg`-aware LSP can route the block to a matching language server

Generalizes uniformly :

```
connector cleanup : Shell    { script = bash! { rm -rf /tmp/work } }
connector audit : Postgres   { insert = sql!  { INSERT INTO audit_events ... } }
operator custom : Wasm       { source = rust! { pub fn transform(env: Envelope) -> Envelope { ... } } } // pure ; I/O imports require a Connector
```

Bindings use ordinary frontend arguments: `expr = jq!(data) { .type }`.

## jaq operates on the full envelope

Inside `Jaq { program = jq! { ... } }`, the input visible to the program is the
**full envelope** in its canonical `meta + data` shape :

```json
{
  "meta": {
    "sys": {
      "id":            "9f2c…",
      "parent_id":     "3a17…",
      "origin_id":     "0b44…",
      "role":          "default",
      "graph_version": 42
    },
    "app": { "user_id": "user@example.test" },
    "ext": { "content_type": "application/json" }
  },
  "data": { ... }
}
```

(The `meta.sys` Tier 1 is fixed in [envelope.md](envelope.md). Tier-2
`saga_*` / `loop_*` appear only inside the corresponding aspect
scope. All ids are blake3-derived 128-bit values.)

| Field | Writable by jaq |
|---|---|
| `data` | ✅ |
| All of `meta.*` | ❌ — guests READ metadata, never write it (v1, D-2026-08-10-1): metadata writes belong to the DSL, where the checker sees every key |

`meta` has exactly THREE zones (D-2026-08-10-1): `sys` (runtime,
read-only), `app` (application, read-write, inferred from the deployed
graph's writes), and `ext` (external-boundary context, read-write,
sheet-declared). Each zone is a flat struct whose leaves are scalars or
`array<scalar>`; nested composites belong in `data`. There is no bare
form: `meta.customer_id` is invalid. Types unify across all writers of an
`app` field; `ext` fields come from Connector contracts plus the standard
`content_type`. `meta.sys` is the fixed five-field Tier 1 plus
scope-conditional aspect fields; Connector outcome data lives on its
ordinary typed output port, not in `meta.sys`. Absent-field semantics are
checked by the Rastok expression grid: an undeclared leaf is a compile
error, a required read after an ambiguous join merge must be resolved by
an explicit write, and legitimate absence uses `.?` / `.orValue(...)`.
The jq rule *missing ≡ null* applies only inside `jq!` programs. On the
wire an absent key is omitted, never serialized as `key: null`.

## Data : typed values and literals

An envelope's data is a single **typed value** from the value-type tree
([WHITEPAPER §2.1](../../WHITEPAPER.md)) : `variant` (dynamic, the JSON
model), a `scalar` (`text`, `bytes`, the numeric `int`/`long`/`float`/
`double`/`number`, `bool`, plus logical `date`/`timestamp`/`uuid`/
`decimal`/`duration`), or a `composite` (`struct`, `map`, `array`). In
the DSL an explicit data literal uses the tagged-block / list pattern :

| Type      | Literal form                                  | Notes                                                          |
|-----------|-----------------------------------------------|----------------------------------------------------------------|
| `struct`  | `{ k = v, … }` ; `{}` empty                   | typed record — `=` decides (D-2026-08-10-6) ; schema inferred, or annotated `: <Schema>` |
| `map`     | `{ "k": v, … }` ; `{:}` empty                 | text keys — `:` decides ; mixing `=` and `:` refuses           |
| `array`   | `[ v, v, … ]`                                  | ordered sequence ; `array<struct>` is the relational `table` view |
| `text`    | `"…"` ; `text! { … }`                         | Rastok string, or the verbatim frontend — first newline excluded, margin set by the closing delimiter, no interpolation |
| `bytes`   | `base64! { … }` ; `asset("path")`             | RFC 4648 standard, mandatory padding, ASCII whitespace ignored ; or build-time repo bytes (D-2026-08-10-5). `hex` and `bytes …` forms are DEAD |
| `variant` | `json! <json-value>`                           | the complete JSON value grammar (object, array, string, exact number, boolean, null); always `variant`; duplicate keys refuse |
| whole payload | `asset("path", "content_type")` ; `base64!("ct") { … }` | fresh bytes + `meta.ext.content_type`, whole-data position ONLY (a nested field has no `meta.ext`) |

The `<type> file { … }` forms are DEAD — `asset` replaces them for
build-time resources ; runtime files and URLs are Connectors'
business.

Examples :

```
// Typed record (struct) ; schema inferred
{ city = "Paris", country = "FR" }

// Rows : an array of structs (the relational 'table' view)
[
  { name = "Alice", age = 30, city = "Paris" },
  { name = "Bob",   age = 42, city = "Rome"  }
]

// Rows with an explicit element schema
array<struct { name: text, age: int, city: text }> [
  { name = "Alice", age = 30, city = "Paris" },
  { name = "Bob",   age = 42, city = "Rome"  }
]

// Text — the Rastok string, or the verbatim frontend (margin set by the
// closing brace; no escapes, no interpolation)
"INFO: agent started"
text! {
    INFO: agent started
    INFO: thread created
}

// Bytes — inline base64 (RFC 4648 standard, mandatory padding), or
// build-time repo bytes
base64! { 3q2+7wEC/w== }
asset("fixtures/sample.pdf")

// Dynamic / semi-structured → variant : any complete JSON value
json! { "anything": [1, 2, 3] }
json! null
```

The pre-reform spellings — `bytes base64 { … }`, `bytes hex { … }`,
`json { { … } }`, `<type> file { … }` — are DEAD (D-2026-08-10-5/-6);
runtime files and URLs are Connectors' business.

`asset("path")` reads at BUILD time, relative to the App root, and the
content digest is identity (D-2026-08-10-5). Runtime files and URLs go
through Connectors, whose **codec** decodes the wire format (Parquet,
Avro, JSON, …) into the declared type.

When data is omitted from an envelope declaration, it defaults to `{}` (an empty record).

**Wire codecs versus value types.** Wire codecs (`json`, `jsonl`, `csv`,
`parquet`, `avro`, `protobuf`, `msgpack`, `xml`, `text`, `raw`, …) live
on Connector attributes and are orthogonal to the in-graph value type:
the codec is the fallible bytes↔value boundary; the type is what the
parsed value is. Its failure is a declared typed output/status case, not
a privileged `err` mechanism. A Parquet source can produce
`array<struct>` and an Avro source the same; JSON, Avro and Arrow unions
all project into the union forms in [types.md](types.md).

## References — Rastok expressions, without interpolation

The DSL has no `${…}` interpolation token.

- **Boot phase** (initializer positions only : `val` initializers, Connector
  config attributes) : the expression functions **`env("VAR")`** and
  **`secret("path")`**. Using them in a guard/key/state position is a compile
  error — the language has explicit phases (boot → runtime → blocks).
- **Runtime** : plain expression paths, no sigil — `session.key` (the partition
  field), `meta.app.x`, `data.y`, `status is TimeoutExpired`, a
  `val`/`var`/alias by name.
  String building is expression concatenation :
  `val cwd = "/workspace/" + session.key`.
- **Inside guest-language blocks** (`jq! { … }`, `bash! { … }`, …) : the block is fully
  raw — no interpolation, ever ; DSL refs enter jq as `$name` bindings (the
  jq convention).

## No vertical-specific nodes

Core principle : **the Rust binary contains zero vertical-specific nodes**. No `TeamsWebhook`, no `SlackCallback`, no `DiscordAuth`. Verticals are expressed entirely as `.rkg` files using the generic primitives below.

When a new channel adapter is needed (Slack, Discord, custom HTTP) :
1. Write a new `.rkg` describing the gateway with config tuned to that vertical
2. Possibly write new jaq programs for inbound/outbound shaping

**No Rust changes.** Same principle as NiFi's node catalog, with a smaller, more orthogonal set + jaq for any logic that fits in a transform.

### Generic node catalog

This catalog describes the specified language surface and deferred design. It
does not claim implementation or release status.

| DSL node / concept | Role | Spec status / next step |
|---|---|---|
| `HttpServer` | Connector node | specified in the initial stdlib catalog |
| `HttpClient` | Connector node | specified in the initial stdlib catalog ; response/DLQ semantics follow the delivery spec |
| `Process` | Connector node | specified in the initial stdlib catalog ; codec/protocol matrix must remain narrow |
| `File` | Connector node | deferred; must use the standard Connector contract |
| `Postgres` | Connector node | deferred; must use the standard Connector contract |
| `Sqlite` | Connector node | deferred; must use the standard Connector contract |
| `Clock` | Connector node (mockable clock) | specified in the initial stdlib catalog |
| `Calendar` | Source Connector | civil time (cron+tz / at) — see stdlib.md ; `Clock` = physical time |
| `Memory` | bounded in-process store | initial catalog; append and snapshot-read contract in stdlib.md |
| `Jaq` | Operator alias (jq — document guest over structured values) | specified in the initial stdlib catalog |
| `Sql` | Operator alias (SQL guest over the table view of `array<struct>`) | planned guest; not in the initial executable slice |
| `Reducer` | Stateful Operator alias (`var`, normally keyed with `per`) | specified in the initial stdlib catalog |
| `Aggregator` | Stateful Operator alias (`var` + `per`) | specified ; window contracts remain deferred |
| `Window` | `Aggregator` with a window-lifetime partition | specified surface ; full window contracts remain deferred |
| `Joiner` | Operator with an `on (a, b)` join handler | specified |
| `Router` | Multi-route Operator | specified ; route lowering must preserve the static topology contract |
| `Fanout` | not part of the surface | broadcast is the atomic multi-emit statement `(a, b, c) = x` ; see [Dynamic dispatch primitives](#dynamic-dispatch-primitives-router-dispatch) |
| `Dispatch` | Keyed sub-graph composition | specified ; key expression and lifecycle rules are normative |
| `Saga` (aspect) | Saga aspect — opt-in scoping for long-running compensation-based workflows (model in [`effects-and-sagas.md`](effects-and-sagas.md), generic mechanism in [`aspects.md`](aspects.md)) | grammar specified; runtime manager requires effective `durability >= local` |
| `Loop` (aspect) | Loop aspect — bounded tail-recursive iteration (retry / convergent / counted), spec in [`loop.md`](loop.md) | grammar specified ; runtime support is deferred |
| `-> nothing` | Discard sink (built-in) | not a node — `nothing` as an edge target ; non-blocking drain |
| `RequestReply` | Correlated call pattern | design intent ; may lower to Connector + Operator composition |
| `Metrics` | Sink Connector | future |
| `Join` / `Barrier` | Guarded fan-in | the canonical form is an `on (a, b)` join handler (keyed by the partition ; bounded by `last`/`within`) |
| `WebSocket` | Connector node | planned |
| `Tcp` | Connector node | planned |
| HTTP/2 | Binding option, not a node | planned ; express as Connector binding, not as a `Http2` node type |

Rule for DSL evolution : do not add a new node name to an `.rkg` example
without adding it here and marking whether it is specified, design intent, or
deferred.

## `simulation` blocks

The simulation surface mirrors the graph grammar. A `simulation` is a named set
of `scenario` blocks sharing optional mocks. A scenario contributes the world's
half of the journal : injections at inbound Connector outputs, virtual-time
advances, fault steps, restarts, and expectations. The graph under test is the
same graph used in production ; only its external Connectors are replaced by
declared fixtures or mocks.

Example :

```rastok
lab teams_gateway {
  simulation teams_channel on app {
    scenario plain_message "plain Teams message" {
      inbound.out = {
        meta.app.source = "teams"
        data = {
          type = "message"
          from = { aadObjectId = "abc-123" }
          conversation = { id = "conv-1" }
          channelData = {
            tenant = { id = "tenant-x" }
          }
          text = "Bonjour"
        }
      }

      mock sessions.codex {
        on in where data.method == "turn/start" {
          out = {
            meta.app.kind = "jsonrpc.notification"
            meta.app.method = "item/agentMessage"
            data = {
              method = "item/agentMessage"
              params = { message = "Bonjour" }
            }
          }
        }
      }

      expect "Teams activity" outbound.in where (
        data.type == "message" &&
        data.text == "Bonjour")
    }
  }
}
```

Semantics :

- `inbound.out = { ... }` injects an envelope at an inbound Connector's
  output — classified like any braced value (here the envelope form :
  every root is reserved).
- `mock sessions.codex { on in where ... { out = ... } }` uses ordinary handler
  grammar; an uncovered external call fails the scenario loudly.
- `expect "label" outbound.in where (...)` observes deliveries since the last
  action step. `expect nothing ...` is the explicit empty-window quantifier.

The complete simulation contract is in
[simulation-semantics.md](simulation-semantics.md). The productions below are
the parser surface for that contract.

## `theorem` blocks

`theorem` is reserved for a later verification layer and is not part of the
current parser surface.

A `theorem` block should declare assumptions and proof obligations, not contain
a hand-written proof. A future backend prover or model checker would translate
the closed graph to a KPN-like model, a colored Petri net, TLA+, SMT, or
another formal target, then report `proved`, `refuted` with an Rastok trace
counterexample, or `unknown`.

The initial implementation should focus on `simulation`. The immediate design
constraint is to keep I/O nodes mockable so a production graph can be closed
under simulation.

## What the parser must produce

A `.rkg` source is intended to compile to the same `Graph` value as the Rust
builder API. Concretely :

```rust
let mut g = Graph::new("teams_codex");
val n1 = g.add("inbound", HttpServer::from_config(parsed_cfg));
val n2 = g.add("auth", transformer(Jaq::from_config(parsed_cfg)));
// ...
g.connect(n1, n2, EdgeConfig::default())?;
```

Required Rust extensions for the parser to land :
1. **NodeRegistry** : `type_name → factory`. Lets the parser look up `"HttpServer"` / `"HttpClient"` and instantiate them.
2. **`FromConfig` trait** per node : deserializable JSON/HCL config + `from_config(&Config) -> Self`.
3. **`NodeSpec` config** : flat Connector attributes plus declared ports. Pure Operators may only need config and handlers.
4. **`EdgeConfig`** : extend `connect()` to accept `capacity / backpressure / close / trace`. Transport/codec/protocol live on Connector attributes, not on graph edges.
5. **Multi-port lowering** : a node with both read and write ports compiles to Reader and Writer runtime halves sharing an internal resource handle (correlator, child process handle, socket, HTTP client, …).
6. **Simulation lowering** : compact port fixtures lower to internal simulation sources/sinks that compile to the same runtime traits as real I/O nodes but use declared envelopes and assertions instead of external I/O.
7. **`graph` keyword** : the parser treats lowercase `graph` as the entry keyword, and emits `Graph::new(name)`. The Rust type name stays `Graph`.

## Glossary

| Term | Meaning |
|---|---|
| `graph` | Graph declaration keyword. Compiles to the Rust `Graph` type. |
| Operator | Family of Nodes whose computation is internal to the graph. May be pure or stateful. Concrete types end in `-Operator` (`JaqOperator`, `WasmOperator`, …). |
| Connector | Family of Nodes that talk to an external system. Same shape as Operator plus boundary attributes (URL/path/command, `codec`, protocol, timeout, auth). Concrete types end in `-Connector` (`HttpClient`, `ProcessConnector`, …). |
| Operator parameterization | Three orthogonal axes — `var` state, `per`, and the handler trigger (`on <port>` vs the `on (a, b)` join) — that cover all named patterns (Reducer, Aggregator, Window, Joiner). See [Operator parameterization](#operator-parameterization-state--per--transaction). |
| Stdlib aliases | Short names exposed by the stdlib for frequent Operator parameterizations : `Jaq`, `Reducer`, `Aggregator`, `Window`, `Joiner`, `HttpClient`, `Process`, etc. The compiler resolves them to configurations of the primitive `operator` or to native `Connector` types. |
| Dynamic primitives | The two runtime-decision nodes : Router, Dispatch. They preserve topology staticity. Broadcast is the inline `(a, b, c) = x` statement. See [Dynamic dispatch primitives](#dynamic-dispatch-primitives-router-dispatch). |
| Partition | A `(Key, Lifetime)` pair under which a Node's state is materialized. Partitions are flat — no parent declaration, no automatic cascade. Declared with `partition <name> { key = …; lifetime = … }`. See [WHITEPAPER.md §3](../../WHITEPAPER.md). |
| State read-modify-write | **Imperative within a transaction.** `x` (bare in DSL, or `$x` inside `jq! { … }`) starts at the value the previous transaction committed (or the default); assignments mutate it in place, so later reads see the new value (RHS = Rastok expression; no `++`-style shortcuts). The transaction commits atomically at its end. The Lustre `pre` is *inter-transaction*, not intra. |
| Value types | The data's type tree : `variant` (dynamic, JSON model), scalars (`text`/`bytes`/numeric + logical `date`/`timestamp`/`uuid`/`decimal`/`duration`), composites (`struct`/`map`/`array`), `enum` (closed symbols), `union` (sum type). `table` is the relational view of `array<struct>`, not a type. See [Data](#data--typed-values-and-literals). |
| Envelope | `meta + data`. `meta` is three zones of typed scalars (`meta.sys` read-only/runtime, `meta.app` application-owned, `meta.ext` external context — D-2026-08-10-1) ; `data` is the typed payload. See [WHITEPAPER.md §2.1](../../WHITEPAPER.md). |
| Edge config | The `{ ... }` block after an arrow, configuring `capacity / backpressure / close / trace`. |
| Tagged frontend | `name! <region>` — source owned by a registered reader/compiler. |
| Port naming | Ports are role-named with no direction suffix ; direction comes from the `input` / `output` keyword. `in` / `out` / `err` are default pre-bound ports (stdio-aligned), not keywords. |
| Branchement | Fan-out (`a -> { b; c }`) or fan-in (`{ a; b } -> c`). Real DSL primitive. |
| Endpoint | Connector that owns an external resource address/lifecycle and declares explicit envelope ports. |
| `scenario` | one executable case inside a `simulation` : a fixture (the scripted world-half of the journal) + expectations. See simulation-semantics.md. |
| `mock` | scenario/simulation-level stand-in for a calling Connector — its body is ordinary handler grammar (+ `after`). Replaces the partner, not the bookkeeping. |
| Multi-tenancy | Expressed by declaring a keyed sub-graph in place (`graph sessions per <partition> { … }` — `Dispatch`). The runtime materializes one running copy per distinct partition key. Environmental isolation and deployment policy remain outside this grammar. |

## Formal grammar

This is the LL(1) reference grammar for the `.rkg` parser. The implementation
target is a hand-written recursive-descent parser with one token of lookahead.

### Lexer atoms

```ebnf
IDENT      ::= [a-zA-Z_] [a-zA-Z0-9_]*
INT        ::= "-"? [0-9]+
FLOAT      ::= "-"? [0-9]+ "." [0-9]+
STRING     ::= '"' ( [^"\\\r\n] | "\\" [^\r\n] )* '"'            // host STRING is line-bounded;
                                                                    // multiline text uses `text! { … }`
BOOL       ::= "true" | "false"
NULL       ::= "null"
ARROW      ::= "->"
BIARROW    ::= "<->"
LBRACE     ::= "{"
RBRACE     ::= "}"
LBRACK     ::= "["
RBRACK     ::= "]"
LPAREN     ::= "("
RPAREN     ::= ")"
EQ         ::= "="
EQEQ       ::= "=="                                                   // the HOST's comparison lexemes — the simulation
BANGEQ     ::= "!="                                                   //   expect grammar, never a copy of CEL's vocabulary
BANG       ::= "!"                                                    // tagged frontend after IDENT; prefix `!` inside an expression
                                                                      // belongs to the expression-region lexer
LTEQ       ::= "<="
GTEQ       ::= ">="
COLON      ::= ":"
DOT        ::= "."
COMMA      ::= ","
SEMI       ::= ";"
COMMENT    ::= "//" .* EOL  |  "/*" any* "*/"
// A single-quoted string ('…') is an expression-language spelling : the host lexes it as ONE
// opaque Error lexeme with one voice — its content never re-speaks, and
// recoveries walking lexemes stay quote-immune. Any physical line ending
// terminates either host quote spelling as an unterminated token (backslash
// does not continue it), preserving the next statement for recovery.

// Durations and timestamps have NO dedicated tokens : they are ordinary Rastok expressions
// values — `duration("2h")`, `duration("2h30m")`,
// `timestamp("2026-01-05T00:00:00Z")` — in any scalar position. The duration string
// follows the retained CEL/Go-compatible format (ns/us/ms/s/m/h, compound, fractional — `min`/`d`/`w`
// are not accepted). Rate strings stay opaque Connector-attribute values
// ("10/s per …").
// There is no `${…}` INTERP atom. Boot-time resolution uses the Rastok expression functions
// `env("NAME")` / `secret("path")`, legal in initializer positions only ;
// runtime references are plain Rastok expression paths (`session.key`, `meta.app.x`).
// Rastok expression spans are handed to the expression lexer: the DSL lexer only
// finds the span's END (delimitation rules below).
// Maximal-munch : "->", "<->", "==", "!=", "<=" and ">=" tokenize as single
// atoms — `=` vs `==` is a KIND question, never a premature diagnostic
// (Note : "<-" is not a Rastok token ; wiring is always graph-level.)
// Comments and whitespace are skipped by the lexer.
```

### Reserved keyword sets

```rastok
DECL_KW    = { "graph", "partition", "operator", "connector",
               "type",                                                 // unit-level type alias (D-2026-08-10-8)
               "per", "val", "var", "input", "output", "on",
               "start", "stop", "compensate",                          // lifecycle / saga handler triggers (see effects-and-sagas.md for `compensate`)
               "by", "where", "having", "within", "last",
               "when",                                                 // edge guard : a -> b when <expr>
               "key", "lifetime",
               "persistent", "single_pass", "fixed", "window",
               "tumbling", "sliding",                                  // window modes
               "effect", "behavior",                                   // the two mandatory Connector axes (effects-and-sagas.md ; stdlib.md §2)
               "lab", "simulation", "experiment", "theorem", "over",  // the Lab surface
               "scenario", "defaults",
               "expect", "envelope", "advance", "mock",
               "after", "crash", "restart",                            // scenario steps (simulation-semantics.md §5)
               "as" }
// `profile` is an ordinary attribute name (experiment body), not a keyword.
// Simulation setup uses ordinary fixture injections.
TYPE_KW    = { "variant",                                             // the value-type tree (types.md)
               "struct", "map", "array", "table", "enum", "union",   // composites (+ table view)
               "bool", "int", "long", "uint", "float", "double", "number", // friendly scalar aliases
               "int8", "int16", "int32", "int64",
               "uint8", "uint16", "uint32", "uint64",
               "float16", "float32", "float64",                    // canonical explicit widths
               "bytes", "text", "null",
               "decimal", "date", "timestamp", "timestamp_ntz",      // scalar logical
               "time", "interval", "uuid", "duration" }
// Frontend names are deliberately NOT a keyword set. IDENT BANG selects
// the production; the Registry later resolves `json!`, `jq!`, `sql!`,
// application names, and future formats through one contract.
// Tagged INITIALIZERS are DEAD (D-2026-08-12-1): oauth2_client_credentials,
// measure, exponential, linear, constant, fibonacci, on_crash left the
// vocabulary — a plain struct with a closed-choice discriminant FIELD
// replaces each one.
LITERAL_KW = { "true", "false", "null", "nothing",
               "block", "drop_oldest", "drop_newest", "error",
               "cascade", "mirror", "none", "local", "global",   // none/local/global = durability scopes ; none/cascade/mirror = close policy
               "idempotent", "compensable", "irreversible",       // Connector effect classes — see effects-and-sagas.md
               "deterministic", "probabilistic", "seeded",        // behavior values (stdlib.md §2 ; seeded reserved)
               "ack", "respond",                                  // HttpServer modes
               "job", "service",                                  // Process modes
               "jsonl", "lines",                                  // Process codecs (bytes reserved)
               "evict_oldest", "reject",                          // Memory overflow
               "bypass", "kill",                                  // on_timeout values (execution-model §6.4)
               "read", "write", "duplex",
               "process_group" }
```

`DECL_KW` introduces declarations, labeled blocks, handler clauses
(`where`/`having`/`within`/`last`) and the narrowing operator `as`.
`IDENT BANG` introduces every tagged frontend (`jq! { ... }`,
`jq!(req) { ... }`, `base64!(ct) { ... }`). No frontend name is a
keyword. Expression-space constructors (`bounded(64)`, `asset(...)`,
`decimal(...)`) are ordinary calls.
`TYPE_KW` names the value-type tree — `variant` (the explicit dynamic case), the
composites, and the scalar physical / logical types. `LITERAL_KW` are enum-like
idents allowed as scalar values.

Simulation setup uses fixtures: injection is the setup, and preloading a
`Memory` is expressed as ordinary injections at its feeding edge. Reading
uses the Connector's explicit `read` and `out` ports.

The conventional ports **`in` / `out` / `err`** are not keywords. They
are ordinary pre-bound identifiers and may be explicitly restated once
in their conventional direction; a second declaration or contradictory
direction is `E-PORT-CONFLICT`. M1.3 materializes their effective types
in the resolved port table exactly like every other port.
Reservation is BY POSITION, never a global ban on a spelling
(D-2026-08-12-1; enforcement stated exactly by D-2026-08-13-1
review round 3 — this section supersedes the earlier draft that
claimed `DECL_KW` unmaskable):

- **Binding positions** (graph, lab, node, port — boundary and node
  alike — val/var/let at every level, type names): the value forms
  `nothing` and the expression literals `true`, `false` and `null` are
  RESERVED — one naming policy, refusing at the
  declaration with `E-NAME-RESERVED`. The envelope surfaces
  (`meta`/`data`/`sys`/`app`/`ext`) additionally refuse as node
  names, and `data`/`meta` as binding names. `DECL_KW` words
  (`graph`, `operator`, `val`, …) are NOT reserved: declaration
  positions are structural, so a binding named `graph` shadows
  nothing — the position's own grammar decides.
- **Type positions**: `TYPE_KW` words mean their type (`text` names
  the text type there); a user `type` declaration may not SPELL a
  builtin type — the builtin resolves first and the declaration
  would be silently dead (`E-NAME-RESERVED`, review round 3).
- **Attribute names**: any expressible identifier; dots are owner
  qualification only.
- **Bare-word VALUE positions**: any expressible word the contract's
  choice domain admits — `codec = json`, `framing = text` and
  `capacity = unbounded` are all legal choice words (a choice
  introduces NO binding). Only a following `!` selects a frontend;
  the name itself remains ordinary. `LITERAL_KW`
  words and the `nothing` sentinel keep their literal meaning and
  cannot serve as contract choices.

Frontend names (`json`, `text`, `base64`, `jq`, `sql`, application
names, and future formats) are ordinary identifiers. The Registry
selects a reader/compiler only after `IDENT!` has selected the grammar
production. Old bare tag spellings may receive migration diagnostics,
but do not reserve their names.
Concrete NodeType names such as `HttpServer`,
`Process` and `Postgres` are ordinary PascalCase identifiers resolved
later by the stdlib registry, not lexer keywords.

### Productions

```ebnf
file              ::= top_decl*

top_decl          ::= graph_decl                       // incl. the one `: App` entry graph
                    | lab_decl                           // the Lab is the home of
                    | val_decl                           //  simulation / experiment / theorem ;
                                                         //  no bare top-level `simulation`)
                    | type_decl                          // `type Person = struct { … }` — UNIT-level namespace,
                                                         //  structural sugar (D-2026-08-10-8)
// A graph instantiates by declaration and wires by name ; the entry point is
// the `: App`-annotated graph ; `app` is a pre-bound identifier for it.

graph_decl        ::= "graph" IDENT ( ":" type_annotation )? per_clause? "{" graph_body "}"    // top level = graph | lab | val, STRICTLY — nodes, partitions and edges live in graph bodies ; per_clause → composed per-key (Dispatch), graph-level val/state is per-instance ; `: <aspects>` opt-in to Saga / Loop scoping (see aspects.md)

partition_decl    ::= "partition" IDENT "{" partition_body "}"
partition_body    ::= ( "key" "=" expr
                       | "lifetime" "=" lifetime_value )*

operator_decl     ::= "operator" IDENT ( ":" type_annotation )? per_clause? operator_body
connector_decl    ::= "connector" IDENT ":" type_annotation per_clause? operator_body

// `type_annotation` is a `+`-joined list of stdlib symbols. Each symbol is
// classified by the type checker as a *trait* (Connector NativeType or Operator
// stdlib alias), an *aspect* (Saga, Loop — see aspects.md), or a *role*
// (`App` — graph-only, marks the deployment entry point). Rules :
//   - Connector : exactly one trait (MANDATORY — an external boundary is explicit),
//     zero or more aspects. No implicit type.
//   - Operator  : implicit canonical type `Operator` ; at most one stdlib alias,
//     zero or more aspects (`operator auth {}` ≡ `operator auth : Operator {}`).
//   - Graph     : implicit canonical type `Graph`, optionally specialized by the
//     role `App` and/or aspects (`graph g {}` ≡ `graph g : Graph {}`).
//   - Same name twice in the list = compile error ; at most one role.
// Convention : trait/role first, aspects after.
type_annotation   ::= IDENT ( "+" IDENT )*

operator_body     ::= "{" ( port_decl | val_decl | var_decl
                          | handler_decl | attribute )* "}"
                                                                      // sub_block is DEAD (D-2026-08-12-1); parsed only to refuse
                                                                      //   with its migration hint

graph_body        ::= ( port_decl | partition_decl | operator_decl | connector_decl
                       | val_decl | edge_decl
                       | defaults_block
                       | graph_decl                            // graphs NEST (effects-and-sagas §7.2)
                       | attribute                             // aspect attributes : Saga.timeout = duration("10m"), name = …
                       | handler_decl )*                        // graph-level handlers (including Loop base cases)

type_decl         ::= "type" IDENT "=" type_ref                   // TOP-LEVEL only ; one namespace per unit ; duplicates refuse ;
                                                                  //   recursion refuses (v1) ; budgets: depth ≤ MAX_NESTING, expanded
                                                                  //   tree ≤ 4096 nodes, ≤ 256 fields/struct, ≤ 64 KiB (D-2026-08-10-8)

// A Node carrying `: Loop` must have exactly one qualified attribute:
//   Loop.variant = { kind = measure, initial = 5, next = value - 1, until = value <= 0 }
// This uses the ordinary `attribute` and `braced_value` productions
// (D-2026-08-12-1 — the discriminant is a closed-choice field). Semantic
// validation of the fields and bounds is specified in loop.md §2/§8.

per_clause        ::= "per" ( IDENT ( "," IDENT )*                   // reference declared partition(s) ; composite = tuple
                            | partition_inline )                      // inline ANONYMOUS partition (private to this node ; no name ⇒ no cross-partition read, no <part>.key reference)
partition_inline  ::= "{" partition_body "}"                          // block form — newline- or ;-separated (like a `partition` block)
                    | "(" partition_field ( "," partition_field )* ")"  // compact form
partition_field   ::= "key" "=" expr | "lifetime" "=" lifetime_value

port_decl         ::= ( "input" | "output" ) IDENT ( ":" type_ref )?  // source annotation OPTIONAL; every validated port has an effective type
                    // in / out / err are conventional pre-bound ports and use this same model; the resolved plan materializes them like every other port.
                    // Connector contracts declare fixed or parametric port rules; attributes/codecs resolve them per instantiation.
                    // No port-side inbound shortcut : wiring is always graph-level.
                    // To bind a port to a source, declare the Connector then wire it :
                    //    connector c : Clock { every = duration("1s") }
                    //    c -> a.tick

val_decl          ::= "val" IDENT ( ":" type_ref )? "=" value        // immutable, scope by position
var_decl          ::= "var" IDENT ":" type_ref ( "=" value )?        // mutable Operator state

handler_decl      ::= "on" trigger guard_clause* handler_body?
                    // handler_body is OPTIONAL :
                    // a BODY-LESS handler is the Loop BASE-CASE declaration
                    // (`on status where data is Exited` — loop.md §3) ; compile rule :
                    // body-less handlers are legal ONLY on nodes carrying `: Loop`.
                    // The ITERATING case carries a body, possibly empty :
                    // `on status where data is TimeoutExpired { }`.
trigger           ::= "start"
                    | "stop"
                    | "compensate"                                   // compensation event on a `compensable` Connector — see effects-and-sagas.md
                    | IDENT "." ( "start" | "stop" )                 // ASPECT-QUALIFIED lifecycle : on Loop.start { … } —
                                                                     //   bare start/stop bind to the NODE ; the aspect qualifies (aspects.md §6)
                    | "(" IDENT ( "," IDENT )* ")"                   // join : presence of all named ports
                    | IDENT                                          // single port
guard_clause      ::= "where" expr                                   // content predicate (Tier 1 = 1 port, Tier 2 = ≥2)
                    | "having" expr                                  // aggregate predicate (quorum, e.g. count(p) >= k)
                    | "by" expr                                      // join-key override (default = partition key)
                    | "within" expr                                  // join time window (optional ; constant, duration-typed)
                    | "last" INT                                     // join count bound — a Tier-2 θ-join requires AT LEAST ONE bound (within or last) ; presence joins may use either or none
                    // A `where` clause on any selected outcome input may define the Loop base case when the enclosing node carries `: Loop` — see loop.md §3.
handler_body      ::= "{" ( val_decl | statement )* "}"
statement         ::= "(" lvalue ( "," lvalue )* ")" "=" tuple_or_broadcast   // atomic multi-emit — `(` decides ; tuple = explicit per-port ; broadcast = same value everywhere (`Fanout` is not a surface alias)
                    | "stop" IDENT?                                   // stop action (lifecycle) — word-decided
                    | IDENT path_tail assignment_tail                 // left-factored : ONE consumed word ; the ASSIGNMENT PROBE decides
assignment_tail   ::= ( ":" type_ref )? "=" value                     // the consumed path is the LVALUE : var write / ordinary port emit / dotted meta·data write ( = nothing → no emit ; = . re-emits the input ; braced values classified by reserved roots — see braced_value ). `err` has no special branch.
                    // There is NO implicit yield (D-2026-08-13-1, review round 7) : a
                    // bare expression — IDENT-led or not — is not a statement. It
                    // refuses with ONE statement error hole and the migration hint
                    // (`out = <value>`). A well-framed value is
                    // consumed by its REAL value/region reader before that refusal, so
                    // its interior never becomes enclosing statements. After an IDENT
                    // fails the assignment production, the expression framer chooses only
                    // recovery extent: it commits a complete continuation as one region,
                    // or leaves the stream untouched for line-bounded host-lexeme
                    // recovery. Normal values and recovery share one Complete | Incomplete
                    // extent computation. Behind `=`, a complete value commits to FOLLOW;
                    // an incomplete value commits only its safe line boundary and the expression
                    // compiler emits the one syntax diagnostic — the next statement never
                    // becomes expression text.
                    // A precise structural lexical
                    // fault replaces the generic position voice. A future explicit
                    // `yield <value>` sugar may reopen the position.
                    // In STATEMENT position `(` always selects multi-emit; a
                    // parenthesized expression value is legal only in a VALUE position.
                    // A malformed multi-emit recovers forward at the statement
                    // boundary — no tentative parse, syntactic predicate, or rewind.
path_tail         ::= ( "." ( IDENT | TYPE_KW | LITERAL_KW ) )*       // the dotted walk shared by lvalues and expression prefixes ; reserved segments after a dot permit qualified names such as Loop.variant and stdlib ports such as store.read
lvalue            ::= IDENT path_tail                                 // assignment target: port, var, or dotted writable meta/data path — meta.sys is read-only, never an lvalue
tuple_or_broadcast ::= "(" value ( "," value )* ")"                    // explicit tuple — one value per lvalue, counts must match
                    | value                                            // broadcast — same value to every lvalue


type_ref          ::= union_type ( "?" )?                             // T? optional == T | null
union_type        ::= base_type ( "|" base_type )*                    // sum type
base_type         ::= scalar_type
                    | "variant"
                    | "struct" "{" ( field_decl ( "," field_decl )* )? "}" // typed record — EMPTY struct { } is legal
                    | "map" "<" type_ref "," type_ref ">"
                    | "array" "<" type_ref ">"
                    | "table" "<" type_ref ">"                        // alias for array<struct>
                    | "enum" "{" IDENT ( "," IDENT )* "}"
                    | "union" "{" tagged_case ( ","? tagged_case )* "}" // tagged union; case names are semantic discriminants
                    | IDENT                                           // named type / Schema / declared enum|union
scalar_type       ::= "bool" | "int" | "long" | "float" | "double" | "number"
                    | "bytes" | "text" | "null"
                    | "decimal" "(" INT "," INT ")"
                    | "date" | "timestamp" | "timestamp_ntz" | "time" | "interval" | "uuid" | "duration"
field_decl        ::= field_name ":" type_ref
                    | ".."                                            // open record (row polymorphism)
tagged_case       ::= IDENT ( "{" ( field_decl ( ","? field_decl )* )? "}" )?
field_name        ::= IDENT | TYPE_KW                                 // field NAMES may reuse type words (`text: text`,
                                                                      //   `kind: text`) — position disambiguates

lifetime_value    ::= expr                                            // bare duration value = idle_ttl (constant, duration-typed)
                    | "persistent"
                    | "single_pass"
                    | "fixed" expr                                     //   (constant, duration-typed)
                    | "window" expr ( "tumbling" | "sliding" )         //   (constant, duration-typed)

attribute         ::= lvalue "=" value                               // dots = owner qualification ONLY (Loop.backoff = …)
struct_body       ::= ( attr_entry ( ","? attr_entry )* )?
attr_entry        ::= lvalue "=" value                               // dotted key for an assignment (meta.app.label = … ; meta.sys is read-only)
// sub_block (IDENT "{" struct_body "}") is a DEAD form (D-2026-08-12-1):
// the parser still recognizes it to refuse with the migration hint
// « `env { … }` is no longer a configuration form; write `env = { … }` ».

// ============ WORD CLASSES (D-2026-08-14-1) ============
// Three roles a word can play — the factoring below names them:
// - a USER IDENTIFIER introduces a binding (val/var/let, node, port, type,
//   partition, alias). `nothing` is RESERVED here — it names a structural
//   value form and refuses AT THE DECLARATION (E-NAME-RESERVED) ; the
//   envelope surfaces (meta/data/sys/app/ext) refuse as node names, and
//   data/meta as binding names.
// - any IDENT immediately followed by BANG introduces a tagged frontend.
//   No closed keyword vocabulary participates in this syntax decision.
//   The Registry validates the name and framing contract after parsing.
// - a CONTRACT WORD is a bare word judged by a contract's closed choice
//   (mode = job, codec = jsonl). It introduces NO binding — reservation
//   does not apply ; the contract judges the word.

// ============ VALUES : structural forms, tagged frontends, expressions ============
// Rastok expressions cover intrinsic computation. There are no `ref`, `interp`, or
// leading-dot path forms.

value             ::= braced_value                                    // { … } — the braced record ; classified in envelope positions
                    | "."                                             // identity — exact re-emission (envelope positions only)
                    | "nothing"                                       // emission positions : emit nothing (reserved word, unmaskable)
                    | IDENT value_after_ident                         // left-factored : ONE consumed word, the tail decides
                    | expr_non_ident_start                            // any other first token — expression from that token on

value_after_ident ::= ε                                               // nothing significant on the line and the line ends the
                                                                      //   value : the word ALONE is the expression (span exact)
                    | "!" frontend_args? frontend_payload             // IDENT ! selects the frontend with one-token lookahead
                    | expr_continuation                               // anything else : expression beginning with the word

frontend_args     ::= "(" ( value ( "," value )* ","? )? ")"
frontend_payload  ::= FRONTEND_REGION                                 // one atomic region returned by the resolved frontend;
                                                                      // its contract chooses JSON, braced, STRING, or a future form
FRONTEND_REGION   ::= /* exact source region returned by the selected frontend reader/framer;
                         json! reads one complete JSON object, array, string, number,
                         boolean, or null */

expr_non_ident_start ::= expr                                         // the region scan starts AT the current token (a literal,
                                                                      //   `-`, `(`, `[`, a string, …)
expr_continuation ::= expr                                             // the region scan CONTINUES a consumed prefix : the token is
                                                                      //   re-read through the immutable source slice, forward only ;
                                                                      //   a peeked BOUNDARY token (closer, position byte, line-crossing
                                                                      //   token at a newline-ended position, keyword, EOF) ends the
                                                                      //   expression at the prefix and stays the HOST's

                    // THE LEXER/PARSER FRONTIER (D-2026-08-13-1, review round 5) : the
                    // lexer owns the byte cursor AND the token lookahead ; the parser
                    // reads complete lexemes (peek/next) and consumes sub-language
                    // regions as specialized lexemes — a complete or safely bounded
                    // Rastok expression, or a tagged frontend payload. It never
                    // touches bytes, and no path
                    // can combine a buffered lexeme with a second read of the same bytes.
                    // The named finite-byte-lookahead points — lexer-owned QUESTIONS,
                    // the only byte-driven production choices : the distinct statement
                    // and value head classifications (an expression region may open with bytes
                    // outside the host vocabulary), the path/assignment questions,
                    // the IDENT-BANG frontend question and the line questions
                    // (value-missing, line-exhausted). Inside braced_value,
                    // ONE lexeme decides struct/map/empty : "}" → {} struct ; ":" →
                    // {:} map ; STRING → map_entries ; IDENT → struct_entries. The
                    // host grammar is predictive (LL(1)) up to those named points and
                    // its delegations — each byte has one lexical authority, comments
                    // are trivia in expression regions AND in recoveries (a boundary byte
                    // inside a comment never bounds), and a line's end can only end a
                    // value at depth zero of a newline-ended position. The shared expression
                    // extent is explicit Complete | Incomplete: it may look past newlines
                    // to prove balanced valid input, while normal value parsing commits an
                    // incomplete extent only to its first safe line boundary and leaves the
                    // expression compiler as the one syntax-diagnostic authority, except that an
                    // unterminated block comment stops before `/*` and belongs to the host's
                    // one trivia authority during parsing. Parser error
                    // recovery skips LEXEMES to a sync set ; misplaced complete values
                    // are consumed by their real reader into one error hole ; incomplete
                    // input recovers by host lexeme to its statement boundary,
                    // retaining an unterminated string/comment as the precise lexical
                    // cause ; a dead struct body is recovered with host lexemes (a `}`
                    // in a comment never closes it) ; dead-initializer and
                    // reserved-format bodies are OPAQUE
                    // regions (their bytes may be host-alien, documented).

expr              ::= /* a Rastok expression (homegrown parser+checker+eval).
                       CEL-compatible subset : literals, member/index access, operators, ternary,
                       exists/all/map/filter/has/size macros, duration()/timestamp(),
                       optional_types (`.?` / `.orValue()`), declared join aggregates.
                       Macro iteration over a MAP is in SORTED-KEY order (an
                       evaluator guarantee — golden-trace stability).
                       `is <type-or-case>` tests/narrows and `as <type_ref>` performs a
                       checked conversion at the END of the span.
                       NO leading dots (paths are bare : data.x, meta.app.y, session.key,
                       alias names). No coalesce shorthand and no
                       string-interpolation tokens. `env()`/`secret()` are
                       legal in initializer positions only. NO `now`/`now()` and
                       no ambient clock read of any kind : time enters an
                       expression only as envelope data (a Clock tick's payload)
                       or as a constant duration()/timestamp() value — replay
                       and simulation depend on it. */
expr_path         ::= /* a Rastok expression access path (subset of expr) : IDENT ( "." IDENT )* —
                       used by correlators and key extractors. */

// DELIMITATION of an expr span — how the DSL lexer finds the end.
// The span runs at BRACKET DEPTH 0 (the expression lexer tracks ( ) [ ] { } nesting,
// so expression map literals `{"a": 1}` — the SAME punctuation as the DSL map,
// D-2026-08-10-6 — and struct-ish braces inside the span are fine)
// until the position's terminator set :
//   guard positions   (where/having/by/when)  → { having by within last when NEWLINE "{" "->" }
//   attribute / init  (= …)                   → { NEWLINE ";" "}" }
//   emission          (out = …)               → { NEWLINE ";" "}" }
//   paren positions   (args, tuples)          → { "," ")" } at depth 0
//   expect predicate  (where …)               → { NEWLINE "}" }
// A terminator is never consumed by the expression lexer; the DSL parser resumes on it.
// MULTI-LINE spans : NEWLINE terminates at depth 0 — a predicate spanning lines is
// wrapped in parentheses OPENED ON THE SAME LINE as the keyword
// (`where (a == 1 &&` … `)`) : inside parens, depth > 0, newlines are free.
// The read-only extent calculation may inspect beyond a newline to prove those balanced
// delimiters. Its result is explicit : Complete ends at FOLLOW ; Incomplete in a
// newline-ended value commits only through the first safe line boundary. This preserves
// the next RKG construct while the expression compiler reports the single fault.
// Ordinary expression strings cannot contain a physical CR/LF (nor use backslash as physical-line
// continuation); write `\n`, or use `text! { … }` for multiline text.
// An unterminated `/*` remains a HOST-TRIVIA fault : the expression extent stops before the
// comment and the trivia reader emits E-LEX-UNTERMINATED-COMMENT during parsing. It is
// never deferred to CHECK, which missing-closer parse errors can prevent from running.

braced_value      ::= "{" braced_body? "}"                            // ONE brace, punctuation decides (D-2026-08-10-6)
braced_body       ::= struct_entries | map_entries | ":"              // `{}` empty struct ; `{:}` empty map ;
                                                                      // mixing `=` and `:` entries REFUSES (one diagnostic, first offender named)
struct_entries    ::= struct_entry ( ","? struct_entry )*             // newline or comma separated (Scala-style, § separators)
struct_entry      ::= lvalue "=" value                                // dotted keys nest (a.b = 1) ; reserved-root classification
                    | IDENT "{" struct_body "}"                       //   applies in envelope positions only
map_entries       ::= map_entry ( ","? map_entry )*
map_entry         ::= STRING ":" value                                // text keys in this milestone ; a NON-EMPTY
                                                                      //   struct as the value is inexpressible in v1
                                                                      //   (deferred by name — D-2026-08-10-6 as amended) ;
                                                                      //   `json! { … }` covers dynamic nesting but is
                                                                      //   always `variant`, never a typed struct
                    // In ENVELOPE positions (handler emissions, fixture injections,
                    // mock replies) the checker classifies the body ONCE by its
                    // top-level roots : no reserved root → a fresh data record
                    // (struct), heritable metadata kept ; only reserved roots
                    // (data, meta) → the envelope form (whole-data
                    // assignment = fresh build, otherwise a patch) ; a mix →
                    // E-BRACE-MIXED, wrap the data in `data = { … }`. `{}` is the
                    // empty record, never an identity. Elsewhere the body is
                    // simply a record. (simulation-semantics.md §4)

// IDENT BANG is the sole tagged-frontend family (D-2026-08-14-1).
// Built-ins (`json!`, `text!`, `base64!`, `jq!`, `sql!`) and registered
// frontends share this syntax. The host chooses the production without a
// Registry lookup; the resolved frontend then supplies its reader/framer,
// type contract and compiler. `asset`, `duration` and `decimal` remain
// ordinary expression calls.
//
// REGION TAXONOMY. Expression regions remain intrinsic. JSON is a
// self-delimited frontend payload; text/base64 are verbatim frontends;
// jq/bash/sql/python/rust are guest frontends. The bang selects the owner,
// not the framing algorithm: each frontend returns one exact region and
// the host resumes after it. Never one universal guest scanner.
//
// GUEST FRAMING. The registered frontend brings the framer used for its
// source form; the host does not grow per-guest comment/string rules.
// A future fenced payload is another frontend payload form, not an
// exception to this ownership boundary.

defaults_block    ::= "defaults" "{" struct_body "}"

edge_decl         ::= edge_source ARROW edge_target edge_guard? edge_config?
                    | edge_source BIARROW edge_target edge_config?
                    | branch_source ARROW edge_target edge_guard? edge_config?
                    | edge_source ARROW branch_target edge_guard? edge_config?
                    | branch_source ARROW branch_target edge_config?
                    | chain_edge

edge_source       ::= port_ref
edge_target       ::= port_ref | "nothing"                           // `nothing` = explicit discard sink (non-blocking drain)
port_ref          ::= IDENT ( "." ( IDENT | TYPE_KW | LITERAL_KW ) )* // node (default port), node.port, or a COMPOSED
                                                                       //   path : sessions.codex.in (instance…node.port)
                    | "." IDENT?                                       // current graph's boundary :
                                                                       //   `.`      = default boundary port (resolves to `in` as edge_source, `out` as edge_target)
                                                                       //   `.foo`   = explicitly named graph port (must be declared via `input foo`/`output foo`)
                                                                       // Static rule : `.` (bare) is legal only at the leftmost or rightmost position of a chain_edge or as edge_source/edge_target ; never as an intermediate node. `.foo` is legal anywhere a port_ref is.
                                                                       // Mirrors the jq convention : `.` = "the current thing's surface" (envelope in jq, graph in wiring).
branch_source     ::= "{" port_ref ( ";" port_ref )* "}"
branch_target     ::= "{" port_ref ( ";" port_ref )* "}"
chain_edge        ::= port_ref ( ARROW port_ref )+ edge_config?       // a -> b -> c ; or `. -> a -> b -> c -> .` with the boundary sugar

edge_config       ::= "{" struct_body "}"
edge_guard        ::= "when" expr                                     // boolean edge guard : route -> audit.in when data.kind == "approval_response"

// ============ SIMULATION GRAMMAR ============
// A simulation = a named SET of scenarios sharing a world (mocks, prelude).

// ---- the Lab — the App's twin, never deployed ----
// `lab <name>` binds to the unit's `graph <name> : App` (E-LAB-NO-APP if none).
// CARDINALITY : a Lab is OPTIONAL — an App without a lab compiles (coverage
// tooling reports the absence) ; at most ONE lab identity per App.
// A Lab is a LOGICAL corpus : several `lab <name> { … }` blocks — same name,
// any files of the unit — are FRAGMENTS merged into one Lab with one
// namespace (duplicate member names across fragments = E-LAB-DUP). No other
// grouping sugar (no package/import/file-namespace) : braces and names only.
// A Lab never deploys and never alters the production graph or its
// graph_version.
lab_decl          ::= "lab" IDENT "{" lab_body "}"
lab_body          ::= ( simulation_decl | experiment_decl | theorem_decl )*

simulation_decl   ::= "simulation" IDENT "on" graph_ref "{" ( mock_decl | scenario_decl )* "}"
                                                                      // `on` names the ONE graph the simulation closes.
                                                                      // The executable target is `app` (the App root) ;
                                                                      // a sub-graph target parses and type-checks but is
                                                                      // DEFERRED (E-SIM-TARGET : closing a sub-graph needs a
                                                                      // birth recipe for envelopes entering its public ports —
                                                                      // not yet defined ; wrap the sub-graph in a thin App).
experiment_decl   ::= "experiment" IDENT "on" "app" "{" experiment_body "}"
                                                                      // the target is `app`, LITERALLY — an experiment runs
                                                                      // the deployed App ; a sub-graph target is meaningless
                                                                      // here (unlike simulation, whose graph_ref is kept for
                                                                      // future sub-graph closure).
experiment_body   ::= ( "profile" "=" expr                            // EXACTLY ONE (0 or ≥2 = E-EXPERIMENT-PROFILE) ;
                                                                      // a constant, text-typed Rastok expression referencing an
                                                                      // EXTERNAL deployment profile — no secret, no
                                                                      // endpoint in the DSL
                      | mock_decl                                     // mocks stay legal for the boundaries NOT under test
                      | scenario_decl )*
                                                                      // an experiment runs the REAL App against ≥1 real
                                                                      // Connector boundary, in REAL time : `advance` AND the
                                                                      // `origin` scenario attribute are compile errors inside
                                                                      // an experiment (both belong to virtual time) ;
                                                                      // observation waits are real and bounded (profile-level
                                                                      // knob, deliberately not new DSL surface).
theorem_decl      ::= "theorem" IDENT "over" IDENT "{" reserved_block "}"
                                                                      // RESERVED SURFACE : declared, parsed for
                                                                      // balance, NOT yet specified. `over <simulation>` names
                                                                      // the lab simulation whose mocks/hypotheses close the
                                                                      // graph. A theorem states a UNIVERSAL property over all
                                                                      // admissible trajectories of (graph + simulation) — it
                                                                      // is never "run a few times". There is NO `check` block :
                                                                      // a bounded challenge is an OPERATION on a
                                                                      // theorem (CLI), not a declaration. See
                                                                      // simulation-semantics.md §Theorem.
graph_ref         ::= "app" | IDENT ( "." IDENT )*                    // `app` = the pre-bound App root ; else a root-relative path
reserved_block    ::= /* balanced braces, content opaque — reserved until the
                       theorem property grammar is specified */
scenario_decl     ::= "scenario" IDENT STRING? "{" scenario_body "}"
scenario_body     ::= ( scenario_attr | mock_decl | scenario_step )*
scenario_attr     ::= "origin" "=" expr                               // civil-time anchor : virtual t=0 ≡ this instant
                                                                      // (needed by Calendar ; absent + Calendar used = warning)

scenario_step     ::= injection
                    | "advance" expr                                  // the discrete sweep (constant duration ; execution-model §8)
                    | "crash" "after" crash_selector                  // fault injection : stop at a journal-record boundary
                    | "restart"                                       // standard recovery (fold + outbound triage)
                    | expect_step

injection         ::= port_ref "=" value ( "seq" INT )?              // THE EMISSION SYNTAX — injects at an inbound
                                                                      // Connector's out only (lineage stays true) ;
                                                                      // value = a braced value, classified as everywhere ;
                                                                      // `seq` : explicit inbound_seq

crash_selector    ::= IDENT "." record_kind ( "#" INT )?              // reserve.outbound_open#2
record_kind       ::= "inbound" | "tx" | "outbound_open" | "outbound_closed"

expect_step       ::= "expect" STRING?                                // optional label, for reports
                      ( "nothing" )?                                  // the void quantifier (∅)
                      port_ref
                      ( "count" cmp_op INT )?                         // exact-count quantifier
                      ( "where" expr )?                               // the predicate (Rastok expression — the grid applies)
cmp_op            ::= "==" | "!=" | "<" | ">" | "<=" | ">="
                    // window rule : an expect reads the deliveries since the last
                    // ACTION step (injection / advance / crash / restart), without
                    // consuming them. Default quantifier = ∃ (at least one match).
                    // `app.<port>` is a legal port_ref when that App boundary port exists.

mock_decl         ::= "mock" IDENT ( "." IDENT )* "{" handler_decl* "}"   // dotted = a composed node (sessions.codex) ; body IS handler grammar
                                                                      // (on in where <expr> { … }) ; two levels :
                                                                      // simulation-shared / scenario override
                                                                      // (whole-mock replacement, nearest wins)
// Inside a MOCK handler body only, one extra statement is legal :
//   "after" expr                                                     // dated emission (constant duration) — the reply joins the
                                                                      // dated-events queue, swept by advance
```

### LL(1) decision table

At each non-terminal, look at the current token to pick a production. The
parser never needs to peek further than one token.

| Non-terminal | Current token | Production |
|---|---|---|
| `top_decl` | `graph` | `graph_decl` |
| `top_decl` | `lab` | `lab_decl` |
| `top_decl` | `val` | `val_decl` |
| `lab_body` | `simulation` | `simulation_decl` (header : IDENT `on` graph_ref) |
| `lab_body` | `experiment` | `experiment_decl` (header: IDENT `on app`) |
| `lab_body` | `theorem` | `theorem_decl` (header : IDENT `over` IDENT ; body reserved) |
| `simulation_decl` body | `mock` | `mock_decl` (shared) |
| `simulation_decl` body | `scenario` | `scenario_decl` |
| `experiment_body` | `profile` — next is `=` | the profile attribute (mandatory) |
| `experiment_body` | `mock` / `scenario` | as in a simulation (but `advance` is illegal inside) |
| `scenario_body` | `origin` — next is `=` | `scenario_attr` |
| `scenario_body` | `mock` | `mock_decl` (override, nearest wins) |
| `scenario_body` | `advance` / `crash` / `restart` / `expect` | the matching `scenario_step` |
| `scenario_body` | IDENT — next is `.` or `=` | `injection` (`port_ref = value`) |
| `expect_step` | after `port_ref` : IDENT `count` | count quantifier ; `where` → predicate ; else done |
| mock `handler_body` | `after` — next is an expression span | dated-emission statement (mocks only ; constant duration) |
| `graph_body` | IDENT — next is `->` / `<->` | `edge_decl` |
| `graph_body` | `.` | `edge_decl` (boundary sugar — `port_ref` starting with `.`) |
| `graph_body` | `{` | `edge_decl` (branch_source) |
| `graph_body` | `graph` / `operator` / `connector` / `partition` / `val` / edge starts | the matching declaration (graphs nest ; declaration = instance) |
| `graph_body` | `input` / `output` | `port_decl` |
| `graph_body` | `on` | `handler_decl` (graph-level — Loop base cases) |
| `graph_body` | `graph` | nested `graph_decl` |
| `graph_body` | IDENT (dotted ok) — next is `=` | `attribute` (`Saga.timeout = duration("10m")`, `name = …`) |
| `operator_body` | `input` / `output` | `port_decl` |
| `operator_body` | `val` | `val_decl` |
| `operator_body` | `var` | `var_decl` |
| `operator_body` | `on` | `handler_decl` |
| `operator_body` | IDENT — next is `=` | `attribute` |
| `operator_body` | IDENT — next is `{` | DEAD form → migration refusal (`name = { … }`) |
| `trigger` | `start` / `stop` | lifecycle handler |
| `trigger` | `compensate` | saga compensation handler (only valid on a Connector with `effect = compensable`) |
| `trigger` | `(` | join handler `(a, b)` (presence) |
| `trigger` | IDENT | single-port handler |
| `guard_clause` | `where` / `having` / `by` / `within` / `last` | guard clause (0+, in any order) |
| `handler_body` (statement) | `val` | `val_decl` |
| `handler_body` (statement) | `stop` — alone or followed by IDENT | `stop` action |
| `handler_body` (statement) | `(` | tuple multi-emit `(a, b) = (e1, e2)` |
| `handler_body` (statement) | `lvalue` (IDENT, dotted ok — e.g. `meta.app.label`) — next is `=` or `:` | assignment / port emit (`= nothing` = no emit ; `meta.sys` is read-only, never an lvalue) |
| `handler_body` (statement) | anything else — a bare expression, IDENT-led or not | REFUSAL (`out = <value>` hint): a complete value is consumed by its real reader into one error hole; incomplete input recovers at the line; the implicit yield is REMOVED (D-2026-08-13-1) |
| `attr_entry` | `lvalue` (IDENT, dotted ok) — next is `=` | dotted attribute |
| `attr_entry` | IDENT — next is `{` | DEAD form → migration refusal (`name = { … }`) |
| `value` | IDENT — next token is `!` | `value_after_ident`: tagged frontend; the Registry resolves the name only after the grammar branch is chosen |
| `value` | IDENT — any other follower | Rastok expression / bare word, judged by the expression checker or contract |
| `value` | `{` — then `}` / `:` / STRING / IDENT | `braced_value` : empty struct / empty map / map_entries / struct_entries |
| `value` | `.` (envelope position) | identity — exact re-emission |
| `value` | `nothing` (emission position) | no-emit |
| `value` | anything else | hand the span to the Rastok expression parser (`expr`; `is`/`as type_ref` compose at span end) |
| `lifetime_value` | an expression span | bare duration value → `idle_ttl` |
| `lifetime_value` | `persistent` / `single_pass` | keyword |
| `lifetime_value` | `fixed` / `window` | followed by a constant duration expression span (and mode for window) |
| `port_decl` | `input` / `output` IDENT — next is `:` | typed (pinned) port |
| `port_decl` | `input` / `output` IDENT — no `:` | port with **inferred** type |

The "two-token" decisions happen at the IDENT crossings (e.g.,
`graph_item` IDENT followed by `{` vs `=` vs `->`). These remain LL(1) with one
symbol of lookahead after the IDENT is consumed. The parser never needs to
*backtrack*.

### What this grammar excludes on purpose

- **One braced form, classified once** (D-2026-08-09-1, punctuated by D-2026-08-10-6). `{ … }` with `=` entries is the struct literal, with `:` entries the map literal (`{}` empty struct, `{:}` empty map, mixing refuses) ; in ENVELOPE positions (handler emissions, fixture injections, mock replies) the checker classifies a struct-form body's top-level roots — no reserved root builds a fresh data record, only reserved roots (`data`, `meta`) select the envelope form, a mix refuses with `E-BRACE-MIXED`. There is no `envelope { … }` keyword form, no `struct { … }` value literal, and no `map { … }` spelling. No silent reinterpretation, LL(1)-clean.
- **No tag keyword partition.** Built-in and user-registered frontends
  use `name!`; a bare `name` remains an identifier. `@` is unallocated
  and may later introduce schema classification, not guest dispatch.
- **Separators are Scala-style, scoped by bracket.** Inside a **`{ }` block**
  (node body, `partition` block, inline `per { … }`, struct), a newline
  terminates each statement / attribute and **`;` is the optional same-line
  separator** (`{ key = meta.app.x ; lifetime = duration("2h") }`). Inside **`( … )`** (compact
  values, arg lists, the inline `per ( … )` form) items are separated by **`,`**
  — never `;`.
- **The intrinsic expression grammar is Rastok's CEL-compatible core.** Every expression position —
  guards, keys, selectors, state expressions, `variant` clauses,
  expectations, attribute values — hands its span to the expression chain
  (delimitation rules above). The DSL itself stays declarative ; structural
  reshaping may use tagged frontends (`jq! { … }`, `sql! { … }` —
  stdlib.md §5); the `!` makes that choice explicit.

### Error recovery

Hand-written recursive descent makes error recovery natural :

- At a missing `{` after `operator X` or `connector X : T`, skip to the next declaration keyword and report.
- At a stray token inside `operator_body`, skip to the next IDENT-at-column-zero or `}` and report.
- At a `}` mismatch, report against the OPEN delimiter stack the descent carries — the cursor only ever moves forward (D-2026-08-13-1: no rewind primitive exists).

Because every form starts with a reserved keyword or a clearly typed IDENT, the
parser can re-synchronize on the next `operator`, `connector`,
`partition`, `graph`, `lab`, `simulation`, `experiment`, `theorem`, or
top-level IDENT.

### Reserved-but-not-yet-used keywords

For forward compatibility, the lexer reserves a few additional identifiers
that may enter the DSL later :

```
compose    // reserved for a possible future reuse-instantiation form
foreach    // future per-item fan-out node
spawn      // explicit per-key spawn directive
watermark  // future event-time watermark on a partition/window
           // (`theorem` is an ACTIVE keyword with a reserved surface.
           //  Bounded verification is an OPERATION on a theorem, never a DSL
           //  block ; `rastok check` stays the CLI static-validation verb.)
replay     // future replay directive (audit-only / logical / exact)
```

These are reserved so existing `.rkg` files using them as plain identifiers
won't suddenly break when the parser learns them.
