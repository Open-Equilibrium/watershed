---
name: git
description: "Git workflow discipline: the topic-branch model (branch off main, PR back to main), stable-state commits, squash-ready PR bodies with DCO sign-off, no AI/agent references, and the mandatory autoreview -> clawpatch -> doc-sync closeout before a PR. Use whenever a branch is created, work is committed, or a change is prepared for PR."
---

# Git workflow

## Branching model

`main` is **protected**: PR-only, no direct pushes, all CI checks must pass and the maintainer must approve before merge (GitHub ruleset `protect-main`). Protection is mandatory even in private repos; if it is missing, stop and report instead of pushing to `main`. Everything else is a short-lived branch off `main`.

- **Branch** `<type>/<scope>-<summary>` (e.g. `feat/proto-event-envelope`; `type` in `feat|fix|docs|test|ci|chore|refactor`) — one per topic (can be multiple codex sessions) / logical change, cut from an up-to-date `main`. PRs go branch -> `main`.
- Create topic branches from local `main` after `git merge --ff-only origin/main`; do not create them from `origin/main`, which can make the topic branch track `origin/main`.
- Topic branches must not track `origin/main`. If `git rev-parse --abbrev-ref --symbolic-full-name '@{u}'` resolves to `origin/main`, unset or repoint the upstream before more work.

No long-lived integration branches: a single target (`main`) minimizes overhead, and Watershed's milestones are sequential, so there is no parallel-milestone integration to host.

## Who branches what

- **Main-thread Codex session:** at the start of a logical change, cut a topic branch from an up-to-date `main` and work there. One topic = one branch = one logical change. Keep it current by merging `main` in.
- **Subagents** (`autoreview_runner`, `clawpatch_runner`, ...): never create, switch, or push branches. They commit stable fix states on the **current** topic branch and return commit SHAs; branch/PR control stays with the main-thread session.

## When to commit

Commit every stable state: code compiles, `cargo fmt --check` and `cargo clippy` are clean, affected tests are green, docs are in sync (AGENTS.md Definition of Done). Never commit a broken state; never batch unrelated changes into one commit.

## How to commit

- Use `git commit -s` for new commits by default (DCO sign-off, see CONTRIBUTING.md). Author/committer stay the configured git user.
- PRs are squash-merged. The PR body/squash message is the canonical final `main` commit message: it must stand alone with concise what/why, relevant issue/decision/ADR references, validation evidence and `Signed-off-by:` trailer(s) for all contributors.
- DCO automation on the target repo is authoritative. If CI checks per-commit sign-offs, satisfy that gate; the PR body/squash message is not a workaround for a failing DCO check.
- Branch commit subjects are imperative, scoped like `proto: add v0 event envelope` and <= 72 chars. Bodies should state what + why and reference issue/decision/ADR IDs when practical.
- Do not rewrite otherwise-correct branch commits solely to add missing DCO trailers or decision/ADR references when the squash PR body covers them, unless a maintainer or CI per-commit gate requires it.
- Unpublished commit metadata may be amended/reworded only to correct subject/title, body/message, comment text or trailers. Never amend to add, remove or change source/docs/features; create a new commit for content changes. Never rewrite `main` history.
- **No AI/agent/tool references anywhere**: no Codex/assistant mentions, no `Co-Authored-By` bots, no "Generated with" trailers — in branch names, commit messages, or PR text.
- **Clean and minimal text, no bloat**.

## PR-ready closeout (mandatory, in this order)

1. Full test suite (`cargo nextest run`), coverage >=95% lines from M1 (`cargo llvm-cov nextest --workspace --fail-under-lines 95`), the dependency-hygiene gate (`cargo audit`/`cargo deny`), the `lychee` docs link gate and the current milestone's `PERFORMANCE.md` budget checks pass. Run the full matrix via the `pr_validator` subagent; routine fmt/clippy/nextest may run in the main thread during the tdd loop.
2. Run the `autoreview` skill on the branch diff (`--mode branch --base origin/main`) per its `SKILL.md` — normally via the `autoreview_runner` subagent; verify and fix accepted findings, **commit each stable fix state**, rerun until clean.
3. Run the `clawpatch` skill (final gate, own terminal) per its `SKILL.md` — normally via the `clawpatch_runner` subagent; fix/triage findings, **commit each stable fix state**, rerun until no open actionable findings remain.
4. Draft the PR body in a local untracked file such as `.codex-logs/pr-body.md`. Confirm every box in `.github/PULL_REQUEST_TEMPLATE.md` is satisfiable. The draft must be squash-ready: it is the future `main` commit body and must cover the relevant content, decision/ADR references, validation evidence and DCO trailers for all contributors.
5. Run the `doc_sync` subagent (read-only): confirm documentation + PR/commit standards were followed (minimal/non-duplicating docs, GLOSSARY terms, DCO sign-off via the squash-ready PR body, no AI/agent references, decision/ADR references); fix any finding and recommit.
6. Open the PR **against `main`** with GitHub CLI (`gh`) using the reviewed PR body file. Include a meaningful, concise title and description — same rules as commits — with evidence of changes and validation.
7. Verify GitHub readiness with `gh`: fetch `origin/main`, confirm the branch is current with the PR base, and confirm all required/relevant checks are passing before declaring the PR ready. Use `gh pr view --json mergeStateStatus,mergeable,statusCheckRollup,headRefOid,baseRefOid` and `gh pr checks --watch` as the baseline. Also run `gh pr checks --required --watch` when branch protection reports required checks; if GitHub reports no required checks, still require every relevant workflow in the check rollup to pass. Inspect the latest run with `gh run view <run-id> --json jobs` and keep the PR validation checklist aligned with the actual job steps: every listed validation item must be passed, or have an explicit local/environment caveat backed by a passing CI step. If the branch is behind, update it by merging `origin/main` into the topic branch when clean; if that would conflict or require rewriting published history, stop and report that the branch needs a maintained update path.

## GitHub CLI

- Use `gh auth status` before PR work; if `gh` is missing or unauthenticated, install/authenticate it or report the exact blocker.
- Before any push, verify the current branch is not `main` and its upstream is not `origin/main`.
- Push the current branch with an explicit non-main refspec: `git push -u origin HEAD:refs/heads/<branch>`; never push `main`.
- Create/view PRs with `gh pr create --base main --head <branch> --body-file <reviewed-body-file>` and `gh pr view`.
- Do not use interactive prompts for PR creation when flags can make the command reproducible.

## Rules

- Never merge yourself; never push to `main` directly.
- Never compensate for missing branch protection with process discipline alone; stop and have protection added.
- Never amend for new docs/features/content; metadata-only amendments are allowed only as described above, and should not replace a complete squash-ready PR body.
- Do not create a new branch if there already is a branch that fits to the current tasks context.
- Agent branches/PRs target `main` via a reviewed, CI-gated PR.
