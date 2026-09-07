# R0 repository-contact audit

> Audit date: 2026-09-05. Paths and line numbers refer to the tree inspected at
> the opening of R0. The working tree already contained the user's M1.9/UI
> changes; this audit did not modify executable code.

## Confirmed claims

| Claim | Repository evidence | Result |
|---|---|---|
| A Graph and executable Nodes are already structurally distinct in the plan. | `crates/rastok-core/src/plan.rs:464-475` says Graphs live in `owners`, while `NodeExecution` contains only Operator/Connector. `plan.rs:621-652` defines Graph/Node owner rows. | confirmed |
| The current public taxonomy still has Kind, Type, and Aspect. | `crates/rastok-core/src/node.rs:172-269`. | confirmed; R3 removal/rename required |
| Graph handlers use a synthetic Operator-like checking context. | `crates/rastok/src/compiler/check.rs:1999-2027` constructs `__graph_handler`; `runtime.rs:844-1037` has a separate `fire_graph` evaluator path. | confirmed |
| Direct child output is a second Graph activation path. | `crates/rastok-core/src/runtime/vm.rs:5939-5997` creates `HandlerActivation::ChildPort` beside ordinary route preparation. | confirmed |
| That side path is bounded by the Worker queue, not an ordinary mailbox cursor. | `runtime/vm.rs:5911-5935` accumulates `loop_work` and returns `LoopQueueFull`; `LoopWork::Fire` is defined at `runtime/vm.rs:515-543`. | confirmed |
| RootSink duplicates observation and treats `err` by name. | `runtime/vm.rs:7004-7034` observes at the source, then adds `node: None`; line 7026 compares `port.name() == "err"`. | confirmed |
| A second handler, Connector handler, edge `when`, branch edge, and multi-emit are parsed but fail closed. | `compiler/check.rs:4537-4553`, `:6321-6324`, and `:8319-8381`. | confirmed |
| One-port false guards consume while join false guards retain. | `runtime.rs:599-651` includes the consumed cursor before guard execution; `runtime.rs:654-705` tests a join candidate purely before `fire_join`. | confirmed |
| Emissions are already retained in source order. | `runtime.rs:521-530`; `journal.rs:906-915`; `TransactionRecord.emissions` at `journal.rs:1105-1107`. | confirmed |
| Current Loop run identity can alias two admissions from one root into one instance. | `journal.rs:137-149` derives from only `InstanceRef` and root id. `runtime/vm.rs:5972` reconstructs the run from `origin_id`. | confirmed; exact `OpeningAdmissionRef` required |
| Current Loop recovery explicitly refuses a spatial body. | `runtime/vm.rs:1455-1494`. | confirmed; R7 RecoveryFold required |
| The owner table is semantic authority and must stay. | `plan.rs:547-652`; the plan's Node points to its owner at `plan.rs:478-495`. | confirmed |
| `node::Type` is not transported as an independent plan row. | `plan.rs:464-475`; effective type is derived by `Node::ty` at `plan.rs:498-515`. | confirmed non-movement |
| Format v15 transports `unwired`, Graph handlers, ChildPort, and AspectSpec. | `plan/serialization.rs:52`, `:432-437`, `:589`, `:644`; current shape is documented in `docs/architecture/plan-package.md:328-550`. | confirmed v16 movements |
| Six executable Connector identities currently use `@1`. | `crates/rastok-core/src/connector.rs:12-27`. | confirmed; R4 changes them to `@2` once |
| The current Rust identifier type is named `ConnectorId`. | `crates/rastok-core/src/connector.rs:61`. | confirmed; R3 renames the type to `ContractId` without changing encoded id text |

## Claims corrected by contact with the repository

### A. “A Graph has no transaction” is false literally

`TransactionRecord.owner_path` explicitly permits Graph lifecycle owners at
`crates/rastok-core/src/journal.rs:1062-1066`. The lifecycle controller builds
an empty Graph `TransactionRecord` at
`crates/rastok-core/src/runtime/vm.rs:4540-4556`. Removing those records would
erase generation/containment recovery facts from M1.4.

**R0 correction:** a Graph owns no *application* transaction, handler,
application mailbox, or mutable application state. Generic lifecycle
transactions remain attributed to Graph owners.

### B. “Every Driver event is INBOUND before its transaction” is false

The canonical record taxonomy distinguishes world-originated ingress from an
outbound Call's correlated terminal observation. `docs/spec/identity.md` keys
20–28 define INBOUND roots; keys 40–47 define the existing OUTBOUND Call. The
runtime completion paths call Connector completion machinery against the open
exchange rather than minting a second root (`crates/rastok-core/src/runtime.rs`
around the Connector completion methods at lines 1370–1560).

