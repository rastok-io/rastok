# Rastok core

The allocation-enabled, `no_std` semantic kernel of Rastok: typed envelopes,
pure expressions, validated plans, journal facts, routing and the deterministic
execution machine.

Time and external outcomes enter explicitly. Physical process, file and
network Drivers, the DSL frontend, and the CLI belong to the hosted `rastok`
crate. The kernel does not read environment variables or consult a wall clock.

This alpha qualifies the single-worker reference machine and in-memory
journal/recovery fold. It does not provide a durable journal backend or a
stable public library API. Rust 1.97.1 is the qualified toolchain; embedded
compile checks and the measured bare-metal spike are maintained in the full
source checkout.

Public destinations for the planned 5 November 2026 opening:
[website](https://www.rastok.com),
[documentation](https://docs.rastok.com),
[repository](https://github.com/rastok-io/rastok),
[releases](https://github.com/rastok-io/rastok/releases).
These are release destinations, not a claim that publication has occurred.

License: Apache-2.0.
