import { spawnSync as defaultSpawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const WINDOWS_LAUNCHER_PROBE_ARGS = ["-3", "-c", "import sys"];

function writeError(stderr, message) {
  stderr.write(`${message}\n`);
}

function pythonCandidates(platform, args) {
  return platform === "win32"
    ? [
        {
          executable: "py",
          args: ["-3", ...args],
          probeArgs: WINDOWS_LAUNCHER_PROBE_ARGS,
          missingName: "py -3",
        },
        { executable: "python3", args },
        { executable: "python", args },
      ]
    : [
        { executable: "python3", args },
        { executable: "python", args },
      ];
}

export function runPython(
  args,
  { platform = process.platform, spawnSync = defaultSpawnSync, stderr = process.stderr } = {},
) {
  if (args.length === 0) {
    writeError(stderr, "usage: node scripts/run-python.mjs <python-args...>");
    return 2;
  }

  const missing = [];
  for (const candidate of pythonCandidates(platform, args)) {
    if (candidate.probeArgs) {
      const probe = spawnSync(candidate.executable, candidate.probeArgs, { stdio: "ignore" });
      if (probe.error?.code === "ENOENT") {
        missing.push(candidate.missingName);
        continue;
      }
      if (probe.error) {
        writeError(stderr, `${candidate.executable}: ${probe.error.message}`);
        return 1;
      }
      if (probe.signal) {
        writeError(stderr, `${candidate.executable}: stopped by signal ${probe.signal}`);
        return 1;
      }
      if ((probe.status ?? 1) !== 0) {
        missing.push(candidate.missingName);
        continue;
      }
    }

    const result = spawnSync(candidate.executable, candidate.args, { stdio: "inherit" });
    if (result.error?.code === "ENOENT") {
      missing.push(candidate.executable);
      continue;
    }
    if (result.error) {
      writeError(stderr, `${candidate.executable}: ${result.error.message}`);
      return 1;
    }
    if (result.signal) {
      writeError(stderr, `${candidate.executable}: stopped by signal ${result.signal}`);
      return 1;
    }
    return result.status ?? 1;
  }

  writeError(stderr, `missing Python interpreter: tried ${missing.join(", ")}`);
  return 127;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = runPython(process.argv.slice(2));
}
