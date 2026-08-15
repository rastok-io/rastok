# Rastok logical data types

> **Status: normative M1.3 target.** Until the M1.3 gate closes, a
> construct that the executable subset cannot represent continues to
> refuse by name. This document is nevertheless the semantic authority:
> the implementation is being brought to this model, not the other way
> around.
>
> **Implementation checkpoint (B6 + B3 + B1 completion, 2026-08-15).** The no_std
> core owns the complete `DataType` and materialized `Value` algebras for the
> strict JSON Schema, Avro and Arrow profiles; the `expr` engine consumes those
> authorities. Deterministic `is`/`as` narrowing for precise,
> structural-union and tagged-union targets is executable in memory. Tagged
> constructors and contextual structural-union wrapping are now checked and
> executable in memory; `err` uses the same emission path as any port. Package
> v6 categorically refuses the new families and expression forms; B8 publishes
> their sole v7 encoding.

Rastok owns the logical type system of envelope payloads. The expression
language, Connectors, codecs, guest frontends, state and metadata consume
that system; none of them defines a competing one.

## 1. Four planes, one vocabulary

These planes are orthogonal:

| Plane | Question | Authority |
|---|---|---|
| Logical type | What value does the graph manipulate? | `DataType`, resolved in the graph schema |
| Internal representation | How is that value held now? | `Materialized(Value)` or `Encoded { encoding, bytes }` |
| Connector codec | How does an external boundary turn bytes into a value, or a value into bytes? | the instantiated Connector contract |
| External content type | What does the outside protocol call those bytes? | `meta.ext.content_type` |

The persisted and transported representation of `data` is bytes. That
does **not** make every logical payload `bytes`: `bytes` is the logical
type of opaque binary content. An Avro document can be a logical
`struct` represented as Avro; an Arrow IPC buffer can be a logical
`array<struct>`; a PDF whose structure Rastok does not interpret is
logical `bytes` represented as raw bytes.

The full logical type is resolved through the emitting graph's schema
and `graph_version`; it is not repeated as a complete tree in every
envelope. A union value still carries the discriminant needed to recover
its selected case.

## 2. The logical type tree

The type family is:

```text
DataType = scalar
         | struct
         | map
         | array
         | enum
         | union
         | variant
```

Scalars include `null`, `bool`, signed and unsigned integers with an
explicit width, floating-point values with an explicit width, exact
`decimal(p,s)`, exact `number`, `text`, `bytes` and the logical families anchored on
Arrow semantics (`date`, `time`, `timestamp`, `timestamp_ntz`,
`duration`, `interval`, `uuid`). Fixed-size binary and fixed-size lists
carry their size as a type parameter. Integer and floating widths,
temporal units, timezone semantics and decimal bounds are part of the
canonical type, never host-dependent defaults. Friendly source aliases
such as `int`, `long`, `float` and `double` resolve to those explicit
canonical parameters.

The canonical scalar constructors and source aliases are exact:

| Canonical constructor | Admitted parameters | Source spellings |
|---|---|---|
| `signed(bits)` | `bits ∈ {8,16,32,64}` | `int8`/`int16`/`int32`/`int64`; `int = int32`, `long = int64` |
| `unsigned(bits)` | `bits ∈ {8,16,32,64}` | `uint8`/`uint16`/`uint32`/`uint64`; `uint = uint64` |
| `float(bits)` | `bits ∈ {16,32,64}` | `float16`/`float32`/`float64`; `float = float32`, `double = float64` |
| `decimal(precision, scale)` | decimal128 only: `1 ≤ precision ≤ 38`, `-38 ≤ scale ≤ 38` | `decimal(p,s)` |
| `number` | exact integer-or-decimal, never binary floating point | `number` |
| `bytes(length?)` | absent = variable; fixed `1 ≤ length ≤ 2³¹-1` | `bytes`, `bytes(n)` |
| `text`, `bool`, `null`, `uuid` | no parameter | same word |
| `date(unit)` | `day | millisecond`; `date` defaults to `day` | `date`, `date(unit)` |
| `time(unit)` | `second | millisecond | microsecond | nanosecond`; `time` defaults to `nanosecond` | `time`, `time(unit)` |
| `timestamp(unit, zone)` | the four time units; `zone = utc | named(text)` | `timestamp`, `timestamp(unit, "zone")` |
| `timestamp_ntz(unit)` | the four time units; deliberately no zone | `timestamp_ntz`, `timestamp_ntz(unit)` |
| `duration(unit)` | the four time units; default nanosecond | `duration`, `duration(unit)` |
| `interval(unit)` | Arrow: `year_month | day_time | month_day_nano`; Avro: `month_day_millis_unsigned` | `interval(unit)` |

