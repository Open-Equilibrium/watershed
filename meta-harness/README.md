# Meta-Harness

Planned for M2; see the [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html). The Liquid agent-surface slice in [PLAN.md](../PLAN.md) builds its core first, embedded in Liquid (ADR-0184).

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — Rust core: runs `pi --mode rpc` sessions and translates them into the agent-neutral event model (ADR-0195).
- `pi-extension/` — the pi extension the core loads into each session.

## Core–extension protocol

Internal to the slice; it changes together with both sides.

- **Approvals (ADR-0188):** before every tool call the extension opens a `select` dialog titled `watershed.approval ` followed by `{"tool", "arguments", "readOnly"}` as JSON. The core answers `allow-once`, `allow-session`, `reject` or `allow-auto`. Anything else, including a lost core, blocks the call. The extension records each user decision as a `watershed.approval` custom entry in the pi session.
- **Timing:** for the Trajectory, the extension appends a `watershed.timing` custom entry before each stored model response (`kind: "request"` with `firstTokenMs` and `durationMs`) and before each tool result (`kind: "tool"` with `toolCallId` and `durationMs`); approval entries carry `waitedMs`.
- **Branching:** the command `/watershed-branch <entry-id> [label]` continues the session from that entry, so the next prompt starts a new branch in the same session file.

Test the extension with `node --test meta-harness/pi-extension/watershed.test.ts`. The manual check with real pi and a scripted model is `meta-harness-core/examples/pi_check.rs`; its header shows how to run it.
