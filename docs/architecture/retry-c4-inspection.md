# C4 — public Retry admission and retained inspection

> **Later qualification — 18 September:** the [common C4 ledger](retry-c4-qualification.md)
> records the single v19/v3 blessing and current checks. The candidate status,
> failures and deferred steps below describe this report's dated checkpoint;
> they are not the current [opening checklist](opening-showcase.md).

17 September 2026. **C4 is in progress; M1 and the v19/v3 blessing remain open.**
This continues the [C3 qualification](retry-c3-implementation.md#8-mixed-timers-bounded-publication-and-exact-instances--17-september)
under the existing [C1 algebra](retry-c1-algebra.md). It allocates no format,
Contract, journal fact, measurement report or debugger command.

## 1. Public admission

The public compiler now admits the independently validated finite outbound
Call profile: Process job, File read/replace and HttpClient. The source policy
and safety refusals remain; Graph/Operator/Session/Event/inbound Retry and
same-owner `Loop + Retry` remain unavailable. This is executable support in
the **unblessed v19 candidate**, not release qualification.

All 25 runtime regressions now compile through `compiler::compile`, instead
of bypassing its former final execution refusal. The 11 source tests preserve
negative diagnostics while asserting admission for supported policies; the
six independent package tests still verify hostile plans. The new public
admission regression first failed at the old named gate.

The small [inspection fixture](../../crates/rastok/tests/fixtures/ui-retry.rkg)
receives a decoded HTTP 503, holds its typed response, waits 100 ms, then
publishes the second response. Its two original token samples remain in the
journal; confidential response text stays hidden in both views. No network
or paid service is contacted by its Lab.

```sh
rastok simulate crates/rastok/tests/fixtures/ui-retry.rkg --ui
```

## 2. Inspect operations, not fabricated Loops

The snapshot borrows the VM's accepted Retry fold. Instances shows each exact
operation, its admitted attempt/bound, phase and original due time. Attempt
rows link to their own Call and admitted/closed/produced/consumed records;
Held and Published retain different dispositions. Envelope links reuse the
existing annotation-aware inspector. Neither prepared bytes/settings nor an
unredacted duplicate of DATA are projected.

The graph shows `+ Retry` and its policy tooltip, but no iteration dial,
manager command, new port or `RetryExpired`. The pending-work strip includes
the retained Retry deadline, not a prediction of which transaction executes next.
History names admission, holding, scheduling, final publication and stopping.
Temporal request envelopes enter the existing causal index.

The new regression first exposed **false outgoing activity on Held**: the
UI inferred a public/discard delivery from the static edge and the emitted
port, although the runtime had committed only manager disposition. The
projection now keeps the real input consumption and suppresses those false
outgoing highlights. Runtime routing and journal bytes are unchanged.
The history counter says envelope observations rather than deliveries: held
emissions remain visible without claiming a downstream transfer.

`retry_inspection_agrees_in_steps_batch_and_every_recorded_prefix` compares
batch and interactive records, live views with recorded prefixes, and checks
every recorded prefix for future-evidence leakage. It covers held/final DATA
redaction, distinct Calls, unchanged envelope ids and the retained deadline.
The browser regression is registered in `browser.test.cjs`; a browser was
not available in this session, so it is **not claimed as run**.

## 3. CLI and footprint qualification

`retry_is_public_in_simulation_and_real_recording_without_recalling_the_world`
exercises the fixture through `simulate`, and a real Process `SpawnFailed`
through `run --record`, then removes the source and calls `replay` with an
empty PATH. The input remains open until final publication: immediate stdin
EOF correctly requests Closing and drains an unopened operation as one Call,
as decided in D-2026-09-15-4. That initial one-Call result was a test setup
error, not permission to weaken Closing.

The unchanged Retry/full-recovery Cortex-M33 package is remeasured after
adding the borrowed inspection accessor: `.text` 964,644 B + `.rodata`
83,724 B = **1,048,368 B**, **208 B** below 1 MiB. Peak heap stays
**130,560 B**, **512 B** below 128 KiB; 17 records, same semantic hash,
`SPIKE_OK`. No threshold, package, witness or golden is changed.

The complete workspace run records **1,510 passed, 10 failed, 0 ignored**,
across 55 groups. Nine are the existing unblessed-candidate expectations:
the host hash, echo recording bytes, two network Contract expectations, the
HTTP-500 gallery golden, gauge package, JSONL hash/package and echo package.
No expected byte, golden or assertion was changed to erase those failures.

The tenth is the existing live gateway integration: its CLI did not bind
within 30 seconds and emitted nothing. An isolated Cargo rerun rebuilt the
executable and hit the same startup deadline. After loading that executable,
the unchanged workspace test passed in **0.21 s** (`gateway-warm.log`). The
observed loading delay is consistent with a cold-start issue on this host,
not proof of a Retry failure; both failures and the passing rerun are retained.
The timeout was not raised. The workspace is **not claimed green**.

Strict Clippy, formatting, rustdoc, the no-default-feature workspace check,
all three configured core no_std targets, JavaScript syntax, 15 layout tests
and three editor grammar tests pass. The CLI inspection fixture also reports
`PASS retained.second_attempt`. Local transcripts, including the exact size
and QEMU output, are under `/tmp/rastok-retry-inspection.3FLHn2/`.

After the final UI-only observation-counter wording change, strict Clippy,
all **11 `ui::view::tests`**, JavaScript syntax and the **18 layout/editor
tests** pass again. The hosted all-feature binary is rebuilt with that page.
It passes the Retry fixture through the CLI, and the unchanged live gateway
regression passes again in **0.18 s** (`gateway-final.log`).
This targeted follow-up is not another full-workspace pass or a browser pass.

## 4. Remaining C4 and M1 gates

- The follow-up [example migration](retry-c4-examples.md) makes two-tenants
  and webhook executable on native Retry. Their goldens still need
  the common reviewed migration; native Retry captures and the local browser
  suite are now qualified in the follow-up below. The HTTP gateway keeps OpenAI Responses as
  its target; the follow-up [projected JSON implementation](openai-responses-hydration.md)
  delivers six HTTP scenarios and local wire/Retry evidence. Final blessing
  and live provider qualification are not implied.
- Finish annotation stdlib integration and A4; the [implicit-unit registry
  migration](annotation-types-m1.md#16-explicit-unit-migration-inventory--17-september-before-implementation)
  is implemented. Reports,
  subsets and automatic pricing are removed, not pending.
- Recheck final workspace/targets/footprint and perform the one explained
  v19/v3 witness blessing; the candidate identity expectations are still red.
- Perform the browser/independent opening qualification and the remaining
  distribution and unaided quickstart gates in [opening-showcase.md](opening-showcase.md).

No external reviewer, paid benchmark, release tag, public push or publication
was invoked by this pass.

## 5. Local browser follow-up — 17 September

The existing standalone Chromium harness now passes **35 tests**, including
native Retry, all six webhook and all six Responses scenarios in LIVE,
HISTORY and REPLAY, no report, no secret disclosure and no application console
error. The in-app Browser provider still lists no available browser; this
qualification uses temporary Playwright 1.58.2 and installed Chrome, not a
new repository dependency or an independent review. The first cold startup
exceeded the existing 30-second deadline; a warm rerun passed without raising it.

The pass exposed a real geometry defect: `HttpClient · + Retry` could overlap
its port and an edge before any instance opened. The layout now measures the
same complete subtitle as the renderer. Its focused regression failed before
the correction and passes afterward; all **16** layout tests pass. The other
test corrections supply the fixture's required secret-version environment
variable, open folded inspectors before reading them, and expect the already
inventoried Process Contract `@4`, not historical `@2`.

All **25** focused Rust UI tests pass. Three current flagship stills at
transitions **0/20/34** and a **120-frame, 30.000-second** GIF are rendered from
retained snapshots at **1352×760**. The waiting state is selected from the
actual Retry fold, with its Held batch and due time visible; no Loop dial or
timer is fabricated. The stills were inspected locally. Their common golden
blessing and the independent opening audit remain separate gates.

Evidence: `/tmp/rastok-browser-qualification.DjTmnO/`, including
`browser-final.log`, `capture-final.log`, and the source PNG/GIF assets;
`/tmp/rastok-json-profile.ygk4aj/ui-lib-final.log` for the Rust projection tests.

After the explicit-unit migration, the rebuilt CLI again passes all 35 browser
tests. The final full workspace run has **1,527 passes and the nine unblessed
candidate failures**, zero ignored tests. The cold-start chat incident and its
passing reruns are recorded in the [migration evidence](annotation-types-m1.md#implementation-and-focused-qualification);
they are not removed from the campaign history. Current logs are under
`/tmp/rastok-measure-migration.tZmKT9/`.
