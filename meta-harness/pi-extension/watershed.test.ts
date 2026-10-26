import assert from "node:assert/strict";
import { test } from "node:test";

import { APPROVAL_PREFIX, watershedWith } from "./watershed.ts";

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

function load(clock: () => number = () => 0) {
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>();
  const entries: { customType: string; data: unknown }[] = [];
  // As pi 1.3.0 reports them: its built-ins carry no annotations.
  const builtin = { source: "builtin" };
  const extension = { source: "local" };
  const tools = [
    { name: "read", sourceInfo: builtin },
    { name: "grep", sourceInfo: builtin },
    { name: "tool_search", sourceInfo: builtin },
    { name: "bash", sourceInfo: builtin },
    { name: "lookup", annotations: { readOnlyHint: true }, sourceInfo: extension },
    { name: "find", sourceInfo: extension },
  ];
  watershedWith(clock)(
    {
    on: (event: string, handler: Handler) => handlers.set(event, handler),
    registerCommand: (name: string, options: never) => commands.set(name, options),
    appendEntry: (customType: string, data: unknown) => entries.push({ customType, data }),
    getAllTools: () => tools,
  } as never,
    clock,
  );
  return { handlers, commands, entries };
}

function context(
  answer: string | undefined,
  mode = "rpc",
  session: { entries?: Record<string, unknown>; navigate?: () => Promise<{ cancelled: boolean }> } = {},
) {
  const questions: { title: string; options: string[] }[] = [];
  const notes: string[] = [];
  const branches: unknown[][] = [];
  const drafts: string[] = [];
  const ctx = {
    mode,
    ui: {
      select: async (title: string, options: string[]) => {
        questions.push({ title, options });
        return answer;
      },
      notify: (message: string) => notes.push(message),
      setEditorText: (text: string) => drafts.push(text),
    },
    sessionManager: {
      getSessionId: () => "s1",
      getEntry: (id: string) =>
        session.entries ? session.entries[id] : { type: "message", message: { role: "assistant", content: [] } },
    },
    navigateTree: async (...args: unknown[]) => {
      branches.push(args);
      return session.navigate ? session.navigate() : { cancelled: false };
    },
  };
  return { ctx, questions, notes, branches, drafts };
}

test("asks the core before every tool call and records user decisions", async () => {
  const cases = [
    { answer: "allow-once", result: undefined, recorded: true },
    { answer: "allow-session", result: undefined, recorded: true },
    { answer: "reject", result: { block: true, reason: "Rejected by the user" }, recorded: true },
    { answer: "allow-auto", result: undefined, recorded: false },
    { answer: undefined, result: { block: true, reason: "No approval was given" }, recorded: false },
  ];
  for (const { answer, result, recorded } of cases) {
    const { handlers, entries } = load();
    const { ctx, questions } = context(answer);
    const call = { type: "tool_call", toolCallId: "c1", toolName: "bash", input: { command: "ls" } };
    assert.deepEqual(await handlers.get("tool_call")!(call, ctx), result, String(answer));
    assert.equal(questions.length, 1);
    const [{ title, options }] = questions;
    assert.ok(title.startsWith(APPROVAL_PREFIX));
    assert.deepEqual(JSON.parse(title.slice(APPROVAL_PREFIX.length)), {
      tool: "bash",
      arguments: { command: "ls" },
      readOnly: false,
      session: "s1",
    });
    assert.deepEqual(options, ["allow-once", "allow-session", "reject", "allow-auto"]);
    assert.deepEqual(
      entries,
      recorded
        ? [
            {
              customType: "watershed.approval",
              data: { toolCallId: "c1", tool: "bash", decision: answer, waitedMs: 0 },
            },
          ]
        : [],
    );
  }
});

test("reports read-only tools and stays out of other modes", async () => {
  const { handlers } = load();
  const readOnly = async (toolName: string) => {
    const rpc = context("allow-auto");
    await handlers.get("tool_call")!({ type: "tool_call", toolCallId: "c", toolName, input: {} }, rpc.ctx);
    return JSON.parse(rpc.questions[0].title.slice(APPROVAL_PREFIX.length)).readOnly;
  };
  assert.deepEqual(
    [await readOnly("read"), await readOnly("grep"), await readOnly("tool_search")],
    [true, true, true],
    "pi's reading built-ins",
  );
  assert.deepEqual(
    [await readOnly("bash"), await readOnly("find"), await readOnly("lookup")],
    [false, false, false],
    "a built-in that changes things, an extension tool named like a reading built-in, and a tool " +
      "that only calls itself read-only",
  );
  const read = { type: "tool_call", toolCallId: "c2", toolName: "read", input: { path: "a" } };
  const tui = context("reject", "tui");
  assert.equal(await handlers.get("tool_call")!(read, tui.ctx), undefined);
  assert.equal(tui.questions.length, 0);
});

