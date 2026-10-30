---
title: Diagnostics
description: Compiler error codes, their trigger, the source change to make, and message excerpts from exercised refusals.
order: 5
---

A graph that leaves a possible outcome with nowhere to go does not compile.
`E-UNWIRED-OUTPUT` enforces that rule, including outputs that carry a fault.

Codes identify refusals; messages explain the particular source location.
The code and structured fields are stable, while message wording can change.
The examples below are excerpts collected from compiler runs on test inputs
or from the internal-plan diagnostic regression.

`check`, `simulate` and `run` all compile source first. A Parse or Check
error therefore appears before simulation or physical I/O starts. The CLI
prints a source span and, where relevant, the expected and found values.
See [Command line](../guide/09-command-line.md) for exit status and invocation.

## Parse

Lexing and parsing report malformed source, unavailable syntax and excessive
decision nesting.

| Code | When it fires | What the author changes | Example message excerpt |
|---|---|---|---|
| `E-DISPATCH-DEPTH` | Nested decisions exceed the Dispatch bound; also checked after parsing. | Split the decision into Operators instead of adding nesting. | “nested `match` decisions exceed the Dispatch depth budget” |
| `E-JSON-VALUE` | A json! literal is not valid JSON. | Correct the JSON inside the literal. | “the JSON value does not read: unexpected `?`” |
| `E-LEX-CHAR` | Character outside the source grammar. | Remove or quote the character. | “the character `-` is not part of the grammar” |
| `E-LEX-UNTERMINATED-BLOCK` | Raw frontend block has unmatched delimiters. | Balance its braces and embedded strings. | “this raw block never closes” |
| `E-LEX-UNTERMINATED-COMMENT` | Block comment reaches end of file. | Close the comment. | “this block comment never closes” |
| `E-LEX-UNTERMINATED-STRING` | String reaches a line ending without its closing quote. | Close the string or use the appropriate multiline frontend. | “this string never closes before the end of the line” |
| `E-PARSE-DEFERRED` | The parser recognizes a construct that this build does not implement. | Use the executable subset or omit that construct. | “an experiment syntax is not implemented by this build yet” |
| `E-PARSE-EXPECTED` | A token cannot continue the current production. | Follow the expected token or statement shown at the source span. | “expected `{`, found `lab`” |

## Check

Name resolution is included here because it happens before a plan exists.
Its structured phase is `resolve`; the remaining rows have phase `check`.
Independent plan validation can also surface these semantic refusals.

### Names and declarations

| Code | When it fires | What the author changes | Example message excerpt |
|---|---|---|---|
| `E-APP-COUNT` | The compilation unit has zero or several App roots. | Keep exactly one graph annotated App. | “a deployed unit needs exactly one `: App` graph, found 0” |
| `E-DUP-LAB-IDENTITY` | Simulation, scenario, mock or origin declarations conflict. | Remove the repeated declaration or distinguish its name. | “the simulation `a` is already declared in lab `g`” |
| `E-DUP-NODE` | Two children share a name in one Graph. | Give each child its own name. | “the node name `o` is already declared in this graph” |
| `E-DUP-TYPE` | Two type declarations share a name. | Keep one definition for that name. | “the type `T` is already declared” |
| `E-LAB-NO-APP` | A Lab name does not identify the unit's App. | Use the App graph's declared name after lab. | “the lab `absent` binds to no `: App` graph in this unit” |
| `E-NAME-CLASH` | A child and a Graph port share a name. | Rename one declaration. | “`work` already names a Graph port; ports and children share one namespace” |
| `E-NAME-RESERVED` | A declaration uses a language-reserved name. | Choose a name that is not reserved. | “`nothing` is a reserved word” |
| `E-RESOLVE-UNKNOWN` | A referenced name has no declaration in scope. | Declare the name or correct its spelling. | “`ghost` names neither a Graph port nor a child Node in this Graph” |
| `E-SIM-TARGET` | A simulation targets something other than app. | Simulate the complete App boundary. | “a simulation closes `app`; sub-graph targets are a deferred capability” |

### Types, wiring and execution rules

