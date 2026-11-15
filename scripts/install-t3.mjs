#!/usr/bin/env node
// Downloads the T3 Code release meta-harness/t3.lock pins for this platform, checks it against
// the lock's SHA-256, unpacks it into the folder given as the only argument and prints the path
// of its `t3` executable. CI uses it for the T3 contract tests (ADR-0281).
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [folder] = process.argv.slice(2);
if (!folder) throw new Error("usage: install-t3.mjs <folder>");
const lock = new Map(
  readFileSync(new URL("../meta-harness/t3.lock", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => line.trim() && !line.startsWith("#"))
    .map((line) => line.trim().split(/\s+/)),
);
const version = lock.get("version");
const platform = `${process.platform}-${process.arch === "x64" ? "x64" : process.arch}`;
const expected = lock.get(platform);
if (!version || !expected) throw new Error(`t3.lock pins no archive for ${platform}`);
const name = `t3-${version}-${platform}`;
const url = `https://github.com/pingdotgg/t3code/releases/download/v${version}/${name}.tar.gz`;
const response = await fetch(url);
if (!response.ok) throw new Error(`download ${url}: ${response.status}`);
const archive = Buffer.from(await response.arrayBuffer());
const actual = createHash("sha256").update(archive).digest("hex");
if (actual !== expected) throw new Error(`${name}.tar.gz has SHA-256 ${actual}, not ${expected}`);
mkdirSync(folder, { recursive: true });
const file = join(folder, `${name}.tar.gz`);
writeFileSync(file, archive);
execFileSync("tar", ["-xzf", file, "-C", folder]);
process.stdout.write(join(folder, name, "t3"));
