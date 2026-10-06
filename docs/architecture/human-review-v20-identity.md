# Human-review cutover — v20 identity inventory

> Opened 2026-10-06, from qualified lot 2 at `3bdd807`, on `rework/v20`.
> Blessed and qualified on 2026-10-07 after Fable's review and Yoann's approval.
> The final pre-blessing inventory, including all three complements, was
> committed in `8bc54fd` before any stored witness was replaced.
> The single witness blessing and its complete gate are committed in `18a0d44`.
> The final qualification below supersedes the dated candidate gate results.

## Invariant and scope

One pre-public format movement implements A-T1-1b, A-T1-2, A-T1-6 and
A-T1-9 together. A-T1-7 specifies the future decoder attestation and allocates
its identity domain only; no decoder registry or availability mechanism is
implemented in this cutover. Package 20 and recording 4 have no reader or
converter for an earlier format. Numbering the public format 1 remains a
separate pre-publication TODO, not a second semantic movement.

The semantic rules of Loop, Retry, Closing and Driver exchanges do not change.
Compact domain bytes change their derived identifiers, not their transitions.
Unsealed deadline instants remain outside causal seals and retain their exact
recovery meaning. No general journal index, Saga or UI feature belongs here.

## Measured baseline, before code changes

The existing `package_ledger` tool reproduced each committed package and
trace at the qualified baseline. Body sizes exclude the package header.

| Witness | Semantic hash | Body bytes | Package checksum | Roundtrip causal seal |
|---|---|---:|---|---|
| echo | `878a34b77a7ac6500b6452b903f633ba39848c47ed0ac387ce1597f78a785699` | 964 | `4186227832f5fd8c0d3d6adf205ae9c187fbf217c500310f31fb1dbb5e6a74c0` | `21de00532e754e42a47fc8855895e9358865a1b82c4cf9d2ad47878f5af919d9` |
| jsonl | `3f1b40ec3457d1b371289d9cac615df0394ef1dffe8b52c7f36424961a0dd064` | 1008 | `9562d0b55923badb82264d6ac08817bfe8e280d6dda4412dc3a130f8d1432932` | `eb8aeb71f534bb69cde2cd3ae3a5562f0c70e9061a1ba2cfd6d2617961cb962d` |
| gauge | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | 249 | `efb01e53be0025e53505f712cd930797bd4729bb101f791ca0bf365b8a1a4ddb` | `6f463270fbc011a8f355c93505ea426c5697149ca69c692033fc2f88d2d9346c` |

Final lot-2 Cortex-M33 baselines: echo peak heap 112,656 B, `.text + .rodata`
971,024 B; Retry with full recovery peak heap 130,560 B, code 1,039,012 B.
The 128 KiB heap and 1 MiB code gates are unchanged. Candidate measurements
must use the same workload (one operation, three Calls, sixteen four-byte
frames per channel per attempt), regenerated under the candidate format.

## Reasons that may move witnesses

| Movement | Semantic hash | Package | Causal trace / recording |
|---|---|---|---|
| Contract and protected annotation ids use `std/` | Affected Contracts/annotations only | Corresponding text and checksum | Original facts keep their meaning |
| Graph-born values carry `ccbor`; raw bytes never masquerade as those values | No source-HIR change by itself | Encoded graph-born values where transported | DATA discriminator changes |
| Four `meta.sys` fields; no role | Programs reading the removed field refuse | No unsupported field is transported | Compact envelope form and recording tuple |
| Raw materialization follows the reading port type | Unchanged source stays unchanged | No additional materialization policy | Non-ASCII byte counts are corrected; invalid text refuses typed |
| Compact active registries | Only a semantic domain explicitly enumerated below | Active keys/tags and checksum | Derived ids and compact causal fields |
| Package 20 / recording 4 | Not a semantic-hash input | Header/version/checksum | Recording version, not an extra causal fact |

Gauge is the negative **semantic-hash** witness: no Contract, annotation or
source change may salt its HIR hash. It is not a promise that its trace stays
byte-identical: compact envelope/record keys and the explicit ccbor tag apply
to every trace. Package body/header movements are measured separately.

## Candidate registry and qualification

The following allocation is fixed before changing the encoders. The old
numbers occur only in this migration report, not in the living v20 registry.

