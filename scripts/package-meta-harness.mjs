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
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { crc32, deflateRawSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const PLATFORMS = ["darwin-arm64", "linux-x64", "win32-x64"];
const USAGE =
  "usage: package-meta-harness.mjs --meta <meta> --t3 <t3-<version>-<platform> folder> " +
  "--platform <darwin-arm64|linux-x64|win32-x64> --output <new folder>";
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

function workspaceVersion(cargoToml) {
  const table = cargoToml.split(/^\[/m).find((part) => part.startsWith("workspace.package]"));
  const version = table?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!SEMVER.test(version ?? "")) throw new Error("the root Cargo.toml has no [workspace.package] version");
  return version;
}

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

function files(folder) {
  return readdirSync(folder, { withFileTypes: true }).flatMap((entry) => {
    const path = join(folder, entry.name);
    if (entry.isDirectory()) return files(path);
    if (entry.isFile()) return [path];
    throw new Error(`${path} is neither a file nor a folder, which a Windows ZIP cannot hold`);
  });
}

function zip(folder, name, archive) {
  const records = [];
  const central = [];
  let offset = 0;
  for (const path of files(join(folder, name)).sort()) {
    const entry = Buffer.from(relative(folder, path).split(sep).join("/"), "utf8");
    const data = readFileSync(path);
    const packed = deflateRawSync(data);
    const checksum = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(entry.length, 26);
    const header = Buffer.alloc(46);
    header.writeUInt32LE(0x02014b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(20, 6);
    header.writeUInt16LE(0x0800, 8);
    header.writeUInt16LE(8, 10);
    header.writeUInt32LE(checksum, 16);
    header.writeUInt32LE(packed.length, 20);
    header.writeUInt32LE(data.length, 24);
    header.writeUInt16LE(entry.length, 28);
    header.writeUInt32LE(offset, 42);
    records.push(local, entry, packed);
    central.push(header, entry);
    offset += local.length + entry.length + packed.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(central.length / 2, 8);
  end.writeUInt16LE(central.length / 2, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  if (offset > 0xffffffff || central.length / 2 > 0xffff) throw new Error("the bundle needs ZIP64");
  writeFileSync(archive, Buffer.concat([...records, directory, end]));
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
  const t3Source = realpathSync(t3);
  const windows = platform === "win32-x64";
  const t3Program = windows ? "t3.exe" : "t3";
  if (!statSync(join(t3Source, t3Program)).isFile()) throw new Error(`${t3} holds no ${t3Program} program`);
  if (existsSync(output)) throw new Error(`${output} already exists`);
  mkdirSync(output);
  try {
    const name = `meta-harness-${version}-${platform}`;
    const stage = join(output, name);
    for (const [leaf, source, mode] of [
      ...(windows
        ? [["bin/meta.exe", meta, 0o755]]
        : [
            ["bin/meta", meta, 0o755],
            ["install.sh", join(ROOT, "meta-harness", "install.sh"), 0o755],
          ]),
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
    cpSync(t3Source, t3Destination, { recursive: true, verbatimSymlinks: true });
    writeFileSync(join(stage, "bundle-info"), `${version}\n${platform}\n`);

    const archive = join(output, `${name}.${windows ? "zip" : "tar.gz"}`);
    if (windows) {
      zip(output, name, archive);
    } else {
      const tar = spawnSync("tar", ["-czf", archive, "-C", output, name], {
        encoding: "utf8",
        env: { ...process.env, COPYFILE_DISABLE: "1" },
      });
      if (tar.error || tar.status !== 0) {
        throw new Error(`tar failed: ${tar.error?.message ?? tar.stderr.trim()}`);
      }
    }
    rmSync(stage, { recursive: true });
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
