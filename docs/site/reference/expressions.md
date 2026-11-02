---
title: Expressions and dispatch
description: Operator precedence, built-in functions, narrowing, transaction dispatch and envelope writes accepted by the compiler.
order: 2
---

An expression reads its declared inputs and returns a value. Dispatch selects
the statements of an Operator transaction. Neither performs external I/O;
Connectors own that boundary.

## Operators

Rows are ordered from highest to lowest precedence. Parentheses select an
explicit grouping. Operators at the same arithmetic or comparison level
associate left to right; the conditional's false branch associates right.

| Level | Syntax | Rule |
|---|---|---|
| Access and narrowing | `data.name`, `items[index]`, `value as T` | Member paths require a known structure; indexes require a checked collection |
| Unary | `!value`, `-value` | Boolean negation; numeric or duration negation |
| Multiplication | `*`, `/`, `%` | Checked numeric operations |
| Addition | `+`, `-` | Numeric operations, text concatenation, supported timestamp/duration arithmetic |
| Relation and type test | `==`, `!=`, `<`, `<=`, `>`, `>=`, `is T` | Equality, ordering and type/case test share this parser level |
| Conjunction | `a && b` | Short-circuits when `a` is false |
| Disjunction | `a \|\| b` | Short-circuits when `a` is true |
| Conditional | `test ? yes : no` | Evaluates only the selected branch |

Integer overflow, invalid indexing, division by zero and a failed explicit
narrowing are evaluation faults. A fault commits no input consumption,
state change or emission from that transaction. Expected domain rejection
must be represented by an authored branch and output.

Expressions are limited to 128 nesting levels. The serialized plan has a
separate 256-level reader bound; nested values can reach it even when the
expression itself fits.

## Functions and contextual references

| Signature | Result | Availability and constraints |
|---|---|---|
| `size(text \| bytes \| array<T> \| map<K,V> \| variant)` | `int` | Unicode character count for text, byte count for bytes, entry count otherwise |
| `has(object.member)` | `bool` | Final-member presence on a closed struct or text-keyed map; present null is true; only dotted selection |
| `now()` | `timestamp` | One transaction-fixed civil instant, derived from the run origin and logical time; handler context only |
| `duration(text)` | `duration` | Signed units `h`, `m`, `s`, `ms`, `us`, `ns`; combined or fractional quantities; no `min`, `d` or `w` |
| `timestamp(text)` | `timestamp` | RFC 3339 UTC spelling ending in `Z`; other offsets refuse in this constructor |
| `decimal(text)` | `number` | Exact decimal at its written scale, with no binary-float conversion |
| `media_type(text)` | `media_type` | Concrete media type; malformed syntax, duplicates and wildcard ranges refuse |
| `asset(path)` or `asset(path, content_type)` | `bytes` | Build-time file contents; the second argument is for a whole-payload position and sets its content type |
| `env(name)` | Boot reference | Admitted at the HttpClient URL setting, resolved by the deployment profile |
| `secret(name)` | Boot reference | Admitted at HttpClient `auth_bearer`; no raw secret value in graph data |

`env` and `secret` are references, not evaluator functions. Other sites refuse.
`has` does not establish the value's type: use `is` before reading it as a
number or another narrower type. The receiver still has to be evaluable.
`now` has no arguments and is unavailable in keys, settings or boot values.

## Narrowing and dispatch

| Form | Selection | Requirement |
|---|---|---|
| `value is T` | Tests a type or tagged case | A positive fact narrows the right side of `&&`, a conditional's true branch or a guarded handler |
| `value as T` | Checked conversion | Exactly one structural-union branch must match; decimals rescale only exactly |
| `match value { Case => ... }` | Closed match | Tagged union, enum or bool; exhaustive, disjoint case sets |
| `match { predicate => ...; _ => ... }` | Ordered predicates | First true predicate wins; fallback is mandatory |
| `on in where predicate { ... }` | Single-input filter | False consumes the input and emits nothing |
| `on (a, b) where predicate ...` | Join candidate selection | False consumes nothing; bounded waiting is covered in [Instances, state and time](../guide/06-instances-state-and-time.md) |