| Registry | Old → candidate |
|---|---|
| Envelope | id 1→1; parent 2→2; origin 3→3; graph version 5→4; DATA 8→5; app metadata 9→6; role/mime/unimplemented tier-2 allocations removed |
| Record discriminator | 10→7 |
| Ingress fields | envelope 20→8; connector path 21→9; sequence 22→10; replica 23→11; input 24→12; routes 25→13; session 26→14; source 27→15; inbound Call 28→16 |
| Transaction fields | owner 30→17; generation 31→18; trigger 32→19; consumed 33→20; join inputs 34→21; delta 35→22; emissions 36→23; deadline ids 37→24; routes 38→25; cause 39→26 |
| Egress fields | emitting id 40→27; connector path 41→28; key 42→29; digest 43→30; status 44→31; closure 45→32 |
| Other active fields | edge 47→33; parent generation 48→34; close scope 49→35; session exit 50→36; session fact 51→37; inbound Call exit 52→38; logical time 54→39; Loop 55→40; deadlines 56→41; instance commands 57→42; Retry 58→43; Call closed-at 59→44 |
| Removed field allocations | unused edge cursor 46; old Loop 53 |
| Identity domains | Root 1→1; Emission 2→2; LoopAdmission 6→3; RetryAdmission 7→4; Call 16→5; Session 17→6; SessionWrite 18→7; LoopRun 20→8; LoopCommand 21→9; RetryOperation 22→10; Delivery 32→11; InstanceLifetime 33→12; InstanceClosing 34→13; InstanceStop 35→14; Decoder newly allocated 15 (attestation implementation deferred) |
| Deadline kinds | Call 1→1; join 2→2; lifetime 3→3; recurrence 5→4; drain 6→5; settlement 7→6; inbound Call 8→7; handshake 9→8 |
| Extensions | App 2→1; Loop 3→2; Retry 4→3 |
| ConnectorSpec | contract 1→1; behavior 3→2; attributes 4→3; roles 6→4; exchange 7→5; idempotent 8→6 |
| Annotation applications | sensitive 1→1; secret 2→2; personal 5→3; declared 6→4 |
| Metadata write target | DATA 1→1; app 2→2; ext 5→3 |
| Causal Call-status projection | opened 1→1; closed 2→2; abandoned 4→3 |
| Materialized value tags | bool/int/long 1/2/3 unchanged; double 5→4; bytes 7→5; text 8→6; null 9→7; decimal 10→8; date 11→9; timestamp 12→10; timestamp-ntz 13→11; time 14→12; uuid 15→13; duration 16→14; struct 17→15; map 18→16; array 19→17; enum 20→18; union 21→19; uint 23→20; interval 24→21. Float/number/variant slots 4/6/22 had no emitted Value form and are removed; logical types remain distinct in DataType. |
| Lifecycle tags | Start/Stop 1/2 unchanged; Next 4→3; Timeout 5→4; Evict 6→5; Recover 7→6; Closing 8→7; Reply 9→8; Exit 10→9; Retry 11→10; unsupported compensation allocation removed |

Ccbor DATA's encoded bytes and materialized value are representation states
of the same payload and compare equally. Raw bytes remain a different payload,
even if they happen to be a valid ccbor value. The record codec stores both
ccbor states as `[2, canonical_value_bytes]`; the reader verifies canonical
form and value consistency before materializing. No guessed UTF-8 or decoded
cache can erase the raw/ccbor distinction.

The lossless record codec has a separate Call-status tuple: Opened 1,
Response 2, Timeout 3, Abandoned 4. All four forms are active and unchanged;
the causal projection above deliberately folds Response and Timeout into
Closed. These two registries do not share discriminants.

## Measured candidate, before blessing

The existing package ledger and its read-only gallery mode compiled and ran
the candidate on 6 October. The gallery uses its checked-in deployment
profiles with explicitly synthetic secret values, never real credentials.
No old packet was replayed: existing golden lines were inspected only as
generic canonical CBOR to verify their own stored seals.

| Package witness | Body bytes | Semantic hash | Package checksum |
|---|---:|---|---|
| echo | 961 | `562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813` | `a9f7b7c5c11f68ccedc4463edb803774681b3125d695fa67889e83228fbc964d` |
| jsonl | 1005 | `0aeff047b9efde5c6dc4efd6f411c5541a786fb24f816108e110eb29d00e026f` | `801f64797fd51a698de7cbaefe1e06e4281f0b92d9441b396a6a85d894cf5148` |
| gauge | 249 | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `342c5147300138d74fd7ffe4d54ebd8859c7db3578afceba585efd0bd64d2746` |

Echo and jsonl change semantic hash because the Process Contract name is
`std/process@4`; their bodies shrink by three bytes. Gauge's semantic hash
and body size do not move, but its body includes the App Extension's compact
tag, 2 → 1; its body is therefore not byte-identical. All three checksums
include the package-20 header and the corresponding body.

