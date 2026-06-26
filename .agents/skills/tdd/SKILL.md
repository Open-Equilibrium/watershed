---
name: tdd
description: "Red/green TDD loop for code changes: failing test first, minimal code to green, scoped refactor. Use for any new behavior, bug fix, or contract change in Rust crates; not for docs-only or fixture-only edits."
---

# TDD

Default implementation loop for code changes. Test layers and determinism rules are defined in `TESTING.md`; budgets in `PERFORMANCE.md`.

Before starting a change, **orient by delegation** to keep the main thread lean: `repo_mapper` for where the relevant code/tests/fixtures live, `docs_scout` for the exact contract (`PROTOCOL.md`/V-Spec/ADR) you are implementing. Then run the loop:

1. **Red.** Write the smallest failing test that pins the new behavior at the right `TESTING.md` layer (unit/integration, deterministic FSM, script/schema, event-ordering/persistence, sandbox boundary, golden loop). Run it; confirm it fails for the expected reason.
2. **Green.** Write the minimal code to pass. No speculative abstractions, no drive-by refactors, no unrelated changes.
3. **Refactor.** Only with all tests green; keep the diff scoped to the change.
4. Repeat per behavior. A bug fix starts with a failing regression test.

## Rules

- Never weaken, skip or delete a test to get green; that is a maintainer decision (AGENTS.md rules).
- Deterministic inputs only: LLM/tool outputs are mocked fixtures; no real time, network or randomness in assertions.
- Tests assert documented contracts (`PROTOCOL.md`, V-Specs, ADRs). If a contract is unclear, delegate the lookup to `docs_scout` first; if the docs do not decide it, stop per AGENTS.md rule 7.
- Closeout: `cargo fmt`, `cargo clippy`, `cargo nextest run` (affected tests) — then apply the `git` skill.
