# Rastok expressions and tagged frontends

> **Status: normative and executable through M1.3 B9.** The
> executable module is `expr` and consumes core `DataType`/`Value`. Its
> `is`/`as`, closed-struct and tagged-constructor trees execute, are
> type-checked, and travel in the v7 registry with their resolved targets.
> Categorical package v12 extends that registry only with the open
> `media_type(text)` constructor. The
> `IDENT!` frontend surface and full JSON-value reader described below are
> executable. A `variant` must be narrowed before member or index access.

Rastok has an intrinsic, pure expression language. Its syntax and much of
its operator semantics are inspired by CEL, and the compatible core stays
differentially tested against cel-spec. CEL is an oracle for that shared
subset, not the owner of Rastok's payload types or values.

## 1. Expression contract

An expression:

- is pure and deterministic;
- terminates under a declared structural/cost bound;
- has no ambient clock, environment, filesystem, network or randomness;
- reads only names present in its compile-time signature;
- is checked against the logical [`DataType`](types.md) domain;
- evaluates to the one Rastok materialized `Value` domain.

The admitted CEL-compatible core includes literals, paths,
member/index access, arithmetic, comparisons, short-circuit logic,
ternary expressions, lists, maps and the existing bounded functions.
Rastok-owned extensions include exact `decimal(...)`, canonical
`media_type(...)`, checked `as`,
type/case tests through `is`, structs, tagged unions and the
Arrow-anchored logical types.

Every divergence from the CEL-compatible core is named and tested. In
particular, Rastok's comments, line-bounded strings, absence rules,
narrowing and type domain are not described as upstream CEL behavior.

`instance` is a reserved contextual root, like `data` and `meta`. Its
currently admitted member is `instance.key`, available only while checking or
running a Node that declares `instance.key`. It denotes that owner's local
selected key; it never searches an ancestor and never becomes null. An
enclosing Graph passes such context to descendants through an immutable
lexical `val`.

Expression availability is phase-checked. Unit/App boot `val`s and effective
settings bound at boot may use `env()` / `secret()`. A Node-level per-instance
`val`, or an effective Connector setting bound at the instance site, may use
enclosing immutable bindings and that owner's local key but not `data`, `meta`,
or `var`. Handler expressions have their ordinary transaction bindings;
invocation-site Connector settings use their declared request/call signature.
A contract declares the latest permitted site, while checking selects and the
plan transports the earliest actual site at which the expression closes. A
plan-shaping contract attribute permits only the plan site and cannot become
dynamic by spelling.

### 1.1 Compatibility oracle and divergence ledger

The differential harness pins dev-only `cel-interpreter = 0.10.0` with
default features disabled. It compares independent parse/evaluation results
for literals, integer/unsigned/double arithmetic, text concatenation,
comparisons, short-circuit boolean logic, ternaries, list/map indexing and
`size`. Rastok's ordinary unit and conformance tests remain the authority;
the oracle detects accidental drift only in this named common subset.

| Surface | Relationship to CEL | Rastok authority |
|---|---|---|
| literals/operators/ternary/list/map/`size` in the harness | differentially equal | pinned oracle test plus Rastok tests |
| `//` comments and ordinary string escapes | compatible subset, with Rastok's line boundary made explicit | lexer and recovery tests |
| `/* ... */` comments | Rastok extension | shared `trivia` recognizer |
| `decimal(...)`, `media_type(...)`, `is`, `as`, named types and union cases | Rastok extensions | `DataType`, `Value`, typed narrowing tests |
| absence/optional access, macros/comprehensions and `in` | not admitted in this build | named compile-time refusals |
| `variant`, structs, enums and structural/tagged unions | Rastok payload semantics, not CEL types | [`types.md`](types.md) |

This is a test-only dependency: it adds 38 package entries to the lockfile,
none to runtime or no_std builds. The repository previously had no
independent expression evaluator, so self-tests alone could not provide the
differential capability. The crate is exact-pinned and MIT licensed; the
exit strategy is this versioned source corpus plus a replacement oracle
adapter, with no production API or package-format consequence.

`media_type(text)` parses one concrete Internet media type and returns the
canonical open `media_type` scalar. It rejects malformed syntax, duplicate
parameters and wildcard media ranges. A source literal written directly in a
position already typed `media_type` is the only implicit convenience; a
dynamic `text` or `variant` must call the constructor or use an explicit
`as media_type`. This keeps a failed protocol parse visible instead of hiding
it in assignment.

## 2. A tagged frontend is not an expression keyword

Any value whose bytes belong to a dedicated reader or compiler uses one
surface:

```text
tagged_value ::= IDENT "!" frontend_args? frontend_payload
```

The bang is lexically adjacent to the name; trivia between them is a syntax
error, not an alternative spelling:

```rastok
json! { "name": "Ada" }
text! { verbatim text }
base64!("image/png") { iVBORw0KGgo... }
jq!(data) { .items[] }
sql!(data) { SELECT * FROM input }
starlark!(data) { ... }
```

After an identifier, the single `!` token selects the tagged-frontend
production. The parser does not need a closed keyword list or a registry
lookup to choose between a Rastok expression and a frontend. Built-in and
application-registered frontends have exactly the same syntax.