Their roundtrip traces retain 8, 8 and 4 records respectively. New seals:
echo `566aed6f75b16361988aae27ed8af674c2f8648cca11118829b96e8c1d65efff`,
jsonl `7afbcaaba2f085995ac7262e256058c2f3ed459fce10a2ce8f5dd6a3617dc099`,
gauge `0f1dd432e3e4e5f4b3bc20a2f86e024a67e620db51f8e41575eb7ae3cd2a9528`.
The reasons are compact field/value tags, compact derived-identity domains,
and graph-born ccbor DATA; no new causal step is introduced.

### Gallery evidence

All 28 scenario assertions pass independently of the old-golden comparison.
The 27 existing scenarios retain their record counts. `files.utf8` is the
new A-T1-1b regression: `é\n` is three bytes on a `bytes` port, not two
characters. No trace file has been created for it yet.

Every listed existing trace moves for the shared compact representation.
Process/File/network examples also carry `std/` Contract pins; Retry traces
retain those pins in Opened facts. The sensor example has no Connector:
its representation moves but not its program's semantic identity. No
scenario's logic or expected result is relaxed to match a seal.

| Trace witness, relative to tests/golden | Records, stored → candidate | Stored seal | Candidate seal |
|---|---:|---|---|
| gallery/echo/smoke.roundtrip.trace | 8 → 8 | `21de00532e754e42a47fc8855895e9358865a1b82c4cf9d2ad47878f5af919d9` | `566aed6f75b16361988aae27ed8af674c2f8648cca11118829b96e8c1d65efff` |
| gallery/echo/smoke.filtered.trace | 4 → 4 | `f76107eb428b4d2ba321123370588b7f3eef444760b5e9e4f989e64eb3a3c5fe` | `7cd9e23c6af5c004268c7f590ea012d47d06b1d64949a333e8220c5db4b6978f` |
| gallery/echo/smoke.silence_expires.trace | 8 → 8 | `4f4950c1930a504df7ba677c5002f232361b4156ea7ab9741eabeacbadab9fed` | `4a1c315c7132de01c8bf5b23dbf91cb17a88084a6f7db6030377675782eb7d99` |
| gallery/file-pipeline/files.copy.trace | 12 → 12 | `2930005f68e37edb8077e36fa14f672770413d22e999390c81d72b59a8bcddac` | `50b6357a920f562ed961f967f3fd8bc92b5e91b842e420393244291b24819721` |
| gallery/file-pipeline/files.empty.trace | 8 → 8 | `84d0e22e3feff92533afd864cd7370e5472b6a4baddeb261c1c67ac60cfca097` | `5c59610ead25df512f539866ee94ee9ce0139a1766a445d7442471b5731fe281` |
| gallery/file-pipeline/files.utf8.trace | new → 12 | `none` | `22d12627046efa6bd31def4160edbde5d69e24cce3758fd0afa9602c0a142b92` |
| gallery/file-pipeline/files.directory.trace | 6 → 6 | `615a5a7a210cebc3f28d0ed2b12613f5e2302af10a89a90f38274ff0ef88310d` | `e993283c1d730ef4d83b98c6a87debb5465b07a571577cde5b0e9088a59902d5` |
| gallery/two-tenants/ui_tour.two_tenants.trace | 29 → 29 | `5da6e21cbc9c31dcec0cb47f681e85d70e6f33c9c93db700cfbb502b31eb1fc2` | `11bea6acfff27b4efc59dfe320694aa10462091bf30357b4f3b7472820c7f8b6` |
| gallery/two-tenants/ui_tour.lifetime.trace | 30 → 30 | `28b8b18fb75253db9eb6868f1a111e80534dcb2fd258ab1c9d4c819bbf0ef842` | `1be7011e12b84ce812cfb2a85d255da66df4a73825dded8ac8e138eaf99a21fd` |
| gallery/webhook-to-partner/webhook.nominal.trace | 18 → 18 | `fe27ba52836b5060ed21ab6369836f65b2d01e0330e17be0c3f5627959de9179` | `d6dc13c583c51fe8274e2c49199b2850802d6bb95bc0e4ca69868ab73ee98c52` |
| gallery/webhook-to-partner/webhook.timeout_then_success.trace | 23 → 23 | `d261ea4b1575baf1ee6b8386309fa390eb0489d077f6fb3fb0972b39737bf46b` | `9b8e06771cdc344c7b2c8d9b586a81d3a1daa496dfe37abf7e203b5195b186fb` |
| gallery/webhook-to-partner/webhook.partner_dead.trace | 23 → 23 | `ffe3f847964b147a4dbae40736d7ec160cf2ffc85aeda813e44fb2b01a6aa941` | `691fcc58e3911ec85cbab67b757d6db7c167293da68ed1face122d3123728d44` |
| gallery/webhook-to-partner/webhook.duplicate.trace | 35 → 35 | `b6804d852110f84f45d10fc690724683d073bb417b6db91f3d349b9a2242577f` | `e6eaa9fc680ca6fceb7c719fd379ce674ceb540ffeeb2c83172c13a259cf71e3` |
| gallery/webhook-to-partner/webhook.invalid_payload.trace | 9 → 9 | `2b9196216ce5d95a715793f0336631ca4e1edc6d94d521f3a987a6e6b897f232` | `079544d241039cde9d6c2bd429f865dd0a96f0b9e0196f671791f6075a18a3fd` |
| gallery/webhook-to-partner/webhook.http_500.trace | 23 → 23 | `60427925392f11a772445aa3e506a806e20829d52dcd71468dec63ea67f6a807` | `075f7304e3c4ace008d9a9820a747e62887eb580294f4e51bd29919ff248b6db` |
| gallery/sensor-alarm/sensors.hysteresis.trace | 11 → 11 | `bea8ce2efbae1748edbc8b456b2739900f61720f880e61e46856413ec39ff9a4` | `3db3072dbbc8013cb3ae7292f674d51e7662337020a95f26554a49fd51fa7eb1` |
| gallery/sensor-alarm/sensors.silent_sensor.trace | 8 → 8 | `1b0e8baafe38d23d5c12f13b91dd3b548abe1969e8197a89fdf5da2ca646b743` | `b273bd2353eeb7b8ad09b2b5a55556ac3f6a5b8d5e77717a36a63d9690658f04` |
| gallery/sensor-alarm/sensors.virtual_day.trace | 4 → 4 | `bc4197008499f514ce7f2fcf6058d147f05b98c62577b64fbddb4303c00a6112` | `9de2f8f4a2c1ad627873679b88408b9d90f101d99ba146d4e2362cd30ae4af75` |
| gallery/agent-gateway/scripted_model.alice_and_bob.trace | 50 → 50 | `6c88fc5d9a603db82754e884402fd8bf511e8b5cf37ffb731a047e635a45aa54` | `3ff4aaeaa24d95e7a0a91fd3b1a9f67c258fd0ac612521da7439d585f997011b` |
| gallery/agent-gateway/scripted_model.fresh_child.trace | 27 → 27 | `0585b7852189fa1a8d3979ddcbb1fbabdaa6655422ba57f4a7a5742d73065897` | `1e53edc0b605cf1b4da893d4373e7b0a0ad2dd580327a55acefb9852c93cc5fb` |
| gallery/agent-gateway/app_server.two_conversations.trace | 89 → 89 | `8608a793a365d92010a8d85997a21bd38004c50ee7692597b60fcead1396ef85` | `b93a418f0f68e4d85d5627e652e11bd04254d9aca2465385839e86c963aeff23` |
| gallery/agent-gateway/app_server.unknown_method.trace | 12 → 12 | `7613209afbdff695150864c096b4761a9d4ac3a1d36413436e4e102e0cd520b5` | `6f77b66a3598eb93ab531790abf971c3f9ec498cf6f4325af74d01f2a9ade2a5` |
| gallery/agent-http/responses.two_sessions.trace | 31 → 31 | `410683daf1c6543ddff7ee6402895ee9eda74f4d68b44216b0caff902ad5f989` | `53fde566fb48ea019d3f0b337349482361780d44c0520e3714578b29dd0d4c72` |
| gallery/agent-http/responses.missing_usage.trace | 19 → 19 | `368617955fa96f6f1f51c22f205b3b3e5ae49855d3503df3e4fd149b606dde06` | `cc24772b564cca3294108068d6e7a9d08eb0e93113f88897e783649c6cd80774` |
| gallery/agent-http/responses.observed_zero.trace | 12 → 12 | `fe0307addd028e09ddb56a27df8f65ac6afa02d854904aaed4cb109af92351b7` | `52832dc9b8508a8dd8676a4f11677bf042aa268058dc0566b8a36dab9ca8c480` |
| gallery/agent-http/responses.schema_failure.trace | 10 → 10 | `c80f66ccd6242fb2872af5fb956e03968497b887db9a1071ebbfa039b01605e6` | `451fa758b3a2e61ca4566d5be0ead739b4986648d679a602d0130d54680efb9a` |
| gallery/agent-http/responses.transport_failure.trace | 10 → 10 | `2382de8b0715e6136e37f516ab8067771ef1137c9c21049775448bce72be8032` | `fd8153806cc896e4c81321bb1ac6b021ac77f816f8552fdf9cee56b045214312` |
| gallery/agent-http/responses.http_errors.trace | 18 → 18 | `5437810490b743e54ab028631f1f02821df600e1f2680e36ff9e6fdc6bb2ce38` | `7b3107f2fe33c4ad15f6e42c7c7912e49ca6839c82a98acd325d732559dfaa33` |

