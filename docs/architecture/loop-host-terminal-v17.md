# Loop host terminals — v17 change and qualification ledger

Status: qualified 2026-09-08; v17 package witnesses blessed once.

Authority: D-2026-09-08-1 and the user's 2026-09-08 specification.
The existing uncommitted M1.9 UX work is a separate, preserved change.

## Identity before implementation

- Package version 16 becomes 17 once; v16 rejects by version.
- LoopSpec retains keys 1–5 and adds key 6 for an optional resolved exhaustion
  target `[port, complete_type]`. Absence is valid only when expiry is proved
  impossible. No tag is reassigned.
- No `Loop.expired` port row remains. Authored terminal unions remain declared
  interfaces; outbound Call terminal pins are extended with the exact expiry
  case. The base Connector Contracts remain `@2`: Drivers have no new case.
- The Loop-specific semantic stream includes the resolved exhaustion target
  and type. Non-Loop programs receive no additional semantic contribution.
- Journal r2 packet, commands, run/admission domains and timer retention are
  unchanged. Expiry emission ids change with their ordinary destination port;
  recovery validates the carrying port against the plan, not a fixed spelling.
- Loop fixture changes require this semantic reason. Non-Loop package headers
  and checksums change categorically, bodies, semantic hashes and traces do not.

Baseline witnesses:

| Witness | Semantic hash |
|---|---|
| echo | `5ffba3ea18c52a98a538a63ed0a7bf23cd3b4be0fa1b7d7acc51ba85dd73cfa3` |
| JSONL | `2f89c8b5fa22f7fbe489d6aa770d517c3c9db8f92e850028c1e2bb1280efb2fc` |
| gauge | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` |

## Required evidence

Ordinary `err`, custom terminal, Call `exit`, no target under proof, missing
or incompatible target, old wiring migration, hostile target/type tampering,
case exhaustivity, Driver refusal, annotation retention, recovery after
produced/unconsumed commands and exhaustion, unchanged non-Loop witnesses.

Debugger: admitted applications `0 → 1 → 2 → 0`, simultaneous runs and
instance filtering, pending-delay indication, exact historical prefix,
command disposition labels, distinct r2 fact labels and a Loop-runs table.

No validation result is claimed until recorded below. The embedded heap and
code-size gates remain 128 KiB and 1 MiB respectively; remeasure the baseline
before changing core and the candidate before blessing.

## Candidate audit before the single blessing

The existing `package_ledger` generator decoded each candidate, compared
complete body bytes against the tracked v16 package and replayed its Lab.
All three bodies are byte-identical; hashes above are unchanged.

| Package | Body | v17 checksum |
|---|---:|---|
| echo | 939 B | `22fa0ea1b5bb24a60841be041a838be48427da6057dda463ce8042abb57a7940` |
| JSONL | 983 B | `8aa249c8d4571a5c44d66eef62cda6a0f2deb96312f16a33be9e74a99eaf5eb8` |
| gauge | 249 B | `4faf928f324b53da8c9fa78083c69f8453cae3e161b8cc297ada47e0210ea2c2` |

Causal seals are unchanged: echo `21de0053…`, `f76107eb…`, `4f4950c1…`;
JSONL `eb8aeb71…`; gauge `6f463270…`. No causal golden is re-blessed.
The package-only blessing targets echo, JSONL and gauge; the bare-metal
echo binary is the same generated candidate. Loop source fixtures now declare
host terminals; their tests derive the changed Loop identities and compare
uninterrupted/recovered traces. There is no separate tracked Loop package golden.

The Cortex-M33 candidate was measured before blessing using the explicit
temporary package override: `SPIKE_OK`, 67 records, heap **111,452 B** unchanged
from the pre-change baseline, **19,620 B** below 128 KiB. Linked `.text`
860,700 B plus `.rodata` 68,156 B = **928,856 B**, **119,720 B** below 1 MiB.
Relay maximum remains 2,065 ticks, decode 21,298 ticks (baseline 21,287).
This is the established echo workload, not a worst-case Loop memory claim.

The targeted Rust pass is green: 356 library tests, 9 host-terminal tests,
75 hostile-plan tests. Browser qualification: 11 isolated Chrome tests,
including admitted-application history, delay, two runs, instance filtering
and command-only labels; layout: 13 tests. Full workspace gates follow below.

## Final regression pass

The host-terminal suite now has 10 tests. Its added proof regression first
refused with `E-LOOP-TERMINAL`: a proof-only Loop with `Loop.terminal = [out]`
still tried to use the nonterminal `err` frame as the default exhaustion
target. Resolution now skips an unused default when exhaustion is impossible;
an explicitly written target still receives the normal validity checks.
The full host-terminal suite passes, including independent package validation.

The final browser pass is **11/11**, **0 skipped**, and also compares the
waiting dial at the same boundary in HISTORY and REPLAY. Measured playback
request medians are 1,005 ms at 1 event/s and 252 ms at 4 events/s. The existing
isolated Chrome harness was used because the in-app browser's installed
bootstrap still references a missing service module. No Claude/Fable process,
user browser profile or existing debugger session was invoked or restarted.
Captures B0, B1, B8 and B32, plus history/light/narrow views, are generated by
that harness; their directory is selected through `RASTOK_UI_CAPTURES`.

One stale v16 assertion remained in the noncanonical-header test during the
first full pass (1,271 passed, 1 failed). Both its expected canonical version
and its deliberately widened integer now encode 17: the test still proves
non-canonicity, not rejection of a different version. The corrected package
suite is 23/23; Loop fixtures are 37/37. No test was removed or ignored.

## Final qualification

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast` | **1,273 passed, 0 failed, 0 ignored**; 38 result summaries including empty targets/doc-tests |
| `cargo doc --workspace --no-deps` | succeeds |
| core `thumbv7m-none-eabi` | succeeds |
| core `thumbv8m.main-none-eabihf` | succeeds |
| core `riscv32imac-unknown-none-elf` | succeeds |
| SVG layout | 13 passed |
| isolated Chrome qualification | 11 passed, 0 skipped |
| Cortex-M33 echo | `SPIKE_OK`; memory and code margins recorded above |

The independent type and wire-depth audits include the complete exhaustion
type at LoopSpec key 6. Proof-only Loops contribute no exhaustion target;
ordinary and custom host terminals, Call terminal extension, annotation
retention, context isolation and recovery remain covered through real plans.
Echo, JSONL and gauge semantic/causal witnesses remain unchanged, including
their complete non-Loop package bodies.

This closes the host-terminal amendment and its debugger projections, not
the remaining [M1.9 UX qualification](release-qualification.md). No dependency,
Connector Contract version or journal r2 fact/domain was added. Existing
uncommitted UX work remains separate in scope; no commit or push is part of
this pass.
