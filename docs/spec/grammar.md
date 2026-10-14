# Rastok DSL — Grammar notes

> **Status (2026-10-06): normative source model for format 20.**
> The implemented subset and migration evidence are recorded in
> [R9](../architecture/rework-r9-qualification.md). Unsupported forms refuse
> by name at check. The model includes Node-owned `instance`, `lifetime`, `closing`,
> nested Graph composition and instance-local joins. Its categories are
> Node/Graph/Executor/Connector/Extension. Only Operators have DSL handlers;
> Dispatch and case routing require explicit output dispositions. Loop
> recurrence uses manager commands rather than spatial ports. M1.8 security/privacy guarantees and
> exact `env()`/`secret()` boot references are preserved; the generic measure
> signature is now exactly `id` and `unit`.
> The [Retry contract](../architecture/retry-extension-refactor.md) admits
> finite outbound Call `+ Retry`, plan-owned `idempotent` and static retry
> attributes. Unsupported profiles still refuse by name. Retry contributes
> no port, command or state binding. The [`@struct` declaration](annotations.md#2-flatness-and-parsing)
> parses as a distinct LL(1) base type; flat data and typed annotation
> applications are executable without a separate value algebra. Qualified
> movements and unchanged witnesses live in the [C4 ledger](../architecture/retry-c4-qualification.md).
>
> **Conceptual basis** : this document assumes the model defined in [WHITEPAPER.md](../../WHITEPAPER.md) and the normative instance contract in [instances.md](instances.md). This document is the grammar ; foundations is the constitution.
>
> The `simulation` DSL is covered in this document. Formal verification,
> distributed multi-tenancy and cluster deployment remain future work. Keyed
> instances, local execution and the bounded Loop proof are implemented.

## File extension

`.rkg` — short for "rastok graph". Complete checked programs are indexed in
[the example catalogue](../examples/README.md). Inline blocks below illustrate
individual productions, not necessarily complete deployable units. A declared
future form does not override a current named capability refusal.

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
  (`Loop.delay = { kind = exponential, … }`).
- **Dotted attribute names are owner qualification ONLY** (`Loop.delay`, `Saga.timeout`, `HttpClient.timeout`) — never a patch path into a composite value.
- **The host grammar is LL(1) after left-factoring the identifier
  prefixes** (D-2026-08-13-1): classification observes the form (one
  raw significant byte, or one peeked token) without irreversible
  effect; once chosen, ONE branch consumes and the cursor never moves
  backward. Sub-languages are selected by an explicit syntactic form,
  then read by their dedicated reader — each byte has one lexical
  authority, and host trivia (comments included) never changes an
  AST, a plan, or a hash. See `Formal grammar` for the decision
  table.

Example of executable Process job settings (excerpt; wire its outputs in an App):

```rastok
connector echo : Process {         // type annotation with ":"
  command  = "cat"                  // attribute "="
  args     = []                     // list value
  mode     = job
  codec    = jsonl
  idempotent = true
  behavior = deterministic
  timeout  = duration("5s")         // expression value
  env      = { "LANG": "C" }       // map value
}
```

## Single composite concept: `graph` — and the `: App` entry point

The `app` and `compose` keywords are not part of the grammar. The `App`
Extension marks the deployment entry point instead.

There is **one** keyword for composite units. No `pipeline`, `flow`,
`session`, `template` — and no `app`.

The following is a **target-architecture composition sketch**, not a checked
program: durable storage and the application-protocol adapter remain named gates,
and a complete current program must also declare typed boundaries and dispose
of every productible output. For an executable session boundary, use the
[WebSocket gateway](../examples/codex-websocket-gateway.rkg).
Protocol adaptation is omitted from this composition sketch: Process parses
JSONL values, while Operators interpret their application meaning and
correlation (D-2026-09-10-2).

```rastok
graph teams_gateway : App {          // the DEPLOYMENT ENTRY POINT — exactly one per deployed unit
  durability = local                 // app-level attributes live here

  connector inbound : HttpServer {
    listen = "0.0.0.0:8080" ; path = "/teams/webhook" ; codec = json
  }

  graph sessions {                   // a graph DECLARES → it IS a composite Node
    instance = { key = meta.app.session_key }
    lifetime  = { kind = idle ; after = duration("2h") }
    closing   = { drain_for = duration("30s") ; settle_for = duration("5s") }
    connector codex : Process { mode = service ; command = "codex" ; args = ["app-server"]
                                codec = jsonl
                                behavior = probabilistic }
    in -> codex.in
    codex.out -> out
  }

  connector outbound : HttpClient { url = env("TEAMS_URL") ; codec = json
                                    timeout = duration("10s")
                                    idempotent = false ; behavior = deterministic }

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
- **`: App` is the root Graph Extension**: it marks the deployment entry
  point. A deployed unit has
  **exactly one** `: App` graph (0 or ≥2 = build error). App-level concerns —
  `durability` and retention overrides — are its attributes
  and body. `app` is no longer a keyword; it survives as a **pre-bound
  identifier** aliasing the `: App` graph, so a declared App boundary port
  remains addressable as `app.<port>`. There is no implicit root sink:
  each productible output case has an explicit disposition, and
  `app.<port>` observes only a real App-boundary crossing.
- **Durability is lexical.** A stateful Node's effective value is its
  explicit `durability`, otherwise the closest enclosing Graph's explicit
  value, otherwise the App's value, otherwise `none`. A child may strengthen
  or weaken the inherited value; weakening emits
  `W-DURABILITY-WEAKENED` and is included in coverage output.
- **Declaration = instantiation, for every Node form.** A declared graph is a
  composite Node in its enclosing scope, wired **by name** — exactly like an
  Operator or Connector. A declared graph that is never wired = dead-code
  warning (the unfed-input diagnostic class).
- **Instance addressing is uniform** : any Operator, Connector, or Graph may
  carry `instance = { key = <expr> }`. `lifetime` is a separate common
  attribute. Physical capacity, spawn, placement, shard, and Worker policy do
  not enter this semantic sheet.
- **Identity guard** : the `: App` graph is the composition **root** ;
  `node_path` is relative to the root and the root's own name is
  **excluded** ([envelope.md §2](envelope.md)).
- **Reuse note** : instantiating one graph *type* at several sites is
  deferred. If introduced, it can use
  an explicit import/instantiation form decided with that feature; the current model does
  not overload the Extension annotation with a Graph type.
  `compose` and `app` are
  reserved-but-not-used.

## Vocabulary: Node, Graph, Executor, Connector, Extension, and ports

Rastok has two structural Node forms:

```text
Node
├── Graph
└── Executor
    ├── Operator
    └── Connector
```

- A **Graph** is a typed boundary, lexical scope, child-Node set, and wiring
  body. It owns no `var`, handler, application mailbox, or application
  transaction.
- An **Executor** owns executable ports, a bounded activation mailbox,
  journal-derived semantic state, and atomic transactions with ordered
  emissions.
- An **Operator** is an Executor whose transition is a DSL `Dispatch` and may
  declare private `var` state.
- A **Connector** is an Executor which owns one exact versioned **Contract**;
  its transition comes from that Contract and typed Driver events. It has no DSL handler
  or author-declared `var` in M1.
- An **Extension** is attached to a Node and may contribute qualified
  attributes, bindings, admission constraints, manager commands, and ports.
  `App`, `Loop`, `Retry`, and the deferred `Saga` are Extensions.

The declaration keyword fixes the structural form. A Connector annotation
starts with one mandatory Contract name and may continue with Extensions. A Graph or
Operator annotation contains only Extensions:

```rastok
graph app : App + Loop { ... }
operator countdown : Loop { ... }
connector pay : HttpClient + Loop { ... }
```

Extension order is non-semantic and canonical. Duplicate, unknown, misplaced,
or incompatible symbols refuse. Graph and Operator have no redundant
`node::Type`; a Connector carries the exact `ContractId` of its Contract. See
[extensions.md](extensions.md).

`Reducer`, `Aggregator`, `Window`, and `Joiner` may remain guide names for
visible primitive compositions. `Router`, `Dispatch`, and fan-out instead name
actual routing/transaction operations, not Node categories or annotations.

All Nodes may own `instance`, `lifetime`, and `closing`. A Graph may own
immutable lexical `val` declarations. Mutable `var` belongs only to an
Operator. Operator and Connector share transaction machinery, but the
semantic category does not imply a public Rust trait.

An input or output is a typed envelope endpoint. The current Graph boundary
and child Nodes share one namespace: a bare name resolves to exactly one
boundary port or child, otherwise `E-NAME-CLASH`. Direction is positional:
inside a Graph, its input is a source and output a destination; a child input
is a destination and child output a source. An unambiguous child name selects
`in` as a destination or `out` as a source.

Extension contributions are always qualified: `Loop.fuel`,
`Loop.expired = failed` (a target attribute), and `pay.Loop.again` (a command).
Loop contributes no port. No business-port spelling is reserved.

Connector external configuration remains flat attributes plus typed ports.
Composite values such as `env = { ... }` and `tls = { ... }` are replaced
atomically; dotted names qualify an owner and never patch a subobject.

This keeps protocol layers separate :

```text
transport = stdio | http | websocket | tcp | file | memory
codec     = jsonl | json | sse | text | bytes | multipart
protocol  = jsonrpc | teams_activity | mcp | log | raw
```

HTTP and JSON-RPC are therefore not the same layer : HTTP is a transport/application binding, JSON-RPC is a message protocol that can run over `stdio+jsonl`, WebSocket text, HTTP JSON, etc.

Example : a `Process` node for `codex app-server` owns one child process
address, binds `stdin/stdout/stderr` to `in`/`out`/`err`, and exposes an
additional typed invocation `exit` output. The compiler lowers those ports to
runtime Reader/Writer halves that share one process handle and one
correlator.

`HttpServer` and `HttpClient` deliberately name separate Contracts because
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

WebSocket also uses separate client/server contracts, but its cardinality is
session-shaped rather than one Call per message:

```text
WebSocketServer                       WebSocketClient
  world ──open──▶ session               connector ──open──▶ world
  world ──message*──▶ connector.out      world ──message*──▶ connector.out
  connector.in ──message*──▶ world       connector.in ──message*──▶ world
  session ──one exit──▶ connector.exit   session ──one exit──▶ connector.exit
```

`in`, `out`, and `exit` are ordinary ports. HTTP/1.1 Upgrade, HTTP/2
Extended CONNECT and HTTP/3 Extended CONNECT are Driver bindings; they do not
introduce `Http2`, `Http3`, `WebSocketH2` or `WebSocketH3` node names. A
session message is not an implicit statement form and does not receive a
per-message invocation `exit`.

HTTP-derived Connector attributes keep well-known and extension fields
separate without inventing a second literal syntax:

```rastok
headers = { content_type = media_type("application/json") }
custom_headers = { "X-Trace-Id": ["one"] }
```

The first struct is width-subtyped by the sheet; the second is the open
`map<text,array<text>>` namespace. Driver-owned and typed header names cannot
be restated through `custom_headers`.

### Cardinality belongs to the Node definition

A Graph is a Node, so it declares its own addressing exactly like an Operator
or Connector:

- **`graph X { … }`** without `instance.key` → one singleton inside each
  enclosing Graph instance;
- **`graph X { instance = { key = … }; … }`** → one logical instance per
  canonical selected key.

Keying is intrinsic to the definition, not re-decided at each edge. Lifetime
is orthogonal and uses its own attribute. A Graph-level immutable `val` may be
derived from `instance.key` and is lexically visible to descendants. Mutable
`var` state remains private to the Node that declares it; a Graph never lends
mutable memory to child Nodes.

### Graph composability

A graph references another graph **by name, as a Node** — a structural Graph,
not a special construct:

This composition excerpt omits the application adapter and complete boundary
types; the checked [resident dialogue](../../examples/agent-dialogue.rkg) is
the executable reference. Process itself has no `protocol`, `init` or `kill`
DSL attributes: an Operator interprets the protocol, and instance Closing
controls system shutdown.

```rastok
graph sessions {
  instance = { key = meta.app.session_key }
  lifetime  = { kind = idle ; after = duration("2h") }
  closing   = { drain_for = duration("30s") ; settle_for = duration("5s") }

  // Per-session immutable context derives from the selected local key.
  val codex_home = "/var/codex/" + instance.key
  val cwd        = "/workspace/" + instance.key

  connector codex : Process {
    command  = "codex"
    args     = ["app-server"]
    mode     = service
    cwd      = cwd                        // graph-level val, by name
    env = { "CODEX_HOME": codex_home }

    codec    = jsonl
    behavior = probabilistic
    // in / out / err / exit = stdin / stdout / stderr / child terminal
  }

  in        -> codex.in
  codex.out -> out
  codex.err -> nothing
  codex.exit -> nothing
}

graph teams_inbound {
  // Top-level → implicitly singleton.
  connector inbound : HttpServer {
    listen = "0.0.0.0:8080"
    codec  = json
  }

  // `sessions` — declared above, at the unit's top level — is
  // wired BY NAME : declaration is instantiation.
  inbound.out -> sessions.in
  sessions.out  -> ...
}
```

A declared Graph is a Node instance. Its logical cardinality comes from its
own optional `instance` attribute and its temporal behavior from `lifetime`.
Capacity, spawn strategy, placement, Worker count, and shard assignment are
physical deployment policy and are not inferred as source semantics.

Composition rules :
- A graph's `input <name>: <type>` declarations become inbound ports on the composed graph node.
- A graph's `output <name>: <type>` declarations become outbound ports on the composed graph node.
- A composed graph remains a **Graph** and is never reclassified as an
  Executor. It is effect-free when it contains no effective Connector and
  effectful when it wraps a boundary; that property belongs to analysis, not
  to the structural taxonomy.
- A Graph that contains a Node binding a unique resource (HTTP listener, TCP
  listen, etc.) cannot be key-instanced unless that Connector contract
  explicitly supports replication. Validation refuses the resolved contract,
  not the spelling of the key.

DAG validation runs **flat**: after expansion, Kahn rejects cycles across
composition boundaries too. This is a validation projection, not owner
erasure: the validated plan retains every Graph and its parent relation for
instance addressing, lifetime and lifecycle (instances.md §3).

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
  Connector producing INGRESS records) — is **dead** : diagnostic
  `W-DEAD-GRAPH` (warning, not an error — work-in-progress graphs are
  legitimate), and its boundary-port wiring obligations are suspended.
  Any partial wiring, or any source Connector inside, reinstates the normal
  explicit-destination obligation of every productible output. A deliberate
  discard is written `-> nothing`.

## Operator parameterization : state + instance + lifetime + transaction

Per [WHITEPAPER.md §2.3](../../WHITEPAPER.md), what informal vocabulary called
“Reducer”, “Aggregator”, “Window”, and “Joiner” are guide names for the same
Operator parameterized along four orthogonal axes. They are not aliases or
structural categories:

| Axis | Values | Effect |
|---|---|---|
| `var` state | absent / typed `S` | Operator-private state |
| `instance` | absent / one canonical key expression | singleton / keyed addresses |
| `lifetime` | absent / one profile | persistent / temporal instances |
| handler trigger | `on <port>` / `on (a, b)` | ordinary transaction / join |

The named archetypes map mechanically to combinations of these axes:

| Alias          | Equivalent Operator parameterization                                       |
|----------------|----------------------------------------------------------------------------|
| Pure transform | no `var`, no `instance.key` |
| Reducer | `var S`, no `instance.key` |
| Aggregator | `var S`, `instance.key = K` |
| Window | `var S`, optional `instance.key`, temporal `lifetime` |
| Joiner | optional `var S`, optional `instance.key`, `on (a, b)` bounded as required |

These names do not appear in the type annotation. The compiler sees the
ordinary Operator, state, instance policy, lifetime, and handler form directly.

Less frequent patterns (filter-with-dedup, buffer-by-size, rate-limit, debouncer,
memoize-cache, …) compose Operator state, expressions and lifetime policy rather
than new DSL keywords. jq/SQL guest evaluation is still a named capability gate;
its proposed spelling is not an executable alternative to current expressions.

### State semantics — read-modify-write per transaction

State is **read-modify-write, imperatively, within a transaction**. A persistent field `var x: T = default` is read by its bare name `x` (or `$x` inside a `jq! { … }` block — jq's own convention). At the *start* of a transaction `x` holds the value the previous transaction committed (or the default if uninitialized) ; assignments mutate it **in place**, so a later read in the same transaction sees the new value, like an imperative function. The transaction commits atomically (state + emissions + audit) at its end.

```rastok
operator counter {
  var count: int = 0                          // default 0 on first transaction
  // no `instance.key` → one Node-local singleton cell in the enclosing scope

  on msg {
    count = count + 1                         // Rastok expression; later reads see the new value
    out = { meta.app.count = count }          // default 'out' port
  }
}
```

**State expressions are Rastok expressions**: `count = count + 1`, `x = x * 2 + data.n`.
There are no mutation shortcuts (`x++` / `x += v`) ; they are redundant with
ordinary expressions. The proposed jq/SQL tagged guests for structural reshaping remain deferred;
current executable handlers use the checked Rastok expression subset.

**Emitting** : a braced value is classified ONCE by its top-level roots (D-2026-08-09-1). With only the reserved roots `data` and `meta` it is the ENVELOPE form : `out = { meta.app.count = count }` inherits the input envelope (meta + data) and patches the listed fields, and a whole-`data` assignment makes the data fresh. With no reserved root, `out = { total = n }` builds a fresh data record (a `struct`), keeping the input's heritable metadata per the uniform propagation rule (D-2026-08-10-2 ; `content_type` excluded — D-2026-08-10-4). Mixing the two refuses with `E-BRACE-MIXED` — business fields named `data` or `meta` take the explicit escape `data = { … }`. `out = .` preserves data and heritable metadata but creates a fresh emission identity; `out = nothing` makes that statement emit nothing. It never cancels an earlier emission. Every executed assignment to an output is retained, in source order, in the same atomic transaction. There is no `previous(x)` operator — a transaction reads the previous transaction's committed state at its start, then proceeds imperatively. The synchronous-dataflow `pre` of Lustre/SCADE applies at the **inter-transaction** boundary (each transaction is one cycle), not inside a transaction. `now()` observes the one logical timestamp fixed for the current firing; it never reads the host clock and its value is sealed when observable (see [expressions.md §1.2](expressions.md)).

## Dispatch, route selection, and keyed placement

Three different decisions remain separate:

| Decision | Surface | Semantics |
|---|---|---|
| transaction control | Operator `Dispatch` | select one checked handler branch |
| static route selection | `source -> match { ... }` | select closed destination sets from a source tag |
| keyed placement | `instance.key` | choose the logical Node instance |

None creates a dynamic topology. Every possible physical destination is known
in the validated spatial DAG.

A route match accepts only a tagged union, enum, or bool. It is exhaustive,
has disjoint case sets, does not evaluate expressions, and preserves the
complete tagged value:

```rastok
request.exit -> match {
  Completed                  -> nothing
  TimeoutExpired | DnsFailed -> retry
  _                          -> failed
}
```

A simple route and one match may coexist additively on a source; selecting the
same destination twice for one case is `E-ROUTE-DUPLICATE`. Case routing is
stored separately from `PhysicalEdge` identity.

The old boundary spellings `.` and `.foo` are removed from wiring. Current
Graph ports are named directly because ports and children share one scope:

```rastok
graph webhook {
  input request: HttpRequest
  output response: HttpResponse
  output error: ErrorReport

  request -> handler
  handler.out -> response
  handler.err -> error
}
```

Old dot-prefixed wiring receives a migration diagnostic. It is never an
alias. Equivalent explicit source must lower to the same plan and semantic
hash. Value-position `.`, for example `out = .`, remains the envelope
identity value.

Broadcast to multiple outputs is not a separate primitive — it is the
broadcast form of the atomic multi-emit statement :

```rastok
operator notify {
  on in {
    // RHS once; one distinct ordered envelope per declared output
    (sms, email, push, slack) = data
  }
}
```

This subsumes a separate `Fanout` type entirely. A future conditional form
will use the same ordinary emissions and declared ports; it will not add a
Router-specific selector contract.

## Instances and lifetime in the DSL

The complete semantic contract lives in [instances.md](instances.md). The host
surface uses ordinary composite attributes on the Node that owns the behavior:

```rastok
operator session_counter {
  instance = {
    key = meta.app.session_id
  }

  lifetime = {
    kind = idle
    after = duration("2h")
  }

  closing = {
    drain_for = duration("30s")
    settle_for = duration("5s")
  }

  var count: int = 0

  on msg {
    count = count + 1
    out = { meta.app.count = count }
  }
}
```

Composite correlation dimensions are one structured value, not an ordered
list of bindings:

```rastok
operator user_workspace_quota {
  instance = {
    key = {
      user = meta.app.user_id
      workspace = meta.app.workspace_id
    }
  }

  var quota: int = 0
  on msg { quota = quota + data.cost }
}
```

No `instance.key` means one singleton Node instance. No `lifetime` means
persistent. The admitted lifetime sheet is:

```rastok
lifetime = { kind = persistent }
lifetime = { kind = idle       ; after = duration("2h") }
lifetime = { kind = fixed      ; after = duration("30s") }
lifetime = { kind = single_pass }
lifetime = { kind = tumbling   ; every = duration("5m") }
```

Every row above produces a validated executable plan. Sliding and event-time
windowing remain named deferrals rather than partial lifetime spellings.

The Closing sheet is one closed composite:

```rastok
closing = {
  drain_for  = duration("30s")
  settle_for = duration("5s")
}
```

Both fields are required positive constant durations. Unknown, duplicate,
missing, nested, or non-duration fields refuse. There are no action fields and
no defaults. A finite owner whose subtree contains `Process` must declare the
sheet; elsewhere it is optional. Bare `stop` in a handler closes that current
owner generation after the handler transaction commits. A target operand and
`stop` inside `on stop` both refuse.

`instance`, `lifetime`, `closing`, `kind`, `key`, `after`, `every`,
`drain_for`, and `settle_for` are contract
attribute/field names, not structural keywords. The parser sees the ordinary
`attribute = value` production; the checker judges the closed common Node
sheet. `meta.sys` remains read-only but is readable by the key expression.
Only immutable `val`s from enclosing scopes are visible while selecting an
address; the owner's own body has not been entered yet. `instance.key` is
well-typed only for an owner that declares `instance.key` and never resolves
to an ancestor key.

There are no stdlib instance names. Reuse comes from expressions and types,
not a nominal mutable-state binding. A `var` stays private to the declaring
Node; Graph-level immutable `val`s remain lexical.

## Naming convention

| Element | Convention | Example |
|---|---|---|
| Declaration keyword | `graph` / `operator` / `connector` | `graph teams_inbound { ... }`, `operator classify { ... }`, `connector inbound : HttpServer { ... }` |
| Graph names | snake_case (graphs are named like modules, not like node types) | `teams_inbound`, `codex_session`, `api_to_s3` |
| Contract names and Extensions | PascalCase | `HttpServer`, `Process`, `App`, `Loop` |
| Node instances | snake_case | `inbound`, `to_codex`, `client_ws`, `sessions` |
| Ports | snake_case, role-named, **no direction suffix** (direction from declaration/use) ; `in`/`out`/`err` are conventional spellings | `event`, `delta`, `flush`, `matched` |
| Config keys | snake_case | `listen`, `nonce_ttl`, `jwks_url` |

Contract names describe a stable external role. Extension names describe a
separate orchestration protocol.
No `Reader`, `Writer`, or `Endpoint` suffix is needed in the DSL :

| Type | Role is inferred from | Examples |
|---|---|---|
| `HttpServer` | server address/binding | inbound request on `out`, optional response on `in` (default ports) |
| `HttpClient` | client address/binding | outbound POST, GET download, delivery/report output |
| `Process` | typed stdio + lifecycle ports | default `in`/`out`/`err`/`exit` = `stdin`/`stdout`/`stderr`/tagged invocation terminal, one child-process lifecycle per job call |
| `Jaq` | its transformer contract | pure `in -> out` envelope transform |
| `Memory` | its fixed store contract | `in` appends; `read` requests a bounded snapshot on `out` |
| `File` | `mode` plus the contract sheet | `read`: variant trigger → bytes; `replace`: bytes → terminal exit; both use a required invocation-capable `path` |

Rust implementation structs may use explicit names (`FileReader`,
`JsonRpcWriter`, etc.). The NodeRegistry exposes suffix-free DSL type names
and chooses the runtime halves from the node's ports and Connector attributes.
Direction and duplex behavior are properties of ports and Connector
contracts, not a second user-facing classification.

## Port naming and default edges

**Ports may be inferred from the handler.** `on <name>` contributes an input ;
assigning `<name> = …` in a handler declares an output. So Operators
usually declare *no* ports at all — the `input` / `output` keywords are
optional, needed only to **pin** a type/schema (`output x: T`), or to
document a node with no handler (e.g. a saga) or a graph's public
interface. Direction is carried by the keyword, not the name. The names
**`in` / `out` / `err`** are conventional default spellings — their usual
directions are known without a declaration, but a plan row
exists only when the program declares, handles, emits or wires it. They are
typed by the same mechanism as every other port. A `Process` maps them
exactly to stdin, decoded stdout and raw stderr; its ordinary `exit`
output carries the tagged lifecycle/outcome value. Conventions :

- ports are named for their role, **no direction suffix** (`event`, `delta`, `flush`)
- a single-input / single-output node just uses the defaults `in` and `out`
- a port's **type is inferred** ; annotate (`output x: T`) only to pin / force
- Connector contracts declare fixed or parametric port rules; the
  instantiated contract plus codec resolves their effective types
- an Operator is not required to have `err`; when present it conventionally
  carries an explicit domain alternative (`I -> Result<R,E>` for the simple
  one-in/one-out shape), never a synthesized machine fault
- `Process.err` specifically carries framed stderr; `err` has no special
  grammar, type or transaction semantics
- a Connector's `exit` conventionally carries the tagged terminal outcome of
  one admitted invocation; it is still an ordinary typed output

Every productible output is wired explicitly or sent to `nothing`:

```rastok
client.out -> codex.in
codex.out      -> client.in
codex.err      -> logs.in
codex.exit     -> lifecycle.in
```

Multi-port Operators (including the routing and aggregation patterns) expose additional named ports
via the same `.<name>` syntax — no separate `ports { ... }` declaration block
needed :

```rastok
operator route {
  input in: variant
  output audit: variant
  output invoice: variant
  output refund: variant

  on in match {
    data is Invoice => invoice = data
    data is Refund  => refund = data
    _               => audit = data
  }
}

route.audit   -> audit_log.in
route.invoice -> invoice_handler.in
route.refund  -> refund_handler.in
```

The possible destinations come from these declared outputs and their visible
edges. If several upstream edges target the same input port, the
runtime treats that as an implicit funnel / fan-in into the port. That default
merge carries no guard or ordering policy. A guarded fan-in such as "wait for
`a`, `b` and `c` for the same key before emitting" is expressed by an
`on (a, b, c)` join handler (§2.3), bounded by `last` (and optionally
`within`) — not the default funnel.

The Aggregator pattern is the stateful counterpart for keyed folds: fragments
arrive on a declared input such as `delta`, flush signals arrive on `flush`,
and the aggregate is emitted on a declared output such as `aggregate`.

Closed routing by source case is:

```rastok
codex.out -> match {
  Delta     -> { aggregate_reply.delta typing_feedback.in }
  Completed -> aggregate_reply.flush
  _         -> nothing
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

Every possibly productible output or closed output case requires a
disposition. Leaving one unwired is `E-UNWIRED-OUTPUT`, including at a nested
Graph boundary. Resolve it by wiring a consumer or writing `-> nothing`.
Only an output whose impossibility is re-derived from the validated program is
exempt. `W-DEAD-OUTPUT` applies only to outputs declared by the author.
Proved-impossible Loop exhaustion requires no output declaration. There is no implicit sink.

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

Real DSL primitives, not just sugar:

```
parse -> {
  store
  audit
  metrics
}

# one envelope, three delivery ids; target order is non-semantic
```

```
{
  api
  cron
  worker
} -> sink

# three independent mpsc sources; no join or source ordering
```

Destination sets reserve every target atomically and route one envelope.
Source sets expand to independent mpsc edges. `nothing` cannot share a
destination set with a real or manager destination.

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
| Protected repetition of one Call | a finite outbound Call with `+ Retry`, `Retry.on`, bounded attempts and the required idempotence guarantee; no authored Loop verdict |
| DLQ + replay | Failed → `Memory`; operations tooling reads a bounded snapshot, then injects selected values as roots of a new run |
| Iterative refinement, fixed N | Static unfolding : `step -> step -> step` |
| Iterative refinement, dynamic N | the finite `: Loop` Extension |
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
      "graph_version": 42
    },
    "app": { "user_id": "user@example.test" },
    "ext": { "content_type": "application/json" }
  },
  "data": { ... }
}
```

(The four `meta.sys` fields are fixed in [envelope.md](envelope.md).
Extension context lives in the journal. Ids are BLAKE3-derived 128-bit values.)

| Field | Writable by jaq |
|---|---|
| `data` | ✅ |
| All of `meta.*` | ❌ — guests READ metadata, never write it (v1, D-2026-08-10-1): metadata writes belong to the DSL, where the checker sees every key |

`meta` has exactly THREE zones (D-2026-08-10-1): `sys` (runtime,
read-only), `app` (application, read-write, inferred from the deployed
graph's writes), and `ext` (external-boundary context, read-write,
sheet-declared). Each zone is a flat struct whose leaves use the closed
`scalar | array<scalar> | map<text,array<scalar>>` metadata carrier; the map
form serves lossless protocol fields such as `http_headers`, while other
nested composites belong in `data`. There is no bare
form: `meta.customer_id` is invalid. Types unify across all writers of an
`app` field; `ext` fields come from Connector contracts plus the standard
`content_type`. `meta.sys` has four fixed identity fields; Extension context
lives in the journal, and Connector outcome data lives on its
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
`double`/`number`, `bool`, plus logical `media_type`/`date`/`timestamp`/`uuid`/
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
| `media_type` | `media_type("application/json")` | open canonical concrete Internet media type; wildcards/ranges refuse |
| whole payload | `asset("path", "content_type")` ; `base64!("ct") { … }` | fresh bytes + canonical `meta.ext.content_type`, whole-data position ONLY (a nested field has no `meta.ext`) |

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
parsed value is. Its failure is a declared typed exit case, not
a privileged `err` mechanism. A Parquet source can produce
`array<struct>` and an Avro source the same; JSON, Avro and Arrow unions
all project into the union forms in [types.md](types.md).

## References — Rastok expressions, without interpolation

The DSL has no `${…}` interpolation token.

- **Unit constants**: a top-level `val name[: Type] = value` is an immutable
  plan-time constant shared by the selected source files, the App and its Lab.
  All top-level constants are checked before Graph and Lab consumers, even if
  their file follows a consumer file. Initializers may refer to earlier unit
  constants in compilation/source order; forward references and cycles refuse.
  They cannot read an envelope, instance state, logical time, `env()` or
  `secret()`. Duplicate names and local bindings or join aliases shadowing a
  global refuse `E-DUP-BINDING`; `data`, `meta`, `instance`, `Loop` and `now`
  cannot be global binding names. The expression/value grammar and declared-type
  rules are unchanged. Annotated unit constants, including annotated aliases,
  remain a named gate: use a Graph or Operator binding so data-policy metadata
  cannot disappear through substitution.
  A unit constant is folded once and substituted with its checked type before
  production HIR; it creates no state cell or package field. Unused constants
  and constants used only by Lab do not enter the graph identity. Replacing a
  scalar literal with a constant holding that same literal preserves the plan
  and hash. Computed/typed constants are not a general algebraic-equivalence
  promise ([identity.md §1](identity.md#1-the-semantic-hash)).
  Expression-valued Connector attributes accept visible constant names;
  literal choices such as `codec = lines` retain their Contract meaning.
- **Boot phase target** (unit/App boot `val`s and effective Connector settings
  whose selected evaluation site is boot) : the exact root reference forms
  **`env("VAR")`** and **`secret("path")`**. M1.8 A4 executes only
  `HttpClient.url = env(...)` and
  `HttpClient.auth_bearer = secret(...)`; unit/App boot `val`s and every other
  setting remain named gates until their consumers land. Using either form in an instance key,
  per-instance `val`, guard, handler-local binding, or state expression is a
  compile error which names the compatible setting rather than reporting an
  unknown function — the language has explicit phases (boot → instance →
  transaction → frontend region).
- **Instance phase** : a Node-level `val` is derived once for each new
  instance from enclosing immutable `val`s and its local `instance.key`; it
  cannot read `data`, `meta`, or mutable state. A Graph's values are then
  lexically visible to descendants; Operator/Connector values stay private.
  A Connector contract may permit an operational attribute as late as the
  instance site; the same bindings are legal there. The checker selects the
  earliest legal actual site, so constants still resolve at plan and an
  `env()`-only expression at boot. Profile, port-type and other plan-shaping
  attributes permit only plan resolution.
- **Runtime** : plain expression paths, no sigil — `instance.key` (the current
  owner instance key), `meta.app.x`, `data.y`, `exit is TimeoutExpired`, a
  visible `val`/`var`/alias by name. Handler-local `val`s are transaction
  bindings.
  String building is expression concatenation :
  `val cwd = "/workspace/" + instance.key`.
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
| `HttpServer` | Connector node | exact inbound-Call sheet; `ack` and `respond` execute over HTTP/1.1–3 |
| `HttpClient` | Connector node | exact H2 outbound-Call sheet; live HTTP versions execute at H4 |
| `Process` | Connector node | specified in the initial stdlib catalog ; codec/protocol matrix must remain narrow |
| `File` | Connector node | executable C3 sheet: explicit one-file `read` / atomic `replace`; `watch` and directory operations are named deferrals |
| `Postgres` | Connector node | deferred; must use the standard Connector contract |
| `Sqlite` | Connector node | deferred; must use the standard Connector contract |
| `Timer` | Connector node (one-shot or periodic timestamp events) | specified in the initial stdlib catalog; not the runtime time authority |
| `Calendar` | Source Connector | civil time (cron+tz / at) — see stdlib.md |
| `Memory` | bounded in-process store | initial catalog; append and snapshot-read contract in stdlib.md |
| `Jaq` | possible future Executor form or Operator engine (jq guest over structured values) | frontend is registered; execution contract remains deferred |
| `Sql` | possible future Executor form or Operator engine (SQL guest over `array<struct>`) | planned guest |
| Reducer pattern | ordinary Operator with `var`, optionally keyed | executable without an annotation |
| Aggregator pattern | Reducer with `instance.key` | executable without an annotation |
| Window pattern | stateful owner with a finite `lifetime` | processing-time profiles executable; sliding/event-time deferred |
| Joiner pattern | Operator with an `on (a, b)` join handler | executable without an annotation |
| Router pattern | closed/open Dispatch emitting to declared output ports | executable; `: Router` refuses |
| Fanout pattern | atomic multi-emit statement `(a, b, c) = x` | executable; no category |
| Dispatch | recursive `Linear | ClosedMatch | OpenMatch` within one Operator transaction | executable |
| `Saga` Extension | future compensation orchestration | direction only; fail closed |
| `Loop` Extension | finite temporal recurrence over an admissible host | executable in [loop.md](loop.md) |
| `-> nothing` | Discard sink (built-in) | not a node — `nothing` as an edge target ; non-blocking drain |
| `RequestReply` | Correlated call pattern | design intent ; may lower to Connector + Operator composition |
| `Metrics` | Sink Connector | future |
| `Join` / `Barrier` | Guarded fan-in | the canonical form is an `on (a, b)` join handler (correlated by the owning Operator instance ; bounded by `last`/`within`) |
| `WebSocketServer` | Connector node | exact both-role Session sheet; accepted duplex execution over H1/H2/H3 |
| `WebSocketClient` | Connector node | exact both-role Session sheet; opened duplex execution over H1/H2/H3 |
| `Tcp` | Connector node | planned |
| HTTP/1.1, HTTP/2, HTTP/3 | Binding options, not nodes | all three execute for HTTP Calls and WebSocket Sessions; negotiated selection is observation, not node identity |

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
- `mock sessions.codex { on in where ... { out = ... } }` uses the Lab's
  Driver-script grammar. Its handler-shaped clauses describe the simulated
  external peer; they are not Connector-owned DSL handlers and do not become
  application `Dispatch` in the plan. An uncovered external call fails the
  scenario loudly.
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
| `graph` | declaration keyword for a structural Graph Node |
| Executor | Node category with bounded activations and atomic transitions |
| Operator | Executor whose transition is described by DSL Dispatch and private `var` state |
| Connector | Executor which owns one versioned Contract; its transition is described by that Contract and typed Driver events |
| Extension | protocol attached to a Node, such as `App`, `Loop`, or future `Saga` |
| Operator parameterization | Four orthogonal choices — Node-local `var`, optional `instance.key`, optional `lifetime`, and ordinary/join handler — covering the Reducer/Aggregator/Window/Joiner guide patterns. |
| Dynamic patterns | Router = Dispatch with static outputs; Fanout = destination set or multi-emit; neither adds a Node category |
| Instance | One logical execution context of a Node owner. Its address is the owner path plus the outer-to-inner stack of keyed instance segments. A singleton owner has no local key segment. See [instances.md](instances.md). |
| Lifetime | The policy that says when an instance closes. It is independent from key selection: keyed and singleton owners may both be persistent or finite-lived. |
| State read-modify-write | **Imperative within a transaction.** `x` (bare in DSL, or `$x` inside `jq! { … }`) starts at the value the previous transaction committed (or the default); assignments mutate it in place, so later reads see the new value (RHS = Rastok expression; no `++`-style shortcuts). The transaction commits atomically at its end. The Lustre `pre` is *inter-transaction*, not intra. |
| Value types | The data's type tree : `variant` (dynamic, JSON model), scalars (`text`/`media_type`/`bytes`/numeric + logical `date`/`timestamp`/`uuid`/`decimal`/`duration`), composites (`struct`/`map`/`array`), `enum` (closed symbols), `union` (sum type). `table` is the relational view of `array<struct>`, not a type. See [Data](#data--typed-values-and-literals). |
| Envelope | `meta + data`. `meta` is three zones of typed scalars (`meta.sys` read-only/runtime, `meta.app` application-owned, `meta.ext` external context — D-2026-08-10-1) ; `data` is the typed payload. See [WHITEPAPER.md §2.1](../../WHITEPAPER.md). |
| Edge config | The `{ ... }` block after an arrow, configuring `capacity / backpressure / close / trace`. |
| Tagged frontend | `name! <region>` — source owned by a registered reader/compiler. |
| Port naming | Ports are role-named with no direction suffix ; direction comes from the `input` / `output` keyword or use position. `in` / `out` / `err` are conventional spellings, not keywords or mandatory rows. |
| Branchement | Fan-out (`a -> { b; c }`) or fan-in (`{ a; b } -> c`). Real DSL primitive. |
| Endpoint | Connector that owns an external resource address/lifecycle and declares explicit envelope ports. |
| `scenario` | one executable case inside a `simulation` : a fixture (the scripted world-half of the journal) + expectations. See simulation-semantics.md. |
| `mock` | scenario/simulation-level stand-in for a calling Connector — its body is ordinary handler grammar (+ `after`). Replaces the Driver, not the bookkeeping. |
| Multi-tenancy | Expressed by a keyed owner (`instance = { key = … }`). The runtime materializes one logical instance per canonical key and may assign each keyed boundary to an independent virtual shard. Environmental isolation and physical placement remain outside this grammar. |

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
// There is no `${…}` INTERP atom. Boot-time resolution reserves the exact root
// forms `env("NAME")` / `secret("path")`. M1.8 A4 admits them only for the
// declared HttpClient URL/bearer settings; unit/App boot vals remain a named
// gate. The contract declares the latest site, and checking chooses the
// earliest legal one.
// runtime references are plain Rastok expression paths (`instance.key`, `meta.app.x`).
// Rastok expression spans are handed to the expression lexer: the DSL lexer only
// finds the span's END (delimitation rules below).
// Maximal-munch : "->", "<->", "==", "!=", "<=" and ">=" tokenize as single
// atoms — `=` vs `==` is a KIND question, never a premature diagnostic
// (Note : "<-" is not a Rastok token ; wiring is always graph-level.)
// Comments and whitespace are skipped by the lexer.
```

