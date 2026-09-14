# Resident Process — implementation and qualification

14 September 2026. D-2026-09-14-1 implements the approved resident transport,
following the [contract review](process-contract-review.md). The visible
invariant is one child per exact Node instance, reusable across messages,
with journal-before-write and exactly one observed or unknown resource exit.

## Contract and identity

| Profile | Exact Contract | Exchange / roles | Publication |
|---|---|---|---|
| `job`, default | `rastok/process@2` | Call / outbound | Bounded captured frames and one exit, atomic |
| `service` | `rastok/process@3` | Session / inbound and outbound | Complete frames while alive, one resource exit |

The source catalogue's resolved profile can select an exact protocol id,
roles and exchange. Colliding profile/catalogue identities refuse. All three
fields already travel in v18; core validation accepts only the exact sheet
for the selected id. No new package version, journal key, tag or dependency
is needed. The existing job's semantic hash, requests and causal witnesses
must not change. The new dialogue owns new goldens, not replacement old ones.

The opening's prepared bytes seal command, arguments, cwd, explicit environment,
codecs, frame cap and first framed input. The real Driver and the Lab use the
same pure preparation. Physical spawn occurs only after the Session opening
intent commits. Later writes seal the framed bytes under their own retained
write references. Neither PIDs nor OS scheduling become resource identity.
Declared environment values are ordinary data, not a new credential door.

## Resource ownership and termination

The exact Connector instance owns at most one live child. A foreign
`meta.ext.session_id`, such as the enclosing WebSocket connection's reference,
does not select the Process resource. Alice and Bob therefore share source
code, not a process. A terminal ends the child incarnation; a new input can
open another Session in the still-open instance. There is no automatic
restart, resend, hidden request table or child-memory restoration.

Application `quit` is ordinary stdin data. A local write settlement cannot
prove that it was understood or that the process exited. Explicit
`child.instance.stop` separately requests Closing for that exact generation.
During draining, already admitted work may settle. At the retained drain
deadline the Unix worker requests SIGTERM; at settlement it requests SIGKILL.
Without an observed terminal the core retains `OutcomeUnknown`, never a
successful kill inferred from requesting a signal. Resource release occurs
after the terminal transaction commits. Late events cannot create another
exit or migrate to a successor.

Physical workers own the process group, stdin writer and independent stream
readers. Killing the resource also cleans up group members holding its pipes.
Session callbacks never block on pipe I/O, signal delivery or child wait.
Released workers remain owned by the hosted Driver until cleanup finishes;
they still count against its resource bound. Host teardown joins them outside
the VM transaction path so process exit cannot strand a child after an
asynchronous cleanup request. This is not a new journal fact. The physical
factory refuses non-Unix hosts before effects, while the Lab remains portable.

## Framing and bounds

Service stdin/stdout use `lines` or `jsonl`; stderr uses `lines`. Each write
contains one frame. A trailing newline is framing, not payload; embedded
newlines refuse. Empty lines are valid text but invalid JSON. A nonempty
unterminated final frame is accepted at EOF; malformed UTF-8/JSON is a typed
failure. Process emits `variant` for JSONL, and Operators own domain decoding.

`max_frame`, default `1MiB`, bounds payload bytes excluding the newline on each
channel. Positive size and host representability are checked before binding;
allocation failure and excess intake retain typed overflow. No aggregate
lifetime-output counter eventually kills a long-running healthy conversation.
The hosted Driver bounds resources at 64, pending writes at eight per resource,
and internal reader/writer/publication queues at eight slots each. A pending
frame blocks its resource's terminal publication until the core decides it.
A reserved atomic control slot bypasses full queues for cleanup. Operating
system pipes and reader buffers are physical pacing, not semantic state.

Service `bytes`, job `timeout`/`max_output`/`on_timeout`, `correlate`, and
`restart` refuse. Resource lifetime and Closing remain ordinary Node policy;
a business reply timeout belongs to an application Operator, not Process.

## Lab and recovery

The Lab binds `process@3` without a physical Process Driver. Each input executes
the existing checked `match` Dispatch: zero or more frames, optionally one
final exit. It keeps resource identity across inputs; the first request is
evaluated once. A terminating mock resource admits no later writes while its
preceding frames await admission. Mock output storage is bounded by the
finite checked Dispatch, 64 resources and an eight-proposal intake watermark;
it is not a claim that the Lab allocates exactly the same OS buffers.