**R0 correction:** only world-originated Driver facts are INBOUND. A correlated
outbound completion enters a bounded Connector activation path and closes the
existing exchange before/with its ordinary Connector transaction; it never
gets a fabricated root identity.

### C. A produced Loop command needs its own durable fact

The opening text required recovery of “a command committed but not consumed”
but listed only a later `Next | Again` fold transition. Current v15 carries one
`loop_fact` per transaction (`journal.rs:1089-1091`) and represents Graph work
only in RAM as `LoopWork::Fire` (`runtime/vm.rs:515-543`). That cannot distinguish
producer commit from manager consumption.

**R0 correction:** v16 records `CommandProduced` in the producer transaction
and `CommandConsumed` in the manager transaction. Pending commands are the
journal fold difference.

### D. A proof does not always make `Loop.expired` impossible

The current contract says explicit fuel remains the deliberate cap when fuel
and proof coexist (`plan.rs:685-694`; `docs/spec/loop.md:177-185`). A valid
measure-derived bound may exceed a smaller explicit fuel.

**R0 correction:** measure-only makes exhaustion impossible; fuel+measure is
conservatively productible unless the proof establishes termination within the
explicit cap.

### E. Run context cannot be recovered from `origin_id`

Current direct-child activation derives the run from the owner instance and
`origin_id` (`runtime/vm.rs:5972-5988`). The v16 requirement explicitly permits
multiple runs within one instance, and a fan-out may deliver the same envelope
twice through distinct edges.

**R0 correction:** exact `OpeningAdmissionRef` enters `LoopRunRef`, and routed
deliveries retain an active LoopContext stack. Joins within a Loop application
require identical contexts.

## Assertions not yet executable, but consistent with the seams

| Target assertion | Existing seam | Required gate |
|---|---|---|
| Operator and Connector share the Executor commit pipeline. | `TransactionExecutor`, `TransactionOutcome`, `validate_and_prepare_emissions`, and `route_emissions`. | R3/R6 rename and generalize without a public trait |
| Route selection can happen before capacity reservation. | `validate_and_prepare_emissions` calls `prepare_routes`, then `check_room_for_routes` before commit at `runtime/vm.rs:5861-5936`. | R6 inserts the case-table lookup before the same choke point |
| Destination sets can reserve atomically. | preparation already accepts all routes for every emission before commit. | R3/R6 expand one canonical set to N prepared routes |
| Recursive Dispatch fits the current evaluator. | guards and linear `Step` already execute under scratch isolation. | R3/R4 change the plan algebra; R6 selects one leaf |
| Multi-emission can stay one transaction. | parser syntax exists and the executor buffer already orders emissions. | R3 removes the deferral and reserves all products together |
| Manager work can be recovered from journal. | v15 `fold_loop_run` already re-derives run state from transaction facts (`journal.rs:577-619`). | R4 adds r2 facts; R7 supplies the full spatial fold |

## Historical documents

M1.7 backlog and exit reports accurately describe what landed in v13/v14 and
remain historical evidence. They are not rewritten to pretend the old design
never existed. R2 supersedes their live claims through `decisions.md`, the
current spec documents, and the architecture map.

## R7 implementation contact — temporal recovery evidence (2026-09-06)

The requested complete RecoveryFold cannot reconstruct every deadline from
the existing records alone. `TransactionRecord::timers` retains `(kind, ref)`,
not the due instant; `logical_time` is sparse and exists only when a firing
observes `now()`. `Vm::route_prepared` arms join-within deadlines from its
explicit current time; `fire_lifecycle_next` arms lifetime budgets. Replaying
those operations at restart time would silently lengthen or shorten them.

Loop r2's `Advance.due_at` already retains the needed evidence for its own
delay. The initial spatial recovery gate therefore admits persistent Operators
and those Loop delays, and refuses timed joins, other lifetimes, Closing and
Connector obligations. The full R7 gate must retain the corresponding exact
timing facts before promising recovery for those profiles. They belong to the
journal's retained recovery metadata, not a lossy observation/export stream;
their causal-seal treatment must be enumerated in the identity ledger. No
restart clock may stand in for missing history.

**R4 review follow-up, 2026-09-07 (D-2026-09-07-2).** The representation is
now allocated before the v16 blessing: retained key 56 carries the explicit
arm instant and ordered `(kind, reference, due_at)` schedule. Its strict
core codec is executable, and Loop delay captures the field before commit;
missing or inconsistent Loop dates refuse atomically. Key 56 stays outside
the causal projection and must be integrity-protected by storage. The full
R7 work still needs to capture every join/lifetime/idle/Closing/Call/handshake
arm at acceptance and reconstruct its target, payload and stale fence from
the causal record. Those profiles remain refused; no new field allocation is
postponed to R7. This does not claim a shipped durable journal backend.
