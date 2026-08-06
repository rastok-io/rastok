# The plan blob — sealed serialization of the ExecutionPlan

> Format note — the bytes follow this document, never precede it.
> Specified 2026-08-06 (M0.5 item 4), after the delivery-reference
> work froze the vocabulary the plan carries. The blob is a
> TRANSPORT format: versioned and free to evolve by version bump.
> Identity stays where it always was — the semantic hash over the
> canonical HIR; the blob merely carries a plan and proves it intact.

## Shape

One byte discipline exists in this repository — the deterministic
CBOR of the golden traces (RFC 8949 §4.2 profile: shortest-form
integers, definite lengths, fixed unsigned map keys in table order).
The blob speaks it too; `postcard` and friends were rejected for the
canonical roles (non-canonical, implicit schema —
[decisions.md](../spec/decisions.md) D-2026-08-03-1).

```text
blob := magic "RKPB"                      4 raw bytes, sniffable
        ┌ canonical CBOR map ────────────────────────────────┐
        │ 1 : format_version   uint      (this document: 1)  │
        │ 2 : semantic_hash    bytes(32) (the plan's identity)│
        │ 3 : body             bytes     (embedded, see below)│
        │ 4 : seal             bytes(32) = blake3(body bytes) │
        └────────────────────────────────────────────────────┘
```

The body is an **embedded byte string** — the seal covers its exact
bytes, so a decoder verifies integrity before parsing anything.
Refusals are typed and total: wrong magic, unknown `format_version`,
a seal that does not match, or a body outside the tables below. There
is no silent migration — a new format is a new version, and an old
decoder refuses it by name.

## Body tables (format v1)

The body is one canonical CBOR value encoding the `ExecutionPlan`.
Compound values use fixed-shape arrays with a leading numeric
discriminant; maps use fixed unsigned keys; absent optional = omitted
key. `tagged_value` is the registry of
[identity.md §3](../spec/identity.md) — the same sub-form the golden
traces seal, shared code and shared bytes.

```text
plan       := {1: [node*], 2: [boundary*], 3: [edge*], 4: [handler*]}

node       := {1: path (text), 2: kind, 3: [port_decl*], 4: [var*]}
kind       := 1                                   — operator
            | [2, spec]                           — connector
spec       := {1: contract (text), 2: effect, 3: behavior (uint: 1 deterministic · 2 probabilistic),
               4: [[name (text), attr]*], 5: [error_kind (text)*]}
effect     := 0 | [1, class (uint: 1 idempotent · 2 compensable · 3 irreversible)]
attr       := [1, word (text)] | [2, tagged_value]
port_decl  := [name (text), direction (uint: 1 in · 2 out)]
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
target     := [1, [seg (text)*]] | [2, field (text)] | [3, ns (text), field (text)] | [4]
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
```

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
golden projection (std, evidence) and the blob (core, transport).

## Verification contract

- **Round trip**: compile → encode → decode ⇒ the decoded plan is
  structurally equal to the original and the header's semantic hash
  equals the compilation's. (The hash is computed over the HIR and
  travels with the blob; a plan alone cannot re-derive it — equality
  plus the sealed header is the proof.)
- **Blessed fixture**: the example's blob is pinned like a golden
  trace and re-blessed only deliberately.
- **Typed refusals**: a bumped version, a corrupted body byte, and a
  truncated blob each fail with their own error — tested.
