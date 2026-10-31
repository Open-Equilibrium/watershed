import { spawnSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";

function git(...args) {
  const result = spawnSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) throw result.error;
  return result;
}

// Repository agent setup runs no gates and documentation only the documentation gates.
const SETUP = [":(top,exclude)AGENTS.md", ":(top,exclude).agents/**", ":(top,exclude).codex/**"];
const DOCUMENTATION = /\.(md|html)$/;
// A product's directories select its gates and those of its dependents: liquid_bridge
// builds on meta-harness-core. Any other path, and any Cargo manifest because it can
// change the features other members build with, selects every product.
const PRODUCTS = ["flow_agent", "meta_harness", "liquid"];
const OWNERS = [
  ["flow-agent/", ["flow_agent"]],
  ["core/", ["flow_agent"]],
  ["proto/", ["flow_agent"]],
  ["install/", ["flow_agent"]],
  ["meta-harness/", ["meta_harness", "liquid"]],
  ["liquid/", ["liquid"]],
];

function changed(base) {
  const diff = git("diff", "--name-only", "--no-renames", "-z", base, "HEAD", "--", ".", ...SETUP);
  if (diff.status !== 0) throw new Error("Cannot compare CI product changes");
  return diff.stdout.split("\0").filter(Boolean);
}

function owners(path) {
  if (path === "Cargo.toml" || path.endsWith("/Cargo.toml")) return PRODUCTS;
  return OWNERS.find(([prefix]) => path.startsWith(prefix))?.[1] ?? PRODUCTS;
}

let docs = true;
let selected = new Set(PRODUCTS);
if (process.env.GITHUB_EVENT_NAME !== "workflow_dispatch") {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
  let base;
  switch (process.env.GITHUB_EVENT_NAME) {
    case "pull_request":
      base = event.pull_request?.base?.sha;
      break;
    case "push":
      base = event.before;
      if (base === "0".repeat(40)) {
        const mergeBase = git("merge-base", "HEAD", "origin/main");
        if (mergeBase.status !== 0) throw new Error("Cannot find the new branch's CI base");
        base = mergeBase.stdout.trim();
      }
      break;
    default:
      throw new Error("Unsupported CI event");
  }
  if (typeof base !== "string" || !/^[a-f0-9]{40}$/i.test(base)) {
    throw new Error("CI comparison requires a commit SHA");
  }
  const paths = changed(base);
  docs = paths.length > 0;
  selected = new Set(paths.filter((path) => !DOCUMENTATION.test(path)).flatMap(owners));
}
// Flow Agent's gates test the whole workspace; the others test the Liquid stack.
const packages = selected.has("flow_agent") ? "--workspace" : "-p meta-harness-core -p liquid_bridge";
const outputs = [
  `docs=${docs}`,
  `product=${selected.size > 0}`,
  ...PRODUCTS.map((product) => `${product}=${selected.has(product)}`),
  `rust_packages=${packages}`,
];
appendFileSync(process.env.GITHUB_OUTPUT, `${outputs.join("\n")}\n`);
