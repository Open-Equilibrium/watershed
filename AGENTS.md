# AGENTS.md

## Documentation policy

1. **Minimal** — write the least text that fully conveys the point; no filler.
2. **Clean** — one clear structure per file; prune anything outdated on edit.
3. **Non-overlapping** — exactly one canonical home per topic; **reference, never duplicate**.
4. **Single source of truth** — if information belongs in another file, link to it.
5. **Smallest sufficient read** — structure docs so an agent can answer from the smallest relevant file.
6. **Simplicity** — keep structure, code, and documentation as simple as possible; human understanding and reviewability are the bottleneck.

These rules apply to every file an agent creates or edits, including this one.

## V-Spec exception

The visual V-Spec files under `docs/concept/` optimize for information quality and implementation clarity over token efficiency. Keep them accurate, navigable and self-contained; do not duplicate canonical policy/spec text when a reference is enough.

## Hard rules (non-negotiable)

1. **English only** — all code, comments, docs and identifiers in English.
2. **Never assume. Ask or record.** If anything is ambiguous or underspecified, stop and ask the maintainer, or record the entry in `docs/decisions/open-decisions.html` (assume the reader is _not_ an expert in that area). The file MUST remain valid, self-contained HTML that renders correctly after every change. Do not fill a gap with a guess.
3. **Never decide for the maintainer.** Architectural, product, naming, dependency, licensing and design choices are the maintainer's to make unless the maintainer has explicitly decided them in the current task. ADRs require prior approval (see [Decision flow](#decision-flow)).
4. **Blocked = stop and report.** In autonomous runs, if an open question blocks progress: stop, do not work around it, and return the exact list of decisions needed including realistic options with pros/cons and recommendation.
5. **No secrets.** Never read, print, or commit credentials, tokens, keys, cookies, or `.env` files. Reference CI secrets by name only.
6. **Tests are sacred.** Write a failing (red) test before new behavior; never weaken, skip, delete, or lower coverage thresholds to make a run pass. Tests must be meaningful, behavior-focused, and proportionate to the behavior under test; implementation-symptom tests do not count.
7. **Durable artifacts over chat.** Record decisions and state in files, not in conversation history. Human review is the final gate.
8. **Concurrency awareness.** At any time another agent may be active in any of the other tools (Liquid / Loop Agent / Meta-Harness). Coordinate through Git branches/PRs and the open-decisions flow; never assume exclusive ownership of files or branches; expect and cleanly merge parallel changes.
9. **MVP VCS boundary.** Do not add Watershed-owned project-code VCS/history behavior. Liquid workspace action history over its own data remains in scope (ADR-0011; `SECURITY.md`).

## Decision flow

Open milestone-relevant question → add it to `docs/decisions/open-decisions.html` → maintainer decides → record a terse entry in `docs/adr/ADR-LOG.md`.

- **`docs/decisions/open-decisions.html`** is for the human: plain-language explanation, realistic options with pros/cons, a recommendation, renderable, and limited to live decisions.
- **`docs/adr/ADR-LOG.md`** is for agents: minimal, token-efficient, decided items only.
- **Hygiene limit:** active ADR entries plus live open-decision entries should stay under 100 total; consolidate before 80. Obvious, superseded or purely operational history belongs in the canonical docs, not in permanent decision lists.

## Conventions

- **Platform priority:** Linux and macOS are primary; Windows compatibility remains required but must not drive cross-platform design. Evaluate dependency and build effects per target—a `cfg(windows)`-only dependency is not a Linux/macOS cost—and prefer one portable boundary unless evidence justifies a target split.
- **Less is more:** prefer deletion and consolidation over addition; every net-new line must be the smallest evidence-backed way to preserve required behavior without duplicating code, tests, docs or abstractions.
- **Meaningful tests:** follow the test-economy rules in `TESTING.md`; protect all established behavior, including prior milestones, through distinct functional, contract, risk or regression cases—never line-by-line coverage tests.
- **Commits/branches:** small, scoped; one logical change per change. `main` is PR-only/protected; work happens on short-lived topic branches cut from `main` and PR'd back to `main` using `gh` for PR work (model + GitHub protection: `git` skill, ADR-0025/ADR-0046/ADR-0047/ADR-0048).
- **Definition of Done:** code + tests and coverage per `TESTING.md` + relevant budget checks per `PERFORMANCE.md` + green CI gates (`rustfmt`/`clippy`/`nextest`, coverage, `cargo audit`/`cargo deny`, `lychee` docs link) + docs updated; no new terminology without a `GLOSSARY.md` entry.

