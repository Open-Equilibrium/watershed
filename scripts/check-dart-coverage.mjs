import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The TESTING.md coverage minimum, shared with the Rust gates.
const MINIMUM = 88;
// Generated code and the FFI-backed session glue are not measured, as Rust excludes its FFI glue.
const EXCLUDED = [/^lib\/src\/rust\//, /\.g\.dart$/, /^lib\/src\/session\/backend\.dart$/];
const DEFAULT_LCOV = fileURLToPath(new URL("../liquid/app/coverage/lcov.info", import.meta.url));

const lcov = process.argv[2] ?? DEFAULT_LCOV;
function fail(message) {
  console.error(message);
  process.exit(1);
}
function counter(text) {
  if (!/^\d+$/.test(text)) fail(`invalid Dart coverage counter ${text} in ${lcov}`);
  return Number(text);
}
let path = null;
let linesFound = null;
let linesHit = null;
let found = 0;
let hit = 0;
for (const line of readFileSync(lcov, "utf8").split("\n")) {
  if (line.startsWith("SF:")) path = line.slice(3);
  else if (line.startsWith("LF:")) linesFound = counter(line.slice(3));
  else if (line.startsWith("LH:")) linesHit = counter(line.slice(3));
  else if (line === "end_of_record") {
    if (path === null || linesFound === null || linesHit === null || linesHit > linesFound) {
      fail(`malformed Dart coverage record for ${path ?? "an unnamed file"} in ${lcov}`);
    }
    if (!EXCLUDED.some((pattern) => pattern.test(path))) {
      found += linesFound;
      hit += linesHit;
    }
    path = null;
    linesFound = null;
    linesHit = null;
  }
}
if (path !== null) fail(`unterminated Dart coverage record for ${path} in ${lcov}`);

if (found === 0) {
  console.error(`no Dart lines were measured in ${lcov}`);
  process.exit(1);
}
// Compare exactly; the printed value is truncated so it never rounds up to the minimum.
const percent = (Math.floor((hit * 10000) / found) / 100).toFixed(2);
console.log(`Dart line coverage ${percent}% (minimum ${MINIMUM}%)`);
if (hit * 100 < MINIMUM * found) {
  console.error(`Dart line coverage must be at least ${MINIMUM}%`);
  process.exit(1);
}
