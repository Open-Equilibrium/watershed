import { spawnSync } from "node:child_process";

const args = process.argv.slice(2);
if (args.length === 0) {
  console.error("usage: node scripts/run-python.mjs <python-args...>");
  process.exit(2);
}

const missing = [];
for (const executable of ["python3", "python"]) {
  const result = spawnSync(executable, args, { stdio: "inherit" });
  if (result.error?.code === "ENOENT") {
    missing.push(executable);
    continue;
  }
  if (result.error) {
    console.error(`${executable}: ${result.error.message}`);
    process.exit(1);
  }
  if (result.signal) {
    console.error(`${executable}: stopped by signal ${result.signal}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

console.error(`missing Python interpreter: tried ${missing.join(", ")}`);
process.exit(127);
