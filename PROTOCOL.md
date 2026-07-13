# Protocol

The protocol is the **integration seam** between the tools (editor + LSP model). Tools are protocol clients, not compiled-in modules. This file is the canonical contract; build tools against it, not against each other's internals. ADR-0029 selects local JSON-RPC over stdio for designed control/RPC surfaces, but M1's implemented runtime stream is bare JSONL events. The envelope is transport-agnostic and all cross-tool state is addressed by IDs.

Loop Agent is a **standalone product**, and its event stream is a public runtime contract in its own right (CLI JSONL mode, future RPC mode and local session log all carry these events — see [`docs/concept/V-Spec_LoopAgent.html`](docs/concept/V-Spec_LoopAgent.html)). Meta-Harness and Liquid consume that contract; they are not required to run Loop Agent.

## Participants

- **Loop Agent** — emits execution events; accepts local loop commands. Standalone; its event stream is public.
- **Meta-Harness** — self-contained headless control plane: consumes events from N agents through adapters; issues control/config commands; emits metrics. Exposes its own CLI/API/service surface for Liquid and BYOA (transport: D-023; see [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html)).
- **Liquid** — standalone workspace product; consumes events/metrics for rendering and issues user-originated commands. Liquid also exposes its **own** workspace CLI/API surface so external agents/tools read and edit workspace data; those mutations go through Liquid's permissioned pipeline and are recorded in its action history (see [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html), D-027). Loop Agent and Meta-Harness must use that surface; they do not mutate Liquid storage internals.
- **Adapters** — translate external agents (Codex CLI, Claude Code, Pi Agent, etc.) into the same contract.

## MVP boundary

Protocol v0 is designed for the Loop Agent CLI MVP and later Meta-Harness integration. It does **not** require a Watershed project-history/VCS engine. Host Git/project events may appear as artifacts when the host tool provides them, but protocol correctness must not depend on Watershed owning version control.

## Runtime event families (v0 scope)

- **Session lifecycle:** `session.started | session.paused | session.resumed | session.completed | session.failed`.
- **Loop/activity:** `loop.started | loop.completed | loop.failed | phase.entered | step.started | step.completed`.
- **Transcript:** `message.delta | message.completed` (near-real-time transcript sync; deltas are first-class).
- **Tool/runtime:** `tool.started | tool.progress | tool.completed | tool.failed | tool.timed_out`.
- **Artifacts:** `artifact.logged` (logs, summaries, handoff packs, checkpoints, host-provided diffs).
- **Attention:** `attention.requested` (input/approval required).
- **Metrics:** `metric.sample` (AgentPulse).
- **Errors:** `error` (generic runtime/protocol error event).

Runtime events use the v0 Loop Agent short-form name set decided in ADR-0036. `message.delta` and `tool.progress` stay first-class for near-real-time consumers. Do not maintain a second event naming convention.

M1 Loop Agent emits the families exercised by the D-015 fixtures and runtime error paths. `session.paused`, `tool.timed_out`, `artifact.logged`, `attention.requested` and `metric.sample` are v0-designed names for later emitters and are not emitted by the M1 runtime.

Command/request messages are not runtime event types. The future RPC/control surface uses JSON-RPC over stdio for local transport (ADR-0029); ADR-0055 selects the initial method set as `loop.start`, `loop.status`, `loop.cancel`, `loop.tail` and `loop.export`. Resulting runtime events may use `correlation_id` to link back to a request, and must still address state by IDs.

## Required v0 event-envelope fields

The v0 wire format is one UTF-8 JSON object per event. JSONL mode and `.loop/sessions/<session_id>.jsonl` store one event object per line; future RPC event delivery carries the same object in JSON-RPC payloads.

| Field | Type / rule |
| --- | --- |
| `protocol_version` | string, fixed to `"0"` for v0 |
| `event_id` | non-empty opaque string, unique within the session |
| `event_type` | one of the v0 runtime event names above |
| `session_id` | path-safe v0 token; opaque to consumers |
| `loop_id` | optional runtime loop invocation id when loop-scoped; unique within the session |
| `parent_loop_id` | optional parent runtime loop invocation id for subloop events |
| `sequence` | unsigned integer, starts at 1 and increases by exactly 1 per `session_id` |
| `timestamp` | RFC 3339 UTC timestamp string |
| `source` | non-empty opaque string identifying the emitter, e.g. `loop-agent-cli` |
| `payload` | JSON object; event-specific fields below |
| `correlation_id` | optional non-empty opaque string linking request/result events |

