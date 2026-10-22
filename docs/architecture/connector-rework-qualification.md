# Connector rework — qualification

21–22 October 2026. Branch: `realign/node-ontology`. This report records the
T5/T9 implementation checks; it does not approve the separate Process
Contract identity proposal. Package format 21, Process `std/process@4`
(job) and `std/process@5` (service) remain current at this checkpoint.

**Status: the byte-neutral T5/T9 series (A-T5-1, A-T5-2, A-T5-3,
A-T5-5 and A-T5-6) is qualified. A-T5-4 has its committed identity inventory
and proposed amendment, but no Process id or format change is authorized yet.**

## Reference after A-T5-1

Commit `491e397` changes Execution terminology, including the debugger,
without changing program behavior. Its qualification reproduced 1,598
tests in 61 groups, zero failures and zero ignored tests; the 12 site
checks, 35 browser tests, three no_std targets and both embedded workloads
passed. The 73 protected artifacts were byte-identical.

## A-T5-2 — portable interfaces and shared Contract declarations

Exchange data and per-Driver interfaces are in `connector::exchange` and
`connector::driver`. Polling and completion checks belong to execution;
scheduler obligations remain with execution timing. The unused supervision
interface is removed. Standard declarations under `connector::stdlib`
supply both the compiler and independent core validation, replacing the
second `ContractShape` table. No physical Driver is required to validate a
plan or compile a Lab scenario.

The source changes preserve admission, commit-before-effect, polling order
and exchange lifecycle. Contract ids, package fields, record tuples and
causal projections do not change. The 73 protected artifact digests still
match the reference; no example, fixture, golden, package or trace is
regenerated.

### Checks

The initial full suite passed 1,598 tests in 61 groups with no failure or
ignored test; formatting and strict Clippy passed too. Rustdoc found two
references to the old inbound-Call method. They were corrected to
`InboundCall::poll_completions`; no executable Rust statement changed.

Two subsequent attempts to replay the complete suite hit the live-server
startup failures described below. Both were stopped after those failures;
neither is counted as a passing complete suite.

On resumption, the CLI started normally and both live-server probes passed
individually, with their existing deadlines: the chat probe in 3.02 s and
the resident-gateway probe in 1.72 s. The fresh four-check run then completed:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo test --locked --workspace --all-features --no-fail-fast`: 1,598
  passed in 61 groups, zero failures, zero ignored tests, including both
  live-server probes and the doctest groups.
- `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps`:
  passed with zero warnings.

The extended qualification also completed these checks:

- `site_content`: 12/12 (also passed in the all-features suite).
- Browser tests: 35/35, no skipped or cancelled test.
- `cargo check --locked -p rastok-core --no-default-features` for
  `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, and
  `riscv32imac-unknown-none-elf`.
- Rustdoc Markdown-link lint: 230 Rust files, no relative Markdown link.
- Cortex-M33 echo and Retry-with-recovery: both ended in `SPIKE_OK`.

### Embedded measurements

Both rows use the same qualification command and release profile before
and after the change. Code is `.text + .rodata`, not the ELF file size.

| Workload | Decode peak heap | Total peak heap | Code before | Code after | Heap margin to 128 KiB | Code margin to 1 MiB |
|---|---:|---:|---:|---:|---:|---:|
| Echo | 36,160 B | 111,404 B | 968,852 B | 971,868 B | 19,668 B | 76,708 B |
| Retry and recovery | 30,684 B | 129,188 B | 1,040,736 B | 1,043,764 B | 1,884 B | 4,812 B |

The heap peaks are unchanged. Echo code grows by 3,016 B; Retry code grows
by 3,028 B. The shared declarations add readonly data while replacing
validation code: echo `.text`/`.rodata` are 897,820/74,048 B; Retry
958,948/84,816 B. The code margin remains a constraint on subsequent core
changes; neither embedded gate is raised.

Retry uses one operation, three Calls, 16 frames per channel and four
bytes per frame. Live and recovered peak heap are both 129,188 B; the
journal has 17 records. Its semantic hash remains
`d8e89d6dc345e7c6199ee5ffc5664b682e09040a8589c6e934016bbe1c32581f`.

Commands, with Rust 1.97.1 and the repository's release profile:

```sh
cargo run --locked -p rastok --example retry_spike_package -- /tmp/retry-qualification.rkp 3
cd spikes/rp2350-emu
env -u RASTOK_SPIKE_PACKAGE -u RASTOK_RETRY_OPERATIONS -u RASTOK_RETRY_FRAMES -u RASTOK_RETRY_FRAME_BYTES RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 -nographic -monitor none -serial none -semihosting-config enable=on,target=native -icount shift=0
env RASTOK_SPIKE_PACKAGE=/tmp/retry-qualification.rkp RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release --features retry-qualification
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 -nographic -monitor none -serial none -semihosting-config enable=on,target=native -icount shift=0
```

