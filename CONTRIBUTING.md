# Contributing

Thanks for contributing to Watershed.

## Before your first contribution

- **Sign off contributions (DCO).** Prefer `git commit -s` for new commits. Because PRs are squash-merged, the final PR/squash message must include `Signed-off-by: Name <email>` trailers for all contributors, certifying they have the right to submit the contribution ([Developer Certificate of Origin](https://developercertificate.org/)). Any DCO automation on the target repo must pass. Contributions are licensed inbound = outbound under the repository license; **no CLA is required**.
- **License.** Use the SPDX identifier `AGPL-3.0-only` exactly in package metadata and docs; the license terms are in [README](README.md#license).
- **Code of Conduct.** Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Ground rules

Follow [AGENTS.md](AGENTS.md) for language, documentation, scope, VCS, and Definition-of-Done rules. Test and performance requirements are canonical in [TESTING.md](TESTING.md) and [PERFORMANCE.md](PERFORMANCE.md).

Code carries no comments or docstrings; only generated files and shebang lines are exempt. `tests/test_no_code_comments.py` enforces this, except inside scripts embedded in strings or heredocs, which follow the rule unchecked.

## Workflow

1. Pick or open an issue; for anything that implies a decision, use `docs/decisions/open-decisions.html` first.
2. Small, scoped commits.
3. Tests per `TESTING.md`; performance-relevant changes must pass the `PERFORMANCE.md` gates.
4. Open a PR referencing the issue/decision.