A tagged frontend occupies one complete host `value` position. It is not an
expression atom: `val x = json! 2` is valid, while
`val x = 1 + json! 2` refuses and must bind the frontend result first. This
keeps the frontend's region/framer outside the intrinsic expression parser.
Maximal munch remains lexical: `json!= x` is the ordinary expression
`json != x`, because `!=` is one token; it is never read as `json!` followed
by `=`.

The registry is consulted only after that syntactic choice, to obtain the
frontend's contract, framer/compiler and input/output type rules. The
selected frontend owns its payload bytes and returns one complete region
or one typed refusal; the host resumes after that region and never
tokenizes its interior.

The sigil answers **which frontend owns the bytes**. It does not by
itself solve **where the region ends**: JSON is self-delimiting, while a
guest with its own strings/comments must supply the registered framing
contract chosen in the guest slice.

## 3. Built-in readers and guests

The common syntax does not erase semantic distinctions:

| Frontend | Kind | Logical result |
|---|---|---|
| `json!` | data reader | `variant` |
| `text!` | verbatim reader | `text` |
| `base64!` | binary reader | `bytes` |
| `jq!` | pure guest compiler | normally `variant -> variant` |
| `sql!` | pure guest compiler | relational view over `array<struct>` |
| registered name, e.g. `starlark!` | contract-selected frontend | its declared input/output types |

`asset(...)`, `duration(...)` and `decimal(...)` remain ordinary
expression constructors: they take ordinary arguments and do not hand a
source region to another parser. `nothing` remains a structural host
word, not a frontend.

Frontend names are not reserved as identifiers. `val json = 1` and
`json! 1` are unambiguous programs.

## 4. Full JSON values return

The bang removes the ambiguity that previously forced JSON to object and
array documents. `json!` accepts every JSON value:

```rastok
json! { "name": "Ada" }
json! [1, 2, 3]
json! "Ada"
json! 42
json! -19.5
json! true
json! false
json! null
```

Every spelling above has static type `variant`. The untagged Rastok
literal keeps its static type:

```rastok
42             // int
"Ada"          // text
json! 42       // variant containing a JSON number
json! "Ada"    // variant containing a JSON string
```

The house JSON reader consumes exactly one value and reports its end.
Trailing non-trivia before the host position's terminator is a JSON
diagnostic, never a continuation in the expression language. Duplicate
keys refuse. Numbers keep the existing exact integer/decimal128 rules;
parsing never silently narrows a JSON number to `double`.

## 5. Narrowing

The expression language must be able to consume the types the envelope
system defines. `is` tests a logical type or tagged-union case and makes
the successful branch narrowable; `as` requests a checked conversion:

```rastok
on exit where data is Exited {
  out = data.exit_code
}

val customer = data as Customer
```

The implemented minimum is deliberately small and deterministic:

- `is T` returns false for an ordinary non-match, but ambiguity or malformed
  discriminator configuration is a typed evaluation fault;
- `as T` returns the narrowed value or the same typed `NarrowingFailure`;
- structural unions require exactly one conforming branch and record its
  canonical branch index;
- a fixed decimal target rescales integer or exact-decimal carriers to its
  declared scale without rounding; padding zeros is exact, removing non-zero
  digits or exceeding precision is `NoMatch`;
- an already wrapped structural union flows implicitly only to the identical
  resolved union. Explicit `as OtherUnion` reselects the exactly-one target
  branch and records its target-relative canonical index;
- after a successful narrowing, expression operators consume the selected
  structural value; the branch index remains present in the materialized
  `Value`, journal and identity and is never discarded at those boundaries;
- tagged unions preserve their case name; an untagged variant needs an
  explicitly resolved discriminator before it can become one;
- a positive `is` fact narrows the right side of `&&`, the true branch of a
  ternary, and the body of a guarded handler. No negation/disjunction
  inference is invented.

Named aliases are expanded before entering the tree, so alias spelling is
not identity. V7 serializes the resolved target and any explicit external
discriminator; it never serializes an alias name or a compiler inference
variable.

Closed structs and `Case { ... }` tagged values originate in the host `value`
grammar, not in the intrinsic expression parser. After contextual checking
they are ordinary `Expr::Struct`/`Expr::Tagged` tree nodes, so their fields use
the same evaluator, constants and narrowing operations. Likewise, implicit
construction into a structural union is stored as an explicit resolved
`Expr::As`; the expression engine executes a decision already made by the
checker. V7 gives these constructor nodes and `is`/`as` one canonical
representation.

Member and index access on `variant` refuse at expression check. A positive
`is` fact or explicit `as` must first expose a precise struct, map, array or
union branch; no runtime lookup restores the old dynamic shortcut. The
`meta` root is not an exception for payloads: its temporary expression
binding is checked afterwards against the sealed `sys`/`app`/`ext` metadata
grid, which is a distinct statically validated namespace. `decimal(...)`
produces the one exact Rastok decimal value and its transported result type is
canonical `number`.

## 6. Future `@` annotations

`@` is no longer reserved as the guest-language sigil. Its allocation is
left open for the separate schema-classification design (`@sensitive` is
the leading candidate). Tagged frontends use `IDENT!` exclusively.
