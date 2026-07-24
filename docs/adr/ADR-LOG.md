# ADR Log

Agent-facing index of accepted decisions. Keep detail in the canonical docs; keep this file short enough to scan before implementation.

Policy: decision flow and hygiene rules live in [AGENTS.md](../../AGENTS.md#decision-flow).

Format: `ID | date | status | decision — canonical context`

## Decisions

- **ADR-0001** | 2026-06-05 | Accepted | Flow Agent has a Rust core; no product Node runtime — `README.md`, `PERFORMANCE.md`.
- **ADR-0002** | 2026-06-05 | Accepted | Monorepo with shared `core` + `proto`, not one monolith — `README.md`.
- **ADR-0003** | 2026-06-09 | Accepted | License is SPDX `AGPL-3.0-only` — `LICENSE`, `README.md`.
- **ADR-0004** | 2026-06-05 | Accepted | Scripts define capabilities; sandbox policy enforces them — `SECURITY.md`.
- **ADR-0005** | 2026-06-05 | Accepted | Reuse proven sandbox primitives and Wasmtime plugin isolation — `SECURITY.md`.
- **ADR-0006** | 2026-06-05 | Accepted | Authored scripts are the source of truth; visual graphs are views — `GLOSSARY.md`.
- **ADR-0007** | 2026-06-05 | Accepted | AgentPulse belongs to Meta-Harness — `PLAN.md`, Meta-Harness V-Spec.
- **ADR-0008** | 2026-06-05 | Accepted | Meta-Agent config writes are policy-gated, audited and approval-aware — `SECURITY.md`.
- **ADR-0009** | 2026-06-05 | Accepted | Integration happens through protocol clients; Liquid is the rich UI consumer — `PROTOCOL.md`.
- **ADR-0010 / ADR-0012** | 2026-06-05 / 2026-06-09 | Accepted | Project name is Watershed; owner is `Open-Equilibrium` — `README.md`.
- **ADR-0011** | 2026-06-09 | Accepted | MVP excludes Watershed-owned project-code VCS/history behavior — `AGENTS.md`, `SECURITY.md`, `PLAN.md`.
- **ADR-0013** | 2026-06-10 | Accepted | CLI names are `flow`, `meta` and `liq` — V-Specs.
- **ADR-0014** | 2026-06-10 | Accepted | Liquid NFRs use tiered falsifiable budgets — `PERFORMANCE.md`.
- **ADR-0015** | 2026-06-10 | Accepted | Contributions are DCO-only; no CLA — `CONTRIBUTING.md`.
- **ADR-0016** | 2026-06-11 | Accepted | Product framing is one AGPL/free-software platform with three independently usable layers — `README.md`, `VISION.md`.
- **ADR-0017** | 2026-06-11 | Accepted | Adoption sequence is Flow Agent, then Meta-Harness, then Liquid — `PLAN.md`.
- **ADR-0018** | 2026-06-11 | Accepted | Layer positioning: Flow Agent execution runtime, Meta-Harness headless control plane, Liquid standalone workspace/action product — V-Specs.
- **ADR-0019** | 2026-06-11 | Accepted | Docs keep an AGPL/free-software posture and make no monetization/open-core claims — `README.md`, `VISION.md`.
- **ADR-0020** | 2026-06-11 | Accepted | Official repo target is `Open-Equilibrium/watershed`; npm is not a product target — `Cargo.toml`, `package.json`.
- **ADR-0021 / ADR-0043 / ADR-0045** | 2026-06-16 / 2026-06-21 / 2026-06-22 | Accepted | M0 gates include fixed-viewport Chromium rendering of self-contained HTML through the canonical render script — `TESTING.md`.
- **ADR-0022 / ADR-0060** | 2026-06-16 / 2026-07-13 | Accepted | From M1, meaningful line coverage is gated at >=90% via `cargo llvm-cov nextest --workspace --fail-under-lines 90`; timing-sensitive perf tests run optimized outside coverage — `TESTING.md`.
- **ADR-0023 / ADR-0026 / ADR-0027** | 2026-06-16 | Accepted | Codex subagent topology, delegation points and explicit standards compliance are canonical in `AGENTS.md`.
- **ADR-0024** | 2026-06-16 | Accepted | Codex hooks are opt-in defense-in-depth, never a security boundary — `AGENTS.md`.
- **ADR-0025 / ADR-0046 / ADR-0047 / ADR-0048** | 2026-06-16 / 2026-06-22 | Accepted | Protected-main topic-branch and GitHub CLI PR workflow rules are canonical in `AGENTS.md`.
- **ADR-0028** | 2026-06-18 | Accepted | Build Flow Agent orchestration in-house; reuse general plumbing crates; integrate Codex CLI as a Meta-Harness adapter, not a Flow Agent base — Flow Agent V-Spec, Meta-Harness V-Spec.
- **ADR-0029** | 2026-06-19 | Accepted | Designed control transport is local JSON-RPC over stdio; M1 implemented runtime stream is bare JSONL — `PROTOCOL.md`.
- **ADR-0030** | 2026-06-19 | Accepted | The M0 acceptance bar is the milestone DoD and its referenced gates — `PLAN.md`, `TESTING.md`, `SECURITY.md`, `.github/workflows/ci.yml`.
- **ADR-0031 / ADR-0061** | 2026-06-19 / 2026-07-14 | Accepted | Building-block scripts use strict YAML 1.2 through exact-pinned `noyalib` 0.0.15, typed semantic validation, explicit references and canonical resolved JSON; no fallback parser — `SECURITY.md`, Flow Agent V-Spec.
- **ADR-0032 / ADR-0052** | 2026-06-19 / 2026-06-30 | Accepted | M0 has policy artifacts and escape tests; M1 uses deterministic in-process policy enforcement/emulation; Linux Landlock/seccomp is deferred and macOS parity remains planned — `SECURITY.md`, `PLAN.md`.
- **ADR-0033** | 2026-06-19 | Accepted | Keep shared contract crates separate from Flow Agent runtime and CLI crates — Flow Agent V-Spec.
- **ADR-0034** | 2026-06-19 | Accepted | Fixture suite is `smoke-flow`, `hello-flow` and sandbox-negative fixtures using a stub model — `TESTING.md`.
- **ADR-0035** | 2026-06-19 | Accepted | M1 ships human CLI and JSONL event stream; RPC and embedded core are designed-for seams — Flow Agent V-Spec.
- **ADR-0036** | 2026-06-19 | Accepted | Runtime event names and envelope shape are the v0 public event contract — `PROTOCOL.md`.
- **ADR-0037** | 2026-06-19 | Accepted | Session logs are append-only `.flow/sessions/<session_id>.jsonl` files with replay/tail/resume semantics — Flow Agent V-Spec.
- **ADR-0038** | 2026-06-21 | Accepted | Keep protocol seams remote-capable now; defer remote implementation — `PROTOCOL.md`.
- **ADR-0039** | 2026-06-21 | Accepted | Remote/session durability is replication plus durable storage, not container lifetime — Flow Agent V-Spec.
- **ADR-0041** | 2026-06-21 | Accepted | Fixture workspaces use checked-in `.flow/config.yaml` to select registry root and stub model profile — `TESTING.md`.
- **ADR-0042** | 2026-06-21 | Accepted | Trusted core code owns the predefined-command registry; flow YAML may only reference command IDs — `SECURITY.md`.
- **ADR-0049** | 2026-06-24 | Accepted | M1 Flow Agent performance budgets are fixed: FSM p95 <= 1 ms/event, no-op local tool-dispatch p95 <= 50 ms, memory <= 10 MiB/active top-level flow before payloads, log append p95 <= 5 ms/event, and 10 fixture top-level flows complete without deadlock or unbounded memory growth — `PERFORMANCE.md`, `TESTING.md`.
- **ADR-0050 / ADR-0058** | 2026-06-24 / 2026-07-13 | Accepted | M1 uses deterministic, cache-stable `flow-context-v0`: mandatory active scope, bounded continuity, reproducible manifests/hashes and no provider truncation, embeddings, RAG or adaptive compaction; persisted compaction and retrieval remain post-M1 — Flow Agent V-Spec.
- **ADR-0051** | 2026-06-24 | Accepted | M1 Linux-target network policy is fail-closed deny-all; deterministic in-process runs reject non-empty allowlists; CIDR allowlists remain policy artifacts until a post-M1 egress backend exists — `SECURITY.md`.
- **ADR-0055** | 2026-07-01 | Accepted | Post-M1 Flow Agent control starts with minimal local JSON-RPC `flow.start`/`flow.status`/`flow.cancel`/`flow.tail`/`flow.export`; no `cmd.*` runtime events — `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0056** | 2026-07-05 | Accepted | M1 merge protection requires the main-branch ruleset to gate PR merges on the M1 CI jobs; `feat/**` push CI stays advisory — `AGENTS.md`, `TESTING.md`, `.github/workflows/ci.yml`.
- **ADR-0057** | 2026-07-05 | Accepted | Trusted Codex project config keeps `sandbox_workspace_write.network_access = true` with `approval_policy = "never"` and `web_search = "disabled"` for networked repo closeout; this is not product runtime egress — `.codex/config.toml`, `AGENTS.md`.
- **ADR-0059** | 2026-07-13 | Accepted | Each session validates, sequences and appends canonical events through one serial writer; bounded micro-batching and checkpoint durability avoid per-delta `fsync`, and committed events remain recoverable by sequence — `PROTOCOL.md`, `PERFORMANCE.md`.
- **ADR-0062** | 2026-07-14 | Accepted | M1 live delivery uses per-session bounded, non-blocking, caller-owned coalescing high-watermark notifications; receivers replay committed events by sequence from the authoritative session log, and core owns no arbitrary blocking transport — `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0063** | 2026-07-16 | Accepted | Registry loading starts from one workspace capability and opens every registry directory and YAML leaf without following links; exact-pinned `cap-std`/`cap-fs-ext` provide the private Linux/macOS-first cross-platform boundary — `AGENTS.md`, `SECURITY.md`.
- **ADR-0064** | 2026-07-16 | Accepted | The capability boundary's Windows-only `winx 0.36.4` is absent from Linux/macOS builds and has an exact package-specific `Apache-2.0 WITH LLVM-exception` license exception — `AGENTS.md`, `deny.toml`.
- **ADR-0065** | 2026-07-16 | Accepted | Flow Agent is host-local; each Meta-Harness controls only CLI agents on its own host while exposing local-or-remote client APIs; Liquid uses Meta-Harness as its sole agent-control path and preserves per-instance authority in merged projections — `VISION.md`, `PROTOCOL.md`, V-Specs.
- **ADR-0066** | 2026-07-16 | Accepted | Liquid is always local-first; Pages and Blocks are its authored-surface terms, Blocks own Views and Connections stay explicit, with the sync topology refined by ADR-0069 — `GLOSSARY.md`, Liquid V-Spec.
- **ADR-0054 / ADR-0067** | 2026-07-01 / 2026-07-17 | Accepted | Registry scan, YAML-file and selected-closure bounds remain; their exact values are canonical in the Flow Agent V-Spec, and the active-flow memory budget is canonical in `PERFORMANCE.md`.
- **ADR-0068** | 2026-07-18 | Accepted | Fixes depth 16, direct fan-out 32, 512 cumulative invocations, 155,750 cumulative events, 32 process-wide live invocations and measured byte/bundle limits; canonical JSONL rotates at 16 MiB instead of ending the session, and the former 10 MiB stream cap is removed — `PERFORMANCE.md`, `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0069** | 2026-07-22 | Accepted | Liquid replicas form a star around one central Sync Server; authorized user devices sync complete Workspaces, while an optional headless Liquid replica requires workspace-level opt-in and remains logically separate from sync and Meta-Harness — `VISION.md`, `PROTOCOL.md`, Liquid V-Spec.
- **ADR-0070** | 2026-07-22 | Accepted | Liquid owns Pages, Blocks, Views, Sources, Connections, Automations, Roles and History: Pages flow before grid arrangement, Sources share Explorer navigation, synchronized Views reuse one Block, duplicate creates a new Block, and last-View deletion applies the reversible dependency cascade — `GLOSSARY.md`, Liquid V-Spec.
- **ADR-0071** | 2026-07-22 | Accepted | Liquid Roles are reusable allow-only permission compositions with default-denied unlisted access; all write surfaces share one permissioned mutation/History pipeline for users, groups, agent profiles, sessions and Automations, while explicit deny rules are deferred — `SECURITY.md`, Liquid V-Spec.
- **ADR-0072** | 2026-07-22 | Accepted | Liquid uses Connections/formulas, Automations and App Blocks as three logic levels; the App SDK targets workspace-local restricted JavaScript/TypeScript Apps, the Block SDK targets signed sandboxed Block Registry extensions and MCP adapters map external capabilities to typed App actions — Liquid V-Spec, `PLAN.md`.
- **ADR-0073** | 2026-07-23 | Accepted; vocabulary clause superseded by ADR-0074 | Rename the unreleased execution product and identity seams to Flow Agent: packages and paths use `flow-agent*`, the CLI uses `flow`, the event source uses `flow-agent-cli` and local state uses `.flow`, without legacy aliases; generic Loop workflow, event and context vocabulary stays unchanged — `README.md`, `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0074** | 2026-07-23 | Accepted | The complete unreleased execution domain uses Flow, including Flow Agent, Flow, Subflow, definitions, invocations, context, events and registry; remove old names without aliases or migration, superseding ADR-0073's generic Loop-vocabulary clause — `GLOSSARY.md`, `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0075** | 2026-07-23 | Accepted | Close M1 as the Flow Agent deterministic runtime foundation with fixture/stub execution and in-process policy emulation; M1.1 adds practical provider/tool/session operations, and M1.2 adds OS-enforced isolation — `PLAN.md`, `SECURITY.md`, Flow Agent V-Spec.
- **ADR-0076** | 2026-07-23 | Accepted | M1 execution requires the explicit fixture profile, builds a pure signed plan and applies each unfinished side effect at most once; controlled returns explicitly attempt ownership cleanup, rolling back only an empty reservation while preserving active bundle artifacts, and crash locks require manual recovery; all session surfaces share canonical paths and namespace rules, while Resume and complete quota/bundle validation use the full inventory — `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0077** | 2026-07-24 | Accepted | Controlled Run and Resume returns explicitly reconcile operation, writer-finalization and ownership-cleanup results without losing any failure; empty reservations roll back, active artifacts remain, and Drop is only a best-effort panic/unwinding fallback that cannot claim failed physical lock removal — `PROTOCOL.md`, Flow Agent V-Spec.
- **ADR-0078** | 2026-07-24 | Accepted | Node 22.23.1 and pnpm 11.15.1 are exact dev/CI-only pins; CI installs Node through immutable `actions/setup-node` v6.5.0 before Corepack, while Watershed and Flow Agent ship no Node runtime — `TESTING.md`, `.github/workflows/ci.yml`.
- **ADR-0079** | 2026-07-24 | Accepted | Protocol ownership, real modules, explicit runtime/CLI boundaries, responsibility splits, a finite typed execution-plan IR and aligned test architecture are mandatory small-PR entry criteria before M1.1 provider or general subprocess implementation — `PLAN.md`.
- **ADR-0080** | 2026-07-24 | Accepted; supersedes ADR-0078's Node pin | Node 24.18.0 LTS is the exact dev/CI-only Node pin; pnpm remains 11.15.1, CI setup and product-runtime boundaries remain unchanged — `TESTING.md`, `.github/workflows/ci.yml`.
