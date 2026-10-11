   # Vocabulary ledger — the named entities of the code, for validation

> Draft of 11 October 2026, prepared by Fable for Yoann's validation during
> the M1 human review. Status: **nothing here is decided**. Each line marked
> **⟂** is a question for Yoann; a validated answer moves to the
> [review ledger](human-review-m1.md) and, when it touches code, to the
> realignment lot of [D-2026-10-11-1](../spec/decisions.md).

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
specification already uses. Three rules are proposed for the validation,
themselves **⟂** to confirm:

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

## Collisions to settle first

Ranked by the cost of leaving them to the external reviewer.

1. **Executor.** The specification's taxonomy word for Operator | Connector
   (extensions.md §1). The code reuses it for two mechanisms:
   `runtime::HandlerExecutor` ("prepares a handler transaction in scratch
   isolation") and `runtime::GuestExecutor` ("pure, deterministic, budgeted
   guest execution — the contract the jq and SQL engines will implement").
   **⟂** rename both mechanisms away from the taxonomy word (interpreter,
   evaluator, engine, …).
2. **Dispatch.** In the specification and the plan, Dispatch is the
   Operator's typed decision tree (`plan::Dispatch`, `DispatchStep`; spec 82
   uses). At the boundary, `boundary::Dispatcher` is "the loop between the
   journal and the outbound drivers" (spec "dispatcher" 11 uses), with
   `DispatchError`, `DispatchSelection` (runtime, the handler match) on both
   sides of the word. **⟂** keep Dispatch for the Operator tree; give the
   boundary loop its own word.
3. **Bindings.** Four unrelated meanings: the verified Driver routing table
   (`OutboundConnectorBindings`, `SessionConnectorBindings`, host
   `RuntimeBindings`), the deployment boot inputs (`BootBindings`), the
   expression evaluation environment (`expr::Bindings`, `LazyBindings`) and
   the per-request Driver settings (`HttpClientRequestBindings`,
   `FileRequestBindings`, `ProcessRequestBindings`). **⟂** one word each.
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
   the other four.
6. **Node.** `plan::Node` is an executable row only, narrower than the
   specification's Node, which includes Graph. Settled by D-2026-10-11-1;
   listed here for completeness.
