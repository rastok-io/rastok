# C1 — identity inventory before the Retry cutover

15 September 2026. Baseline: `c51f165`, executable package **v18**, diagnostic
recording **v2**. This is the target allocation sheet for C2 review, not an
announcement that the writer or reader accepts these forms. No golden is
changed in C1. Read with the [algebra](retry-c1-algebra.md),
[reference traces](retry-c1-traces.md) and current
[identity registry](../spec/identity.md).

**C1 review amendment:** key 59 below is uniform across outbound Call
closures, not conditional on Retry (D-2026-09-15-3). The Loop warning is
explicit in the C2 checklist; pending-manager and retained-journal capacities
have separate qualification obligations. No executable allocation changes in
this documentary follow-up.

Use one package cutover, proposed **v19**, and one recording cutover, proposed
**v3**, after their complete schema and independent validators are ready.
Do not silently amend a v18 body or reinterpret a v2 CallOpen tuple. C2 must
land the complete declared representation, even where execution remains
fail-closed until C3. A missing field discovered later returns to this inventory
before blessing; it is not permission for an undocumented format movement.

## 1. Current facts verified in code

| Registry / authority | Baseline |
|---|---|
| [`plan/serialization.rs`](../../crates/rastok-core/src/plan/serialization.rs) | FORMAT_VERSION=18; ExtensionSpec App=2, Loop=3; old Loop tag 1 retired |
| ConnectorSpec map | 1 ContractId, 2 effect, 3 behavior, 4 attributes, 6 roles, 7 exchange |
| LoopSpec map | 1 fuel, 2 delay, 3 proof, 4 entry, 5 terminals, 6 resolved expiry target/type |
| [`connector.rs`](../../crates/rastok-core/src/connector.rs) | process@2 job, process@3 service; File and four network Contracts @2 |
| [`journal.rs`](../../crates/rastok-core/src/journal.rs) | three record species; OUTBOUND Call causal fields do not include CallOpen.effect |
| [`loop_protocol.rs`](../../crates/rastok-core/src/journal/loop_protocol.rs) | sparse key 55, r2 facts and derivation domains 0x14 / 0x15 / 0x06 |
| [`timing.rs`](../../crates/rastok-core/src/journal/deadline.rs) | retained key 56, armed_at and ordered due arms; not part of the causal seal |
| [`instance_stop.rs`](../../crates/rastok-core/src/journal/instance_stop.rs) | causal key 57 and command domain 0x23 |
| [`recording/codec.rs`](../../crates/rastok/src/recording/codec.rs) | CallOpen tuple includes an effect word; Call tuple has only CallOpen and status |

Existing lifecycle activation tags run through 10 (`Exit`); `Next=4` remains
Loop's meaning. Saga timeout kind 4 is a different registry from Extension
tags. It is not allocated to Retry. No existing tag or domain below is reused.

## 2. Plan and semantic hash