| Code | When it fires | What the author changes | Example message excerpt |
|---|---|---|---|
| `E-ASSET` | A build-time asset cannot be read within its root and size limits. | Correct the literal path, placement or asset size. | “this compilation has no asset root” |
| `E-BRACE-MIXED` | A braced value mixes data members with envelope roots. | Place ordinary members inside data. | “this braced value mixes data fields with reserved envelope roots; wrap the data in `data = { … }`” |
| `E-CHECK-DEFERRED` | A parsed construct has no executable lowering in this build. | Use a supported value or construct; also reported during Plan. | “a duration, timestamp, or null emission is outside the executable subset of this build” |
| `E-CONNECTOR-HANDLER` | A Connector contains a DSL handler. | Put application decisions in an Operator; the Driver implements the Contract. | “a Connector cannot own DSL handlers; its Contract and Driver events define its transitions” |
| `E-CONNECTOR-STATE` | A Connector declares application-owned mutable state. | Keep that state in an Operator. | “a Connector's semantic state is defined by its Contract; author private `var` state in an Operator” |
| `E-CONNECTOR-UNAVAILABLE` | The compiler has no executable definition for that Contract. | Choose a Contract and mode listed by rastok connectors. | “`Timer` is declared in the catalogue; its Driver is not in this build” |
| `E-CONTENT-TYPE` | A content-type qualifier appears inside a nested value. | Apply it to the whole payload instead. | “a content type qualifies the WHOLE payload” |
| `E-CONTRACT-ATTR` | A Connector attribute violates its mode's definition. | Use an allowed value and evaluation site from the Contract catalogue. | “`codec` violates the Process sheet: expected one of ["bytes", "jsonl", "lines"], got `utf16`” |
| `E-CONTRACT-REQUIRED` | A required Connector setting is missing. | Declare that setting with the required type. | “the Process sheet requires `behavior`” |
| `E-DATA-ANNOTATION` | A field annotation cannot be resolved or applied consistently. | Correct its name, constant arguments, target or measurement unit. | “unknown data annotation” |
| `E-DISPATCH-POSITION` | A decision or guard occurs after work or in a conflicting position. | Place the decision first and combine duplicate guards. | “a handler admits at most one `where`; combine predicates or use one `match` dispatch” |
| `E-DUP-BINDING` | A binding shadows a name already visible in its scope. | Rename the new binding. | “`limit` shadows an already visible binding” |
| `E-DUP-KEY` | Two literal map keys canonicalize to the same value. | Keep a single entry for each canonical key. | “the map key `a` is already present” |
| `E-EXPR` | An expression is invalid, unbound or outside the supported expression subset. | Correct the expression at the reported span. | “`mystery` is not a visible name” |
| `E-EXTENSION-CONFLICT` | Extensions make incompatible demands on one owner. | Separate the owners, for example a Retry Connector inside a Loop Graph. | “Loop and Retry on the same owner are not admitted; put the Retry Connector inside a Loop Graph” |
| `E-GRAPH-CYCLE` | Static wiring contains a cycle. | Keep wiring acyclic and express bounded recurrence with an Extension. | “the wiring cycles through `a`; edges form a DAG” |
| `E-GRAPH-HANDLER` | A Graph contains application handler code. | Move the handler to an Operator child and wire it. | “a Graph cannot own application handlers; move the handler into an Operator child and wire its ports” |
| `E-GRAPH-STATE` | A Graph declares private mutable state. | Move the state and transitions to an Operator. | “a Graph is a scope and cannot declare private `var` state; move the state and its handlers into an Operator child” |
| `E-HANDLER-DUPLICATE` | An Operator repeats a handler for one trigger. | Combine its alternatives in one match. | “an Operator has exactly one handler for each trigger; put the alternatives in one `match`” |
| `E-INSTANCE` | Instance keys, lifetime or closing settings do not satisfy their contract. | Correct the reported setting and supply bounded closing where required. | “closable owner `shell` contains outbound Connector `shell` and therefore requires `closing = { drain_for = <duration>; settle_for = <duration> }`” |
| `E-INSTANCE-COMMAND` | An instance closure command is used in an invalid routing position. | Use it only as a permitted final destination in its owning context. | “an instance closure command is a destination, never a source port or an intermediate endpoint” |
| `E-LOOP` | A Loop setting or finitude condition is invalid. | Supply a positive fuel bound or a provably decreasing measure. | “a `+ Loop` Executor requires positive `Loop.fuel` or a `Loop.measure` that can be proved finite” |
| `E-LOOP-CARDINALITY` | Repeated frames could produce several verdicts for one application. | Derive the verdict from a once-per-application terminal. | “this Contract may produce several frames on the port; a Loop application needs exactly one verdict” |
| `E-LOOP-CONTROL` | The checker cannot prove one verdict for every application path. | Complete the verdict paths and required finite Call timeouts. | “cannot prove that Loop `c` always terminates at `timeout`: an outbound Call in Loop needs a positive plan-constant timeout” |
| `E-LOOP-ENTRY` | No unique recurrent input can be selected. | Declare the input and Loop.entry consistently. | “Loop needs one recurrent Graph input; declare `Loop.entry = <input>`” |
| `E-LOOP-HOST` | The host cannot provide a finite Loop application. | Use an Operator, admissible Graph or finite outbound Call. | “Loop requires an Operator or a resolved outbound Call Contract with closed frame and terminal guarantees” |
| `E-LOOP-TERMINAL` | The selected terminal ports are ambiguous or incompatible. | Correct the host terminal declaration; Call terminals come from their Contract. | “every `Loop.terminal` item must name one output port” |
| `E-MATCH-DUPLICATE` | One source has more than one closed route match. | Combine them into one route match. | “a source may have at most one closed route `match`” |
| `E-MATCH-NONEXHAUSTIVE` | A possible case has no arm, or an ordered match lacks a fallback. | Cover the remaining cases or add the required fallback. | “an open match requires a final `_` fallback” |
| `E-MATCH-NOT-CLOSED` | A closed match scrutinizes an open or unsupported type. | Match a closed union, enum or bool, or use ordered predicates. | “route `match` requires a pinned tagged union, enum, or bool source” |
| `E-MATCH-OVERLAP` | Two closed match arms cover the same case. | Assign each case to one arm. | “route case `true` appears in two arms” |
| `E-META-PREFIX` | One application metadata key is a prefix of another. | Choose keys that are consistently leaves. | “`meta.app.audit` is a leaf and `meta.app.audit.origin` makes it a branch” |
| `E-META-SYS-READONLY` | A write targets metadata owned by execution. | Write application metadata under meta.app instead. | “meta.sys is read-only and runtime-owned; it is never an lvalue” |
| `E-META-TYPE` | Writes to one application metadata field disagree on its type. | Use one type or separate fields. | “`meta.app.x` is written as Integer(Signed(Bits32)) and as Text” |
| `E-META-ZONE` | Metadata access leaves the allowed zones or their field vocabulary. | Use a full permitted path under sys, app or ext. | “a metadata zone is not a value” |
| `E-MULTI-EMIT-ARITY` | The output tuple and value tuple have different lengths. | Provide one value per output or use broadcast syntax. | “this atomic multi-emission has 3 targets but 2 tuple values” |
| `E-MULTI-EMIT-DUPLICATE` | An atomic emission lists the same output twice. | Name each output once. | “output `left` appears twice in one atomic multi-emission” |
| `E-NODE-ANNOTATION` | A Node's Contract and Extension annotation is invalid. | Declare the Contract and compatible Extensions in the required order. | “a Connector requires its exact Contract first, before every Extension” |
| `E-PORT-CONFLICT` | A port is redeclared or inferred with incompatible roles. | Give the port one declaration and direction. | “`chan` is already an input port; it cannot also be an output (an emission target)” |
| `E-PORT-DIRECTION` | A port is used as a source, destination or emission against its direction. | Reverse the wiring or select the appropriate port. | “`chan` is an input port; an emission targets an output port” |
| `E-PORT-TYPE` | A destination cannot accept the inferred value. | Constrain the port or narrow the value without an implicit lossy conversion. | “the boundary port `in` has no effective payload type; add a type constraint” |
| `E-PORT-UNKNOWN` | A Node has no port with the referenced name. | Use a declared port from that Node or Contract. | “`o` exposes no port named `mystery`” |
| `E-RETRY` | A Retry setting is missing, repeated, misplaced or invalid. | Supply the required policy with positive bounds and valid settings. | “`Retry.attempts` must be a positive integer, first attempt included” |
| `E-RETRY-CASES` | Retry.on does not select distinct declared terminal cases. | Use case names from the Contract's exit union, once each. | “`SpawnFailed` appears twice in Retry.on” |
| `E-RETRY-EFFECT` | A selected retry case requires a repetition guarantee that is absent. | Select a before_effect case or declare a justified idempotence guarantee. | “Retry after `TimeoutExpired` (outcome_unknown) requires idempotent = true” |
| `E-RETRY-HOST` | Retry is attached to something other than a supported outbound Call. | Put the policy on the outbound Call Connector. | “Retry requires an outbound Call Connector” |
| `E-ROUTE-DUPLICATE` | Routing selects the same destination twice for one case. | Remove the duplicate route, including equivalent shorthand. | “destination `sink.in` is selected twice for the same source case” |
| `E-ROUTE-MIXED` | One case is routed both to nothing and to a destination. | Choose discard or delivery for that case. | “`nothing` cannot coexist with a real destination for the same source case” |
| `E-TYPE-BUDGET` | Type expansion exceeds a nesting, node, width or alias bound. | Reduce the dimension reported in the diagnostic. | “the type `Deep` nests 129 levels deep” |
| `E-TYPE-DUP` | A field or symbol is repeated inside one type. | Remove or rename the duplicate member. | “the struct field `x` is declared twice” |
| `E-TYPE-INVALID` | A type constructor violates its invariants. | Correct its parameters, keys or branches. | “array&lt;int32&gt; is not a canonical map-key type” |
| `E-TYPE-RECURSIVE` | A named type expands back to itself. | Use a finite nonrecursive type. | “the type `Node` does not expand: `Node` reaches itself” |
| `E-TYPE-UNRESOLVED` | An inference constraint reaches a position needing a closed type. | Supply a concrete type or enough contextual constraints. | “an open struct is an inference constraint, not an effective type” |
| `E-UNION-OVERLAP` | Structural-union branches overlap in the values they accept. | Use disjoint branches or a tagged union. | “structural-union branches `int32` and `int64` overlap” |
| `E-UNWIRED-OUTPUT` | A possible output or case has no explicit disposition. | Wire it to a destination or deliberately discard it with nothing. | “`o.side` can be produced but has no destination; route it or write `o.side -> nothing`” |
| `E-WIRING-MIGRATION` | Wiring uses a removed spelling or continues after a command. | Use current port paths, match routing and final command destinations. | “edge `when` was removed; route a closed value with `-> match { ... }` or decide in an Operator” |

