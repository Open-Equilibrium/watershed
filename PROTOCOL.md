# Protocol

The protocol is the **integration seam** between the tools (editor + LSP model). Tools are protocol clients, not compiled-in modules. This file is the canonical contract; build tools against it, not against each other's internals. ADR-0029 selects local JSON-RPC over stdio for designed control/RPC surfaces, but M1's implemented runtime stream is bare JSONL events. The envelope is transport-agnostic and all cross-tool state is addressed by IDs.

Flow Agent is a **standalone host-local product**, and its event stream is a public runtime contract in its own right (CLI JSONL mode, future RPC mode and local session log all carry these events — see [`docs/concept/V-Spec_FlowAgent.html`](docs/concept/V-Spec_FlowAgent.html)). A Meta-Harness on the same host consumes that contract; neither Meta-Harness nor Liquid is required to run Flow Agent.

## Participants

- **Flow Agent** — standalone CLI that emits execution events and accepts commands on its current host; its event stream is public.
- **Meta-Harness** — self-contained, host-scoped headless control plane: consumes events from CLI agents on its own host through adapters; issues control/config commands; emits metrics; and exposes a local-or-remote CLI/API/service surface for Liquid and BYOA (transport: D-023; see [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html)). It never controls another host's processes.
- **Liquid** — standalone local-first workspace product. Each interactive or headless instance reads and mutates a local replica, exchanges committed Workspace changes through the central Sync Server, and may present projections from one or more Meta-Harness instances. Every projection retains its instance identity, freshness and authority. Liquid exposes its **own** Workspace CLI/API so external agents and integrations use typed actions through its Role, permission and History pipeline (see [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html), D-027). Flow Agent and Meta-Harness do not mutate Liquid storage internals.
- **Sync Server** — central star-topology service for resumable Workspace change exchange. It neither runs Liquid Apps nor routes Meta-Harness commands.
- **Adapters** — translate external agents (Codex CLI, Claude Code, Pi Agent, etc.) into the same contract.

## Topology and ownership invariants

- Agent-process ownership is host-local: a Meta-Harness may start, stop and observe only CLI processes on its own host.
- API reachability is independent of execution locality: Liquid or BYOA may call a Meta-Harness from another device when authenticated transport exists.
- Liquid routes every live command to the Meta-Harness instance that owns the addressed session or configuration. A merged projection never creates cross-instance authority.
- Liquid replicas connect to the Sync Server, never directly to one another. The normal replication unit is an authorized Workspace, not a hand-selected set of Blocks.
- User devices normally sync each authorized Workspace in full. A headless Liquid replica requires explicit workspace-level opt-in before it receives that Workspace.
- The Sync Server and headless Liquid replica are separate logical participants even when one deployment co-locates them.
- Workspace sync and live agent control are separate planes. Sync exchanges Liquid Actions/state; it does not tunnel Meta-Harness commands or imply that cached agent state is controllable offline.
- Loss of sync connectivity does not change a Liquid replica's working store: local reads and mutations continue, while resumable exchange waits for connectivity.
- Agents use Liquid through the Workspace CLI/API and effective Roles. No visibility, View placement, Connection or Meta-Harness reachability grants implicit Workspace authority.

## MVP boundary

Protocol v0 is designed for the Flow Agent CLI MVP and later Meta-Harness integration. It does **not** require a Watershed project-history/VCS engine. Host Git/project events may appear as artifacts when the host tool provides them, but protocol correctness must not depend on Watershed owning version control.

## Runtime event families (v0 scope)

- **Session lifecycle:** `session.started | session.paused | session.resumed | session.completed | session.failed`.
- **Loop/activity:** `loop.started | loop.completed | loop.failed | phase.entered | step.started | step.completed`.
- **Transcript:** `message.delta | message.completed` (near-real-time transcript sync; deltas are first-class).
- **Tool/runtime:** `tool.started | tool.progress | tool.completed | tool.failed | tool.timed_out`.
- **Artifacts:** `artifact.logged` (logs, summaries, handoff packs, checkpoints, host-provided diffs).
- **Attention:** `attention.requested` (input/approval required).
- **Metrics:** `metric.sample` (AgentPulse).
- **Errors:** `error` (generic runtime/protocol error event).

Runtime events use the v0 Flow Agent short-form name set decided in ADR-0036. `message.delta` and `tool.progress` stay first-class for near-real-time consumers. Do not maintain a second event naming convention.

M1 Flow Agent emits the families exercised by the ADR-0034 fixtures and runtime error paths. `session.paused`, `tool.timed_out`, `artifact.logged`, `attention.requested` and `metric.sample` are v0-designed names for later emitters and are not emitted by the M1 runtime.

### v0 lifecycle ordering

