# Contributing a Connector Driver

> Current contributor path, updated 2026-10-22.
> Current format and exact standard Contract ids: [identity.md](../spec/identity.md)
> and [stdlib.md](../spec/stdlib.md).

A Connector is a Node in the program. Its Contract specifies the observable
behavior: ports, data types, attributes, evaluation phases, exchange protocol
and terminal obligations. A Driver implements that Contract using a process,
filesystem or network service. Deployment selects an implementation; it does
not change the Contract or the program's semantic identity.

Start with the catalogue. Then choose an existing Contract or define a new
one, implement only the required interfaces, register the implementation,
and qualify its real and simulated behavior.

## 1. Find the Contract

```sh
rastok connectors
rastok connectors --json
```

Both views come from the same definitions. The machine view includes the
complete resolved type tree, not only a display name such as `struct`.
When a type depends on a required attribute such as `codec`, the catalogue
lists the alternatives and the required choice; it does not invent a default.

The catalogue must distinguish four facts:

- The Contract is known to the compiler.
- Its selected mode is executable, rather than a named deferral.
- The Lab supports its exchange and fixtures.
- A physical implementation is installed and can satisfy its deployment
  requirements.

One fact does not imply the others. In particular, simulation does not need
the physical implementation. File's `watch` mode is declared but not an
executable mode; an available File Driver does not enable it.

A listed implementation matches the Contract, mode, direction and exchange.
Preparation still validates the deployment's resolved settings and host
capabilities; for example, the Contract's protocol-version choices do not mean
every version is linked into this particular binary.

A Lab binding is also narrower than a complete scripted exchange. An
HttpServer `respond` Node can remain silent beside the flow exercised by a
scenario, as in the chat example. Authored source injection still supports
`ack` only; the binding does not supply a graph-response simulation model.

Portable definitions live in `crates/rastok-core/src/connector/stdlib/`, one file
per standard Contract. The compiler and independent package validator read
those same definitions. Human explanations and the compiler catalogue are
hosted; they do not enter the embedded package.

Use the same Contract for another implementation only when its actual
guarantees match. A provider-specific operation or weaker atomicity guarantee
cannot be hidden behind an implementation choice.

## 2. Read the interfaces for the selected exchange

The shared interfaces are in `connector::driver`; the data they exchange is
in `connector::exchange`. Hosted implementations and their conformance harness
are under `rastok::connector::drivers`.

| Interface | Responsibility |
|---|---|
| `Driver` | Expose `InboundRole` through `inbound_role()` and `OutboundRole` through `outbound_role()`, over the same implementation object. |
| `InboundRole` | Retain and propose an external arrival; receive the admission result. |
| `InboundCall` | Return graph responses or scheduler timeouts to an inbound caller and report their completion. |
| `OutboundRole` | Expose the implemented outbound Call or Session interface. |
| `OutboundCall` | Prepare, start and cancel Calls; poll their correlated responses. |
| `Session` | Open, write, close and observe a resident exchange. |

`OutboundDrivers` is different: it routes execution to all selected Driver
objects and checks exchange ownership. An individual Driver does not implement
that routing table. `RuntimeDrivers` supplies the hosted implementation.

A bidirectional Driver returns views of one object, not two independent
objects that happen to have the same Contract id. Role-view availability must
not change after preparation.

Execution retains control over identities, admission, ordering, timeouts and
journal commits. A Driver reports observations, not journal records.

## 3. Worked example: File

Read these files in order:

1. `crates/rastok-core/src/connector/stdlib/file.rs`: the Contract declaration,
   portable exit cases and independent validation.
2. `crates/rastok/src/connector/drivers/file/mod.rs`: request preparation, factory,
   Driver interfaces and filesystem implementation.
3. `crates/rastok/tests/conformance_file.rs`: the shared conformance fixture.
4. `examples/file-pipeline.rkg`: a complete executable Lab example.

File is an outbound Call. Its real Driver implements `Driver` and
`OutboundCall`; it has no inbound role view and no resident Session protocol.
The common outbound view exposes that Call interface.
The core supplies this per-object `OutboundRole` view for every `OutboundCall`
implementation; this is not the execution-wide `OutboundDrivers` routing table.

```rkg
graph read_one : App {
  input in: text
  output out: bytes
  connector file : File {
    mode = read
    path = "input.txt"
    max_read = "32B"
  }
  in -> file
  file.out -> out
  file.exit -> nothing
}
```

The logical `path` belongs to the program. The physical root is supplied
when constructing `FileFactory`. Root selection does not reinterpret the
logical operation or grant directory traversal.

### Preparation is pure

`FileRequestSettings` validates the selected mode and resolved settings,
checks the logical path, and produces a `PreparedRequest`. Its sealed bytes
describe the complete operation: path and read limit, or path and replacement
bytes. It holds no physical root and cannot observe the filesystem.

The real Driver and its Lab substitute share these request rules through
the private `CallProtocol` interface. They also share the projection of
scheduler outcomes to portable terminal cases. The Lab substitutes the
external result, not request framing or timeout semantics.

### Effects start after the journal open

Execution resolves invocation settings and commits the Call opening before
calling `OutboundCall::start`. File then starts bounded filesystem work.
No file is opened or replaced during request preparation.

