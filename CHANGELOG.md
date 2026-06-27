# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]
### Added
- Governance and spec set: README, VISION, PLAN, PROTOCOL, SECURITY, TESTING, PERFORMANCE, GLOSSARY, AGENTS, V-Spec concept files, open-decisions dashboard, ADR log (ADR-0001…ADR-0051).
- OSS hygiene: Code of Conduct, issue/PR templates, security reporting policy.
- Local project harness and closeout helpers, with canonical behavior in `AGENTS.md`, `package.json` and ADR-0023…ADR-0027.
- M0 validation gates made mandatory; see `TESTING.md`, `SECURITY.md` and ADR-0021.
- Test-coverage gate: ≥95% line coverage via `cargo llvm-cov`, enforced from M1 (ADR-0022).
- Git branching model (ADR-0025/ADR-0046/ADR-0047/ADR-0048): protected `main` with topic branches off `main`; PR work uses `gh`; topic branches must not track `origin/main`; commit metadata amendments are limited to unpublished commits.
- ADR-0028: Loop Agent build strategy & external-agent reuse — orchestration built in-house, non-differentiating plumbing reused via general-purpose crates, Codex CLI integrated as a Meta-Harness adapter (with Claude Code/Pi Agent), not as a Loop Agent base/fork; recorded as D-042.
- `PROTOCOL.md` no-co-location contract rule (keeps the local-only MVP transport from foreclosing later remote topologies).
- ADR-0038…ADR-0040: deployment topology, Loop Agent cloud/session durability and Liquid generative-UI scope decisions; D-034 enriched with the script-runtime trade-off triangle.
- M0 protocol, fixture, policy-artifact and Rust workspace scaffold; canonical contracts live in `PROTOCOL.md`, `TESTING.md`, `SECURITY.md` and ADR-0029…ADR-0051.

### Changed
- Local harness topology and closeout flow aligned with `AGENTS.md` and ADR-0023…ADR-0027.
- Dependency hygiene, formatting, linting, test and docs gates aligned with `TESTING.md`, `SECURITY.md` and ADR-0021.
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
