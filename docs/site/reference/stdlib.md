---
title: Standard library
description: Find the types, Connector Contracts, annotations and value frontends supplied with Rastok.
order: 6
---

The standard library supplies definitions that an application can use without
copying them into its source. A Connector Contract describes an interaction;
a Driver implements it. Selecting physical Drivers does not change the
standard types available to the compiler.

## Types

These names are available without an import or a local declaration. Their
definitions are the same logical types used by the Connector ports.

| Type | Definition and cases |
|---|---|
| `FileExit` | [File](connectors/file.md#exit-union), read and replace |
| `ProcessExit` | [Process](connectors/process.md#exit-union), job and service |
| `HttpClientExit` | [HttpClient](connectors/http-client.md#exit-union) |
| `HttpServerExit` | [HttpServer](connectors/http-server.md#exit-union), ack and respond |
| `WebSocketExit` | [WebSocketClient](connectors/websocket-client.md#exit-union) and [WebSocketServer](connectors/websocket-server.md#exit-union) |

Write `output failed: FileExit` without a `type FileExit` declaration.
These names also work in aliases, fields, collections and `is`/`as`.
For example, `type StorageResult = FileExit` declares an application alias
without creating a different type identity.

Redeclaring a standard name is `E-NAME-RESERVED`. A typed destination resolves
constructors such as `Completed {}`; without one, constructor lookup uses
only unions explicitly declared in the source. The [type reference](types.md)
covers scalar, collection, struct and union notation and schema projections.

## Connector Contracts

[Connectors and contracts](contracts.md) lists File, Process, HttpClient,
HttpServer, WebSocketClient and WebSocketServer. Each Connector page gives
its exact Contract id, modes, ports, settings, terminal outcomes and
qualification limits.

`rastok connectors` lists the Contracts and the implementations present in
the current binary; `rastok connectors --json` produces the machine-readable
catalog. Physical networking depends on the binary's feature selection,
not on an import in the program.

## Annotations

| Annotation | Meaning |
|---|---|
| `@sensitive(level = ...)` | Classifies confidentiality as internal, confidential or restricted |
| `@personal(...)` | Classifies personal data with identifier, special and criminal flags |
| `@secret` | Declares secret handling; clear secret-bearing executable schemas are refused |
| `@measure(id = ..., unit = ...)` | Associates a numeric field with a measurement id and unit |

These annotations are available without imports. Their interpretation and
current limits are described in
[Privacy, secrets and measurements](../guide/08-privacy-secrets-and-measurements.md).
A user annotation with the same field shape does not acquire the authority
of a standard annotation.

## Value frontends and expressions

The standard tagged frontends are `json!` for a JSON value, `text!` for
verbatim text and `base64!` for bytes. Their syntax and restrictions are in
[Expressions and dispatch](expressions.md#tagged-frontends). That reference
also lists the available operators and expression functions.

## Modules and imports

There is no `import`, `use` or `using` directive in this version. Standard
definitions are available implicitly. Modules, explicit imports, visibility
and namespace rules remain a later design discussion; no syntax or delivery
date is decided.

## Specified, not shipped

| Item | Specification |
|---|---|
| File `watch`, Timer, Calendar and Memory | `docs/spec/stdlib.md` §§3.4–3.7; see the [Contract limits](contracts.md#specified-not-shipped) |
| Additional tagged guest-language frontends | `docs/spec/expressions.md` §§2–3; see the [expression limits](expressions.md#specified-not-shipped) |

Sources: `docs/spec/stdlib.md` §1.2, `docs/spec/types.md`,
`docs/spec/annotations.md` and the standard Contract definitions.
