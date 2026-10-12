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

The pure `journal::instance_stop::InstanceCommandView` reconstructs target bindings and all
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

## 7. Session lifecycle follow-up — 10 September

The resident-Process preparation found a check/run mismatch in direct input
forwarding: `App.close -> conversation.close -> socket.instance.stop` checked,
but injection refused because the root emitter has no retained ancestry inside
`conversation`. Neither a matching business key nor a singleton shortcut can
supply that proof. Composition now refuses this source at check with
`E-INSTANCE-COMMAND`; the package reader independently rejects the same forged
boundary inventory. No target-selection rule, journal packet or version changes.
The regression fails on the former checker accepting the program, not on an
unrelated syntax or missing-output error. It covers singleton/keyed scopes and
unconditional/case-routed requests.

Two additional `machine_vm.rs` tests qualify the common Session machinery
through an effectless WebSocket Contract, **not Process service**:

- Alice and Bob reuse separate resources across repeated messages. Child
  death leaves the Node instance open; time alone never respawns it. A new
  input opens a fresh Session in that exact instance. Old writes are abandoned,
  not resent; late events cannot settle or close the replacement. Recovery
  produces exactly the same causal records.
- An Operator inside Alice's conversation emits repeated child-stop requests.
  Only Alice's resource is closed, exactly once and without renewing the drain
  deadline. A request after closure remains redundant; Bob can still write to
  his original Session. No stopped generation reopens.

The focused closure suite has 21 passing tests; the VM suite has 76. This
evidence does not admit service framing, physical Process termination or Lab
Session scripts; those remain in the Process contract review.

The repeated Cortex-M33 relay reports `SPIKE_OK`: peak heap **111,732 B**,
67 records and the same echo semantic hash. Linked text plus read-only data
is **951,020 B** (+240 B from §6); 97,556 B remain below 1 MiB. Relay
avg/min/max stays 1,908 / 1,859 / 2,093 calibrated ticks. This remains a
non-Loop relay measurement, not an embedded Session or saturation workload.
Core also checks on Cortex-M3, Cortex-M33 and RISC-V32.

The complete rerun, `cargo test --workspace --all-features --no-fail-fast --
--test-threads=1`, passes **1,353 tests**, **zero failed**, **zero ignored**,
across **43 suite results**. Fmt, strict all-target/all-feature Clippy,
`cargo doc --workspace --no-deps` and `git diff --check` pass. No package,
semantic-hash or causal golden was re-blessed for this correction. Logs are
retained in `/tmp/rastok-session-lifecycle.wjG2Fe/`. No new browser or actual
Process-service qualification is claimed.

## 8. Application shutdown is data — 10 September

D-2026-09-10-4 keeps `quit` and other application shutdown messages on the
ordinary data path. They neither request instance closure implicitly nor prove
resource termination when the local write settles. The Process review owns
the separate, still-unimplemented signal escalation policy.

The `machine_vm.rs` probe
`a_protocol_quit_is_data_until_an_observed_exit_or_bounded_instance_close`
executes two outcomes, each uninterrupted and with recovery:

- Alice's exact Session receives a retained `quit` write. Settlement alone
  leaves both resources live and emits no terminal. A later observed resource
  exit closes only Alice's Session, leaving her Node instance open.
- When the world ignores `quit`, a separately authored child-stop command
  starts bounded Closing. It requests closure at the original drain deadline;
  without an observed terminal it publishes `OutcomeUnknown` at the original
  settlement deadline and closes that Node generation.

In both cases, a late terminal cannot emit a second exit, Bob can still write
to his original Session, and recovery produces the same complete causal
records without resending `quit`. The probe checks the retained write bytes,
not only the number of responses. Like §7, it uses an effectless WebSocket
Contract to qualify the common Session machinery; it is not a physical
Process service, OS-signal or JSONL-service qualification. No runtime, plan,
Contract or golden changes are part of this follow-up.

