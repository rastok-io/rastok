# C4 — migrate the teaching examples to native Retry

> **Later qualification — 18 September:** the [common C4 ledger](retry-c4-qualification.md)
> records the single v19/v3 blessing and current checks. The candidate status,
> failures and deferred steps below describe this report's dated checkpoint;
> they are not the current [opening checklist](opening-showcase.md).

17 September 2026. **Three public examples migrated; the single v19/v3 blessing
remains open.** This implements the already approved
[operation contract](retry-c1-algebra.md), following
[public admission and inspection](retry-c4-inspection.md). It introduces no
language rule, Contract, plan key, journal fact or report.

## Inventory before changing the examples

| Example | Observable migration | Witness consequence |
|---|---|---|
| `examples/keyed-retry.rkg` | Replace the App Loop and mutable request `attempt` field with Process `+ Retry`; pin the request, hold the failed attempt, publish only the final one. Keep two tenant instances, nested audit and the idle-lifetime scenario. | Both `two-tenants` traces change: Retry operation/attempt facts replace Loop facts; audit runs once per admitted request, not once per attempt. The answer no longer fabricates an attempt count: inspect the retained attempt ordinals. |
| `examples/webhook-to-partner.rkg` | Replace `deliver : Loop` and the downstream charge extractor with a plain scope containing HttpClient `+ Retry`, typed `Receipt`, and a final business-status adapter. | Five webhook traces change; the invalid-business-payload trace never reaches the Call and remains byte-identical (verified in the [common qualification](retry-c4-qualification.md)). The last `TimeoutExpired` is the bounded failure, not `LoopExpired`; both 500 and final charges belong to original Connector emissions. Held batches never reach the business adapter. |
| `examples/agent-http.rkg` | Replaces the historical `docs/examples/agent-http-candidate.rkg`: six scenarios, projected Responses struct directly on HttpClient, original measures, no downstream extractor or invented inference idempotence. See [profile and qualification](openai-responses-hydration.md). | No pre-existing executable golden for this variant. Its first traces belong to the common blessing; the approved profile uses existing plan attributes and adds no format key. |

Goldens are not updated during this migration. Their old bytes remain visible
until the common annotation/Retry inventory is complete and the one reviewed
blessing runs. The three non-Retry witnesses keep their separately inventoried
candidate movements; no additional change is authorized here.

The Loop-specific two-tenant debugger fixture remains a Loop regression:
it teaches authored state progression and is not silently replaced with Retry.
Native Retry inspection has its own fixture and tests. The subsequent
[browser follow-up](retry-c4-inspection.md#5-local-browser-follow-up--17-september)
replaces the flagship screenshots and GIF with current native Retry snapshots
at transitions 0/20/34, without changing the versioned causal goldens.

## Qualification

- Asymmetric Alice/Bob Calls, identical prepared requests across Alice's
  attempts, distinct Call identities, one published result each, fresh idle
  generation; batch and interactive journal equality.
- Six webhook scenarios, unchanged ACK/validation/deduplication/privacy
  rules; original 0.01 and 0.02 USD values on held/published Connector emissions,
  no downstream extraction or computed total.
- HTTP follow-up: exact per-job/session metadata, absent evidence versus
  reported zero, fail-closed incompatible schema and secret-free retained
  records under the approved profile now pass. The six Lab scenarios are
  complemented by complete JSON decoding and a real local Retry endpoint;
  this is not a paid OpenAI invocation. See the linked qualification ledger.

The two public CLI examples pass all eight scenarios (two tenant/lifetime,
six webhook). Focused Rust tests pass: `retry_showcase` (1),
`webhook_showcase` (4), `loop_control` (18), `site_content` (6).
They include deterministic complete batch/step journals and personal-data
redaction at every webhook transition. Lab callers now provide the original
program hash, required to validate Retry admission; an identity-free helper
must not produce an unrelated boot refusal that masquerades as a privacy test.

The broader run found one more such helper in `recording/tests.rs`, while
reconstructing the public keyed example. It now uses `run_program` too;
`keyed_retries_loops_and_bounded_joins_survive_recording_and_prefix_reconstruction`
passes separately, bringing the focused total to **30**. No producer, recovery
algorithm or recording codec was changed to make that test pass.

Formatting, strict workspace Clippy, rustdoc, 15 layout tests and the final
site-content check pass. No browser pass is claimed. The complete
`cargo test --workspace --all-features --no-fail-fast` was interrupted after
20 minutes, at `loop_host_exit`: several harnesses spent minutes with no
output and zero observed CPU before running. Read-only `--list` loading did
not make this a practical full pass; those loader processes were stopped too.
The incomplete transcript contains **659 passed, 9 failed, 0 ignored, 24
completed groups**. Eight failures are unblessed identity/golden expectations;
the ninth is the recording helper corrected and re-run above. The remaining
workspace suites did not complete, so this is not a green workspace result.
Transcripts are under `/tmp/rastok-retry-gallery.XKMQTF/`.

The versioned golden comparison remains deliberately unblessed. The former
traces already differ from the unedited examples; the pages expose that fact
instead of advertising a passing `--golden` command. No core/runtime code is
changed by this example migration. The tiny embedded margins from C4 are not
remeasured or improved by it.

No public release, paid API call, browser review or golden blessing is implied
by this inventory.
