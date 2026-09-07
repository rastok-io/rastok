# R0 identity ledger — target categorical plan v16 and Loop r2

> Final allocation and the single golden update are enumerated in
> [R9 qualification](rework-r9-qualification.md). This document records
> the pre-implementation movement contract, not a remaining code gate.

> Frozen before implementation on 2026-09-05. This is a pre-commit ledger, not
> a claim that v16 has landed. Numeric tags and keys are allocated in R4 only
> after the source/HIR shapes stabilize; no existing number may be reused.

## 1. Three independent registries

| Registry | v16 rule |
|---|---|
| package | every encoded package moves categorically from format 15 to 16 once; v15 refuses |
| semantic | only source meaning changed by the new ontology/routes/Dispatch/Loop or Connector `@2` changes its semantic hash; spelling-only desugaring is neutral |
| causal | only executions using new dispositions, Dispatch branches, explicit output routing, multi-emission, or Loop r2 change records; unrelated traces remain byte-identical |

No implementation checkpoint may re-bless the package twice. R3 keeps new
source forms fail-closed; R4 allocates the complete v16 representation in one
wave.

## 2. Categorical plan movements

1. `AspectSpec` becomes closed `ExtensionSpec`; the Loop variant gains entry
   and terminal resolution while App is represented as the root Extension.
2. handler `guard + [Step]` becomes recursive `Dispatch`; joins retain a
   separate admission guard.
3. `HandlerOwner::Graph` and activation `ChildPort = 4` are retired. Their tags
   remain reserved.
4. `RouteRule` is added beside `PhysicalEdge`; closed case sets select canonical
   disposition sets. Nested Graphs retain the unflattened inventory at sparse
   body key 11; decode re-derives the flat selections and crossed boundary
   obligations. Plans without nested boundaries omit the key.
5. manager commands become a destination category but never an endpoint or
   PhysicalEdge.
6. destination/source sets become canonical expansion material.
7. ordered multi-emission becomes executable plan data distinct from one
   envelope with a destination set.
8. `PortDecl.unwired` and `UnwiredPolicy` are removed. The old mode values remain
   retired; direction is encoded independently.
9. output/case productibility evidence becomes re-derivable validation
   material. No trusted `possible` boolean is admitted.
10. Loop's old `next` port is removed; `Loop.expired` is the sole Loop-contributed
    output; `Loop.next`, `Loop.again`, and `Loop.done` are commands.
11. Loop r2 carries exact opening-admission identity, entry port, terminal set,
    command facts, and run-context propagation.
12. six executable standard Contract identities move once:
    `rastok/process@2`, `rastok/file@2`, `rastok/http-client@2`,
    `rastok/http-server@2`, `rastok/websocket-client@2`, and
    `rastok/websocket-server@2`.

## 3. Explicit non-movements

- `owners`, `OwnerPath`, parent-first containment, instance policy, lifetime,
  Closing, lexical owner values, and owner generations remain authorities.
- `node::Type` has no independent wire row today; deleting the Rust/HIR concept
  is not itself a package-field removal.
- envelope spine, `DataType`, `Value`, annotations, metadata zones, and
  `graph_version` are unchanged.
- the three journal record kinds remain INBOUND, TRANSACTION, OUTBOUND.
- ordinary transaction consumed positions, state delta, emissions, routes,
  timers, exchange records, and lifecycle triggers remain.
- timer kind 5 remains Loop delay; no kind is renumbered.
- Connector roles and `Event | Call | Session` tags remain.
- `behavior = deterministic | probabilistic` remains the Connector attribute.
- the Rust/HIR name `ConnectorId` becomes `ContractId`, but its validated text,
  wire representation and semantic role are unchanged.
- removing the retired raw handler-owner enum leaves each valid handler's
  owner encoded as `[1, operator_slot]`. Owner tag 2 and activation tag 4
  still refuse by name; removing their Rust variants is not another wire move.
- App/public-boundary semantics remain, despite App moving from `node::Type` to
  `Extension` in source/HIR.

## 4. Causal extensions, append only

R4 allocates unused transaction/inbound subkeys for these meanings:

```text
opening_loop_admissions : [
  OpeningAdmissionRef,
  LoopRunRef,
  entry_port,
  initial_state,
  initial_budget,
  LoopContext
]

manager_commands_produced : [
  ManagerCommandRef,
  LoopRunRef,
  iteration,
  kind,
  payload-envelope-or-cause
]

manager_commands_consumed : [ManagerCommandRef]

loop_context on routed admission : [LoopRunRef*]

loop_fact_r2 : Open | Advance | Admit | Close | Expired
```

The exact numeric keys are deliberately not guessed in R0. Existing key 53
(`loop_fact` r1) is never reinterpreted. R4 either appends a new transaction key
for r2 or uses a version-disjoint closed subform whose decoder cannot confuse r1
and r2; the preferred choice is a new key because old journal canonical forms
are extend-only. The domains `rastok.loop.r1` and `rastok.aspects.r1` remain
historical. New identities use distinct domains:

