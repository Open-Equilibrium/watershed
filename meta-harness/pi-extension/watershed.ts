// Watershed's pi extension. The Meta-Harness core loads it into pi's RPC mode: it pauses every
// tool call until the core decides (ADR-0188), lets the core branch a session at any entry and
// records timing as session entries, so the Trajectory reads everything from the session file
// (ADR-0189).
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** Title prefix of the select dialog the core answers instead of the user. */
export const APPROVAL_PREFIX = "watershed.approval ";

/**
 * pi's built-in tools that only read; pi gives its built-ins no annotations. Only these count as
 * read-only: another tool's own `readOnlyHint` is a claim nobody checks (ADR-0208).
 */
const READING_BUILTINS = new Set(["read", "grep", "find", "ls", "tool_search"]);

/** Answers the core may give; only the first three are user decisions. */
const DECISIONS = ["allow-once", "allow-session", "reject", "allow-auto"];

/** Milliseconds on a monotonic clock. */
type Clock = () => number;

/** The extension with an injectable clock; pi loads the default export. */
export function watershedWith(clock: Clock) {
  return (pi: ExtensionAPI): void => watershed(pi, clock);
}

export default watershedWith(() => performance.now());

function watershed(pi: ExtensionAPI, clock: Clock): void {
  const since = (start: number) => Math.round(clock() - start);
  pi.on("tool_call", async (event, ctx) => {
    if (ctx.mode !== "rpc") {
      return undefined;
    }
    const tool = pi.getAllTools().find((candidate) => candidate.name === event.toolName);
    const payload = {
      tool: event.toolName,
      arguments: event.input,
      readOnly: tool?.sourceInfo?.source === "builtin" && READING_BUILTINS.has(tool.name),
      // Allowances for a session end with it, also when an extension command switches it.
      session: ctx.sessionManager.getSessionId(),
    };
    const asked = clock();
    const decision = await ctx.ui.select(APPROVAL_PREFIX + JSON.stringify(payload), DECISIONS);
    if (decision === "allow-auto") {
      return undefined;
    }
    if (decision === "allow-once" || decision === "allow-session" || decision === "reject") {
      pi.appendEntry("watershed.approval", {
        toolCallId: event.toolCallId,
        tool: event.toolName,
        decision,
        waitedMs: since(asked),
      });
    }
    if (decision === "allow-once" || decision === "allow-session") {
      return undefined;
    }
    // Fail closed: a rejection, a dismissed dialog or a lost core all block the call.
    const reason = decision === "reject" ? "Rejected by the user" : "No approval was given";
    return { block: true, reason };
  });

  // pi runs these handlers before it stores the message or tool result, so a request's timing
  // entry directly precedes its response. A response's tools run in parallel and their results
  // are stored together, so each tool's timing entry is stored when that tool ends.
  // A request starts with pi's turn, before the provider answers with headers, so connection
  // and retry time count.
  let turnStarted: number | undefined;
  let request: { started: number; firstToken: number | null } | undefined;
  pi.on("turn_start", () => {
    turnStarted = clock();
  });
  pi.on("message_start", (event) => {
    if (event.message.role === "assistant") {
      request = { started: turnStarted ?? clock(), firstToken: null };
      turnStarted = undefined;
    }
  });
  pi.on("message_update", (event) => {
    // A block starts empty; the first token is its first content.
    if (request && request.firstToken === null && event.assistantMessageEvent?.type?.endsWith("_delta")) {
      request.firstToken = clock();
    }
  });
  pi.on("message_end", (event) => {
    if (event.message.role !== "assistant" || !request) {
      return;
    }
    const { started, firstToken } = request;
    request = undefined;
    pi.appendEntry("watershed.timing", {
      kind: "request",
      firstTokenMs: firstToken === null ? null : Math.round(firstToken - started),
      durationMs: since(started),
    });
  });
  pi.on("tool_execution_end", (event) => {
    pi.appendEntry("watershed.timing", {
      kind: "tool",
      toolCallId: event.toolCallId,
      durationMs: event.durationMs ?? null,
    });
  });

  pi.registerCommand("watershed-branch", {
    description: "Continue the session from an earlier entry",
    handler: async (args, ctx) => {
      // The label is the rest as written, so it may hold spaces.
      const [, entryId = "", label = ""] = /^(\S*)\s*([\s\S]*)$/.exec(args.trim()) ?? [];
      if (!entryId) {
        ctx.ui.notify("watershed-branch needs an entry id", "error");
        return;
      }
      const target = ctx.sessionManager.getEntry(entryId);
      if (!target) {
        ctx.ui.notify(`No entry ${entryId} to branch from`, "error");
        return;
      }
      try {
        const { cancelled } = await ctx.navigateTree(entryId, label ? { label } : {});
        if (cancelled) {
          ctx.ui.notify("The branch was cancelled", "error");
          return;
        }
      } catch (error) {
        ctx.ui.notify(`Could not branch: ${error instanceof Error ? error.message : String(error)}`, "error");
        return;
      }
      // Branching at a prompt continues before it; pi's own UI then offers the prompt for
      // editing, so hand it to the client's input the same way.
      if (target.type === "message" && target.message.role === "user") {
        ctx.ui.setEditorText(promptText(target.message.content));
      }
    },
  });
}

function promptText(content: string | readonly { type: string; text?: string }[]): string {
  if (typeof content === "string") {
    return content;
  }
  return content
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("");
}
