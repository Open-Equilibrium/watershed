## Summary

-

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo nextest run --locked --workspace --all-targets -E 'not (test(fsm_transition_p95_stays_under_m1_budget) | test(near_cap_event_validation_enforces_m1_memory_budget) | test(noop_dispatch_p95_stays_under_m1_budget) | test(hello_loop_runtime_emit_p95_stays_under_m1_budget) | test(hello_loop_resume_append_p95_stays_under_m1_budget) | test(ten_successful_fixture_loop_invocations_complete_under_m1_runtime_contract) | test(runtime_rejects_duplicate_subloop_work_over_m1_budget))'`
- [ ] `cargo nextest run --locked -p loop-agent-core --release -E 'test(fsm_transition_p95_stays_under_m1_budget) | test(near_cap_event_validation_enforces_m1_memory_budget) | test(noop_dispatch_p95_stays_under_m1_budget) | test(hello_loop_runtime_emit_p95_stays_under_m1_budget) | test(hello_loop_resume_append_p95_stays_under_m1_budget) | test(ten_successful_fixture_loop_invocations_complete_under_m1_runtime_contract) | test(runtime_rejects_duplicate_subloop_work_over_m1_budget)'`
- [ ] `cargo llvm-cov nextest --locked --workspace --fail-under-lines 95 --ignore-filename-regex '(^|[\\/])(tests?|src[\\/]tests\.rs)([\\/]|$)' -E 'not (test(fsm_transition_p95_stays_under_m1_budget) | test(near_cap_event_validation_enforces_m1_memory_budget) | test(noop_dispatch_p95_stays_under_m1_budget) | test(hello_loop_runtime_emit_p95_stays_under_m1_budget) | test(hello_loop_resume_append_p95_stays_under_m1_budget) | test(ten_successful_fixture_loop_invocations_complete_under_m1_runtime_contract) | test(runtime_rejects_duplicate_subloop_work_over_m1_budget))'`
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