A match is the final control form of its block or arm. Only pure `val`
declarations precede it. Dispatch permits at most 32 nested decisions,
including a single-port `where`. A `variant` must be narrowed before member
or index access. A negative test or disjunction supplies no inferred positive
type fact.

Route selection is separate: `source -> match { Case -> target }` chooses
among declared destinations using a tag, enum or bool. Its arms contain
destinations, not expressions. A simple route may coexist with a match,
but duplicate destinations for one case refuse.

## Fan-out and emission

The shipped fan-out is bounded by the graph and the selected match arm.
There is no collection-iteration statement in that arm.

| Form | Result |
|---|---|
| `source -> { left; right }` | One emitted envelope delivered along two declared edges |
| `(left, right) = data` | Evaluate once, then emit a distinct ordered envelope on each output |
| `(left, right) = (first, second)` | One value per named output, committed together |
| `out = .` | Re-emit the exact single input envelope |
| `out = nothing` or `nothing` | No emission |
| `out = { count = 1 }` | New structured data, inherited metadata |
| `out = { data = replacement, meta.app.label = "checked" }` | New payload plus explicit metadata writes |

Emission order within each port and transaction contributes to identity.
An edge fan-out does not duplicate the emission itself. Iteration over state
uses [Loop](../guide/07-loops-and-retries.md), whose manager admits subsequent
applications outside the spatial DAG.

## Envelope fields and bindings

| Surface | Read | Write |
|---|---|---|
| `data` | Payload of the current single input | Construct a new output payload or checked patch |
| `meta.sys.id`, `parent_id`, `origin_id`, `graph_version` | Runtime identity and version fields | Runtime only, including in the Lab |
| `meta.app.*` | Fields inferred from graph-authored writes | Declared-by-use application metadata; conflicting types refuse |
| `meta.ext.*` | Contract-declared boundary context | Declared fields only; the payload/content-type coupling remains enforced |
| `instance.key` | Local selected key of the declaring Node | Never assigned |
| `Loop.state` | Envelope admitted to the current Loop application | Replaced through a manager command, not mutated directly |
| `<port>.data`, `<port>.meta` | Named inputs in a join | Emissions select a named source explicitly |

In this build a metadata leaf is a scalar (`int`, not `uint`; no duration
or timestamp) or an array of scalars.
The `app` and `ext` namespaces have prefix-free field names. Heavy or nested
payload structures belong in `data`. A Lab cannot introduce application
metadata fields absent from the production graph.

`val name = value` is immutable and lexical. Unit/App constants can be
referenced by Lab declarations; instance-level values can use that owner's
key and enclosing immutable values. `var name: Type = value` is mutable
Operator state for one instance generation. Its writes become visible with
the transaction commit; children do not inherit it.

## Tagged frontends

| Form | Result | Boundary |
|---|---|---|
| `json! <JSON value>` | `variant` | One complete JSON value; duplicate keys refuse and numbers stay exact |
| `text! { ... }` | `text` | Verbatim text, closing-delimiter margin, no interpolation |
| `base64! { ... }` | `bytes` | Padded standard base64; whitespace ignored |

The name and `!` are adjacent. A frontend fills a complete value position,
not an expression atom. Bind its result before combining it with arithmetic.
`base64!("image/png") { ... }` may also declare content type in a whole-payload
position.

## Specified, not shipped

| Item | Specification |
|---|---|
| Per-item fan-out (`foreach` is reserved); `for item in` is not accepted | `docs/spec/grammar.md`, reserved-keyword section |
| Expression macros `exists`, `exists_one`, `all`, `map`, `filter`, optional-access syntax and the `in` operator | `docs/spec/grammar.md`, expression production; current subset in `docs/spec/expressions.md` §1.1 |
| jq, SQL and additional registered guest-language implementations | `docs/spec/expressions.md` §§2–3 |
| `map<text, array<scalar>>` metadata leaves | `docs/spec/envelope.md` §0 |
| Unit/App boot values using `env` or `secret` outside the declared settings | `docs/spec/expressions.md` §1 |

Sources: `docs/spec/expressions.md`, `docs/spec/grammar.md`,
`docs/spec/envelope.md` and `docs/spec/execution-model.md` §1.1.
