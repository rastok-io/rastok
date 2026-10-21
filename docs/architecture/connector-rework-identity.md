# Connector rework — proposed Process identity inventory

Date: 22 October 2026. Baseline measured on 21 October.
Status: **proposal, not implemented or approved**.
This is the A-T5-4 review boundary of the T5/T9 series. The mechanical
Connector reorganization does not authorize this identity movement.

## Decision requested

Use one exact Process Contract, `std/process@6`. The statically selected
`mode = job | service` determines the resolved direction, exchange and
lifecycle. Omission still materializes `mode = job` before semantic hashing.
Neither `std/process@4` nor `std/process@5` is reassigned or accepted as an
alias. Old artifacts must be regenerated, not converted.

The open format choice is independent of the Contract major:

| Choice | Consequence |
|---|---|
| Package format 22, recommended in the brief | One new approval line; format 21 refuses before body decoding. Every package checksum changes, including pure gauge, because the version is covered. The body schema and key allocation do not change. |
| Reapprove package format 21 | No version-only movement for non-Process packages, but the prior once-per-format approval rule needs an explicit exception. Old Process packages refuse by unsupported Contract rather than package version. |

Neither choice renumbers the public format to 1 now. That remains the
separate pre-publication TODO. Diagnostic recording remains version 4:
no record tuple changes. Its embedded package must satisfy whichever
package version Yoann approves.

## What remains exactly the same

- Job is an outbound Call: one request, ordered frames, one terminal exit.
- Service is a resident Session: one exact resource per Session, many
  messages, one terminal lifecycle, bounded instance closure.
- Call and Session ports, codecs, settings, defaults, terminal cases,
  timeout and shutdown rules retain their current meanings.
- Loop and Retry host restrictions do not broaden. Service does not gain
  Call or Retry semantics from sharing a Contract id with job.
- Effects start only after the opening journal commit. Admission,
  correlation, cancellation, polling order and recovery are unchanged.
- Physical implementation selection remains deployment configuration,
  outside semantic identity and outside the journal.

`ProfileProtocol.id` disappears. The existing mode declaration retains its
direction and exchange facts. The compiler resolves those facts before
constructing the Connector Node, rather than replacing an already selected
Contract. Independent core validation checks the same mode word and facts
from the shared standard definition; it does not consult the hosted catalogue.

## Why an identity changes without a behavior change

The exact ContractId is part of canonical HIR and of the Connector's
transported description. Replacing its text changes the semantic hash of
every program containing Process. This is not a claim that its input/output
behavior changed; it is the declared change in the identity material.

The Retry opening additionally records both the originating semantic hash
and exact Contract. Its causal projection therefore changes for a Process
Retry, including `keyed-retry`. Ordinary Call and Session records do not
become a different protocol merely because the Rust module moved.
Every affected trace must be measured; a package-hash change is not by itself
permission to replace an otherwise unchanged causal trace.

## Baseline witnesses

The audit reads the tracked package, independently decodes it, verifies a
canonical encode/decode round trip, and inspects actual Connector Nodes.
It does not infer Contract dependencies from a textual search in CBOR or
in the hex representation of a package.