`timestamp` with no arguments is `timestamp(nanosecond, utc)`. A named
timezone is identity material; the spellings `UTC`, `Etc/UTC` and
`+00:00` normalize to the canonical `utc` case. A timezone-naive value is
not UTC by omission: it is `timestamp_ntz`. Offset widths for variable
text/binary and Arrow dense/sparse, dictionary or run-end layouts are
representation facts and do not create logical types.

`Value` carries the canonical data, not a second copy of its surrounding
`DataType`. The resolved port/state/schema selected through `graph_version`
owns constraints such as integer and float width, fixed collection length,
decimal precision, temporal unit and timestamp timezone. An envelope does not
repeat that tree merely because its payload is materialized.

The materialized carriers are consequently smaller than the type algebra:

- signed integers use one `i64` carrier and unsigned integers one `u64`
  carrier; the declared width is checked by `DataType`;
- binary16 and binary32 values are promoted exactly into the binary64
  carrier. Conformance to a narrower float requires an exact round trip;
  canonical NaN and signed zero remain value distinctions;
- variable/fixed bytes share one byte sequence, and variable/fixed arrays
  share one sequence. The declared length and element homogeneity remain in
  the type; a `variant` array may remain dynamically heterogeneous;
- a date is canonical whole days; time, timestamps and durations are
  canonical nanoseconds; an interval carries its month/day/nanosecond
  components. Unit, timezone and external physical width remain type
  parameters. The V1 value bound is the published i64 nanosecond domain;
  an external value outside it refuses instead of overflowing;
- UUID carries exactly 16 network-order bytes;
- `map<text,V>` uses the unique text-keyed form; other admitted key types use
  canonical tagged key values. Two canonically equal keys refuse;
- enum symbols and both union discriminants remain in the value because they
  select semantic data, not merely a schema constraint.

A decimal value carries scale because scale is part of its exact canonical
value, but does not repeat precision; conformance checks its coefficient
against the declared precision and requires its carried scale to equal the
declared scale. A schema-less exact-number value may use the wider decimal128
exponent domain (`-6143..=6144`) already fixed by the value registry;
narrowing it to `decimal(p,s)` rescales it **exactly** to the declared `s` in
`-38..=38`. Integers enter with scale zero. Increasing the scale pads the
coefficient with zeros; decreasing it is legal only when every removed digit
is zero. Coefficient overflow, an inexact scale reduction, or excess declared
precision is `NoMatch`, never rounding. The narrowed value then carries the
declared scale, irrespective of whether it came from a minimal-scale JSON
number, `decimal("...")`, or an already typed decimal. `number` and `variant`
are types, not wrapper values: their values retain the concrete canonical
scalar or tree cases they contain, but do not remember constraints discarded
by an explicit conversion to `variant`.

`number` is a closed exact-numeric scalar, not a floating-point escape
hatch: it preserves whether its value is an integer or a bounded exact
decimal until an explicit narrowing chooses a fixed integer, decimal or
floating representation. It is useful for JSON Schema's numeric domain;
it does not make unrelated fields dynamically typed.