These gallery programs have no separately committed package witness except
echo. Candidate package measurements are supplied to make the new trace's
program association explicit; there is no invented old package checksum.

| Program | Body bytes | Candidate semantic hash | Candidate package checksum |
|---|---:|---|---|
| echo | 961 | `562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813` | `a9f7b7c5c11f68ccedc4463edb803774681b3125d695fa67889e83228fbc964d` |
| file-pipeline | 2651 | `3a440d6a3fcb13bc30bb5d7bb97f7062db6cbdf579d4dcae2196a77ce6b9232b` | `54c996d846782afd9ba7f1849d5327e609d32a609960cd906f30db37a7490ff2` |
| keyed-retry | 2483 | `3a4096b235c40700a6f162d4eeb21fc2a08f59bf1563aa4dfa2b24af8878b7e6` | `d3121b7f9f15f20a18efc46bd6a42571d3cfc2036a20329e5be105d1030cb964` |
| webhook-to-partner | 7982 | `0f71ef3c737707827fc6d48add0b392f8743bce594a8ea0a530ae3820220680c` | `d1391ba99e170ad581d3d8f8badf4794d7cbd27a3d47b4a7e43274c2de5e8779` |
| sensor-alarm | 1223 | `2b7994353b59abd40ccbb012e61b97011d377c86555e575fdc7c18c8c20caf04` | `a0f03bdb8975b4b9cd7b6402fd45f19901bebc309800d04c72272603e1e63136` |
| agent-dialogue | 3360 | `8daad6d576ae7687b000c0385dd4664aa6f4763afcafb41a94da0618fa923564` | `1a41faee3846f792e4dc56db7d9fabff5bd540428a7e53f88e0af8e00796d682` |
| agent-gateway | 3710 | `0e0f027d6dc5c31d65bde7c840be8c0f9a8bc68028d4d28da7180a2b06c3083d` | `73493ca59eda0afe8f1fa20b431618047c9cf3ef59e4aa8faad3b9ec0be85c5e` |
| agent-http | 7354 | `b34ca7e9fe4a2943fc89774968ae3cf99d9fcc6d692d459c76f778f5b96a248c` | `eb2545b106e864bc00aef1b4d1d55a35dc0c3631ac309d7d0d8556c88213ba8f` |

