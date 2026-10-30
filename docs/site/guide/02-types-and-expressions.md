---
title: Types and expressions
description: Follow typed payloads through guards, narrowing and pure expressions without confusing text, bytes or decoded JSON.
order: 2
---

After this chapter you can read a declared payload type, follow a guarded
transition, and distinguish a business rejection from a typed evaluation
fault. You can also explain why decoding JSON is not proof that its
contents satisfy your application.

## Give the payload a shape

A port carries an envelope. Its `data` is the payload the Operator works
on; `meta` carries separate provenance and application metadata. The
payload has one resolved logical type. Leaving off an annotation asks the
compiler to infer that type before execution.

The webhook example names the shape it needs. This declaration is from
`examples/webhook-to-partner.rkg`, lines 3–7:

```rkg
type Webhook = struct {
  event_id: text
  email: text @personal(identifier = true)
  amount: int
}
```

A `struct` names fields and their types. This one requires text for the
event identifier and email, and an integer for the amount. `int` is a
signed 32-bit type, not a machine-dependent integer. The annotation on
`email` adds personal-data policy metadata; it does not change `text` into
a second payload type. Running for real returns to that policy later.

Named type declarations are structural aliases. The checked field shape
matters, not the alias spelling. A tagged union is different: its case
names, such as File's `Completed` and `NotFound`, select observable outcomes.
Use the existing declarations as the starting point, not an assumption
that every pair of JSON-looking objects is interchangeable.

## Test before projecting

With `codec = json`, the webhook listener supplies a `variant`: a decoded
value whose application shape is not yet known. An Operator cannot simply
read arbitrary members of it. It first tests and narrows the value.

The validation Operator, from the same file, lines 42–53, separates shape
validation from the business rule:

```rkg
operator validate {
  input in: variant
  output out: Webhook
  output failed: Failure
  on in match {
    data is Webhook => match {
      data.amount > 0 && size(data.event_id) > 0 => out = data as Webhook
      _ => failed = Rejected { reason = "positive amount and event id required" }
    }
    _ => failed = Rejected { reason = "expected event_id, email and integer amount" }
  }
}
```

`is` asks whether the value fits a type or union case. On the successful
branch, member access such as `data.amount` is checked with the narrower
shape. `as` requests a conversion that succeeds only for a compatible value.
The final `_` branch makes the alternative visible.

The two rejections here are expected application outcomes. They become
ordinary emissions on `failed`, with a typed `Rejected` payload. A decoded
object with a negative amount is not the same problem as malformed JSON
that the boundary could not decode. The next chapter follows that distinction
through the Connector.

## Keep bytes and text distinct

File reads produce `bytes`. Its inspection Operator, from
`examples/file-pipeline.rkg`, lines 28–37, accepts only nonempty bytes:

```rkg
operator inspect {
  input in: bytes
  output accepted: bytes
  output bytes_read: int
  output empty: bool
  on in match {
    size(data) > 0 => { bytes_read = size(data); accepted = data }
    _ => empty = true
  }
}
```

The successful branch emits two values, in source order: a size and the
unchanged bytes. The alternative emits a boolean and never sends bytes to
the writer. `size` on bytes counts bytes. The example's UTF-8 scenario
therefore counts three bytes for `é` followed by a newline, not two characters.

The shipped examples also use checked literals: `duration("2s")` for a
deadline budget, `base64!` for byte fixtures and `decimal("0.02")` for an
exact quantity. A codec changes external representation; changing the
logical payload type requires an explicit, supported conversion.

## Understand the expression boundary

Expressions are pure, deterministic and evaluated under bounds. They do
not read a file, contact a server or obtain randomness. Guards such as
Echo's `on in where data != ""` decide whether a handler runs; `match`
selects an explicit branch inside the transition.

Bounded evaluation is not a guarantee that every value succeeds. An invalid
cast can produce a typed `NarrowingFailure`; such an evaluation fault does
not invent an `err` envelope or commit a partial transaction. Test expected
alternatives with `is` and emit the domain result you intend. The Lab runs
these same expressions rather than mocking the Operators. Committed
transactions preserve their state changes and ordered emissions in the journal.

The authorities are `docs/spec/types.md`, `docs/spec/expressions.md` and
`docs/spec/stdlib.md`. This chapter covers the executable examples, not
every proposed frontend or codec. See
[what is available today](../index.md#what-is-available-today) for the alpha limits.

## Summary

- A resolved type is different from an external encoding.
- `is` tests a shape; `as` performs a checked conversion.
- Expected rejection is typed application data; evaluation faults are not
  implicit output ports.
- Lab and App execute the same Operator expressions.

## Next

Read [Connectors and outcomes](03-connectors-and-outcomes.md), then try
[File pipeline](../examples/02-file-pipeline.md). Its empty and UTF-8
scenarios make the distinction between payload type and representation concrete.