An implementation that returns `Pending` must have performed no effect for
that attempted start. `Started` transfers responsibility for that invocation
and its eventual observation. Finite internal capacities remain visible as
backpressure, not dropped messages or unbounded work queues.

The physical worker reports a `CallResponse` with the original `CallId`.
`Completion` contains ordered output drafts and exactly one final `exit`
draft. Execution validates those drafts, assigns envelope identities, and
commits the Connector transaction. The Driver does not manufacture envelopes
or routing facts.

### Cancellation is scoped to the exact exchange

File indexes active work by `CallId`. Cancellation sets the flag of that
operation only. A completed or cancelled operation must not affect another
Call or a successor instance.

The scheduler decides timeouts. A late physical answer cannot add a second
causal completion after closure. `OutcomeUnknown` is not a fabricated normal
success when bounded shutdown cannot establish the external outcome.

## 4. Register and select an implementation

`DriverRegistry` describes available implementations. `RuntimeDrivers::prepare`
checks the plan's requirements and establishes Node and exchange ownership
before execution. Registration order is not a selection policy.

The selected object must expose the views it advertised. A factory may reserve
bounded passive resources, such as a listener, while preparing the complete
binding. It must not acknowledge input or perform an outbound effect before
admission. Failed preparation must release partial reservations.

The implementation name belongs to deployment configuration. It is not a DSL
attribute, a Contract alias, a new journal identity or an automatic provider
fallback. Multiple compatible implementations require an explicit selection.

For example, a deployment profile can select the implementation independently
for two Nodes using the same Contract:

```json
{
  "drivers": [
    { "connector": "incoming_files", "implementation": "local-file" },
    { "connector": "outgoing_files", "implementation": "audited-file" }
  ]
}
```

These example implementation names require their factories to be registered.
The standard binary does not acquire another implementation by reading its
name. With exactly one compatible registered implementation, selection may
be omitted. Missing, incompatible and ambiguous selections refuse before
any factory is invoked. `simulate` continues to substitute Lab behavior;
this deployment field cannot make a simulation call a physical provider.

After Execution has recovered a journal prefix, `restore_ownership` rebuilds
routing from that prefix and the same deployment selection, without calling
any Driver method. Implementation names are not in the journal, so the caller
must supply that selection unchanged. This restores routing, not sockets or
processes. Unsupported recovery situations keep their named refusal.

## 5. Supply Lab and conformance fixtures

A fixture is prepared test material: a graph, inputs, expected results and
the reasons some terminal cases cannot be forced by that fixture.

For File, `DriverTestFixture` covers successful reads, invalid paths, non-file
targets, missing files and read limits. Its interleaving probe has two Calls
with different paths and expected bytes, so crossed responses fail visibly.
Host-dependent cases remain explicitly listed as untestable by that portable
fixture; an absent test is not reported as a qualified guarantee.

The shared harness checks the Contract shape, effective port and terminal
types, silence before start, correlation, commit-before-effect, closure and
late-result handling. Driver-specific tests still cover the filesystem or
wire protocol. A Lab scenario does not qualify TLS, HTTP framing, permissions
or physical cancellation.

Hosted network implementations keep async work and library types under
`connector/drivers/network`. Work, control and result queues have finite slot
and byte bounds; a full queue is backpressure, not a transport-failure case.
Cargo features select the physical stacks. Independent ngtcp2 server evidence
and the Autobahn suite remain open in the
[release qualification](release-qualification.md); the catalogue does not
turn related integration tests into those missing proofs.

Provider quantities are ordinary typed payload fields, with explicit metadata
such as `@measure(id = amount, unit = "USD")`. Preserve each observation and
leave missing quantities unknown; Driver code does not infer a price or a
charge. Credentials use the Contract's permitted `secret()` setting, never a
payload field or diagnostic. See the [annotation contract](../spec/annotations.md)
and [data-policy rules](../spec/observability.md).

Session implementations additionally distinguish cooperative closure,
forced abort and resource release. Each operation is scoped to one exact
`SessionId`; none may cancel a successor or silently discard an unresolved
terminal obligation.

## 6. Adding a new Contract still requires core work

Another Driver for an existing Contract does not require another semantic
schema. A new standard Contract does: add its portable declaration and
protocol rules under core `connector/stdlib`, connect independent validation,
document its normative semantics and identity consequences, and add the
hosted catalogue entry, Lab support and conformance fixtures.

There is no plugin loader or native ABI in this contribution path. A future
loader must provide the same checked capabilities; it cannot bypass them.

## Review checklist

- Contract id, modes, ports, defaults and evaluation phases match the
  specification and independent validator.
- Preparation is pure; effects and acknowledgements follow the relevant
  journal commit.
- Every reported result carries the exact owning Call or Session identity.
- Backpressure, cancellation, close-once and late results have focused tests.
- Real and Lab implementations share protocol rules, not copied variants.
- Every terminal case is tested or has a named qualification limit.
- The registry refuses missing, incompatible and ambiguous implementations
  before effects; recovery never recalls an external operation.
- No dependency leaks hosted I/O into `rastok-core`.
- All four repository checks, the required `no_std` builds and embedded
  gates pass; any witness movement has a reviewed semantic reason.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
```