### Embedded qualification

Same workloads and unchanged 128 KiB heap / 1 MiB code gates:

| Workload | Baseline / candidate peak heap | Baseline / candidate code | Delta |
|---|---:|---:|---:|
| echo, 67 records | 112,656 / 112,656 B | 971,024 / 973,252 B | +2,228 B |
| Retry including full recovery, 17 records | 130,560 / 130,560 B | 1,039,012 / 1,041,704 B | +2,692 B |

Both runs report `SPIKE_OK`. Retry keeps the original one-operation,
three-Call, sixteen-four-byte-frames-per-channel-per-attempt workload.
Its generated candidate package hash is
`d8e89d6dc345e7c6199ee5ffc5664b682e09040a8589c6e934016bbe1c32581f`
(baseline `62d07d3b8e10ea45daf1260d9c3b993fb5046b22edfbb78e66ddaf34e1dce132`).
The peak-heap margin remains 512 B; code margin is 6,872 B. This is not a
claim of a throughput improvement or an additional embedded capacity.

### Additional pinned identities

These comparisons are also witnesses, even though they live in unit or
integration assertions rather than separate golden files. Their expected
constants remain unchanged pending approval.

| Witness | Stored | Candidate | Reason |
|---|---|---|---|
| Lowering semantic-hash pin | `3fe6e5319383eec3792f2981a40f9579fce96a4e1c3efe7b52b491dad932bc1a` | `6a49eb768c73af74cff64dc40bd08009e711fc1456c2b92e9e5f4f71e2508433` | Standard Contract namespace |
| Echo recording digest | `eba461cb2011177539b0a0a864ee01064f39097cace16dd3268b5523ce9e2078` | `6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d` | Recording 4, package 20, role removal, ccbor DATA, compact record/identity forms and inline deadlines (final 7 October measurement) |
| HttpClient defaults | `135282f162cbc350e091d230ff4d35077c920003a8f541c6173a47e671c1c661` | `97116fe8c6748306126066065a264bbf0f0e3e6cb38866ae0812069e198ebb5a` | Standard Contract namespace; explicit/implicit default equivalence still passes |
| HttpClient representative program | `a1bf3c54b12e0bb08795d67b2849560fa1b529ec97f802484d7578ef77134f56` | `c071a2a881181c82d6fc52841d2f789a803c853da67dd6a3c9f9f39f87c51330` | Standard Contract namespace |
| HttpServer representative program | `de5b489357e12e52bbd7b536e133d103dc6e7430734fa65e476dfee38e99fcdf` | `8be9c5c663158e632bb2f9de4dd20852c59580318ad3198dbac931f46ca18638` | Standard Contract namespace |
| WebSocketClient representative program | `e5d8c57b20427655842aa608f078858c744ede0a9648f3a4b881bb3c4b612020` | `33fff893c4ea10aed45047f6b296022138c6ac3cc5528c908c06ee0c0f8b0aff` | Standard Contract namespace |
| WebSocketServer representative program | `37c7043e337fb958e7a743ddd1d9ad1693b1bf9d169297f2b882efbafe356667` | `8e023ac5bed6d799511633ba858330494ae6d8e1f901de9c09be7dcd735ad6f8` | Standard Contract namespace |
| M0 probe, eight records | `7594ec29d0f7c306a830220d06521cdf9d3aab4fc60839e64a3ccbb246f56166` | `858c71fa25d2fb26be6c201ea91af3ec83a3cdd3a2b02db7cff9d2703e04956b` | Compact envelope/record/value registry and graph-born ccbor DATA |
| Delivery identity | `301cd0af333f760b8f5dc13738b821ec` | `b26fc16a16876c6d531bfc0b5ee3442b` | Delivery domain 32 → 11 |
| Instance lifetime identity | `d8977f984e5356cc4df3aca36c442642` | `77901038ea63d061071656d3c7637a9d` | Lifetime domain 33 → 12 |
| Retry operation identity | `d6c4216ebd85518c3497d7410c2707eb` | `f09790a0bce6f64ec5ae4edacec19f2f` | Retry-operation domain 22 → 10 and opening Delivery domain 32 → 11 |
| Retry admission identity | `e9b37274b55fddbac02eee26f3fd4849` | `05cbdcc63115d77e8aeff05c0f1958dc` | Retry-admission domain 7 → 4 and the preceding operation identity |
| Inbound Session identity | `45e41992da920539378d903e21f963a0` | `5ea1374ac75e3968f604f31cbe68e2ca` | Session domain 17 → 6 |
| Outbound Session identity | `835d72e38dc56aaf373d9613449d7354` | `7ddc2c74566fc89f912afee39073d7be` | Session domain 17 → 6 |
| Session-write identity | `19b7714f0076f09221da8c3d9a22f2f8` | `47998f535730ffa5def6866dec6795cd` | Session-write domain 18 → 7 and its parent Session identity |

