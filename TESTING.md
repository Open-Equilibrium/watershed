# Testing

Verification strategy. Tests are how the budgets in `PERFORMANCE.md` and the boundaries in `SECURITY.md` become real. Definition of Done (see `AGENTS.md`) requires the relevant tests.

## MVP boundary

Loop Agent MVP tests must not require Watershed-owned project-history/VCS behavior. Tests may run in a normal Git checkout, but Loop Agent does not auto-commit, branch, slice or rewrite project history in the MVP.

## Test economy

Protect meaningful functionality from every milestone, including behavior established earlier. Each test must cover a distinct function, contract, risk or regression. Prefer compact table-driven or end-to-end cases and shared setup over line-by-line tests. Coverage remains a gate, not a reason for redundant or coverage-only tests.

## Layers

- **Unit/integration per crate** — standard Rust tests run with `cargo nextest` for deterministic, process-isolated execution (no shared-state leakage between tests) and flaky-test detection. Rust `#[ignore]` is reserved for timing-sensitive tests, which run only in the optimized performance gate.
- **Deterministic FSM tests (Loop Agent)** — given a loop script + fixed inputs, phase/step transitions, instruction loading, connection resolution and the *set* of available tools are asserted. LLM/tool outputs are mocked inputs to deterministic transitions.
- **Context compiler tests (Loop Agent)** — `loop-context-v0` tests assert Tier 0 section/order completeness, active-scope filtering, whole-unit recent-interaction selection/eviction, cache-prefix boundaries, budget/estimator behavior, fail-before-provider errors, canonical context hashes/manifests and byte-identical reconstruction on resume. M1 records deterministic absence for runtime inputs without a decided source/schema; typed connection values and artifact/tool-output projections require such a contract before tests can require them. Tests also prove older sources remain in durable history while omitted from provider context (ADR-0058).
- **Golden loops** — recorded end-to-end loops replayed against stub-model/tool responses; event streams and output artifacts are byte-stable and diffed.
- **Event-ordering & transcript persistence (Loop Agent)** — tests cover the ADR-0059/ADR-0062 `PROTOCOL.md` risks: persistence-before-notification and failure visibility, bounded micro-batches with deadline/semantic flush, bounded/coalesced wake-ups, receiver disconnect, session isolation, race-free sequence catch-up, checkpoint durability, safe session IDs, and replay/tail/resume equivalence. Runtime logs are not project VCS/history.
- **Script/schema tests** — valid building-block scripts parse into the expected model; invalid scripts fail with useful diagnostics; scoped-registry tests cover transitive references, shared-definition deduplication, unrelated-block exclusion and closure limits.
- **Sandbox boundary tests (SECURITY)** — sandbox-negative fixtures attempt prohibited path access, forbidden writes, symlink traversal including create-through-symlink ancestors, disallowed network egress including hostname/DNS attempts, forbidden environment inheritance/secret reads, interpreter misuse through allowed commands, out-of-phase tool use and protected-path access using the default list in `SECURITY.md`; expected-decision fixtures validate compiled policy artifacts in M0, while M1 attempts are denied by deterministic in-process policy execution/emulation. Linux Landlock/seccomp OS enforcement and macOS Seatbelt parity remain post-M1 (ADR-0051/ADR-0052).
- **Control-plane tests (Meta-Harness)** — Meta-Harness runs headlessly without Liquid; its CLI/API/service exposes the session registry, event/transcript streams and AgentPulse queries; at least two host-local agents share one normalized session/event model; cross-host process-control attempts are rejected; shared config resolves without duplicate per-agent config directories. Clients exercise the same public API through the bindings selected by D-023, not a duplicate backend.
- **Config-write audit tests (Meta-Harness)** — every config change yields an audit entry; sensitive changes block on approval.
- **Workspace history & agent-edit tests (Liquid)** — action-history and recovery behavior selected by D-028; revert semantics selected by D-031; Page/Block mutation schema; Block View and typed Connection behavior; external-agent CLI/API mutation; permission denial; actor/origin attribution; workspace event subscription; and a no-hidden-writes test proving every UI/CLI/API/Liquid-AI/external-agent/sync write passes through the mutation pipeline and records an action. Liquid's workspace history is a workspace VCS over its own data, not a project-code VCS.
- **ADR-0068 boundary tests (Loop Agent)** — functional tests cover depth 16/17, direct fan-out 32/33, cumulative invocation 512/513, live invocation 32/33, event 155,750/155,751, 320 KiB event size, 16 MiB per event segment or context-manifest segment without record splitting, 48 MiB aggregate event/manifest data, immutable-object hash/deduplication/missing-object rejection and removal of the former 10 MiB stream cap. The exact 155,750-event arithmetic and representative payload distribution are executable assertions.
- **Performance gates** — benchmarks assert the `PERFORMANCE.md` targets/budgets relevant to the current milestone. M1 Loop Agent benchmarks run optimized, serially and only on the fixed `ubuntu-24.04` x64 CI image. They measure per-event distributions (not batch-average distributions), exercise append and notification attempts, a complete 155,750-event replay and full-session inspection, 320 KiB incremental tail reads, the representative ten-session profile and ten full-cap small-payload sessions. The aggregate ADR-0068 gates measure synthetic event storage/replay, not end-to-end runtime. They also retain the 10-root concurrency/near-limit-registry tests. Checkpoint synchronization, external model latency, tool runtime, caller output buffers and transport are excluded. Failures block release.
- **Coverage gate** — from M1 (first real crate logic), the canonical [`Check line coverage` CI job](.github/workflows/ci.yml) enforces **≥90% line coverage**; merge blocks below it. Ignored timing tests run optimized outside llvm-cov. Generated/FFI/CLI-arg-glue code may be excluded so the threshold measures meaningful logic, not boilerplate; region/function coverage are tracked as secondary signals (ADR-0022/ADR-0060).

