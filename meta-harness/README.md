# Meta-Harness

Planned for M2; see the [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html). The Liquid agent-surface slice in [PLAN.md](../PLAN.md) builds its core first, embedded in Liquid (ADR-0184).

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — Rust core: runs `pi --mode rpc` sessions and translates them into the agent-neutral event model (ADR-0195).
- `pi-extension/` — the pi extension the core loads into each session.

## Core–extension protocol

Internal to the slice; it changes together with both sides.

- **Approvals (ADR-0188):** only in pi's RPC mode, which the core uses, and before every tool call, the extension opens a `select` dialog titled `watershed.approval ` followed by `{"tool", "arguments", "readOnly", "builtin", "session", "toolCallId", "writesTo"}` as JSON; `readOnly` holds only for pi's reading built-ins, never for a tool's own `readOnlyHint` (ADR-0208), `builtin` says the tool is pi's own, so only pi's `bash`, `edit` and `write` ask as a command or file changes, and other tools show all their arguments, `session` is pi's session id, so an allowance for the session ends when an extension command switches sessions, `toolCallId` names the call among those pi already started, and `writesTo`, only for pi's `edit` and `write`, names the file pi changes when the given path does not say so plainly. The core answers `allow-once`, `allow-session`, `reject` or `allow-auto`, or cancels the dialog when Stop withdraws the request. Anything else, including a lost core, blocks the call. The extension records each user decision as a `watershed.approval` custom entry in the pi session.
- **Checkpoints (ADR-0234):** only in RPC mode, the extension holds each run before its first request (`before_agent_start`) and once it settled (`agent_settled`, after follow-ups and retries) with an `input` dialog titled `watershed.checkpoint ` followed by `{"session", "turn", "phase"}` as JSON: pi's session id, a new UUID per run and `start` or `end`. The core takes the checkpoint, never shows the dialog and answers `{"ok":true,"canRun":true}` at a turn's start and `{"ok":true}` at its end, the latter with `files`, `added` and `removed` when it could compare the turn's checkpoints, or `{"error","canRun"}`; ids that cannot name a ref are refused. The extension then appends a `watershed.checkpoint` custom entry with the same fields, the counts and any `error` ("No answer from Watershed" when there was none), so the start entry precedes the run's prompt and the end entry follows its last entry; a failed start aborts the run with an error notice unless `canRun` is explicitly `true`. The core permits startup after a failed snapshot only while that turn holds its Git checkout guard or, outside Git, its project-folder guard; repository-opening and guard-registration failures refuse startup. A missing or unreadable answer also refuses startup. End-checkpoint failures remain nonfatal.
- **Timing:** for the Trajectory, the extension appends a `watershed.timing` custom entry before each stored model response (`kind: "request"` with `firstTokenMs` and `durationMs`) and when each tool ends (`kind: "tool"` with `toolCallId` and `durationMs`), before pi stores the response's tool results together; approval entries carry `waitedMs`.
- **Branching:** the command `/watershed-branch <entry-id> [label]` continues the session from that entry, so the next prompt starts a new branch in the same session file. The core also takes this extension command, as pi's `get_commands` lists it with the extension file the core started pi with, as the sign that the extension is loaded. At a prompt, pi continues before it and the extension hands the prompt back as the input text; a cancelled or failed navigation becomes an error notification.

Test the extension with `node --test meta-harness/pi-extension/watershed.test.ts`. The manual check with real pi and a scripted model is `meta-harness-core/examples/pi_check.rs`; its header shows how to run it.

## Session exit

After pi exits, input delivery ends and stdout drains to EOF for at most two seconds, independently of event delivery. Every complete record read is processed in order; a trailing partial record is dropped. If the pipe remains open at the deadline, the core closes it and emits one warning through the existing notification path: "Some agent output may be missing." Pending commands without a response resolve as "agent exited" and `Exited` follows all accepted events. Ordinary buffered output reaches EOF without a warning. This does not terminate descendants.