## Plan

`E-PLAN-DEPTH` also occurs at Check when an expression exceeds its
128-level bound. Plan checks the combined serialized representation against
the reader's separate 256-level bound, with the root at level zero.

| Code | When it fires | What the author changes | Example message excerpt |
|---|---|---|---|
| `E-INTERNAL-PLAN` | Compiler output fails independent plan validation. | Report the source and diagnostic as a compiler defect. | “the lowering emitted an invalid plan” |
| `E-PLAN-DEPTH` | The complete serialized plan exceeds the reader's bound; expressions also have a smaller Check bound. | Reduce combined nesting; the message identifies the boundary. | “the serialized plan reaches level 256; the format requires a level below 256 (root is level 0)” |

The internal-plan case is a compiler defect, not an invitation to weaken a
type or discard an outcome. Its message is exercised by
`compiler::tests::the_internal_plan_diagnostic_carries_its_registered_payload`.

## Simulate

The current runner reports scenario failures, missing observations and golden
differences as scenario results, not as additional structured `E-…` codes.
For a golden mismatch it names the scenario, record, expected and obtained
values, and cause. Correct the graph or the scenario; save a new trace only
after reviewing the intended change.
See [The Lab](lab.md#golden-traces) and
[Echo](../examples/01-echo.md#read-the-golden-difference).

A `TimeoutExpired`, `JoinExpired` or `LoopExpired` value is a typed
outcome that the graph routes. It is not a compiler diagnostic.

## Run

Deployment refusals, unavailable physical Drivers and recording/replay
errors currently use contextual command errors, without a separate registry
of emitted `E-…` codes. Use the reported Connector path, missing binding or
format mismatch to correct the deployment or select the matching artifact.
Command usage errors go to stderr with status 2; compilation and execution
errors return a nonzero status.
See [Command line](../guide/09-command-line.md).

## Specified, not shipped

Source registry: `docs/spec/diagnostics.md`, “Registry”.

| Code | Status in this build | Specification |
|---|---|---|
| `E-LAB-DUP` | Lab fragments merge; duplicate declarations use `E-DUP-LAB-IDENTITY`. | `docs/spec/grammar.md` |
| `E-EXPERIMENT-PROFILE` | Experiments refuse at the parser's deferred-syntax boundary. | `docs/spec/grammar.md` |
| `E-SAGA-DURABILITY` | Saga durability checking is a deferred tool surface. | `docs/spec/effects-and-sagas.md` |
| `E-COMPENSATE-KEY` | Compensation-key checking is deferred with Saga. | `docs/spec/effects-and-sagas.md` |
| `E-REPLAY-VERSION-MISMATCH` | Replay checks compatibility, but does not emit this structured code. | `docs/spec/identity.md` |
| `E-LOOP-CONTEXT` | The name occurs in an execution refusal, not a structured compiler diagnostic. | `docs/spec/loop.md` |
| `E-DUP-EDGE` | Declared but not emitted; duplicate destinations use `E-ROUTE-DUPLICATE`. | `docs/spec/envelope.md` |