The remaining small byte pins have no package or causal seal of their own:
the selected-text instance address changes its value tag 8 → 6 (its
virtual-shard probe becomes 169 instead of 117); metadata map/array tags
18/19 become 16/17; the union payload's struct tag 17 becomes 15; scalar
tags follow the complete registry above; personal annotation `[5, 7]`
becomes `[3, 7]`. The final deadline codec uses an inline optional array,
not a byte string wrapping a single-field map.
The public join/expiry Lab also independently pins the instance generation
at transaction key 31: the candidate moves that field to key 18, without
changing the expected keyed address or generation.
These pinned constants are not silently updated as part of a functional fix.

### Functional discrepancies caught during the cutover

- The existing envelope-cache tests caught an omitted raw-byte cache fill.
  The byte reading remains memoized on the envelope; text uses its typed
  reader without contaminating another port's interpretation.
- A new regression exposed `LoopExpired.last_state` reading raw text as
  bytes. Exhaustion now uses the resolved entry type; recovery additionally
  rejects a forged expiry payload that disagrees with the terminal type.
- The existing equal-date Loop test exposed an accidental dependency on
  numeric run-id order: `next_dated_recurrence` returned early unless both
  a Loop and a Retry fold existed. The journal's arming-order rule now
  applies when either Extension exists alone as well. Opening Graph runs
  also follows the input FIFO rather than derived-id map order. Several
  runs can still open before their first admission; only an earlier unopened
  delivery in the same instance/edge/source stream takes precedence.
  The original expected transition order is unchanged; the regression adds
  checks of opening order and recovery at every prefix around the wakeup.

These are corrections to preserve the existing functional contracts, not
reasons to bless a different result. The completed candidate test run
re-exercises all three corrections.

### Candidate checks and review stop

Checks already completed on the candidate:

- `cargo fmt --all -- --check`, strict workspace/all-targets/all-features
  Clippy, and `cargo doc --workspace --no-deps` pass.
