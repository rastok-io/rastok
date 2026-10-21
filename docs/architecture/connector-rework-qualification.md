# Connector rework — qualification

21 October 2026. Branch: `realign/node-ontology`. This report records the
T5/T9 implementation checks; it does not approve the separate Process
Contract identity proposal. Package format 21, Process `std/process@4`
(job) and `std/process@5` (service) remain current at this checkpoint.

**Status: the A-T5-2 portable-core split is qualified. The hosted module
move and subsequent actions remain separate work.**

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
The hosted module move has not yet been applied at this checkpoint.
