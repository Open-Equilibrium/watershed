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
const DOCUMENTATION = [":(top,exclude,glob)**/*.md", ":(top,exclude,glob)**/*.html"];

function changed(base, exclusions) {
  const diff = git("diff", "--quiet", base, "HEAD", "--", ".", ...exclusions);
  if (diff.status !== 0 && diff.status !== 1) {
    throw new Error("Cannot compare CI product changes");
  }
  return diff.status === 1;
}

let product = true;
let docs = true;
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
  docs = changed(base, SETUP);
  product = changed(base, [...SETUP, ...DOCUMENTATION]);
}
appendFileSync(process.env.GITHUB_OUTPUT, `product=${product}\ndocs=${docs}\n`);
