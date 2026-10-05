# Human review vocabulary lot 1

5 October 2026. Base: `eb3f396`, branch `rework/retry`. Authority:
[human-review ledger](human-review-m1.md) and
[D-2026-10-05-1](../spec/decisions.md#d-2026-10-05-1--align-the-human-review-vocabulary-before-one-public-cutover).
This lot stops for Fable's review before any lot-2 structural work.

## Invariant and scope

The names used to read the core agree with the approved review vocabulary.
Every allocated identity/tag value and every valid encoded record keeps its
bytes. Package **19** and recording **3** remain executable; this is not the
v20 cutover and no witness is re-blessed.

| Action | Implemented |
|---|---|
| A-T1-4 | `IdKind`, fourteen active recipes, explicit `id_kind_tag`, named `UnknownIdKind` refusal; scattered recipe constants removed |
| A-T2-4 | `Ingress` / `Transaction` / `Egress`, admission types/APIs, log event and History record-type labels; `CallStatus` describes the status of a Call, while `UnboundOutbound` still names a missing outbound Driver binding |
| A-T2-5 | `DeadlineKind`, `Deadline`, `Deadlines { created_at, items }`, `deadline_ids`, `deadlines`, and `require_deadline_instants`; logical nanoseconds and creation order remain explicit |
| A-T2-7 | `journal::codec::{encode, Decoder, Error}`; complete records include sealed and unsealed fields; ordinary uses of retain/retained for kept evidence remain |
| A-T2-6(3) | Admission-prefix implementation members follow the trait order |
| A-T1-5 / A-T1-8 residues | Existing `…Exceeded`, `InvalidSecretName`, `select_session_id`, `map_from_canonical` and `…sys_fields` spellings verified; the remaining UI local and live prose are aligned |

The debugger and structured log vocabulary is an intentional presentation
change, not a causal-format change. `call_direction` and inbound/outbound
Connector roles remain directions. An inbound Call response remains the
`exchange_exit` of a transaction, not a new Egress operation.

Primary symbol renames used RustRover semantic refactoring. Scoped patches
completed references missed in wildcard imports, async/test bodies and
out-of-workspace spike code. Moving the codec, introducing the two closed
enums, their explicit byte projections, the tests and prose used patches.
An ambiguous IDE rename selected a local binding rather than a private
decoder method; it did not change encoding or require a compatibility alias.
Incidental IDE substitutions in dated reports were restored, as were prose
uses of Reader and protocol frame that did not name the renamed symbols.

## Corrections found against the repository

- There are **eight** allocated deadline kinds, not nine: bytes
  `1, 2, 3, 5, 6, 7, 8, 9`. The prior range check also admitted unused byte 4.
  Its requested refusal is now enforced by the closed enum and the decoder;
  no valid tag is renumbered. The tests cover all 256 input bytes.
- The old envelope-domain summary listed obsolete/deferred recipes and
  omitted active Session and Retry recipes. Its current table now describes
  the fourteen recipes actually implemented. It allocates no boot-epoch,
  replica or decoder domain. Numeric compaction belongs to the reviewed
  cutover, including the future decoder reservation.
- The earlier raw-byte tests comparing Loop r2 against obsolete r1 domains
  cannot call a closed `IdKind`. They now check separation between active
  run/command/admission identities; an independent byte-table test pins all
  fourteen BLAKE3 domain framings. No legacy reader or alias was added.

## Qualification

| Check | Result |
|---|---|
| `cargo fmt --all`, then `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --workspace --all-features` | 1,554 passed, 0 failed, 0 ignored; 60 result groups |
| `cargo doc --workspace --no-deps` | Passed |
| `cargo check -p rastok-core --no-default-features --target thumbv7m-none-eabi` | Passed |
| `node --test crates/rastok/src/ui/graph_layout.test.cjs` | 16 passed |
| `git diff --check` | Passed |

The first test run was sandbox-restricted: 27 local-network tests could not
bind a loopback socket. It was not a green gate. The complete suite was
restarted with explicit permission for those local tests; no test was
disabled or ignored to accommodate the sandbox.

The two additional tests pin all identity-kind bytes and all deadline-kind
bytes, including rejection. `canonical_parameter_order_is_pinned`, the
echo/JSONL/gauge package and trace witnesses, and the echo recording tuple
pin all pass unchanged.

No change under `crates/rastok/tests/golden/`, `crates/rastok/tests/fixtures/`
or `examples/`; no `.rkp`, `.trace` or recording was re-blessed. The pinned
echo recording tuple hash remains
`eba461cb2011177539b0a0a864ee01064f39097cace16dd3268b5523ce9e2078`.
The echo semantic hash expected by CI remains
`878a34b77a7ac6500b6452b903f633ba39848c47ed0ac387ce1597f78a785699`.

Browser qualification and the Cortex-M33 spike were not rerun for this
vocabulary lot. The source-level spike references follow the API renames;
heap/code measurements are required by lot 2 before accepting payload sharing.

## Review stop and remaining work

Lot 2 remains unopened: shared payload storage, typed timing errors and
journal-acceptance layering, the common admission fold, and the rustdoc-link
lint. Lot 3 remains unopened: role removal, type-directed materialization,
ccbor graph-born values, `std/` identifiers, registry compaction and the
single identity inventory/bless.

Opening is **5 November**, freeze **24 October**. Keep the chantier/target
number **v20** now; renumber the public format to **1** before publication,
with recording numbering handled in that release step. No push, tag,
publication or external reviewer invocation was performed.