- `struct` is a record with named fields. Field order is presentation;
  field names and resolved types are semantics.
- `map<K,V>` is a homogeneous keyed collection. Its admitted key domain
  must have a total canonical representation.
- `array<T>` is an ordered homogeneous sequence.
- `enum` is a closed set of named unit cases.
- A **structural union** (`A | B`) selects one pairwise-disjoint logical
  branch from the value's type. `T?` is the optional spelling built from
  `T` and `null`.
- A **tagged union** has named cases with optional payloads. The case name
  is semantic data and remains visible at runtime; it is the form used
  for protocol and lifecycle outcomes such as `ProcessExit`.
- `variant` is the sole explicitly dynamic type: a self-describing tree,
  notably produced by JSON and jq. It must be narrowed before typed
  structural use.

`table` is not a type. It is the relational view of
`array<struct<...>>`; Arrow/Polars/SQL representations do not add a
second logical family.

Named type declarations are structural aliases. Their fully resolved
expansion is the type identity, so renaming an alias does not change a
plan. Tagged-union case names are not aliases and do affect identity.
Recursive aliases remain illegal in v1.

### 2.1 Canonical composite invariants

The resolved tree admits no source-order aliases:

- struct fields are closed, unique and sorted by field-name UTF-8 bytes;
  an open `struct { .. }` exists only as a compiler constraint and must be
  closed before a validated plan is built;
- enum symbols and tagged-union cases are unique and sorted by their UTF-8
  bytes; enum declaration order and an external ordinal stay adapter facts;
- an array carries an element type and optional fixed length; `array<T>` is
  variable and `array<T,N>` is fixed (including `N = 0`);
- a map carries both key and value types. V1 map keys are closed scalar or
  enum types with a total canonical value encoding: bool, fixed-width
  integer, text, bytes/fixed bytes, decimal, date/time/timestamp/duration,
  uuid or enum. Float, `number`, null, interval, variant and every composite
  key refuse by path;
- text-keyed maps and non-text keyed maps have one materialized spelling each;
  the latter are order-insensitive and sort by canonical tagged key bytes;
- structural-union branches are flattened, unique, pairwise disjoint and
  sorted by their canonical type bytes. Optional `T?` resolves to that same
  union with `null`; source branch order is not identity;
- tagged-union cases are named semantic discriminants. Each case carries a
  closed struct payload (the empty struct is the unit case). A tagged value
  stores the case name plus that payload; case order is not identity.

A structural-union value stores the selected canonical branch index as well
as its value. Although disjointness lets Rastok verify the selection, keeping
the discriminant preserves an Avro/Arrow choice at opaque and replay
boundaries. A mismatching discriminant is malformed data, never recomputed
as first-match-wins. The index is relative to that **exact resolved union**.
Consequently, ordinary assignability between two structural-union types
requires identical resolved unions; adding or removing a branch requires an
explicit `as TargetUnion`, which unwraps the selected value and performs one
exactly-one branch selection against the target's canonical order. This is a
typed conversion, not a boundary repair of malformed data.

Every type is checked against its budgets before admission into a resolved
or validated form: nesting ≤ 128, expanded nodes ≤ 4096,
fields/symbols/cases ≤ 256 per composite, and fixed sizes fit the declared
31-bit bound. These are validity rules of `DataType`, not parser
implementation limits or promises about allocator capacity.

### 2.2 JSON, Avro and Arrow are projections, not rival type systems

`DataType` is deliberately expressive enough to project the supported
JSON Schema, Avro and Arrow schema profiles without losing a logical
distinction. Those ecosystems do not become three alternative type
authorities:

| External schema | Required logical projection |
|---|---|
| JSON / JSON Schema | null, booleans, exact numbers, text, arrays, objects/maps, enums and exclusive unions (`oneOf` or an unambiguous type array) |
| Avro | primitives and logical types, records, arrays, maps, enums, fixed binary, nullability and unions |
| Arrow | signedness and widths, text/binary/fixed binary, decimal and temporal parameters, struct, list/fixed-size list, map and unions |