The actual local package was written under a unique temporary qualification
directory; the path above is the reproducible placeholder. No tracked
package was overwritten.

### Host startup delays

The checks ran through the CLI. Several host test executables waited in
the macOS loader before entering Rust; Cargo also waited for an IDE build
lock. No security setting, test timeout or assertion was changed. The
recheck with concurrent auxiliary `--list` processes encountered two
30-second startup failures in the live chat and resident-gateway probes,
with empty child output and no bound listener. That attempt was stopped.
The auxiliary preloading was removed rather than treated as an established
optimization. A second, sequential attempt reproduced both failures without
concurrent listings or another build. A sample of the gateway child before
its timeout showed only `_dyld_start`, before Rust execution. A direct
`rastok --help` invocation with a terminal also waited without output.
The cause of the host delay is not established. The failures are not
excluded from the tests and their timeouts remain unchanged.

The successful final run still encountered delays before several test
executables started. Two bounded `--list` probes through the RustRover
terminal returned successfully; they listed tests without executing them
or starting a build. They do not establish the cause of the delay. The
four completion commands ran sequentially and completed successfully.

Only the task's failed Cargo attempts and diagnostic/listing processes were
stopped. RustRover, security settings and unrelated processes were left
alone. No push, tag or publication occurred.

The final protected-artifact audit still matches all 73 reference digests.
The hosted module move was not yet applied at that checkpoint.

## A-T5-2 / T9 — hosted Driver organization

Physical implementations now live under `connector::drivers`, together
with the conformance harness. `drivers::registry` describes available
factories; `drivers::runtime` holds plan association and exchange ownership.
Connector catalogue validation is under `connector::contract`; standard
Connector access is under `connector::stdlib`. The non-Connector frontend,
annotation and media-type definitions stay in the original language modules.
The old `connectors` module and root conformance module are removed, without
compatibility aliases.

The physical network and Process implementations change only their imports,
module paths and relative test-fixture paths. Their protocol behavior is
unchanged. `CallProtocol` remains separate from `OutboundCall`: its pure
shared rules borrow immutably and its scheduler projection has no Node-path
argument, unlike the physical Driver interface. No extra adapter is added
to force the two signatures together.

The three shared-definition tests now say what they verify rather than
describing a second core mirror. Their assertions are unchanged. The
separate Session routing trait and global single-Call-Driver routing
implementation remain for the next action, not as compatibility interfaces.

### Checks and measurements

The sequential CLI gate completed successfully:

- Formatting and strict Clippy, all workspace targets and features: passed.
- Workspace tests, all features and `--no-fail-fast`: **1,598 passed in 61
  groups, zero failed, zero ignored**, including both doctest groups.
- Rustdoc with `-D warnings`: passed, zero warnings.
- Separate default-feature `site_content` run: 12/12.
- Browser tests: 35/35, zero failed, skipped or cancelled.
- All three no_std targets listed above: passed.
- Rustdoc-link lint: 235 Rust files, zero relative Markdown links.
- Echo and Retry-with-recovery on Cortex-M33: both `SPIKE_OK`.

The exact embedded commands are the ones above. The hosted move changes
neither embedded measurement:

| Workload | Decode peak heap | Total peak heap | `.text` | `.rodata` | Code total | Heap margin | Code margin |
|---|---:|---:|---:|---:|---:|---:|---:|
| Echo | 36,160 B | 111,404 B | 897,820 B | 74,048 B | 971,868 B | 19,668 B | 76,708 B |
| Retry and recovery | 30,684 B | 129,188 B | 958,948 B | 84,816 B | 1,043,764 B | 1,884 B | 4,812 B |

Retry again records 17 facts for one operation and three Calls, with 16
frames per channel and four bytes per frame. Its recovered peak equals its
live peak. All **73 protected artifacts remain byte-identical**. No fixture,
example, golden, package or trace was regenerated. Old `boundary` and
`connectors` module references are absent from Rust sources in both crates
and the spikes. The full contributor-guide rewrite and live documentation
sweep remain A-T5-6; the path references affected by this move are updated.

Host executable startup delays persisted, but this gate had no failed check
and used unchanged timeouts. No push, tag or publication occurred.

## A-T5-3 — one execution routing interface

Session routing now belongs directly to `OutboundDrivers`; the separate
`SessionConnectorBindings` trait and its method-for-method delegation are
removed. The per-object Call and Session interfaces remain distinct. This
changes no preparation, polling, closure or resource-ownership rule.

