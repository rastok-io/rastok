# External-schema projections

> **Status: normative M1.3 target.** This document fixes the supported V1
> projection profiles between external schemas and Rastok
> [`DataType`](types.md). A Connector may implement a strict subset, but it may
> not reinterpret or weaken a refusal. Library iteration order and physical
> container layout never become type identity.

Schema projection is a boundary operation:

```text
external schema --parse/validate--> resolved DataType
resolved DataType --capability check--> external schema
```

The no-std core sees only `DataType`. An adapter reports a path plus one of the
loss reasons below; it never falls back to `bytes` or `variant` when a schema
was supplied.

The external authorities for this profile are the
[JSON Schema 2020-12 Core](https://json-schema.org/draft/2020-12/json-schema-core)
and [Validation](https://json-schema.org/draft/2020-12/json-schema-validation)
vocabularies, the
[Apache Avro 1.12 specification](https://avro.apache.org/docs/1.12.0/specification/),
and the
[Apache Arrow columnar format](https://arrow.apache.org/docs/format/Columnar.html).
Rastok deliberately supports the strict subset below; a construct existing in
one of those authorities is not automatically admitted.

## 1. Shared rules

- External names are resolved before projection. Rastok aliases remain
  structural and disappear; a name survives only when it is a tagged-union
  discriminant.
- External field order is presentation. Struct fields, enum symbols and
  tagged cases enter their canonical name order.
- External nullability becomes `T?`. Missing/default behavior is an adapter or
  schema-evolution rule, not a second null value.
- Metadata, documentation, aliases, compression, offsets, dictionary indexes,
  dense/sparse layout and container block order do not enter `DataType`.
- Recursive, reference-cyclic and extension/custom schemas refuse in V1.
- A reverse projection succeeds only when it preserves every logical
  distinction. Otherwise it returns `UnsupportedTarget` or `LossyProjection`
  with the logical path.

The stable loss vocabulary is:

| Reason | Meaning |
|---|---|
| `UnsupportedSource` | the external construct is outside the V1 profile |
| `UnsupportedTarget` | the target ecosystem has no admitted equivalent |
| `InvalidSchema` | the external schema violates its own profile |
| `RecursiveSchema` | a resolved reference reaches itself |
| `LossyProjection` | mapping would erase width, sign, parameter, field or discriminant |
| `AmbiguousUnion` | more than one structural branch can accept one value |
| `InvalidMapKey` | the key type is outside Rastok's canonical-key domain |
| `BudgetExceeded` | the resolved tree exceeds a core type budget |

## 2. JSON and JSON Schema

An untyped `json!` document produces `variant`; this section applies when a
JSON Connector or an explicit narrowing supplies a schema.

| JSON Schema form | Rastok projection |
|---|---|
| `null`, `boolean`, `string` | `null`, `bool`, `text` |
| `integer` without a proven finite 64-bit bound | refused; `number` would erase the integer-only constraint |
| bounded integer whose inclusive range fits one canonical width | smallest unsigned width when the range is non-negative, otherwise smallest signed width |
| `number` | `number` |
| string `enum` with unique members | `enum`, symbols sorted canonically |
| homogeneous `array` | `array<T>`; exact `minItems == maxItems` becomes `array<T,N>` |
| object with a finite `properties` set and `additionalProperties: false` | closed `struct` |
| object with no named properties and one schema-valued `additionalProperties` | `map<text,V>` |
| type array containing `null` plus one type | optional union |
| `oneOf` / type array with several alternatives | structural union only when projected branches are provably pairwise disjoint |

`anyOf`, `allOf`, `not`, conditional schemas, regex/pattern properties,
tuple-prefix arrays, mixed named-plus-additional object fields, arbitrary
numeric/string constraints and `$dynamicRef` refuse in V1. A Connector may
validate such constraints before Rastok, but they do not masquerade as a
weaker type.

Narrowing a `variant` to a structural union tests all branches. Exactly one
must conform. Zero is a type mismatch; two or more is `AmbiguousUnion`. A
configured discriminator may instead target a tagged union; the discriminator
value is mapped to a declared case before its payload is checked.

Standard reverse JSON Schema projection succeeds only for distinctions the
2020-12 vocabularies can express exactly. Inclusive integer bounds preserve
fixed integer widths. Decimal scale, temporal units/timezones, opaque bytes,
UUID semantics and tagged discriminants require a separately registered
target profile; no such extension vocabulary is part of V1. Without one the
adapter returns `UnsupportedTarget` rather than lying with plain `number`,
`string` or an untagged `oneOf`.

## 3. Apache Avro

| Avro form | Rastok projection |
|---|---|
| `null`, `boolean` | same logical type |
| `int`, `long` | signed 32/64-bit integer |
| `float`, `double` | binary32/binary64 float |
| `bytes`, `string` | variable bytes/text |
| `fixed(size)` | `bytes(size)` |
| record | closed struct; field defaults remain adapter data |
| array | variable array |
| map | `map<text,V>` (Avro keys are strings) |
| enum | Rastok enum; Avro symbol ordinal remains codec data |
| decimal logical type | decimal128 when `1..=38` precision and Avro's `0 <= scale <= precision` hold |
| date/time/timestamp/local-timestamp logical types | matching date/time/timestamp/timestamp-ntz constructor |
| duration logical type | `interval(month_day_nano)`; Avro's milliseconds are scaled exactly to nanoseconds |
| UUID logical type | `uuid` |
| union | structural when projected branches are pairwise disjoint; otherwise tagged |

An Avro union that needs tags derives stable case names from the fully
resolved Avro branch name; unnamed branches use their canonical schema name.
Names are unique after qualification. The Avro branch index maps explicitly
to that case and never enters Rastok identity. Two same-shape named records
therefore remain distinguishable without making ordinary record aliases
nominal.

Unknown logical types, recursive records, decimal256-sized precision and
unions that cannot obtain unique stable case names refuse. Avro's container
codec, sync markers and block order are representation facts.

## 4. Apache Arrow

| Arrow type | Rastok projection |
|---|---|
| null/bool | same logical type |
| signed/unsigned integer | same sign and bit width (8/16/32/64) |
| half/single/double float | binary16/32/64 float |
| decimal128 | exact precision and signed scale |
| decimal32/64/256 | refused in V1; only decimal128 has a lossless reverse profile |
| binary/large-binary/view | variable bytes |
| UTF-8/large-UTF-8/view | text |
| fixed-size binary | fixed bytes |
| date/time/timestamp/duration/interval | exact unit and timezone parameters |
| struct | closed Rastok struct; Arrow field nullability becomes optional |
| list/large-list/list-view | variable array |
| fixed-size list | fixed array |
| map | Rastok map when its non-null projected key is in the canonical-key domain |
| dictionary | its decoded value type; Arrow schema metadata does not itself declare a closed symbol set |
| dense/sparse union | tagged union, using canonical child names as cases |

Arrow union type codes map explicitly to canonical case names. Dense versus
sparse layout and child buffer offsets do not enter the logical type. Duplicate
or empty child names, duplicate type codes, and children that cannot obtain a
stable case refuse.

Run-end encoding projects to its decoded value type. Extension types,
tensors and schemas requiring recursive or custom interpretation refuse in
V1 unless a future registered projection owns an exact contract.

## 5. Conformance evidence

Each projection implementation must ship table-driven fixtures that prove:

1. every supported row maps to the exact canonical `DataType`;
2. declaration order, map iteration order and physical union layout do not
   change canonical bytes;
3. reverse projection either round-trips or reports a path-specific loss;
4. every unsupported row refuses with one reason from §1;
5. Avro branch indexes and Arrow type codes preserve the selected Rastok
   union case without becoming logical identity;
6. JSON structural-union narrowing distinguishes zero, one and multiple
   matching branches.