7. **Spec.** `instance::Spec` ("one owner's logical instance policy") is a
   bare generic noun; the other `*Spec` types (`ConnectorSpec`, `LoopSpec`,
   `RetrySpec`, `ExtensionSpec`) carry their subject. **⟂** `InstanceSpec`,
   or the specification's word, policy.
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
data, ext }` (see "Validated so far"). Still proposed, awaiting Yoann:
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
Driver's report before commit). Proposed, awaiting Yoann (12 October): `Observed { node, port, envelope }`
→ **`Arrival`**, its own doc's word ("every envelope reaching a node port …
in arrival order"), `observations()` → `arrivals()`; `DeliveryView`
withdrawn, this log is kept live, not rebuilt from facts;
`AccountingObservation`, `ObservationScope` → **`Measurement`**,
**`MeasurementScope`** (their own doc says "retained measurement");
`RecordObservation` stays: the specification's "retained observation" has
the same sense, what was seen without carrying identity.
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
**⟂ 4b.** `Generation` is called "instance reference" in prose and
`instance` in most field names. One of the three.
**⟂ 4c.** `frames: Vec<Segment>` — frame and segment for one thing.
**⟂ 4d.** `Slot`/`OwnerSlot` disappear with D-2026-10-11-1 (one typed
index); confirm the surviving word (index, not slot, not handle).

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
**⟂ 5b.** "bundle" (8 uses) names the immutable values of a `Live`; `Live`
itself has no specification word. Confirm "live instance" / "bundle" or
pick one.
**⟂ 5c.** "scope" carries three meanings; the specification mostly means the
closing or containing scope. Confirm or qualify (`Edge::scope` is an owner).

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
**⟂ 6b.** Activation / trigger / firing: `HandlerActivation` is the static
shape, `Trigger` what fired at run time, `LoopFiring` / `LifecycleFiring` /
`fire_*` the act. Three words, three meanings; confirm them, they recur in
the journal.
**⟂ 6c.** `LoopExpiry` is "a host-owned terminal", not a dated expiry;
collides with family 9.

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
**⟂ 7b.** `Vm::step` is one transaction; `TimeAdvance::step` is one
transaction, one expiry or one poll. Same verb, two granularities.
**⟂ 7c.** Fault / VmError / Error: "fault" is the specification's word for
a transaction that committed nothing; `VmError` is also that. Confirm the
split (Fault = evaluation, VmError = machine) or merge the words.
**⟂ 7d.** The spec's **Worker** has no type (ledger, parked remarks).

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
**⟂ 8b.** "pump" has one meaning (a boundary round) and three report types.
Confirm the verb; it is not in the specification's main vocabulary (18).
**⟂ 8c.** `ConnectorRoles { Inbound, Outbound, InboundAndOutbound }`
implements row 2 of the Contract sheet, *Direction* (who starts), not row 1,
*Role* (what it bridges). Plain word: `ConnectorDirection`; but
`node::Direction` already names a port's Input/Output, so one of the two
must yield the word (raised 11 October).

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
**⟂ 9b.** `LoopExpiry` (6c).
**⟂ 9c.** The decided time model (D-2026-10-10-1) adds *instant*, *origin*,
*logical timeline*; reserve "date" for civil dates and "instant" for the
timeline.

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
**⟂ 11b.** `ScenarioSession` reuses "session", the Connector exchange word.
**⟂ 11c.** The HIR mirrors plan names (`hir::Node`, `Graph`, `Handler`,
`Edge`) — same word, previous layer. Acceptable by module path; confirm.

## Validated so far — 11 October, evening

| Term today | What it is | Yoann's decision | Status |
|---|---|---|---|
| `Draft`, `PortOutput` | what the world provides, at ingress (`Draft { data, ext }`, always beside a `port`) and as a Driver's output (`PortOutput { port, data, ext }`): the same two fields, the same owner, the same rule, the Driver proposes and the VM mints; no `sys`, no `app` | **one type, `EnvelopeDraft { port, data, ext }`** (word closed 11 October, night; merge 12 October). Draft is the specification's word (26 uses) and says the state: not yet an envelope, it becomes one when the admission or the Connector transaction commits. The port belongs to the draft: the ingress doc already says "source port of the draft", the Driver trait says Drivers "birth drafts and never envelopes", and a draft is never built without a port. `PortOutput` disappears; `Completion` holds ordered drafts with exactly one on `exit`, last; the session terminal is a draft. No byte changes: the codec already writes `[port, data, ext]`. `EnvelopeContent` dropped: structural, and too close to `data`. Not `envelope_data` | decided |
| mint (doc comments: "the journal mints the id") | derive and assign the definitive identity | **identify** / **assign_identity** | decided |
| `pump` | one boundary round with the Drivers | **`poll_drivers`** | decided |
| `CompletionGate`, `OpenGate` | a conformance check of a Driver's completion against the Contract | **`CompletionCheck`** | decided |
| bless (goldens, witnesses) | accept a new reference | **approve** | decided |
| `Fold` | a read model rebuilt from committed journal facts: for Loop, every open run with its instance, entry port, iteration, remaining budget, state envelope, admission and pending command | **`LoopView`**, **`RetryView`**, **`InstanceCommandView`** — "a view of" something, less academic than projection (Yoann) | decided |
| `Intent` (`SessionOpenIntent`, `SessionWriteIntent`) | the description of an outbound effect handed to the Driver after its open fact committed | name the effect, drop the suffix: **`CallOpen`**, **`SessionOpen`**, **`SessionWrite`**; journal-before-effect is stated once, on the Dispatcher | decided |
| fact | the specification's word for a committed journal record (110 uses); `Fact` in the Loop and Retry protocols | **kept.** A fact is an assertion the journal answers for: it exists once committed, replay reproduces it on any machine, and it carries the transaction's verdict. An event is an occurrence; a fact is an established assertion, and may describe no occurrence at all ("the Loop admitted iteration 3") | decided |
| event | the exchange kind a Contract declares (`ConnectorExchange::Event`): the world speaks first, each firing is one spontaneous inbound root, nothing is owed back; Timer and Calendar in the stdlib | **kept for the exchange kind only.** The Contract sheet has two independent axes: Direction (who starts: `ConnectorRoles`) and Exchange (what is owed back: nothing / exactly one `exit` / messages then one `exit`). Examined and not retained: InboundCall / OutboundCall / Session, which fuses direction into two of the three kinds, would need Accepted/Opened for sessions, and loses event ≠ inbound call (zero `exit` vs exactly one, enforced by the validator). The empty cell, an outbound event, is the closed-call rule | decided |
| observation | what a Driver reports from the world before any transaction commits it: a response arrived, a session opened, a write settled, the host restarted. A tick and an HTTP request arrive by the same carrier (`IngressProposal`); the Contract's exchange kind says which place the observation takes | **new plain word**, already the word of 23 doc comments in `boundary.rs`. Rename the four carriers: `BoundaryEvent` → `BoundaryObservation`, `InboundCallEvent` → `InboundCallObservation`, `SessionDriverEvent` → `SessionObservation`, `LifecycleEvent` → `LifecycleObservation`; fix the `BoundaryEvent` doc ("one routable boundary fact for the VM": it is not yet a fact). `IngressProposal` keeps its name. Spec sentence to align with the realignment: extensions.md §1 "A Driver supplies a typed event" → observation | decided; the renames are a code lot |
| command | what a protocol asks the next transaction to do: Loop `Command`, `CommandProduced`, `CommandId`; instance-stop `Command`; `instance_commands` on the transaction record | **kept.** With fact, the two CQRS words of the journal protocols; no third word for them | decided |
| admission | the specification's central word (186 uses, ten files): a delivery entering an address, root or internal; the "admission point" is mailbox reservation plus the producer's route commit; `OpeningAdmissionId`, Loop re-admission, the `idle` deadline runs from the last admission | **kept**, examined for removal and refused: no existing word covers every delivery (ingress = roots only, acceptance = the positive outcome only, entry = the Loop's entry port, enter = no verdict); "admission control" is the standard systems term for a bounded entry with accept / pending / reject; removing it is a specification change, not a code rename | decided |
| the ingress chain: `IngressSubmission`, `SubmissionSequence`, `IngressProposal` (with its `reference` field), `IngressEnvelopeProvenance`, `IngressAcceptance`, `IngressDecision`, `inject_submission` | the door's wrappings and verdicts, eight words for one thing offered and judged | **flattened to the specification's words** (11 October, night): `IngressProposal { provenance, draft: EnvelopeDraft, sequence: IngressSequence, session }` is what is proposed, by the host or a Driver (the port is inside the draft, 12 October) (`IngressSubmission` disappears into it; `SubmissionSequence` → `IngressSequence { Next, Explicit }`); `RetainedProposal { handle, proposal }` is the proposal a Driver keeps while it waits; `IngressEnvelopeProvenance` → `IngressProvenance`; `IngressAdmission { Accepted(AdmittedIngress), Pending, Rejected(IngressRejection) }` stays the verdict; `IngressAcceptance` → **`AdmittedIngress { envelope, exchange }`**, named by its place in the chain beside `PreparedIngress` and `CommittedIngress` (Yoann: not `AdmittedRoot`; "root" reads as an image even though the specification uses it 192 times); `IngressDecision` → `Result<AdmittedIngress, IngressRejection>`, the verdict without `Pending` as a standard type; the one transaction behind both doors, `inject_submission`, → `admit_proposal`; `inject` and `admit_ingress` stay as the two doors. Words after: draft, proposal, admission, rejection, ingress; gone: submission, acceptance, decision. `AdmittedIngress` is what the Driver is told (two ids, portable); `CommittedIngress` is the journal's return (seq, envelope, record): the two doc lines must state that difference. Cost: `IngressSubmission` in 16 files, `IngressDecision` in 8, all host Drivers; a code lot, with the realignment or after the opening | decided |
| `plan::emission::Emission` | the lowered right-hand side of `out = …`: `Identity`, `Patch`, `FreshData`, `Data`, `Nothing`; held as `value` by `DispatchStep::Emit` and by `MultiEmitValues` | **`EmitValue`** (12 October, Fable's final proposal after `EmissionSpec` and `EmissionKind` were examined; Yoann's ok to confirm). The word of its neighbours, `Emit { port, value }`, `MultiEmitValues`, `FieldWrite.value`, and the specification's phrase "a braced value is classified once". In a plan every noun is the checked text of what will happen at run time, so "value" means the value expression, as the code already uses it. `journal::Emission` keeps the specification's word; `runtime.rs` imports both today and aliases the plan one `PlanEmission`, which disappears. Not retained: `EmissionSpec` (coherent with the header configurations `ConnectorSpec`, `LoopSpec`, `RetrySpec`, `ExtensionSpec`, not with the handler body where it lives, and `MultiEmitValues` would have had to follow); `EmissionKind` (the code's 18 `*Kind` enums are category tags of a thing that lives elsewhere, 9 of 144 variants carry a small detail; here three of five variants are the content); `EmittedEnvelope` for the journal; `EmissionForm`; `EmissionRule`; `EmissionRecipe`. The `*Spec` suffix stays what it is, the plan's checked description of an authored header declaration; collision 7 concerns only the bare `instance::Spec` | decided in substance |
| `Packet` (loop, retry, instance_stop) | the protocol's sparse contribution to one transaction (its own doc line): Loop = contexts, routes, commands produced and consumed, facts; Retry = attempt context and facts; instance stop = commands produced and consumed | The inner words are validated (fact, command, rows above). The container word is still open: `Contribution` (the doc's word), or by content | open |

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
