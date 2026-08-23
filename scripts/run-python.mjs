import { spawnSync as defaultSpawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const PYTHON_THREE_PROBE_ARGS = [
  "-c",
  "import sys; raise SystemExit(sys.version_info.major != 3)",
];

function writeError(stderr, message) {
  stderr.write(`${message}\n`);
}

function pythonCandidates(platform, args) {
  return platform === "win32"
    ? [
        {
          executable: "py",
          args: ["-3", ...args],
          probeArgs: ["-3", ...PYTHON_THREE_PROBE_ARGS],
          missingName: "py -3",
        },
        { executable: "python3", args, probeArgs: PYTHON_THREE_PROBE_ARGS },
        { executable: "python", args, probeArgs: PYTHON_THREE_PROBE_ARGS },
      ]
    : [
        { executable: "python3", args, probeArgs: PYTHON_THREE_PROBE_ARGS },
        { executable: "python", args, probeArgs: PYTHON_THREE_PROBE_ARGS },
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
    const probe = spawnSync(candidate.executable, candidate.probeArgs, { stdio: "ignore" });
    if (probe.error?.code === "ENOENT") {
      missing.push(candidate.missingName ?? candidate.executable);
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
      missing.push(candidate.missingName ?? candidate.executable);
      continue;
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

  writeError(stderr, `missing Python 3 interpreter: tried ${missing.join(", ")}`);
  return 127;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = runPython(process.argv.slice(2));
}
