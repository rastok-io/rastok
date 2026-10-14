# Flat annotation types and validated metadata

> **Normative; [current format](identity.md).** The standard
> `measure` definition is exactly `id: text` and `unit: text`, under
> [D-2026-09-18-1](decisions.md#d-2026-09-18-1--separate-generic-measurements-otlp-export-and-budget-authority).
> A currency such as `"USD"` is an ordinary unit. Original signed quantities,
> including those in held Retry attempts, remain journaled with their causal
> provenance. M1 implements no report, valuation, aggregation or subset policy.
> The [v20 inventory](../architecture/human-review-v20-identity.md) records
> the approved namespace/tag compaction. Security/privacy meanings
> and the two-field signature are unchanged.

This document owns annotation declarations and their interpretation boundary.
[Types](types.md) owns logical values; [observability](observability.md) owns
the existing policy, privacy and accounting meanings. This amendment changes
how those meanings are declared and bound, not who may perform effects.

## 1. One declaration, two uses

```rkg
type measure = @struct {
  id: text
  unit: text
}
```

This is the executable standard signature. Its spelling is `@struct`, not `struct@`, `annotation { ... }`, or a new
`measure` declaration keyword. The declaration is stdlib-owned; copying its
fields into a user declaration does not grant its runtime interpretation.
User flat declarations and aliases cannot
claim protected stdlib annotation names or the retired `cost` spelling
(`E-NAME-RESERVED`). The illustration is not a user override of the current
builtin signature.

The same annotation describes both quantities:

```rkg
type Usage = struct {
  tokens: int @measure(id = tokens, unit = "token")
  amount: decimal(18,6) @measure(id = amount, unit = "USD")
}
```

The value comes from the field, not the annotation. `USD` grants no charging
authority, and `tokens` selects no OTel instrument. A reported amount, a
tariff-based estimate and a budget reservation are different facts: their
consumer must retain that distinction without enlarging the generic signature.
Neither the example nor this decision introduces an M1 reporting engine.

An `@struct` is a closed, flat, named-field schema. It is usable as an ordinary
data type and is eligible for annotation application. Its values use the
existing `DataType::Struct` / `Value::Struct` representation, not a second
numeric algebra or a new family of runtime values.

```rkg
type note = @struct {
  text: text
  revision: uint
}

type Request = struct {
  prompt: text @note(text = "Provided by the caller", revision = 1)
}
```

The example declares metadata, not an execution hook. The same `note` schema
may describe ordinary data. Annotation arguments must be plan constants;
ordinary data of that type is not required to be constant.

## 2. Flatness and parsing

The local grammar extension is LL(1):

```ebnf
base_type        ::= scalar_type | other_existing_base_type
                   | struct_type | flat_struct_type
struct_type      ::= "struct" "{" fields? "}"
flat_struct_type ::= "@" "struct" "{" fields? "}"
```

`fields` reuses the ordinary closed-field grammar. In a base-type position,
`struct` and `@` select disjoint alternatives. After consuming `@`, the parser
expects `struct`; that is sequential parsing, not two-token lookahead.
Existing postfix `T @name(...)` application remains a different parser
position. No speculative parsing, backtracking, or second spelling is needed.

After alias resolution, every field must be a logical scalar, an enum, or
the optional form of such a type. Structs, arrays, maps, tagged unions with
payloads, open records and `variant` refuse, including behind aliases. The
ordinary type/value budgets still apply. An unsupported constant spelling
must refuse explicitly; declaring a scalar field does not authorize an
environment lookup, guest invocation or arbitrary annotation expression.

Flatness describes the annotation's **arguments**, not its target. Existing
policies may still annotate an ordinary nested struct, a collection or a
canonical subpath wherever their interpretation admits that target.

An optional data field continues to obey ordinary `T | null` conformance:
this decision does not make absent struct fields generally legal. Annotation
argument omission is a separate source rule: for a declared descriptive
annotation, an omitted nullable argument resolves to the same explicit null
as writing it. Every field is present in the transported typed struct. A
missing required field refuses. This normalization does not make omission
legal in ordinary data. Existing stdlib defaults such as omitted `@personal`
flags remain unchanged.

## 3. Definition, application, interpretation

These are distinct authorities:

| Layer | Owns | Does not imply |
|---|---|---|
| Type definition | Flat argument schema, ordinary type checking, annotation eligibility | Permission to execute code or enforce a security policy |
| Annotation application | Reference to that definition, canonical constant arguments, exact target `SchemaPath` | A new envelope field or a mutable runtime counter |
| Registered interpretation | Target constraints, normalization, propagation and required consumer capability | Arbitrary user-defined hooks or physical I/O |

The stdlib provides definitions, including those with interpretations known
to the core. Applications can define metadata using the same type machinery.
Core and hosted consumers read only validated, resolved metadata, never raw
source strings or an unvalidated map from a decoded package.

Descriptive metadata has an explicit metadata-only classification, is typed
and retained for its tools, and grants no execution behavior. A required
semantic interpretation must be supported by the target image or refuse by
name before effects. An undeclared annotation is not metadata-only by default.
The descriptive binding is explicit tag 1 inside a tag-4 application:
declaring identity, ordinary argument type and ordinary constant value. Its
name is a canonical dotted identifier of at most 255 bytes, not a protected
stdlib name. Unknown interpretation tags refuse, including not-yet-supported
semantic consumers. Binding 2 now admits the numerical quantity interpretation of
`std/measure@3`, with the exact protected flat schema and complete ordinary
typed constants. Merely changing the binding on a descriptive application
cannot grant measurement authority.

A definition supplies no handler, `var`, mailbox, transaction, new port, I/O
or private evaluator. This is not an annotation plugin ABI. A generic public
interpreter trait is not required to express the existing concrete consumers.

## 4. Type identity is not interpreter authority

Ordinary named data types remain structural aliases. Two equal flat data
schemas have the same value meaning; the `@struct` capability need not create
a different envelope encoding from the equivalent ordinary struct.

Annotation meaning cannot be inferred from equal field shapes. For example,
two definitions containing `reason: text` may mean documentation and
deprecation, respectively. A user-defined record whose shape matches
`personal` is not the stdlib's personal-data policy.

The validated plan must retain the annotation definition/binding identity
separately from its structural argument schema. An alias resolves to the
same declaration and binding; a separately declared lookalike does not.
Protected stdlib bindings cannot be shadowed or replaced by source spelling,
matching fields, or a forged claim in a package. The current package retains
the declaring name and schema, not the alias used at the application. One
name cannot claim different argument schemas across plan sites. Declaration
field order and application argument order are non-semantic; changing an
argument or choosing another declaration changes annotated identity. An unused
declaration contributes no plan/hash bytes.

Exact copies and projections preserve descriptive applications. Calculations
drop them rather than invent a meaning for a transformed value; alternative
producers must agree per definition. Two applications of the same definition
at one path refuse, including through different aliases. Different descriptive
definitions may coexist in canonical declaring-name order.

An annotation definition whose argument fields themselves carry annotations
is not yet an admitted application: `E-CHECK-DEFERRED` names nested
argument-policy transport. The same flat type still works as ordinary
annotated data. Argument policies are never silently erased to accept an
application, including self-referential annotation declarations.

## 5. Standard annotations and consumers

| Annotation | Meaning | Consumer |
|---|---|---|
| `@sensitive` | Ordered levels, conservative control/data taint, no implicit declassification | Core policy derivation; hosted/Lab/UI redaction |
| `@secret` | Reference-origin credentials only at admitted settings; raw secret data remains refused | Checker/core boundary policy, deployment binding, physical Driver credential overlay |
| `@personal` | Orthogonal personal-data classification, exact flags and attested deployment/retention requirements | Core admission policy, hosted deployment, policy-aware inspection |
| `@measure` | Id and unit of a numerical field, exact individual value and causal scope | Typed retention and read-only inspection of original observations |

This table does not move privacy decisions or secret handling into untrusted
source definitions. Policies continue to be independently derived, and
redaction precedes formatting. Descriptive metadata cannot acquire those
policies' authority through a matching field shape.

Money is not a separate annotation profile. For example,
`decimal(18,6) @measure(id = refund, unit = "EUR")` retains each original
positive, zero or negative refund. M1 calculates no net amount. A structured
response annotates its numerical field, not a Money-shaped container.
A unit neither claims that an amount was billed nor identifies a pricing
source. Valuation and future budgets must define their own authority.

`@cost` and the former `currency`, `basis`, `aggregation` and `subset_of`
arguments refuse with a migration explanation. Every numerical measure uses
the same protected generic binding; there is no separate compact cost tag.

### 5.1 Standard signatures and normalization

The standard source definitions live in
[`stdlib/annotations.rkg`](../../crates/rastok/src/stdlib/annotations.rkg).
They are ordinary flat data types, with protected interpretation on application.
Their aliases retain that authority; independently declared lookalikes do not.
Unknown and duplicate arguments refuse. Arguments are plan constants.

| Definition | Argument fields | Omission / normalization |
|---|---|---|
| `sensitive` | `level: enum { internal, confidential, restricted }` | `level` is required; no implicit public level |
| `secret` | No fields | No arguments are admitted |
| `personal` | `special: bool`, `criminal: bool`, `identifier: bool` | An omitted flag means `false` in the resolved policy |
| `measure` | `id: text`, `unit: text` | Both required; no implicit unit or aggregation |

The checker resolves the declared argument type; core independently verifies
the complete ordinary type/value and protected definition identity.
Sensitive, secret and personal applications lower to their existing compact
policy encodings. Numerical measures use tag 4, binding 2,
`std/measure@3`, and the exact two-field schema. A different definition or
schema cannot claim that interpretation.

## 6. Business quantities, not a model-provider type system

The supported numerical domain reuses Rastok integers and exact decimals;
opening business measures must not make decimal quantities monetary by
default. A measure may describe tokens, items, mass or a declared amount.
The field type remains the quantity authority; an annotation does not
duplicate it with a second carrier declaration authored elsewhere.

D-2026-09-16-6 admits negative quantities, including monetary refunds and
inventory corrections. Signed integers and exact decimals keep their normal
domain; `uint` still excludes negative data. No annotation flag is needed.
Neither a zero nor a negative sample is missing evidence. No absolute-value conversion, saturation at zero,
automatic currency conversion or implicit reversal of an earlier event occurs.
The original signed observations remain available without a computed total.

D-2026-09-16-3 chooses simple, author-chosen ids: `tokens`, `input_tokens`,
`output_tokens`, `carrots`, or another declared business quantity. No OTel,
`rastok.*` or `std.*` prefix is required. An id supplies no interpretation,
implicit unit, instrument, tariff or relationship to another id.

Units are opaque, nonempty text of at most 255 UTF-8 bytes, without control
characters or surrounding whitespace. One id has one unit throughout the
independently validated plan; this also applies to amounts. Two amounts in
different currencies therefore need distinct ids in the same plan. The same
quantity may occur on different integer/decimal field types; inspection keeps
each exact signed value. Missing or malformed values follow ordinary schema
conformance, never a zero default. A mathematical sum that would overflow
does not make its independent original samples invalid.

Provider/model and instance/Call provenance are context, not quantities.
A JSON member's name alone grants no measurement behavior. A consumer must
not silently merge input, output and inclusive totals or count copies of a
value at multiple flow points as new consumption.

Any `gen_ai.*` id has the same authority as any other id and still needs an
explicit unit. Monetary and aggregation arguments are not fields of the
shared schema. No report or subset relationship is implemented in M1.

<a id="61-hydration-supplies-the-values"></a>

### 6.1 Deserialization supplies the values

[D-2026-09-15-6](decisions.md#d-2026-09-15-6--hydrate-annotated-response-types-before-retry-disposition)
chooses the response type itself as the mapping from JSON fields to typed
values. There is no separate extraction DSL. For example, the target response
schema is ordinary nested data, not an `@struct`:

```rkg
type ModelResponse = struct {
  usage: struct {
    input_tokens: uint @measure(id = input_tokens, unit = "token")
    output_tokens: uint @measure(id = output_tokens, unit = "token")
  }
  amount: decimal(38,18) @measure(id = amount, unit = "USD")
}
```

These simple measure ids use the protected typed binding. The declared type
and unit, not the id's spelling, supply its meaning.

Bind it to the existing JSON output (D-2026-09-16-1):

```rkg
connector model : HttpClient {
  codec = json
  url = "https://example.test/model"
  timeout = duration("10s")
  behavior = probabilistic
  output out: ModelResponse
}
```

For `{"usage":{"input_tokens":120,"output_tokens":45},"amount":0.0042}`,
successful deserialization supplies 120 input tokens, 45 output tokens and exactly
0.0042 USD from that response. The annotation arguments do not change; the
values of their target fields supply the samples. Optionality and invalid
fields follow the declared type/codec rules, never an invented zero.
Exact decimal deserialization must not pass through binary floating point.

The selected response schema and its annotations belong to an explicitly
validated Connector codec profile. Every attempt is deserialized and policy
checked before its payload is committed and before Retry retains or publishes
it. The journal therefore retains typed source evidence; a read-only observation
does not decode raw bytes, execute an Operator, create an emission or change
Retry's decision. Held, Consumed and Published still refer to the original
emissions and cannot represent three expenses. Privacy restrictions apply
before retention, not only when reports are rendered.

The HttpClient JSON profile binds this schema through the
existing port table; omission retains `variant`. The M1 codec associates
object fields by name recursively, not by order. It is deliberately not a
general serialization framework. A schema written by hand or generated from
an API description remains an expectation: every actual response is checked.
An incompatible response produces `DecodeFailed` without an `out`, even when
HTTP returned 200 or 500. Native typed Driver completions still pass the
pre-journal conformance gate. No implicit edge conversion, arbitrary field
renaming, partial-record recovery or provider-specific Contract is introduced.
See [types §4.1](types.md#41-explicit-json-response-types-in-m1) for the bounded
schema profile, including opt-in projected JSON for nullable/additional members;
other codecs and ordinary closed structs are unchanged. A projected null supplies
no sample, while a reported numeric zero does. Descriptive annotation definitions
are admitted separately. Generic subsets are outside M1; all numerical quantities use this deserialization path before Retry disposition.

## 7. Extensions can consume metadata without becoming annotations

An Extension or core subsystem may read metadata whose interpretation it
supports through the validated schema. That does not replace Extension
admission, attributes or journal protocols with annotations.

- App retains its deployed public boundary and deployment bindings.
- Loop's `Loop.measure` is a termination proof over state, **not** the
  accounting annotation `@measure`; no ranking or recurrence rule changes.
- Retry retains the original request and every attempt's emissions. Accounting
  reads their metadata and exact attempt correlation. Annotation interpretation
  does not choose `Retry.on`, renew a delay, release a Held batch or authorize
  a new effect.
- Saga and Budget remain named later consumers. No compensation, reservation
  or pre-effect admission is introduced by an observation-only measurement.

Core/hosted consumers still have distinct responsibilities. The metadata
channel is shared; there is no universal callback executed by every
transaction. Lazy accounting and never-block export remain separate from
causal execution, with the existing empty-table fast path preserved.

## 8. Admission and independent validation

The [integration gates](../architecture/annotation-types-m1.md#4-ordered-gates)
own the implementation sequence. The first source slice admits `@struct`
as flat ordinary data, rejects composites after alias resolution and keeps
both the plan and semantic hash equal to the ordinary-struct spelling.
Malformed prefixes recover at the next declaration. A declared annotation
application checks its supplied constants against the resolved flat fields
first. A wrong type, out-of-range value, unknown field or missing required
field reports `E-DATA-ANNOTATION` at the argument, with the target schema path;
diagnostics do not print argument values. Scalar constructors use the existing
constant evaluator, and exact decimal narrowing uses `DataType`. No runtime
binding, environment lookup or I/O is available in that evaluation.

Well-typed descriptive applications now enter the canonical sparse tables.
Independent decode validates their complete argument values, flat schemas,
declaring identities and metadata-only authority, and propagation is
re-derived. Their argument type/value depth counts against the package budget.
They are not silently ignored and cannot masquerade as security or measurement
policies. Unknown definitions and unsupported interpretation tags refuse.
Security interpretations keep their argument rules; numerical measurements
follow the shared signature in §5.

The executable security/privacy interpretations remain authoritative. The initial
flat-data slice changed no plan format, Contract or recording. Typed HTTP
deserialization is part of `http-client@3`, using existing plan and annotation tables;
its namespace movement is enumerated in the v20 inventory. Source, hostile-package,
original-attempt and UI evidence is recorded in the
[qualification ledger](../architecture/retry-c4-qualification.md). Remaining
[opening gates](../architecture/opening-showcase.md) are separate from semantic
admission; this document does not authorize publication.
