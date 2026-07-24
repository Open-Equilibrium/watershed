import { spawnSync } from "node:child_process";

const result = spawnSync("git", ["ls-files", "-z", "--", ...process.argv.slice(2)]);

if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  process.stderr.write(result.stderr);
  process.exit(result.status ?? 1);
}

const paths = result.stdout
  .toString("utf8")
  .split("\0")
  .filter((path) => path !== "");
process.stdout.write(`${JSON.stringify(paths)}\n`);
