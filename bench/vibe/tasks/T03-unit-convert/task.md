# T03 — Unit conversion

Temperature readings arrive in Celsius; downstream wants Fahrenheit.

## Interface (fixed)

- Graph: `graph convert : App`
- No connector — the operator's output leaves at the boundary.
- Input: `{celsius = <int>}`.
- Output: `{fahrenheit = <int>}` where
  `fahrenheit = (celsius * 9) / 5 + 32`. The output record carries
  only that field.

Submit one `.rkg` file containing the graph only — no `lab`.
