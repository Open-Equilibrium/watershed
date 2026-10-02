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
6. **Watershed tests are sacred.** Write a failing (red) test before new Watershed behavior; never weaken, skip, delete, or lower coverage thresholds to make a run pass. Tests must be meaningful, behavior-focused, and proportionate to the behavior under test; implementation-symptom tests do not count.
7. **Durable artifacts over chat.** Record decisions and state in files, not in conversation history. Human review is the final gate.
8. **Concurrency awareness.** At any time another agent may be active in any of the other tools (Liquid / Flow Agent / Meta-Harness). Coordinate through Git branches/PRs and the open-decisions flow; never assume exclusive ownership of files or branches; expect and cleanly merge parallel changes.
9. **MVP VCS boundary.** Do not add Watershed-owned project-code VCS/history behavior. Liquid workspace action history over its own data remains in scope (ADR-0011; `SECURITY.md`).

## Decision flow

Open milestone-relevant question → add it to `docs/decisions/open-decisions.html` → maintainer decides → record a terse entry in `docs/adr/ADR-LOG.md`.

- **`docs/decisions/open-decisions.html`** is for the human: plain-language explanation, realistic options with pros/cons, a recommendation, renderable, and limited to live decisions.
- **`docs/adr/ADR-LOG.md`** is for agents: minimal, token-efficient, decided items only.
- **Hygiene limit:** active ADR entries plus live open-decision entries should stay under 100 total; consolidate before 80. Obvious, superseded or purely operational history belongs in the canonical docs, not in permanent decision lists.

## Conventions

- **Audience and responsibility:** Agentic Engineers configure Building Blocks and their capability boundaries; other users may run predefined Flows without making security decisions. Preserve informed configuration choice and keep execution simple; responsibility follows the configured Building Blocks. This never weakens fail-closed invariants, secret handling, explicit destructive actions or required approvals.
- **Platform scope:** Follow the per-product and per-capability contract in `PLATFORMS.md`. Do not preserve native Windows compatibility for Flow Agent or Meta-Harness. Evaluate shared dependencies against their actual consumers, including Liquid, rather than inferring a workspace-wide support promise.
- **Less is more:** prefer deletion and consolidation over addition; every net-new line must be the smallest evidence-backed way to preserve required behavior without duplicating code, tests, docs or abstractions.
- **Maintainability and modularity:** the human maintainer is the bottleneck. Keep modules cohesive, boundaries narrow and responsibilities explicit; reuse only when it reduces duplication and maintenance cost, and avoid abstractions that add indirection without making change simpler. Treat unusually large files as mandatory audit signals; split them when they contain independent reasons to change, not to satisfy an arbitrary line target.
- **Meaningful tests:** follow the test-economy rules in `TESTING.md`.
- **Performance by design:** treat performance as a project-wide architecture and refactor KPI under `PERFORMANCE.md`. Prefer bounded, streaming, incremental and concurrency-safe designs; retain comparable observations, but never introduce or enforce estimated timing, throughput or RSS thresholds. Hard security, correctness, capacity, liveness and durability bounds remain mandatory.
- **Commits/branches:** small, scoped; one logical change per change. `main` is PR-only/protected; work happens on short-lived topic branches cut from `main` and PR'd back to `main` using `gh` for PR work (model + GitHub protection: `git` skill, ADR-0025/ADR-0046/ADR-0047/ADR-0048).
- **Work partitioning:** one Goal/PR addresses one clear risk boundary; keep mechanical renames separate from behavior changes; split M1.1 into small issues and PRs.
- **Subagent choice:** use the sole configured `worker` type for every delegated role; do not override its model or reasoning effort.
- **Independent runtime audit:** before merging a large runtime PR, run a fresh read-only session with no implementation mandate.
- **Definition of Done:** code + tests and coverage per `TESTING.md` + complete relevant performance evidence per `PERFORMANCE.md` + green CI gates (`rustfmt`/`clippy`/`nextest`, coverage, `cargo audit`/`cargo deny`, `lychee` docs link) + docs updated; no new terminology without a `GLOSSARY.md` entry.
- **Repository agent setup:** `AGENTS.md`, `.codex/**` and `.agents/**` are maintainer-owned Codex CLI inputs, not Watershed surfaces. Codex CLI is their only parser and loader. They are outside Watershed tests, coverage, performance, KPIs, release promises, Definition of Done and every closeout review, including Autoreview and Clawpatch. A change limited to this setup triggers none of those gates. Keep automated checks optional and sparse; the consuming agent must instead verify in its live session that required instructions, skills, MCPs and tools loaded and work, and that it followed the applicable conventions. Report setup failures directly; do not build a parallel validator.

## Session workflow