Session recovery reconstructs retained opens, messages and writes without
re-offering old effects. A lost physical child cannot be recovered by spawning
another and pretending its private memory survived. The qualified VM rebuild
with a still-live Driver is distinct from durable deployment recovery, which
remains M2. Diagnostic recording/replay is likewise restitution, not resource
restoration.

Named Lab limits: no unsolicited or delayed service-world action syntax, no
authored system-close acknowledgement, and no `now()` in Session mock bodies
without an exact retained firing time. A mock can explicitly answer an input
with `Exited`; silence during system close settles unknown. Physical
unsolicited frames are supported. No timestamp is invented to enlarge the Lab.

## Evidence

- `tests/process_service.rs`: repeated turns on one Session; two keyed
  resources; fresh input after quit; codec/profile refusals; independent
  hostile-plan validation; catalogue alias refusal; mock terminal ordering.
- Its marked Unix integration tests: a resident child across several writes,
  foreign Session token isolation, VM recovery without repeated writes,
  natural quit and fresh opening, observed SIGTERM, ignored SIGTERM followed
  by bounded unknown/force, and valid JSON before a malformed-frame terminal.
- `process/service.rs` tests: empty/final/oversized frames, UTF-8/JSON refusal,
  resource cleanup while publication is backpressured, and a host-teardown
  fence for already released cleanup workers.
- Existing generic Session tests cover pending opens/writes, duplicate/late
  terminals, retained deadlines, exact-generation closure and recovery. The
  existing job tests and echo/JSONL/gauge witnesses remain required gates.
- `examples/agent-dialogue.rkg`: two passing scenarios through the public CLI,
  new versioned traces in `tests/golden/gallery/agent-gateway`, and
  [chapter 6](../site/gallery/06-agent-gateway.md).
- `examples/agent-gateway.rkg`: the subsequent resident WebSocket composition,
  with two protocol scenarios, a two-socket/real-child test and an immediate
  frame/opening-order regression. The [gateway evidence](agent-gateway-showcase.md)
  supersedes the earlier gateway-composition remainder, not its explicit
  credential and model-usage limits.

Qualification on the reference host (Rust 1.97.1, macOS arm64):

| Check | Observed result |
|---|---|
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 1,368 passed, 0 failed, 0 ignored, 44 suite results |
| Final host-teardown follow-up, `cargo test -p rastok --all-features --lib --test process_service --no-fail-fast` | 378 library tests and 9 service tests passed; includes one additional teardown-fence regression after the full-run build |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed again after the teardown follow-up |
| `cargo fmt --all -- --check`; `git diff --check` | Passed |
| `cargo doc --workspace --no-deps` | Passed |
| `rastok-core --no-default-features` | Passed on thumbv8m.main-none-eabihf, thumbv7m-none-eabi and riscv32imac-unknown-none-elf |
| Cortex-M33 echo spike | `SPIKE_OK`, 67 records; peak heap 111,732 B; text 881,788 B + rodata 69,680 B = 951,468 B |

Heap margin is 19,340 B below 128 KiB; code margin is 97,108 B below 1 MiB.
These are workload-specific core measurements, not a hosted-process budget.
The final teardown follow-up changes hosted code only. The public CLI verifies
the two new scenario goldens and unchanged echo `5ffba3ea…`, JSONL `2f89c8b5…`
and gauge `0d3ad43d…` identities. The full run includes gallery break-it and
site-content checks. No browser audit, external protocol suite, release tag,
push or paid model invocation is claimed by these tests.

## Gateway work not implied by the service checkpoint

The real example command is `cat`; the Lab supplies imagined answers, not
Codex's wire protocol or model memory. `decode` intentionally distinguishes
valid JSON from a valid application reply. The subsequent
[gateway composition](agent-gateway-showcase.md) now supplies a transparent
resident WebSocket bridge and scripted protocol replies/notifications, with
separate real-socket/shell-child qualification. Authentication, business
timeouts, measured model usage and Process credential handling remain separate
adapter work. The earlier WebSocket-to-job example is not silently renamed
as proof of that system. No external agent or paid model was launched.
