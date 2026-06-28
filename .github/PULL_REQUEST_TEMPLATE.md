## Summary

-

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo nextest run --locked --workspace --all-targets`
- [ ] `cargo llvm-cov nextest --locked --workspace --fail-under-lines 95`
- [ ] `cargo audit`
- [ ] `cargo deny check`
- [ ] `pnpm run docs:render-check`
- [ ] `lychee` documentation link check

## Checklist

- [ ] DCO `Signed-off-by:` trailers are present for all contributors in the PR body/squash message, and any DCO automation passes.
- [ ] Documentation remains minimal, non-overlapping and linked to canonical sources.
- [ ] Decisions/ADRs are referenced where relevant.
- [ ] New terminology has a `GLOSSARY.md` entry where relevant.
- [ ] No undecided architectural choices were made.
- [ ] No project-code VCS/history behavior was added to the MVP.