The exact supported profiles and every named refusal are fixed by
[`schema-projections.md`](schema-projections.md). An adapter may preserve
representation metadata in its codec, but it may not weaken that matrix.

Avro union branches and Arrow union type codes are explicit
discriminants. They project to a Rastok tagged union whenever their
identity cannot be recovered from pairwise-disjoint structural types.
The projection therefore preserves the selected branch even when two
payload schemas happen to have the same shape. Avro's branch index and
Arrow's dense-versus-sparse layout remain codec/representation facts;
they are mapped to the logical discriminant rather than embedded as a
second value model.

JSON values do not lose access to unions merely because `json!` first
produces `variant`. A typed JSON Connector, a declared port constraint,
or `as TargetUnion` validates the value against the target union. A
configured discriminator selects a tagged case. Without one, exactly one
structural branch must conform: zero matches is a type error and several
matches are an ambiguity error. There is no first-branch-wins rule and no
fallback to `variant`.

Arrow dictionary and run-end encodings, Avro container compression and
the dense/sparse choice for Arrow unions are internal representations,
not logical types. A dictionary with a schema-known closed value set may
project to an `enum`; otherwise it projects to its decoded value type.
An extension or recursive external schema outside the supported v1
profile is refused with its path and reason. It is never silently
flattened to `bytes` or `variant`.

Schema translation lives at std/Connector boundaries. The no_std core
owns only the canonical `DataType` and `Value` algebra; it does not depend
on Avro, Arrow or JSON Schema libraries.

## 3. Inference is not dynamic typing

A source annotation is optional where the graph supplies enough
constraints:

```rastok
output result
output result: Customer
```

The compiler may use internal type variables while solving the graph,
but a validated plan contains only effective resolved types. These
states are deliberately distinct:

```text
annotation omitted  != variant
unsolved type        != variant
type error           != variant
```

An exact source annotation is a public constraint. The resolved plan
therefore records both the effective type and whether the constraint was
declared or inferred; it never represents “no effective type” with an
`Option<DataType>`.

Every Node has typed input and output ports under this one mechanism.
`in`, `out` and `err` are conventional default names, not another port
category. Connector contracts conventionally use `exit` for the terminal
outcome of one admitted invocation. A contract may add any number of other
outputs (`rejected`, `audit`, ...), all judged by the same conformance
relation.

The `exit` convention preserves the original typed-error requirement while
removing its special channel: terminal error codes are tagged-union cases on
`exit`, next to successful or neutral terminal cases. Diagnostic bytes remain
ordinary values on `err`.

### 3.1 Invocation exit is not partition stop

The scopes are nested but distinct:

```text
Node definition
  -> partition instance*                 ended by stop
       -> Connector invocation*           ended by exit
            -> out* + err* + one exit
```

A partition may run several invocations, including concurrent ones when its
resolved concurrency policy permits it. Their outputs carry the call identity
and partition-instance generation needed for causal correlation. `exit` is
the terminal barrier for one invocation: no output for that call may follow
it. `stop` begins closure of the containing partition instance. Closure must
refuse new calls and settle every admitted obligation by draining or
cancelling it under a bounded declared policy; it cannot substitute one
partition event for the calls' individual `exit` values.

Framing controls how bytes become values; publication controls when those
values become visible. A line-framed stream may therefore emit many envelopes
on `out` or `err`, whereas an on-exit policy may commit the ordered emissions
with the terminal `exit`. A collector may aggregate them, but only under an
explicit bound. These choices do not change the logical port types or the
one-exit-per-invocation invariant.

## 4. Parametric Connector contracts

A resolved type is concrete **for one Connector instantiation in one
validated plan**. A Connector sheet need not be permanently
monomorphic. Its port rules may be:

- a fixed logical type;
- a type variable inferred from an attached edge;
- the same type as another port;
- a type selected by a profile or attribute;
- a variable constrained by a codec capability.

