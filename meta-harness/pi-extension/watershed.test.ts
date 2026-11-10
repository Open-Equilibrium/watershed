import assert from "node:assert/strict";
import { test } from "node:test";

import { CHECKPOINT_PREFIX, watershedWith } from "./watershed.ts";

type Handler = (event: unknown, ctx: unknown) => Promise<unknown>;

function load(clock: () => number = () => 0) {
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>();
  const entries: { customType: string; data: unknown }[] = [];
  watershedWith(clock)(
    {
    on: (event: string, handler: Handler) => handlers.set(event, handler),
    registerCommand: (name: string, options: never) => commands.set(name, options),
    appendEntry: (customType: string, data: unknown) => entries.push({ customType, data }),
  } as never,
    clock,
  );
  return { handlers, commands, entries };
}

function context(
  mode = "rpc",
  session: { entries?: Record<string, unknown>; navigate?: () => Promise<{ cancelled: boolean }> } = {},
  inputs: (string | undefined)[] = [],
) {
  const asked: string[] = [];
  const notes: string[] = [];
  const branches: unknown[][] = [];
  const drafts: string[] = [];
  const aborted: boolean[] = [];
  const ctx = {
    mode,
    abort: () => aborted.push(true),
    ui: {
      input: async (title: string) => {
        asked.push(title);
        return inputs.shift();
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
  return { ctx, notes, branches, drafts, asked, aborted };
}

test("branches the session at an entry", async () => {
  const { commands } = load();
  const branch = commands.get("watershed-branch")!;
  const labelled = context();
  await branch.handler("874681ad Rate  limit A", labelled.ctx);
  const plain = context();
  await branch.handler(" 6ed210af ", plain.ctx);
  const empty = context();
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
  const edit = context("rpc", { entries: { u1: prompt } });
  await branch.handler("u1", edit.ctx);
  assert.deepEqual(edit.drafts, ["Add rate limiting\nwith tests"]);

  const unknown = context("rpc", { entries: {} });
  await branch.handler("x9", unknown.ctx);
  assert.deepEqual([unknown.branches, unknown.notes], [[], ["No entry x9 to branch from"]]);

  const cancelled = context("rpc", {
    entries: { u1: prompt },
    navigate: async () => ({ cancelled: true }),
  });
  await branch.handler("u1", cancelled.ctx);
  assert.deepEqual([cancelled.drafts, cancelled.notes], [[], ["The branch was cancelled"]]);

  const failed = context("rpc", {
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
  const ctx = context().ctx;
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

test("takes a checkpoint at each run's start and end and records it in the session", async () => {
  const { handlers, entries } = load();
  const answers = [
    '{"ok":true,"canRun":true}',
    '{"ok":true,"files":3,"added":12,"removed":1}',
    '{"ok":true,"canRun":true}',
    '{"error":"git did not finish within 60 s"}',
  ];
  const { ctx, asked } = context("rpc", {}, answers);
  assert.equal(await handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "Hi" }, ctx), undefined);
  assert.equal(asked.length, 1);
  assert.ok(asked[0].startsWith(CHECKPOINT_PREFIX));
  const start = JSON.parse(asked[0].slice(CHECKPOINT_PREFIX.length));
  assert.match(start.turn, /^[A-Za-z0-9_-]{1,128}$/);
  assert.deepEqual(start, { session: "s1", turn: start.turn, phase: "start" });

  await handlers.get("agent_settled")!({ type: "agent_settled", aborted: false }, ctx);
  assert.deepEqual(JSON.parse(asked[1].slice(CHECKPOINT_PREFIX.length)), { ...start, phase: "end" });
  // A settled run without a start asks nothing more.
  await handlers.get("agent_settled")!({ type: "agent_settled", aborted: false }, ctx);
  assert.equal(asked.length, 2);

  // The next run is a new turn; a failed checkpoint keeps the core's reason.
  await handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "Again" }, ctx);
  const next = JSON.parse(asked[2].slice(CHECKPOINT_PREFIX.length));
  assert.notEqual(next.turn, start.turn);
  await handlers.get("agent_settled")!({ type: "agent_settled", aborted: false }, ctx);
  assert.deepEqual(entries, [
    { customType: "watershed.checkpoint", data: start },
    { customType: "watershed.checkpoint", data: { ...start, phase: "end", files: 3, added: 12, removed: 1 } },
    { customType: "watershed.checkpoint", data: next },
    {
      customType: "watershed.checkpoint",
      data: { ...next, phase: "end", error: "git did not finish within 60 s" },
    },
  ]);
});

test("records a checkpoint the core did not answer as failed and asks nothing outside RPC", async () => {
  const { handlers, entries } = load();
  const { ctx, aborted } = context("rpc", {}, [undefined]);
  await handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "Hi" }, ctx);
  await handlers.get("agent_settled")!({ type: "agent_settled", aborted: true }, ctx);
  assert.deepEqual(
    entries.map(({ data }) => (data as { error?: string }).error),
    ["No answer from Watershed"],
  );
  assert.deepEqual(aborted, [true]);

  const outside = load();
  const interactive = context("interactive");
  await outside.handlers.get("before_agent_start")!({ type: "before_agent_start", prompt: "Hi" }, interactive.ctx);
  await outside.handlers.get("agent_settled")!({ type: "agent_settled", aborted: false }, interactive.ctx);
  assert.deepEqual([outside.entries, interactive.asked], [[], []]);
});

test("refuses an unguarded start and tells why, even if the project is repaired before settling", async () => {
  for (const answer of [
    '{"error":"bad config","canRun":false}',
    '{"error":"checkout lock unavailable","canRun":false}',
    '{"error":"unknown guard state"}',
    'not JSON',
    '{"ok":true}',
  ]) {
    const { handlers } = load();
    const { ctx, aborted, notes, asked } = context("rpc", {}, [answer]);
    await handlers.get("before_agent_start")!({}, ctx);
    assert.deepEqual(aborted, [true], answer);
    assert.match(notes[0], /^Could not start:/);
    // Repairing the core cannot revive this run or ask for an end checkpoint.
    await handlers.get("agent_settled")!({}, ctx);
    assert.equal(asked.length, 1);
  }
});

test("keeps running after a guarded snapshot failure or outside Git", async () => {
  for (const error of ["snapshot failed", "not inside a Git working tree"]) {
    const { handlers } = load();
    const { ctx, aborted, asked } = context("rpc", {}, [
      JSON.stringify({ error, canRun: true }), '{"error":"no snapshot"}',
    ]);
    await handlers.get("before_agent_start")!({}, ctx);
    assert.deepEqual(aborted, []);
    await handlers.get("agent_settled")!({}, ctx);
    assert.equal(asked.length, 2);
  }
});

test("refuses startup when the checkpoint dialog rejects", async () => {
  const { handlers } = load();
  const { ctx, aborted, notes } = context();
  ctx.ui.input = async () => { throw new Error("core disconnected"); };
  await handlers.get("before_agent_start")!({}, ctx);
  assert.deepEqual(aborted, [true]);
  assert.deepEqual(notes, ["Could not start: core disconnected"]);
});

test("a failing end dialog records its error and ends the run", async () => {
  const { handlers, entries } = load();
  const { ctx } = context("rpc", {}, ['{"ok":true,"canRun":true}']);
  await handlers.get("before_agent_start")!({}, ctx);
  let asks = 0;
  ctx.ui.input = async () => {
    asks += 1;
    throw new Error("core disconnected");
  };
  await handlers.get("agent_settled")!({}, ctx);
  await handlers.get("agent_settled")!({}, ctx);
  const [start, end] = entries.map(({ data }) => data as Record<string, unknown>);
  assert.equal(entries.length, 2, "the failed end is the run's last checkpoint");
  assert.deepEqual(end, { ...start, phase: "end", error: "core disconnected" });
  assert.equal(asks, 1, "a settled run asks nothing once its end failed");
});
