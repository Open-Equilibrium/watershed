# AGENTS.md

## Documentation policy (token efficiency)

1. **Minimal** — write the least text that fully conveys the point; no filler.
2. **Clean** — one clear structure per file; prune anything outdated on edit.
3. **Non-overlapping** — exactly one canonical home per topic; **reference, never duplicate** (see "Canonical references" below).
4. **Single source of truth** — if information belongs in another file, link to it instead of restating it.
5. **Smallest sufficient read** — structure docs so an agent can answer from the smallest relevant file; keep new docs lean and split only when a topic earns its own home. These rules apply to every file an agent creates or edits, including this one.

## V-Spec exception

The visual V-Spec files under `docs/concept/` optimize for information quality and implementation clarity over token efficiency. Keep them accurate, navigable and self-contained; do not duplicate canonical policy/spec text when a reference is enough.

## Hard rules (non-negotiable)

1. **Never assume. Always ask or record.** If anything is ambiguous or underspecified, stop and ask the maintainer. If the task explicitly permits documentation edits, record the uncertainty in `docs/decisions/open-decisions.html` instead of filling the gap with guesses.
2. **Never decide for the maintainer.** Architectural, product, naming, dependency, licensing and design choices are the maintainer's to make unless the maintainer has explicitly decided them in the current task.
3. **ADRs require prior approval.** Do not write an ADR for a decision that has not been explicitly approved by the maintainer. Propose first (see rule 4), record only after approval.
4. **Maintain the open-decisions dashboard.** For every live open decision that blocks the current or next milestone, add an entry to `docs/decisions/open-decisions.html` with: a plain-language explanation (assume the reader is _not_ an expert in that area), realistic options with lay pros/cons, and a clear recommendation. The file MUST remain valid, self-contained HTML that renders correctly after every change.
5. **Concurrency awareness.** At any time another agent may be active in any of the other tools (Liquid / Loop Agent / Meta-Harness). Coordinate through Git branches/PRs and the open-decisions flow; never assume exclusive ownership of files or branches; expect and cleanly merge parallel changes.
6. **English only.** All code, comments, docs and identifiers in English.
7. **Blocked = stop and report.** In autonomous runs (Codex goal mode included), if an open question blocks progress: stop working, do not work around it, and return the exact list of decisions to be made — existing IDs from `docs/decisions/open-decisions.html` where they exist, otherwise a new entry per rule 4.
8. **MVP VCS boundary.** Do not add **project-code** VCS/history-engine behavior (version control over arbitrary software projects, e.g. Git/Jujutsu-style commit/branch/history management) to Loop Agent, Meta-Harness or Liquid. The MVP works inside normal Git projects; project-code VCS/history questions are deferred until after the MVP. This does **not** restrict Liquid's internal **workspace action history / workspace VCS** over its own workspace data (Pages, Blocks, Views, Connections, Sources and settings), which is an in-scope Liquid product responsibility — see `docs/concept/V-Spec_Liquid.html`.

## Decision flow

Open milestone-relevant question → add to `open-decisions.html` (human-facing, plain language) → maintainer decides → record a terse entry in `docs/adr/ADR-LOG.md` (agent-facing).

- **`open-decisions.html`** is for the human: explanatory, layperson-friendly, renderable, and limited to live decisions.
- **`docs/adr/ADR-LOG.md`** is for agents: minimal, token-efficient, decided items only.
- **Hygiene limit:** active ADR entries plus live open-decision entries should stay under 100 total; consolidate before 80. Obvious, superseded or purely operational history belongs in the canonical docs, not in permanent decision lists.

## Repo map

```
core/  proto/  loop-agent/  meta-harness/  liquid/   (see README.md)
docs/decisions/open-decisions.html   open decisions (human dashboard)
docs/adr/ADR-LOG.md                  decided records (agents)
```

## Canonical references (do not duplicate their content)

- Terminology → `GLOSSARY.md`
- Integration model & emergent features → `VISION.md`
- Milestones & DoD → `PLAN.md`
- Performance budgets (tests must check them) → `PERFORMANCE.md`
- Inter-tool contract → `PROTOCOL.md`
- Security & sandbox model → `SECURITY.md`
- Test/eval strategy → `TESTING.md`

## Conventions