### Reserved keyword sets

```rastok
DECL_KW    = { "graph", "operator", "connector",
               "type",                                                 // unit-level type alias (D-2026-08-10-8)
               "val", "var", "input", "output", "on",
               "start", "stop", "match",                               // Operator lifecycle and Dispatch
               "where", "having", "within", "last",
               "lab", "simulation", "experiment", "theorem", "over",  // the Lab surface
               "scenario", "defaults",
               "expect", "envelope", "advance", "mock",
               "after", "crash", "restart",                            // scenario steps (simulation-semantics.md §5)
               "as" }
// `profile`, `behavior` and `idempotent` are ordinary Contract attribute
// names, not global effect-class keywords. Removed `effect` refuses.
// Simulation setup uses ordinary fixture injections.
TYPE_KW    = { "variant",                                             // the value-type tree (types.md)
               "struct", "map", "array", "table", "enum", "union",   // composites (+ table view)
               "bool", "int", "long", "uint", "float", "double", "number", // friendly scalar aliases
               "int8", "int16", "int32", "int64",
               "uint8", "uint16", "uint32", "uint64",
               "float16", "float32", "float64",                    // canonical explicit widths
               "bytes", "text", "media_type", "null",
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
               "deterministic", "probabilistic", "seeded",        // behavior values (stdlib.md §2 ; seeded reserved)
               "ack", "respond",                                  // HttpServer modes
               "job", "service",                                  // Process modes
               "jsonl", "lines",                                  // Process codecs; `bytes` is TYPE_KW but legal as a contract word
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
`decimal(...)`, `media_type(...)`) are ordinary calls.
`TYPE_KW` names the value-type tree — `variant` (the explicit dynamic case), the
composites, and the scalar physical / logical types. `LITERAL_KW` are enum-like
idents allowed as scalar values.

Simulation setup uses fixtures: injection is the setup, and preloading a
`Memory` is expressed as ordinary injections at its feeding edge. Reading
uses the Connector's explicit `read` and `out` ports.

The conventional port spellings **`in` / `out` / `err`** are not keywords. A
Connector contract may additionally conventionally declare **`exit`** for one
invocation's terminal fact. Their direction is known when used and they may be
explicitly restated once in their conventional direction where the owning node
kind permits declarations; a second declaration or contradictory direction is
`E-PORT-CONFLICT`. An unused conventional Operator port has no row. M1.3
materializes every effective row in the resolved port
table exactly like any other port; no runtime layer recreates one from its
name.
Reservation is BY POSITION, never a global ban on a spelling
(D-2026-08-12-1; enforcement stated exactly by D-2026-08-13-1
review round 3 — this section supersedes the earlier draft that
claimed `DECL_KW` unmaskable):

- **Binding positions** (graph, lab, node, port — boundary and node
  alike — val/var/let at every level, type names): the value forms
  `nothing` and the expression literals `true`, `false` and `null` are
  RESERVED — one naming policy, refusing at the
  declaration with `E-NAME-RESERVED`. The envelope/context surfaces
  (`meta`/`data`/`sys`/`app`/`ext`/`instance`) additionally refuse as node
  names, and `data`/`meta`/`instance` as binding names. `DECL_KW` words
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
Concrete Contract names such as `HttpServer`,
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

graph_decl        ::= "graph" IDENT ( ":" extension_list )? "{" graph_body "}"     // declaration = instantiation; common Node attributes carry instance/lifetime

operator_decl     ::= "operator" IDENT ( ":" extension_list )? operator_body
connector_decl    ::= "connector" IDENT ":" connector_annotation connector_body

// The declaration keyword fixes Graph / Operator / Connector.
// A Connector starts with exactly one Contract; Graph and Operator contain only
// Extensions. Extension order is canonical and non-semantic.
connector_annotation ::= IDENT ( "+" IDENT )*
extension_list    ::= IDENT ( "+" IDENT )*

operator_body     ::= "{" ( port_decl | val_decl | var_decl
                          | handler_decl | attribute )* "}"
                                                                      // sub_block is DEAD (D-2026-08-12-1); parsed only to refuse
                                                                      //   with its migration hint
connector_body    ::= "{" ( port_decl | val_decl | attribute )* "}"  // no DSL var or handler in M1
// D-2026-09-16-1: a Connector port_decl only refines a Contract-admitted
// output type; M1 implements HttpClient/json/out. It never adds a port.

graph_body        ::= ( port_decl | operator_decl | connector_decl
                       | val_decl | edge_decl
                       | defaults_block
                       | graph_decl                            // graphs nest as scopes (extensions.md §1)
                       | attribute )*                          // Graph has no var or handler

type_decl         ::= "type" IDENT "=" type_ref                   // TOP-LEVEL only ; one namespace per unit ; duplicates refuse ;
                                                                  //   recursion refuses (v1) ; budgets: depth ≤ MAX_TYPE_NESTING (128), expanded
                                                                  //   tree ≤ 4096 nodes, ≤ 256 fields/struct, ≤ 64 KiB (D-2026-08-10-8)

// A Node carrying `: Loop` uses ordinary qualified attributes:
//   Loop.fuel = 5
//   Loop.delay = { kind = exponential; first = duration("100ms");
//                  ceiling = duration("5s") }
// `Loop.measure` admits the L6 direct signed-integer ranking fragment;
// Every richer theorem spelling fails closed. `Loop.entry` and
// `Loop.terminal` disambiguate host interfaces. Loop.next / again / done are
// manager commands, not ports; Loop.expired selects an existing host terminal.

port_decl         ::= ( "input" | "output" ) IDENT ( ":" type_ref )?  // source annotation OPTIONAL; every validated port has an effective type
                    // in / out / err are conventional spellings and use this same model; only used/declared rows materialize in the resolved plan.
                    // Connector contracts declare fixed or parametric port rules; attributes/codecs resolve them per instantiation.
                    // No port-side inbound shortcut : wiring is always graph-level.
                    // To bind a port to a source, declare the Connector then wire it :
                    //    connector c : Timer { every = duration("1s") }
                    //    c -> a.tick

val_decl          ::= "val" IDENT ( ":" type_ref )? "=" value        // immutable, scope by position
var_decl          ::= "var" IDENT ":" type_ref ( "=" value )?        // mutable state owned by this Operator; never inherited by child Nodes

handler_decl      ::= "on" trigger where_clause? join_bound* handler_dispatch
trigger           ::= "start"
                    | "stop"
                    | "(" IDENT ( "," IDENT )* ")"                   // join : presence of all named ports
                    | IDENT                                          // single port
where_clause      ::= "where" expr                                   // one-port: consuming filter sugar; join: pre-consumption admission guard
join_bound        ::= "within" expr                                  // legal on a join only; positive constant duration
                    | "last" INT                                     // legal on a join only
                    // Join ports are distinct declared inputs. Their source order fixes bindings and identity.
                    // A bounded join declares `output expired: union { JoinExpired {} }` exactly in R6.
                    // Join expressions use `<port>.data` / `<port>.meta`; bare `data` and `out = .` refuse.
                    // `by` is removed: another correlation domain is another `instance.key` owner.
handler_dispatch  ::= dispatch | "{" val_decl* dispatch "}"
dispatch          ::= linear_dispatch | closed_match | open_match
                    // At most 32 nested match decisions per path, including
                    // the match introduced by a single-port `where`.
                    // Source and checked HIR refuse E-DISPATCH-DEPTH before
                    // recursive evaluation; the plan validator re-derives the bound.
linear_dispatch   ::= statement*
closed_match      ::= "match" expr "{" closed_arm ( sep closed_arm )* "}"
closed_arm        ::= case_set "=>" arm_dispatch
open_match        ::= "match" "{" open_arm* fallback_arm "}"
open_arm          ::= expr "=>" arm_dispatch
fallback_arm      ::= "_" "=>" arm_dispatch
arm_dispatch      ::= statement | handler_dispatch
case_set          ::= IDENT ( "|" IDENT )* | "_" | BOOL
statement         ::= "(" lvalue ( "," lvalue )* ")" "=" tuple_or_broadcast   // atomic multi-emit
                    | "stop"                                          // close the current Node instance after this transaction commits
                    | "nothing"                                       // no emission on this Dispatch leaf
                    | "Loop.again" | "Loop.done"                       // manager Control commands
                    | "Loop.next" "=" value                           // manager Data command
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
path_tail         ::= ( "." ( IDENT | TYPE_KW | LITERAL_KW ) )*       // the dotted walk shared by lvalues and expression prefixes ; reserved segments after a dot permit qualified names such as Loop.fuel and stdlib ports such as store.read
lvalue            ::= IDENT path_tail                                 // assignment target: port, var, or dotted writable meta/data path — meta.sys is read-only, never an lvalue
tuple_or_broadcast ::= "(" value ( "," value )* ")"                    // explicit tuple — one value per lvalue, counts must match
                    | value                                            // broadcast — same value to every lvalue


type_ref          ::= union_type ( "?" )? data_annotation*           // T? optional == T | null; annotations target this resolved position
union_type        ::= base_type ( "|" base_type )*                    // sum type
base_type         ::= scalar_type
                    | "variant"
                    | "struct" "{" ( field_decl ( "," field_decl )* )? "}" // typed record — EMPTY struct { } is legal
                    | "@" "struct" "{" ( field_decl ( "," field_decl )* )? "}" // flat record; scalar/enum/optional fields after alias resolution
                    | "map" "<" type_ref "," type_ref ">"
                    | "array" "<" type_ref ( "," INT )? ">"              // optional exact length; zero is legal
                    | "table" "<" type_ref ">"                        // alias for array<struct>
                    | "enum" "{" IDENT ( "," IDENT )* "}"
                    | "union" "{" tagged_case ( ","? tagged_case )* "}" // tagged union; case names are semantic discriminants
                    | IDENT                                           // named type / Schema / declared enum|union
scalar_type       ::= "bool" | signed_type | unsigned_type | float_type | "number"
                    | "bytes" ( "(" INT ")" )? | "text" | "media_type" | "null"
                    | "decimal" "(" INT "," SIGNED_INT ")"
                    | "date" ( "(" date_unit ")" )?
                    | "time" ( "(" time_unit ")" )?
                    | "timestamp" ( "(" time_unit ( "," timezone )? ")" )?
                    | "timestamp_ntz" ( "(" time_unit ")" )?
                    | "duration" ( "(" time_unit ")" )?
                    | "interval" "(" interval_unit ")"
                    | "uuid"
signed_type       ::= "int" | "long" | "int8" | "int16" | "int32" | "int64"
unsigned_type     ::= "uint" | "uint8" | "uint16" | "uint32" | "uint64"
float_type        ::= "float" | "double" | "float16" | "float32" | "float64"
date_unit         ::= "day" | "millisecond"
time_unit         ::= "second" | "millisecond" | "microsecond" | "nanosecond"
timezone          ::= "utc" | STRING                                 // UTC aliases normalize during resolution
interval_unit     ::= "year_month" | "day_time" | "month_day_nano"
                    | "month_day_millis_unsigned"
SIGNED_INT        ::= "-"? INT
field_decl        ::= field_name ":" type_ref
                    | ".."                                            // open record (row polymorphism)
tagged_case       ::= IDENT data_annotation* ( "{" ( field_decl ( ","? field_decl )* )? "}" )?
field_name        ::= IDENT | TYPE_KW                                 // field NAMES may reuse type words (`text: text`,
                                                                      //   `kind: text`) — position disambiguates
data_annotation   ::= "@" IDENT annotation_arguments?                // M1.8 closed family; A4 admits attested personal data, executable raw @secret sites defer at check
annotation_arguments ::= "(" named_annotation_argument ( "," named_annotation_argument )* ")"
named_annotation_argument ::= IDENT "=" annotation_constant
annotation_constant ::= IDENT ( "." IDENT )* | STRING | SIGNED_INT  // plan constants only; no expression or positional argument

attribute         ::= lvalue "=" value                               // dots = owner qualification ONLY (Loop.delay = …)
struct_body       ::= ( attr_entry ( ","? attr_entry )* )?
attr_entry        ::= lvalue "=" value                               // dotted key for an assignment (meta.app.label = … ; meta.sys is read-only)
// sub_block (IDENT "{" struct_body "}") is a DEAD form (D-2026-08-12-1):
// the parser still recognizes it to refuse with the migration hint
// « `env { … }` is no longer a configuration form; write `env = { … }` ».

// ============ WORD CLASSES (D-2026-08-14-1) ============
// Three roles a word can play — the factoring below names them:
// - a USER IDENTIFIER introduces a binding (val/var/let, node, port, type,
//   alias). `nothing` is RESERVED here — it names a structural
//   value form and refuses AT THE DECLARATION (E-NAME-RESERVED) ; the
//   envelope/context surfaces (meta/data/sys/app/ext/instance) refuse as node
//   names, and data/meta/instance as binding names.
// - any IDENT immediately followed by BANG introduces a tagged frontend.
//   No closed keyword vocabulary participates in this syntax decision.
//   After this syntax-only choice, the Registry resolves the name and framing
//   contract before any payload byte is consumed.
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
                    | "{" struct_body "}"                             // tagged-union constructor; CHECK resolves IDENT as one case
                    | expr_continuation                               // anything else : expression beginning with the word

// IDENT { ... } is parsed without a type-name table. It becomes a genuine
// tagged-union value only when CHECK resolves exactly one compatible case.
// Otherwise a former initializer spelling retains its migration diagnostic;
// this does not revive declaration/configuration sub_block.

// In the frontend branch, IDENT and `!` are byte-adjacent: whitespace or a
// comment between them refuses. A tagged frontend fills this whole `value`
// non-terminal and is not an atom inside `expr`; bind its result before
// combining it with intrinsic operators. Maximal munch decides `!=` before
// this production, so `json!=x` is the expression `json != x`, never a
// frontend followed by `=`.

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
                       exists/all/map/filter/has/size macros,
                       duration()/timestamp()/media_type(),
                       optional_types (`.?` / `.orValue()`), declared join aggregates.
                       Macro iteration over a MAP is in SORTED-KEY order (an
                       evaluator guarantee — golden-trace stability).
                       `is <type-or-case>` tests/narrows and `as <type_ref>` performs a
                       checked conversion at the END of the span.
                       NO leading dots (paths are bare : data.x, meta.app.y, instance.key,
                       alias names). No coalesce shorthand and no
                       string-interpolation tokens. `env()`/`secret()` are
                       legal only in boot-site positions. `now()` is a reserved
                       zero-argument context function available only in firing
                       handlers; it returns the transaction-fixed logical
                       timestamp, never reads an ambient clock, and seals that
                       observation in the transaction record. Bare `now`,
                       arguments and user bindings named `now` refuse. */
expr_path         ::= /* a Rastok expression access path (subset of expr) : IDENT ( "." IDENT )* —
                       used by correlators and key extractors. */

// DELIMITATION of an expr span — how the DSL lexer finds the end.
// The span runs at BRACKET DEPTH 0 (the expression lexer tracks ( ) [ ] { } nesting,
// so expression map literals `{"a": 1}` — the SAME punctuation as the DSL map,
// D-2026-08-10-6 — and struct-ish braces inside the span are fine)
// until the position's terminator set :
//   handler guards    (where)                 → { within last NEWLINE "{" "match" }
//   attribute / init  (= …)                   → { NEWLINE ";" "}" }
//   emission          (out = …)               → { NEWLINE ";" "}" }
//   match predicates / paren positions        → { "=>" "," ")" } at depth 0
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
// type contract and compiler. `asset`, `duration`, `decimal` and `media_type` remain
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

edge_decl         ::= path BIARROW path edge_config?
                    | path ARROW ( path ( ARROW path )* ( ARROW terminal_destination )?
                                 | terminal_destination ) edge_config?
                    | source_set ARROW routed_destination edge_config?
source_set        ::= "{" path ( sep? path )+ "}"
routed_destination ::= path | terminal_destination
terminal_destination ::= destination_set | edge_match | "nothing"
destination_set   ::= "{" simple_destination ( sep? simple_destination )+ "}"
simple_destination ::= path | "nothing"
edge_match        ::= "match" "{" edge_arm ( sep edge_arm )* "}"
edge_arm          ::= case_set ARROW ( simple_destination | destination_set )
path              ::= IDENT ( "." ( IDENT | TYPE_KW | LITERAL_KW ) )*
sep               ::= NEWLINE | ";"
// Set members may also be separated by ordinary lexical whitespace, as in
// `source -> { archive audit }`. Match arms retain their explicit separator.

edge_config       ::= "{" struct_body "}"                            // applied to every expanded PhysicalEdge
// Dot-prefixed boundary wiring and `when` are dead migration forms.
// A route match is closed case selection; it never evaluates an expression.
// Loop.next / Loop.again / Loop.done parse as qualified paths, then resolve
// as manager commands rather than ports. The grammar itself makes a set,
// match, or `nothing` terminal. A resolved manager command must also terminate
// the declaration; only an ordinary port path may continue a chain. A
// source_set cannot start a chain.
// instance.stop / child.instance.stop also resolve as command destinations,
// never ports or PhysicalEdges. Scope is this Graph or one direct child;
// source/intermediate/deep-target use refuses with E-INSTANCE-COMMAND.
// The exact target instance is bound before producer commit; see instances §7.3.

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

mock_decl         ::= "mock" IDENT ( "." IDENT )* "{" handler_decl* "}"   // dotted = a composed node (sessions.codex) ; handler-shaped Lab
                                                                      // Driver script, not Connector-owned application Dispatch ; two levels :
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
| `graph_body` | `{` | `edge_decl` (`source_set`) |
| `graph_body` | `.` | DEAD boundary-wiring form → migration refusal with the explicit port name |
| `graph_body` | `graph` / `operator` / `connector` / `val` / edge starts | the matching declaration; Graph has no `var` or handler |
| `graph_body` | `input` / `output` | `port_decl` |
| `graph_body` | `on` / `var` | DEAD Graph execution form → named migration refusal |
| `graph_body` | `graph` | nested `graph_decl` |
| `graph_body` | IDENT (dotted ok) — next is `=` | `attribute` (`Saga.timeout = duration("10m")`, `name = …`) |
| `operator_body` | `input` / `output` | `port_decl` |
| `operator_body` | `val` | `val_decl` |
| `operator_body` | `var` | `var_decl` |
| `operator_body` | `on` | `handler_decl` |
| `operator_body` | IDENT — next is `=` | `attribute` |
| `operator_body` | IDENT — next is `{` | DEAD form → migration refusal (`name = { … }`) |
| `connector_body` | `input` / `output` / `val` / attribute | corresponding declaration |
| `connector_body` | `on` / `var` | refusal: a Connector's transition and state belong to its Contract |
| `trigger` | `start` / `stop` | lifecycle handler |
| `trigger` | `(` | join handler `(a, b)` (presence) |
| `trigger` | IDENT | single-port handler |
| after one-port `trigger` | `where` | consuming filter sugar, lowered to OpenMatch plus `_ => nothing` |
| after join `trigger` | `where` / `within` / `last` | pre-consumption admission guard and finite bounds |
| `dispatch` | `match` followed by expression | closed match over a finite domain |
| `dispatch` | `match {` | ordered open predicate match with mandatory fallback |
| `dispatch` statement | `stop` / `nothing` / `Loop.again` / `Loop.done` | corresponding terminal/control statement |
| `dispatch` statement | `Loop.next =` | Data manager command |
| `dispatch` statement | `(` | tuple/broadcast multi-emit |
| `dispatch` statement | lvalue followed by `=` or `:` | assignment / port emission |
| `dispatch` statement | anything else | refusal; no implicit yield or mid-block match |
| `attr_entry` | `lvalue` (IDENT, dotted ok) — next is `=` | dotted attribute |
| `attr_entry` | IDENT — next is `{` | DEAD form → migration refusal (`name = { … }`) |
| `value` | IDENT — next token is `!` | `value_after_ident`: tagged frontend; the Registry resolves the name only after the grammar branch is chosen |
| `value` | IDENT — next token is `{` | `value_after_ident`: tagged-union constructor shape; the checker resolves the case from its typed destination or one unambiguous named union |
| `value` | IDENT — any other follower | Rastok expression / bare word, judged by the expression checker or contract |
| `value` | `{` — then `}` / `:` / STRING / IDENT | `braced_value` : empty struct / empty map / map_entries / struct_entries |
| `value` | `.` (envelope position) | identity — exact re-emission |
| `value` | `nothing` (emission position) | no-emit |
| `value` | anything else | hand the span to the Rastok expression parser (`expr`; `is`/`as type_ref` compose at span end) |
| `port_decl` | `input` / `output` IDENT — next is `:` | typed (pinned) port |
| `port_decl` | `input` / `output` IDENT — no `:` | port with **inferred** type |

The "two-token" decisions happen at the IDENT crossings (e.g.,
`graph_item` IDENT followed by `{` vs `=` vs `->`). These remain LL(1) with one
symbol of lookahead after the IDENT is consumed. The parser never needs to
*backtrack*.

### What this grammar excludes on purpose

- **One braced form, classified once** (D-2026-08-09-1, punctuated by D-2026-08-10-6). `{ … }` with `=` entries is the struct literal, with `:` entries the map literal (`{}` empty struct, `{:}` empty map, mixing refuses) ; in ENVELOPE positions (handler emissions, fixture injections, mock replies) the checker classifies a struct-form body's top-level roots — no reserved root builds a fresh data record, only reserved roots (`data`, `meta`) select the envelope form, a mix refuses with `E-BRACE-MIXED`. There is no `envelope { … }` keyword form, no `struct { … }` value literal, and no `map { … }` spelling. No silent reinterpretation, LL(1)-clean.
- **No closed frontend-keyword partition.** Built-in and user-registered frontends
  use `name!`; a bare `name` remains an identifier. `@` introduces M1.8
  schema annotations only; it never selects guest dispatch.
- **Separators are Scala-style, scoped by bracket.** Inside a **`{ }` block**
  (node body, composite attribute such as `instance`/`lifetime`, struct), a newline
  terminates each statement / attribute and **`;` is the optional same-line
  separator** (`{ kind = idle ; after = duration("2h") }`). Inside **`( … )`** (compact
  values and argument lists) items are separated by **`,`**
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
parser can re-synchronize on the next `operator`, `connector`, `graph`,
`lab`, `simulation`, `experiment`, `theorem`, or
top-level IDENT.

### Reserved-but-not-yet-used keywords

For forward compatibility, the lexer reserves a few additional identifiers
that may enter the DSL later :

```
compose    // reserved for a possible future reuse-instantiation form
foreach    // future per-item fan-out node
spawn      // future explicit keyed-instance spawn directive
watermark  // future event-time watermark on an instance/window
           // (`theorem` is an ACTIVE keyword with a reserved surface.
           //  Bounded verification is an OPERATION on a theorem, never a DSL
           //  block ; `rastok check` stays the CLI static-validation verb.)
replay     // future replay directive (audit-only / logical / exact)
```

These are reserved so existing `.rkg` files using them as plain identifiers
won't suddenly break when the parser learns them.