Examples:

```text
Process(codec = lines) : in text, out text (0..n emissions)
Process(codec = jsonl) : in T where JsonEncodable(T), out variant (0..n emissions)
Process(codec = bytes) : in bytes, out bytes (0..1 emission)
Kafka(schema = OrderV2) : out OrderV2
File(codec = parquet, schema = Row) : out array<Row>
```

The compiler resolves these rules and stores the effective port table in
the plan. The runtime never consults the compiler's `Registry` and never
silently invents a conversion.

A codec changes representation at a boundary. A change of logical type
is an explicit Operator. Connecting `OrderV1` to `OrderV2` without an
adapter is a type error even when a mechanical migration could be
guessed.

## 5. Values, expressions and narrowing

There is one materialized `Value` domain conforming to `DataType`.
Expressions operate directly on those values; an evaluator may carry an
internal lazy payload witness, but that witness is not a serializable
logical value.

`variant` and tagged unions require checked narrowing. The common
surface is `is` for a predicate/narrowing guard and `as` for an explicit
checked conversion. The exact failure routing is closed by the M1.3
expression gate; no field access may make an implicit dynamic cast.

Tagged-union construction is a general value form, not an error-only
syntax:

```rastok
exit = Exited { exit_code = 0, signal = null }
```

The same constructor may target any compatible port, state cell or
field. A contextual write to a union performs its one legal wrapping at
check/lowering time: for example, `out = 5` targeting `int32 | text` records
the `int32` branch, and `signal = null` records the null branch of `int32?`.
The checker refuses zero or several matching branches with `E-PORT-TYPE`.
The VM and Connector adapters never infer, repair or re-index a union
discriminant; their final conformance gate only verifies the already wrapped
value. An already wrapped structural union may cross an ordinary edge only
when source and target have the identical resolved union type; conversion to
a different union is explicit `as`. `err = value` is consequently an ordinary
port emission.

`Case { ... }` occupies one complete host value position. The parser records
the shape without consulting a type registry; the checker resolves the case
from the typed destination or, without one, from exactly one named union in
the compilation unit. Reusing a case name across unions is legal, but an
untyped construction is then ambiguous and must acquire an explicit typed
destination. Payload fields are closed, unique and canonicalized by name.
This constructor does not revive old configuration initializers: an obsolete
initializer with no genuine tagged-union context retains its migration.

## 6. Schema evolution and explicit adapters

`meta.sys.graph_version` selects a retained manifest row containing the
schemas needed to decode that build's envelopes. During a rolling
upgrade, envelopes from several graph versions may coexist and remain
decodable.

Changing a schema never rewrites an old envelope in place. An explicit,
typed Operator performs the migration and emits a new envelope:

```text
Envelope<OrderV1, graph_version=41>
             -> UpgradeOrderV1ToV2
             -> Envelope<OrderV2, graph_version=42>
```

The new envelope names the old one through normal lineage, and the
transaction is journaled like any other transformation. An IDE may
generate an adapter skeleton, but the resulting RKG remains visible,
checked and reviewable.

This schema adapter is distinct from:

- a package-format reader (`.rkp` v6 versus v7);
- an internal representation codec;
- a journal-format decoder.

Migration of an existing journal onto a modified graph remains deferred;
until its execution contract lands, replay across unequal semantic
hashes is a typed refusal, never an implicit application of a shim.

## 7. Data classifications are a separate future layer

A future marker such as `@sensitive` classifies a schema field or port;
it does not create a new logical string or struct type. Classification
will eventually govern propagation, sinks, redaction, retention,
telemetry and Lab fixtures. It belongs beside `DataType` in resolved
schema metadata, and will affect plan identity when it affects policy.

M1.3 neither assigns the `@` syntax nor serializes an empty annotation
table pre-emptively. The extension point is explicit; the policy model is
deferred.
