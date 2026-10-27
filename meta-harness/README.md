# Meta-Harness

Planned for M2; see the [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html). The Liquid agent-surface slice in [PLAN.md](../PLAN.md) builds its core first, embedded in Liquid (ADR-0184).

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — Rust core: runs `pi --mode rpc` sessions and translates them into the agent-neutral event model (ADR-0195).
- `pi-extension/` — the pi extension the core loads into each session.

## Core–extension protocol

Internal to the slice; it changes together with both sides.

- **Approvals (ADR-0188):** only in pi's RPC mode, which the core uses, and before every tool call, the extension opens a `select` dialog titled `watershed.approval ` followed by `{"tool", "arguments", "readOnly", "builtin", "session", "toolCallId"}` as JSON; `readOnly` holds only for pi's reading built-ins, never for a tool's own `readOnlyHint` (ADR-0208), `builtin` says the tool is pi's own, so only pi's `bash`, `edit` and `write` ask as a command or file changes, and other tools show all their arguments, `session` is pi's session id, so an allowance for the session ends when an extension command switches sessions, and `toolCallId` names the call among those pi already started. The core answers `allow-once`, `allow-session`, `reject` or `allow-auto`, or cancels the dialog when Stop withdraws the request. Anything else, including a lost core, blocks the call. The extension records each user decision as a `watershed.approval` custom entry in the pi session.
- **Timing:** for the Trajectory, the extension appends a `watershed.timing` custom entry before each stored model response (`kind: "request"` with `firstTokenMs` and `durationMs`) and when each tool ends (`kind: "tool"` with `toolCallId` and `durationMs`), before pi stores the response's tool results together; approval entries carry `waitedMs`.
- **Branching:** the command `/watershed-branch <entry-id> [label]` continues the session from that entry, so the next prompt starts a new branch in the same session file. The core also takes this extension command, as pi's `get_commands` lists it, as the sign that the extension is loaded. At a prompt, pi continues before it and the extension hands the prompt back as the input text; a cancelled or failed navigation becomes an error notification.

Test the extension with `node --test meta-harness/pi-extension/watershed.test.ts`. The manual check with real pi and a scripted model is `meta-harness-core/examples/pi_check.rs`; its header shows how to run it.
