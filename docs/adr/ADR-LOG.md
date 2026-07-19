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
- **ADR-0010 / ADR-0012** | 2026-06-05 / 2026-06-09 | Accepted | Project name is Watershed; owner is `Open-Equilibrium` — `README.md`.
- **ADR-0011** | 2026-06-09 | Accepted | MVP excludes Watershed-owned project-code VCS/history behavior — `AGENTS.md`, `SECURITY.md`, `PLAN.md`.
- **ADR-0013** | 2026-06-10 | Accepted | CLI names are `loop`, `meta` and `liq` — V-Specs.
- **ADR-0014** | 2026-06-10 | Accepted | Liquid NFRs use tiered falsifiable budgets — `PERFORMANCE.md`.
- **ADR-0015** | 2026-06-10 | Accepted | Contributions are DCO-only; no CLA — `CONTRIBUTING.md`.
- **ADR-0016** | 2026-06-11 | Accepted | Product framing is one AGPL/free-software platform with three independently usable layers — `README.md`, `VISION.md`.
- **ADR-0017** | 2026-06-11 | Accepted | Adoption sequence is Loop Agent, then Meta-Harness, then Liquid — `PLAN.md`.
- **ADR-0018** | 2026-06-11 | Accepted | Layer positioning: Loop Agent execution runtime, Meta-Harness headless control plane, Liquid standalone workspace/action product — V-Specs.
- **ADR-0019** | 2026-06-11 | Accepted | Docs keep an AGPL/free-software posture and make no monetization/open-core claims — `README.md`, `VISION.md`.
- **ADR-0020** | 2026-06-11 | Accepted | Official repo target is `Open-Equilibrium/watershed`; npm is not a product target — `README.md`.
- **ADR-0021 / ADR-0043 / ADR-0045** | 2026-06-16 / 2026-06-21 / 2026-06-22 | Accepted | M0 gates include fixed-viewport Chromium rendering of self-contained HTML through the canonical render script — `TESTING.md`.
- **ADR-0022 / ADR-0060** | 2026-06-16 / 2026-07-13 | Accepted | From M1, meaningful line coverage is gated at >=90% via `cargo llvm-cov nextest --workspace --fail-under-lines 90`; timing-sensitive perf tests run optimized outside coverage — `TESTING.md`.
- **ADR-0023 / ADR-0026 / ADR-0027** | 2026-06-16 | Accepted | Codex subagent topology, delegation points and explicit standards compliance are canonical in `AGENTS.md`.
- **ADR-0024** | 2026-06-16 | Accepted | Codex hooks are opt-in defense-in-depth, never a security boundary — `AGENTS.md`.
- **ADR-0025 / ADR-0046 / ADR-0047 / ADR-0048** | 2026-06-16 / 2026-06-22 | Accepted | Protected-main topic-branch and GitHub CLI PR workflow rules are canonical in `AGENTS.md`.
- **ADR-0028** | 2026-06-18 | Accepted | Build Loop Agent orchestration in-house; reuse general plumbing crates; integrate Codex CLI as a Meta-Harness adapter, not a Loop Agent base — Loop Agent V-Spec, Meta-Harness V-Spec.
- **ADR-0029** | 2026-06-19 | Accepted | Designed control transport is local JSON-RPC over stdio; M1 implemented runtime stream is bare JSONL — `PROTOCOL.md`.
- **ADR-0030** | 2026-06-19 | Accepted | The M0 acceptance bar is the milestone DoD and its referenced gates — `PLAN.md`, `TESTING.md`, `SECURITY.md`, `.github/workflows/ci.yml`.
- **ADR-0031 / ADR-0061** | 2026-06-19 / 2026-07-14 | Accepted | Building-block scripts use strict YAML 1.2 through exact-pinned `noyalib` 0.0.15, typed semantic validation, explicit references and canonical resolved JSON; no fallback parser — `SECURITY.md`, Loop Agent V-Spec.
- **ADR-0032 / ADR-0052** | 2026-06-19 / 2026-06-30 | Accepted | M0 has policy artifacts and escape tests; M1 uses deterministic in-process policy enforcement/emulation; Linux Landlock/seccomp is deferred and macOS parity remains planned — `SECURITY.md`, `PLAN.md`.
- **ADR-0033** | 2026-06-19 | Accepted | Keep shared contract crates separate from Loop Agent runtime and CLI crates — Loop Agent V-Spec.
- **ADR-0034** | 2026-06-19 | Accepted | Fixture suite is `smoke-loop`, `hello-loop` and sandbox-negative fixtures using a stub model — `TESTING.md`.
- **ADR-0035** | 2026-06-19 | Accepted | M1 ships human CLI and JSONL event stream; RPC and embedded core are designed-for seams — Loop Agent V-Spec.
- **ADR-0036** | 2026-06-19 | Accepted | Runtime event names and envelope shape are the v0 public event contract — `PROTOCOL.md`.
- **ADR-0037** | 2026-06-19 | Accepted | Session logs are append-only `.loop/sessions/<session_id>.jsonl` files with replay/tail/resume semantics — Loop Agent V-Spec.
- **ADR-0038** | 2026-06-21 | Accepted | Keep protocol seams remote-capable now; defer remote implementation — `PROTOCOL.md`.
- **ADR-0039** | 2026-06-21 | Accepted | Remote/session durability is replication plus durable storage, not container lifetime — Loop Agent V-Spec.
- **ADR-0040** | 2026-06-21 | Accepted | Liquid MVP uses a fixed trusted Block palette plus Script Block compute; sandboxed custom UI is post-MVP — Liquid V-Spec.
- **ADR-0041** | 2026-06-21 | Accepted | Fixture workspaces use checked-in `.loop/config.yaml` to select registry root and stub model profile — `TESTING.md`.
- **ADR-0042** | 2026-06-21 | Accepted | Trusted core code owns the predefined-command registry; loop YAML may only reference command IDs — `SECURITY.md`.
- **ADR-0049** | 2026-06-24 | Accepted | M1 Loop Agent performance budgets are fixed: FSM p95 <= 1 ms/event, no-op local tool-dispatch p95 <= 50 ms, memory <= 10 MiB/active top-level loop before payloads, log append p95 <= 5 ms/event, and 10 fixture top-level loops complete without deadlock or unbounded memory growth — `PERFORMANCE.md`, `TESTING.md`.
- **ADR-0050 / ADR-0058** | 2026-06-24 / 2026-07-13 | Accepted | M1 uses deterministic, cache-stable `loop-context-v0`: mandatory active scope, bounded continuity, reproducible manifests/hashes and no provider truncation, embeddings, RAG or adaptive compaction; persisted compaction and retrieval remain post-M1 — Loop Agent V-Spec.
- **ADR-0051** | 2026-06-24 | Accepted | M1 Linux-target network policy is fail-closed deny-all; deterministic in-process runs reject non-empty allowlists; CIDR allowlists remain policy artifacts until a post-M1 egress backend exists — `SECURITY.md`.
- **ADR-0053** | 2026-07-01 | Superseded by ADR-0068 | The former 64-level composition cap was replaced by the measured ADR-0068 workload limits — Loop Agent V-Spec.
- **ADR-0055** | 2026-07-01 | Accepted | Post-M1 Loop Agent control starts with minimal local JSON-RPC `loop.start`/`loop.status`/`loop.cancel`/`loop.tail`/`loop.export`; no `cmd.*` runtime events — `PROTOCOL.md`, Loop Agent V-Spec.
- **ADR-0056** | 2026-07-05 | Accepted | M1 merge protection requires the main-branch ruleset to gate PR merges on the M1 CI jobs; `feat/**` push CI stays advisory — `AGENTS.md`, `TESTING.md`, `.github/workflows/ci.yml`.
- **ADR-0057** | 2026-07-05 | Accepted | Trusted Codex project config keeps `sandbox_workspace_write.network_access = true` with `approval_policy = "never"` and `web_search = "disabled"` for networked repo closeout; this is not product runtime egress — `.codex/config.toml`, `AGENTS.md`.
- **ADR-0059** | 2026-07-13 | Accepted | Each session validates, sequences and appends canonical events through one serial writer; bounded micro-batching and checkpoint durability avoid per-delta `fsync`, and committed events remain recoverable by sequence — `PROTOCOL.md`, `PERFORMANCE.md`.
- **ADR-0062** | 2026-07-14 | Accepted | M1 live delivery uses per-session bounded, non-blocking, caller-owned coalescing high-watermark notifications; receivers replay committed events by sequence from the authoritative session log, and core owns no arbitrary blocking transport — `PROTOCOL.md`, Loop Agent V-Spec.
- **ADR-0063** | 2026-07-16 | Accepted | Registry loading starts from one workspace capability and opens every registry directory and YAML leaf without following links; exact-pinned `cap-std`/`cap-fs-ext` provide the private Linux/macOS-first cross-platform boundary — `AGENTS.md`, `SECURITY.md`.
- **ADR-0064** | 2026-07-16 | Accepted | The capability boundary's Windows-only `winx 0.36.4` is absent from Linux/macOS builds and has an exact package-specific `Apache-2.0 WITH LLVM-exception` license exception — `AGENTS.md`, `deny.toml`.
- **ADR-0065** | 2026-07-16 | Accepted | Loop Agent is host-local; each Meta-Harness controls only CLI agents on its own host while exposing local-or-remote client APIs; Liquid uses Meta-Harness as its sole agent-control path and preserves per-instance authority in merged projections — `VISION.md`, `PROTOCOL.md`, V-Specs.
- **ADR-0066** | 2026-07-16 | Accepted | Liquid is always local-first; Pages and Blocks are its only authored-surface terms: Blocks own Views, connect explicitly, implement the Block SDK and flow before optional responsive arrangement; sync exchanges local actions resumably through an independent sync host — `GLOSSARY.md`, Liquid V-Spec.
- **ADR-0054 / ADR-0067** | 2026-07-01 / 2026-07-17 | Partly superseded by ADR-0068 | Registry scans remain capped at 1,024 entries / 16 MiB, each YAML file at 128 KiB and the selected closure at 1 MiB; the former 16 MiB whole-session cap was replaced by segmented storage — Loop Agent V-Spec, `PERFORMANCE.md`.
- **ADR-0068** | 2026-07-18 | Accepted | Fixes depth 16, direct fan-out 32, 512 cumulative invocations, 155,750 cumulative events, 32 process-wide live invocations and measured byte/bundle limits; canonical JSONL rotates at 16 MiB instead of ending the session, and the former 10 MiB stream cap is removed — `PERFORMANCE.md`, `PROTOCOL.md`, Loop Agent V-Spec.
