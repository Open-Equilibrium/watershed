// Watershed's pi extension. The Meta-Harness core loads it into pi's RPC mode: it pauses every
// tool call until the core decides (ADR-0188) and lets the core branch a session at any entry.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** Title prefix of the select dialog the core answers instead of the user. */
export const APPROVAL_PREFIX = "watershed.approval ";

/** Answers the core may give; only the first three are user decisions. */
const DECISIONS = ["allow-once", "allow-session", "reject", "allow-auto"];

export default function watershed(pi: ExtensionAPI): void {
  pi.on("tool_call", async (event, ctx) => {
    if (ctx.mode !== "rpc") {
      return undefined;
    }
    const tool = pi.getAllTools().find((candidate) => candidate.name === event.toolName);
    const payload = {
      tool: event.toolName,
      arguments: event.input,
      readOnly: tool?.annotations?.readOnlyHint === true,
    };
    const decision = await ctx.ui.select(APPROVAL_PREFIX + JSON.stringify(payload), DECISIONS);
    if (decision === "allow-auto") {
      return undefined;
    }
    if (decision === "allow-once" || decision === "allow-session" || decision === "reject") {
      pi.appendEntry("watershed.approval", {
        toolCallId: event.toolCallId,
        tool: event.toolName,
        decision,
      });
    }
    if (decision === "allow-once" || decision === "allow-session") {
      return undefined;
    }
    // Fail closed: a rejection, a dismissed dialog or a lost core all block the call.
    const reason = decision === "reject" ? "Rejected by the user" : "No approval was given";
    return { block: true, reason };
  });

  pi.registerCommand("watershed-branch", {
    description: "Continue the session from an earlier entry",
    handler: async (args, ctx) => {
      const [entryId, label] = args.trim().split(/\s+/);
      if (!entryId) {
        ctx.ui.notify("watershed-branch needs an entry id", "error");
        return;
      }
      await ctx.navigateTree(entryId, label ? { label } : {});
    },
  });
}
