---
title: Types and schemas
description: Logical types, literal forms and the loss-checked JSON Schema, Avro, Arrow and OpenAPI projections shipped in the alpha.
order: 1
---

Types describe values independently of their byte representation. A schema
projection produces a Rastok type; it does not install a wire codec or a
Driver. The [types chapter](../guide/02-types-and-expressions.md) introduces
the notation.

## Scalars

| Type | Parameters and aliases | Value rule |
|---|---|---|
| `int8`, `int16`, `int32`, `int64` | `int` = `int32`; `long` = `int64` | Signed integer within the declared width |
| `uint8`, `uint16`, `uint32`, `uint64` | `uint` = `uint64` | Unsigned integer within the declared width |
| `float16`, `float32`, `float64` | `float` = `float32`; `double` = `float64` | Binary floating point; a narrower value must round-trip exactly |
| `decimal(p,s)` | Precision `1..38`, scale `-38..38` | Exact decimal128 coefficient and scale; narrowing never rounds |
| `number` | No parameter | Exact integer or decimal, not binary floating point |
| `bool` | No parameter | `true` or `false` |
| `null` | No parameter | The null value |
| `text` | No parameter | Unicode text |
| `bytes` | Variable length | Opaque bytes |
| `bytes(n)` | `1 <= n <= 2147483647` | Exactly `n` bytes |
| `uuid` | No parameter | Exactly 16 network-order bytes |
| `media_type` | No parameter | One concrete media type; canonical names and parameter order, no wildcard |

## Dates, times and intervals

For `time`, `timestamp`, `timestamp_ntz` and `duration`, the unit is
`second`, `millisecond`, `microsecond` or `nanosecond`.

| Type | Default or parameters | Distinction preserved |
|---|---|---|
| `date(unit)` | `day` or `millisecond`; bare `date` means `day` | Date, carried as whole days |
| `time(unit)` | Bare `time` means nanoseconds | Time of day, without a date |
| `timestamp(unit, "zone")` | Bare `timestamp` means nanoseconds in UTC | Instant with a declared zone; `UTC`, `Etc/UTC` and `+00:00` normalize to UTC |
| `timestamp_ntz(unit)` | Nanoseconds by default | Date and time without a zone; not UTC by omission |
| `duration(unit)` | Nanoseconds by default | Signed elapsed quantity, carried in i64 nanoseconds |
| `interval(year_month)` | Months | Calendar interval |
| `interval(day_time)` | Days and time | Calendar interval with millisecond precision |
| `interval(month_day_nano)` | Signed months, days and nanoseconds | Arrow interval constraints |
| `interval(month_day_millis_unsigned)` | Unsigned 32-bit months, days and milliseconds | Avro duration constraints, distinct from the Arrow interval |

Timestamp carriers contain signed i64 seconds and a nonnegative nanosecond
fraction. The unit limits precision, not the seconds range. Arithmetic
outside the carrier refuses. Logical execution time and civil timestamps
are separate: `now()` observes the recorded origin plus the transaction's
logical instant.

## Composite types

| Form | Meaning | Constraints |
|---|---|---|
| `struct { name: text, count: int }` | Named fields | Closed, unique fields; declaration order is not type identity |
| `array<T>` | Ordered homogeneous sequence | Variable length |
| `array<T,N>` | Fixed sequence | Exactly `N` elements, including `N = 0` |
| `map<K,V>` | Homogeneous keyed collection | Canonical, non-null scalar or enum keys |
| `enum { ready, done }` | Named unit cases | Unique symbols, canonical name order |
| `T?` | `T` or null | Absence and null are distinct at a boundary |
| `A \| B` | Structural union | Branches must be pairwise disjoint; exactly one branch matches |
| `union { Ok { value: text }, Refused }` | Tagged union | Named cases with closed payloads; the case remains part of the value |
| `variant` | Self-describing value tree | Narrow with `is` or `as` before typed member or index access |

