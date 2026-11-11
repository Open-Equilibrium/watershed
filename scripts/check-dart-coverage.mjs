import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The TESTING.md coverage minimum, shared with the Rust gates.
const MINIMUM = 88;
// Generated code and the FFI-backed session glue are not measured, as Rust excludes its FFI glue.
const EXCLUDED = [/^lib\/src\/rust\//, /\.g\.dart$/, /^lib\/src\/session\/backend\.dart$/];
const DEFAULT_LCOV = fileURLToPath(new URL("../liquid/app/coverage/lcov.info", import.meta.url));

const lcov = process.argv[2] ?? DEFAULT_LCOV;
let path = "";
let linesFound = 0;
let linesHit = 0;
let found = 0;
let hit = 0;
for (const line of readFileSync(lcov, "utf8").split("\n")) {
  if (line.startsWith("SF:")) path = line.slice(3);
  else if (line.startsWith("LF:")) linesFound = Number(line.slice(3));
  else if (line.startsWith("LH:")) linesHit = Number(line.slice(3));
  else if (line === "end_of_record" && !EXCLUDED.some((pattern) => pattern.test(path))) {
    found += linesFound;
    hit += linesHit;
  }
}

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
