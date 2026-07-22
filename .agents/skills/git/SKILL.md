---
name: git
description: "Git mechanics for protected-main topic branches, stable commits, ordered closeout, and squash-ready PRs."
---

# Git workflow

This skill is the procedural home for branch, commit, push and PR mechanics. The ordered quality/review closeout is canonical in `AGENTS.md`.

## Conventions

- **Branches:** `<type>/<scope>-<summary>`, with `type` in `feat|fix|docs|test|ci|chore|refactor`; one logical change per branch.
- **Commit subjects and PR titles:** imperative, scoped (for example, `runtime: bound run-state ids`) and <= 72 characters.
- **Bodies:** concise what + why, with relevant issue/decision/ADR references and validation evidence. Never batch unrelated changes.
- **No AI/agent/tool references anywhere:** not in branch names, commits or PR text; no bot co-author or generated-with trailers.
- Never expose secrets in Git or GitHub text.

## Branches

- `main` is protected by GitHub ruleset `protect-main`: PR-only, green CI and maintainer approval. If protection is missing, stop; never push to `main`.
- Reuse the current branch when it fits. Otherwise, fetch `origin`, fast-forward local `main`, then create a short-lived topic branch from local `main`.
- A topic branch must not track `origin/main`; unset or repoint that upstream before continuing. Keep current by merging `origin/main`; do not rewrite published history.
- Subagents never create, switch, merge or push branches and never open PRs. They may commit stable fixes in their assigned scope.

## Commits

- Commit each stable state after affected checks pass and docs match behavior. Do not stop for separate permission.
- Use `git commit -s`; author and committer remain the configured Git user.
- Subjects follow the conventions above. Bodies explain what + why and reference relevant issue/decision/ADR IDs.
- After a stable commit, put source/docs/content changes in a new commit. Amend unpublished commits only to correct metadata such as messages or trailers; never rewrite `main`.

## Pull requests

1. Complete the pre-PR review tiers in `AGENTS.md` before opening the PR.
2. Draft `.codex-logs/pr-body.md` and satisfy `.github/PULL_REQUEST_TEMPLATE.md`. Because PRs are squash-merged, the title/body must stand alone as the final commit message: what + why, references, validation evidence and `Signed-off-by:` trailers for every contributor. DCO automation remains authoritative.
3. Run `gh auth status`. Before pushing, verify the branch and upstream are neither `main` nor `origin/main`, and verify main protection exists.
4. Push explicitly: `git push -u origin HEAD:refs/heads/<branch>`.
5. Create explicitly: `gh pr create --base main --head <branch> --body-file .codex-logs/pr-body.md`.
6. Complete `doc_sync`, then verify mergeability and the complete check rollup with `gh pr view --json mergeStateStatus,mergeable,statusCheckRollup,headRefOid,baseRefOid` and `gh pr checks --watch`; also use `gh pr checks --required --watch` when GitHub exposes required checks. Every relevant check must pass even if none are marked required.

If the branch is behind, merge `origin/main` when clean. Stop if updating would conflict or require rewriting published history. Never merge your own PR.
