---
name: clawpatch
description: "Final runtime package review gate after autoreview and before PR."
---

Obey AGENTS.md.

# Clawpatch Gate

This skill is the procedural home for the clawpatch sequence only.

Run in the project root without interleaving unrelated commands.

1. Once per checkout: `pnpm install`, then `pnpm exec clawpatch doctor`.
2. First run per repo: `pnpm exec clawpatch init`, then `pnpm exec clawpatch map`; rerun `map` after structural changes.
3. Review open work: `pnpm exec clawpatch review --limit <n> --jobs 3` (`n` = number of features), then `pnpm exec clawpatch report`.
4. Inspect one finding at a time with `next` / `show --finding <id>`.
5. Findings are advisory: verify each against real code paths, adjacent tests, cross-references and downstream effects. Triage false positives with a concrete note `triage --finding <id> --status false-positive --note "<real reason>"`; fix real findings under the AGENTS.md red/green rule, then `revalidate --finding <id>`.
6. Commit stable state using the `git` skill.
7. Repeat until `pnpm exec clawpatch revalidate --all --status open` has no open actionable findings, unless the open search-space stop below triggers.

## Open Search-Space Stop

Apply the canonical [closeout open search-space stop](../../../AGENTS.md#closeout-open-search-space-stop). If it triggers:

- Do not run another `review`, `revalidate`, or variant search.
- Report clawpatch `BLOCKED` and stop immediately.

## Hard Rules

- Repository agent setup (`AGENTS.md`, `.codex/**` and `.agents/**`) is outside Clawpatch scope. Do not map, review, triage or revalidate it; a change limited to that setup makes Clawpatch not applicable.
- Never edit `.clawpatch/` by hand!
- Never weaken tests!
- If a finding implies an undecided architectural/product question, stop and record it per `AGENTS.md` instead of fixing by guesswork.