The global implementation which treated any Call Driver as an execution-wide
routing table is removed. Its consumers were test fixtures and the Retry
qualification spike, not the production hosted association. Integration
tests use an explicitly named, test-only Call adapter; core fixtures and the
spike state their routing implementation locally. The production replay
adapter and the named no-outbound/start-refusal adapters already implement
the routing interface explicitly and remain unchanged. The Process unit
probe now uses its real factory and `RuntimeDrivers`, like hosted execution.

### Checks and measurements

The sequential CLI gate passed formatting, strict all-target/all-feature
Clippy, **1,598 tests in 61 groups, zero failures and zero ignored tests**,
and rustdoc with `-D warnings`. Separate checks passed: site content 12/12,
browser tests 35/35, the three no_std targets listed above, and the rustdoc
Markdown-link lint. No assertion or test timeout was relaxed.

Both Cortex-M33 workloads ended in `SPIKE_OK`, using the same commands,
release profile and Retry workload parameters recorded above:

| Workload | Decode peak heap | Total peak heap | `.text` | `.rodata` | Code total | Heap margin | Code margin |
|---|---:|---:|---:|---:|---:|---:|---:|
| Echo | 36,160 B | 111,404 B | 897,820 B | 74,048 B | 971,868 B | 19,668 B | 76,708 B |
| Retry and recovery | 30,684 B | 129,188 B | 958,948 B | 84,816 B | 1,043,764 B | 1,884 B | 4,812 B |

All **73 protected artifacts are byte-identical**. No example, fixture,
golden, package, trace or recording was regenerated. The old Session routing
trait and global Call-to-routing implementation have no remaining reference
in the crates or spikes. Host startup pauses persisted before several test
executables produced their first output; this gate nevertheless completed
without a failed check. No security setting changed, and no push, tag or
publication occurred.

## A-T5-5 — named implementations and exact exchange ownership

The hosted registry accepts several named implementations of the same Contract.
Deployment configuration selects by Connector Node path, independently of boot
attestation and program identity. Without an explicit name, exactly one
implementation must satisfy the resolved Contract, mode, direction and exchange.
All selections are checked before any factory is called; insertion order is
never a selection policy. Factories expose the same per-object Driver views.

Private implementation indexes own Calls, inbound Calls, Sessions and pending
ingress callbacks. Equal proposal handles from different objects cannot alias.
Known responses, writes, cancellation and closure cannot cross object ownership,
even when the objects share one Contract. Runtime recovery reconstructs routing
from the accepted journal prefix under the same deployment selection, without
calling a Driver or replaying an external operation. This restores ownership,
not a socket or process; the selection itself is not recoverable from the journal.
No new public execution-index type, plugin loader or provider fallback is added.

Local two-implementation tests exercise interleaved Calls and instance generations,
Session openings/messages/writes/closure, wrong-object observations, equal ingress
handles, and recovery without external calls. Negative tests cover missing,
ambiguous and incompatible selection, unknown Nodes and names, contradictory
mode metadata, and malformed deployment configuration.

### Checks and measurements

The final CLI gate, launched from the RustRover terminal, passed formatting,
strict all-target/all-feature Clippy, **1,613 tests in 61 groups, zero failures and zero ignored
tests**, and rustdoc with `-D warnings`. The separate site suite passed 12/12;
the browser suite passed 35/35. All three no_std targets and the rustdoc-link
lint passed. No test deadline or assertion was relaxed.

The first full attempt was stopped after two chat-simulation failures: new
capability metadata incorrectly excluded a silent HttpServer `respond` Node
from Lab binding. The declaration now preserves the existing passive binding;
an added regression proves that source injection still refuses outside `ack`.
This does not implement graph-response simulation. A second full attempt was
stopped after the CLI WebSocket test encountered an already-used loopback port;
the listener's preparation error named that conflict. The final complete suite
runs with `--test-threads=1`, without another Rust build, and includes the
unchanged chat/WebSocket tests and the new focused regression. No test is
omitted and no deadline is relaxed.

Both Cortex-M33 workloads reached `SPIKE_OK` using the commands, release
profile and Retry parameters documented above:

| Workload | Decode peak heap | Total peak heap | `.text` | `.rodata` | Code total | Heap margin | Code margin |
|---|---:|---:|---:|---:|---:|---:|---:|
| Echo | 36,160 B | 111,404 B | 897,668 B | 74,048 B | 971,716 B | 19,668 B | 76,860 B |
| Retry and recovery | 30,684 B | 129,188 B | 958,892 B | 84,824 B | 1,043,716 B | 1,884 B | 4,860 B |