Qualification: `cargo test --workspace --all-features --no-fail-fast --
--test-threads=1` passes **1,354 tests**, **zero failed**, **zero ignored**,
across **43 suite results**. Fmt, strict all-target/all-feature Clippy,
`cargo doc --workspace --no-deps` and `git diff --check` pass. The focused
shutdown-message test also passes with `--no-default-features`, without a
hosted network Driver. Evidence is
retained in `/tmp/rastok-protocol-shutdown.LNuiCu/`. Browser, embedded memory
and cross-target checks were not rerun: this pass changes tests and documents,
not the runtime or Drivers. Physical service admission remains open.

## 9. Two-budget Session shutdown — 10 September

Yoann accepted D-2026-09-10-4's timing without a third budget: admitted work
drains during `drain_for`, cooperative shutdown is requested at its deadline,
and forced local cleanup is requested at the settlement deadline if no terminal
was observed. A Unix resident Process will map those requests to `SIGTERM`
and `SIGKILL`; this slice implements the common handoffs, not that physical
Driver or service-profile admission.

The common boundary now distinguishes:

- `close_session`: the existing cooperative protocol request, retryable under
  bounded handoff pressure during settlement;
- `abort_session`: a nonblocking, repeat-safe local shutdown request, fenced
  by the exact SessionRef and authorized by the committed Closing request and
  its original deadline. It does not wait for the terminal output's mailbox;
- `release_session`: post-terminal-commit notification, followed by removal
  of the registry's correlation. Neither preparation nor a failed terminal
  commit releases the resource's logical ownership.

The registry regression first failed because forgetting a Session never
notified its owning Driver. It now checks one release, idempotent abort, no
terminal fabricated by abort, retention of correlation until release,
rejection of a stale reference, and an independent second Session still able
to write. Missing bound views surface a typed routing fault, not silent cleanup.

Two focused VM tests cover the schedule and failure windows:

- `session_release_waits_for_the_committed_terminal_even_after_a_storage_failure`
  exercises an observed terminal and scheduler-owned `OutcomeUnknown`, each
  with and without reconstruction from the committed prefix. Forced shutdown
  follows the retained deadline even when terminal storage refuses; release
  waits for commit. Reconstruction neither sends a signal nor invents an
  uncommitted observed exit. Bob keeps his original resource.
- `session_abort_does_not_wait_for_a_full_terminal_route` fills the recipient
  with 1,024 actual unmatched join deliveries, the current Worker bound. A
  blocked terminal cannot postpone the forced-stop handoff, but cannot commit
  or release its correlation without capacity. The source edge-capacity block
  is still deferred; this test does not claim to admit it.

The existing accepted/ignored-`quit` probe also checks exact resource release
and immunity to a late exit. These tests use effectless Session worlds: no
Unix signal, hosted WebSocket abort, resident JSONL frame or Lab service script
is qualified by them. Physical Drivers must implement and qualify the new
handoffs before claiming physical cleanup. Hosted WebSocket still uses its
existing protocol-close path and the new callbacks' no-op defaults; this pass
does not add physical WebSocket abortion. The crash window between terminal
commit and physical resource release also remains a service Driver qualification;
no OS handle or PID is reconstructed from the journal.

No new state counter, queue, timer, plan field, Contract ID, semantic domain or
causal record is introduced. No golden is re-blessed. Existing Process job
cancellation is unchanged. Generic reconstruction remains effect-free.

Embedded qualification ran before the core edit and after it. Both runs report
`SPIKE_OK`, 67 records, the same echo hash, a 111,732-byte heap watermark and
1,908 average relay ticks. The 128 KiB heap gate retains 19,340 bytes of margin.
Linked `.text + .rodata` is 951,060 bytes, up 40 bytes, with 97,516 bytes left
below 1 MiB. These are measurements of the existing relay workload, not of a
resident Process or a full Session queue. Core checks pass for
`thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf`.

Final qualification: `cargo test --workspace --all-features --no-fail-fast --
--test-threads=1` passes **1,356 tests**, **zero failed**, **zero ignored**,
across **43 suite results**. `cargo fmt --all -- --check`, strict workspace
all-target/all-feature Clippy, `cargo doc --workspace --no-deps` and
`git diff --check` pass. Command logs are retained in
`/tmp/rastok-session-cleanup.oCsdUO/`; the final complete run is
`workspace-final.log`. Earlier exploratory runs are not the qualification
result. No browser or physical-service run is claimed.