M1 Loop Agent derives timestamps from its event clock: `timestamp = base + (sequence - 1) seconds`. Fixture workspaces use a fixed base for byte-stable golden streams; non-fixture workspaces use a wall-clock base captured once at session start rather than sampling wall time per event.

## v0 ID safety and loop identity

- `session_id` is a token, not a path. V0 session IDs match `^[a-z0-9_-]{1,128}$`; lowercase-only IDs avoid filename aliasing on case-insensitive targets. Producers reject externally supplied values outside that grammar before reading or writing `.loop/sessions/<session_id>.jsonl`. Reject path separators (`/`, `\`), drive prefixes, absolute paths, percent-encoded separators, `.`, `..` and empty strings before filesystem access. If a future protocol accepts broader external session IDs, it must specify a canonical filename encoding instead of joining raw IDs into paths.
- `loop_id` is a runtime invocation id, not the registry/definition id. The root loop and every subloop invocation get distinct `loop_id` values within the session. Reusing one subloop definition twice therefore emits two different `loop_id` values, each with `parent_loop_id` equal to the containing runtime loop invocation id.
- Loop definition identity travels in payload fields, not in `loop_id`. `loop.*` events carry `loop_definition_id`; `loop_name` is optional display metadata.

## M1 local session storage

The append-only session event log is authoritative for replay and catch-up. Local paths, locks, recovery and replay/resume behavior are defined in the [Loop Agent V-Spec](docs/concept/V-Spec_LoopAgent.html#surfaces); other tools consume public surfaces, never this store directly (see "No co-location assumption" below).

## M1 local append and live delivery (ADR-0059)

One asynchronous serial writer owns each session's event order. For every event or ordered micro-batch it:

1. constructs the typed event and validates it against the active protocol version;
2. assigns or validates the stable `event_id`, then assigns the next per-session `sequence`;
3. canonically serializes the event once;
4. appends the canonical bytes to the session's append-only log and confirms the process-level write;
5. publishes the same committed event, in sequence, to the internal bus and live observers.

Publication never overtakes persistence. Publication order equals log sequence order; an append failure is not visible to observers, stops the session writer, and prevents every later event from passing it. A retry preserves the logical `event_id` and `sequence`. If the failure prevents a terminal error event from being appended, the command returns the runtime/I/O failure status while leaving the prior log as a valid prefix.

The writer uses bounded queues and backpressure. `message.delta` and `tool.progress` may share an ordered micro-batch for at most 25 ms; the complete batch is appended first and then published in sequence. A semantic/terminal event closes any pending batch immediately. Normal delivery is event-driven, not polling, and buffering is never unbounded. A disconnected or persistently lagging observer is detached rather than rolling back an appended event or blocking the session indefinitely; it catches up from its highest contiguous `sequence`. Live delivery is therefore at least once: consumers ignore duplicate `event_id` values, detect gaps through `sequence`, and use replay/catch-up after a gap or reconnect. Do not claim exactly-once network delivery.

Append-before-publish is distinct from machine/power-loss durability. A successful append means the ordered bytes have crossed Loop Agent's userspace buffering boundary into the local log; it does not mean one `fsync` per event. The writer flushes and synchronizes at `message.completed`, `tool.completed`, `tool.failed`, `tool.timed_out`, `session.paused`, `session.completed` and `session.failed`, and at least once per second while an active stream has unsynchronized events. High-frequency deltas may share these boundaries. Remote replication cadence, crash recovery on a new host and the durable ownership lease remain post-M1 under ADR-0039.

Future Liquid/Meta-Harness consumers render published events immediately, retain the highest contiguous sequence, deduplicate by `event_id`, request catch-up on a gap and reconnect from that sequence. Live delivery supplies low latency; the authoritative log and sequence replay supply correctness.

Minimum v0 payload fields:

All listed payload fields are strings unless noted otherwise; string arrays are JSON arrays of strings. `role` is `system | user | assistant | tool`, `value` is a JSON number, `exit_code` is an integer and `data` is a JSON object.

- `session.*`: `reason` optional except failure events, where it is required. M1 currently emits `reason: "fixture-start"` on `session.started` for all run starts.
- `loop.*`: `loop_definition_id` required; `loop_name` optional; `error` required for `loop.failed`.
- `phase.entered`: `phase_id`, `phase_name`, `instruction_ids` and `tool_ids` (string arrays; empty when none).
- `step.started | step.completed`: `step_id`, `step_name`, optional `phase_id`, optional `instruction_id`, optional `connection_ids` and `connection_kinds` (string arrays; `connection_kinds` values are `data | trigger | refresh`). If either connection array is present, both are present with the same length; index `i` in `connection_ids` pairs with index `i` in `connection_kinds`, in the owning Step block's `connection_refs` order after registry resolution. With no connections, omit both arrays or emit both as empty arrays.
- `message.delta`: `message_id`, `role`, `content_delta`.
- `message.completed`: `message_id`, `role`.
- `tool.started`: `tool_id`, `tool_name`, `tool_kind` (`predefined-command | own-script`), `read_scope` and `write_scope` (string arrays), `allowed_parameters` (string array of allowed parameter names), `network_access` (`deny | declared`).
- `tool.progress`: `tool_id`, `message`.
- `tool.completed`: `tool_id`, optional `exit_code`.
- `tool.failed | tool.timed_out`: `tool_id`, `error`.
- `artifact.logged`: `artifact_id`, `artifact_type`, `uri`.
- `attention.requested`: `request_id`, `reason`.
- `metric.sample`: `metric_name`, `value`.
- `error`: `code`, `message`, optional `data`.

## CLI exit status

- `0`: command completed successfully.
- `64`: command-line usage or input validation error.
- `65`: runtime, registry, policy, protocol, session-state or I/O failure.

## Canonical event JSONL serialization (v0)

D-015 golden streams and `.loop/sessions/<session_id>.jsonl` logs use the same canonical event JSONL bytes:

- UTF-8; one event object per line; LF line endings; final LF required.
- No insignificant whitespace outside or inside JSON objects.
- Object members are sorted lexicographically by key at every object level, including `payload`.
- Arrays preserve their schema-defined order.
- Strings are NFC-normalized. Emit printable non-control Unicode as UTF-8; escape only `"` and `\`, plus control characters using the shortest JSON escape.
- Numbers are finite JSON numbers. Integers use base-10 with no leading zeros; non-integers use the shortest round-trippable decimal form; `-0` serializes as `0`.
- Literals are lowercase JSON `true`, `false` and `null`; `null` appears only where the event payload schema explicitly allows it.

Byte-stable golden diffs compare these canonical bytes. Consumers may still parse events structurally for compatibility, but checked-in M0/M1 fixture streams do not choose their own object ordering, whitespace or escaping.

## Contract rules

- **Versioned & additive.** Breaking changes bump the protocol version; clients negotiate.
- **Normalized events.** Adapters must map native agent events into the families above; do not leak native shapes.
- **Artifact contract over runtime parity.** Agents differ in runtime semantics; they must agree only on this message contract.
- **Deterministic ordering within a session.** A participant must emit monotonically increasing `sequence` values per session.
- **No exfiltration via protocol.** Events and future commands carrying writes are subject to the security policy in `SECURITY.md`.
- **No co-location assumption.** A participant must not assume it shares a host, filesystem or process tree with another. All cross-tool state is addressed by `session_id`/`workspace_id` over the protocol; a tool never reads another tool's local store directly (e.g. Loop Agent's `.loop/sessions` is consumed via the event stream or tail/export surfaces, and RPC when implemented, never from disk by Meta-Harness or Liquid). This keeps the local-only M0 transport (D-002) from foreclosing later remote topologies (D-043/ADR-0038).

## Implementation constraints

The `proto` v0 implementation must serialize these JSON event envelopes for JSONL output, local logs and future JSON-RPC event delivery without adding co-location assumptions. Control methods stay separate from runtime events; do not add `cmd.*` event names.

Later cloud/remote durability requires replication plus durable storage, with live Meta-Harness ingestion where attached and a persistent `.loop` append-only JSONL volume otherwise. Remote replication cadence, resume on a new host, crash replay to the last durable `sequence` and the session-ownership lease must be defined before remote execution ships.