- **Platform priority:** Linux and macOS are primary; Windows compatibility remains required but must not drive cross-platform design. Evaluate dependency and build effects per target—a `cfg(windows)`-only dependency is not a Linux/macOS cost—and prefer one portable boundary unless evidence justifies a target split.
- **Less is more:** prefer deletion and consolidation over addition; every net-new line must be the smallest evidence-backed way to preserve required behavior, without duplicating code, tests, docs or abstractions.
- **Meaningful tests:** follow the test-economy rules in `TESTING.md`; protect all established behavior, including prior milestones, through distinct functional, contract, risk or regression cases—never line-by-line coverage tests.
- **Commits/branches:** small, scoped; one logical change per change. `main` is PR-only/protected; work happens on short-lived topic branches cut from `main` and PR'd back to `main` using `gh` for PR work (model + GitHub protection: `git` skill, ADR-0025/ADR-0046/ADR-0047/ADR-0048).
- **Definition of Done:** code + tests and coverage per `TESTING.md` + relevant budget checks per `PERFORMANCE.md` + green CI gates (`rustfmt`/`clippy`/`nextest`, coverage, `cargo audit`/`cargo deny`, `lychee` docs link) + docs updated; no new terminology without a `GLOSSARY.md` entry.

## Codex setup

- Project config: `.codex/config.toml` (model/sandbox/approval/web-search posture; applies when the project is trusted). ADR-0057 keeps `sandbox_workspace_write.network_access = true` for networked repo closeout while `approval_policy = "never"` and `web_search = "disabled"` remain fixed; this is contributor/agent harness configuration, not product runtime egress.
- Repo skills (`.agents/skills/`, each self-documenting in its `SKILL.md`):
  - `tdd` — red/green implementation loop; default for all code changes.
  - `git` — branching model (topic branches off `main`), stable-state commits, squash-ready PR body DCO sign-off, PR-ready closeout (canonical order: tests → autoreview → clawpatch → doc-sync → PR).
  - `autoreview` — structured closeout review (vendored from openclaw/agent-skills, MIT — license in the skill folder); mandatory before a branch is declared PR-ready.
  - `clawpatch` — final PR-readiness gate (pinned dev dependency in `package.json`; dev tooling only, no product Node runtime per ADR-0001).
- Subagents (`.codex/agents/`) keep heavy/scoped work and its output out of the main thread (ADR-0023, ADR-0026). Information-gathering scanners (`repo_mapper`, `docs_scout`) run gpt-5.6-luna/medium; the `pr_validator`/`doc_sync` validators run gpt-5.6-sol/medium; edit-capable closeout agents run gpt-5.6-sol/xhigh; all return only concise summaries with evidence + references:
  - `repo_mapper` (read-only) — session-start structural orientation (layout, crates, entry points, where things live).
  - `docs_scout` (read-only) — contract/spec lookups from the canonical docs.
  - `pr_validator` (writes build artifacts only, never source) — the full pre-PR gate matrix, run once; routine fmt/clippy/nextest stay in the main thread during the tdd loop (cheaper than a subagent round-trip).
  - `autoreview_runner` (edit) — owns the autoreview closeout loop; commits stable fixes; returns commit refs.
  - `clawpatch_runner` (edit) — owns the clawpatch gate loop; commits stable fixes; returns commit refs.
  - `doc_sync` (read-only) — pre-PR audit that documentation + PR/commit standards were followed.
  - `ui_validator` (Playwright) — planned, deferred until Liquid UI implementation (M3); not yet created.
- Delegation points: **session start** — `repo_mapper` (structure) + `docs_scout` (relevant contracts) gather context; **during the tdd loop** — `docs_scout` resolves contract questions before any guess; **closeout** — `pr_validator` → `autoreview_runner` → `clawpatch_runner` → `doc_sync` (see the `git` skill).
- Subagent liveness: never infer staleness from elapsed time or quiet output; slow hardware and long checks are expected. Leave about 20 minutes between routine status requests, then ask the subagent directly and wait for its reply; ask sooner only for a concrete coordination need.
- Subagents are bound by these AGENTS.md standards (English-only, the hard rules, the doc policy): each agent's `developer_instructions` reference them and read this file on demand, and the closeout chain (`autoreview` → `clawpatch` → `doc_sync`) enforces them before any PR (ADR-0027). Codex does not document whether subagents inherit `AGENTS.md`, so compliance is set explicitly, not assumed.
- Codex hooks (`.codex/hooks.json`; `[features] hooks`, canonical — the old `codex_hooks` key is deprecated): opt-in, defense-in-depth lifecycle guards (ADR-0024). EXPERIMENTAL per OpenAI, advisory only, and never a security boundary.
- Local dev tools (contributors/agents): Rust toolchain (rustfmt, clippy), `cargo-nextest`, `cargo-llvm-cov`, `cargo-audit`, `cargo-deny`, `lychee` (see `TESTING.md`/`SECURITY.md`), Python 3, pnpm (`pnpm install` provides clawpatch), `gh` for PR work.
