import { spawnSync } from "node:child_process";

const args = process.argv.slice(2);
if (args.length === 0) {
  console.error("usage: node scripts/run-python.mjs <python-args...>");
  process.exit(2);
}

const candidates =
  process.platform === "win32"
    ? [
        { executable: "py", args: ["-3", ...args] },
        { executable: "python3", args },
        { executable: "python", args },
      ]
    : [
        { executable: "python3", args },
        { executable: "python", args },
      ];

const missing = [];
for (const candidate of candidates) {
  const result = spawnSync(candidate.executable, candidate.args, { stdio: "inherit" });
  if (result.error?.code === "ENOENT") {
    missing.push(candidate.executable);
    continue;
  }
  if (result.error) {
    console.error(`${candidate.executable}: ${result.error.message}`);
    process.exit(1);
  }
  if (result.signal) {
    console.error(`${candidate.executable}: stopped by signal ${result.signal}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

console.error(`missing Python interpreter: tried ${missing.join(", ")}`);
process.exit(127);