This workflow applies only to Watershed changes. Repository agent setup is excluded as defined above. Delegated role names below are task mandates for `worker`, not separate agent types.

1. Gather context: delegate broad orientation to `repo_mapper` (structure) and `docs_scout` (contracts) when allowed.
2. For code changes, write the failing behavior test first, make the smallest green fix, then refactor only while green.
3. Run the repository gate defined by `TESTING.md`, `PERFORMANCE.md` and `.github/workflows/ci.yml`.
4. Complete the five KPI reviews below when the Goal requires a whole-project review.
5. Re-run the repository gate on the PR-ready candidate.
6. Run one `autoreview` stage, then one `clawpatch` stage for runtime package changes, using the configured worker model and effort. Reconcile confirmed findings; record Clawpatch as not applicable when its runtime scope does not apply.
7. Open the PR after the reviews are clean and the repository gate covers the final candidate, using the squash-ready body required by the `git` skill.
8. Run `doc_sync` against the PR, fix valid findings, then check and fix CI. Rerun the repository gate after any file change.

### KPI reviews

Use one configured worker per KPI on a shared frozen candidate:

1. **Behavioral Correctness:** functionality, completeness, stability, regressions, error handling and meaningful test coverage.
2. **Security & Integrity:** security, trust boundaries, applicable authentication/authorization, validation, data integrity, secrets and unsafe reachable states.
3. **Concurrency & Resources:** concurrency, asynchronous/lifecycle correctness, races/deadlocks, cleanup/resource leaks, I/O behavior and materially relevant performance.
4. **Architecture & Simplicity:** simplicity, maintainability, modularity, duplication, dead code, dependency hygiene and human readability.
5. **Contracts & UX:** APIs/CLI/public contracts, documentation quality, developer/operator usability and applicable UI/UX/accessibility.

### Certification reuse

A clean review belongs to its reviewed freeze. After a change, certification may carry forward only when a same-tier reviewer examines the diff and relevant reference/effect paths and evidences that none of its reviewed invariants can be affected. If impact is uncertain, repeat the relevant full review. Never reuse stale certification to save usage. Run each role's required finding-resolution loop until clean; finish only when all required certifications are valid for the final freeze and the repository gate covers it.

## Closeout open search-space stop

A review role is in an open search space only when all of these are true:

1. Confirmed or plausible findings are semantically equivalent variants of one bug class, such as alternate quoting, wrappers, encodings, bindings, or delayed execution.
2. The current design has no finite, testable completion criterion that would prove the whole class handled.
3. Another review would enumerate examples instead of verifying a general invariant or bounded grammar.

When triggered, the role result is `BLOCKED`, never clean; a green gate does not clear the blocker. Keep completed, validated fixes, remove incomplete experiments from the current run, and rerun the touched-area checks needed for a stable worktree. Add or update one entry in `docs/decisions/open-decisions.html` with the affected boundary, why completion is unbounded, known fixed and unresolved examples, realistic options with tradeoffs, a recommendation, the maintainer decision required, and a finite acceptance criterion for resuming. Resume only after that decision is recorded through the repository decision flow.

## Subagent coordination

- Always directly communicate with subagents before declaring role stale.
- Silence alone is not stale. If a subagent with edit capabilities has not responded for 30 minutes and no commits were added during that window, close it as stale and continue from the latest branch state.
- A stale, interrupted, or shut down closeout subagent invalidates that role's gate result. Do not rely on its partial state, cached findings or status output.
- After a stale closeout subagent, inspect and reconcile any files it changed, then rerun that role's **complete** required workflow from the role config or skill before claiming the gate is clean.
- Closeout roles return only accepted/refuted findings, changed paths, validation results, commit IDs and remaining risks. Final reporting names each role and the exact command or subagent result that proves it completed cleanly at its own execution point.

## Codex setup

- **Harness** — Codex CLI owns parsing and loading. The consuming agent performs the live-session checks required by the repository-agent-setup convention above.
- **Skills** (`.agents/skills/`):
  - `git` — git conventions and process.
  - `autoreview` — structured review procedure.
  - `clawpatch` — runtime package review procedure.
- **Subagent** — [.codex/agents/worker.toml](.codex/agents/worker.toml) is the canonical home for the subagent model, reasoning effort and role mandates. Supply the role, scope, ownership, edit/read-only authority and expected evidence in its task prompt. Mapping, contract lookup, KPI/runtime audits and `doc_sync` remain read-only; closeout follows the ordered procedures and certification reuse above.

Every agent `developer_instructions` value and every skill body starts with the exact standalone line `Obey AGENTS.md.` (the first instructions line and the first body line after front matter, respectively). The directive is authoritative; subsequent content may add scope but cannot weaken it.