- `session.started` is the first event. `session.completed` or `session.failed` is last and requires every started loop, step, tool and message to be terminal.
- A loop-scoped event follows its unique `loop.started` and precedes that loop's `loop.completed` or `loop.failed`. A subloop's `parent_loop_id` identifies its unchanged, active parent.
- `phase.entered` requires no active step and selects that loop's current phase. Each `step.started` belongs to the current phase; a loop has at most one active step, closed by the matching `step.completed`.
- `tool.started` belongs to the active step. Its progress and terminal event use the same loop, phase, step and tool identity. A pre-phase `tool.failed` may omit `tool.started` to record a failure during preflight; after `phase.entered`, it may not.
- `message.delta` belongs to the active step and starts a message identity; further deltas and the matching `message.completed` retain its role. Completed lifecycle identities cannot be reused.

Command/request messages are not runtime event types. The future RPC/control surface uses JSON-RPC over stdio for local transport (ADR-0029); ADR-0055 selects the initial method set as `flow.start`, `flow.status`, `flow.cancel`, `flow.tail` and `flow.export`. Resulting runtime events may use `correlation_id` to link back to a request, and must still address state by IDs.

## Required v0 event-envelope fields

The v0 wire format is one UTF-8 JSON object per event. JSONL mode and the session's ordered event segments store one event object per line; future RPC event delivery carries the same object in JSON-RPC payloads.

| Field | Type / rule |
| --- | --- |
| `protocol_version` | string, fixed to `"0"` for v0 |
| `event_id` | non-empty opaque string, unique within the session |
| `event_type` | one of the v0 runtime event names above |
| `session_id` | path-safe v0 token; opaque to consumers |
| `loop_id` | optional runtime loop invocation id when loop-scoped; unique within the session |
| `parent_loop_id` | optional parent runtime loop invocation id for subloop events |
| `sequence` | unsigned integer, starts at 1 and increases by exactly 1 per `session_id` |
| `timestamp` | canonical RFC 3339 UTC form ending in literal `Z`; numeric zero offsets are not accepted |
| `source` | non-empty opaque string identifying the emitter, e.g. `flow-agent-cli` |
| `payload` | JSON object; event-specific fields below |
| `correlation_id` | optional non-empty opaque string linking request/result events |

Consumers retain unknown top-level fields so additive v0 extensions survive replay and forwarding unchanged.

M1 Flow Agent derives timestamps from its event clock: `timestamp = base + (sequence - 1) seconds`. Fixture workspaces use a fixed base for byte-stable golden streams; non-fixture workspaces use a wall-clock base captured once at session start rather than sampling wall time per event.

## v0 ID safety and loop identity