## Session workflow

1. Gather context: delegate broad orientation to `repo_mapper` (structure) and `docs_scout` (contracts) when allowed.
2. For code changes, write the failing behavior test first, make the smallest green fix, then refactor only while green.
3. Run the repository gate defined by `TESTING.md`, `PERFORMANCE.md` and `.github/workflows/ci.yml`.
4. Run `autoreview_lite`, then, for runtime package changes, `clawpatch_lite`; reconcile confirmed findings while changes are still expected. Record Clawpatch as not applicable in the closeout evidence when its runtime scope does not apply.
5. Re-run the repository gate on the PR-ready candidate.
6. Run `autoreview_pro`, then, for runtime package changes, `clawpatch_pro` as the final high-assurance review tier.
7. Open the PR after the review tiers are clean, using the squash-ready body required by the `git` skill.
8. Run `doc_sync` against the PR, fix valid findings, then check and fix CI. Rerun the repository gate after any file change.

**Role-local completion:** Run each closeout role's own loop until that role returns clean. Once clean, the role remains complete: findings or fixes from later roles never restart it or any earlier role. Only the final repository gate must cover the resulting commit candidate.

## Closeout open search-space stop

A review role is in an open search space only when all of these are true:

1. Confirmed or plausible findings are semantically equivalent variants of one bug class, such as alternate quoting, wrappers, encodings, bindings, or delayed execution.
2. The current design has no finite, testable completion criterion that would prove the whole class handled.
3. Another review would enumerate examples instead of verifying a general invariant or bounded grammar.

When triggered, the role result is `BLOCKED`, never clean; a green gate does not clear the blocker. Keep completed, validated fixes, remove incomplete experiments from the current run, and rerun the touched-area checks needed for a stable worktree. Add or update one entry in `docs/decisions/open-decisions.html` with the affected boundary, why completion is unbounded, known fixed and unresolved examples, realistic options with tradeoffs, a recommendation, the maintainer decision required, and a finite acceptance criterion for resuming. Resume only after that decision is recorded through the repository decision flow.

## Subagent coordination

- Always directly communicate with subagents before declaring role stale.
- Silence alone is not stale. If a subagent with edit capabilities has not responded for 30 minutes and no commits were added during that window, close it as stale and continue from the latest branch state.
- A stale, interrupted, or shut down closeout subagent invalidates that role's gate result. Do not rely on partial state, cached findings, or status output left behind by `reviewer`, either review tier, or `doc_sync`.
- After a stale closeout subagent, inspect and reconcile any files it changed, then rerun that role's **complete** required workflow from the role config or skill before claiming the gate is clean.
- Final closeout reporting must name each closeout role and the exact command or subagent result that proves the role completed cleanly at its own execution point.

## Codex setup

- **Harness** — trusted `.codex/config.toml` keeps workspace-write network access enabled with approvals and web search disabled (ADR-0057); `.codex/hooks.json` is opt-in defense-in-depth, never a security boundary (ADR-0024).
- **Skills** (`.agents/skills/`):
  - `git` — git conventions and process.
  - `autoreview` — shared procedure for `autoreview_lite` and `autoreview_pro`.
  - `clawpatch` — shared procedure for `clawpatch_lite` and `clawpatch_pro`.
- **Subagents** (`.codex/agents/`) keep heavy, scoped work and its output out of the main thread.
  - `repo_mapper` (read-only) — session-start structural orientation.
  - `docs_scout` (read-only) — contract/spec lookups from the canonical docs.
  - `autoreview_lite` (edit) — iterative lightweight autoreview runner.
  - `clawpatch_lite` (edit) — iterative lightweight clawpatch runner.
  - `autoreview_pro` (edit) — final PR-ready autoreview.
  - `clawpatch_pro` (edit) — final PR-ready clawpatch.
  - `doc_sync` (read-only) — post-creation PR audit of doc + commit/PR standards.

Every subagent must obey this file explicitly; role files may add scope but cannot weaken it (ADR-0023/ADR-0026/ADR-0027).
