# The plan package — the transportable, verifiable ExecutionPlan

> Format note — the bytes follow this document, never precede it.
> Specified 2026-08-06 (M0.5 item 4), after the delivery-reference
> work froze the vocabulary the plan carries; **revised to format v2
> on 2026-08-07** after the adversarial review refuted v1 on three
> counts (an ambiguous value tag, a checksum that did not cover the
> declared identity, and a decoder that validated shape but not
> structure — errata E-2026-08-07-1, decisions D-2026-08-07-2/-3);
> **revised to format v3 on 2026-08-08** when the M1 multiport lift
> widened `port_decl` with the optional pinned type
> (D-2026-08-08-3) — a new format is a new version, so the typed
> table did not ride under a v2 header; **revised to format v4 on
> 2026-08-09** when the braced-form reform admitted the EMPTY struct
> literal (tag 17) into expression constants (D-2026-08-09-2) — an
> extension of the expressible values is a version bump, exactly as
> this document's own rule demands.
> The package is a TRANSPORT format: versioned and free to evolve by
> version bump; none of v1, v2, or v3 was ever published and no
> decoder reads them — each refuses by its number.
> **Format v5 is CURRENT** (D-2026-08-10-9, landed 2026-08-10): the
> three-zone metadata write targets, the travelling `app`/`ext`
> schemas, the content-addressed asset table, expanded named types
> (they never reach the format — expansion is the checker's), and
> the revised numeric canonicalization (D-2026-08-10-7). v4 joined
> the refused drafts. **v5 is FROZEN**: the `JoinMetaPlan` ops land
> with the joins slice under a LATER version — this reader's
> re-encoding check refuses unknown keys, so nothing can ride v5
> silently.
> Identity stays where it always was — the semantic hash over the
> canonical HIR; the package merely carries a plan and proves it intact.

## Shape

One byte discipline exists in this repository — the deterministic
CBOR of the golden traces (RFC 8949 §4.2 profile: shortest-form
integers, definite lengths, fixed unsigned map keys in table order).
The package speaks it too; `postcard` and friends were rejected for the
canonical roles (non-canonical, implicit schema —
[decisions.md](../spec/decisions.md) D-2026-08-03-1).

The API lives under `plan` — the codec is a capability of the plan,
not a domain of its own: `rastok::plan::encode(&ValidatedExecutionPlan,
&SemanticHash) -> Vec<u8>` and `rastok::plan::decode(&[u8]) ->
Result<(ValidatedExecutionPlan, SemanticHash), DecodeError>`.
`encode` is infallible because structural validity rides in the type;
what `encode` accepts, `decode` reads back — by construction. "Plan
Package" stays the artifact's documentary name (`.rkp`); no public
module carries it.

```text
package := magic "RKPB"                      4 raw bytes, sniffable
           ("Rastok Plan Binary" — the pinned meaning of the magic)
        ┌ canonical CBOR map ─────────────────────────────────┐
        │ 1 : format_version   uint      (this document: 5)   │
        │ 2 : semantic_hash    bytes(32) (the plan's identity)│
        │ 3 : body             bytes     (embedded, see below)│
        │ 4 : checksum         bytes(32) (see recipe below)   │
        └─────────────────────────────────────────────────────┘

checksum = blake3( "rastok.plan-package\0"   fixed domain string
                 ‖ format_version (u32 LE)
                 ‖ semantic_hash  (32 bytes)
                 ‖ body bytes )
```

The checksum covers the **whole association** — a tampered declared
hash or version refuses exactly like a tampered body (v1 covered the
body alone; refuted by counter-example). Every covered field is
fixed-width or last, so fields cannot slide into one another. The
vocabulary is exact: the producer **declares** the semantic identity
(computed over the HIR, which the plan alone cannot reproduce); the
format **protects the association** against later corruption; the
decoder verifies the association and **never re-derives the claim**.
An unkeyed BLAKE3 is an integrity checksum: it proves the package
untampered in transit, not who wrote it — authorship would need a
signature, which this format does not claim. A version outside the
readable set is reported exactly as read, never saturated.

The body is an **embedded byte string**, verified before a byte of
it is parsed. After parsing, the decoder holds the plan to the rest
of the contract, in order:

1. **Structural validation, carried by a type** — the parsed plan
   passes through `ValidatedExecutionPlan` (private field, no
   unverified constructor, no mutable access): every slot in bounds,
   every referenced port declared with the right direction, every
   handler owned by its node, activation on an input port, emission
   on an output port, paths and names unique, every state write
   resolvable. Well-shaped bytes naming impossible structure (an
   edge to slot 999) refuse typed — and the machine's `Vm::boot`
   accepts only the validated type, so no raw plan reaches it by
   any path. The compiler's lowering earns the same type once.
2. **Domain canonicity, audited by the same type** — every collection
   the plan documents as canonically ordered actually is: nodes by
   (kind rank, path), boundary and port tables sorted and unique with
   the implicit trio never declared, edges by their endpoint text,
   handlers by (slot, activation key) with one handler per
   activation, vars and attributes sorted, the error set sorted (it
   is a SET — the hash reads it sorted), and every CEL map literal
   construction-sorted, walked recursively through guards, steps,
   and writes. These orders live in domain arrays the serializer
   reproduces faithfully, so the re-encoding check below cannot see
   them — the type must.
3. **Canonicity, as one total check** — re-encoding the decoded plan
   must reproduce the body byte for byte, and re-encoding the header
   fields must reproduce the envelope. This refuses non-minimal
   integers, reordered or duplicated map keys, and every other
   non-canonical variant at once, instead of chasing them rule by
   rule.

The canonical reader is itself strict and bounded: success means
the bytes were canonical (minimal heads, strictly increasing unique
map keys, the one canonical NaN), and nesting beyond its documented
budget (`canonical_cbor::MAX_NESTING`, 128) refuses as a typed
error, never a stack overflow. The budget cuts both ways:
`ValidatedExecutionPlan` refuses any plan whose body would nest
beyond it (`NestingBudgetExceeded`, measured without recursion), and
the compiler surfaces that refusal as the user diagnostic
`E-PLAN-DEPTH` — so what `encode` accepts, `decode` reads back, with
no gap at the boundary.

Deep semantic checking (CEL typing, contract coherence) remains the
compiler's job — the package's validation holds the *structural*
invariants the runtime dereferences without checking. Refusals are
typed and total; there is no silent migration — a new format is a
new version, and an old decoder refuses it by name.

## Body tables (format v5)

The body is one canonical CBOR value encoding the `ExecutionPlan`.
Compound values use fixed-shape arrays with a leading numeric
discriminant; maps use fixed unsigned keys; absent optional = omitted
key. `tagged_value` is the registry of
[identity.md §3](../spec/identity.md) — the same sub-form the golden
traces seal, shared code and shared bytes; since 2026-08-07 the CEL
`uint` domain has its own registry tag 23, and tag 3 is `long` only
(E-2026-08-07-1 — v1 reused tag 3 for both, and a decode could turn
an `int` into a `uint` while keeping the plan's hash). Since v4 an
expression literal may carry the struct tag 17; the validator holds
shipped constants to the expressible subset — the EMPTY record only,
the one struct constant a compiler emits (D-2026-08-09-2).

```text
plan       := {1: [node*], 2: [boundary*], 3: [edge*], 4: [handler*],
               5: app_schema?, 6: ext_schema?, 7: [asset_bytes*]?}
                                                  // 5/6 : the travelling zone schemas —
                                                  //   [[field (text), type]*], sorted, prefix-free
                                                  //   (D-2026-08-10-1) ; absent = empty
                                                  // 7 : the asset table — RAW blob bytes in
                                                  //   digest order ; digests are DERIVED at decode
                                                  //   (and thereby verified) ; absent = none

node       := {1: path (text), 2: kind, 3: [port_decl*], 4: [var*]}
kind       := 1                                   — operator
            | [2, spec]                           — connector
spec       := {1: contract (text), 2: effect, 3: behavior (uint: 1 deterministic · 2 probabilistic),
               4: [[name (text), attr]*], 5: [error_kind (text)*]}
effect     := 0 | [1, class (uint: 1 idempotent · 2 compensable · 3 irreversible)]
attr       := [1, word (text)] | [2, tagged_value]
port_decl  := [name (text), direction (uint: 1 in · 2 out), type?]
              — the third element is the PINNED type (same encoding
              as var types), present exactly when the source
              annotates: an unannotated port keeps its two-element
              form in the BODY. The claim is scoped: the body bytes
              of an unannotated plan are unchanged, but the PACKAGE
              moved to v3 with this widening (version is checksum
              material), and the semantic hash of any graph with a
              port table moved with the annotation's identity slot —
              both pinned by the gauge fixture (D-2026-08-08-3)
var        := {1: name (text), 2: type, 3: init (tagged_value)?}

edge       := [endpoint, endpoint | null]         — null = «-> nothing»
endpoint   := [slot (uint) | null, port (text)]   — null slot = the boundary

handler    := {1: slot (uint), 2: activation, 3: guard (tree)?, 4: [step*]}
activation := [1, port (text)]
step       := [1, name (text), tree]              — let
            | [2, port (text), emission]          — emit
            | [3, name (text), tree]              — set var
emission   := [1] | [2, [write*]] | [3, [write*]] | [4, tree] | [5]
              identity · patch · replace · data · nothing
write      := [target, tree]
target     := [1, [seg (text)*]]                  — data path (empty = whole data)
            | [2, field (text)]                   — meta.app, FLAT dotted field (D-2026-08-10-1)
            | [5, field (text)]                   — meta.ext, flat field
                                                  // 3 (custom namespace) and 4 (mime) are RETIRED
                                                  //   draft tags — extend-only, never reused
              data path · app meta · custom meta · mime type

tree       := [type, expr]
type       := uint: 1 bool · 2 int · 3 uint · 4 double · 5 text · 6 bytes ·
              7 duration · 8 timestamp · 9 null · 10 dyn
            | [11, type]                          — list of
            | [12, type]                          — map of
expr       := [1, tagged_value]                   — literal
            | [2, [seg (text)*]]                  — path
            | [3, unop (uint), expr]              — unary   (1 not · 2 neg)
            | [4, binop (uint), expr, expr]       — binary  (1 mul … 13 or, declaration order)
            | [5, expr, expr, expr]               — ternary
            | [6, expr, expr]                     — index
            | [7, fn (uint), [expr*]]             — call    (1 duration · 2 timestamp · 3 size)
            | [8, [expr*]]                        — list literal
            | [9, [[key (text), expr]*]]          — map literal (already construction-sorted)
            | [10, index (uint)]                  — asset reference into body key 7
                                                  //   (D-2026-08-10-5 ; the HASH seals the content
                                                  //   digest, never this index)
```

The `tagged_value` alphabet gained the exact decimal (registry tag
10, D-2026-08-10-7): typed decimals at their declared scale, dynamic
ones at minimal scale, 38 significant digits and the decimal128
exponent domain as hard budgets.

All discriminants and keys are extend-only: append, never renumber.
Anything a future plan carries that these tables cannot express is a
format-version bump, not a creative reuse.

## Placement

The codec lives in `rastok-core` — both directions. The brief placed
the encoder std-side; implementing both halves beside the tables they
share is simpler and loses nothing: the *callers* keep the intended
layering (the compiler/CLI encodes, the embedded boot decodes), and
LTO strips the unused half from a firmware. The generic canonical
CBOR machinery moves from `golden` into the kernel for the same
reason — one implementation of the byte discipline, used by the
golden projection (std, evidence) and the package (core, transport).

## Verification contract

- **Round trip**: compile → encode → decode ⇒ the decoded plan is
  structurally equal to the original and the header's DECLARED
  semantic hash equals the compilation's. (The hash is computed over
  the HIR and travels with the package; a plan alone cannot
  re-derive it — the round trip checks equality of plan and of
  declared hash, under a checksum-protected association. No party
  proves the hash *belongs* to the plan; the producer declares it.)
- **Blessed fixture**: the example's package is pinned like a golden
  trace and re-blessed only deliberately.
- **Semantic round trip**: beyond structural equality, the decoded
  plan must EXECUTE identically — the boundary-value matrix
  (`i32::MAX`, `i32::MAX + 1`, `i64::MAX`, negatives) runs on both
  sides of a round trip and compares outputs.
- **Typed refusals**: a bumped version, a corrupted body byte, a
  corrupted declared hash, an out-of-bounds slot, a non-canonical
  envelope, and a truncated package each fail with their own error —
  tested.
