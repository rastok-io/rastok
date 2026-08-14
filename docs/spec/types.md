# Rastok logical data types

> **Status: normative M1.3 target.** Until the M1.3 gate closes, a
> construct that the executable subset cannot represent continues to
> refuse by name. This document is nevertheless the semantic authority:
> the implementation is being brought to this model, not the other way
> around.

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
carry their size as a type parameter. The exact physical widths,
temporal units, timezone semantics and decimal bounds are part of the
canonical type, never host-dependent defaults. Friendly source aliases
such as `int`, `long`, `float` and `double` resolve to those explicit
canonical parameters.

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

### 2.1 JSON, Avro and Arrow are projections, not rival type systems

`DataType` is deliberately expressive enough to project the supported
JSON Schema, Avro and Arrow schema profiles without losing a logical
distinction. Those ecosystems do not become three alternative type
authorities:

| External schema | Required logical projection |
|---|---|
| JSON / JSON Schema | null, booleans, exact numbers, text, arrays, objects/maps, enums and exclusive unions (`oneOf` or an unambiguous type array) |
| Avro | primitives and logical types, records, arrays, maps, enums, fixed binary, nullability and unions |
| Arrow | signedness and widths, text/binary/fixed binary, decimal and temporal parameters, struct, list/fixed-size list, map and unions |

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
Process(codec = lines) : in T where LinesEncodable(T), out text
Process(codec = jsonl) : in T where JsonEncodable(T), out array<variant>
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
field. `err = value` is consequently an ordinary port emission.

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
