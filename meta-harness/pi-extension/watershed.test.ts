import assert from "node:assert/strict";
import { test } from "node:test";

import watershed, { APPROVAL_PREFIX } from "./watershed.ts";

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

function load() {
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>();
  const entries: { customType: string; data: unknown }[] = [];
  const tools = [{ name: "read", annotations: { readOnlyHint: true } }, { name: "bash" }];
  watershed({
    on: (event: string, handler: Handler) => handlers.set(event, handler),
    registerCommand: (name: string, options: never) => commands.set(name, options),
    appendEntry: (customType: string, data: unknown) => entries.push({ customType, data }),
    getAllTools: () => tools,
  } as never);
  return { handlers, commands, entries };
}

function context(answer: string | undefined, mode = "rpc") {
  const questions: { title: string; options: string[] }[] = [];
  const notes: string[] = [];
  const branches: unknown[][] = [];
  const ctx = {
    mode,
    ui: {
      select: async (title: string, options: string[]) => {
        questions.push({ title, options });
        return answer;
      },
      notify: (message: string) => notes.push(message),
    },
    navigateTree: async (...args: unknown[]) => {
      branches.push(args);
      return { cancelled: false };
    },
  };
  return { ctx, questions, notes, branches };
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
    });
    assert.deepEqual(options, ["allow-once", "allow-session", "reject", "allow-auto"]);
    assert.deepEqual(
      entries,
      recorded
        ? [{ customType: "watershed.approval", data: { toolCallId: "c1", tool: "bash", decision: answer } }]
        : [],
    );
  }
});

test("reports read-only tools and stays out of other modes", async () => {
  const { handlers } = load();
  const rpc = context("allow-auto");
  const read = { type: "tool_call", toolCallId: "c2", toolName: "read", input: { path: "a" } };
  await handlers.get("tool_call")!(read, rpc.ctx);
  assert.equal(JSON.parse(rpc.questions[0].title.slice(APPROVAL_PREFIX.length)).readOnly, true);
  const tui = context("reject", "tui");
  assert.equal(await handlers.get("tool_call")!(read, tui.ctx), undefined);
  assert.equal(tui.questions.length, 0);
});

test("branches the session at an entry", async () => {
  const { commands } = load();
  const branch = commands.get("watershed-branch")!;
  const labelled = context(undefined);
  await branch.handler("874681ad A", labelled.ctx);
  const plain = context(undefined);
  await branch.handler(" 6ed210af ", plain.ctx);
  const empty = context(undefined);
  await branch.handler("", empty.ctx);
  assert.deepEqual(labelled.branches, [["874681ad", { label: "A" }]]);
  assert.deepEqual(plain.branches, [["6ed210af", {}]]);
  assert.deepEqual(empty.branches, []);
  assert.deepEqual(empty.notes, ["watershed-branch needs an entry id"]);
});
