---
name: clawpatch
description: "Run, fix, and triage clawpatch findings as the final PR-readiness gate: after the autoreview closeout is clean and before any PR is created. Covers map/review/report/triage/fix/revalidate."
---

# Clawpatch gate

clawpatch (pinned dev dependency in `package.json`) maps the repo into semantic feature slices, reviews them with the local Codex CLI as provider, and persists findings under `.clawpatch/`. It never commits, pushes, or opens PRs. Run it in its **own dedicated terminal session**; do not interleave other commands with a running review.

A branch is PR-ready only when the `autoreview` closeout is clean **and** clawpatch reports no open actionable findings.

## Sequence (after autoreview is clean, before PR creation)

1. Once per checkout: `pnpm install`, then `pnpm exec clawpatch doctor` (verifies the Codex provider).
2. First run per repo: `pnpm exec clawpatch init`, then `pnpm exec clawpatch map` (re-run `map` after structural changes).
3. `pnpm exec clawpatch review --limit <n> --jobs 3` (<n> = the number of total open findings), then `pnpm exec clawpatch report`.
4. Work findings one at a time via `next` / `show --finding <id>`:
   - Findings are advisory: verify each against the real code path first - assess cross-references and effects, do not check files in isolation!
   - False positive or intentional → `triage --finding <id> --status false-positive --note "<real reason>"`.
   - Real → `fix --finding <id>` (requires a clean worktree — commit pending stable work first), review the resulting changes yourself, run the affected tests, then `revalidate --finding <id>`.
5. After each accepted fix passes its tests, commit the stable state per the `git` skill.
6. Repeat until `pnpm exec clawpatch revalidate --all --status open` reports no open actionable findings.

## Rules

- You are not permitted to change anything inside the `.clawpatch` folder by hand!
- Never triage away a real finding - challenge your assumption before triaging; triage notes must state a real reason.
- Never let clawpatch findings bypass the tdd rules (no weakened tests).
- Sync documentation if anything needs to be updated.
- If a finding implies an undecided architectural question, stop per AGENTS.md rule 7 instead of fixing by guesswork.