test("branches the session at an entry", async () => {
  const { commands } = load();
  const branch = commands.get("watershed-branch")!;
  const labelled = context(undefined);
  await branch.handler("874681ad Rate  limit A", labelled.ctx);
  const plain = context(undefined);
  await branch.handler(" 6ed210af ", plain.ctx);
  const empty = context(undefined);
  await branch.handler("", empty.ctx);
  assert.deepEqual(labelled.branches, [["874681ad", { label: "Rate  limit A" }]]);
  assert.deepEqual(plain.branches, [["6ed210af", {}]]);
  assert.deepEqual(empty.branches, []);
  assert.deepEqual(empty.notes, ["watershed-branch needs an entry id"]);
});

test("hands a prompt back for editing and reports a branch that did not happen", async () => {
  const { commands } = load();
  const branch = commands.get("watershed-branch")!;
  const prompt = {
    type: "message",
    message: { role: "user", content: [{ type: "text", text: "Add rate limiting\nwith tests" }] },
  };
  const edit = context(undefined, "rpc", { entries: { u1: prompt } });
  await branch.handler("u1", edit.ctx);
  assert.deepEqual(edit.drafts, ["Add rate limiting\nwith tests"]);

  const unknown = context(undefined, "rpc", { entries: {} });
  await branch.handler("x9", unknown.ctx);
  assert.deepEqual([unknown.branches, unknown.notes], [[], ["No entry x9 to branch from"]]);

  const cancelled = context(undefined, "rpc", {
    entries: { u1: prompt },
    navigate: async () => ({ cancelled: true }),
  });
  await branch.handler("u1", cancelled.ctx);
  assert.deepEqual([cancelled.drafts, cancelled.notes], [[], ["The branch was cancelled"]]);

  const failed = context(undefined, "rpc", {
    entries: { u1: prompt },
    navigate: async () => {
      throw new Error("compacting");
    },
  });
  await branch.handler("u1", failed.ctx);
  assert.deepEqual([failed.drafts, failed.notes], [[], ["Could not branch: compacting"]]);
});

test("records request and tool timing for the Trajectory", async () => {
  let now = 1000;
  const { handlers, entries } = load(() => now);
  const ctx = context(undefined).ctx;
  const fire = (event: string, payload: unknown) => handlers.get(event)!(payload, ctx);
  // A request starts with pi's turn, before the provider answers with headers.
  await fire("turn_start", {});
  now = 1100;
  await fire("message_start", { message: { role: "user" } });
  await fire("message_start", { message: { role: "assistant" } });
  // A block starts empty; the first content arrives later.
  now = 1500;
  await fire("message_update", { assistantMessageEvent: { type: "thinking_start" } });
  now = 1900;
  await fire("message_update", { assistantMessageEvent: { type: "thinking_delta", delta: "Hm" } });
  now = 1950;
  await fire("message_update", { assistantMessageEvent: { type: "text_delta", delta: "Hi" } });
  now = 4400;
  await fire("message_end", { message: { role: "assistant" } });
  await fire("turn_start", {});
  now = 4450;
  await fire("message_start", { message: { role: "assistant" } });
  now = 4500;
  await fire("message_end", { message: { role: "assistant" } });
  await fire("tool_execution_end", { toolCallId: "c1", durationMs: 12 });
  await fire("tool_execution_end", { toolCallId: "c2" });
  assert.deepEqual(entries, [
    { customType: "watershed.timing", data: { kind: "request", firstTokenMs: 900, durationMs: 3400 } },
    { customType: "watershed.timing", data: { kind: "request", firstTokenMs: null, durationMs: 100 } },
    { customType: "watershed.timing", data: { kind: "tool", toolCallId: "c1", durationMs: 12 } },
    { customType: "watershed.timing", data: { kind: "tool", toolCallId: "c2", durationMs: null } },
  ]);
});

test("records how long an approval waited for the user", async () => {
  let now = 0;
  const { handlers, entries } = load(() => now);
  const ctx = {
    mode: "rpc",
    ui: {
      select: async () => {
        now = 9400;
        return "allow-once";
      },
    },
    sessionManager: { getSessionId: () => "s1" },
  };
  await handlers.get("tool_call")!({ toolCallId: "c9", toolName: "bash", input: {} }, ctx);
  assert.deepEqual(entries, [
    {
      customType: "watershed.approval",
      data: { toolCallId: "c9", tool: "bash", decision: "allow-once", waitedMs: 9400 },
    },
  ]);
});
