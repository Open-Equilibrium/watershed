import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [executable, ...args] = process.argv.slice(2);

if (!executable) {
  process.stderr.write("usage: node scripts/run-isolated-rust-test.mjs <executable> [args...]\n");
  process.exitCode = 2;
} else {
  const root = mkdtempSync(join(tmpdir(), "watershed-rust-test-"));
  const home = join(root, "home");
  if (process.platform !== "win32") {
    mkdirSync(home, { mode: 0o700 });
  }
  const child = spawn(executable, args, {
    env: { ...process.env, FLOW_AGENT_HOME: home },
    stdio: "inherit",
  });
  const forwardedSignals = ["SIGINT", "SIGTERM"];
  const signalHandlers = new Map(
    forwardedSignals.map((signal) => [
      signal,
      () => {
        if (!child.killed) {
          child.kill(signal);
        }
      },
    ]),
  );
  for (const signal of forwardedSignals) {
    process.on(signal, signalHandlers.get(signal));
  }

  let spawnFailed = false;
  child.on("error", (error) => {
    spawnFailed = true;
    process.stderr.write(`${executable}: ${error.message}\n`);
  });
  child.on("close", (code, signal) => {
    for (const forwardedSignal of forwardedSignals) {
      process.off(forwardedSignal, signalHandlers.get(forwardedSignal));
    }
    try {
      rmSync(root, { recursive: true, force: true });
    } catch (error) {
      process.stderr.write(`failed to remove isolated Rust test home: ${error.message}\n`);
      process.exitCode = 1;
      return;
    }
    if (spawnFailed) {
      process.exitCode = 1;
    } else if (signal === "SIGINT") {
      process.exitCode = 130;
    } else if (signal === "SIGTERM") {
      process.exitCode = 143;
    } else {
      process.exitCode = code ?? 1;
    }
  });
}
