# Explicit instance closure — implementation and qualification

10 September 2026. **Exact-instance command execution, package v18.** The semantic authority is
[D-2026-09-10-1](../spec/decisions.md#d-2026-09-10-1--route-explicit-instance-closure-without-restoring-graph-handlers)
and [instances §7.3](../spec/instances.md#73-explicit-closure-commands).
The qualification results below distinguish implementation from measured evidence.
This does not open Process service or reinterpret a v17 package.

## 1. Result protected

Application logic can request closure of its conversation or only its child
process through static wiring. The Graph remains a passive scope. Its existing
lifecycle controller, not a Graph handler or a synthetic Node, executes the
request under the existing bounded Closing policy.

Two distinct dispositions, shown as wiring fragments inside a conversation:

```rastok
decide.end_conversation -> instance.stop
decide.end_process -> codex.instance.stop
```

The first ends this conversation generation and its subtree. The second ends
only its direct child `codex` instance: its owned Process resource closes,
while the conversation, sibling Operators and another conversation's process
remain live. Neither spelling means restart, terminate all instances at a
path, kill an arbitrary OS process, or close one selected Session among many
owned by an instance. A targeted child Graph closes only its own subtree.

`stop` inside an Operator continues to close that Operator alone. There is no
targeted statement and no new `Lifecycle` Extension. Existing ports, Contract
terminals, Loop commands, route-case selection and `nothing` exclusivity keep
their meanings. A closure command counts as an explicit disposition but never
as a PhysicalEdge or an additional port.

## 2. Exact targeting and lifecycle

The plan must resolve the lexical owner: the wiring Graph itself or one direct
child. Each command must retain the exact generation-bearing `InstanceRef`
before producer commit. Existing admission/containment evidence, not a lookup
of the latest generation by path, key or `origin_id`, supplies that binding.

The implementation refuses an ambiguous target, does not create a
missing child merely to close it, and does not extend targeting to arbitrary
ancestors, siblings of the wiring Graph, or keyed instances selected through
a global lookup.
The selection and missing-target rules are enumerated in §5. A socket disconnect needs its own exact retained
conversation binding; the same textual user key is not sufficient evidence.

Production atomically retains the command, its cause and target together with
the transaction's other products, after a bounded capacity proof. The existing
lifecycle controller applies the winning request in a target-owned `l:closing`
record. The source Executor never writes another owner's close scope into its
own record. Pending commands are reconstructed from the journal; their
producer is not executed again after a crash.

Consumption starts Closing without requiring application quiescence. Waiting
for the resident process to end first would prevent the command from closing
it. Do not import Loop's verdict/quiescence rule into instance closure. Normal
drain, exchange cancellation and settlement, observed exits, inner-first
`l:stop`, stale-timer fencing and successor admission remain unchanged.

The first causal close request wins. A retained command whose exact target
has since begun or completed Closing is redundant/stale, not an invitation
to stop the successor. Its settled disposition must be reconstructible; the
settlement is derived from that exact retained close, without another record.
Closing invalidates the target generation's Loop work as it does today.

## 3. Repository contact and identity inventory

| Existing seam | Evidence | Required treatment |
|---|---|---|
| Static dispositions | `crates/rastok-core/src/plan/routing.rs`, `RouteDestination` | InstanceStop is a separate encoded lifecycle disposition, not a fake Loop command |
| Plan validation/encoding | `plan/routing_validation.rs`, `plan/serialization.rs` | Prove owner scope, exact targeting requirements, capacity and canonical route sets independently; new meaning cannot reuse a tag |
| Application stop | `runtime.rs`, `DispatchStep::Stop` | Preserve current local close request and its identities |
| Closing controller | `runtime/vm.rs`, `request_close` / `activate_close` | Reuse target-owned `l:closing`, retained deadlines and subtree cascade |
| Journal ownership guard | `journal.rs`, `InvalidCloseScope` validation | Preserve `close_scope == transaction.instance`; the command-producing record is not the target's closing record |
| Recovery ownership guard | `runtime/vm/recovery.rs`, `close_scope` reconstruction | Keep exact target/generation checks; recover pending requests and their settled disposition |
| Bounded command precedent | `journal/loop_protocol.rs` and its fold | Reuse the produced/consumed discipline, not LoopRunRef, Loop tags or Loop's state machine |
| Diagnostic recordings | `crates/rastok/src/recording/codec.rs` | Audit retention of every new command fact and explicit version/refusal requirements; no silent loss in replay |

The necessary changes belong to three separately audited registries:

1. **Plan/package:** an explicit lifecycle destination and independent
   validation. Blessed v17 is not amended silently. Section 5 allocates v18
   before blessing; no previous tag is reinterpreted.
2. **Program semantics:** only programs using the new disposition gain its
   identity contribution. Bare `stop`, Loop-only programs and unrelated plans
   remain semantic-hash witnesses.
3. **Journal/recovery:** command identity, production and settlement facts
   must survive the producer/controller crash window. Existing close-scope,
   timer and Loop facts are not reinterpreted. Existing unaffected causal
   goldens remain fixed; new traces need an enumerated ledger before blessing.

The generic closure addition does not alone justify changing a Connector's
Contract id. Process service's profile, framing, correlation and resource
rules have their separate [review](process-contract-review.md).

## 4. Admission and qualification gates

- Parse/check the two command spellings without adding a port or spatial edge;
  independently reject forged targets and ambiguous instance selection.
- Prove child-only closure: Alice's `codex` ends, Alice's sibling state and
  conversation remain, Bob's `codex` is unaffected.
- Prove Graph closure: only the selected Graph generation's subtree closes;
  pending child stop emissions and exchange terminals retain their ordering.
- Seal the target before commit; a late command cannot close a successor, and
  a never-opened child is not spawned merely to satisfy the command.
- Prove no partial fan-out when command capacity or an ordinary destination
  is full; no unbounded pending-command queue is introduced.
- Crash after command production, before consumption and after the winning
  `l:closing`; recover without losing the request or replaying its producer.
- Race commands with duplicate requests, lifetime expiry, Loop wakeups,
  resource exit and parent Closing; keep one winning closure per generation.
- Prove unchanged Operator `stop`, Loop r2 and non-participating witnesses;
  remeasure embedded heap before changing its hot path.
- Show command dispositions in the debugger without manufacturing a port,
  spatial edge, mutable counter or unclassified DATA export.

The focused tests exercise the public compiler, package reader and VM, with
independent protocol sabotage tests in core. Whole-workspace and embedded
measurements are recorded separately below, not inferred from those tests.

## 5. Implementation checkpoint — 10 September

The implementation request opens work on the agreed existing-target paths.
Yoann confirmed instance-local closure for every Connector, not just Process:
the key is the exact instance reference, including its generation. Repeated
requests do not restart Closing, repeat resource cancellation, or extend its
deadlines. A never-opened target still refuses by name; idempotence of an
existing target does not authorize creating or guessing a missing child.

Selection follows exact parent-generation links from the emitting instance to
the lexical wiring Graph. The target is that Graph itself, the emitting child
itself when it is the named target, or its uniquely bound direct child. Multiple
child instances refuse; the last generation or matching business key is never
a selection rule. If there is no live child, a single previously opened child
in this exact parent generation is an already-closed target; several historical
generations are ambiguous, not an invitation to select the latest. The target
and cause are retained before producer commit. Every unkeyed child is local to
its enclosing Graph instance; a child-specific key or lifetime may subdivide
that scope. Closing a child does not close its siblings or parent.

Pending requests are ordered by retained record order, then canonical command
identity within one record. The winner is consumed by the target-owned
`l:closing`. A request superseded by a retained close of its exact target or
ancestor is settled by that fact; it does not require a fabricated transaction
on an already stopped instance. Replay re-derives that disposition from the
same prefix. There is no Loop-style quiescence prerequisite.

Identity inventory, recorded before blessing the new format:

| Registry | Allocation / invariant |
|---|---|
| Plan | Package v18; route destination tag 4 = `[4, Graph-owner lexical scope, target owner]`, using the existing tagged owner framing; tags 1–3 unchanged; boundary inventory retained for command scope proof even in a flat Graph |
| Semantic | Same destination tag 4 in the existing framed routing contribution; only new-command programs add bytes; `stop`, Loop r2 and the three non-participating witnesses unchanged |
| Command | `rastok.instance-stop.r1`, ID domain `0x23`; frame source endpoint, exact source/scope/target instance references and causal envelope id; no business-key-only identity |
| Journal | Optional command packet on INBOUND / TRANSACTION, production and winning consumption separate; causal key 57, omitted when absent (56 remains timer schedules); existing record tags and close-scope ownership unchanged |
| Recording | Diagnostic recording v2, explicit command packet retention; previous/unknown versions refuse, never silently omit commands |
| Contracts | No Contract ID movement; closure reuses the existing lifecycle protocol |

Pre-change Cortex-M33 relay measurement: 111,452 B peak heap, 67 retained
records, `SPIKE_OK`. This is the non-Loop relay baseline, not a measurement of
command execution or recovery. The new focused source regressions initially
refuse the command as an unknown name / composed path, as expected.

The implementation resolves both spellings into a distinct
`InstanceStopTarget`, retains its lexical Graph through routing composition,
and rejects deep targets and source/intermediate use with
`E-INSTANCE-COMMAND`. Ordinary ports named `instance` are not reclassified.
The routing-field codec carries tag 4; independent validation checks both
routing tables, direct-child scope, canonicalization and bounded Closing.
Journal key 57 retains a packet only on participating records. The lifecycle
queue is bounded at 1,024 pending requests; a full queue refuses the complete
fan-out before commit. Plans with no such command build no command fold on
their execution hot path. No Connector Contract or Driver code changes.

The pure `journal::instance_stop::Fold` reconstructs target bindings and all
pending/consumed/superseded dispositions. VM recovery re-derives each production
from the source and prefix routing, then restores only pending commands. A
recording carries the same packet losslessly. No target is re-selected when a
command is consumed, and no producer is rerun to recover its command.

**Named limits:** a never-opened target refuses; multiple candidate child
instances refuse. An input forwarded from outside a nested wiring Graph
cannot itself supply that Graph's exact source ancestry: a direct closure
route refuses with `E-INSTANCE-COMMAND`. An Operator inside the Graph can
emit the request in its retained context. Child-only closure inside an active Graph Loop still needs
the deferred lifecycle-dependent control proof; the current Loop Graph can
stop itself, or its enclosing Graph can stop that child. Process service/RPC,
one resource per conversation and the agent-gateway conversation scenario
remain separate work. File/HttpClient/Process qualification here uses their
existing outgoing Call profiles, not a fabricated resident Process.

## 6. Qualification evidence

Focused public tests live in
[`instance_stop.rs`](../../crates/rastok/tests/instance_stop.rs), protocol tests
in [`journal/instance_stop/tests.rs`](../../crates/rastok-core/src/journal/instance_stop/tests.rs).
The public matrix covers Graph/child isolation, repeated closure, successor
fencing, missing-target refusal, two independent pending targets, both capacity
refusals, bounded effect settlement for File/HttpClient/Process, a Loop Graph's
Stopped verdict, foreign-plan sabotage and every-prefix recording/recovery.
Protocol tests additionally attack identity, malformed packets, ambiguous keys,
duplicate consumption and cross-target consumption order. The source matrix
contains 19 tests and the independent protocol matrix eight.

Echo, JSONL and gauge package bodies and semantic hashes compare byte-for-byte
with v17. Only header version and checksum move, as declared above: package
sizes 1,020 / 1,064 / 329 B; the spike's binary echo copy is identical to the
decoded hex fixture. No causal `.trace` fixture is changed by this amendment.

The rebuilt Cortex-M33 relay reports `SPIKE_OK`, the same echo semantic hash
and 67 records. Peak heap is **111,732 B** (+280 B; 19,340 B below 128 KiB).
Text + read-only data is **950,780 B** (+22,204 B; 97,796 B below 1 MiB).
Calibration is 39,999 ticks for 2,000,000 instructions; relay avg/min/max is
1,908 / 1,859 / 2,093 ticks. No gate is raised. These are this relay's linked
image and heap, not an embedded command-saturation or recovery measurement.

Core checks pass for Cortex-M3, Cortex-M33 and RISC-V32. The optional Wasm
check could not compile because `wasm32-unknown-unknown` is not installed;
no Wasm qualification is claimed. The 15 JavaScript layout tests pass. The
optional browser harness could not start with the current test-host setup:
Node could not resolve `playwright`. No fresh browser qualification is claimed.

The first completed all-feature workspace pass recorded 1,345 passed, two
failed and none ignored across 43 suite results. One failure pinned the old
package version in `plan_fields`; its assertion now names v18 and all 11 tests
in that suite pass. The other was a live WebSocket startup deadline: the
isolated test and all 21 CLI tests pass on rerun without changing code or
relaxing the deadline. The final command,
`cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`,
passes **1,349 tests**, **zero failed**, **zero ignored**, across **43 suite
results**, including the final 19 public closure tests and eight protocol tests.

`cargo fmt --all -- --check`, strict all-target/all-feature Clippy,
`cargo doc --workspace --no-deps` and `git diff --check` pass. Qualification
logs for this pass are retained locally in
`/tmp/rastok-instance-stop.ttKkGE/`; `workspace-final.log` records the earlier
two failures, `workspace-verified.log` the final sequential run.