| Tracked witness | Body bytes | Decoded Contracts | Semantic hash |
|---|---:|---|---|
| `crates/rastok/tests/golden/echo.rkp` | 934 | `std/process@4` | `562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813` |
| `crates/rastok/tests/golden/jsonl.rkp` | 979 | `std/process@4` | `0aeff047b9efde5c6dc4efd6f411c5541a786fb24f816108e110eb29d00e026f` |
| `crates/rastok/tests/golden/gauge.rkp` | 230 | none | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` |
| `spikes/rp2350-emu/echo.rkp` | 934 | `std/process@4` | same as echo |

| Witness | Current package checksum |
|---|---|
| Echo, including spike | `c24806e6918130b4d8d35c6660d3f254c598d17c41aac9930f6545d4cf748dc0` |
| JSONL | `6d03694522f95d6ea2b8cf3654938ee73f65e5fa75c6dae8bcfc83943ed733d6` |
| Gauge | `3acf7c7f98b481b4d779f99a234ccea6173136e6e2a79417da99164f968b0196` |

The seven Process source examples are `echo`, `jsonl`, `csv-count`,
`keyed-retry`, `agent-gateway`, `agent-dialogue` and `agent-chat`.
The chat source must compile with `examples/` as its asset root, exactly
as the CLI and its integration test do; its HTML is not edited.
Each source compiled successfully, and its package independently decoded and
re-encoded byte-identically. The table below records that read-only baseline;
scenario record counts and causal seals are inventoried separately below.

| Compiled source | Body bytes | Semantic hash | Package checksum |
|---|---:|---|---|
| `examples/echo.rkg` | 934 | `562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813` | `c24806e6918130b4d8d35c6660d3f254c598d17c41aac9930f6545d4cf748dc0` |
| `examples/jsonl.rkg` | 979 | `0aeff047b9efde5c6dc4efd6f411c5541a786fb24f816108e110eb29d00e026f` | `6d03694522f95d6ea2b8cf3654938ee73f65e5fa75c6dae8bcfc83943ed733d6` |
| `examples/gauge.rkg` | 230 | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `3acf7c7f98b481b4d779f99a234ccea6173136e6e2a79417da99164f968b0196` |
| `examples/csv-count.rkg` | 1311 | `281f031c56dc947596d6ab51c10b71eb417dc902900d899bb646264a45846816` | `2ce53c6ba7e57fcd13b9efe8db1fa8253b27b7e0e14257490dd005f111997d06` |
| `examples/keyed-retry.rkg` | 2008 | `3a4096b235c40700a6f162d4eeb21fc2a08f59bf1563aa4dfa2b24af8878b7e6` | `30b6fa11543f2a6486dbe82637440d8f9c78d7196104ffd9b51bd46910170ef0` |
| `examples/agent-gateway.rkg` | 2903 | `0e0f027d6dc5c31d65bde7c840be8c0f9a8bc68028d4d28da7180a2b06c3083d` | `476669a359c0d76e5e2d32ad749b0a1141cff6c59cc55e672e91b861a3fd3599` |
| `examples/agent-dialogue.rkg` | 2625 | `8daad6d576ae7687b000c0385dd4664aa6f4763afcafb41a94da0618fa923564` | `4b8830a63c191cacb4c3f158edab973af03758091f131749afad9001ba7a18e7` |
| `examples/agent-chat.rkg` | 17090 | `5ee9755a1929602f108714469d8a47993a042ecce076d727f0967dae745c977f` | `a8987eef8df235e023ddd522b386cae48d0b9afd0576fe052950c7cf97069805` |
| `examples/file-pipeline.rkg` | 2261 | `3a440d6a3fcb13bc30bb5d7bb97f7062db6cbdf579d4dcae2196a77ce6b9232b` | `80e2db934134b97527729c049619be1dfd7807170e6913b4fb233600768fac73` |
| `examples/webhook-to-partner.rkg` | 7370 | `0f71ef3c737707827fc6d48add0b392f8743bce594a8ea0a530ae3820220680c` | `627b535994fcd5cc1823cc522224671d2b3694d28b8c71cdc6f37d7120db18f2` |
| `examples/sensor-alarm.rkg` | 1193 | `2b7994353b59abd40ccbb012e61b97011d377c86555e575fdc7c18c8c20caf04` | `7f97495e9b4eaa0ae709e008d45549b434e2775f7b292310cae53bb3f9d8035c` |
| `examples/agent-http.rkg` | 7280 | `b34ca7e9fe4a2943fc89774968ae3cf99d9fcc6d692d459c76f778f5b96a248c` | `e1c0a20b173a6c176db53b3b67f8e90807ead7e1f94db92a6aca0f74f4d70420` |

Gauge is the negative semantic witness. File-only and HTTP-only examples
also retain their hashes and causal traces. A version-22 checksum movement
for these packages must be recorded separately as container-only.

## Baseline causal traces

The read-only gallery audit reproduced all 28 versioned scenario traces.
Both the record count and causal seal matched the stored trace in every
case. Webhook and HTTP-model scenarios used the public test value
`lab-only-not-a-credential` and its declared `fixture-v1` version;
no real provider credential was used.

| Trace below `crates/rastok/tests/golden/` | Records | Current causal seal |
|---|---:|---|
| `gallery/echo/smoke.roundtrip.trace` | 8 | `566aed6f75b16361988aae27ed8af674c2f8648cca11118829b96e8c1d65efff` |
| `gallery/echo/smoke.filtered.trace` | 4 | `7cd9e23c6af5c004268c7f590ea012d47d06b1d64949a333e8220c5db4b6978f` |
| `gallery/echo/smoke.silence_expires.trace` | 8 | `4a1c315c7132de01c8bf5b23dbf91cb17a88084a6f7db6030377675782eb7d99` |
| `gallery/file-pipeline/files.copy.trace` | 12 | `50b6357a920f562ed961f967f3fd8bc92b5e91b842e420393244291b24819721` |
| `gallery/file-pipeline/files.empty.trace` | 8 | `5c59610ead25df512f539866ee94ee9ce0139a1766a445d7442471b5731fe281` |
| `gallery/file-pipeline/files.utf8.trace` | 12 | `22d12627046efa6bd31def4160edbde5d69e24cce3758fd0afa9602c0a142b92` |
| `gallery/file-pipeline/files.directory.trace` | 6 | `e993283c1d730ef4d83b98c6a87debb5465b07a571577cde5b0e9088a59902d5` |
| `gallery/two-tenants/ui_tour.two_tenants.trace` | 29 | `11bea6acfff27b4efc59dfe320694aa10462091bf30357b4f3b7472820c7f8b6` |
| `gallery/two-tenants/ui_tour.lifetime.trace` | 30 | `1be7011e12b84ce812cfb2a85d255da66df4a73825dded8ac8e138eaf99a21fd` |
| `gallery/webhook-to-partner/webhook.nominal.trace` | 18 | `d6dc13c583c51fe8274e2c49199b2850802d6bb95bc0e4ca69868ab73ee98c52` |
| `gallery/webhook-to-partner/webhook.timeout_then_success.trace` | 23 | `9b8e06771cdc344c7b2c8d9b586a81d3a1daa496dfe37abf7e203b5195b186fb` |
| `gallery/webhook-to-partner/webhook.partner_dead.trace` | 23 | `691fcc58e3911ec85cbab67b757d6db7c167293da68ed1face122d3123728d44` |
| `gallery/webhook-to-partner/webhook.duplicate.trace` | 35 | `e6eaa9fc680ca6fceb7c719fd379ce674ceb540ffeeb2c83172c13a259cf71e3` |
| `gallery/webhook-to-partner/webhook.invalid_payload.trace` | 9 | `079544d241039cde9d6c2bd429f865dd0a96f0b9e0196f671791f6075a18a3fd` |
| `gallery/webhook-to-partner/webhook.http_500.trace` | 23 | `075f7304e3c4ace008d9a9820a747e62887eb580294f4e51bd29919ff248b6db` |
| `gallery/sensor-alarm/sensors.hysteresis.trace` | 11 | `3db3072dbbc8013cb3ae7292f674d51e7662337020a95f26554a49fd51fa7eb1` |
| `gallery/sensor-alarm/sensors.silent_sensor.trace` | 8 | `b273bd2353eeb7b8ad09b2b5a55556ac3f6a5b8d5e77717a36a63d9690658f04` |
| `gallery/sensor-alarm/sensors.virtual_day.trace` | 4 | `9de2f8f4a2c1ad627873679b88408b9d90f101d99ba146d4e2362cd30ae4af75` |
| `gallery/agent-gateway/scripted_model.alice_and_bob.trace` | 50 | `3ff4aaeaa24d95e7a0a91fd3b1a9f67c258fd0ac612521da7439d585f997011b` |
| `gallery/agent-gateway/scripted_model.fresh_child.trace` | 27 | `1e53edc0b605cf1b4da893d4373e7b0a0ad2dd580327a55acefb9852c93cc5fb` |
| `gallery/agent-gateway/app_server.two_conversations.trace` | 89 | `b93a418f0f68e4d85d5627e652e11bd04254d9aca2465385839e86c963aeff23` |
| `gallery/agent-gateway/app_server.unknown_method.trace` | 12 | `6f77b66a3598eb93ab531790abf971c3f9ec498cf6f4325af74d01f2a9ade2a5` |
| `gallery/agent-http/responses.two_sessions.trace` | 31 | `53fde566fb48ea019d3f0b337349482361780d44c0520e3714578b29dd0d4c72` |
| `gallery/agent-http/responses.missing_usage.trace` | 19 | `cc24772b564cca3294108068d6e7a9d08eb0e93113f88897e783649c6cd80774` |
| `gallery/agent-http/responses.observed_zero.trace` | 12 | `52832dc9b8508a8dd8676a4f11677bf042aa268058dc0566b8a36dab9ca8c480` |
| `gallery/agent-http/responses.schema_failure.trace` | 10 | `451fa758b3a2e61ca4566d5be0ead739b4986648d679a602d0130d54680efb9a` |
| `gallery/agent-http/responses.transport_failure.trace` | 10 | `fd8153806cc896e4c81321bb1ac6b021ac77f816f8552fdf9cee56b045214312` |
| `gallery/agent-http/responses.http_errors.trace` | 18 | `7b3107f2fe33c4ad15f6e42c7c7912e49ca6839c82a98acd325d732559dfaa33` |

JSONL additionally pins 8 records with seal
`7afbcaaba2f085995ac7262e256058c2f3ed459fce10a2ce8f5dd6a3617dc099`;
gauge pins 4 records with seal
`0f1dd432e3e4e5f4b3bc20a2f86e024a67e620db51f8e41575eb7ae3cd2a9528`.
The generated Retry spike currently has semantic hash
`d8e89d6dc345e7c6199ee5ffc5664b682e09040a8589c6e934016bbe1c32581f`.
The unit and integration gates independently check these existing pins.

Only the Process-dependent traces are candidates for movement. The
non-Process Retry examples `webhook-to-partner` and `agent-http` are
negative causal witnesses too: their originating semantic hashes do not
change with the Process Contract or a package-container version.

## Pins and artifacts to qualify

- The four package files above, regenerated from source, not byte-patched.
- Process-dependent gallery traces under `crates/rastok/tests/golden/gallery/`
  and the reference traces and documented hashes under `docs/examples/`.
- Hash and Contract pins in `contracts`, `literals`, `plan_package`,
  `plan_fields`, `plan_validation`, `execution`, `gauge_fixture`,
  `jsonl_fixture`, `instance_stop`, `retry_plan`, `retry_runtime` and
  `site_content`; manifest and Process unit tests; the CI echo hash.
- The generated Retry spike package and its recovery workload. The Retry
  opening binds the exact plan hash; recovery must reject a mismatch.
- Recording tests, including the existing echo record-tuple digest
  `6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d`.
  There is no tracked `.rkrec` file to convert. A newly captured recording
  embeds the newly approved package, without changing the record codec.

The implementation commit must replace this proposal's candidate columns
with measured old/new values, one semantic reason for each movement.
No functional assertion is weakened and no test is ignored.

## Proposed normative amendment

After Yoann's approval, add D-2026-10-22-1 to `decisions.md` and align
`stdlib.md` §1.1 and §3.3 and the status heading of `identity.md`:

> Yoann decides that Process is one versioned Contract, `std/process@6`.
> Its statically selected mode determines direction, exchange and resource
> lifecycle. `mode = job` is the materialized default and remains an
> outbound Call; `mode = service` remains a resident Session. This replaces
> the mode-specific identity clauses of D-2026-09-14-1. It does not change
> either mode's codecs, ports, terminal obligations, instance closure,
> timeout rules, Loop/Retry restrictions or external-effect ordering.
> The package-format choice is the option explicitly approved by Yoann
> in this inventory. No old Contract alias or compatibility reader is added.

This quoted text is a proposal, not an addition to the settled-decision
registry. Until approval, runtime code keeps `@4`/`@5` and format 21.

## Approval and measurement gate

First obtain approval of the Contract id, amendment text and format number.
Then change the implementation and measure candidates without overwriting
witnesses. Add their actual identities and reasons here before the single
regeneration pass. Finally run the complete workspace, site, no_std and
Cortex-M33 gates, including code and peak heap for echo and Retry recovery.
The existing 128 KiB heap and 1 MiB code gates do not move.

No push, tag or publication belongs to this operation.

## Qualification of this proposal

This documentation-only checkpoint passed formatting, strict all-target and
all-feature Clippy, rustdoc with warnings denied, and **1,598 tests in 61
groups, zero failures and zero ignored tests**. Site content passed 12/12,
browser tests 35/35, all three no_std targets passed, and both Cortex-M33
workloads ended in `SPIKE_OK`. Their heap/code figures are unchanged from
the [A-T5-3 qualification](connector-rework-qualification.md): echo
111,404 / 971,868 B, Retry with recovery 129,188 / 1,043,764 B.
All 73 protected artifacts still match their baseline digests.

One shell run was interrupted because of repeated host executable startup
pauses. The subsequent IDE-terminal run passed the four checks, site checks
and no_std, then stopped on an unavailable `rg` in the local qualification
script after a successful echo image. The extended checks were rerun to
completion with the script's status check using available `grep`; no source,
timeout, test assertion or security setting changed to obtain these results.
This qualification validates the unchanged implementation, not the proposed
Process identity movement. No push, tag or publication occurred.