All **73 protected artifacts remain byte-identical**. No example, fixture,
golden, package, trace or recording was regenerated. Process ids and package
format remain unchanged. Several local executables again paused before their
first output; the qualified run uses the original tests and deadlines. No
security setting was changed and no public operation was performed.

## A-T5-6 — checked catalogue and the contributor path

The binary exposes `rastok connectors` and `rastok connectors --json`.
Both project the same checked standard definitions, default resolved port types
and alternative type rules, attribute phases/defaults, terminal cases and
outbound Call cardinalities. Physical and Lab capability manifests are joined
without constructing a Driver. Full conformance fixtures share their named
untestable cases with the catalogue; other modes explicitly name the missing
fixture rather than treating related integration tests as complete qualification.
File.watch remains deferred. A silent HttpServer.respond Lab binding is not
presented as support for graph-response simulation or authored source injection.

Catalogue tests compare the projection with the real declarations and resolved
ports, separate checker/Lab/installed support, keep missing capabilities visible,
and check human and JSON presentation. A CLI regression exercises both commands
and rejects unknown options. The worked File source in the rewritten contributor
guide compiles under the same catalogue. There is no dependency or new execution
command; no catalogue field enters an embedded plan or journal.

The first focused catalogue run exposed required codec attributes without a
default: resolving all ports using defaults alone correctly refused. The
projection now reports the required choice and the declared alternatives,
without inventing a default. A new regression covers this case. Its first
assertion incorrectly assumed HttpServer.out carried its own alternatives;
the declaration instead makes it follow in. The assertion now checks that
relationship and independently resolves each input codec. All six focused
catalogue tests and the separate CLI regression then passed; no existing
functional assertion was removed or relaxed.

README, WHITEPAPER and the living architecture/specification pages describe
the new module paths and per-Node selection. The strategy repository's English
reference narrative is copied verbatim in both READMEs and WHITEPAPER; the
whitepaper's missing full quotation is restored, not rewritten. PlantUML views
03, 04 and 06 and their SVGs are regenerated together
and visually checked. Dated design evidence keeps its historical text, with
current-path pointers where requested. The two review ledgers receive Applied
sections; their existing Fable review and Yoann decisions are unchanged.

Final source scans find no old Connector interface/type names from the rename
table, no boundary/connectors module paths, and no standalone VM word in
crates or the current specification (dated decisions excluded). The same
current-name checks cover README, WHITEPAPER, ARCHITECTURE, the contributor
guide, component map and refreshed diagram sources. Historical review tables
are intentionally not rewritten. ProfileProtocol's overriding id remains
until the separately authorized identity change; unrelated HTTP Response types
and the new connectors CLI word are not obsolete module/type references.

### Final checks and measurements

The sequential CLI gate (workspace tests use `--test-threads=1`) passes
formatting, strict all-target/all-feature Clippy,
**1,620 tests in 61 groups, zero failures and zero ignored
tests**, and rustdoc with `-D warnings`. Separate site checks pass 12/12,
browser checks 35/35, and all three no_std targets and the rustdoc-link lint
pass. Existing functional assertions and test deadlines are unchanged. The catalogue is also
invoked directly in human and JSON forms; it performs no physical operation.

Both Cortex-M33 workloads end in `SPIKE_OK`, using the exact commands and
Retry parameters recorded above:

| Workload | Decode peak heap | Total peak heap | `.text` | `.rodata` | Code total | Heap margin | Code margin |
|---|---:|---:|---:|---:|---:|---:|---:|
| Echo | 36,160 B | 111,404 B | 897,668 B | 74,048 B | 971,716 B | 19,668 B | 76,860 B |
| Retry and recovery | 30,684 B | 129,188 B | 958,892 B | 84,824 B | 1,043,716 B | 1,884 B | 4,860 B |

The catalogue and selection metadata are hosted; this action adds no embedded
payload. Stack is not remeasured here. All **73 protected artifacts are
byte-identical**, including the three package witnesses, the spike package
and reference traces. No witness is regenerated; recording tests remain green
and the recording format is unchanged.

### Remaining approval and scope

The identity inventory is commit `eabdee9`; named implementation selection is
`0b0ad9d`. The Process proposal (`std/process@6`, proposed decision and
choice between format 22 and an explicit format-21 exception) still needs
Yoann's approval. The identity-moving commit is therefore absent, not silently
deferred or folded into this catalogue commit. Current ids and formats remain
those in identity.md and stdlib.md. No public push, tag or publication occurs.
This report records Astra's checks; it does not claim a new Fable replay.