Milestone pass/fail criteria and DoD are canonical in [`PLAN.md`](PLAN.md).

## Fixture contract (ADR-0034)

Checked registry fixtures follow the [Loop Agent V-Spec](docs/concept/V-Spec_LoopAgent.html); expected JSONL follows [`PROTOCOL.md`](PROTOCOL.md), with fixed fixture identities and timestamps for byte stability. Tests assert source-definition contracts separately from public event payloads.

- `smoke-loop`: the checked-in stream uses this event-type order: `session.started`, `loop.started`, `phase.entered`, `step.started`, `message.delta`, `message.completed`, `tool.started`, `tool.completed`, `step.completed`, `loop.completed`, `session.completed`. Payloads identify one phase, one instruction and one read-only predefined-command tool with no network or writes by those protocol fields.
- `hello-loop`: starts and ends with the same session/loop lifecycle shape as `smoke-loop`; includes at least two `phase.entered` events, at least two tool runs covering predefined-command and own-script tools by public `tool_kind`, read-only and declared write scope, one allowed-parameter case, at least one `tool.progress`, at least two phase-scoped instruction payloads, data and trigger/refresh connection ids/kinds using the protocol payload fields above, and one subloop definition reused at least twice. Each subloop invocation has a distinct runtime `loop_id`, the same payload `loop_definition_id` and `parent_loop_id` set to the containing runtime loop id.
- Sandbox-negative fixtures: tiny loops for forbidden write, forbidden network including hostname/DNS attempts, forbidden environment inheritance/secret reads, out-of-phase tool, protected-path access, symlink traversal including create-through-symlink ancestors and interpreter misuse through an allowed command. Each expected stream rejects before side effects and ends with `error`, `loop.failed` and `session.failed`; if a tool launch was attempted, it also contains `tool.failed`. Negative streams must not contain `tool.completed`, `loop.completed` or `session.completed`.

## Liquid MVP pass/fail (M3)

In addition to the provisional M3 DoD in `PLAN.md`, the Liquid MVP must prove: deterministic first-action Block creation; content flow and Arrange mode preserve one canonical Block order; Views share their owning Block's state; Connections do not depend on layout or active View; mobile rendering retains every Block with safe focused editing for complex Views; every UI/CLI/API/sync mutation records an action; external-agent changes can be reverted; action history shows actor, target, operation and diff/patch; and selected D-028/D-031/D-035 history, recovery and sync behavior works.

## AgentPulse as a quality gate

The same metrics AgentPulse reports (rework ratio, first-attempt success rate, cost-per-productive-outcome) are tracked in CI on golden loops once their formulas are decided. The formulas are an open decision until M2 planning.

## CI

- Run on Linux + macOS + Windows.
- Mandatory gates: `rustfmt --check` over every tracked Rust source, `cargo clippy`, `cargo nextest run` (ignored timing tests run separately), optimized Loop Agent performance tests, the M1 coverage gate, `cargo audit` + `cargo deny`, and the `lychee` docs link + HTML render checks.
- If Windows cannot run llvm-cov because the Rust GNU profiler runtime is unavailable, verify the pushed branch's `Check line coverage` CI jobs with `gh`; do not add wrapper or WSL workaround code.
- Block merge on any mandatory gate failure, including current-milestone performance checks and planned platform sandbox/parity checks once implemented.
