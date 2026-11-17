// Packs already-built Meta-Harness parts into the download a person verifies and unpacks: `meta`,
// the T3 Code folder meta-harness/t3.lock pins, the installer and the license files (ADR-0286,
// ADR-0303). It only packs: it never builds, signs or publishes.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  createReadStream,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const PLATFORMS = ["darwin-arm64", "linux-x64"];
const USAGE =
  "usage: package-meta-harness.mjs --meta <meta> --t3 <t3-<version>-<platform> folder> " +
  "--platform <darwin-arm64|linux-x64> --output <new folder>";
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

// The workspace version: `version` in the [workspace.package] table of Cargo.toml.
function workspaceVersion(cargoToml) {
  const table = cargoToml.split(/^\[/m).find((part) => part.startsWith("workspace.package]"));
  const version = table?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!SEMVER.test(version ?? "")) throw new Error("the root Cargo.toml has no [workspace.package] version");
  return version;
}

// The T3 Code version meta-harness/t3.lock pins (ADR-0281).
function lockedT3Version(lock) {
  const version = lock.match(/^version\s+(\S+)\s*$/m)?.[1];
  if (!version) throw new Error("meta-harness/t3.lock pins no version");
  return version;
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function main(argv) {
  const { values } = parseArgs({
    args: argv,
    options: {
      meta: { type: "string" },
      t3: { type: "string" },
      platform: { type: "string" },
      output: { type: "string" },
    },
  });
  if (!values.meta || !values.t3 || !values.output || !PLATFORMS.includes(values.platform)) {
    throw new Error(USAGE);
  }
  const { platform } = values;
  const meta = resolve(values.meta);
  const t3 = resolve(values.t3);
  const output = resolve(values.output);
  const version = workspaceVersion(readFileSync(join(ROOT, "Cargo.toml"), "utf8"));
  const t3Name = `t3-${lockedT3Version(readFileSync(join(ROOT, "meta-harness", "t3.lock"), "utf8"))}-${platform}`;
  if (basename(t3) !== t3Name) {
    throw new Error(`the T3 Code folder must be named ${t3Name}, not ${basename(t3)}`);
  }
  if (!statSync(join(t3, "t3")).isFile()) throw new Error(`${t3} holds no t3 program`);
  // A prepared release is never replaced, so the output must be new.
  if (existsSync(output)) throw new Error(`${output} already exists`);
  mkdirSync(output);
  try {
    const name = `meta-harness-${version}-${platform}`;
    const stage = join(output, name);
    for (const [leaf, source, mode] of [
      ["bin/meta", meta, 0o755],
      ["install.sh", join(ROOT, "meta-harness", "install.sh"), 0o755],
      ["LICENSE", join(ROOT, "LICENSE"), 0o644],
      ["LICENSE-t3code", join(ROOT, "meta-harness", "LICENSE-t3code"), 0o644],
      ["THIRD_PARTY_NOTICES.md", join(ROOT, "THIRD_PARTY_NOTICES.md"), 0o644],
    ]) {
      mkdirSync(dirname(join(stage, leaf)), { recursive: true });
      copyFileSync(source, join(stage, leaf));
      chmodSync(join(stage, leaf), mode);
    }
    const t3Destination = join(stage, "lib", "meta-harness", t3Name);
    mkdirSync(dirname(t3Destination), { recursive: true });
    // Relative links stay relative: without verbatimSymlinks they name this machine's folders.
    cpSync(t3, t3Destination, { recursive: true, verbatimSymlinks: true });
    writeFileSync(join(stage, "bundle-info"), `${version}\n${platform}\n`);

    const archive = join(output, `${name}.tar.gz`);
    // COPYFILE_DISABLE keeps macOS tar from adding AppleDouble metadata entries.
    const tar = spawnSync("tar", ["-czf", archive, "-C", output, name], {
      encoding: "utf8",
      env: { ...process.env, COPYFILE_DISABLE: "1" },
    });
    if (tar.error || tar.status !== 0) {
      throw new Error(`tar failed: ${tar.error?.message ?? tar.stderr.trim()}`);
    }
    rmSync(stage, { recursive: true });
    // Written last, once the archive is complete.
    writeFileSync(join(output, "SHA256SUMS"), `${await sha256(archive)}  ${basename(archive)}\n`);
    return archive;
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  }
}

try {
  console.log(await main(process.argv.slice(2)));
} catch (error) {
  console.error(`package-meta-harness: ${error.message}`);
  process.exitCode = 1;
}