- `session_id` is a token, not a path. V0 session IDs match `^[a-z0-9_-]{1,128}$` except Windows DOS device basenames (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`); lowercase-only IDs avoid filename aliasing on case-insensitive targets. Producers reject externally supplied values outside that grammar before reading or writing a session bundle. Reject path separators (`/`, `\`), drive prefixes, absolute paths, percent-encoded separators, `.`, `..` and empty strings before filesystem access. If a future protocol accepts broader external session IDs, it must specify a canonical filename encoding instead of joining raw IDs into paths.
- `loop_id` is a runtime invocation id, not the registry/definition id. The root loop and every subloop invocation get distinct `loop_id` values within the session. Reusing one subloop definition twice therefore emits two different `loop_id` values, each with `parent_loop_id` equal to the containing runtime loop invocation id.
- Loop definition identity travels in payload fields, not in `loop_id`. `loop.*` events carry `loop_definition_id`; `loop_name` is optional display metadata.

## M1 local session storage

The ordered append-only event segments are authoritative for replay and catch-up. The first is `.flow/sessions/<session_id>.jsonl`; later segments are `<session_id>.<six-digit-ordinal>.jsonl`, beginning at `000002`. Rotation occurs before an event would exceed the canonical uncompressed-byte per-segment limit; one event is never split, sequence and all [session safety limits](PERFORMANCE.md#adr-0068-safety-envelope) continue unchanged, and prior segments become immutable. Context manifests follow the same rotation and ordinal rules from `<session_id>.contexts.jsonl`; one manifest record is never split.

Context manifests reference the exact canonical source bytes through `session-object:sha256:<digest>`. Flow Agent stores those session-owned immutable objects once per digest, accounts existing objects again on resume and verifies availability and content before use. Larger future artifacts must be chunked; no reference needed to validate or reconstruct recorded canonical history and provider context may point only to mutable or externally owned storage. Export and deletion operate on the complete bundle. The bundle preserves canonical history but cannot reproduce an external provider, tool, compatible registry for continuation, mutable environment or undeclared side effect. Local paths, locks, recovery and replay/resume behavior are defined in the [Flow Agent V-Spec](docs/concept/V-Spec_FlowAgent.html#surfaces); other tools consume public surfaces, never this store directly.

## M1 local append and live delivery (ADR-0059, ADR-0062)

Runtime execution constructs each typed event, assigns its stable `event_id` and next per-session `sequence`, and canonically serializes it once. One asynchronous serial writer then owns each session's append order. For every event or ordered micro-batch it:

1. validates the constructed event against the active protocol version and expected session order;
2. appends the canonical bytes to the session's append-only log and confirms the process-level write;
3. updates the session's highest committed sequence and attempts a non-blocking live notification.

Notification never overtakes persistence. A failed write notifies any complete event prefix only after removing an incomplete suffix, then stops the writer before later events can pass it. Failed cleanup reports no new readable prefix. If the failure prevents a terminal error event from being appended, the command returns the runtime/I/O failure status; successful cleanup leaves the prior log as a valid prefix.

Each caller-owned subscription has one pending wake-up slot retaining its earliest committed `sequence` and shared state containing the highest committed `sequence`; notifications carry no event payload. The producer updates that high-watermark after append and uses a non-blocking send. A full slot coalesces the wake-up, and a closed receiver is ignored, so a slow or disconnected consumer cannot block a run or another session. The core owns no caller transport, output task or arbitrary blocking writer. The CLI owns stdout; future adapters own their socket or IPC transport.

A receiver owns its last fully processed sequence cursor. It subscribes before replay, reads validated events where `sequence > cursor` from the authoritative log, advances the cursor only after processing each event, then drains/rechecks notifications until its cursor reaches the observed high-watermark before waiting again. The earliest pending sequence lets an operation-scoped projection exclude commits made before that operation without bypassing validation of the log. This closes the replay/live race: dropped and coalesced wake-ups lose no committed event. Session-log reads and notification state remain explicitly bounded. Network transports still must not claim exactly-once delivery.

Consecutive `message.delta` and `tool.progress` events share a bounded ordered micro-batch for at most 25 ms; the complete batch is appended before its per-event notifications. A semantic or terminal event closes any pending batch immediately. Append and notification are event-driven; only replay/tail clients poll the authoritative store when no live subscription is available.

Append-before-notification is distinct from machine/power-loss durability. A successful append means the ordered bytes have crossed Flow Agent's userspace buffering boundary into the local log; it does not mean one `fsync` per event. The writer flushes and synchronizes at `message.completed`, `tool.completed`, `tool.failed`, `tool.timed_out`, `session.paused`, `session.completed` and `session.failed`, and at least once per second while an active stream has unsynchronized events. High-frequency deltas may share these boundaries. Remote replication cadence, crash recovery on a new host and the durable ownership lease remain post-M1 under ADR-0039.

Minimum v0 payload fields:

All listed payload fields are strings unless noted otherwise; string arrays are JSON arrays of strings. `role` is `system | user | assistant | tool`, `value` is a JSON number, `exit_code` is an integer and `data` is a JSON object.

- `session.*`: `reason` optional except failure events, where it is required.
- `loop.*`: `loop_definition_id` required; `loop_name` optional; `error` required for `loop.failed`.
- `phase.entered`: `phase_id`, `phase_name`, `instruction_ids` and `tool_ids` (string arrays; empty when none).
- `step.started | step.completed`: `step_id`, `step_name`, optional `phase_id`, optional `instruction_id`, optional `connection_ids` and `connection_kinds` (string arrays; `connection_kinds` values are `data | trigger | refresh`). If either connection array is present, both are present with the same length; index `i` in `connection_ids` pairs with index `i` in `connection_kinds`, in the owning Step block's `connection_refs` order after registry resolution. With no connections, omit both arrays or emit both as empty arrays.
- `step.completed` closes the step lifecycle on success or failure; derive outcome from the tool, error, loop and session events.
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

ADR-0034 golden streams and `.flow/sessions/<session_id>.jsonl` logs use the same canonical event JSONL bytes:

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
- **Deterministic ordering within a session.** A participant must follow the event envelope's `sequence` rule per session.
- **No exfiltration via protocol.** Events and future commands carrying writes are subject to the security policy in `SECURITY.md`.
- **No private-store or implicit co-location coupling.** A protocol client must not infer shared filesystem/process access from API reachability. All cross-tool state is addressed by IDs and public surfaces; a tool never reads another tool's local store directly. The only deliberate process co-location is a Meta-Harness executor owning CLI agents on the same host. Remote Liquid/Meta-Harness clients remain possible without remote agent-process ownership (ADR-0038).

## Implementation constraints

The `proto` v0 implementation must serialize these JSON event envelopes for JSONL output, local logs and future JSON-RPC event delivery without adding co-location assumptions. Control methods stay separate from runtime events; do not add `cmd.*` event names.

Later server-host durability requires replication plus durable storage, with live ingestion by the Meta-Harness on that host and a persistent `.flow` append-only JSONL volume otherwise. Replication cadence, crash replay and any transfer of session ownership to a new host must be defined before such migration ships; a Meta-Harness must never silently control a CLI process on another host.
