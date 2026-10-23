   # Vocabulary ledger — the named entities of the code, validated

> Drafted 11 October 2026 by Fable; every line validated by Yoann on 11 and
> 12 October. Status: **closed — every ⟂ is settled**. Decisions are
> recorded in place, and the rename lot they produce is the
> [Outcome](#outcome--the-rename-lot-closed-12-october) section. Yoann
> decided on 12 October that the lot is applied **with the realignment of
> [D-2026-10-11-1](../spec/decisions.md), before the opening**, together
> with A-T4-4; the T4 second pass is suspended after its block 1 until then.

**Follow-up status — 21 October.** The closure above describes the
12 October rename lot. T5 has since reopened the Connector organization
and its vocabulary. The [pre-freeze Connector rework](#connector-rework--21-october-before-the-freeze)
below records Yoann's requested direction and Astra's complete proposal
for Fable's review; it is not an implementation report.

## Why and how

Yoann's remark (11 October): many terms overlap because they were coined
session after session; an external reviewer will ask "another new term —
what is this one?". The specification already has the right discipline in
one place, simulation-semantics §1 *"Vocabulary — one word per layer"*. This
ledger extends that discipline to the whole code.

Method. The public types were extracted mechanically from the two crates on
`fedada7`: **380** in `rastok-core` (204 structs, 161 enums, 14 traits, one
alias) and **238** in `rastok` (152 structs, 81 enums, 3 traits, two
aliases); the complete list is the appendix. Each family below names the
competing words, quotes the type's own first doc line as its meaning, and
gives the word's frequency in `docs/spec/*.md` as evidence of which word the
specification already uses. Six rules were confirmed during the validation; rules 5 and 6 were added
on the way, each after a case that needed it:

1. **One word per concept.** When the specification has a word, the code
   takes it. A word that exists only in the code names a mechanism, not a
   business concept, and is defined once, where its type lives.
2. **One concept per word.** A word already owned by the specification's
   taxonomy (Node, Graph, Executor, Operator, Connector, Extension,
   Contract, Dispatch, Driver) is never reused for a mechanism.
3. **Nestings are not synonyms.** Three names for three wrappings of one
   object are a smell unless each wrapping adds a fact a reader needs.
4. **Plain over imagery** (Yoann, 11 October). A name must unfold into a
   sentence without its doc comment. An image is kept only where the
   specification adopted it and the public reads it: `Envelope`, `mailbox`,
   `budget`, `deadline`, `drain`, `worker`, `sealed` / `unsealed` /
   `retained`, `trace`.
5. **A state is a type, an alternative is an enum** (Yoann, 11 October,
   night). Successive states of one thing are distinct types, and the step
   to the next state consumes the value: a prepared ingress cannot be
   committed twice, an uncommitted one cannot be routed, because the
   compiler refuses it. Alternatives true at one instant are one enum. A
   `status` field with `Option` fields for the data of other states is
   refused: it makes a false state representable. Renaming removes the
   names that duplicate a state without adding one.
6. **A collision is settled with an existing word first** (12 October,
   after `Arrival`). A new word is the last resort, when neither the
   specification nor a validated suffix (`*View`, `*Spec`, `*Kind`) covers
   the thing; the ledger exists to end up with fewer words, not different
   ones.

## Collisions to settle first

Ranked by the cost of leaving them to the external reviewer.

1. **Executor.** The specification's taxonomy word for Operator | Connector
   (extensions.md §1). The code reuses it for two mechanisms:
   `runtime::HandlerExecutor` ("prepares a handler transaction in scratch
   isolation") and `runtime::GuestExecutor` ("pure, deterministic, budgeted
   guest execution — the contract the jq and SQL engines will implement").
   **⟂** rename both mechanisms away from the taxonomy word (interpreter,
   evaluator, engine, …). **Proposed 12 October:** `HandlerExecutor` →
   **`TransactionPreparer`**. It is a function object bound to a plan, a
   graph version and the logical time, `fire(&self, slot, instance,
   delivery, state) -> TransactionOutcome`, and it prepares every
   transaction, Connector completions included, so "Handler" was already too
   narrow; "prepare" is the journal's own word (`prepare_ingress` /
   `commit_ingress`). `TransactionBuilder` (Yoann's question) not retained:
   a Rust reader expects the builder pattern, setters then `build()`, which
   this is not, and "builder" already means the envelope builder in the
   same file (six uses). `GuestExecutor` → **`GuestRunner`**: the
   specification says "guest language" and "guest program", the trait's one
   method is `run`, and the implementations run guest programs. Not
   `GuestEngine` (Yoann: no), not `GuestLanguage` (names the language, not
   what runs it), not `GuestRuntime` ("runtime" is the project's word for
   the whole execution layer).
2. **Dispatch.** In the specification and the plan, Dispatch is the
   Operator's typed decision tree (`plan::Dispatch`, `DispatchStep`; spec 82
   uses). At the boundary, `boundary::Dispatcher` is "the loop between the
   journal and the outbound drivers" (spec "dispatcher" 11 uses), with
   `DispatchError`, `DispatchSelection` (runtime, the handler match) on both
   sides of the word. **⟂** keep Dispatch for the Operator tree; give the
   boundary loop its own word. **Settled 12 October (Yoann):**
   `boundary::Dispatcher` → **`DriverPoller`**, consistent with the decided
   verb `poll_drivers`; `Dispatch`, `DispatchStep`, `DispatchSelection` stay,
   the specification's sense. Not "loop", the Extension's taxonomy word.
3. **Bindings.** Four unrelated meanings: the verified Driver routing table
   (`OutboundConnectorBindings`, `SessionConnectorBindings`, host
   `RuntimeBindings`), the deployment boot inputs (`BootBindings`), the
   expression evaluation environment (`expr::Bindings`, `LazyBindings`) and
   the per-request Driver settings (`HttpClientRequestBindings`,
   `FileRequestBindings`, `ProcessRequestBindings`). **⟂** one word each.
   **12 October:** the specification itself has two senses, name bindings in
   expressions (the main one) and "the Driver binding" of a Connector at
   deployment. `expr::Bindings`, `LazyBindings` stay (main sense);
   `OutboundConnectorBindings`, `SessionConnectorBindings`, host
   `RuntimeBindings` stay (the bound Drivers, one doc sentence says so);
   `BootBindings` **kept for now** (Yoann): the resolved `env()` / `secret()`
   values are names bound at boot, a binding in the main sense. Proposed:
   `*RequestBindings` → **`*RequestSettings`**, per-request Driver settings
   are not bindings in either sense.
4. **Emission.** Two types of that name: `journal::Emission` ("one emission
   of a transaction, in text order") and `plan::emission::Emission` ("what an
   emission builds"). Same word, two layers, both public. **⟂** the plan
   side is a builder or a template, not an emission. **Settled 12 October:
   `journal::Emission` keeps the specification's word; the plan side becomes
   `EmitValue`**, the word of its neighbours in the handler body (`Emit {
   port, value }`, `MultiEmitValues`, `FieldWrite.value`). `EmissionSpec`,
   recorded the night before, was withdrawn once the neighbours were read:
   `*Spec` names the header configurations, not a statement's right-hand
   side. Details under "Validated so far".
5. **Profile.** Six meanings: `deployment::DeploymentProfile`,
   `PrivacyProfile`, `accounting::CostProfile`, `conformance::Profile`
   (Call/Event/Session provocation profiles), `contract::Profile` (Contract
   profiles with `ProfileSelector`, `ProfileProtocol`), and
   `json::HydrationProfile`. The specification uses "profile" for the
   deployment profile and the Contract profile. **⟂** keep those two; rename
   the other four. **Proposed 12 October:** `PrivacyProfile` →
   **`PrivacyDeclaration`** (controller-supplied, verified for completeness
   and digest only; not `PrivacyFacts`, Yoann: "fact" already has its
   sense); `CostProfile` → **`PriceBook`** (Yoann, 12 October; its own doc's word;
   `PriceTable` offered as the plainer word and not retained),
   `CostProfileName` → `PriceBookName`;
   `json::HydrationProfile` → **`DeserializationPolicy`** in module `json`
   (Yoann, 12 October: `JsonDeserializationPolicy`; written qualified,
   `json::DeserializationPolicy`, the module already says Json, as
   `instance::Generation`; Yoann picks the spelling). "Hydration" is ORM
   imagery; the universal technical word for turning JSON into a typed
   struct is deserialization, which every C#, Java or Rust reader knows.
   Fable first proposed `MaterializationPolicy` and withdrew it: materialize
   is the specification's word for a payload's *state* change, `Encoded {
   bytes }` → `Materialized(Value)`, lazy, for every encoding; deserialize is
   the codec's *procedure* that produces the typed value from one format
   under the schema, the means by which a JSON payload materializes. Two
   concepts, two words. `hydrate` → `deserialize`, `hydrate_with_profile` →
   `deserialize_with_policy`, module `json/hydration.rs` →
   `json/deserialization.rs`; the specification's 31 "hydration" (12 in
   decisions.md) align on "deserialization" in the realignment's doc pass.
   `materialize` does not move;
   `conformance::Profile` read 12 October: the contract under test, a
   happy-path probe with its submission and exit case, one provocation per
   provocable terminal case, the unprovable cases with their reasons, the
   mute probe for the timeout battery, the interleaved-correlation probe
   (examples: File reads `missing.bin` → `NotFound`, a directory →
   `NotAFile`, `../outside` → `InvalidPath`, `large.bin` under a 4-byte
   limit → `ReadLimitExceeded`; unprovable `PermissionDenied`, "no portable
   fixture can force host permission denial"). Yoann did not know the
   word "provocation", which settles it: the module takes the universal
   test words. **Settled 12 October (Yoann): `Profile` →
   `DriverTestFixture`**, `EventProfile` / `SessionProfile` →
   `EventTestFixture` / `SessionTestFixture`: the fixed material one Driver
   brings to the shared battery, in the xUnit sense; the battery itself is
   the suite, one for all Drivers, so `TestSuite` (also Yoann's question)
   was not retained. The specification's simulation vocabulary already uses
   "fixture" for a scenario's scripted world-half; same sense, prepared
   material a runner consumes, said once: fixture = prepared test material,
   a scenario's in the Lab, a Driver's in conformance. `Provocation` →
   **`ExitCaseTest`**, an input built to force one declared exit case;
   `unprovable` → **`untestable`**, the declared cases no portable test can
   force, each with its written reason. `DriverProbes` and
   `DriverConformanceCases` withdrawn.
6. **Node.** `plan::Node` is an executable row only, narrower than the
   specification's Node, which includes Graph. Settled by D-2026-10-11-1;
   listed here for completeness.
7. **Spec.** `instance::Spec` ("one owner's logical instance policy") is a
   bare generic noun; the other `*Spec` types (`ConnectorSpec`, `LoopSpec`,
   `RetrySpec`, `ExtensionSpec`) carry their subject. **⟂** `InstanceSpec`,
   or the specification's word, policy. **Proposed 12 October:**
   **`InstanceSpec`**: `instance.key`, `instance.ttl` are header
   declarations, exactly what the plan's `*Spec` names.
8. **Event.** An exchange cardinality (`ConnectorExchange::Event`), four
   observation carriers (`BoundaryEvent`, `InboundCallEvent`,
   `SessionDriverEvent`, `LifecycleEvent`), and a syntax trigger
   (`syntax::ExtensionEvent`). **Settled 11 October (Yoann): four words
   are kept, one each.** *event* = the exchange kind a Contract declares
   (`ConnectorExchange::Event`; Timer and Calendar in the stdlib);
   *observation* = what a Driver reports before any transaction commits it
   (the four `*Event` carriers become `*Observation`); *fact* = a committed
   record; *command* = what a protocol asks of the next transaction. The
   table under "Validated so far" carries the definitions.
   `syntax::ExtensionEvent` is a recovery-only capture of the removed
   `.start`/`.stop` forms in the host parser, a third use of the word;
   **⟂** rename it with the recovery it serves (`RemovedLifecycleHook`) or
   leave it, Yoann to confirm.

## Families

Each table: the word as the code spells it, where it lives, its own first
doc line, and the specification's count for the bare word (across
`docs/spec/*.md`, case-insensitive, whole word).

### 1. The world enters — ingress

| Code | Where | Meaning (own doc line) | Spec word (uses) |
|---|---|---|---|
| `Draft` | boundary | the payload and birth metadata of a submission; the runtime mints `meta.sys` — a draft never carries one | draft 26 |
| `IngressSubmission` | boundary | a draft proposed for atomic acceptance: inbound Drivers propose drafts, never envelopes | submission **2** |
| `SubmissionSequence` | boundary | how a submission's sequence number is chosen | — |
| `IngressProposal` | boundary | one bounded world fact offered without pre-minting an envelope (= submission + session framing + Driver handle) | proposal 28 |
| `IngressProposalHandle` | boundary | Driver-local identity of one bounded inbound proposal | — |
| `InboundSession` | boundary | Session meaning of one inbound proposal before the accepting commit | — |
| `IngressEnvelopeProvenance` | boundary | provenance shared by root envelopes entering through one logical Connector replica | — |
| `IngressAdmission` | boundary | result of attempting one proposal against bounded runtime admission (Accepted / Pending / Rejected) | admission 186 |
| `IngressAcceptance` | boundary | positive result returned only after the accepting journal commit | acceptance 22 |
| `IngressRejection` | boundary | portable contract-level reasons why a proposed fact was not accepted | — |
| `IngressDecision` | boundary | a final decision delivered to the Driver for its retained proposal | — |
| `PreparedIngress`, `CommittedIngress`, `IngressRecord` | journal | the minted-but-uncommitted root, the committed root, the INGRESS record | ingress 62 |
| `inject`, `admit_ingress`, `inject_submission` | vm | the host's door, the Driver's door, the one transaction behind both | injection (Lab) / admission |

**⟂ 1a.** Draft ⊂ Submission ⊂ Proposal are three wrappings of one offered
fact. The specification says "draft" and "proposal", and "submission" twice
in total. **Settled 11 October, night:** `EnvelopeDraft` inside one flat
`IngressProposal`; `RetainedProposal` for the Driver's waiting copy. See
"Validated so far".
**⟂ 1b.** Admission / Acceptance / Decision / Rejection: four result nouns
for one verdict. **Settled:** admission (verdict) and rejection (reason)
stay; acceptance → `AdmittedIngress`; decision → `Result`.
**⟂ 1c.** `inject` is the Lab's and host's word (simulation-semantics:
"injections"), `admit` the Driver's. **Settled:** the pair stays, the
host door has neither handle nor `Pending`, stated once in the `Vm` docs;
the shared transaction is `admit_proposal`.

### 2. Messages — envelope, delivery, emission, output, observation

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Envelope` | envelope | immutable emitted message: `meta` written on the outside, `data` sealed inside | envelope 396 |
| `Delivery` | envelope | one delivery waiting in a mailbox (port, edge, seq, source instance, envelope) | delivery 168 |
| `RoutedDelivery` | journal | one routed delivery whose target instance was selected atomically with the transaction that emitted it | — |
| `Emission` | journal | one emission of a transaction, in text order | emission 95 |
| `Emission` | plan/emission | what an emission builds; field writes retain their authored evaluation order | — |
| `EmissionBuffer`, `Built`, `BuiltData` | runtime | ordered per-port emissions of one transaction; what one emission carries out of its builder; a builder's payload verdict | — |
| `BoundaryOutput` | vm | one envelope that left the graph through its boundary, with the port it left by | output 224 |
| `PortOutput` | boundary | one ordinary output proposed by an external driver | — |
| `Observed` | vm | one observed delivery — the debugger-shaped observation surface | — |
| `RecordObservation`, `RecordObservationKind`, `RecordCorrelation`, `ExchangeCorrelation` | observation | one retained, non-causal projection of a causal record | retained 169 |
| `AccountingObservation`, `ObservationScope` | accounting | measurement observations | — |
| `BoundaryEvent` | boundary | doc: "one routable boundary fact for the VM"; actually a settled outbound call's completion awaiting its transaction, an observation → `BoundaryObservation` (11 October) | fact 110 |

**⟂ 2a.** Collision 4 (two `Emission`). **Settled 12 October:**
`journal::Emission { port, index, envelope }` is exactly the specification's
emission and stays; `plan::emission::Emission` (`Identity | Patch |
FreshData | Data | Nothing`, held as `value` on a Dispatch leaf) →
**`EmitValue`**. Examined and not retained: `EmissionSpec` (header
configurations, not the handler body); `EmissionKind` (a `*Kind` is a
category tag without content); `journal::EmittedEnvelope` (loses the
specification's word, and fails the last-word test: it is an envelope with
its emission coordinates, not an envelope); `EmissionForm` (Yoann: no);
`EmissionRule` (`RouteRule` is a checker decision, not an authored
declaration); `EmissionRecipe` ("recipe" is the identity recipes' word).
**Settled 12 October:** `PortOutput` merges into `EnvelopeDraft { port,
data, ext }` (see "Validated so far"). Settled the same day (Yoann):
crate-private `Built`, `BuiltData` → `BuiltEmission`, `BuiltPayload` (the
chain then reads `EmitValue` in the plan, `BuiltEmission` in the
transaction's scratch, `Emission` in the journal); `RoutedDelivery` (journal
fact) and `Delivery` (mailbox entry) keep their names, their two doc lines
state the layer. `EmissionBuffer`, `BoundaryOutput`, `ConsumedPosition`
stay.
**⟂ 2b.** Observed / RecordObservation / AccountingObservation /
`observations()`: "observation" is used for a debugger surface, a journal
projection and a measurement. The specification uses "retained
observation" for the second. Sharpened by collision 8 (observation = a
Driver's report before commit). **Settled 12 October (Yoann):** `Observed { node, port, envelope }`
→ **`DeliveryView`**: a delivery as the debugger sees it (node, port,
envelope; no edge, no cursor), read by the Lab's expectations, `main` and
the UI view; the `*View` suffix is already validated, so no new word.
`Arrival`, proposed and withdrawn the same night (Yoann: "why not
Departure too?"): a word the specification does not have, invented to fix
a collision. Not `trace`: in the specification that is a Scenario's outer
trace, what goes in, not what comes out. `AccountingObservation`,
`ObservationScope` → **`Measurement`**, **`MeasurementScope`** (their own
doc says "retained measurement");
`RecordObservation` stays: the specification's "retained observation" has
the same sense, what was seen without carrying identity.
**Corrected 12 October after Astra's review:** `Observed` → `EnvelopeView`,
not `DeliveryView`: the log also records each emission at its source port.
**⟂ 2d.** `Delivery { port, edge: String, seq, source_instance, envelope }`
carries two derivable fields: `port` follows from the edge, and `edge` is
canonical text where it is a wiring index, text that also enters
`delivery_id` and the budget key. Yoann's remark (11 October, "a sequence
should be an index"); for realignment lot 1 (D-2026-10-11-1): `edge` as an
index, `port` derived. `index` (per port, per transaction, identity) and
`seq` (per edge × source instance, the cursor) count two different things
and both stay.
**⟂ 2c.** "fact" is the specification's and the doc comments' word for a
committed record (110 uses) but names no type; `Record` does. Confirm that
"fact" stays prose and `Record` stays the type. **Settled 11 October:** fact
is kept as the word for a committed record (collision 8). Whether the record
types take the word (`Record` vs `Fact`) was not discussed.

### 3. The world answers — call, completion, closure, settlement, terminal

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Intent`, `SessionOpenIntent`, `SessionWriteIntent` | boundary | journaled open before the effect executes | intent 35 |
| `PreparedRequest`, `RequestPreparation`, `RequestCodec` | boundary | the sealed request descriptor; the result of framing one invocation before its EGRESS record opens | request 203 |
| `Response` | boundary | protocol-correlated driver answer; a driver answers or stays silent — never a timeout | response 146 |
| `Completion`, `CompletionGate`, `CompletionError`, `SchedulerCompletion` | boundary | one admitted invocation's ordered boundary completion; the gate that judges it; a terminal fact decided by the machine | completion 81 |
| `Closure`, `CloseVerdict`, `CallStatus` | journal | what closed a call — the driver's completion or the timeout, exactly one; who won the race | closure 122 |
| `*Exit` (`ProcessExit`, `FileExit`, `HttpClientExit`, `HttpServerExit`, `WebSocketExit`) | boundary | portable terminal facts of one Connector family | terminal 253 |
| `InboundCallEvent`, `InboundCallResponse`, `InboundCallTimeout`, `*Start` | boundary | the inbound-Call dialogue with its Driver | — |
| `settled`, `SettledEntry`, `settle_next`, `settlement` | vm | won closures awaiting their completion transaction; the Closing stage after drain | settlement 35 |

**⟂ 3a.** Five words around "the call ended": completion (the Driver's
answer), closure (what closed it, answer or timeout), terminal (the fact
kind), exit (the Contract's port and typed fact), settlement (a Closing
stage, and the `settled` queue). The specification uses all five with
distinct meanings; the question is whether the code keeps them distinct —
`settled` holds closures that are not yet committed, which is neither
settlement nor completion.
**⟂ 3b.** "intent" (journal-before-effect) is the code's word for the open
fact of an outbound effect; the specification says "open fact" more often.
Confirm "intent".

**Settled 11 October:** the `Intent` types take the effect's name
(`CallOpen`, `SessionOpen`, `SessionWrite`); "open fact" stays the
specification's prose; journal-before-effect is stated once, on the
`DriverPoller`.
### 4. Identity — id, identity, reference, address, generation, slot, path

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Id`, `IdKind` | envelope | deterministically derived 128-bit identity; the closed domains of the journal's identity recipes | identity 362 |
| `Identity` | instance | canonical logical address of one owner instance | address 102 |
| `Generation` | instance | complete reference to one live generation of a logical address | generation 162, reference 152 |
| `Segment` (`Live::frames`) | instance | one dynamic component of a logical instance address | segment 25, frame 50 |
| `CallId`, `InboundCallId`, `SessionId`, `SessionWriteId`, `LoopRunId`, `OpeningAdmissionId`, `CommandId`, `ContractId`, `MeasureId` | various | placement-neutral identities of one exchange, run, command, contract, measure | — |
| `IngressProposalHandle` | boundary | Driver-local identity of one pending proposal | — |
| `Slot`, `OwnerSlot` | plan, node | compile-time resolved node / owner reference, valid only inside a validated plan | — |
| `Path`, `OwnerPath` | node | stable root-relative node address; root-relative lexical owner path | — |
| `SemanticHash`, `GraphVersion`, `Manifest` | plan | `blake3(canonical HIR)`; its sequential flyweight; the (version ↔ hash) ledger | identity 362 |

**⟂ 4a.** `Identity` is an instance *address*; `Id` is an envelope
*identity*. Two readers will swap them. The doc lines already say "address"
and "identity": the types could.
**Settled 12 October (Yoann):** `Identity` → **`InstanceAddress`**, its own
doc's words and the specification's (address 102: "admission of a delivery
into an absent address starts a new instance"); `Id` stays.
**⟂ 4b.** `Generation` is called "instance reference" in prose and
`instance` in most field names. One of the three.
**Settled 12 October:** `Generation` stays, always written
`instance::Generation` and never imported bare (zero bare imports, 332
qualified uses today); the prose says "generation", not "reference". Not
`InstanceGeneration` (Yoann's question): the module already qualifies it,
`instance::InstanceGeneration` would stutter.
**⟂ 4c.** `frames: Vec<Segment>` — frame and segment for one thing.
**Settled 12 October (Yoann):** `frames` → **`segments`**; "frame" is the
word of Process frames.
**⟂ 4d.** `Slot`/`OwnerSlot` disappear with D-2026-10-11-1 (one typed
index); confirm the surviving word (index, not slot, not handle).

**Settled 12 October (Yoann):** the surviving word is **index**,
`NodeIndex`, with the realignment lot.
### 5. Instances — owner, scope, bundle, lifetime, closing

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Owner`, `OwnerKind` | plan | one lexical lifecycle and instance owner; the declaration represented by one owner row | owner 221 |
| `instance::Spec` | instance | one owner's logical instance policy | instance 564 |
| `Lifetime` | instance | closed transport vocabulary for lifetime policies | lifetime 123 |
| `Closing`, `ClosingStage`, `Phase` | instance | the one bounded Closing protocol; its stages; admission and closure as exclusive states | closing 177 |
| `Live` | instance | one selected owner generation and its immutable instance bundle | bundle 8 |
| `InstanceAdmissions`, `AdmissionPrefix` | journal/admission | the admission view shared by live routing and prefix recovery | admission 186 |
| "scope" | everywhere | the closing scope (`close_scope`), the owner scope (`Segment::scope`), the edge scope (`Edge::scope`) | scope 90 |

**⟂ 5a.** Collision 7 (`Spec`).
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `InstanceSpec`.
**⟂ 5b.** "bundle" (8 uses) names the immutable values of a `Live`; `Live`
itself has no specification word. Confirm "live instance" / "bundle" or
pick one.
**Settled 12 October (Yoann):** `Live` → **`LiveInstance`**; the name of
its immutable values ("bundle", 8 uses) is read before it is chosen.
Read 12 October: `Live` is crate-private, `{ owner, parent, frames,
values, settings, created_by, last_activity, residual, phase }`; "bundle" is
only its doc's word for the immutable `values` and `settings`, no field
carries it. The doc says "its immutable values and settings"; nothing else
to name.
**⟂ 5c.** "scope" carries three meanings; the specification mostly means the
closing or containing scope. Confirm or qualify (`Edge::scope` is an owner).

**Settled 12 October (Yoann):** "scope" keeps the specification's sense,
the closing or containing scope; `Edge::scope` → **`owner`**.
### 6. The plan — nodes, wiring, routes, handlers

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `ExecutionPlan`, `ValidatedExecutionPlan` | plan | the compiled graph, in the database sense; the plan whose structural invariants have been verified | plan — |
| `Node`, `NodeExecution`, `Owner` | plan | one planned node (executable only); its executable payload; the lexical owner | node, owner |
| `Handler`, `HandlerActivation`, `Trigger` | plan, node | one Operator transition; what activates it (Port / Join / Lifecycle); what fired it | handler 123, trigger 57, activation 22 |
| `Dispatch`, `ClosedArm`, `OpenArm`, `DispatchStep`, `LoopCommand` | plan/dispatch | the Operator's typed decision tree and its checked statements | dispatch 82 |
| `Edge`, `EdgeEnd` | plan | one planned edge; one resolved end — a boundary end has no node | edge 107 |
| `Endpoint`, `EndpointOwner` | plan/routing | one end of an edge, default ports made explicit; what owns it | endpoint 18 |
| `RouteRule`, `RouteSelection`, `RouteArm`, `RouteDestination` | plan/routing | one checked routing decision per source port; its static selection; one destination selected before the producer commits | route 94, destination 61 |
| `RouteTarget` | journal | one pre-commit target selection for a single inbound envelope | target 178 |
| `InstanceStopTarget`, `LoopCommandTarget` | plan/routing | static authority for an instance closure; an addressed manager command as a destination | — |
| `PortDecl`, `ResolvedPort`, `ResolvedPortTable`, `TypeOrigin` | plan | a port table entry with optional pin; the port after type resolution | port — |
| `VarCell`, `OwnerValue`, `PlanConstant`, `EvaluationSite`, `AttrSetting` | plan | one planned state cell; one immutable binding with its site; one plan-reduced constant; one effective attribute | — |
| `ExtensionSpec`, `LoopSpec`, `RetrySpec`, `ConnectorSpec`, `LoopExpiry`, `LoopDelay` | plan | the fully configured Extensions and Connector | extension — |

**⟂ 6a.** Edge / EdgeEnd / Endpoint / EndpointOwner / RouteDestination /
RouteTarget / target / destination: the wiring has one object, the end of
an arrow, under four type names and two prose nouns. D-2026-10-11-1 fixes
one `Endpoint` (index + port) and one `Destination` enum; confirm those
two words and retire the rest.
**Settled 12 October (Yoann):** **`Endpoint`** (index + port) and
**`Destination`** with the realignment lot; `EdgeEnd`, `EndpointOwner`,
`RouteDestination` go. `RouteTarget` stays: it is the selected *instance*,
not an endpoint, and "target" is the specification's word (178).
**⟂ 6b.** Activation / trigger / firing: `HandlerActivation` is the static
shape, `Trigger` what fired at run time, `LoopFiring` / `LifecycleFiring` /
`fire_*` the act. Three words, three meanings; confirm them, they recur in
the journal.
**Settled 12 October (Yoann):** the three words stay, three
specification words with three senses: activation (static shape), trigger
(what fired), firing (the act).
**⟂ 6c.** `LoopExpiry` is "a host-owned terminal", not a dated expiry;
collides with family 9.

**Settled 12 October (Yoann):** `LoopExpiry { port, ty }` →
**`LoopTermination`**: how the Loop terminates, on which existing port, with
which effective type. Not `LoopTerminal` (Fable's first proposal): the
specification reserves "terminal" for the Connectors' terminal facts, the
`*Exit` types. "Expiry" returns to the time family.
**Corrected 12 October after Astra's review:** `LoopExpiredOutput`; the type
only names the output for `LoopExpired`, `LoopTermination` overclaimed.
### 7. Execution — transaction, step, firing, fault

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `HandlerExecutor` | runtime | prepares a handler transaction in scratch isolation | executor (taxonomy) |
| `GuestExecutor` | runtime | the contract the jq and SQL engines will implement | — |
| `TransactionOutcome`, `TransactionRecord` | runtime, journal | the commit packet and nothing else; the atomic commit packet of one node firing | transaction 285 |
| `State`, `Mailboxes`, `Scheduler` | runtime, vm | state cells by complete instance reference; bounded per-(node, instance) FIFO queues; the run-to-completion scheduler | mailbox 97, worker 64 |
| `JoinCandidate`, `OperatorWork` | runtime, vm | one complete join candidate; what an Operator step will do (single, join, evict) | candidate 37 |
| `LoopFiring`, `LifecycleFiring` | runtime | exact temporal application selected before an Operator transaction; the causal identity of one lifecycle firing | firing 23 |
| `Fault`, `VmError`, `DispatchError`, `Error` (journal, expr, deadline, folds) | various | a runtime fault of one transaction; a machine fault; a dispatcher fault; module errors | — |
| `Vm`, `TimeAdvance`, `step`, `drain`, `pump`, `advance` | vm | the pure execution engine; the resumable discrete-time sweep; the four host verbs | step 33, advance 63, drain 42, pump 18 |

**⟂ 7a.** Collision 1 (`*Executor`).
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `TransactionPreparer`, `GuestRunner`.
**⟂ 7b.** `Vm::step` is one transaction; `TimeAdvance::step` is one
transaction, one expiry or one poll. Same verb, two granularities.
**Proposed 12 October:** `Vm::step` keeps the specification's `step`, one
transaction (33 uses); `TimeAdvance::step` → **`advance_once`**, one unit of
the `advance` it slices (a transaction, an expiry or a poll).
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `advance_once`.
**⟂ 7c.** Fault / VmError / Error: "fault" is the specification's word for
a transaction that committed nothing; `VmError` is also that. Confirm the
split (Fault = evaluation, VmError = machine) or merge the words.
**Proposed 12 October:** `Fault` stays (the specification's word, 34 uses;
`TransactionFault` if Yoann wants both qualified); `VmError` → **`VmFault`**:
its own doc says "a machine fault … never a business error", and "error" is
the specification's word for the `err` port and business error values (170
uses); with `DriverPoller`, `DispatchError` → **`PollFault`**. Module-local
`Error` types stay, the `io::Error` convention.
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `VmFault`, `PollFault`; `Fault` stays.
**⟂ 7d.** The spec's **Worker** has no type (ledger, parked remarks).

**Proposed 12 October:** a **`Worker`** type in the `rastok` crate, the
specification's word (64 uses), owning the hosted loop of `main.rs` and
`lab.rs` (boot, rounds, stepping, shutdown); created with the runtime
organisation lot of D-2026-10-11-1, not a rename.
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `Worker`, with the runtime organisation lot.
**Applied 15 October (`b7670ff`), reviewed by Fable.** `rastok::worker::Worker`
owns one `Execution`, its `MemJournal` and its `RuntimeBindings`; public
verbs `run`, `run_until_idle`, `step`, `shutdown`, plus `accept_input` and
`advance_to`; the Lab face is crate-private. `round` from the parked remark
became the private `process_ready`. Names the lot introduced without
passing here first: `worker::Host`, `worker::Error`, `worker::RunError`.
**⟂ 7e.** `worker::Host` (15 October). The trait names what the Worker
needs from its surroundings: logical time, non-blocking App input, a
checkpoint after committed work and the waiting policy; the CLI's
`CliHost` implements it over stdin, the monotonic clock, the recorder and
the 10 ms sleep. The specification uses **Host** for the machine of the
clustered model that runs Workers (delivery-semantics "Host loss",
instances "Worker, Host, lease, and leader epoch"). The reading "the
Worker's view of what hosts it" is consistent with that use, and the code
already leans on the word (`HostedTimeSource`, `hosted-ui`). Rule 2 is not
engaged, Host not being a taxonomy word; rule 1 asks that the
specification's word keep its meaning.
**Proposed 15 October:** keep `Host`, and let the trait doc say it is the
Worker's view of its Host, never a second machine model; the alternative is
a mechanism word (`worker::Environment`). To Yoann.
### 8. The boundary — driver, contract, binding, dispatcher, gate, pump

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `OutboundConnectorDriver`, `CallConnectorDriver`, `SessionConnectorDriver`, `InboundConnectorDriver`, `InboundCallConnectorDriver`, `Supervisor` | boundary | the world roles a bound Connector exposes | driver 208 |
| `ConnectorExchange`, `ConnectorRoles`, `SessionOpening`, `ContractId` | connector | the cardinality and terminal lifecycle of one interaction; the roles a contract requires; who may open | contract 421, exchange 50 |
| `OutboundConnectorBindings`, `SessionConnectorBindings`, `RuntimeBindings` (host) | boundary, registry | the verified routing tables consumed by the pump | binding 118 |
| `Dispatcher`, `PumpBatch`, `PumpReport`, `InboundCallPumpReport` | boundary, vm | the loop between the journal and the outbound drivers; one settled pump round; what a round observed besides causal progress | dispatcher 11, pump 18 |
| `CompletionGate`, `OpenGate`, `PlanGate` | boundary, vm | judges a completion against the resolved contract; admits everything; the plan-backed gate | gate 77 |
| `DriverRegistry`, `ConnectorDriverFactory`, `BoundConnectorDriver`, `DriverManifest` | registry (host) | the registry, the factories, the bound Driver, the manifest | — |
| `*Factory`, `*Config`, `*Binding`, `*RequestBindings` | connectors (host) | per-family construction, configuration, plan binding, request settings | — |

**⟂ 8a.** Collision 2 (`Dispatch`/`Dispatcher`) and 3 (`Bindings`).
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** Collision 2 closed (`DriverPoller`); collision 3: `*RequestSettings`, the rest kept.
**⟂ 8b.** "pump" has one meaning (a boundary round) and three report types.
Confirm the verb; it is not in the specification's main vocabulary (18).
**Settled 12 October:** the verb is `poll_drivers` (11 October) and every
derivative follows: `PumpBatch` → `PollBatch`, `PumpReport` → `PollReport`,
`InboundCallPumpReport`, `InboundCallResponsePumpReport`,
`InboundCallTimeoutPumpReport` → `*PollReport`, `pump_inbound*` →
`poll_inbound*`, `InboundPumpError` → `InboundPollError`. Polling is the
architecture, not an image: no Driver ever calls the core, the VM decides
when it looks at the world, which is what makes stepping and replay
possible; the code's own docs already say "poll" ten times. The
`DriverPoller` doc states the two halves of a round in one sentence: hand
every journaled open call to its Driver, then collect and settle the
answers that arrived since the last round, the sense of `Future::poll`.
**"drain" (Yoann: still an image)** has two senses in the code and one in
the specification. The specification's sense stays: the closing phase of
an instance, no new admission while admitted work finishes; 56 uses and
the grammar's `drain_for` attribute (15), the standard systems word
("connection draining"). `ClosingStage::Draining` follows it. The VM's
sense goes: `Vm::drain` ("runs every ready transaction: settled closures
and unclaimed mailboxes") is not a closing → **`run_ready`**,
`drain_with_bindings` → `run_ready_with_bindings`; `try_drain` to follow
once read. Not `run_until_idle`: "idle" is the specification's instance
lifetime word.
`try_drain` read 12 October: host observability, "drains every line
currently available without waiting" from a channel; this is the Rust
standard library's own verb (`Vec::drain`, `HashMap::drain`), kept.
**⟂ 8c.** `ConnectorRoles { Inbound, Outbound, InboundAndOutbound }`
implements row 2 of the Contract sheet, *Direction* (who starts), not row 1,
*Role* (what it bridges). Plain word: `ConnectorDirection`; but
`node::Direction` already names a port's Input/Output, so one of the two
must yield the word (raised 11 October). **Settled 12 October (Yoann):**
`ConnectorRoles` → **`ConnectorDirection`**; proposed with it,
`node::Direction` → **`PortDirection`** so both say their subject.

### 9. Time — budget, deadline, timeout, lifetime, expiry, delay

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Deadline`, `Deadlines`, `DeadlineKind` | journal/deadline | one dated obligation by kind and subject id; an immutable schedule accepted with one record; the closed kinds (key 24) | deadline 66 |
| `DatedPayload` (`DeliveryBudget`, `InboundCallBudget`, `JoinWithin`, `LoopDelay`, `RetryDelay`, `InstanceLifetime`, `InstanceClosing`) | boundary | what a dated event carries; the scheduler's projection of the obligations | budget 97 |
| `ClosingDeadline`, `InstanceDeadline` | boundary | scheduler-owned stages of bounded Closing; semantics of a finite dated lifetime | — |
| `TimeAdvance` | vm/advance | the resumable discrete-time sweep | advance 63 |
| `LoopDelay`, `LoopExpiry` | plan | closed Loop delay policy; a host-owned terminal (not a dated expiry) | delay —, expiry — |
| `now`, `logical_time`, `due_at`, `created_at`, `closed_at` | vm, journal | the machine's virtual date; a transaction's fixed instant; the unsealed instants | timeout 115 |

**⟂ 9a.** One obligation, three words along its life: *budget* while it
runs (from the delivery), *deadline* as the journaled record, *timeout* as
the terminal fact. The specification uses all three this way. Confirm, and
state it once in `timing.rs`.
**12 October:** confirmed as is; the three words and their stage are
stated once at the head of `timing.rs`.
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** Confirmed.
**⟂ 9b.** `LoopExpiry` (6c).
**Settled 12 October:** see 6c, `LoopTermination`.
**⟂ 9c.** The decided time model (D-2026-10-10-1) adds *instant*, *origin*,
*logical timeline*; reserve "date" for civil dates and "instant" for the
timeline.

**Settled 12 October (Yoann: "pas mal"):** `DatedPayload` →
**`Obligation`**. The seven variants are the exact obligation to re-check
when its instant arrives, and the `Scheduler` doc already says "every
future obligation … crosses the same frontier". Not `Due*`: the value is
held in the scheduler's queue while it waits, keyed by its due instant, so a
`Due*` name would be false until that instant; "due" is the moment, already
named by `due_at` and `peek_due`. "Dated" goes with it: since
D-2026-10-10-1, "date" is civil and "instant" is the logical timeline; the
prose for `now` says instant.
### 10. The journal — record, fact, packet, fold, protocol, projection

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `Record` (`IngressRecord`, `TransactionRecord`, `EgressRecord` → `CallRecord`, `SessionRecord`) | journal | one causal record; the three kinds | record 228 |
| `Packet` (loop_protocol, instance_stop, retry_protocol) | journal | one sparse protocol sub-record carried by a transaction | packet 14 |
| `Fold` (loop, retry, instance_stop) | journal | a disposable projection rebuilt from committed facts | fold 74 |
| `Fact`, `Command`, `Disposition`, `Operation`, `Opening`, `Settlement` | journal protocols | the Loop fact; the instance-stop command and its disposition; the Retry operation and its phases | fact 110 |
| `Journal`, `MemJournal` | journal | semantic journal operations and queries; the linear M1 in-memory journal | — |
| sealed / unsealed / retained | identity.md | the three classes of a record's fields | sealed 52, unsealed 23, retained 169 |

**⟂ 10a.** `Fold` is the code's word for what the specification calls a
*projection* (100 uses: journal projection, scheduler projection). Confirm
"fold" as the mechanism name, "projection" as the concept, or merge.
**Settled 11 October:** `LoopView`, `RetryView`, `InstanceCommandView`;
"projection" stays the specification's prose word for the concept.
**⟂ 10b.** `loop_protocol::Fact` vs prose "fact" (= any record). Settled
with collision 8: fact is the word; a `loop_protocol::Fact` is a fact of the
Loop protocol, no collision.

### 11. Lab and tooling — scenario, injection, trace, recording, conformance

| Code | Where | Meaning | Spec (uses) |
|---|---|---|---|
| `ScenarioReport`, `ScenarioRun`, `ScenarioSession`, `ScenarioInjectionDraft`, `ScenarioStepError` | lab | the batch report; the interactive run; the step-by-step session | scenario (sim §1) |
| `Trace`, `Divergence` | golden | the run's journal, canonically serialized; one golden difference | golden trace (sim §1) |
| `Provocation`, `Profile`, `InterleaveProbe`, `Report` | conformance | the conformance suite's provocations and profiles | — |
| `Recorder`, recording types | recording | the diagnostic recording (recording 4) | — |
| `Compilation`, `Unit`, `Hir`, `SyntaxTree`, `SymbolTable`, `Diagnostic`, `Bag` | compiler | the compiler's layers and side tables | — |

**⟂ 11a.** Collision 5 (`Profile`).
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `PrivacyDeclaration`, `PriceBook`, `json::DeserializationPolicy`, `DriverTestFixture`.
**⟂ 11b.** `ScenarioSession` reuses "session", the Connector exchange word.
**Proposed 12 October:** `ScenarioSession` → **`Simulator`**, the
specification's own word (simulation-semantics §1: "the simulator — the
deterministic runner: virtual clock, mocks, scripted journal — one, in the
runtime"); its fields are exactly that: plan, scenario, vm, journal, scripted
driver, action cursor, windows. Not bare `Scenario` (Yoann's question):
`hir::Scenario` is already "one checked scenario", the declaration, and
that is the right owner of the bare word. Not `PlannedScenario`: scenarios
never enter the core plan (zero mentions in `plan.rs`; the `lab` corpus is
"never deployed"), and the live thing is not a declaration. `ScenarioWorld`
withdrawn: "world" is the fixture half. `ScenarioRun { report, records }`
stays, the result of one run.
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** `Simulator`.
**⟂ 11c.** The HIR mirrors plan names (`hir::Node`, `Graph`, `Handler`,
`Edge`) — same word, previous layer. Acceptable by module path; confirm.

**Proposed 12 October:** accepted as is; same concept, previous layer, the
module says the layer; the lowering, the one file holding both, prefixes
`hir::`, as one writes `syn::Item`.
**Settled 12 October (Yoann, batch ok: "pour le reste on est à peu près bon").** Accepted.
## Validated — 11 and 12 October

| Term today | What it is | Yoann's decision | Status |
|---|---|---|---|
| `Draft`, `PortOutput` | what the world provides, at ingress (`Draft { data, ext }`, always beside a `port`) and as a Driver's output (`PortOutput { port, data, ext }`): the same two fields, the same owner, the same rule, the Driver proposes and the VM mints; no `sys`, no `app` | **one type, `EnvelopeDraft { port, data, ext }`** (word closed 11 October, night; merge 12 October). Draft is the specification's word (26 uses) and says the state: not yet an envelope, it becomes one when the admission or the Connector transaction commits. The port belongs to the draft: the ingress doc already says "source port of the draft", the Driver trait says Drivers "birth drafts and never envelopes", and a draft is never built without a port. `PortOutput` disappears; `Completion` holds ordered drafts with exactly one on `exit`, last; the session terminal is a draft. No byte changes: the codec already writes `[port, data, ext]`. `EnvelopeContent` dropped: structural, and too close to `data`. Not `envelope_data` | decided |
| mint (doc comments: "the journal mints the id") | derive and assign the definitive identity | **identify** / **assign_identity** | decided |
| `pump` | one boundary round with the Drivers | **`poll_drivers`** | decided |
| `CompletionGate`, `OpenGate`, `PlanGate` | a conformance check of a Driver's completion against the Contract: the trait, the implementation that accepts anything, the plan-backed one | **`CompletionCheck`**, **`AcceptAnyCompletion`**, **`PlanCompletionCheck`** (Astra's split, 12 October: a trait and its implementations cannot share a name) | decided |
| bless (goldens, witnesses) | accept a new reference | **approve** | decided |
| `Fold` | a read model rebuilt from committed journal facts: for Loop, every open run with its instance, entry port, iteration, remaining budget, state envelope, admission and pending command | **`LoopView`**, **`RetryView`**, **`InstanceCommandView`** — "a view of" something, less academic than projection (Yoann) | decided |
| `Intent` (`SessionOpenIntent`, `SessionWriteIntent`) | the description of an outbound effect handed to the Driver after its open fact committed | name the effect, drop the suffix: **`CallOpen`**, **`SessionOpen`**, **`SessionWrite`**; journal-before-effect is stated once, on the Dispatcher | decided |
| fact | the specification's word for a committed journal record (110 uses); `Fact` in the Loop and Retry protocols | **kept.** A fact is an assertion the journal answers for: it exists once committed, replay reproduces it on any machine, and it carries the transaction's verdict. An event is an occurrence; a fact is an established assertion, and may describe no occurrence at all ("the Loop admitted iteration 3") | decided |
| event | the exchange kind a Contract declares (`ConnectorExchange::Event`): the world speaks first, each firing is one spontaneous inbound root, nothing is owed back; Timer and Calendar in the stdlib | **kept for the exchange kind only.** The Contract sheet has two independent axes: Direction (who starts: `ConnectorRoles`) and Exchange (what is owed back: nothing / exactly one `exit` / messages then one `exit`). Examined and not retained: InboundCall / OutboundCall / Session, which fuses direction into two of the three kinds, would need Accepted/Opened for sessions, and loses event ≠ inbound call (zero `exit` vs exactly one, enforced by the validator). The empty cell, an outbound event, is the closed-call rule | decided |
| observation | what a Driver reports from the world before any transaction commits it: a response arrived, a session opened, a write settled, the host restarted. A tick and an HTTP request arrive by the same carrier (`IngressProposal`); the Contract's exchange kind says which place the observation takes | **new plain word**, already the word of 23 doc comments in `boundary.rs`. Rename the four carriers: `BoundaryEvent` → `BoundaryObservation`, `InboundCallEvent` → `InboundCallObservation`, `SessionDriverEvent` → `SessionObservation`, `LifecycleEvent` → `LifecycleObservation`; fix the `BoundaryEvent` doc ("one routable boundary fact for the VM": it is not yet a fact). `IngressProposal` keeps its name. Spec sentence to align with the realignment: extensions.md §1 "A Driver supplies a typed event" → observation | decided; the renames are a code lot |
| command | what a protocol asks the next transaction to do: Loop `Command`, `CommandProduced`, `CommandId`; instance-stop `Command`; `instance_commands` on the transaction record | **kept.** With fact, the two CQRS words of the journal protocols; no third word for them | decided |
| admission | the specification's central word (186 uses, ten files): a delivery entering an address, root or internal; the "admission point" is mailbox reservation plus the producer's route commit; `OpeningAdmissionId`, Loop re-admission, the `idle` deadline runs from the last admission | **kept**, examined for removal and refused: no existing word covers every delivery (ingress = roots only, acceptance = the positive outcome only, entry = the Loop's entry port, enter = no verdict); "admission control" is the standard systems term for a bounded entry with accept / pending / reject; removing it is a specification change, not a code rename | decided |
| the ingress chain: `IngressSubmission`, `SubmissionSequence`, `IngressProposal` (with its `reference` field), `IngressEnvelopeProvenance`, `IngressAcceptance`, `IngressDecision`, `inject_submission` | the door's wrappings and verdicts, eight words for one thing offered and judged | **flattened to the specification's words** (11 October, night): `IngressProposal { provenance, draft: EnvelopeDraft, sequence: IngressSequence, session }` is what is proposed, by the host or a Driver (the port is inside the draft, 12 October) (`IngressSubmission` disappears into it; `SubmissionSequence` → `IngressSequence { Next, Explicit }`); `RetainedProposal { handle, proposal }` is the proposal a Driver keeps while it waits; `IngressEnvelopeProvenance` → `IngressProvenance`; `IngressAdmission { Accepted(AdmittedIngress), Pending, Rejected(IngressRejection) }` stays the verdict; `IngressAcceptance` → **`AdmittedIngress { envelope, exchange }`**, named by its place in the chain beside `PreparedIngress` and `CommittedIngress` (Yoann: not `AdmittedRoot`; "root" reads as an image even though the specification uses it 192 times); `IngressDecision` → `Result<AdmittedIngress, IngressRejection>`, the verdict without `Pending` as a standard type; the one transaction behind both doors, `inject_submission`, → `admit_proposal`; `inject` and `admit_ingress` stay as the two doors. Words after: draft, proposal, admission, rejection, ingress; gone: submission, acceptance, decision. `AdmittedIngress` is what the Driver is told (two ids, portable); `CommittedIngress` is the journal's return (seq, envelope, record): the two doc lines must state that difference. Cost: `IngressSubmission` in 16 files, `IngressDecision` in 8, all host Drivers; a code lot, with the realignment or after the opening | decided |
| `plan::emission::Emission` | the lowered right-hand side of `out = …`: `Identity`, `Patch`, `FreshData`, `Data`, `Nothing`; held as `value` by `DispatchStep::Emit` and by `MultiEmitValues` | **`EmitValue`** (12 October, Fable's final proposal after `EmissionSpec` and `EmissionKind` were examined; Yoann's ok to confirm). The word of its neighbours, `Emit { port, value }`, `MultiEmitValues`, `FieldWrite.value`, and the specification's phrase "a braced value is classified once". In a plan every noun is the checked text of what will happen at run time, so "value" means the value expression, as the code already uses it. `journal::Emission` keeps the specification's word; `runtime.rs` imports both today and aliases the plan one `PlanEmission`, which disappears. Not retained: `EmissionSpec` (coherent with the header configurations `ConnectorSpec`, `LoopSpec`, `RetrySpec`, `ExtensionSpec`, not with the handler body where it lives, and `MultiEmitValues` would have had to follow); `EmissionKind` (the code's 18 `*Kind` enums are category tags of a thing that lives elsewhere, 9 of 144 variants carry a small detail; here three of five variants are the content); `EmittedEnvelope` for the journal; `EmissionForm`; `EmissionRule`; `EmissionRecipe`. The `*Spec` suffix stays what it is, the plan's checked description of an authored header declaration; collision 7 concerns only the bare `instance::Spec` | decided (batch ok, 12 October) |
| `boundary::Dispatcher` | the loop between the journal and the outbound Drivers: each journaled open call is handed to its Driver exactly once, journal before effect, and closures settle through the close-once race | **`DriverPoller`** (Yoann, 12 October), with the verb `poll_drivers`; the Operator's `Dispatch` keeps the specification's word. Collision 2 closed | decided |
| `ConnectorRoles` | Inbound, Outbound, both: row 2 of the Contract sheet, *Direction* | **`ConnectorDirection`** (Yoann, 12 October); `node::Direction` → `PortDirection` proposed with it | decided |
| `BootBindings` | fully resolved boot inputs: the `env()` / `secret()` values bound at boot | **kept** (Yoann, 12 October): a binding in the specification's main sense | decided |
| `syntax::ExtensionEvent` | recovery-only capture of the removed `.start` / `.stop` forms | **`RemovedLifecycleHook`**: the parser already has `RemovedWindowMode` for the same job | decided (batch ok, 12 October) |
| `Packet` (loop, retry, instance_stop) | the protocol's sparse contribution to one transaction (its own doc line): Loop = contexts, routes, commands produced and consumed, facts; Retry = attempt context and facts; instance stop = commands produced and consumed | **`LoopRecord`**, **`RetryRecord`**, **`InstanceCommands`** (Yoann, 12 October, "quite ok"): the names the public fields of `TransactionRecord` already carry (`loop_record`, `retry_record`, `instance_commands`), so no new word. Reservation written in each doc: a `LoopRecord` is the Loop protocol's part of a transaction record, carried by it and never standalone, not a fourth causal record kind. `Contribution` not retained | decided |

## Outcome — the rename lot, closed 12 October

Every line of this ledger is settled. What follows is the deliverable: the
renames to apply, by family, with what stays and why. No rename changes a
journal byte; the codec writes fields, not type names. Placement decided 12 October (Yoann): the lot is applied with the
realignment, before the opening; the T4 second pass is suspended after block
1 so that the realignment, A-T4-4 and this lot start.

### Renames

| Today | Decided | Note |
|---|---|---|
| `Draft`, `PortOutput` | `EnvelopeDraft { port, data, ext }` | one type for what the world provides, both directions |
| `IngressSubmission`, `IngressProposal.reference` | `IngressProposal { provenance, draft, sequence, session }`, `RetainedProposal { handle, proposal }` | the port is inside the draft |
| `SubmissionSequence`, `IngressEnvelopeProvenance` | `IngressSequence`, `IngressProvenance` | |
| `IngressAcceptance`, `IngressDecision` | `AdmittedIngress { envelope, exchange }`, `Result<AdmittedIngress, IngressRejection>` | `IngressAdmission`, `IngressRejection` stay |
| `inject_submission` | `admit_proposal` | `inject`, `admit_ingress` stay as the two doors |
| `BoundaryEvent`, `InboundCallEvent`, `SessionDriverEvent`, `LifecycleEvent` | `BoundaryObservation`, `InboundCallObservation`, `SessionObservation`, `LifecycleObservation` | event = exchange kind only; fix the `BoundaryEvent` doc ("fact") |
| `Intent`, `SessionOpenIntent`, `SessionWriteIntent` | `CallOpen`, `SessionOpen`, `SessionWrite` | journal-before-effect stated once on the poller |
| `CompletionGate` (trait), `OpenGate`, `PlanGate` | `CompletionCheck` (trait), `AcceptAnyCompletion`, `PlanCompletionCheck` | a trait and its two implementations cannot share one name; the permissive one says it checks nothing (Astra, 12 October) |
| `Dispatcher`, `DispatchError` | `DriverPoller`, `PollFault` | `Dispatch*` of the Operator stay |
| `pump`, `PumpBatch`, `PumpReport`, `InboundCall*PumpReport`, `pump_inbound*`, `InboundPumpError` | `poll_drivers`, `PollBatch`, `PollReport`, `InboundCall*PollReport`, `poll_inbound*`, `InboundPollError` | |
| `ConnectorRoles`, `node::Direction` | `ConnectorDirection`, `PortDirection` | |
| `HttpClientRequestBindings`, `FileRequestBindings`, `ProcessRequestBindings` | `*RequestSettings` | `BootBindings`, `expr::Bindings`, `*ConnectorBindings` stay |
| `HandlerExecutor`, `GuestExecutor` | `TransactionPreparer`, `GuestRunner` | Executor returns to the taxonomy |
| `Built`, `BuiltData` | `BuiltEmission`, `BuiltPayload` | crate-private |
| `plan::emission::Emission` | `EmitValue` | `journal::Emission` stays; `PlanEmission` alias disappears |
| `Vm::drain`, `drain_with_bindings` | `run_ready`, `run_ready_with_bindings` | the closing sense of drain stays |
| `TimeAdvance::step` | `advance_once` | `Vm::step` stays |
| `VmError` | `VmFault` | `Fault` stays; module `Error`s stay |
| `DatedPayload` | `Obligation` | "dated" leaves the timeline vocabulary |
| `Observed`, `observations()` | `EnvelopeView`, accessor follows | an envelope seen at a node port, leaving or arriving: `vm.rs` logs the emission at its source port ("`expect relay.out` watches departures") and each delivery at its target, so `DeliveryView` covered half (Astra, 12 October) |
| `AccountingObservation`, `ObservationScope` | `Measurement`, `MeasurementScope` | `RecordObservation` stays |
| `CostProfile`, `CostProfileName` | `PriceBook`, `PriceBookName` | |
| `PrivacyProfile` | `PrivacyDeclaration` | |
| `json::HydrationProfile`, `hydrate`, `hydrate_with_profile`, `json/hydration.rs` | `json::DeserializationPolicy`, `deserialize`, `deserialize_with_policy`, `json/deserialization.rs` | `materialize` does not move |
| `conformance::Profile`, `EventProfile`, `SessionProfile`, `Provocation`, `unprovable` | `DriverTestFixture`, `EventTestFixture`, `SessionTestFixture`, `ExitCaseTest`, `untestable` | |
| `ScenarioSession` | `Simulator` | `hir::Scenario`, `ScenarioRun` stay |
| `syntax::ExtensionEvent` | `RemovedLifecycleHook` | |
| `instance::Spec` | `InstanceSpec` | |
| `instance::Identity` | `InstanceAddress` | `Id` stays |
| `Live`, `Live::frames` | `LiveInstance`, `segments` | "bundle" is prose only |
| `Edge::scope` | `owner` | |
| `LoopExpiry` | `LoopExpiredOutput` | only the output chosen to publish `LoopExpired`; `LoopSpec.terminals` are the business terminals, so `LoopTermination` overclaimed (Astra, 12 October) |
| `Fold` (loop, retry, instance_stop) | `LoopView`, `RetryView`, `InstanceCommandView` | |
| `LoopDelay` (type shared by `LoopSpec.delay` and `RetrySpec.delay`) | `DelayPolicy` | Loop and Retry share the policy, not the concept; `Obligation::LoopDelay` / `RetryDelay` stay two obligations (Astra, 12 October) |
| `Packet` (loop, retry, instance_stop) | `LoopRecord`, `RetryRecord`, `InstanceCommands` | the names their fields already carry |
| mint, bless (doc verbs) | identify / assign_identity, approve | |

### With the realignment (D-2026-10-11-1)

`Slot` / `OwnerSlot` → `NodeIndex`; `EdgeEnd`, `EndpointOwner`,
`RouteDestination` → `Endpoint` (index + port) and `Destination`;
`Delivery.edge` becomes an index and `Delivery.port` is derived; a `Worker`
type in the `rastok` crate owns the hosted loop.

### Kept after examination

admission; `instance::Generation` (always qualified, never imported bare);
`BootBindings`, `expr::Bindings`, `*ConnectorBindings`; `Dispatch`,
`DispatchStep`, `DispatchSelection`; `journal::Emission`, `Delivery`,
`RoutedDelivery`, `RouteTarget`; `RecordObservation`; `Fault`; drain
(closing sense, `drain_for`), `try_drain`; module-local `Error`; the HIR
names; budget / deadline / timeout; activation / trigger / firing;
`DeploymentProfile`, `contract::Profile`; the `*Record` and `*Exit` types.

### Specification sentences to align

extensions.md §1 "A Driver supplies a typed event" → observation; the 31
"hydration" → deserialization. Both in the realignment's doc pass.

### Docs to write once

The two halves of a `DriverPoller` round; `AdmittedIngress` (what the
Driver is told) versus `CommittedIngress` (the journal's return); what a
`*Spec` is in the plan; fixture = prepared test material, a scenario's in
the Lab, a Driver's in conformance; the three time words at the head of
`timing.rs`; `LiveInstance`'s "immutable values and settings"; `EmitValue` is
"the expression evaluated to build the envelope", not a computed value;
`*View` is a read-only view for a reader — the Loop, Retry and
InstanceCommand views rebuilt from the journal, `EnvelopeView` the
debugger's live log (both Astra).

### Applied — 13 October (Astra), reviewed by Fable

Astra applied the lot in ten commits on `realign/node-ontology`, `32bc654` to
`5bc7f0a`, from `4b4a4a4`: one commit per family, the two specification
sentences in their own commit (`008cbb4`), the file renames last (`fold.rs`
→ `loop_view.rs` / `retry_view.rs`, `hydration.rs` → `deserialization.rs`,
`emission.rs` → `emit_value.rs`, test `machine_executor.rs` →
`transaction_preparer.rs`). Fable's review, 13 October:

- Every decided name is declared and used; the old names survive only in
  `decisions.md` (immutable), the historical reports and this ledger.
- No witness moved: no identity literal changed in Rust, no fixture, golden,
  package, trace or recording touched; `Draft` and `PortOutput` are one
  `EnvelopeDraft`; `IngressDecision` is a `Result`; `IngressAdmission` keeps
  its explicit `Pending`.
- Checks run by Fable on `5bc7f0a`: `cargo fmt --check`, `cargo clippy
  --workspace --all-targets --all-features -D warnings`, `cargo test
  --workspace --all-features` (1 572 passed in 61 groups, 0 failed, 0 ignored), `cargo doc --workspace --no-deps`,
  and `cargo check` of the rp2350 spike on `thumbv8m.main-none-eabihf`.
  The one test above the brief's 1 571 was already on `review/t2-t4`.
- The docs to write once are written. The three time words sit at the head of
  `journal/deadline.rs`, where the `Deadline` type lives, not `timing.rs`:
  the line above is corrected, the placement is the right one.
- `inject_submission` already reads `admit_proposal`; its decomposition stays
  A-T4-4 in realignment lot 1. Astra also corrected the `VmFault` doc, which
  claimed that a failing operation committed nothing: the journal decides.

**Correction under rule 1 — bless (to Yoann).** The specification names the
act: simulation-semantics writes "reviewed **`--bless`**", the CLI flag
`--bless` and `RASTOK_BLESS=1` carry it, `golden.rs` implements it. The
11 October line "bless → approve" was written without that check and
conflicts with rule 1; Astra applied it faithfully to 19 prose lines
(README, ROADMAP, the golden and fixture test module docs), which now say
"approval" for what the flag calls bless. Fable proposes to withdraw the
"approve" line and restore "bless" in those 19 lines; the flag never moved.

**Derivatives the table missed (follow-up, mechanical, to Yoann).**
`PrivacyProfileName` → `PrivacyDeclarationName`; `CostProfileAttestation`
→ `PriceBookAttestation`, with `DeploymentProfile::with_cost_profile` and
the field `DeploymentAttestation.cost_profile` — the deployment JSON key
`cost_profile` is user-facing and stays. `hir::LoopDelay`, shared by
`hir::LoopSpec.delay` and `hir::RetrySpec.delay`, is the plan's collision
again; HIR names were kept as a block, so Astra was right not to touch it —
Yoann decides whether it follows `DelayPolicy`. `journal::admission::Fold`
(crate-private, the lifecycle projection used by admission) was not among the
three folds listed. Residual words: "packet" in
`retry_protocol/encoding.rs::packet_tree`, `instance_stop::Error::PacketShape`,
a few doc comments and the test enum `Retained::Packet`; "mint" in the UI
placeholder "minted on admission" (`ui/view.rs`) and in test messages;
"dispatcher" in the label of `04-app-lab-effect-cycle.puml`; "unprovable" in
`components.md` and `connector-drivers.md`. `mint_capability` (the UI
authorization token) is another act and stays.

### Execution interface — addition approved 13 October (Yoann)

| Current name | Decided name | Meaning preserved |
|---|---|---|
| `Vm`, `runtime::vm` | `Execution`, `runtime::execution` | live execution state for one validated plan, driven by the hosted Worker |
| `VmFault`, `VmInspection` | `ExecutionFault`, `ExecutionInspection` | fault and read-only inspection of that execution |
| `Simulator::vm()`, `SnapshotInput::vm` | `Simulator::execution()`, `SnapshotInput::execution` | access to the same execution; snapshot JSON fields do not change |
| `inject` | `accept_input` | accept an App input proposal |
| `admit_ingress` | `accept_connector_output` | accept a world-originated source-Connector proposal; not settle an outbound Call |
| `install_inbound_exchange` | `apply_exchange_admission` | update live Call or Session state after journal commit |
| `install_target` | `apply_instance_admissions` | update or create the admitted instance chain after commit |
| `install_graph_contexts` | `update_graph_loop_contexts` | discard obsolete contexts and add the committed ones |

The shared sequence reads **prepare → commit → apply**. `commit` remains
the journal operation; `apply` updates memory from the committed decision.
These are not two transactions. No reader-visible acceptance or recovery
behavior changes. The two public acceptance methods are alternatives, not
two phases. The admission nouns, `admit_proposal`, `step`, `run_ready`,
`poll_drivers` and the journal's `commit_ingress` keep their names. A complete
post-commit helper, if needed by the decomposition, is `apply_admission`;
this decision does not require adding a helper for its own sake.

This addition supersedes only the corresponding “kept” entries of the
12 October lot. Status: implemented; see the [naming checkpoint](node-realignment-qualification.md#execution-interface-naming-checkpoint)
for its checks. The remaining Node representation changes belong to lot 1,
not to this naming addition.

### Execution methods — follow-up approved 13 October (Yoann)

The construction, time and readiness methods now follow the same interface:

| Previous name | Decided name | Meaning preserved |
|---|---|---|
| `Execution::boot`, `boot_at`, `boot_with_deployment`, `boot_at_with_deployment`, `boot_program` | `new`, `new_at`, `new_with_deployment`, `new_at_with_deployment`, `new_program` | construct fresh state without starting a handler or Driver |
| `Execution::advance` | `advance_to` | advance to an absolute logical instant, processing due obligations and ready work |
| `Execution::now` | `logical_time` | read the current logical instant |
| `Execution::has_ready_transaction` | `has_ready_work` | test for work that can proceed without new external input or a time advance |

The DSL `now()` and scenario `advance` keep their names and semantics.
`BootBindings` still describes deployment inputs. There is no `run()` or
process-level `stop()` on `Execution`: the hosted Worker remains a separate
lot. The private constructor becomes `new_at_inner`.

Yoann authorizes a patch for the three ambiguous method declarations
(`boot`, `advance`, `now`) and their verified references: RustRover's
name-only selector cannot distinguish them from other symbols in the file.
Unique names use semantic refactoring; verified references outside the IDE
workspace are repaired under the earlier exception. No compatibility alias,
journal field, package byte or execution rule changes.

### Applied — 14–15 October (Astra), realignment lots 1 and 2, reviewed by Fable

The realignment's names landed as written above: `Slot` / `OwnerSlot` →
`NodeIndex`; `EdgeEnd`, `EndpointOwner`, `RouteDestination` → `Endpoint`
and `Destination`; `Delivery.edge` is an `EdgeIndex` and `Delivery.port` is
derived through `runtime::DeliveryRoutes`; the `Owner` and `Node` rows are
one `plan::Node` with a `NodeKind` body (Graph wiring, Operator variables
and handlers, Connector specification), and `Handler::owner` is gone; `Vm`
→ `Execution` as approved on 13 October; the inspection types live in
`runtime::execution::inspection`, `CallDirectionInspection` is removed
because it restated `CallReferenceInspection`, `SessionDirectionInspection`
stays because a `SessionId` does not say who opened; `Worker` (⟂ 7d).
Mechanism words that landed without passing here, each defined where its
type lives: `EdgeIndex`, `DeliveryRoutes`, `NodeKind`,
`expr::MAX_EXPRESSION_DEPTH` (D-2026-10-14-1, the expression bound now
distinct from the reader's `MAX_NESTING`), `worker::Host` (⟂ 7e),
`worker::Error`, `worker::RunError`. Two user-visible Lab failure sentences
changed wording with the Worker: "the machine did not settle within 64
boundary rounds" became "execution did not become idle within 64 Driver
polling passes", and "the boundary round faulted" became "Driver polling
faulted", while `conformance.rs` keeps the former; no test pins either.
Still to Yoann from 13 October: bless and the missed derivatives above.

### T5 — 17 October (Yoann), the effect boundary and the Driver seam

Read on the skeleton of `rastok-core/src/boundary.rs` (3 369 lines, flat,
no section marker), `connector.rs`, `connector/call.rs` and
`rastok/src/connectors/registry.rs`, from Fable's T5 brief. Names only at
this stage; the layout questions are recorded for v21.

**⟂ 8a. `boundary` is a catch-all module name.** The file holds six
concerns: the exchange identities (`CallId`, `InboundCallId`, `SessionId`,
`SessionWriteId`, `IngressProvenance`), the journaled intents (`CallOpen`,
`SessionOpen`, `SessionWrite`, `EnvelopeDraft`), the world's observations
and their admission (`IngressProposal` to `IngressAdmission`, `Response`,
`Completion`, `SessionObservation`), the scheduler's `Obligation` and
deadlines, the eleven Driver traits with the `DriverPoller`, and the
standard-library mirror (fourteen `process_`/`file_`/`network_`/`http_`/
`websocket_` functions and five `*Exit` enums read by `plan/validation.rs`).
Yoann's reading: a Connector module should hold the Connector interfaces,
but `connector` is taken by `ContractId` and versioning. Fable's
verification: the specification uses *boundary* 247 times in 21 files (the
effect boundary, the App boundary, "the boundary"), *Driver* 208 times,
*Connector* as one of the two Executor forms; `driver` is free as a module
name in both crates. **Proposed:** `boundary` keeps the specification's
word for the boundary vocabulary (identities, intents, observations,
admission, obligations); the Driver traits go under a `driver` module (⟂ 8b);
the standard-library mirror leaves `boundary.rs` — its destination is the
structure question of the T5 brief, not a name. Layout: v21.

**⟂ 8b. The repeated suffix `ConnectorDriver` names a missing module.**
`InboundConnectorDriver`, `InboundCallConnectorDriver`,
`OutboundConnectorDriver`, `CallConnectorDriver`, `SessionConnectorDriver`
in core; `ConnectorDriverFactory`, `BoundConnectorDriver` in the host
registry. Rule 6, the existing word first: *Driver* is the specification's
word (decisions.md: "A **Driver** is one runtime implementation of that
contract"). **Proposed:** `driver::Inbound`, `driver::InboundCall`,
`driver::Outbound`, `driver::Call`, `driver::Session` in core;
`driver::Factory` and `driver::Bound` (or the registry keeps them with the
short names) in the host. Zero bytes; the host imports the seam through
twenty-six `use crate::boundary::…` lines that move with it.

**⟂ 8c. `*Bindings`, reopened by the reading.** Settled on 12 October under
collision 3: `OutboundConnectorBindings`, `SessionConnectorBindings` and
`RuntimeBindings` stay, in the specification's second sense — decisions.md:
"A **Binding** is the verified association between a plan Connector and the
Driver instance that will serve it". Yoann's 17 October reading: the word
suggests name-to-value bindings, which these are not; the trait reads as
"the interface for a Driver's outbound calls". Fable's verification: it is
not one Driver's interface but the routing façade over every bound Driver,
by exact Node path, for the outbound direction — the type `Execution`
consumes (`&mut dyn OutboundConnectorBindings`); `RuntimeBindings` is the
host's complete association. The name says what the thing is (the bound
Drivers), not what `Execution` does with it. **Fable's recommendation:**
keep the settled word, specification-grounded, and have the first doc line
of each trait state the sense ("the Drivers bound to this plan, outbound
face"); if Yoann reopens, the candidates are `OutboundDrivers`,
`SessionDrivers`, `BoundDrivers`. The overlap between the two traits
(`OutboundConnectorBindings` already carries the Session methods that
`SessionConnectorBindings` repeats, `RuntimeBindings` implementing both and
delegating one to the other) is a structure question in the T5 brief, not
a name. **To Yoann.**

**⟂ 8d. "VM" survives in prose after `Vm` → `Execution`.** Settled by
Yoann on 17 October: comments say `Execution` from now on — "consumed by
the `Execution` of the `ValidatedExecutionPlan`". Count: 77 mentions of
"VM" in code comments (18 in `boundary.rs`, 13 in `connectors/registry.rs`,
10 in `lab.rs`, 6 in `runtime.rs`, 5 each in `execution.rs` and
`execution/advance.rs`), and 29 in the specification outside
`decisions.md` (`observability.md` 7, `ui.md` 6, `execution-model.md`,
`simulation-semantics.md` and `types.md` 3 each, six files once).
Fable's note: the specification is the semantic authority, so its 29 move
first or together, else the code's word diverges from the specification's;
`decisions.md` is dated and stays. Recorded as A-T5-1 in the review ledger.

### Connector rework — 21 October, before the freeze

Yoann requested that this proposal enter the human-review work to do
**before the freeze**, and that Fable review it for further simplification.
The actions and qualification order are
[A-T5-2 through A-T5-6](human-review-m1.md#connector-rework--requested-before-the-freeze-21-october).
The preceding T5 notes remain the dated record of the discussion; this
entry replaces their proposed layout and scheduling, including the choice
to retain `boundary` and the Driver-association `*Bindings` names.

**Status.** The direction and the placement `connector/drivers` and
`connector/stdlib` are agreed with Yoann. The tables below record the
requested refactor target, still open to improvements from Fable before
implementation. They do not claim that the current specification already
defines one Contract for all Process modes, or that the current registry
can select several implementations of one Contract. No code, normative
specification or witness changes in this documentation commit.

#### 1. The model a contributor should read

| Current organization | Target | What stays distinct |
|---|---|---|
| One DSL name can select different exact ContractIds: Process job `std/process@4`, service `std/process@5`. | One DSL name designates one versioned Contract; attributes statically select a declared mode. | Job remains an outbound Call; service remains a resident Session. A mode changes the resolved interface/lifecycle, not which unnamed contract the author selected. |
| `DriverRegistry` admits one factory per ContractId. | Several physical implementations may satisfy a Contract, with checked selection per Connector Node. | Contract identity describes semantics; implementation selection describes the physical service used. |
| `RuntimeBindings` groups Drivers and correlates Calls/Sessions by ContractId. | `RuntimeDrivers` associates each Node and open exchange with the selected implementation. | Two implementations of the same Contract must never receive each other's responses, writes or closures. |
| Contract definitions, core rules, Driver implementations and contributor documentation have different entry points. | A standard catalogue links the checked Contract, its modes, available Drivers, Lab behavior and conformance fixture. | Knowing a Contract does not imply that a real Driver is installed or needed for simulation. |

The resolved Contract declares direction separately from exchange kind:
Event, Call or Session. An Event has no per-event terminal obligation; an
inbound Call does. The response to an outbound Call closes that existing
Call, not a new ingress root. A Session carries many messages and one
terminal lifecycle, not one Call per message. Semantic Connector state is
journal-owned; process/socket/pool state belongs to the Driver.

ObjectStorage is the motivating **future** example: an S3, Azure Blob or
GCP implementation could implement the same Contract only if each meets
its actual guarantees. Provider-specific operations cannot be silently
accepted or weakened. Provider/resource/credential selection may be
deployment configuration; a requirement that affects semantics belongs to
the Contract and checked program. Sharing a Contract does not make
different providers' data, outcomes or traces equal. No ObjectStorage
Contract or cloud implementation is added by this work.

#### 2. Module responsibilities

Paths in this table are relative to each crate's `src/`. They group
responsibilities, not one file per type. Existing cohesive modules stay
together where the split buys nothing.

| Current location / responsibility | Target location | Reason |
|---|---|---|
| Core `boundary.rs`: exchange identities, requests, completions, ingress proposals and observations | Core `connector/exchange.rs` | Data exchanged between the Driver and execution, without polling or scheduling policy. |
| Core `boundary.rs`: per-Driver traits and the execution-facing Driver routing interface | Core `connector/driver.rs` | The interfaces a Driver implements and the interface execution consumes, documented separately. |
| Core `boundary.rs`: Process/File/HTTP/WebSocket type and completion rules; standard ids in `connector.rs` | Core `connector/stdlib/` | Portable standard Contract rules used by Registry-free validation. The semantic namespace remains `std/…`. |
| Core `boundary.rs`: `DriverPoller`, `PollBatch`, `PollFault` | Core `runtime/execution/driver_polling.rs` | Execution orchestration: send committed requests, collect responses, settle journal facts. Not a physical Driver implementation. |
| Core `boundary.rs`: `Obligation`, `ClosingDeadline`, `InstanceDeadline` | Existing core `runtime/execution/timing.rs` | Scheduler-owned obligations live with execution timing, not the contributor's Driver data catalogue. |
| Hosted `connectors/`: physical implementations, assembly, manifest and private network support | Hosted `connector/drivers/` | These are Connector Drivers, not general-purpose system drivers. No top-level `rastok::drivers`. |
| Hosted `connectors/registry.rs`: factory registry and runtime association mixed in one file | Hosted `connector/drivers/registry.rs` and `connector/drivers/runtime.rs` | Registry = available implementations; runtime association = the selected objects and exchange ownership for a plan. |
| Connector definitions within hosted `contract.rs` and `stdlib.rs` | Hosted `connector/contract.rs` and `connector/stdlib/` | Descriptive Contract definitions/catalogue, selectors, attributes and checked standard entries are discoverable under Connector. |
| Hosted `conformance.rs` | Hosted `connector/drivers/conformance.rs` | A Driver implementer finds the verification harness beside the implementation interfaces and factories. |

Use **`stdlib`**, not `standard` or `std`, for these module names: it
identifies the standard library without conflicting with Rust's `std`,
particularly in the `no_std` crate. Do not move the non-Connector parts of
`contract.rs`/`stdlib.rs` (frontends, annotations and other language
definitions) just to make the directory tree uniform.

The hosted `connector` module exposes the shared core types and adds
hosted definitions/implementations; it does not duplicate the types.
These deliberate cross-crate exports are not aliases preserving the old
`boundary` or `connectors` paths. The core never depends on hosted I/O,
the compiler catalogue or generated documentation. Its independent
Contract checks and the catalogue parity tests remain required.

`Supervisor` and `LifecycleObservation` currently have no caller in the
Rust crate/spike sources inspected for this proposal. Review their removal
separately instead of relocating an unused public abstraction by default;
this is not authorization to add a supervision system.

#### 3. Contract vocabulary

| Before | Proposed after | Meaning |
|---|---|---|
| `ConnectorIdError` | `ContractIdError` | Invalid versioned Contract identifier. |
| `ConnectorExchange` | `ExchangeKind` | Event, Call or Session; not direction. |
| `contract::Profile` | `ContractMode` | A named, statically selected mode of a Contract. |
| `ProfileSelector` | `ModeSelector` | The attribute and default selecting that mode. |
| `ProfileSupport` | `ModeSupport` | Checker/execution support for a declared mode, not availability of a physical Driver. |
| `SheetDefect` | `ContractValidationError` | A malformed Contract definition. |
| `ProfileProtocol` with an overriding `id` | Direction and exchange facts of the selected mode, without another ContractId | **Model change**, only after the Process Contract specification and identity inventory are reviewed. The final data layout still needs review. |

Keep `ContractId`, `ConnectorContract`, `ContractCatalog`,
`ConnectorDirection`, `PortContract`, `AttributeSpec`, `PortTypeRule` and
`CallSemantics`. Do not rename deployment profiles or JSON policies when
renaming the Contract's modes. The new words describe the same rules,
except for the explicitly identified ContractId consolidation.

#### 4. Driver interfaces

Here `driver::` means core `connector::driver`, exposed by the hosted
crate as well; `drivers::` means hosted `connector::drivers`.

| Before | Proposed after | Implemented or used by |
|---|---|---|
| `InboundConnectorDriver` | `driver::Ingress` | Driver proposing incoming Events, Calls or Session messages. It is not an Event-only trait. |
| `InboundCallConnectorDriver` | `driver::InboundCall` | Driver returning responses or timeouts for inbound Calls. |
| `CallConnectorDriver` | `driver::OutboundCall` | Driver executing outbound Calls. |
| `SessionConnectorDriver` | `driver::Session` | Driver opening, writing, closing and observing Sessions. |
| `OutboundConnectorDriver` | `driver::Outbound` | The outbound Call/Session interfaces exposed by one Driver object. |
| `BoundConnectorDriver` | `driver::Driver` | One implementation object exposing its supported ingress/outbound interfaces over shared physical state. Moving this generic trait into core must introduce no hosted dependency. |
| `ConnectorDriverFactory` | `drivers::DriverFactory` | Hosted construction of a checked implementation object. |
| `OutboundConnectorBindings` | `driver::OutboundDrivers` | Execution's routing interface to all selected outbound Drivers, not a trait each individual Driver must implement. |
| `SessionConnectorBindings` | Remove after consolidation into `driver::OutboundDrivers` | The existing outbound routing trait already repeats its Session methods. Do not rename two redundant interfaces and keep both. |
| `RuntimeBindings` | `drivers::RuntimeDrivers` | Plan-to-Driver associations and exact exchange ownership. |

Keep `DriverRegistry`, `DriverManifest` and the concrete Driver names.
Keep `expr::Bindings` and `BootBindings`: those really are name-to-value
bindings. The semantic association called a Driver binding may still be
described as such in the specification; removing `Bindings` from these
Rust type names does not remove the association.

Implementers supply only the interfaces required by the resolved
Contract. Do not replace them with one large trait full of optional
methods. A bidirectional Driver exposes views over the **same object**;
it must not accidentally construct two unrelated Session states. The
runtime association's responsibility remains routing, correlation and
ownership, not protocol implementation.

#### 5. Exchange data and method names

| Before | Proposed after | Exact meaning |
|---|---|---|
| `Response` | `CallResponse` | A response correlated with an outbound CallId. |
| `BoundaryObservation` | `ClosedCall` | A Call closure already accepted by the journal, awaiting the Connector transaction; not an arbitrary unvalidated Driver response. |
| `RetainedProposal` | `PendingIngress` | A Driver-local handle with an ingress proposal awaiting an admission decision. |
| Private `ScriptedConnectorProtocol` | Private `CallProtocol` | Pure request/completion rules shared by real Drivers and Lab implementations, not a script-only protocol. |
| `InboundConnectorDriver::proposal` | `driver::Ingress::poll_proposal` | Poll for an incoming proposal. |
| `InboundConnectorDriver::decide` | `driver::Ingress::notify_admission` | Tell the Driver execution's admission result; the Driver does not decide admission. |
| `CallConnectorDriver::events` | `driver::OutboundCall::poll_responses` | Poll outgoing-Call responses. |
| `InboundCallConnectorDriver::events` | `driver::InboundCall::poll_completions` | Poll inbound-Call response/timeout completion observations. |
| `SessionConnectorDriver::session_events` | `driver::Session::poll_observations` | Poll Session observations. |
| `CompletionCheck::admit` | `CompletionCheck::check` | Validate a completion, without admitting or committing it. |
| `ConnectorDriverFactory::connector_id` | `drivers::DriverFactory::contract_id` | Name the implemented Contract, not a Connector Node. |
| `ConnectorDriverFactory::bind` / `bind_with_boot` | `drivers::DriverFactory::create` / `create_with_boot` | Construct prepared Driver objects under the existing no-effect-before-admission constraints. |
| `RuntimeBindings::try_new` | `drivers::RuntimeDrivers::prepare` | Resolve and check plan associations before execution/effects. |

Keep `IngressProposal`, `IngressSequence`, `IngressProvenance`,
`AdmittedIngress`, `IngressRejection`, `EnvelopeDraft`, `PreparedRequest`,
`Completion`, `CallId`, `SessionId`, `CallOpen`, `SessionOpen`,
`SessionWrite`, `DriverPoller`, `PollBatch` and `PollFault`.
`close_session`, `abort_session` and `release_session` also stay distinct:
orderly closure, cancellation and physical-resource release are different
operations. Review derived names and doc comments with their main type;
do not leave a renamed API explained using the removed name.

The private `FamilyRequirement`/`BoundFamily` names and `CrossFamily*`
routing errors need a site-by-site pass during registry restructuring:
an incompatible Contract and the wrong owning implementation are no
longer interchangeable. Do not mechanically replace both by the same
word or introduce Family as a new semantic category.

#### 6. Several implementations, without another semantic identity

| Current constraint | Target rule | Evidence needed |
|---|---|---|
| Factory uniqueness is keyed by ContractId. | The registry can describe several implementations of the same Contract and the modes each supports. `DriverName` is the proposed hosted configuration name, not a journal-derived id. | Selection refuses absence, incompatibility and ambiguity before effects. Registration order is not a fallback. |
| One Contract group has one exchange kind. | Selection also checks the resolved mode/direction/exchange for each Node. | Process job and service can coexist under one Contract without erasing their distinct lifecycles. |
| Call/Session ownership maps point to a ContractId. | Every open exchange routes back to its exact selected implementation. | Interleave two implementations of the same Contract; no response, Session write, cancellation or close crosses ownership. |
| All Nodes of a Contract share its bound group. | Associate by planned Node and permit intentional sharing of a Driver object/pool where valid. | Neither a new object per Node nor one object per Contract is imposed universally. Preserve per-instance isolation. |
| Manifest describes ContractId and roles. | Describe implementation identity and the Contract modes/capabilities it actually satisfies. | The advertised support and the object's exposed interfaces agree before execution. |

The exact selection configuration, manifest representation and private
storage layout remain to be reviewed. No new DSL attribute is invented
here. Semantic requirements are checked against the Contract; deployment
may choose among equivalent implementations. Physical selection must not
enter causal identity, and cannot weaken the Contract or change the
deterministic admission/polling order of an existing deployment. Recovery
must restore correct ownership without repeating an external operation.
No dynamic loader, stable native ABI or public execution-slot vocabulary
is needed to demonstrate this with two local test implementations.

#### 7. Self-documentation and the implementer's path

| Information to expose | Source of truth |
|---|---|
| DSL name, exact ContractId, declared modes | Contract definition and mode selector. |
| Effective ports, types, direction, exchange kind and cardinality | The same checked definitions used by compilation and independent core validation. |
| Required/defaulted attributes, domains and evaluation phase | Existing attribute declarations, not a second JSON schema system. |
| Terminal cases and whether execution or the Driver produces them | Resolved output types and Contract completion rules; no parallel error-set catalogue. |
| Meaning, limits and a small scenario per mode | Concise descriptions beside definitions and executable, tested Lab examples. |
| Check support, Lab support and installed real implementations | Distinct checks over mode support, Lab availability and the Driver registry/manifest. |
| Conformance evidence and named untested cases | The existing Driver test fixtures and results, not an assertion that every known case was exercised. |

Produce human-readable and machine-readable views from this material;
the exact CLI/export presentation is not frozen by this entry. An LLM
writing a scenario must be able to distinguish a declared-only mode from
one that actually simulates, and simulation must not require an absent
physical Driver. Documentation does not become a second type system or
runtime authority.

The contributor path is: read the standard catalogue → choose an existing
Contract or specify a new one → implement only its required interfaces →
register the implementation → supply its Lab and conformance fixtures →
run the documented checks. For an existing Contract, a new physical
implementation should not require a new DSL name or editing `Execution`.
For a **new Contract**, current M1 still requires portable core rules and
Registry-free validation: the new guide must show that work, not imply
that a hosted descriptor dynamically teaches the core new semantics.

#### 8. Review before implementation

Fable is asked to check the proposal against the actual Contract, Driver,
registry and Lab code, with particular attention to fewer concepts and
smaller public interfaces. Concrete remaining questions:

1. Is the per-Driver / all-Drivers distinction clear, particularly
   `driver::Driver` versus `driver::OutboundDrivers`, and is any proposed
   wrapper or split unnecessary?
2. What is the smallest checked mode representation that unifies Process
   identity without weakening Call/Session, Closing, Loop or Retry rules?
3. Which metadata is necessary to select and correlate two implementations
   of the same Contract, and which can remain private instead of becoming
   another public type?
4. Can the catalogue and core rules stay consistent without copying
   semantic definitions or putting hosted descriptions into embedded
   packages?
5. Which changes are strictly byte-neutral and which need an explicit
   specification/identity inventory? Name each affected witness before
   changing it, not after a failing test.

This entry requests review, not automatic invocation of another model.
There is no implementation or qualification claim yet. The semantic
specification remains authoritative until a reviewed amendment lands;
the mechanical phase cannot silently perform the Process Contract
movement. The action ledger owns the pre-freeze sequence and its gates.

### Applied — 22 October (Astra), byte-neutral T5/T9 series

| Action | Commit | Applied result |
|---|---|---|
| A-T5-1 | `491e397` | Execution terminology in specification, code comments and UI. |
| A-T5-2 | `55155ca`, `e2d980c` | Shared portable Contract definitions; exchange and Driver interfaces; execution-owned polling; hosted `connector/drivers` modules and split registry/runtime responsibilities. |
| A-T5-3 | `958d0bb` | One outbound routing interface; per-object Call/Session views retained; no global blanket Call-to-routing implementation. |
| A-T5-4, preparation only | `eabdee9` | Decoded identity inventory and proposed amendment; current Process ids and package format unchanged. |
| A-T5-5 | `0b0ad9d` | Named implementations selected per Node, unique-compatible default, exact object ownership and routing reconstruction from a recovered prefix. |
| A-T5-6 | This catalogue/documentation commit | Human/JSON catalogue from checked definitions and manifests, File contributor guide, live-document and diagram updates. |

The final gate records **1,620 passed, 0 failed, 0 ignored in
61 groups**; fmt, strict Clippy and rustdoc pass. Site: 12/12;
browser: 35/35; all three no_std targets and both QEMU workloads pass.
Echo peak heap/code: 111,404/971,716 B. Retry with
recovery: 129,188/1,043,716 B, leaving
1,884 B heap and 4,860 B code.
All 73 protected artifacts remain byte-identical. Commands, intermediate
failures and exact measurements are in
[connector-rework-qualification.md](connector-rework-qualification.md).

**Open, not implemented:** A-T5-4's Process Contract id, proposed decision
and package-version choice require Yoann's approval of
[the identity inventory](connector-rework-identity.md). No identity-moving
commit, compatibility alias, public push, tag or publication is included.
No new Fable review is claimed by this Applied entry; the existing review
and Yoann's 21 October decisions above remain intact.

### Applied — 22 October (Astra), approved Process identity amendment

Yoann approved A-T5-4's inventory (`eabdee9`), `std/process@6`, package
format 22 and the proposed amendment after the neutral series (`3ca0867`).
D-2026-10-22-1 records that decision. This identity commit implements it:
one Contract, with job/Call and service/Session resolved from the shared
mode definition by both the compiler and independent core validator.
Implementation selection still distinguishes the two physical resources;
neither mode gains new ports, codecs or Loop/Retry behavior.

The [identity inventory](connector-rework-identity.md) records measured
old/new values and reasons before the single regeneration pass. Six of
73 protected artifacts move; 67 remain identical. Gauge's body and semantic
hash are unchanged. The only causal trace changes are the two Process
Retry scenarios, whose opening facts bind the originating plan and Contract.
Recording remains version 4. There is no old-id alias or package-21 reader.

The complete Rust suite passes **1,623 tests in 61 groups, zero failures and
zero ignored**; fmt, strict Clippy and rustdoc pass. Site: 12/12; browser:
35/35; three no_std targets and both QEMU workloads pass. Echo peak heap/code:
111,404/971,428 B. Retry with recovery: 129,188/1,043,452 B, leaving
1,884 B heap and 5,124 B code. Commands, intermediate failures and final
measurements are in the [qualification](connector-rework-qualification.md).
This entry supersedes the neutral checkpoint's pending approval, not its
historical measurements. It claims no new Fable replay and authorizes no
push, tag or publication.

### Reviewed — 22 October (Fable)

Tables 3, 4 and 5 are applied in both crates; no retired name survives in
`crates` or the living documents except the derived names below, and
`decisions.md` and the dated M1.5 rows of `spec-to-code.md` keep theirs as
history. Two complements for Yoann:

| Name | Status | Note |
|---|---|---|
| `ProfileProtocol`, `ContractMode::profile`, `ContractMode::owning_profile`, parameters named `profile` | ⟂ residual derived names of the retired `Profile` word, 27 sites in nine files | Proposed `ModeProtocol`, `mode`, `owning_mode`; zero bytes; action A-T5-7 in the action ledger, lot to decide. |
| `RuntimeDrivers::prepare_with_selection`, `RuntimeDrivers::restore_ownership`, `OutboundDrivers::forget_session`, `DriverRecoveryError` | new public names, not in the tables | Apt. `forget_session` is a fourth verb beside `close_session`, `abort_session`, `release_session`, on the routing interface only: it releases routing state after a terminal commit. Recorded, no change proposed. |

The appendix inventory of public types is still anchored on `fedada7` and
predates the rename lot, the realignment and this rework; re-anchoring it is
Fable's debt.

### Residue sweep — 22 October (Fable), the finishing lot for Astra

Yoann asked for one complete pass over every retired term, in code,
comments and documents, after the Connector rework landed. Method: every
"before" word of the three rename sources (the 12 October lot, the
realignment and the Connector rework tables) was searched on `4c912eb` with
word boundaries over the two crates, the spike sources, the UI scripts,
the specification, the living documents and the dated documents; each hit
was read for its sense. Dated reports and `decisions.md` keep their names by
rule and are not counted below; the two review registers are excluded. The
scripted sweep is reproducible from the terms listed here.

The pattern is the same each time: the types were renamed, the words derived
from them (locals, fields, parameters, helper names, prose) were not. Nothing
below moves a byte; every item is a `cargo test` away from proof.

**To finish, in code (A-T5-7, owner Astra, zero bytes).**

| Derived residue | Sites, approximate | Proposed |
|---|---:|---|
| `vm` as the name of `Execution` values (locals, fields, parameters), from the realignment's `Vm` → `Execution` | 1 678 lines, of which 210 in `src` (`ui/view.rs`, `conformance.rs`, `network/http_server.rs`, `drivers/process`, `runtime/execution/advance.rs`, `compiler.rs`, `main.rs`, `recording`) and the rest in integration tests (`execution.rs` 659, `retry_runtime.rs` 201, `v16_loop.rs` 150, `v16_recovery.rs` 90, `multiport.rs` 70) | `execution` |
| `bindings` as the name of `RuntimeDrivers` / `OutboundDrivers` values, from `RuntimeBindings` → `RuntimeDrivers` | 349 code lines in the Driver, execution and poller files (`runtime/execution.rs` 78, `drivers/runtime/tests.rs` 53, `worker.rs` 31, `conformance.rs` 30, `http_server.rs` 21, `driver_polling.rs` 19) plus 38 comments | `drivers` |
| `Execution::step_with_bindings`, `run_ready_with_bindings`, `recover_with_bindings`; `lab::mock_bindings` | 240 call lines, mostly tests (`execution.rs` 87, `retry_runtime.rs` 60, `v16_recovery.rs` 25) | `step_with_drivers`, `run_ready_with_drivers`, `recover_with_drivers`, `mock_drivers`. `Dispatch::fire_with_bindings` stays: those are expression bindings. |
| `NoOutboundBindings` (the empty routing table in `execution.rs`) | 5 | `NoOutboundDrivers` |
| `conformance::bind` and `recording::replay::Contracts::bind`, from `ConnectorDriverFactory::bind` → `create` and `RuntimeBindings::try_new` → `prepare` | 8 call sites | `prepare` |
| `ProfileProtocol`; `ContractMode::profile`, `owning_profile`; `ModeSelector::profiles`; locals and parameters named `profile` in the Contract-mode sense (`connector/contract.rs` both crates, `connector/stdlib`, `compiler/check.rs`, `compiler/lower.rs`, `connector/call.rs` and its tests, `tests/contracts.rs`, `conformance_process.rs`, `process_service.rs`) | about 200 lines | `ModeProtocol`, `mode`, `owning_mode`, `modes`, locals `mode` |
| `profile` as the name of `DriverTestFixture` values in `conformance.rs`, from the 12 October `conformance::Profile` → `DriverTestFixture` | 56 lines | `fixture` |
| `RuntimeDrivers::decide_inbound`, from `InboundConnectorDriver::decide` → `notify_admission` | 6 | `notify_inbound_admission` |
| `MockOutboundConnectorDriver` in `lab.rs`, the retired `ConnectorDriver` suffix | 8 | `MockOutboundDriver` |
| `connector_id` parameter of the WebSocket binder in `drivers/websocket_protocol.rs` | 3 | `contract_id` |

**To align, in the specification and the living documents (same lot).**

| Where | Sentence | Note |
|---|---|---|
| `stdlib.md` §3.3 and §3.3's service paragraphs | "required for both profiles", "The resident Session profile", "`correlate` is not part of this profile", "The profile uses D-2026-08-25-1's generic Session exchange", "A profile…" (A4 boot references); §3.7 "both profiles consume exactly one `in`" | the Contract-mode sense becomes "mode". "Application profile", "idempotence profiles", "inbound/event profile", "Retry profile" are specification concepts and stay. |
| `grammar.md` | "Unsupported profiles still refuse by name" (status block); the grammar comment "`profile`, `behavior` and `idempotent` are ordinary Contract attribute names" | the first becomes "modes"; the second is to check against the example it annotates, since the selector attribute is `mode`. |
| `loop.md` | "Process service profile" (table), "resident-service profiles refuse" | "mode". "resolved application profile" and "host profile" stay. |
| `errata.md` E-2026-09-14-2 | "Where: `runtime/vm.rs`" | dated entry; add "(now `runtime/execution.rs`)" rather than rewriting. |
| `components.md` §4 item 5 | "boundary/Driver bindings" | "Driver interfaces and the runtime Driver association". |
| `components.md` §3.4 | `recover_with_bindings` | follows the method rename. |
| `parallelism-notes.md` | "the existing one-Worker VM" | "Execution". |
| `simulation-debugger-explanations.md` | "The VM …", "several VM transactions" | "Execution". |
| `spec-to-code.md` | the historical M1.x rows keep `VM`, `boundary::`, `connectors::`, `RuntimeBindings` | add one sentence to "Keeping the map honest": historical rows keep the names of their time; the current table uses current names. No row rewrite. |

**Kept, read and not residues.** `Obligation::LoopDelay` and `RetryDelay`
(two obligations, decided 12 October); the hosted network `pump` and its
prose in `stdlib.md`, `delivery-semantics.md`, README and WHITEPAPER (the
M1.6 engine, distinct from the retired `Vm::pump`); `mint_capability` and
"minted" for the debugger's authorization token (not envelope identity);
`openai-responses-hydration.md` as a file name and the `hydration` anchors
in `annotations.md` and `decisions.md` (stable link targets); `std/process@4`
and `@5` in the refusal tests of `process_service.rs` and `plan_package.rs`
and in the identity inventory; "application profile", "deployment profile",
"experiment `profile =`", `json_profile`, "privacy profile" and "cost
profile" in prose (specification concepts; the renamed types are
`PrivacyDeclaration` and `PriceBook`); `BootBindings`, `expr::Bindings`,
`LazyBindings`, `LocalBindings`, `PlanValueBindings` and
`fire_with_bindings` (name-to-value bindings); "Driver binding" in seven
code comments and two specification sentences (the association, allowed by
the 12 October rule); `Fold` as a verb in two comments; "Packet algebra" in
`loop.md`; `DataSlot`; `hir::EndpointOwner`, `hir::RouteDestination` and
the other HIR names (kept in block); `Draft` as a user type in `loop.md`'s
example; `TcpListener::bind` and `UdpSocket::bind`; `try_new` on types other
than the Driver association. The following retired names have **zero**
occurrences anywhere outside dated documents: every "before" name of the
Connector rework tables except `ProfileProtocol`; `IngressSubmission`,
`BoundaryEvent`, `CompletionGate`, `Dispatcher`, `PumpBatch`,
`ConnectorRoles`, `*RequestBindings`, `HandlerExecutor`, `PlanEmission`,
`DatedPayload`, `AccountingObservation`, `CostProfileName`, `Provocation`,
`unprovable`, `ScenarioSession`, `LoopExpiry`, `LoopTermination`,
`install_target`, `admit_ingress`, `inject_submission`, `VM`, `Vm`,
"virtual machine".

**Gate for the lot.** The four checks, the full suite, `site_content` 12/12,
the three `no_std` targets; every package, trace, recording and spike witness
byte-identical; the counts above at zero on re-run of the sweep. Fable
replays and records "applied" here.

### Applied and replayed — 22 October evening (Astra `819a2ff`, Fable)

Astra applied the finishing lot in one commit, `819a2ff`, on top of
`590d070`: 83 files, 72 of them Rust, +4 020 / −3 096 lines, most of the
volume being rustfmt re-wrapping after the longer identifiers. Astra's own
qualification, with the Haiku 5.5 read-only pass it credits, is the
[A-T5-7 section](connector-rework-qualification.md#a-t5-7--finish-derived-names-comments-and-living-documentation)
of the Connector rework qualification. Neither review register was edited by
that commit; this entry is Fable's replay.

**Replay, same checkout.** `cargo fmt --check`, strict Clippy and rustdoc
pass. `cargo test --workspace --all-features`: 61 groups, 1 623 passed,
0 failed, 0 ignored. `site_content` 12/12. The three `no_std` targets
check. QEMU Cortex-M33: echo heap 111 404 B, code 971 428 B (`.text`
897 420 + `.rodata` 74 008), hash `7106f322…`; Retry with recovery heap
129 188 B, code 1 043 460 B (`.text` 958 676 + `.rodata` 84 784), hash
`3cad77ef…`, the Retry package regenerated on `819a2ff` byte-identical to
the one generated on `974f85f`. Against `590d070`: echo unchanged; Retry
`.rodata` +8 B, which is the one `assert!(execution.is_stable())` in the
spike's `retry.rs` whose panic text embeds the renamed identifier. Margins:
heap 1 884 B, code 5 116 B.

**Byte neutrality.** The commit touches `.rs` and `.md` files only: no
package, trace, recording, SVG or other witness; no 64-hex literal appears
on any line of the diff.

**How the diff was read.** Each old/new pair of the inventory (`vm` and
`execution`; `bindings` and `drivers`; `profile`, `mode` and `fixture`;
`decide_inbound` and `notify_inbound_admission`; the two mock Driver names;
`connector_id` and `contract_id`; `bind` and `prepare`) was mapped to one
placeholder on both sides, and the token streams were compared with
whitespace ignored. Over 72 Rust files, what remains is: commas and closure
braces moved by rustfmt; the struct-literal shorthand `execution`; the
conformance parameter `driver_registry`, which now distinguishes the
available registry from the selected `drivers`; the local `mode_word` in the
Retry checker; four test names that contained `vm`; the constant `PROFILES`
→ `MODES` in `tests/contracts.rs`; and eleven diagnostic strings where
"profile" becomes "mode" or "fixture" (the `Display` impls of
`connector/contract.rs`, the selector refusal of `compiler/check.rs`,
`retry_control.rs`, the File `watch` deferral reason, the conformance
fixture violations). Comments: the rename plus six rewordings read one by
one, among them the stale "does not admit the deferred Process service
profile" in `tests/execution.rs`, now "independently of Process service
mode", which is correct since `std/process@6`. No executable change.

**Sweep re-run on `819a2ff`.** The script was extended with lower-case
`vm`, which the 22 October version did not pattern; Astra noticed the gap
and swept it anyway. Code: `vm` 0 lines (from 1 678); `*_with_bindings` and
`mock_bindings` 0 apart from `fire_with_bindings`, kept; `NoOutboundBindings`,
`ProfileProtocol`, the `ConnectorDriver` suffix, `connector_id` and
`decide_inbound` 0. `bindings` falls from 332 to 181 lines and `profile`
from 304 to 134; every remaining line was read: name-to-value bindings
(`expr`, `dispatch`, `BootBindings`, `data_policy`, the `FileBinding` and
`ProcessBinding` maps) and the deployment, price, JSON, schema, lifetime,
exchange and Call-semantics profiles. No new hit appears anywhere.

**Second layer, found at replay and absent from the 22 October inventory —
Fable's gap, not Astra's.** All zero bytes.

| Where (`819a2ff`) | What | Proposed |
|---|---|---|
| `drivers/file/mod.rs:350`, `drivers/process/mod.rs:502`, `drivers/process/service.rs:59` and `:198`, `drivers/network/http_client.rs:288` | fields and parameters named `bindings` holding `FileRequestSettings`, `ProcessRequestSettings`, `HttpClientRequestSettings`: the 12 October `*RequestBindings` → `*RequestSettings` rename without its derived word, about 40 lines. The `bindings: BTreeMap<Path, FileBinding>` maps beside them are named correctly. | `settings` |
| `tests/connector_composition.rs:253` | panic text "cannot create bindings for WebSocketClient" | "cannot create Drivers for" |
| `identity.md:28` | "the profile selector's word included" | "mode selector" |
| `delivery-semantics.md:512` | "an inbound Call `respond` profile" | "`respond` mode" |
| `types.md:386` | "a type selected by a profile or attribute" | "by a mode or attribute" |
| `simulation-semantics.md:200` | "unmodeled source profiles refuse at check" | "source modes" |
| `WHITEPAPER.md:1539` and `:1562` | "the resident Process service profile are sessions"; "The resident Process profile uses one Session" | "service mode"; body text, outside the verbatim narrative |
| `components.md:274` | "Contract profiles" in the `ValidatedExecutionPlan` row | "Contract modes" |
| `plan-package.md:131` | "the two Process profiles" | "the two Process modes" |
| `opening-showcase.md:683` | "the executable job profile" | "job mode" |
| `process-service.md:27–28` and `:116` | "resolved profile", "profile/catalogue identities", "codec/profile refusals" | "mode"; a dated ledger with a current pointer, so annotate or amend, Yoann's call |

Kept on reading: "exchange profile" (Event, Call or Session:
`components.md:205`, `simulation-semantics.md:122`, `ui.md:306`, the
recovery messages "contradicts its Contract profile"); "finite Call profile"
and "host profile" (R1, `loop.md:69`, `plan/call_loop.rs`, `plan/retry.rs`,
E-LOOP-HOST); "lifetime profiles"; `json_profile`; "lowering profile";
"external schema profile"; the pre-v16 sketches of `dsl-examples.md`
("Prospective append/log profile"), preserved as historical by that file's
own notice.

**Observations for T10, not residues.** The catalogue human view labels the
`lab_models` key "Lab bindings" (`connector_catalogue.rs:562`, asserted by
`cli.rs:1606`); the label should follow the key. The empty
`crates/rastok/src/connectors/{file,network,process}` directories from the
21 October move are still on disk, untracked; `git` does not see them.

**Status.** A-T5-7 applied and reviewed. The second layer is A-T5-8 in the
[actions register](human-review-m1.md); whether it joins a lot before the
freeze or waits for v21 is Yoann's decision.

### Applied and replayed — 23 October (Astra, three commits; Fable)

Astra delivered three commits on top of `20086f6` during the night of 22 to
23 October; Fable replayed the gate on the resulting `3736817` in the same
checkout on the morning of 23 October. Neither review register was edited by
those commits; this entry is Fable's replay.

**The three commits.**

- `ff0227d` "Align request settings and Contract mode vocabulary" is A-T5-8:
  17 files, +84 / −52. The 39 `bindings` lines holding `*RequestSettings`
  values become `request_settings` (File 6, the scripted-protocol factory
  closure of `drivers/http_client.rs` 1, HttpClient 11, Process 6, resident
  Process 9, the Lab twin 5, the panic text 1), and the eleven sentences read
  "mode" (twelve lines, `process-service.md` spanning two). The fields are
  `request_settings`, not the table's `settings`, because `prepare` in
  `drivers/process/service.rs` already takes a `settings` parameter. Astra
  kept "are unchanged" in `opening-showcase.md:683`: the subject is compound
  ("Other Contracts' correlators and the executable job mode"), and the
  brief's singular was wrong. The empty `connectors` directories are gone.
  Astra's qualification:
  [A-T5-8 — applied](connector-rework-qualification.md#a-t5-8--applied).
- `2a1cd11` "Align Loop and Retry fact and manager names": 31 files,
  +230 / −227. Facts in the past participle in both protocols (`Opened`,
  `Advanced`, `Admitted`, `Closed`; `Expired`, `Held`, `Consumed` and
  `Published` already were), `loop_view::RunState` → `Run`,
  `check_node_manager` → `check_loop_manager`, `retry_admission_load`
  extracted from `recurrence_admission_load`, which is now the sum of the
  three loads, `retry_attribute` → `is_retry_attribute`. The specification's
  words follow in `identity.md:280`, `:315` and `:331`, `loop.md` and
  `ui.md`. Integer tags, canonical forms, the UI transport labels and the
  dated documents are untouched.
- `3736817` "Use E-RETRY for general Retry diagnostics": `E-RETRY-ATTRIBUTE`
  → `E-RETRY`, the general Retry policy code now named like `E-LOOP`; five
  files; zero residue of the old code in `crates/`, `docs/`, README,
  WHITEPAPER and CHANGELOG. Fable had listed this public-code asymmetry as
  outside the lot; Yoann took it directly, so the commit carries Astra's
  trailer alone.

**Why Loop and Retry, and what the reading found.** On the evening of
22 October Yoann asked whether the two temporal Extensions were symmetric in
types and names. Layer by layer they are: `node::Extension`;
`ExtensionSpec::{Loop, Retry}` with `Node::loop_spec()` and `retry_spec()`;
`hir::LoopSpec` and `hir::RetrySpec`; the shared `DelayPolicy` and
`PlanConstant` bounds; the `IdKind` pairs `0x03`/`0x04` and `0x08`/`0x0a`;
the `loop_protocol` and `retry_protocol` modules with `LoopRecord` and
`RetryRecord`, `Fact`, `CloseReason`, `Phase`, `Error`, `LoopView` and
`RetryView`, `run_ref` and `operation_ref`; `loop_manager.rs` and
`retry_manager.rs` with `step_*_manager`, `*_manager_ready` and
`manager_record`; the three `plan/{call_loop, graph_loop, retry}.rs::validate`;
`check/loop_control.rs` and `check/retry_control.rs`; `Obligation::LoopDelay`
and `RetryDelay`. The content differences are the specification's: Loop has
commands (`LoopCommand`, `LoopCommandTarget`, `LoopCommandKind`,
`Destination::LoopCommand`, `CommandId`, `CommandValue`, `CommandProduced`,
`DoneKind`, `IdKind::LoopCommand`, the three statements) and Retry none;
Retry has `Pin`, `Policy`, `Batch`, `AttemptId`, `Opening` and `Settlement`
and Loop none; run / application / iteration from zero against operation /
attempt / ordinal from one, both specified. The naming asymmetries were the
lot above.

**Left in place, deliberately.** `LoopRunId`, prefixed and in `journal.rs`,
beside `OperationId` and `CommandId`, bare in their protocols: kept by the
11 October table. `l:next` (`Lifecycle::Next = 3`) for the Loop manager
transaction against `l:retry` (`Lifecycle::Retry = 10`): a command's name
against an Extension's. Correction, 23 October midday: the spelling is the
sealed material, not the byte. The trigger enters the Emission recipe as
UTF-8 text (`identity.md` §2 and `envelope.md` §1.1, `id = blake3(node_path
‖ trigger ‖ parent_id ‖ port ‖ index)`) and the TRANSACTION record's key 19 as
text; the `Lifecycle` byte serves only the plan stream's `HandlerActivation`,
where only `start` and `stop` occur. Renaming `l:next` is therefore a format
change, not zero bytes; Yoann's call for v21 or never — decided on 23 October at
midday, see [the `l:` registry](#decided--23-october-midday-yoann-the-l-registry)
below. The UI transport labels `l:open`, `l:admit`, `l:advance`, `l:close`,
`l:expired` and `retry-advance` carry the former tenses; they are asserted by
`browser.test.cjs` and described in `ui.md`, T10. `LoopRepeatWarning` lives
in `plan/retry.rs`; Loop runs are projected inline in `ui/view.rs::loop_runs`
while Retry has `ui/view/retry.rs`; the tests are `v16_loop*.rs` against
`retry_*.rs`: placement, T10. `UiCommand::RetryStep` and `ui.md:74` use
"retry" for re-executing a faulting transaction, a homonym of the Extension
that the 11–12 October tables did not see. Loop has `loop.md`; Retry's
contract is one paragraph of `extensions.md`, key 43 of `identity.md` and the
operation algebra in `retry-c1-algebra.md`, and the four `E-RETRY*` rows of
`diagnostics.md` link to `retry-extension-refactor.md` where the `E-LOOP*`
rows link to `loop.md`. That last asymmetry is documentation, the largest,
and Yoann's; a `retry.md` would be an extraction, not new writing.

**Replay, same checkout, `3736817`.** `cargo fmt --check`, strict Clippy and
rustdoc pass. A first `cargo test --workspace --all-features` stopped after
29 groups when `tests/member_presence.rs` returned neither a result nor a
failure, the start-up stall Astra recorded in the A-T5-8 qualification; run
alone it passes 4/4, and the complete suite with `--no-fail-fast` gives
61 groups, 1 623 passed, 0 failed, 0 ignored, `site_content` 12/12 among
them. The three `no_std` targets check. Browser suite 35/35 against the
cached Chromium headless shell 1243 driven by the Playwright core that `npx`
cached (revision 1248 expected); the globally installed Playwright 1.49
expects revision 1148, and driving the installed Chrome with it fails six
tests on `page.waitForFunction` with a string predicate, refused by the page's
`default-src 'self'` policy, which is tooling, not code. QEMU Cortex-M33: echo
heap 111 404 B, code 971 428 B (`.text` 897 420 + `.rodata` 74 008), hash
`7106f322…`, output identical to `819a2ff`; Retry with recovery heap
129 188 B, code 1 043 492 B (`.text` 958 676 + `.rodata` 84 816), hash
`3cad77ef…`, output identical, the package regenerated on `3736817`
byte-identical to the `819a2ff` and `974f85f` ones. Against `819a2ff`:
`.rodata` +32 B, the `Debug` strings of the renamed variants linked into the
spike, as the brief anticipated. Margins: heap 1 884 B, code 5 084 B.

**Byte neutrality.** `20086f6..3736817` touches `.rs` and `.md` files only;
no 64-hex literal appears on any line of the diff.

**How the diff was read.** The 22 October token normalization was extended
with the new pairs (`bindings` / `request_settings`, the four fact tenses,
`RunState` / `Run`, `check_node_manager` / `check_loop_manager`,
`retry_attribute` / `is_retry_attribute`, `E_RETRY_ATTRIBUTE` / `E_RETRY`).
Over 36 Rust files what remains is: thirteen refusal and assertion strings
that spell the new fact names ("a Loop Opened has no exact queued admission",
"Retry Advanced must name the following attempt", "Advanced is not
Admitted"), six comments reworded for the participle, the
`retry_admission_load` extraction, the `E-RETRY` literals, the panic text and
rustfmt brace moves. No executable change beyond the extraction, which keeps
the saturating sum.

**Sweep re-run.** Against the `819a2ff` detail, ignoring line shifts:
`bindings` −39 / +1, the 39 being exactly the inventory's sites and the one
addition the prose of Astra's qualification paragraph; `profile` −12 / +0,
the eleven sentences; old fact names, `RunState`, `check_node_manager`, bare
`retry_attribute`, `E-RETRY-ATTRIBUTE` and `bindings` typed `*RequestSettings`:
0. The remaining upper-case `VM` lines in living documents were read:
`spec-to-code.md`'s historical rows and B9 narrative, `build-order.md` and
`process-service.md` under their 22 October current-implementation notices,
and the two qualification reports narrating the retired word. Kept.

**Status.** A-T5-8 applied and reviewed. The Loop/Retry lot is A-T2-12 in
the [actions register](human-review-m1.md), applied and reviewed. Open for
Yoann: `l:next`, a `retry.md`, the UI transport labels and the placements,
all listed above.

### Decided — 23 October midday (Yoann), the `l:` registry

At midday on 23 October Yoann asked what the `l` stood for, then whether
`l:next` served Loop and Retry alike, then the difference between Next and
Again, then for a cleaner registry independent of the freeze. He ratified the
proposal and the findings below as a rework lot for now; the brief is Astra's.
This section records what the reading established and what was decided.

**What the reading established.**

- `l` is lifecycle. The TRANSACTION `trigger` is `p:<port>` or
  `l:<lifecycle>` so that port and lifecycle names never collide
  (`envelope.md` §1.1). The word enters the Emission recipe as UTF-8 and key
  19 as text; the `repr(u8)` byte serves only the plan stream's
  `HandlerActivation`, where only `start` and `stop` occur. Only `on start`
  and `on stop` are authored (grammar `trigger ::= "start" | "stop"`); the
  other eight are internal runtime triggers.
- `l:next` is not the re-admission trigger but the trigger of every Loop
  manager transaction. The eight `manager_record` calls
  (`loop_manager.rs:193, 244, 510, 541, 764`, `graph_loop.rs:467, 573, 877`)
  carry Opened at opening, initial and recurrent Admitted, Advanced, Closed
  and Expired, and the `LoopExpired` emission takes `l:next` in its id recipe
  (`loop_manager.rs:734`). `envelope.md:356` titles the row "Loop
  re-admission" and `loop.md:233` says "no re-admission and no `l:next` are
  created" at expiry: the prose is narrower than the code. The code matches
  `l:retry`, which already means "the Retry manager acted" (`carriers.rs:11`).
- Next and Again are both necessary and both stay. Next carries a typed value
  that becomes the next state; Again carries nothing, retains `Loop.state`,
  accepts any causal source, and is the one `W-LOOP-REPEAT` watches. Both
  consume one fuel unit. `CommandValue::{Next, Again}` and `Advanced.command`
  already record which one was consumed, so a separate `l:again` would
  duplicate the journal and break the rule `l:stop` and `l:closing` already
  follow: one word per mechanism, `parent_id` discriminates the cause.
- `l:retry` is allocated (`envelope.md:168`) but no specification table has a
  row for it. Its allocation lives in `retry-c1-identity.md` ("future
  activation tag 11"); D-2026-08-27-2 says "`l:exit` (tag 10)"; the code has
  carried Exit = 9 and Retry = 10 since `8bc54fd` (format 20, D-2026-10-07-1).
  No artifact is affected: the specification simply published no byte table.
- The debugger's `l:open`, `l:admit`, `l:advance`, `l:close` and `l:expired`
  imitate the registry (`ui/view.rs:1043–1047`, `ui.md:192`); `l:close`
  collides with the new word below.
- `node::Lifecycle` declares its variants out of byte order (`Retry` first).

**Decided (Yoann, 23 October).** One rule: the word after `l:` names the
emitting mechanism as the specification names it, in one bare word; never a
command, a fact or a phase; `parent_id` carries the cause; the `l:` prefix is
reserved to the registry.

| Byte | Before | After | Emitting mechanism |
|---|---|---|---|
| 1 | `l:start` | `l:start` | handler `on start` |
| 2 | `l:stop` | `l:stop` | handler `on stop` |
| 3 | `l:next` | **`l:loop`** | the Loop manager, every transaction |
| 4 | `l:timeout` | `l:timeout` | Call budget expiry, the DSL word `timeout =` |
| 5 | `l:evict` | `l:evict` | join-buffer eviction |
| 6 | `l:recover` | `l:recover` | recovery resolution |
| 7 | `l:closing` | **`l:close`** | durable instance close request |
| 8 | `l:reply` | `l:reply` | inbound Call terminal |
| 9 | `l:exit` | `l:exit` | Session terminal |
| 10 | `l:retry` | `l:retry` | the Retry manager, every transaction |

Bytes do not move; the registry remains extend-only. `envelope.md` §3.1
becomes the single authority with a byte column and the `l:retry` row;
`execution-model.md:702` is retitled; `loop.md:233` is corrected. Debugger
labels for Loop facts drop their `l:` and show `opened`, `admitted`,
`advanced`, `closed`, `expired`. The spelling is sealed material, so the lot
carries package format 23 (D-2026-10-23-1, written by Astra); diagnostic
recording stays 4. Exactly four tracked artifacts embed either word and move
by content: `gallery/agent-gateway/app_server.two_conversations.trace`,
`gallery/agent-gateway/scripted_model.alice_and_bob.trace`,
`gallery/two-tenants/ui_tour.lifetime.trace`,
`gallery/webhook-to-partner/webhook.duplicate.trace`; the four `.rkp`
(`echo`, `gauge`, `jsonl`, the spike's `echo`) move by header only.

Rejected: `l:again` (above); `l:recur`, because "recurrence" is the umbrella
of both managers in `recurrence_admission_load`; a third namespace for the
internal triggers, because the authored/internal distinction lives in the
grammar, not in the name. Not touched: the seven other words; `l:stop`'s two
causes (a specification choice, `parent_id` distinguishes them); the
Closing-named Rust identifiers, which name the phase; the dated documents;
`retry.md`; `UiCommand::RetryStep`.

**Status.** A-T2-13 in the [actions register](human-review-m1.md): applied by
Astra in `0413fdf` the same afternoon and replayed by Fable; the record follows.

### Applied and replayed — 23 October afternoon (Astra `0413fdf`; Fable)

Astra delivered one commit, `0413fdf` "Align lifecycle triggers with their
emitting mechanisms", on top of `d38ff9f` on the afternoon of 23 October:
49 files, +531 / −173, both trailers, signed off. Fable replayed the gate on
it in the same checkout the same afternoon. Neither review register was
edited by the commit; this entry is Fable's replay.

**What the commit does.** `node::Lifecycle::Next` → `Loop` and `Closing` →
`Close`, the variants now declared in byte order; `Display` and the codec's
text table follow (`codec.rs:778–787`); the wire-registry test pins the word
and the `l:` form beside the ten bytes and the 246 refused ones. Every Rust
site moves with it: `execution.rs` (the close request), `graph_loop.rs`,
`loop_manager.rs` (the `LoopExpired` emission id and `manager_record`),
`recovery.rs`, `instance_stop.rs`, `loop_view.rs`, `golden.rs`, nine test
files. A new codec test round-trips the ten current words through the record
tuple and refuses `l:next`, `l:closing` and the four former UI pseudo-triggers
as `Invalid("transaction trigger")`; its Retry fixture carries the carrier the
decoder also checks. `FORMAT_VERSION` 22 → 23, the three version pins and
`plan_package.rs`'s header byte `0x16` → `0x17`; the older-version refusal
loop covers 22 by construction. Debugger: Loop fact labels `opened`,
`admitted`, `advanced`, `closed`, `expired` in history and in the future list,
kind `loop-next` → `loop-manager`, verb "Loop continued" → "Loop manager",
the future lifetime deadline spelled `l:close`; `browser.test.cjs:500`
asserts `admitted`. Specification: `envelope.md` §3.1 is the table with the
byte column, the `l:retry` row and the rule stated above it; §1.1's list,
`execution-model.md` §10 (retitled, with a Retry row and a pointer to §3.1),
`loop.md:233` ("no re-admission is created. The manager's `l:loop`
transaction retains Expired"), `instances.md`, `identity.md` header and
key 35, `extensions.md`, `simulation-semantics.md`, `ui.md`. Living
documents: CHANGELOG, ROADMAP (a new 23 October paragraph; the 22 October
one keeps "format 22" as history), WHITEPAPER header and §10,
`plan-package.md` (whose diagram said 21), `diagnostic-recording.md`,
`opening-showcase.md`, `docs/examples/README.md`, `spec-to-code.md` rows R5
and L5 with "then `l:closing`, `l:close` since format 23". D-2026-10-23-1 in
`decisions.md`, the inventory `lifecycle-registry-identity.md` written
before regeneration with old and new seals, and `site_content.rs` requiring
the inventory's name in the catalogue and the status pages.

**Normalized diff.** With `Next` / `Loop`, `Closing` / `Close`,
`l:next` / `l:loop` and `l:closing` / `l:close` folded, what remains over the
Rust files is: the version constant and its four pins, the enum reorder,
three doc comments, the new codec test, the two `site_content` assertions and
the UI label strings. No executable change beyond the renames.

**Byte audit, inverted control.** The repository tracks 36 traces and
packages. Exactly eight change between `d38ff9f` and `0413fdf`, the eight
the brief named: `app_server.two_conversations`,
`scripted_model.alice_and_bob`, `ui_tour.lifetime` and `webhook.duplicate`,
each by one record (key 19 `l:closing` → `l:close`, text length 9 → 7) and
its seal, no emission id and no other record touched; `echo`, `gauge`,
`jsonl` and the spike's `echo.rkp` by the header byte and the checksum,
bodies and semantic hashes unchanged, the spike binary byte-for-byte the hex
`echo.rkp`. Each changed trace holds one close request and no Loop manager
record; the `l:loop` identity change is exercised by the protocol and
runtime tests, not by a gallery fixture. The 28 other witnesses are
byte-identical. The Retry spike package regenerated on `0413fdf` differs
from the `3736817` one by the header byte and the checksum
only, 33 bytes; its semantic hash `3cad77ef…` is
unchanged.

**Residue sweep.** `l:next`, `l:closing`, `Lifecycle::Next`,
`Lifecycle::Closing`, `loop-next`, "Loop continued" and the five
pseudo-triggers: zero in `crates/` outside the new test's refusal list; in
`docs/`, only the dated documents (`instance-stop-command.md`,
`m1-slice.md`, `m1.4-instances-backlog.md`, `m1.7-*`, `retry-c1-identity.md`,
`retry-c3-implementation.md`, `rework-r0-*`, `rework-r2-supersession.md`),
the historical `sim-03` derivation under its pre-v16 notice, the two
`spec-to-code.md` history rows, and twelve lines of `decisions.md` before
today's entry. Kept, by the rule for historical sections.

**Two precision remarks on the new authority.** The Loop row's cause column
in `envelope.md` §3.1 says "the consumed Next/Again command (Advanced,
recurrent Admitted, Expired …)". In the code the recurrent Admitted
(`loop_manager.rs:541`, `graph_loop.rs:573`) takes as cause the re-admitted
state envelope's id, whose `parent_id` is the command's cause
(`loop_view.rs:393`); only Advanced and Expired (`loop_manager.rs:764`) take
`command.value.cause()` directly. And the Graph Loop manager also commits a
command-production record with no fact (`graph_loop.rs:877`, `produced`
only, cause the current state), which the row does not list. Neither moves
a byte; both are one sentence each in the table, for the next documentation
touch. Also noted, as at lot 2: the inventory ends on a machine path
(`/tmp/rastok-lifecycle-v23.JSj1ey/`).

**Replay, same checkout, `0413fdf`.** `cargo fmt --check`, strict Clippy and
strict rustdoc pass. `cargo test --workspace --all-features`: 61 groups,
1 624 passed, 0 failed, 0 ignored, `site_content` 12/12 among them; the one
added test is the codec round trip. The three `no_std` targets check (`thumbv8m.main-none-eabihf`,
`thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf`). Browser suite 35/35
with the cached Chromium headless shell 1243 driven by the `npx` Playwright
core. QEMU Cortex-M33: echo heap 111 404 B, code 971 432 B (`.text`
897 420 + `.rodata` 74 012), hash `7106f322…`, 67 records, stdout identical
to `3736817`; Retry with recovery heap 129 188 B, code 1 043 488 B
(`.text` 958 676 + `.rodata` 84 812), hash `3cad77ef…`,
17 records, stdout identical. Against `3736817`: `.rodata` +4 B on
echo and −4 B on Retry, `.text` unchanged, the string table moved by the
shorter words. Margins: heap 1 884 B, code 5 088 B. Astra's
figures in the inventory are the same.

**Status.** A-T2-13 applied and reviewed; the two §3.1 precision remarks
open. Left to Yoann from this morning: `retry.md`, the tense of the UI
transport labels and the placements (T10), `UiCommand::RetryStep`.

## What validation produces

For each **⟂**: keep / rename / merge, with the chosen word. The answers
go to the review ledger as the vocabulary section of the realignment
(D-2026-10-11-1, point 5: the specification's words for business concepts,
explicit names for mechanisms); renames that move bytes (a journal or
package field name) are listed separately — most of these are type names
and move none. The appendix is the checklist: every public type should be
findable in one family above, or it is a term nobody introduced.

## Appendix — complete public type inventory on `fedada7`

Generated mechanically (`pub struct | enum | trait | type`, test modules
excluded), by module: 380 types in `rastok-core`, 238 in `rastok`.

### `rastok-core`

- `accounting.rs`: `ExactDecimal` `Money` `CostProfileName` `UnitPrice` `CostProfile` `MeasureSample` `ObservationScope` `AccountingObservation` `AccountingError`
- `boundary.rs`: `IngressEnvelopeProvenance` `CallId` `InboundCallId` `SessionId` `SessionWriteId` `PreparedRequest` `RequestCodec` `BootReferencePolicy` `ProcessExit` `FileExit` `HttpClientExit` `HttpServerExit` `WebSocketExit` `OutputOverflowReason` `Intent` `SessionOpenIntent` `SessionWriteIntent` `PortOutput` `Completion` `CompletionError` `Response` `RequestPreparation` `SchedulerCompletion` `DriverRouteError` `CompletionGate` `OpenGate` `BoundaryEvent` `SubmissionSequence` `IngressSubmission` `InboundSession` `IngressProposalHandle` `IngressProposal` `IngressAcceptance` `InboundExchangeId` `IngressRejection` `IngressDecision` `IngressAdmission` `Draft` `DatedPayload` `ClosingDeadline` `InstanceDeadline` `LifecycleEvent` `InboundConnectorDriver` `InboundCallEvent` `InboundCallResponse` `InboundCallTimeout` `InboundCallResponseStart` `InboundCallTimeoutStart` `InboundCallConnectorDriver` `OutboundConnectorDriver` `CallStart` `SessionStart` `CallConnectorDriver` `SessionDriverEvent` `SessionConnectorDriver` `SessionConnectorBindings` `OutboundConnectorBindings` `Supervisor` `PumpBatch` `DispatchError` `Dispatcher`
- `canonical.rs`: `TypeDecodeError`
- `canonical_cbor.rs`: `Value` `CanonicalDouble` `DecodeError`
- `connector/call.rs`: `TerminalVisibility` `FrameProduction` `CallSemantics` `CallCommandError` `RetryCaseError`
- `connector.rs`: `ContractId` `ConnectorIdError` `ConnectorExchange` `ConnectorRoles` `SessionOpening`
- `data_policy.rs`: `Binding` `Environment` `PropagationError` `RedactedValue`
- `datatype.rs`: `MediaTypeError` `IntegerWidth` `IntegerType` `FloatType` `DecimalType` `BinaryType` `ByteLength` `TimeUnit` `DateUnit` `IntervalUnit` `TimeZone` `NamedTimeZone` `TemporalType` `TypeField` `StructType` `MapType` `ArrayType` `ArrayLength` `EnumType` `StructuralUnionType` `TaggedCase` `TaggedUnionType` `DataType` `Discriminator` `DiscriminatorError` `NarrowingFailure` `TypeError` `ConformanceReason` `ConformanceViolation`
- `deployment.rs`: `EnvironmentName` `SecretName` `BootReference` `SecretProviderId` `SecretVersion` `SecretBinding` `SecretMaterial` `SecretProvider` `SecretProviderError` `PrivacyProfileName` `PolicyName` `RecipientPolicy` `TransferPolicy` `PrivacyRetention` `PrivacyRequirements` `PrivacyProfile` `PrivacyAttestation` `CostProfileAttestation` `EnvironmentAttestation` `DeploymentAttestation` `DeploymentProfile` `BootBindings` `DeploymentError`
- `envelope.rs`: `IdKind` `UnknownIdKind` `Id` `Scalar` `Encoding` `Data` `MetaSys` `MetaValue` `Namespace` `WriteTarget` `Meta` `Envelope` `Delivery`
- `expr.rs`: `EvalValue` `PayloadSource` `Signature` `NarrowType` `NarrowTypeError` `NarrowTarget` `Bindings` `DiagnosticKind` `Diagnostic` `Error` `ErrorKind` `Tree` `Expr` `UnaryOp` `BinaryOp` `Function`
- `instance.rs`: `Segment` `IdentityError` `Identity` `Generation` `Spec` `Closing` `Lifetime` `Live` `Phase` `ClosingStage`
- `journal/admission.rs`: `InstanceAdmissions` `AdmissionPrefix`
- `journal/codec.rs`: `Error` `Decoder`
- `journal/deadline.rs`: `Error` `Deadline` `Deadlines`
- `journal/instance_stop.rs`: `Command` `Packet` `Disposition` `Fold` `Error`
- `journal/loop_protocol/fold.rs`: `Error` `Phase` `RunState` `Fold`
- `journal/loop_protocol.rs`: `OpeningAdmissionId` `CommandId` `CommandValue` `DoneKind` `CommandProduced` `CloseReason` `Fact` `RoutedContext` `Packet`
- `journal/retry_protocol/fold.rs`: `Error` `Opening` `Settlement` `Phase` `Operation` `Fold`
- `journal/retry_protocol.rs`: `OperationId` `AttemptId` `Pin` `Policy` `Batch` `CloseReason` `Fact` `Packet`
- `journal.rs`: `DeadlineKind` `UnknownDeadlineKind` `Record` `IngressRecord` `InboundExchangeRecord` `InboundCallIngressRecord` `InboundExchangeExitRecord` `SessionIngressRecord` `RouteTarget` `ConsumedPosition` `Emission` `LoopRunId` `TransactionRecord` `RoutedDelivery` `EgressRecord` `CallRecord` `SessionRecord` `SessionStatus` `CallStatus` `Closure` `CloseVerdict` `CommittedIngress` `PreparedIngress` `Error` `Journal` `MemJournal`
- `json/hydration.rs`: `HydrationProfile`
- `json.rs`: `DeclaredNumber` `JsonError`
- `loop_proof.rs`: `Certificate` `Error`
- `node.rs`: `InvalidName` `Path` `OwnerPath` `OwnerSlot` `Port` `Direction` `Kind` `Extension` `Behavior` `Lifecycle` `UnknownLifecycle` `Trigger` `HandlerActivation`
- `observation.rs`: `RecordObservationKind` `ExchangeCorrelation` `RecordCorrelation` `RecordObservation`
- `plan/case_partition.rs`: `CasePartitionError`
- `plan/composition.rs`: `RouteCompositionError`
- `plan/dispatch.rs`: `Dispatch` `ClosedArm` `OpenArm` `DispatchStep` `MultiEmitValues` `LoopCommand`
- `plan/dispatch_validation.rs`: `DispatchErrorKind` `DispatchValidationError`
- `plan/emission.rs`: `Emission` `FieldWrite`
- `plan/retry.rs`: `LoopRepeatWarning`
- `plan/routing.rs`: `RouteRule` `RouteSelection` `RouteArm` `RouteDestination` `InstanceStopTarget` `LoopCommandTarget` `LoopCommandKind` `Endpoint` `EndpointOwner`
- `plan/routing_validation.rs`: `RouteConflictKind` `RouteConflict` `RouteErrorKind` `RouteValidationError`
- `plan/serialization/fields.rs`: `FieldError`
- `plan/serialization.rs`: `DecodeError`
- `plan/type_resolution.rs`: `TypeOrigin` `ResolvedPort` `ResolvedPortTable`
- `plan/validation.rs`: `PlanValidationError` `ValidatedExecutionPlan` `HandlerContextUse`
- `plan.rs`: `Slot` `PortDecl` `ExecutionPlan` `AssetBlob` `NodeExecution` `Node` `OwnerKind` `Owner` `ExtensionSpec` `PlanConstant` `RetrySpec` `LoopSpec` `LoopExpiry` `LoopDelay` `EvaluationSite` `OwnerValue` `ConnectorSpec` `AttrSetting` `VarCell` `Handler` `Edge` `EdgeEnd` `SymbolTable` `SemanticHash` `GraphVersion` `Manifest`
- `runtime/dispatch.rs`: `DispatchSelection`
- `runtime/vm/advance.rs`: `TimeAdvance`
- `runtime/vm.rs`: `VmError` `PumpReport` `InboundCallPumpReport` `BoundaryOutput` `Observed` `InstancePhaseInspection` `InstanceInspection` `MailboxInspection` `CallDirectionInspection` `CallReferenceInspection` `CallInspection` `SessionDirectionInspection` `SessionInspection` `VmInspection` `Scheduler` `Vm`
- `runtime.rs`: `GuestExecutor` `Fault` `State` `Mailboxes` `EmissionBuffer` `TransactionOutcome` `LoopFiring` `LifecycleFiring` `JoinCandidate` `HandlerExecutor` `Built` `BuiltData` `PinViolation` `DataSlot` `LazyBindings`
- `schema_annotation/application.rs`: `AnnotationBinding` `AnnotationApplication`
- `schema_annotation.rs`: `SchemaPathSegment` `SchemaPath` `SensitivityLevel` `CurrencyCode` `MeasureId` `PersonalDataClass` `DataAnnotation` `AnnotationKind` `AnnotationSet` `AnnotationTable` `AnnotatedSchema` `SchemaAnnotationError`
- `trivia.rs`: `Comment`
- `value.rs`: `Value` `TaggedUnionValue` `ValuePathSegment` `ValueDefect` `ValueError` `ValueKind`

### `rastok`

- `compiler/check.rs`: `Unit`
- `compiler/hir.rs`: `Hir` `Graph` `LoopSpec` `LoopDelay` `Const` `Val` `NodeSemantics` `RetrySpec` `Node` `InstancePolicy` `ClosingPolicy` `InstanceLifetime` `Attribute` `AttrValue` `Var` `Handler` `Edge` `EdgeTarget` `Lab` `Simulation` `Mock` `Scenario` `ScenarioStep` `InjectionTarget` `Expectation` `Quantifier` `Cmp` `ExpectTarget`
- `compiler/lexer.rs`: `TokenKind` `Token` `TerminatorSet` `TagOpener` `AssignmentHead` `ValueHead` `StatementHead` `JsonRegion` `Lexer`
- `compiler/lower.rs`: `LowerError`
- `compiler/syntax.rs`: `Name` `IntLit` `ExprSource` `ErrorNode` `SyntaxTree` `TopDecl` `LabDecl` `SimulationDecl` `MockDecl` `ScenarioDecl` `ScenarioItem` `ExpectStep` `CmpOp` `GraphDecl` `Annotation` `GraphItem` `OperatorDecl` `ConnectorDecl` `OperatorItem` `PortDirection` `PortDecl` `ValDecl` `VarDecl` `RemovedPartitionDecl` `RemovedPartitionField` `RemovedLifetimeValue` `RemovedWindowMode` `RemovedPerClause` `HandlerDecl` `Dispatch` `MatchArm` `MatchSelector` `Trigger` `ExtensionEvent` `Guard` `Statement` `LValue` `Value` `MapEntry` `FrontendValue` `FrontendPayload` `StructBody` `StructEntry` `Attribute` `SubBlock` `EdgeDecl` `EdgeEndpoint` `EdgeDestination` `EdgeMatch` `EdgeMatchArm` `EdgeCaseSet` `PortPath` `TypeRef` `SchemaAnnotation` `SchemaAnnotationArgument` `SchemaAnnotationConstant` `BaseType` `TaggedCaseDecl` `TypeDecl` `FieldDecl` `ScalarType`
- `compiler.rs`: `Compilation` `ScenarioDefinition` `SourceDefinition` `SourceArm`
- `conformance.rs`: `Provocation` `Profile` `CallProfile` `EventProfile` `SessionProfile` `InterleaveProbe` `InterleaveExpectation` `Report`
- `connectors/assembly.rs`: `HostedAssemblyError`
- `connectors/file/mod.rs`: `FileRequestBindings` `FileSettingsError` `FileBindError` `FileFactory` `File`
- `connectors/http_client.rs`: `HttpBearer` `HttpRequest` `HttpClientPlanBinding` `HttpClientRequestBindings` `HttpRequestRejection`
- `connectors/http_server.rs`: `HttpServerMode` `HttpServerRouteBinding` `HttpServerLabInput`
- `connectors/http_shared.rs`: `HttpVersion`
- `connectors/manifest.rs`: `DriverManifestEntry` `DriverManifest` `DriverManifestError`
- `connectors/mod.rs`: `ScriptedConnectorProtocol` `ScriptedProtocolBinder`
- `connectors/network/http_client.rs`: `HttpClientConfig` `HttpClientConfigError` `HttpClientFactory`
- `connectors/network/http_server.rs`: `HttpServerConfig` `HttpServerConfigError` `HttpServerFactory`
- `connectors/network/websocket.rs`: `WebSocketConfig` `WebSocketConfigError` `WebSocketClientFactory` `WebSocketServerFactory`
- `connectors/process/mod.rs`: `ProcessRequestBindings` `ProcessSettingsError` `Process` `ProcessBindError` `ProcessFactory`
- `connectors/process/service.rs`: `ProcessServiceFactory`
- `connectors/registry.rs`: `BoundConnectorDriver` `ConnectorDriverFactory` `DriverFactoryError` `DriverRegistry` `RoutedInboundProposal` `InboundPumpError` `InboundCallResponsePumpReport` `InboundCallTimeoutPumpReport` `RuntimeBindings` `DriverRegistrationError` `DriverBindingError`
- `connectors/websocket_protocol.rs`: `WebSocketLabDriver` `WebSocketMessage` `WebSocketClientOpening` `WebSocketPlanBinding` `WebSocketServerBinding`
- `contract.rs`: `PortDataType` `PortTypeChoice` `PortTypeRule` `PortContract` `PortResolutionError` `Availability` `FrontendFraming` `FrontendReader` `FrontendContract` `ConnectorContract` `ProfileSelector` `Profile` `ProfileProtocol` `ProfileSupport` `AttributeSpec` `Presence` `DefaultValue` `ValueDomain` `SheetDefect` `ContractCatalog`
- `deployment_config.rs`: `DeploymentConfigError`
- `diagnostics.rs`: `SourceId` `Span` `Severity` `Phase` `Code` `Diagnostic` `Bag`
- `golden.rs`: `Trace` `Divergence`
- `lab.rs`: `ScenarioReport` `ScenarioRun` `ScenarioActionKind` `ExpectationInspection` `ScenarioStepError` `ScenarioInjectionDraft` `ScenarioSession`
- `observability.rs`: `JsonLogQueueError` `ProjectionOutcome` `ProjectionBatch` `JsonLogSink` `JsonLogReceiver` `JournalLogProjector`
- `openapi.rs`: `ResolveMode` `ResolveReport` `ResolveError`
- `recording/mod.rs`: `Error` `Recording` `Recorder`
- `render.rs`: `Sources`
- `schema_projection/arrow.rs`: `ArrowBinaryKind` `ArrowTextKind` `ArrowDecimalWidth` `ArrowUnionMode` `ArrowField` `ArrowUnionChild` `ArrowType` `ArrowTypeCodeMapping` `ArrowUnionMapping` `ArrowProjection`
- `schema_projection/avro.rs`: `AvroUnionMapping` `AvroProjection`
- `schema_projection/mod.rs`: `LossReason` `ProjectionError` `UnionSelection`
- `stdlib.rs`: `MediaTypeCatalogEntry`
- `ui/mod.rs`: `UiCommand` `UiSnapshot` `UiStartError` `UiServer`
- `ui/view.rs`: `ExecutionState` `FaultView` `ActivityView` `SnapshotInput`