- Core checks without default features pass for `thumbv7m-none-eabi`,
  `thumbv8m.main-none-eabihf` and `riscv32imac-unknown-none-elf`.
- Both final Cortex-M33 workloads above pass, including Retry recovery.
- The strengthened equal-date Loop regression passes with unchanged expected
  execution order and byte-identical journals after each recovery cut.
- The current suites pass for representation (8 tests), multiport delivery
  (63), machine VM (83), Retry runtime (25), spatial recovery (14), Loop
  composition (5), and public site content (12).
- JavaScript projection tests: 32 pass; SVG layout: 16 pass. CI helper tests:
  14 pass; the rustdoc-link lint checks 208 Rust files with no relative
  Markdown citation. The Playwright browser suite has not run: its optional
  local test dependency is unavailable.

`cargo test --locked --workspace --all-features --no-fail-fast` completed:
**1,543 passed, 25 failed, 0 ignored**, across 61 test groups. Its exit status
is 101, not a green gate. Every observed failure is an unchanged stored-byte
or identity pin listed above:

| Target | Failed tests | Pinned movement |
|---|---:|---|
| `rastok --lib` | 2 | Lowering semantic hash; echo recording digest |
| `contracts` | 2 | Standard Contract semantic hashes |
| `gallery` | 1 | Existing scenario traces, compared after successful assertions |
| `gauge_fixture` | 2 | Package and trace; semantic hash still passes |
| `golden_trace` | 2 | M0 probe and echo Lab traces |
| `instances_lab` | 1 | Instance-generation trace key 31 → 18; both scenario assertions pass |
| `jsonl_fixture` | 3 | Semantic hash, package and trace |
| `plan_package` | 1 | Echo package |
| `rastok-core --lib` | 11 | Scalar/union/metadata tags, instance/Session/Delivery/Retry identities, deadline key and annotation tag |

No functional assertion was weakened and no test was disabled or ignored.
All 28 gallery scenarios also pass in the read-only ledger run, independently
of golden comparison. Existing pins remain unchanged until Fable's review
and Yoann's explicit approval. The complete gate must run again after the
single blessing, including assertions beyond each currently failing pin;
the candidate is **not yet release-qualified**.

Native macOS test startup repeatedly paused for minutes before entering
Rust; sampling found the process at `_dyld_start`. Two initial local
chat/gateway integration attempts exceeded their unchanged 30-second listener
deadline with empty child output. The final full-suite run re-exercised them
successfully: chat 4/4 and gateway 3/3, including their real local sockets and
child-process shutdown. No host security setting or application test timeout
was changed.

No bless, push, merge, tag, publication or external reviewer invocation has
been performed.

## Final pre-blessing inventory — 7 October

Yoann approved the blessing after Fable's candidate review, subject to the
three complements. These measurements precede every stored-witness change.

- A-T2-6(4): deadline arrays now sit directly in each lossless record tuple;
  `Deadlines::try_new` still enforces the same acceptance invariants. The
  record codec is positional: registry key 41 is not an outer map in this
  carrier. Call `closed_at` was already inline and does not move.
- The 6 October intermediate echo recording digest was
  `69d7131c777178e9b395dfc70f45205a83bbbc411e9fbb10c8561935fa465c76`.
  Removing the nested deadline byte strings gives the final digest in the
  table above. No causal seal changes from this complement. There is no
  committed recording file to convert: the test rebuilds its records from
  the echo Lab and pins their complete lossless encoding.
- The confirmed publisher rule also renames `std/env@1`, `test.local/test@1`
  and dotted privacy/pricing example references. These are deployment
  attestations, not new graph facts. No package or gallery seal moves from
  this complement; profile version syntax remains unchanged.
- The explicit `repr(u8)` discriminants and `From`/`TryFrom` implementations
  preserve every allocated byte. Tests cover all 256 inputs for each of the
  15 identity, 8 deadline and 10 lifecycle kinds, including named refusals.

The package ledger was rebuilt against the final source and re-ran echo,
JSONL, gauge and all eight gallery programs. Every body size, semantic hash,
checksum, trace count and seal agrees with the tables above; all 28 scenario
assertions pass. The regenerated Retry spike package also keeps the measured
candidate hash. Gauge remains the negative semantic-hash witness.

**Inventory corrections found by re-measurement:** the four network-program
rows had their stored/candidate columns reversed in the 6 October report;
the rows above now agree with the actual assertions and computed values.
The Retry operation row had compared a partially migrated independent
recipe with the new identity, rather than the original pinned string.
It now names the original pin and also inventories the subsequent admission
assertion, which was hidden behind that first failing assertion. These are
evidence corrections before blessing, not changes to execution semantics.

