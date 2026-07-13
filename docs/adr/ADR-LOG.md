# ADR Log

Agent-facing index of accepted decisions. Keep detail in the canonical docs; keep this file short enough to scan before implementation.

Policy: decision flow and hygiene rules live in [AGENTS.md](../../AGENTS.md#decision-flow).

Format: `ID | date | status | decision — canonical context`

## Decisions

- **ADR-0001** | 2026-06-05 | Accepted | Loop Agent has a Rust core; no product Node runtime — `README.md`, `PERFORMANCE.md`.
- **ADR-0002** | 2026-06-05 | Accepted | Monorepo with shared `core` + `proto`, not one monolith — `README.md`.
- **ADR-0003** | 2026-06-09 | Accepted | License is SPDX `AGPL-3.0-only` — `LICENSE`, `README.md`.
- **ADR-0004** | 2026-06-05 | Accepted | Scripts define capabilities; sandbox policy enforces them — `SECURITY.md`.
- **ADR-0005** | 2026-06-05 | Accepted | Reuse proven sandbox primitives and Wasmtime plugin isolation — `SECURITY.md`.
- **ADR-0006** | 2026-06-05 | Accepted | Authored scripts are the source of truth; visual graphs are views — `SECURITY.md`, Loop Agent V-Spec.
- **ADR-0007** | 2026-06-05 | Accepted | AgentPulse belongs to Meta-Harness — `PLAN.md`, Meta-Harness V-Spec.
- **ADR-0008** | 2026-06-05 | Accepted | Meta-Agent config writes are policy-gated, audited and approval-aware — `SECURITY.md`.
- **ADR-0009** | 2026-06-05 | Accepted | Integration happens through protocol clients; Liquid is the rich UI consumer — `PROTOCOL.md`.
- **ADR-0010** | 2026-06-05 | Accepted | Project name is Watershed — `README.md`.
- **ADR-0011** | 2026-06-09 | Accepted | MVP excludes Watershed-owned project-code VCS/history behavior — `AGENTS.md`, `SECURITY.md`, `PLAN.md`.
- **ADR-0012** | 2026-06-09 | Accepted | Project owner is `Open-Equilibrium` — `README.md`.
- **ADR-0013** | 2026-06-10 | Accepted | CLI names are `loop`, `meta` and `liq` — V-Specs.
- **ADR-0014** | 2026-06-10 | Accepted | Liquid NFRs use tiered falsifiable budgets — `PERFORMANCE.md`.
- **ADR-0015** | 2026-06-10 | Accepted | Contributions are DCO-only; no CLA — `CONTRIBUTING.md`.
- **ADR-0016** | 2026-06-11 | Accepted | Product framing is one AGPL/free-software platform with three independently usable layers — `README.md`, `VISION.md`.
- **ADR-0017** | 2026-06-11 | Accepted | Adoption sequence is Loop Agent, then Meta-Harness, then Liquid — `PLAN.md`.
- **ADR-0018** | 2026-06-11 | Accepted | Layer positioning: Loop Agent execution runtime, Meta-Harness headless control plane, Liquid standalone workspace/action product — V-Specs.
- **ADR-0019** | 2026-06-11 | Accepted | Docs keep an AGPL/free-software posture and make no monetization/open-core claims — `README.md`, `VISION.md`.
- **ADR-0020** | 2026-06-11 | Accepted | Official repo target is `Open-Equilibrium/watershed`; npm is not a product target — `README.md`.
- **ADR-0021** | 2026-06-16 | Accepted | M0 gates: `cargo fmt`, `clippy`, `nextest`, `cargo audit`, `cargo deny`, `lychee`, HTML render check — `TESTING.md`.
- **ADR-0022 / ADR-0060** | 2026-06-16 / 2026-07-13 | Accepted | From M1, meaningful line coverage is gated at >=90% via `cargo llvm-cov nextest --workspace --fail-under-lines 90`; timing-sensitive perf tests run optimized outside coverage — `TESTING.md`.
- **ADR-0023** | 2026-06-16 | Accepted | Subagent topology includes `pr_validator`, closeout edit agents and `doc_sync` — `AGENTS.md`.
- **ADR-0024** | 2026-06-16 | Accepted | Codex hooks are opt-in defense-in-depth, never a security boundary — `AGENTS.md`.
- **ADR-0025** | 2026-06-16 | Accepted | `main` is PR-only/protected; work happens on topic branches — `AGENTS.md`.
- **ADR-0026** | 2026-06-16 | Accepted | `repo_mapper` and `docs_scout` gather context at defined delegation points — `AGENTS.md`.
- **ADR-0027** | 2026-06-16 | Accepted | Subagent standards compliance is explicit by reference to `AGENTS.md` — `AGENTS.md`.
- **ADR-0028** | 2026-06-18 | Accepted | Build Loop Agent orchestration in-house; reuse general plumbing crates; integrate Codex CLI as a Meta-Harness adapter, not a Loop Agent base — Loop Agent V-Spec, Meta-Harness V-Spec.
- **ADR-0029** | 2026-06-19 | Accepted | Designed control transport is local JSON-RPC over stdio; M1 implemented runtime stream is bare JSONL — `PROTOCOL.md`.
- **ADR-0030** | 2026-06-19 | Accepted | `PLAN.md` M0 pass/fail checklist is the M0 bar — `PLAN.md`.
- **ADR-0031** | 2026-06-19 | Accepted | Building-block scripts use strict YAML 1.2, schema validation, explicit registry references and canonical resolved JSON — `SECURITY.md`, Loop Agent V-Spec.
- **ADR-0032 / ADR-0052** | 2026-06-19 / 2026-06-30 | Accepted | M0 has policy artifacts and escape tests; M1 uses deterministic in-process policy enforcement/emulation; Linux Landlock/seccomp is deferred and macOS parity remains planned — `SECURITY.md`, `PLAN.md`.
- **ADR-0033** | 2026-06-19 | Accepted | Crate layout is `core/core-script`, `core/core-policy`, `proto/proto`, `loop-agent/loop-agent-core`, `loop-agent/loop-agent-cli` — `PLAN.md`.
- **ADR-0034** | 2026-06-19 | Accepted | Fixture suite is `smoke-loop`, `hello-loop` and sandbox-negative fixtures using a stub model — `TESTING.md`.
- **ADR-0035** | 2026-06-19 | Accepted | M1 ships human CLI and JSONL event stream; RPC and embedded core are designed-for seams — Loop Agent V-Spec.
- **ADR-0036** | 2026-06-19 | Accepted | Runtime event names and envelope shape are the v0 public event contract — `PROTOCOL.md`.
- **ADR-0037** | 2026-06-19 | Accepted | Session logs are append-only `.loop/sessions/<session_id>.jsonl` files with replay/tail/resume semantics — Loop Agent V-Spec.
- **ADR-0038** | 2026-06-21 | Accepted | Keep protocol seams remote-capable now; defer remote implementation — `PROTOCOL.md`.
- **ADR-0039** | 2026-06-21 | Accepted | Remote/session durability is replication plus durable storage, not container lifetime — Loop Agent V-Spec.
- **ADR-0040** | 2026-06-21 | Accepted | Liquid MVP uses a fixed component palette plus script-as-compute; sandboxed custom UI is post-MVP — Liquid V-Spec.
- **ADR-0041** | 2026-06-21 | Accepted | Fixture workspaces use checked-in `.loop/config.yaml` to select registry root and stub model profile — `TESTING.md`.
- **ADR-0042** | 2026-06-21 | Accepted | Trusted core code owns the predefined-command registry; loop YAML may only reference command IDs — `SECURITY.md`.
- **ADR-0043** | 2026-06-21 | Accepted | CI renders self-contained HTML docs in Chromium at fixed desktop/mobile viewports — `TESTING.md`.
- **ADR-0044** | 2026-06-21 | Accepted | Flow Agent/Flow terminology is renamed to Loop Agent/Loop — docs and runtime naming.
- **ADR-0045** | 2026-06-22 | Accepted | HTML render gate is `pnpm run docs:render-check` via `scripts/check-html-render.mjs` — `TESTING.md`.
- **ADR-0046** | 2026-06-22 | Accepted | Branch workflow uses one short-lived topic branch per logical change, reused when it fits — `AGENTS.md`.
- **ADR-0047** | 2026-06-22 | Accepted | PR workflow uses GitHub CLI (`gh`) — `AGENTS.md`.
- **ADR-0048** | 2026-06-22 | Accepted | Only unpublished commit metadata may be amended; topic branches must not track `origin/main` — `AGENTS.md`.
- **ADR-0049** | 2026-06-24 | Accepted | M1 Loop Agent performance budgets are fixed: FSM p95 <= 1 ms/event, no-op local tool-dispatch p95 <= 50 ms, memory <= 10 MiB/active top-level loop before payloads, log append p95 <= 5 ms/event, and 10 fixture top-level loops complete without deadlock or unbounded memory growth — `PERFORMANCE.md`, `TESTING.md`.
- **ADR-0050 / ADR-0058** | 2026-06-24 / 2026-07-13 | Accepted | M1 uses deterministic, cache-stable `loop-context-v0`: mandatory active scope, bounded continuity, reproducible manifests/hashes and no provider truncation, embeddings, RAG or adaptive compaction; persisted compaction and retrieval remain post-M1 — Loop Agent V-Spec.
- **ADR-0051** | 2026-06-24 | Accepted | M1 Linux-target network policy is fail-closed deny-all; deterministic in-process runs reject non-empty allowlists; CIDR allowlists remain policy artifacts until a post-M1 egress backend exists — `SECURITY.md`.
- **ADR-0053** | 2026-07-01 | Accepted | Recursive loop composition is capped at 64 levels across registry validation, policy compilation and runtime emission — Loop Agent V-Spec.
- **ADR-0054** | 2026-07-01 | Accepted | M1 registry/session-log reads use fixed caps: 1 MiB per registry file, 16 MiB registry total and 16 MiB per session log; tail validates appended JSONL suffixes against prior state — Loop Agent V-Spec.
- **ADR-0055** | 2026-07-01 | Accepted | Post-M1 Loop Agent control starts with minimal local JSON-RPC `loop.start`/`loop.status`/`loop.cancel`/`loop.tail`/`loop.export`; no `cmd.*` runtime events — `PROTOCOL.md`, Loop Agent V-Spec.
- **ADR-0056** | 2026-07-05 | Accepted | M1 merge protection requires the main-branch ruleset to gate PR merges on the M1 CI jobs; `feat/**` push CI stays advisory — `PLAN.md`, `AGENTS.md`.
- **ADR-0057** | 2026-07-05 | Accepted | Trusted Codex project config keeps `sandbox_workspace_write.network_access = true` with `approval_policy = "never"` and `web_search = "disabled"` for networked repo closeout; this is not product runtime egress — `.codex/config.toml`, `AGENTS.md`.
- **ADR-0059** | 2026-07-13 | Accepted | Each session validates, sequences and appends canonical events through one serial writer before publishing the committed values; bounded micro-batching/backpressure gives near-real-time at-least-once delivery repaired by sequence replay, with checkpoint durability but no per-delta `fsync` — `PROTOCOL.md`, `PERFORMANCE.md`.
