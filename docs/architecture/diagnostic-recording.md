# Diagnostic recording and read-only replay

Status: 6 October 2026, candidate diagnostic format **4**, embedding
plan package **20**. The [v20 inventory](human-review-v20-identity.md) records
the byte changes before blessing. There is no reader or converter for another
recording format. Exact-instance closure packets travel on INGRESS and
TRANSACTION; unused packets are absent from the causal seal.
The durable deployment Journal remains M2 work.

## Commands and observable result

```sh
printf 'hello\n' | rastok run examples/echo.rkg --record echo.rkrec
rastok replay echo.rkrec
rastok replay echo.rkrec --ui
```

The first command runs the real Process and records its retained facts. The
second prints the same App outputs and reports the same causal seal without
starting that Process. The third opens the existing loopback debugger over
immutable prefixes. `--ui-port PORT` selects its port; `hosted-ui` must be
compiled in. The original `.rkg` source and files mentioned by the program
are not needed for replay.

An existing recording path is refused, never overwritten, including a symlink.
On Unix, newly created recordings have mode 0600. Creation precedes Driver
binding/effects. The recording is sensitive diagnostic evidence, not a file
to commit or publish indiscriminately.

`replay` is **retained-state reconstruction**, not a second execution of the
DSL and not a generated Lab scenario. Journal acceptance and existing VM
recovery validate and reconstruct the prefix. Pure Contract projections
reconstruct scheduler completions; physical Drivers, handlers and guests
never run. This proves preservation of the retained facts, not authenticity
of an arbitrary file or correctness of a freshly re-executed producer.
The production-to-Lab extraction in simulation-semantics §9 remains deferred.

## What is retained

- Independently validated plan package and declared semantic hash.
- Source definitions and arm positions for inspection; presentation data,
  never code executed by the reader.
- Deployment graph version and whether an empty deployment header existed.
- Complete INGRESS, TRANSACTION and EGRESS records, including typed DATA,
  META, prepared request bytes, completion extension metadata, manager
  commands, Loop states and exact retained timer schedules.
- Checkpoint prefix lengths and explicit logical observation instants.
- On normal completion, a footer containing record count and causal seal.

The golden projection is deliberately insufficient as the storage codec: it
omits non-causal retained fields needed for faithful recovery. The diagnostic
codec therefore preserves those fields without fabricating journal facts.
Raw DATA is distinguished from a graph-born ccbor byte value; encoded and
materialized ccbor are representation states of the same payload. Decimals
and canonical NaNs round-trip exactly. The tuple has four system fields
(id, parent, origin, graph version), app/ext metadata and DATA; no role.
Deadlines are an inline optional tuple field:
`[created_at, [[kind, id, due_at], ...]]`, with signed logical nanoseconds.
There is no nested CBOR byte string or second map around that array. The
reader calls `Deadlines::try_new`; malformed carriers and invalid dated
obligations remain distinct errors. Winning Call `closed_at` is already an
inline signed integer. Neither field contributes to the causal seal.

## Framing and refusal

The file begins with `RKRC`. Each following frame has a four-byte big-endian
body length, a 32-byte BLAKE3 checksum, and a canonical-CBOR body. The checksum
covers `rastok.diagnostic-recording\0`, the previous checksum (zero initially)
and the current body. Header, checkpoint and completion-footer shapes are
versioned by this diagnostic format, independently of the embedded plan.

The reader refuses unknown versions, noncanonical or lossy records, partial
frames, checksum mismatches, inconsistent repeated envelope identities,
invalid journal transitions, failed plan/recovery validation, and a footer
whose count or causal seal differs. Checksums detect corruption; they are
**not signatures**. An attacker who rewrites a whole recording can recompute
them; independent acceptance is not an authenticity certificate.

A file ending after a complete checkpoint but without a footer is inspectable
as an **incomplete run**. It is never reported as completed. A torn frame
refuses; the reader does not silently trim or repair it. Writers append
checkpoints from the hosted pump and synchronize the final footer. They do
not implement write-ahead durability, fsync-before-effect, automatic restart,
continued execution, rotation or retention enforcement.

Bounds: 64 MiB per file and 100,000 retained records. The optional debugger
additionally admits at most 1,000 records and 64 MiB of serialized snapshot
history; larger recordings remain available to CLI replay within its bounds.
Snapshots are reconstructed from prefixes, not retained as another authority.

## Time and debugger

Replay starts at transition zero. Each following position is a retained record
prefix, not a claim about an independently stepable live execution. A hosted
pump checkpoint can contain several records. For an interior prefix, the
displayed logical time is that checkpoint's **observation instant**, not an
invented exact commit timestamp. Retained due times and Loop ordinals remain
facts of the selected prefix.

Next transition, Back, the slider, Replay/Pause and Finish only navigate these
prefixes. No scenario, reset, retry, injection, branch or execution command is
available. The server refuses execution POSTs even from its controller; Close
only shuts the viewer. `run --ui` remains refused. Existing redaction, mailbox,
source, cost and causal views consume the same validated projection.

## Privacy and first-slice limits

`@personal` may be retained in the attested in-memory execution journal. That
capability does not authorize persistence: this recording format refuses a
plan containing personal-data policy **before file creation or effects**.
The check includes boundary/child ports, owner values, state and local bindings.
UI masking is not data erasure. Durable personal-data storage needs separately
qualified retention/erasure capabilities; see the
[opening ledger](opening-showcase.md#persistence-and-personal-data).

Nonempty deployment attestations and boot-dependent Connector settings
(`env()` / `secret()`) also refuse in this slice: a reference-only header and
offline bindings must be qualified before admitting them. No physical secret
provider is consulted by the reader. A price-profile capture is consequently
not yet supported. Original typed measurements remain in their journaled
envelopes; replay does not calculate prices or totals. Missing usage is unknown,
not zero (D-2026-09-16-7).

Annotations are not automatic personal-data discovery. Unclassified source,
payloads and literal settings can contain sensitive data. `@sensitive` alone
does not prevent storage: its values remain in the recording and are redacted
by the existing presentation policy. There is no encryption-at-rest claim,
GDPR certificate, share-safe export, or redaction of the sealed stored bytes.

## Evidence

`recording::tests` covers every prefix of echo, keyed-retry, countdown and
expense-approval; exact version/metadata/DATA round-trips; conflicting envelope
content; corruption, truncation and incomplete checkpoints; and personal-data
refusal. CLI tests remove both a File input and its source after recording,
replay with an empty PATH, compare outputs and seals, reject overwrite and
verify Unix permissions. UI protocol tests reject execution commands; the
browser recording test exercises navigation and paused playback without an
execution POST. These tests do not close the independent release audit gate.