Focused checks: seven deadline-codec tests pass; twelve envelope tests pass;
the lifecycle exhaustive-byte test passes. Eleven recording tests pass,
including full prefix reconstruction and rejection of nested deadline
carriers; the remaining recording test fails only on its old digest shown
above. The core run has 360 passing tests and the ten remaining inventoried
pins; the Contract suite has 83 passing tests and its two inventoried hash
pins. No expected functional result, refusal or timeout was weakened.

The blessing will regenerate the three package witnesses, the embedded echo
package, four root trace witnesses, 28 gallery traces (including `files.utf8`),
the recording digest and the additional pins enumerated above. The CI echo
hash follows the same echo row. The post-blessing complete gate is still
required; these focused runs are not its substitute.

Final embedded measurements after the three complements, before accepting
their footprint (same regenerated packages, workloads and fixed gates):

| Workload | Peak heap | `.text + .rodata` | Heap / code margin |
|---|---:|---:|---:|
| echo, 67 records | 112,656 B | 973,028 B | 18,416 / 75,548 B |
| Retry with full recovery, 17 records | 130,560 B | 1,041,440 B | 512 / 7,136 B |

Both report `SPIKE_OK`. Relative to qualified lot 2, code grows by 2,004 B
and 2,428 B respectively; peak heap does not grow. Relative to the first v20
candidate, code shrinks by 224 B and 264 B. Retry remains one operation,
three Calls, sixteen four-byte frames per channel per attempt. The M2 entry
constraint is unchanged: no core growth without reclaiming bytes first.

## Single blessing and qualification — 7 October

The final inventory was committed as `8bc54fd` before any witness was
replaced. The blessing in `18a0d44` then regenerated the three package witnesses,
embedded echo package, four root traces and 28 gallery traces from the
current sources and scenarios. The new `files.utf8` witness observes three
bytes for `é\n`; no earlier artefact was converted or retained as a second
accepted representation. All additional assertion pins follow their rows
above. No functional assertion or test timeout changed, and no test was
disabled or ignored.

Independent comparisons after regeneration confirm every package body,
checksum and gallery seal against the pre-blessing inventory. Gauge's
semantic hash is still `0d3ad43d…`; its package and trace move only for the
separately inventoried representation changes. The echo recording digest is
`6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d`.

Completed checks after the final implementation and inventoried pin update:

| Check | Observed result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Pass |
| `cargo doc --locked --workspace --no-deps` | Pass |
| `cargo test --locked --workspace --all-features --no-fail-fast` | Pass: 1,571 tests, zero failures/ignored, 61 result groups; exit 0 |
| Core, no default features | Pass on `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf`, `riscv32imac-unknown-none-elf` |
| Cortex-M33 echo and Retry/recovery | Both `SPIKE_OK`; final heap/code figures are in the preceding table |
| SVG layout, editor grammar and readable chat-page JavaScript | 32 tests pass |
| DCO and rustdoc-link CI helpers | 14 tests pass; citation lint checks 210 tracked Rust files |
| Gallery assertions | 28 scenarios pass; their regenerated seals match the inventory |
| Real echo recording and replay | Eight records, output `hello`, same seal `80f1263274f3b1b7480fdb13c007459e18e1e4cf10cc9bf08b5d12b63a4b7e0e`; replay performs no Driver I/O |

The first ordinary Cargo invocation completed with **1,570 passed, one
failed, zero ignored**, across 61 groups. It encountered the same native
macOS startup delay as the candidate: the chat
child produced no output before the unchanged 30-second listener deadline.
Sampling a delayed test executable found it at `_dyld_start`, before Rust
entry. With the identical built binaries and no configuration change, the
isolated chat regression subsequently passed in 1.30 seconds; all three
gateway tests, including real local WebSockets and resident children, also
passed. The **complete ordinary Cargo command then passed**, not just the
failed target: 1,571 passed, zero failures/ignored, 61 groups, exit 0. No
build ran concurrently with either network-test campaign. The initial
failure remains evidence and is not counted as a passing run.

The three tests added to the 1,568-test candidate are the exact inline
deadline form, rejection of nested deadlines through the recording decoder,
and exhaustive lifecycle-byte conversion. The complete run also covers
hostile package-20 and recording-4 inputs: unknown tags/versions, forged
types/commands, noncanonical DATA, malformed tuples, signed-instant bounds
and invalid deadline relationships refuse rather than being repaired.

No host security setting was changed. No release archive was rebuilt, no
independent browser review was performed, and no paid provider was called.
Decoder attestation/availability, general journal indexes and
Extension-neutral journal facts remain M2. Public-format-1 numbering stays
a separate pre-publication TODO; no push, tag or publication is authorized.