```text
rastok.loop-run.r2
rastok.loop-command.r1
rastok.loop-admission.r1
```

Domain byte constants are allocated only with independent collision and
round-trip tests. A retired byte is never reused.

### Retained timing allocation at R4 (2026-09-07)

D-2026-09-07-2 allocates key **56**, `timer_schedule`, in the retained INBOUND /
TRANSACTION view: `[armed_at_ns, [[kind, reference_bytes16, due_at_ns], ...]]`.
The field uses exact signed instants and actual arm order. It is deliberately
absent from the causal seal; storage must retain and integrity-protect it
atomically with the accepting record. Key 37 and observable `now()` key 54
keep their existing meanings. No package bytes or witness identities change.
The strict field codec and Loop-delay producer land before v16 is blessed;
R7 captures the remaining temporal profiles without needing another allocation.

## 5. Ordering and semantic identity

| Construct | Order semantic? | Canonical treatment |
|---|---:|---|
| Extension list | no | sort by closed Extension tag |
| source set | no | sort unique canonical endpoints |
| destination set | no | sort unique canonical dispositions |
| closed match arms/cases | no | expand and sort by type tag/case tag |
| open match predicates | yes | retain source order |
| linear statements | yes | retain source order |
| multi-emission targets | yes | retain written order |
| physical deliveries of one destination set | no | sort route records canonically |
| join ports | yes | retain declared tuple order |

One-port `where P body` is lowered to the same OpenMatch as
`match { P => body; _ => nothing }` before semantic hashing. Removing `.` and
`.foo` wiring sugar is also neutral: an accepted migration twin must produce
the same v16 plan body and semantic hash after desugaring.

## 6. Historical witness ledger before R4

The table below records the v15 baseline, not the current package. The blessed
v16 sizes, hashes and checksums are in
[R9 §1](rework-r9-qualification.md#1-identity-inventory-before-blessing).

v15 witnesses recorded before the single bump:

| Witness | Semantic hash | v15 package (body + header) | v15 categorical checksum | Expected v16 semantic result |
|---|---|---:|---|---|
| `examples/echo.rkg` / embedded spike | `e50026eeb0b8dc77fe2f5e52de6dfaa33b50cc351211cd87c21081d28aa43709` | 988 B | `73ea33a7…4847a` | moves: Process Contract `@2` and explicit dispositions |
| JSONL Process golden | `036f7ea5346fac883a3bad155fbdf41b303f6e54bef93f8c850ff5f138f769de` | 1,032 B | recorded in M1.8 ledger | moves: Process Contract `@2` and explicit dispositions |
| connector-free `examples/gauge.rkg` | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | 323 B | recorded in M1.8 ledger | **must remain semantically byte-identical** if only wiring sugar changes |

Package containers for all three move categorically to v16 even when semantic
hash does not. The gauge is the negative semantic-scope witness. Its source
currently uses boundary sugar, so the R9 migration must prove old-sugar and
explicit-boundary twins before the old spelling begins to refuse.

**Measurement correction, 2026-09-06.** The original heading called these
988/1,032/323-byte sizes “body”. They include the package header. Independent
header decoding finds body sizes **907/951/243 bytes**, respectively. This
corrects the ledger's unit, not any stored fixture or semantic identity.

Additional R4/R9 witnesses required:

- a one-port `where` / explicit OpenMatch semantic twin;
- reordered closed arms and destination-set members with identical plan/hash;
- reordered open arms and multi-emission targets with different plan/hash;
- one output proven impossible, showing no `nothing` edge or hidden policy;
- all six `@1 -> @2` Contract moves enumerated by source and resolved contract;
- three Loop r2 reference traces, including produced-but-unconsumed recovery;
- a non-Loop, non-Connector causal trace which stays byte-identical.

## 7. Re-bless protocol

Before blessing any golden in R4/R9:

1. print old/new package version, body size, checksum, semantic hash, and trace
   seal for every moved witness;
2. attribute each changed byte range to one item in §2;
3. prove every §3 non-movement with an unchanged semantic or causal witness;
4. decode and independently validate the package before accepting its compiler
   hash;
5. run the three `no_std` targets and the Cortex-M33 spike before calling the
   registry closed.

No golden moves solely because a test expected v15 bytes.

## 8. R3 parser-conformance movement (not plan identity)

`crates/rastok/tests/fixtures/minimal.ast` moves from
`HandlerDecl.body: Some([Statement])` to
`HandlerDecl.dispatch: Some(Linear { statements, span })`. The existing
statement, source text and statement spans remain unchanged; the new Linear
node owns the block span `49..60`. This snapshot records the intentional R3
AST algebra change, not a package, semantic-hash or causal-trace move.
The parser fixture test was re-run after the explicit snapshot update. No
`.rkp` or trace golden was re-blessed in R3; the three §6 witnesses stay fixed.
