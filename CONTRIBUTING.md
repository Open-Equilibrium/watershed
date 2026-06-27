# Contributing

Thanks for contributing to Watershed.

## Before your first contribution

- **Sign off contributions (DCO).** Prefer `git commit -s` for new commits. Because PRs are squash-merged, the final PR/squash message must include `Signed-off-by: Name <email>` trailers for all contributors, certifying they have the right to submit the contribution ([Developer Certificate of Origin](https://developercertificate.org/)). Any DCO automation on the target repo must pass. Contributions are licensed inbound = outbound under the repository license; **no CLA is required** (ADR-0015).
- **License target.** Watershed-authored files are free software under SPDX `AGPL-3.0-only` unless otherwise stated. Use that exact identifier in package metadata, file headers and docs. Vendored third-party material keeps its own license and must be listed in `THIRD_PARTY_NOTICES.md`.
- **Code of Conduct.** Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Ground rules

- **English only** — code, comments, docs, identifiers.
- **Keep docs minimal and non-overlapping** — reference, don't duplicate. The visual V-Specs are the exception: optimize them for information quality.
- **Definition of Done** — see `AGENTS.md` and `TESTING.md`.
- **MVP VCS boundary** — do not add Watershed project-history/VCS-engine behavior to MVP work. Loop Agent runs in normal Git projects but does not own VCS.

## Workflow

1. Pick or open an issue; for anything that implies a decision, use `docs/decisions/open-decisions.html` first.
2. Small, scoped commits.
3. Tests per `TESTING.md`; performance-relevant changes must pass the `PERFORMANCE.md` gates.
4. Open a PR referencing the issue/decision.