**Subsequent design gate (D-2026-09-15-5):** the approved
[`@struct` annotation model](../spec/annotations.md) requires an additional
exact inventory for annotation definitions/bindings, generic decimal measures
and monetary desugaring before its implementation. The tables below remain
the Retry candidate inventory, not an allocation for that later metadata
work. The subsequent [common binding inventory](annotation-types-m1.md#41-common-binding-inventory-16-september-before-implementation)
appends annotation tuple tag 6 inside this unblessed v19 candidate, without
a new root key, Contract, recording field or version. Its first executable
binding is descriptive metadata (1); unsupported interpretation tags refuse.
The final pre-blessing amendment,
[18 September inventory](annotation-types-m1.md#19-two-field-measure-migration-inventory--18-september-before-code),
gives binding 2 exactly `id: text` and `unit: text` under
`rastok/measure@3`. All quantities, including amounts, use it. Old implicit
measure tag 3, cost tag 4 and protected definition `@2` refuse by name;
none is reinterpreted. Security/privacy tags and the attested deployment
profile representation remain unchanged. Numeric DATA, original emissions and
their causal provenance are not revalued.

The earlier 16 September source-only `@cost` migration was neutral at that
checkpoint. The subsequent two-field simplification is deliberately not:
annotated hashes and packages move for their schema/definition or retired
compact interpretation. It introduces no root key, Contract major, journal
fact, additional version or separate blessing.

| Movement | Target |
|---|---|
| package header/checksum | v18 → v19; every package container moves |
| ConnectorSpec key 2 | retire effect; presence refuses by name, never interpreted as a boolean |
| ConnectorSpec key 8, new | resolved idempotence PlanConstant for admitted outbound Call profiles; boolean and plan-constant independently checked |
| non-Call profiles | no key 8; exact Contract opening semantics replace effect-based role shortcuts |
| ExtensionSpec tag 4, new | `[4, RetrySpec]`; canonical list order, duplicate/conflicting Extensions refused |
| RetrySpec key 1 | positive total attempts, PlanConstant |
| RetrySpec key 2 | canonical set of selected terminal case names, validated against the exact Contract |
| RetrySpec key 3 | optional fixed/exponential delay, existing delay encoding and bounded values |
| RetrySpec keys 4 / 5 | resolved request entry / Call terminal names, checked against the Contract |
| HttpClient terminal type | ServerError { status: int } added; usable 500–599 only; Completed covers the other admitted final statuses |
| HttpClient JSON output (16 September amendment) | Explicit `output out: T` refines the existing output under the JSON hydration profile; `T` and its annotations use existing port/schema tables, no new key; unrefined output remains `variant` |
| HttpClient projected JSON (17 September amendment) | Opt-in plan attribute `json_profile = projected` in the existing attribute table; required JSON/struct output independently checked. No default row, new key, journal fact or recording tuple. Strict plans and request descriptors stay unchanged; opting in changes the program hash. |
| frame cardinality | canonical set of response-producing terminal cases replaces the single-case internal rule |

The guarantee is normalized from the Contract and an admitted author constant:
Process job/HttpClient omission equals false; File read/replace remains fixed
true. It is not a per-invocation expression. The plan's effective attribute
and resolved guarantee must agree; the latter is not a second user choice.
Original binding is required on recovery. No idempotence boolean is added to
CallOpen or any retained attempt fact.

Retry settings are hashed once in a new sparse `rastok.retry.r1` semantic
section, ordered by owner, using the canonical policy values and resolved
ports. Extension spelling order and `Retry.on` order do not change identity;
attempt count, selected cases or delay do. A duplicate case is rejected rather
than silently deduplicated. The old effect attribute disappears from the
existing node attribute stream; idempotence follows its effective Contract
default there. Do not create an unrelated global hash prefix for pure graphs.

Keep `rastok.loop.r2`, LoopSpec and its canonical hash contribution unchanged.
There is no new Loop delay restriction, measure alias, port, command, Dispatch
or DataType. Sharing Rust machinery is not itself a semantic hash movement.

## 3. Contract majors — exact profiles, no blanket renaming

| Contract | Baseline | Cutover target | Reason |
|---|---|---|---|
| Process job | rastok/process@2 | rastok/process@4 | remove global effect; declare plan-owned Call idempotence |
| Process service | rastok/process@3 | rastok/process@5 | remove inherited effect requirement; preserve local Session opening |
| File read/replace | rastok/file@2 | rastok/file@3 | replace fixed effect class with fixed Call guarantee |
| HttpClient | rastok/http-client@2 | rastok/http-client@3 | guarantee, ServerError/status/cardinality, explicit typed JSON output and opt-in projected profile (D-2026-09-16-1 / D-2026-09-17-1), all before candidate blessing |
| HttpServer | rastok/http-server@2 | rastok/http-server@3 | remove effect sentinel; preserve peer admission / Call response roles |
| WebSocketClient | rastok/websocket-client@2 | rastok/websocket-client@3 | remove effect requirement; preserve local opening |
| WebSocketServer | rastok/websocket-server@2 | rastok/websocket-server@3 | remove effect sentinel; preserve peer opening and orphan-write disposition |

Process job and service cannot both become `process@4`: distinct contracts
already occupy one versioned name. All physical Drivers, Lab bindings,
documentation and independent core mirrors must use the matching new profile.
No new Contract version means Saga or Retry support for Session exchanges.
Old identifiers refuse; no compatibility alias gives them the new semantics.

## 4. Shared journal, distinct causes

Preserve causal key 55 and its r2 meanings. Add a sparse **key 58** for Retry
to TRANSACTION and outbound Call facts; it is absent on unrelated records,
INBOUND facts and Sessions. One recurrence implementation consumes both
policy subforms. Retry never manufactures a Loop command or changes an r2
Advance's cause type.

The proposed packet is a closed map with empty entries omitted:

```text
retry_packet := { 1: [operation_ref, attempt]?, 2: [fact+]? }
```

Key 1 correlates a Call open/closure or Connector completion with one admitted
attempt. Key 2 carries these tagged facts in validated causal order:

| Tag | Fact payload after the tag | Required carrier |
|---:|---|---|
| 1 | Opened(operation, opening_delivery, entry, original_plan_hash, ContractId, operation_pin) | manager TRANSACTION |
| 2 | Admitted(operation, attempt, admitted_envelope, delivery_ref) | manager TRANSACTION |
| 3 | Held(operation, attempt, CallRef, ordered_emission_ids) | that Call's Connector TRANSACTION |
| 4 | Consumed(operation, attempt, CallRef, ordered_emission_ids) | manager TRANSACTION |
| 5 | Advance(operation, from_attempt, to_attempt, CallRef) | same manager TRANSACTION as Consumed |
| 6 | Published(operation, attempt, CallRef, ordered_emission_ids) | that Call's Connector TRANSACTION |
| 7 | Close(operation, attempt, reason) | with Published, or manager after Closing |

`reason` is `1 = final`, `2 = stopped`; it is not Loop's existing close-reason
registry. Attempt ordinals are one-based; no admitted ordinal is invented when
Closing cancels an operation before its first admission. That Close uses 0,
allowed only for an opened, never-admitted operation. A final Close cannot use 0.

`operation_pin` retains the original envelope reference, prepared bytes,
resolved operation settings, optional partner key and original opening time
needed by the CallOpen carrier. Its causal projection seals the reference,
prepared digest, canonical resolved setting values and key, not duplicate
request bytes or a wall clock. The opening time is retention-only, supplied
as an explicit logical input. Original owner/instance come from the ordinary
transaction header; enclosing Loop contexts remain in the existing r2 context.
The original plan binding supplies policy and guarantee, not current source.

The producer of an ordered batch is resolved through its Call and emission
references. There is no stored journal array index, independent BatchId,
per-attempt idempotence boolean or extra random correlation id. Held/Published
lists must equal that Connector transaction's entire ordered emissions,
including its unique terminal. Manager Consumed must match one pending Held.
Consumed+Advance and Published+Close are atomic pairs; Consumed+Close(stopped)
is the approved cancellation of a pending held batch.

Final routes use the existing route/cursor fields. A held batch's explicit
manager destination is the Held fact, not an authored edge and not empty
routes alone. No `PhysicalEdge`, port, `nothing` alias or downstream cursor is
created for that disposition. Unknown fields/tags, illegal carriers and
contradictory context/facts refuse independently of the runtime scheduler.

Manager transactions use a new internal lifecycle **`l:retry`**, with future
activation tag **11** where the closed lifecycle codec needs it. It is not a
DSL `on retry` handler. The debugger labels Opened/Admitted/Advance/Close from
the packet, not as `l:next`. Existing Loop trigger bytes do not move.

## 5. References and timing

| Identity | Derivation / preserved rule |
|---|---|
| RetryOperationRef, new domain 0x16 | canonical framing of instance address, generation, entry and exact opening DeliveryRef |
| subsequent request admission, new domain 0x07 | operation ref, next one-based ordinal and previous winning CallRef |
| first admission | original request envelope and delivery, not a re-emission |
| subsequent envelope | new admission id, retained DATA/travelling metadata and origin; previous terminal envelope is its parent |
| CallRef | unchanged 0x10 recipe over request delivery and prepared digest; distinct admissions make distinct Calls |
| emission and DeliveryRef | unchanged existing recipes; no attempt suffix grafted onto a partner key |
| Loop refs | 0x14 run, 0x15 command, 0x06 admission unchanged; r1 domains remain historical |

The new temporal request cursor has an internal Retry namespace, derived from
operation and entry; it is not a serialized authored graph edge. No ambiguity
with existing `@loop` or physical cursors is allowed. C2 must pin the canonical
framing/endianness and independently constructed reference vectors before any
golden is blessed. No counters or Worker-slot numbers enter these recipes.

The C2 algebra fixes that framing as existing `Id::derive` framing: domain
byte, then each component prefixed by its byte length as little-endian u32.
Operation components are the canonical instance-address bytes, generation
u64 little-endian, entry UTF-8 and opening DeliveryRef bytes. Admission
components are operation bytes, next ordinal u64 little-endian and the
previous CallRef bytes. The internal cursor text is
`@retry:<operation-id>:<entry>`; the following DeliveryRef uses the ordinary
delivery recipe with that cursor and the new admission envelope id. Tests
construct the operation framing independently. None of these candidate
fields is admitted into current v18/v2 carriers; the single cutover remains
outstanding. The independently framed `pay` singleton-generation-0 fixture
pins operation `d6c4216ebd85518c3497d7410c2707eb` and its second admission
`e9b37274b55fddbac02eee26f3fd4849` in `journal/retry_protocol/tests.rs`;
the fixture specifies the exact opening delivery and predecessor Call bytes.

**Timing movements are part of this same cutover:**

- New retained-only **key 59**: the original logical closure instant (signed
  nanoseconds), mandatory on every winning outbound Call closure (response,
  timeout or abandonment), whether or not the Connector has Retry. It is
  absent on an open Call; a late discarded observation cannot replace it.
  It is not the opening `CallOpen.logical_time`. Inbound Calls and Sessions do
  not use this OUTBOUND-Call retention rule.
- Extend **key 37** to accepting Retry OUTBOUND closures: the causal presence
  and reference of the original retry arm. Existing timer meaning/order stays.
- Extend retained-only **key 56** to those closures: `armed_at = closed_at`,
  exact `due`, kind 5, reference of the potential next admission. The reference
  is derivable before the terminal envelope is emitted, from the operation,
  next ordinal and winning CallRef. An actual admission still waits for receipt.
- Advance references that original arm; it does not arm another delay. Missing
  delay means no arm, not an invented zero-duration timer. Final outcomes and
  Closing do not arm a retry. Previously retained arms remain visible but inert
  after cancellation.

As for existing scheduler retention, due/closure instants are excluded from
the causal projection but preserved by the recording integrity checksum and
validated against each other, the policy and prefix. Logical-time differences
may still change which events win, and therefore change the causal trace.
This is not a generalized commit clock or an OTLP feature.
The first C1 proposal made key 59 Retry-only to limit the initial retention
surface, not because a measured heap cost prohibited uniformity. The reviewed
target is uniform. Actual Rust layout/allocator and codec deltas must be
measured: a signed i64's eight bytes are not a complete peak-memory estimate.

## 6. Recording and compatibility

Diagnostic format v3 embeds the v19 plan and retains the complete new facts,
pins, exact times and schedules. Remove the effect member from its CallOpen
tuple; do **not** insert an idempotence member in the vacated position. The
new version owns the changed tuple arity. Extend Call/transaction retention
with Retry packets and the closure schedule/time, using the same fact codec
as journal reconstruction, not a separate replay interpretation.
Non-Retry Call recordings also carry the mandatory closure time, but still
omit Retry packets and retry timer arms. That changes retained v3 bytes, not
their causal seals, and must be exercised on echo and JSONL.

The existing whole-recording checksum domain remains, with the new version
inside its protected bytes. Old v2 recordings and old plan versions refuse by
version; no current-source fallback, transparent upgrade or M2 durable-store
compatibility claim. `replay` still starts no effect and executes no new
eligible admission. An incomplete original plan or forged Contract binding
refuses even if the Call carried an old idempotence word.

`CallOpen.effect` never was a causal field on ordinary OUTBOUND maps. Its removal
therefore does not by itself justify changing their seals. The retained
recording tuple *does* change. Prepared bytes, request key and Call identity
remain separately checked; don't confuse Rust layout, retained codec, semantic
hash and causal projection.

## 7. Witnesses and expected movements

The current pins are those in
[R9's blessed ledger](rework-r9-qualification.md#1-identity-inventory-before-blessing)
and the existing fixture tests; C1 changes none of them:

| Witness | Baseline semantic hash | Required cutover behavior |
|---|---|---|
| gauge | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | pure body/hash/trace unchanged; package version/checksum changes |
| echo | `5ffba3ea18c52a98a538a63ed0a7bf23cd3b4be0fa1b7d7acc51ba85dd73cfa3` | Process Contract/default changes move body/hash; v3 retains closure time; ordinary causal seals remain exact, not automatically re-blessed |
| JSONL | `2f89c8b5fa22f7fbe489d6aa770d517c3c9db8f92e850028c1e2bb1280efb2fc` | same Contract/hash move; framing/request bytes and ordinary causal trace remain witnesses |
| pure Loop without a Connector | existing specific fixture pin | LoopSpec/hash/r2 facts unchanged; only package container moves |
| non-Retry network/Session fixtures | existing per-Contract pins | hashes move for Contract/default changes; opening, close-once and orphan-write behavior must remain |
| HTTP 5xx, even without Retry | Completed(status) today | semantic type and affected terminal/causal records move to ServerError with the explicit reason |
| Retry examples | none executable today | new manager facts, held dispositions, attempt ids and deadlines; no claim of old Loop-trace identity |

The compiler's M0 probe is **not** the echo example: its current hash is
`d8c35016ba0d9a1c8a6b57e1abd83192840a5e9c11c63abd7efc977e365f126f`.
Keep its migration separate. C2/C4 must enumerate all changed hashes, actual
body sizes/checksums and causal fixture reasons before the one blessing.

The [explicit-unit annotation migration](annotation-types-m1.md#16-explicit-unit-migration-inventory--17-september-before-implementation)
retires the old implicit-registry annotation tag 3 by name, without reusing it.
Numerical quantities use the already allocated tag-6 measure binding. Only
formerly implicit-registry metadata/hash bytes move; explicit-unit bindings,
compact monetary/security bytes and original causal records did not move in
that dated slice. D-2026-09-18-1 subsequently retires compact monetary tag 4
and replaces `rastok/measure@2` with the two-field `rastok/measure@3` binding;
[the pre-code inventory](annotation-types-m1.md#19-two-field-measure-migration-inventory--18-september-before-code)
is the additional annotation movement in the single cutover. The
[C4 ledger](retry-c4-qualification.md) gives the measured current witness
bodies, hashes, checksums and causal non-movements.

Non-movements: envelope/DataType/value tags, expression ids including `has`,
redaction policy, ordinary Dispatch, source/destination sets,
RouteRule/PhysicalEdge separation, owner/instance/address tables, `behavior`,
Call close-once, state deltas, ordinary cursor ordering, instance-stop key 57,
Loop r2 facts and no-recall inspection. Internal sharing alone moves no bytes.

## 8. C2 gate checklist

1. Audit these fresh slots against the registry again immediately before use;
   any new allocation since this checkpoint requires updating this inventory.
2. Carry the entire package/recording/timing algebra in the one cutover; retain
   named execution refusals until the runtime is complete.
3. Independent hostile validation covers Contract opening, guarantee constancy,
   Retry host/cases/attempts/cardinality, exact producer/consumer pairing,
   references, original binding, schedules, capacity and generation fencing.
   All winning outbound Call closures require key 59, including non-Retry
   responses, timeouts and abandonments; malformed/missing time refuses, an
   open Call cannot masquerade as closed, and causal seals exclude the field.
4. Prove canonical Extension and case order; prove semantically changed policy
   inputs change identity. No speculative Saga tag, counter or public trait.
5. Measure embedded heap and code size **before** growing retained structures;
   common admission/receipt caps must not become unbounded or duplicated per
   Extension. At most one unconsumed batch exists per Retry operation, but the
   journal retains all intermediate batches plus the final attempt. Measure
   both, with decoded stdout/stderr and metadata/reference/recording overhead;
   `max_output` bounds aggregate raw capture, not total retained heap. Prove
   Consumed frees pending capacity without deleting historical evidence.
   Do not weaken the existing gates to make the design pass.
6. Do not migrate a historical trace by ignoring failures. Each affected
   witness gets a reason; pure and ordinary nonparticipating witnesses stay
   exact in the registers that do not change.
7. Implement and register advisory `W-LOOP-REPEAT` with its tests before
   closing C2: Again with a possibly visible non-idempotent Call, including a
   successful sibling, warns; a proven entirely BeforeEffect path or an
   idempotent Call does not. Omitted guarantee equals false. Include no
   promotion of Next to Retry, stable canonical warning order and no new
   plan/hash/causal bytes caused by a tool warning. Unsafe Retry remains an
   independent error. The name stays prospective until these tests land.