Map keys admit booleans, fixed-width integers, text, media types, bytes,
decimals, dates, times, timestamps, durations, UUIDs and enum symbols.
Floats, `number`, null, intervals, dynamic values and composite keys refuse.
Instance keys additionally admit closed structs of valid key leaves.

Named type declarations are structural aliases. Recursive aliases refuse.
Type trees are bounded to 128 nesting levels, 4096 nodes and 256 members per
aggregate. `type Name = @struct { ... }` declares a flat, annotation-capable
struct; annotation use and interpretation are described in
[Privacy, secrets and measurements](../guide/08-privacy-secrets-and-measurements.md).

## Standard types without imports

The [standard library](stdlib.md#types) lists the named types available
without imports, including `FileExit` and `ProcessExit`. They use the same
definitions as the Connector ports, independently of the physical Drivers
included in the binary. Write `output failed: FileExit` without copying its
declaration; the library reference explains name collisions and the module
and import design still to be discussed.

## Literals and constructors

| Spelling | Result | Notes |
|---|---|---|
| `42`, `42u`, `1.5`, `true`, `null`, `"Ada"` | Intrinsic scalar | Ordinary expression literals retain their checked type |
| `decimal("12.30")` | `number` carrying an exact decimal | A declared decimal target checks precision and rescales exactly |
| `duration("2h30m")` | `duration` | Signed duration; units include `h`, `m`, `s`, `ms`, `us`, `ns` |
| `timestamp("2026-10-30T12:00:00Z")` | `timestamp` | Parsed civil instant |
| `media_type("application/json")` | `media_type` | Parses and canonicalizes a concrete media type |
| `json! { "count": 42 }` | `variant` | Also accepts scalar JSON; duplicate keys refuse; numbers stay exact |
| `text! { message }` | `text` | Verbatim reader |
| `base64! { SGk= }` | `bytes` | Standard base64, padding required; ASCII whitespace ignored |
| `asset("fixtures/sample.pdf")` | `bytes` | Bytes read at build time and embedded in the plan |

Tagged frontends occupy a complete value position. Bind their result before
using it inside a larger expression. See [Expressions and dispatch](expressions.md).

## Schema projection rules

The schema projection adapters report a path and a named loss reason.
Reverse projection succeeds only when projecting the result back preserves
the logical type. Source field order, physical union indexes and layout do
not become type identity.

| Refusal | Meaning |
|---|---|
| `UnsupportedSource` | Construct outside the admitted source subset |
| `UnsupportedTarget` | No exact representation in the selected target |
| `InvalidSchema` | Invalid schema or invalid parameters |
| `RecursiveSchema` | Reference cycle |
| `LossyProjection` | Conversion would discard a constraint or distinction |
| `AmbiguousUnion` | Overlapping branches |
| `InvalidMapKey` | Key outside the canonical-key domain |
| `BudgetExceeded` | Resolved type exceeds a type-tree bound |

### JSON Schema 2020-12

| Source or direction | Projection or refusal |
|---|---|
| `null`, `boolean`, `string`, `number` | `null`, `bool`, `text`, exact `number` |
| Integer with inclusive bounds exactly matching a machine width | Corresponding signed or unsigned integer |
| Integer range `0..10` | `LossyProjection`, rather than widening to a machine integer |
| String enum; homogeneous array; equal `minItems` and `maxItems` | Enum; variable array; fixed array |
| Required named properties and `additionalProperties: false` | Closed struct |
| Schema-valued additional properties without named fields | `map<text,V>` |
| Null plus one type; disjoint `oneOf` | Optional; structural union |
| Optional property without a fill policy; required key on a map | `LossyProjection` |
| Boolean schema, `pattern`, conditional constraints, tuple prefixes | `UnsupportedSource` |
| Cyclic local reference; overlapping `oneOf` | `RecursiveSchema`; `AmbiguousUnion` |
| Invalid definition or empty type array | `InvalidSchema` |
| Reverse bytes, decimal scale, time units or tagged discriminants without an extension profile | `UnsupportedTarget` |

### Avro 1.12

| Source or direction | Projection or refusal |
|---|---|
| Null, boolean, int, long, float, double, bytes, string | Matching logical scalar; integers retain 32/64-bit widths |
| Fixed, record, array, string-keyed map, enum | Fixed bytes, closed struct, array, text map, enum |
| Decimal logical type | Decimal128, precision up to 38, `0 <= scale <= precision`; fixed storage must fit |
| Date, time, timestamp and local-timestamp logical types | Matching date/time constructors, including zone-free timestamps |
| Duration; UUID | `interval(month_day_millis_unsigned)`; `uuid` |
| Disjoint union branches | Structural union with an explicit branch-index mapping |
| Same-shape named record branches | Tagged union using resolved names |
| Recursive record | `RecursiveSchema` |
| Precision 39 or unknown logical type | `UnsupportedSource` |
| Insufficient fixed decimal storage, forward undefined name or duplicate underlying union branch | `InvalidSchema` |
| Reverse type without an exact Avro form | `UnsupportedTarget` or `LossyProjection` |

### Arrow

| Source or direction | Projection or refusal |
|---|---|
| Integers, floats, decimal128 | Width, sign, precision and signed scale preserved |
| Binary, large binary, binary view; UTF-8 variants | Bytes; text |
| Fixed binary; list variants; fixed list | Fixed bytes; variable array; fixed array |
| Date, time, timestamp, duration, interval | Unit and zone preserved |
| Struct, nullable field, map | Closed struct, optional field, canonical-key map |
| Dictionary or run-end encoding | Decoded logical value type |
| Dense or sparse union | Tagged union with explicit child-name/type-code mapping |
| Extension type, decimal32/64/256 or out-of-range scale | `UnsupportedSource` |
| Nullable or unsupported map key | `InvalidMapKey` |
| Duplicate union type codes or child names | `InvalidSchema` |
| Reverse tagged union with 129 cases | `UnsupportedTarget`: the flat target union cannot address that many children |

### OpenAPI 3.0 and 3.1

`rastok --resolve-open-api <project-dir> [--check]` discovers static HTTP
endpoints and generates `openapi.generated.rkg`. It leaves authored sources
untouched. `--check` reports a missing or stale generated file without writing.

| Source | Projection or refusal |
|---|---|
| Local component reference | Expanded alias; cycles give `RecursiveSchema` |
| 3.0 `nullable: true`; 3.1 type array with null | `T?` |
| Integer format `int32` or `int64` | Corresponding signed width; unbounded integer gives `LossyProjection` |
| Number, number format float/double | Exact `number`, binary32/binary64 |
| String, uuid, date, date-time, binary/byte, media-type formats | Text, UUID, date, timestamp, bytes, media type |
| Fully required closed object; pure additional-property map | Closed struct; `map<text,V>` |
| Disjoint `oneOf` | Structural union |
| Optional fields, open/mixed object or required map keys | `LossyProjection` |
| Unknown format, discriminator, `readOnly` or `writeOnly` | `UnsupportedSource` |
| Dynamic endpoint, no matching document or ambiguous document | Resolver refusal; no guessed endpoint |
| Component name that is a standard type name | `UnrepresentableName`; no conflicting declaration is written |

The HttpClient `json_profile = projected` setting is a separate boundary policy:
it can skip extra fields and fill omitted nullable fields with null. It
still rejects a wrong type or missing required field. It does not widen the
OpenAPI schema projection above.

## Specified, not shipped

| Item | Specification |
|---|---|
| Registered extension profiles for projections that need additional schema vocabulary | `docs/spec/schema-projections.md` §§1–4 |
| Decoder availability resolved through an attested registry | `docs/spec/envelope.md` §5.1 |

Sources: `docs/spec/types.md`, `docs/spec/schema-projections.md`;
executable projection evidence in `crates/rastok/tests/schema_projections.rs`
and the OpenAPI projection/resolver tests. Runtime wire support remains
specific to each [Contract](contracts.md).
