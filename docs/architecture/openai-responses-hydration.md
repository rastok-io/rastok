# OpenAI Responses — schema contact before typed hydration

17 September 2026, qualification updated 18 September. **Limited codec profile
and HTTP adapter implemented in v19.** The [common C4 ledger](retry-c4-qualification.md)
records its six first-time goldens and local checks. Live provider qualification
is not claimed.
Yoann kept OpenAI Responses as the target of the HTTP gateway. A simpler,
fictional model API must not replace it. This note records the mismatch found
while preparing the [C4 example migration](retry-c4-examples.md); it does not
relax ordinary data conformance or allocate a format field. Following the
contact below, Yoann approved defining an explicit JSON profile for absent
and additional members, keeping wrong types fail-closed. Section 4 defines
that bounded profile; the implementation evidence follows the original contact.

## Target and source of truth

The [adapter](../../examples/agent-http.rkg) makes one ordinary,
non-streaming `POST /v1/responses` per job, with a session/job coordinate in
application metadata and a secret-bound bearer. The existing model selection
is unchanged. No paid request, model availability check or live wire capture
was performed in this pass. Scripted examples are not provider observations.

The official [Responses reference](https://developers.openai.com/api/reference/typescript/resources/responses/methods/create)
and its [expanded Response schema](https://developers.openai.com/api/reference/typescript/__sdk_schema?declaration=%28resource%29+responses+%3E+%28model%29+response+%3E+%28schema%29&selected=%28resource%29+responses+%3E+%28method%29+create)
were read on 17 September. They describe optional properties, including
`usage`, and multiple output-item forms. An example response is not an exact
field-presence contract for every successful response.

Adding JSON response properties is explicitly compatible under OpenAI's
[API compatibility policy](https://developers.openai.com/api/reference/overview#backwards-compatibility).
Thus a closed snapshot of today's response fields can reject a valid future
response without OpenAI breaking that policy.

## Fields of interest, not a replacement response schema

The [documented usage object](https://developers.openai.com/api/reference/typescript/__sdk_schema?declaration=%28resource%29+responses+%3E+%28model%29+response+%3E+%28schema%29+%3E+%28property%29+usage&selected=%28resource%29+responses+%3E+%28method%29+create)
contains these counts:

| JSON path below `usage` | Retained meaning |
|---|---|
| `input_tokens` | Reported input count |
| `input_tokens_details.cached_tokens` | Reported cache-read detail |
| `input_tokens_details.cache_write_tokens` | Reported cache-write detail |
| `output_tokens` | Reported output count |
| `output_tokens_details.reasoning_tokens` | Reported reasoning detail |
| `total_tokens` | Provider's reported total, not a recomputation |

This usage object supplies no currency amount. M1 must not turn these counts
into a bill. Each declared measure will remain an original typed value with
its path and causal coordinates, with simple author-chosen ids; neither
`subset_of` arithmetic nor a report is needed. A missing usage object yields
no quantity evidence. An explicitly reported zero remains zero.

The previous draft's fixtures omitted parts of this object and used a
downstream Operator. That draft has been replaced, not relabelled as native
measurement evidence. The executable adapter retains selected quantities on
`model.out`; its explanation Operator emits only missing-usage notices.

## Contact with Rastok's executable codec

[Types §4.1](../spec/types.md#41-explicit-json-response-types-in-m1) and
[`json::hydrate`](../../crates/rastok-core/src/json/hydration.rs) agree:

| Situation | Current M1 rule | Consequence for Responses |
|---|---|---|
| Missing declared member | `MissingField`, even for a nullable member | A declaration `usage: Usage?` still requires the `usage` key; it does not implement optional presence |
| Extra object member | `UnexpectedField` | A short struct with just `id`, `output` and `usage` is not an open view of the HTTP body |
| Explicit null | Accepted only by a nullable target | Must not be confused with absent evidence or a reported zero |
| Heterogeneous content | `variant` can retain it; general tagged-union decoding is not admitted | A fixed single-message struct cannot claim to cover all output-item forms |
| Incompatible body, including HTTP 500 | `DecodeFailed`, no partial `out` | Typing only the success object can hide the HTTP `ServerError` behind a decoding failure; error-body handling must be specified too |

`has(...)` on an untyped value remains useful in an Operator but cannot run
that Operator on an intermediate Retry attempt: held emissions are not
delivered downstream. Switching the whole port back to `variant` would
therefore abandon the requested native typed measurements. Declaring every
field from one provider example would only qualify that exact fixture, not
resolve the optional/evolving-schema mismatch.

## 4. Limited projected profile

The target spelling is `json_profile = projected`, a plan-time HttpClient
attribute admitted only with `codec = json` and an explicitly typed struct
output. Omission keeps strict hydration exactly as specified today. There is
no implicit profile choice based on the URL, provider, response or annotations.
No default attribute is added to programs which do not opt in.

At each expected struct in the declared response schema:

| JSON member | Projected hydration |
|---|---|
| Declared and present | Recursively validate and hydrate under the declared type |
| Declared, absent, type admitting null (for example `T?`) | Supply typed null, never a numeric default |
| Declared, absent, non-nullable | `DecodeFailed`, no partial `out` |
| Not declared | Omit from the projected struct |
| Present but wrong type or out of range | `DecodeFailed`, even if the field is nullable |

This is a **lossy, explicit projection at the codec boundary**, not an open
Rastok struct. The resulting ordinary struct still contains exactly its
declared fields. Absent and explicit null intentionally become the same
typed value in this profile; `has` on the hydrated struct cannot recover that
wire distinction. Additional members are not retained in the projected DATA
or its diagnostic recording. Authors who need an untyped subtree can declare
that member as `variant` or a typed map, both already supported. No hidden raw
response copy is added to the journal or debugger.

The existing JSON reader processes the complete body first. Syntax, duplicate
member, numeric representation and byte limits are not bypassed for members
which projection later omits. The existing depth and type limits stay in force.
Maps retain their keys under their declared value type; `variant` retains its
subtree. Arrays recurse into their declared element type. No renaming, computed
field, per-field default, general union discriminator or new optional-field
type is introduced.

Hydration and policy checks still precede completion retention and Retry's
Held/Published decision. Only the original successfully hydrated emission
supplies samples. Null supplies no measure; numeric zero supplies zero. Neither
held-attempt extraction by a downstream Operator nor an aggregate is added.

### Example binding

```rkg
type JsonObject = map<text, variant>
type ResponseUsage = struct {
  input_tokens: uint @measure(id = input_tokens, unit = "token")
  output_tokens: uint @measure(id = output_tokens, unit = "token")
  total_tokens: uint @measure(id = total_tokens, unit = "token")
  input_tokens_details: JsonObject?
  output_tokens_details: JsonObject?
}
type ModelResponse = struct {
  id: text
  object: enum { response }
  output: array<variant>
  status: text?
  usage: ResponseUsage?
  error: JsonObject?
}

connector model : HttpClient {
  url = env("MODEL_URL")
  auth_bearer = secret("showcase/model/token")
  method = "POST"
  codec = json
  json_profile = projected
  output out: ModelResponse
  timeout = duration("10s")
  behavior = probabilistic
}
```

This selects three counts without inventing a currency amount or summing the
reported total again. The two detail subtrees are retained without a new
interpretation; authors can refine them explicitly later. The request's model,
prompt and `stream = false` / `store = false` remain ordinary request DATA in
the existing adapter, not Connector settings hidden in this excerpt.

The response struct requires `id`, `object` and `output`. A missing `usage`
becomes null; a present incomplete `usage` lacking a required count refuses.
An HTTP error body which is not this Response shape remains `DecodeFailed`:
projection does not fabricate the missing required success fields to force a
`ServerError`. A body that does fit the schema with status 500–599 keeps the
ordinary `ServerError`. This preserves the existing decoder/terminal precedence
and does not claim complete coverage of all OpenAI error-body forms.
There is no automatic retry of inference or invented idempotence guarantee.

### Identity and implementation boundary

The attribute belongs in the existing canonical Connector attribute table;
the output type and annotations stay in their existing tables. The independent
validator must enforce the HttpClient/JSON/typed-output restriction and reject
unknown profile values. The physical binder must take the profile from that
validated plan, never from the wire or a deployment override. Lab mocks remain
typed values, not raw-wire decoding tests.

The candidate `http-client@3` is amended before its single blessing; no new
package version, journal fact or recording tuple is required by this design.
Opting in changes the program identity. Omission leaves previous strict plans,
request descriptors and their identities unchanged. Verify this, along with
recording/replay and the tight embedded footprint, before claiming delivery.
No general serializer, OpenAI-specific core, field-mapping language or report
is authorized. The migrated HTTP adapter is linked from the gateway chapter.
Its scenario goldens remain part of the common reviewed blessing, not a separate
format move.

## Evidence required after approval

Offline physical-codec tests must cover documented normal shapes, optional
usage absent, explicit null under the selected policy, reported zero, every
selected numeric path, extra properties, wrong types, and error bodies.
Lab tests then cover session/job attribution and final/held original samples
without an extraction Operator or report. Replaying each retained prefix must
yield the same facts; secret canaries must remain absent from journal and UI.
A Lab mock alone is not a wire-decoder test. Live qualification remains a
separate explicitly authorized action.

Before implementation the selected response type checked under the strict
codec, while `json_profile` refused with `E-CONTRACT-ATTR`. The new source
regression reproduced that refusal before the profile was opened. In particular,
nullable opaque objects use `JsonObject?`: `variant?` is not a legal shortcut
because `variant` already includes null and those union branches overlap.

## Implementation and qualification — 17 September

The shared hydrator has explicit `Strict` and `Projected` policies. The existing
`hydrate` entry point remains strict. HttpClient obtains its policy only from
the validated plan; neither the request descriptor, deployment overrides nor
the response can select it. The independent validator checks the attribute's
value and JSON/struct restriction, not a compiler claim. Omission produces the
same plan fields and prepared request; opt-in is semantic and survives decode.

The public [HTTP example](../../examples/agent-http.rkg) has six deterministic
scenarios: three jobs on Alice/Bob instances, absent/null usage, observed zero,
schema refusal, timeout, and decoded versus undecodable HTTP failures. It keeps
the requested model and Responses endpoint, without declaring inference
idempotent or retrying it. Its `object` member is `text`, not a claimed literal
discriminator: the wire codec supports enums, but M1's Lab expression language
does not construct enum literals. The schema above independently exercises
wire enum checking. This is a selected structural view, not the whole OpenAI
schema or a promise to accept every provider error-body shape.

Tests cover recursive optional structs, maps/arrays, enum decoding, exact counts,
zero, extra members, wrong types, missing required members, malformed JSON,
duplicate keys and numeric overflow even in omitted members. Complete parse
and hydration precede the completion; invalid bodies retain only `DecodeFailed`.
Source tests cover profile identity, package round trips and forged settings.
The adapter checks original per-job samples, no manufactured currency amounts,
determinism and bearer canaries at each debugger transition.

A real local HTTP server returns a typed 503 response with usage twice. An inert
GET, explicitly idempotent, proves two original measurement samples, one Held
and one Published batch, only one downstream response and every-prefix recovery.
That test does not declare OpenAI inference safe to repeat.

Diagnostic recording is tested through every prefix of a non-boot-dependent
twin. The actual secret-bound adapter still refuses diagnostic persistence
under the existing M1 contract: this work introduces no credential-bearing
recording header. Real OpenAI calls and full visual browser approval remain
separate actions. Detailed final check results and footprint are recorded below.

The unchanged three-attempt Retry/full-recovery Cortex-M33 package was measured
before this change at **1,048,368 B** of code and **130,560 B** peak heap. The
final profile and attribute-mirror implementation measures `.text` **964,764 B**
plus `.rodata` **83,756 B**, or **1,048,520 B**: **56 B** remain below 1 MiB.
Peak heap stays **130,560 B**, **512 B** below 128 KiB. QEMU retains the same
package hash, 17 records and `SPIKE_OK`. The repeated validation was folded
into the existing port walk before accepting this measurement; no threshold,
workload package or golden was changed. This does not qualify maximal metadata
tables or linked embedded inspection. Any further core growth must first be
measured and reduced within the existing gate, or receive an explicit bound
decision. Logs are under `/tmp/rastok-json-profile.ygk4aj/`.
