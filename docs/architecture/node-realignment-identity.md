# Node realignment — package-21 candidate inventory

Date: 14 October 2026. Authority: D-2026-10-11-1. This is review material
prepared **before** regenerating package witnesses, not an approval of the
candidate or an amendment to the current-format heading in `identity.md`.
The measured implementation is `dd5bda4`, following the indexed-endpoint
checkpoint `59f1ddf`.
The implementation and measurements are in
[the qualification report](node-realignment-qualification.md).

## What changes, and why

The package changes its representation of declarations and wiring, not the
program's behavior. The compiler's semantic-hash HIR and the journal codec
are unchanged.

- Package format 20 becomes 21. Every other package version refuses before
  body decoding. There is no compatibility reader or artifact converter.
- New plan-body key **12** carries one Node table, with the App Graph at
  index zero. A row owns its parent, ports, instance policy, immutable
  declarations, Extensions and nature-specific body. Existing top-level
  keys are not reassigned to new meanings.
- The Operator body contains its variables and handlers. A handler no
  longer transports a redundant reference back to its Operator.
- Each Graph transports its own reference wiring. Endpoints name a Node
  index and port; destinations explicitly distinguish ports, discard,
  Loop commands and exact-instance closure. The containing Graph fixes
  each boundary port's face.
- Separate owner/executor tables and independently transported physical
  edges, composed routes and boundary inventories disappear. Validation
  derives private execution indexes from the one reference description.
- The resolved-port table retains its independent verification role, but
  its rows now address the unified Node table, including the root.

The package envelope remains `RKPB` followed by canonical CBOR. Its checksum
still covers the format number, declared semantic hash and body. Thus a
checksum change here is expected even though the semantic hash is fixed.

Mailbox `EdgeIndex` values are execution-local and **not transported**.
Journal routing and consumed positions retain their canonical path strings.
The indexed-delivery step makes no further package movement.

## Package witnesses

All body lengths below exclude the package envelope. The existing
`package_ledger` example compiled the sources, encoded and independently
decoded each candidate, then compared its declared identity to the tracked
witness. No tracked file was overwritten.

| Witness | Format | Body bytes | Semantic hash |
|---|---|---|---|
| Echo | 20 → 21 | 961 → 934 | `562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813`, unchanged |
| JSONL | 20 → 21 | 1,005 → 979 | `0aeff047b9efde5c6dc4efd6f411c5541a786fb24f816108e110eb29d00e026f`, unchanged |
| Gauge | 20 → 21 | 249 → 230 | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e`, unchanged |

| Witness | Format-20 checksum | Candidate checksum |
|---|---|---|
| Echo | `a9f7b7c5c11f68ccedc4463edb803774681b3125d695fa67889e83228fbc964d` | `c24806e6918130b4d8d35c6660d3f254c598d17c41aac9930f6545d4cf748dc0` |
| JSONL | `801f64797fd51a698de7cbaefe1e06e4281f0b92d9441b396a6a85d894cf5148` | `6d03694522f95d6ea2b8cf3654938ee73f65e5fa75c6dae8bcfc83943ed733d6` |
| Gauge | `342c5147300138d74fd7ffe4d54ebd8859c7db3578afceba585efd0bd64d2746` | `3acf7c7f98b481b4d779f99a234ccea6173136e6e2a79417da99164f968b0196` |

The reasons are the same structural changes above, not new Connector
contracts, values, ordering or behavior. Gauge also changes package bytes:
even its small pure graph now uses the unified Node table and Graph-owned
wiring. Its semantic hash and trace remain the negative witnesses.

Files to regenerate after approval:

- `crates/rastok/tests/golden/echo.rkp`
- `crates/rastok/tests/golden/jsonl.rkp`
- `crates/rastok/tests/golden/gauge.rkp`
- `spikes/rp2350-emu/echo.rkp`, the binary copy of the echo package.

The current spike file was checked byte-for-byte against the decoded hex
golden: both contain the same 1,042-byte format-20 package. Its replacement
must come from the newly compiled source, not from editing the old bytes.
The explicit version assertions in `plan_fields`, `instance_stop` and
`retry_plan` must move to 21 in the same approval change. Their functional
assertions are not relaxed.

The format heading and representation descriptions in `identity.md`,
`plan-package.md` and the spec-to-code map must be aligned at the same
checkpoint. In particular, the separate owner/executor tables described
by the current package document are not the candidate representation.
This is documentation of the accepted ontology, not a semantic amendment;
the human-review registers and decision entries remain with their owners.

## Witnesses that must not move

| Causal witness | Record count | Seal, unchanged |
|---|---:|---|
| Echo roundtrip | 8 | `566aed6f75b16361988aae27ed8af674c2f8648cca11118829b96e8c1d65efff` |
| Echo filtered | 4 | `7cd9e23c6af5c004268c7f590ea012d47d06b1d64949a333e8220c5db4b6978f` |
| Echo silence expires | 8 | `4a1c315c7132de01c8bf5b23dbf91cb17a88084a6f7db6030377675782eb7d99` |
| JSONL roundtrip | 8 | `7afbcaaba2f085995ac7262e256058c2f3ed459fce10a2ce8f5dd6a3617dc099` |
| Gauge roundtrip | 4 | `0f1dd432e3e4e5f4b3bc20a2f86e024a67e620db51f8e41575eb7ae3cd2a9528` |

All 28 gallery comparisons also retain their record counts and causal
seals, including timed joins, Loop, Retry, keyed instances, Process Sessions
and HTTP Lab scenarios. No `.trace` is scheduled for regeneration.

Diagnostic recording stays at version **4**. Its record tuples and the echo
tuple digest remain unchanged:
`6130123972daf8e882d56c726361a0ca26a4b6ac320613e697848e6bca6abb4d`.
The existing independent pin test was rerun successfully. A complete newly
captured recording embeds a format-21 package and therefore has a different
file checksum; this does not justify changing any journal tuple or causal
seal. There is no tracked `.rkrec` file to replace. Previous captures with
an embedded format-20 package refuse at package validation and must be
recaptured, not converted.

The echo semantic hash in CI stays unchanged. No source example, Contract
identifier, deadline tag, identity domain, annotation or runtime command
changes as part of this approval.

## Review boundary

The measured candidate uses boxed nature-specific Node bodies. On the fixed
Cortex-M33 Retry workload with recovery it leaves **1,884 B** below the
128 KiB heap gate and **7,672 B** below the 1 MiB code gate; echo leaves
19,668 B and 79,564 B respectively. These are measured workloads, not bounds
for every valid program. The qualification report retains the alternatives,
load peaks and resident costs rather than inferring them from package size.

Recommendation: retain this layout, then regenerate only the four package
files and update the three explicit version pins above. The complete
workspace gate must subsequently pass without exceptions before lot 1 is
called complete. Layout approval, package regeneration and that final green
gate have **not** been claimed by this inventory. No push, tag or publication
is part of the work.
