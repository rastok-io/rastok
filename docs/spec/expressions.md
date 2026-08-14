# Rastok expressions and tagged frontends

> **Status: normative M1.3 target.** The current executable module is
> still named `cel`; M1.3 migrates it to this contract and keeps named
> refusals until the migration is complete.

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
Rastok-owned extensions include exact `decimal(...)`, checked `as`,
type/case tests through `is`, structs, tagged unions and the
Arrow-anchored logical types.

Every divergence from the CEL-compatible core is named and tested. In
particular, Rastok's comments, line-bounded strings, absence rules,
narrowing and type domain are not described as upstream CEL behavior.

## 2. A tagged frontend is not an expression keyword

Any value whose bytes belong to a dedicated reader or compiler uses one
surface:

```text
tagged_value ::= IDENT "!" frontend_args? frontend_payload
```

Canonical spelling keeps the bang adjacent to the name:

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

The M1.3 implementation gate closes the exact flow-sensitive and runtime
failure rules before either form becomes executable. Until then they
remain named deferrals; `variant` does not permit unchecked structural
access.

## 6. Future `@` annotations

`@` is no longer reserved as the guest-language sigil. Its allocation is
left open for the separate schema-classification design (`@sensitive` is
the leading candidate). Tagged frontends use `IDENT!` exclusively.
