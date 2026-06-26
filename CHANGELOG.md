# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]
### Added
- Governance and spec set: README, VISION, PLAN, PROTOCOL, SECURITY, TESTING, PERFORMANCE, GLOSSARY, AGENTS, V-Spec concept files, open-decisions dashboard, ADR log (ADR-0001…ADR-0051).
- OSS hygiene: Code of Conduct, issue/PR templates, security reporting policy.
- Codex enablement: project config (`.codex/config.toml`), repo skills (`tdd`, `git`, `clawpatch`, and the vendored `autoreview` closeout-review skill — `.agents/skills/autoreview`, MIT, from openclaw/agent-skills), a goal-mode stop rule in AGENTS.md, project subagents (in `.codex/agents/`; see ADR-0023 for the current roster), and a dev-tooling manifest (`package.json` pinning clawpatch; dev only, no product Node runtime).
- M0 CI quality-gate toolchain made mandatory: `cargo fmt --check`, `cargo clippy`, `cargo nextest run` (deterministic test runner), `cargo audit` + `cargo deny` (dependency hygiene), `lychee` docs link gate and HTML render checks (ADR-0021).
- Test-coverage gate: ≥95% line coverage via `cargo llvm-cov`, enforced from M1 (ADR-0022).
- Codex subagents `pr_validator`, `autoreview_runner`, `clawpatch_runner` and `doc_sync` (ADR-0023); opt-in Codex lifecycle hooks in `.codex/hooks.json` (PreToolUse Bash guard, Stop closeout check; experimental, Linux/macOS-only) (ADR-0024).
- Git branching model (ADR-0025/ADR-0046/ADR-0047/ADR-0048): protected `main` with topic branches off `main`; PR work uses `gh`; topic branches must not track `origin/main`; commit metadata amendments are limited to unpublished commits.
- `repo_mapper` subagent (gpt-5.4-mini, read-only) for session-start orientation; explicit agent delegation points across the dev process (ADR-0026).
- ADR-0028: Loop Agent build strategy & external-agent reuse — orchestration built in-house, non-differentiating plumbing reused via general-purpose crates, Codex CLI integrated as a Meta-Harness adapter (with Claude Code/Pi Agent), not as a Loop Agent base/fork; recorded as D-042.
- `PROTOCOL.md` no-co-location contract rule (keeps the local-only MVP transport from foreclosing later remote topologies).
- ADR-0038…ADR-0040: deployment topology, Loop Agent cloud/session durability and Liquid generative-UI scope decisions; D-034 enriched with the script-runtime trade-off triangle.
- ADR-0029…ADR-0037: M0 transport, scope, script format, sandbox depth, crate layout, fixture strategy, runtime surfaces, event schema and session store.
- D-015 fixture suite definition: `smoke-loop`, `hello-loop` and sandbox-negative fixtures with stub-model, byte-stable golden event streams.
- Canonical D-015 fixture event-stream contract, concrete v0 JSON event-envelope field rules, lowercase path-safe session IDs, runtime loop invocation IDs, canonical event JSONL serialization and default sandbox protected-path patterns for later M0 scaffold work.
- Minimum v0 building-block schema field contract, predefined-command `command_id`/literal `argv`, typed allowed parameters, one-block YAML registry container/discovery, ordered Step `connection_refs`, own-script `posix-sh` semantics, CIDR-only network egress grammar and local replay/tail/resume semantics for the append-only Loop Agent session log.
- Canonical M0 `core-policy` policy artifact path/schema/serialization, deterministic array order, direct-exec command identity, typed allowed parameters, tool-scoped capability shape, own-script runtime identity, cleared environment with non-secret allowlists plus execution-control, proxy, VCS-helper, config-injection and credential-handle denials, CIDR-only network allow entries with Linux M1 fail-closed deny-all enforcement, component-wise symlink-aware protected-path matching/grants and sandbox-negative attempt/expected-decision shape.
- Exact `connection_ids[i]`/`connection_kinds[i]` pairing for step connection payloads in the v0 event schema.
- D-047, D-048, D-049 and D-050 decided fixture discovery, trusted command-registry ownership, the HTML render gate requirement and exact render command packaging/viewport constants.
- M0 scaffold implementation: Rust workspace/toolchain policy, `proto`, `core-script`, `core-policy`, `loop-agent-core`, `loop-agent-cli`, D-015 fixture workspaces with checked-in expected JSONL streams, policy artifact fixtures and GitHub CI gate wiring.
- ADR-0049…ADR-0051: M1 Loop Agent performance budgets, deterministic M1 context scope and fail-closed Linux network enforcement.

### Changed
- Subagent topology: `validator` renamed to `pr_validator` and scoped to the one-shot pre-PR gate sweep; edit-capable closeout agents and a read-only `doc_sync` auditor added; per-agent model/effort pinned (ADR-0023).
- `commit` skill renamed to `git` and expanded with the branching model; the SessionStart hook was removed as redundant (Codex auto-loads `AGENTS.md`).
- `docs_scout` moved to gpt-5.4-mini (read-heavy contract scanner) (ADR-0026).
- Subagents explicitly bound to AGENTS.md standards by reference; enforced at closeout via `doc_sync`/`autoreview`/`clawpatch` (ADR-0027).
- Dependency hygiene switched from `cargo vet` to `cargo deny` (run with `cargo audit` as mandatory M0 CI gates); formatting/linting standardized on `cargo fmt --check`/`cargo clippy`, test runs standardized on `cargo nextest` and the `lychee` docs link + HTML render gate elevated to an M0 essential (ADR-0021).
- License identifier pinned to SPDX `AGPL-3.0-only`; contributions are DCO-only (no CLA, ADR-0015).
- Liquid performance targets rewritten as tiered, falsifiable budgets (ADR-0014).
- CLI binary names fixed: `loop`, `meta`, `liq` (ADR-0013).
- Platform framing, wedge order, layer positioning and license posture recorded as ADR-0016…ADR-0019; D-036…D-041 closed.
- Naming confirmed and recorded as ADR-0020 (repo `Open-Equilibrium/watershed`, crates.io/pub.dev free); D-003 closed.
- Loop Agent determinism framing sharpened (VISION + Loop Agent V-Spec): deterministic orchestration over a non-deterministic generator — bounded, reproducible, measurable, governable; determinism of process, not output.
- Liquid positioning clarified (absorb the long tail of personal/internal apps in a sovereign, reversible workspace; not "most apps obsolete"); audience-convergence note added to PLAN.
- Drift fixes: Liquid V-Spec performance numbers realigned to the ADR-0014 tiered budgets; ADR range in this changelog corrected to ADR-0001…ADR-0051.
- M0 documentation packet updated after ADR-0029…ADR-0037: initial M0 blockers cleared, `PROTOCOL.md` reconciled to the v0 event names/envelope, `SECURITY.md` aligned to the D-013 sandbox depth, and `TESTING.md` made canonical for the D-015 suite.
- Global Flow Agent/Flow terminology renamed to Loop Agent/Loop across docs, paths, CLI examples and runtime field names (ADR-0044).
- D-012 canonical serialization clarified as deterministic UTF-8 JSON of the schema-validated, registry-resolved building-block model.
- Decision docs cleaned up: `open-decisions.html` now lists live milestone-relevant decisions only, and `ADR-LOG.md` is a compact accepted-decision index with a 100-entry hygiene limit.
