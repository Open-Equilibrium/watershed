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
- **Event-ordering & transcript persistence (Loop Agent)** — tests cover the ADR-0059/ADR-0062 `PROTOCOL.md` risks: persistence-before-notification and failure visibility, bounded/coalesced wake-ups, receiver disconnect, session isolation, race-free sequence catch-up, checkpoint durability, safe session IDs, and replay/tail/resume equivalence. Runtime logs are not project VCS/history.
- **Script/schema tests** — valid building-block scripts parse into the expected model; invalid scripts fail with useful diagnostics.
- **Sandbox boundary tests (SECURITY)** — sandbox-negative fixtures attempt prohibited path access, forbidden writes, symlink traversal including create-through-symlink ancestors, disallowed network egress including hostname/DNS attempts, forbidden environment inheritance/secret reads, interpreter misuse through allowed commands, out-of-phase tool use and protected-path access using the default list in `SECURITY.md`; expected-decision fixtures validate compiled policy artifacts in M0, while M1 attempts are denied by deterministic in-process policy execution/emulation. Linux Landlock/seccomp OS enforcement and macOS Seatbelt parity remain post-M1 (ADR-0051/ADR-0052).
- **Control-plane tests (Meta-Harness)** — Meta-Harness runs headlessly without Liquid; its CLI/API/service surface exposes the session registry, event/transcript streams and AgentPulse queries; at least two agents are represented through one normalized session/event model; shared config resolves to agent-specific runtime config without duplicate per-agent config directories. Liquid integration is exercised against the public API, not a duplicate backend.
- **Config-write audit tests (Meta-Harness)** — every config change yields an audit entry; sensitive changes block on approval.
- **Workspace history & agent-edit tests (Liquid)** — action-history append; revert (incl. compensating actions); diff; external-agent CLI/API mutation; permission-denied; actor/origin attribution; component mutation schema; snapshot/checkpoint restore; workspace event subscription; and a no-hidden-writes test proving every UI/CLI/API/Liquid-AI/external-agent write passes through the mutation pipeline and records an action. Liquid's workspace history is a workspace VCS over its own data, not a project-code VCS.
- **Performance gates** — benchmarks assert the `PERFORMANCE.md` targets/budgets relevant to the current milestone. M1 Loop Agent benchmarks run optimized in CI, measure per-event distributions (not batch-average distributions), exercise append and producer-side notification attempts through deterministic fixture/stub-model runs, and exclude model latency, tool runtime, checkpoint synchronization, caller replay and caller transport. Failures block release.
- **Coverage gate** — from M1 (first real crate logic), `cargo llvm-cov nextest --workspace --fail-under-lines 90` enforces **≥90% line coverage**; merge blocks below it. Ignored timing tests run optimized outside llvm-cov. Generated/FFI/CLI-arg-glue code may be excluded through llvm-cov ignore configuration so the threshold measures meaningful logic, not boilerplate; region/function coverage are tracked as secondary signals (ADR-0022/ADR-0060).

Milestone pass/fail criteria and DoD are canonical in [`PLAN.md`](PLAN.md).

## Fixture contract (ADR-0034)

Checked fixture registry roots and expected JSONL streams cover M0 contracts and execute in M1. Each fixture root uses the Loop Agent V-Spec v0 container: recursive `.yaml`/`.yml` files, one YAML 1.2 document and one top-level block per file, with identity from block `id`/`name`. Expected streams use the `PROTOCOL.md` v0 envelope, fixed lowercase path-safe `session_id` tokens, fixed fixture IDs, fixed fixture timestamps and monotonic per-session `sequence`, serialized with `PROTOCOL.md` canonical event JSONL bytes. Contract assertions use the public payload fields in `PROTOCOL.md` (`loop_definition_id`, `tool_kind`, `read_scope`, `write_scope`, `allowed_parameters`, `network_access`, `instruction_ids`, `connection_ids` and `connection_kinds`), not fixture-private names. Connection assertions enforce the `PROTOCOL.md` pairing rule: `connection_ids[i]` and `connection_kinds[i]` describe the same registry-resolved Step `connection_refs[i]` entry. Script-definition assertions are separate from golden event-stream assertions: the `hello-loop` source fixture must include one predefined-command tool using the v0 `command.command_id` plus literal `command.argv` shape and one allowed parameter with a typed, constrained value, and one own-script tool using the v0 `script_runtime: posix-sh` and `script_body` fields from the Loop Agent V-Spec. Those fields are not public event payload fields unless `PROTOCOL.md` later adds them.

- `smoke-loop`: the checked-in stream uses this event-type order: `session.started`, `loop.started`, `phase.entered`, `step.started`, `message.delta`, `message.completed`, `tool.started`, `tool.completed`, `step.completed`, `loop.completed`, `session.completed`. Payloads identify one phase, one instruction and one read-only predefined-command tool with no network or writes by those protocol fields.
- `hello-loop`: starts and ends with the same session/loop lifecycle shape as `smoke-loop`; includes at least two `phase.entered` events, at least two tool runs covering predefined-command and own-script tools by public `tool_kind`, read-only and declared write scope, one allowed-parameter case, at least one `tool.progress`, at least two phase-scoped instruction payloads, data and trigger/refresh connection ids/kinds using the protocol payload fields above, and one subloop definition reused at least twice. Each subloop invocation has a distinct runtime `loop_id`, the same payload `loop_definition_id` and `parent_loop_id` set to the containing runtime loop id.
- Sandbox-negative fixtures: tiny loops for forbidden write, forbidden network including hostname/DNS attempts, forbidden environment inheritance/secret reads, out-of-phase tool, protected-path access, symlink traversal including create-through-symlink ancestors and interpreter misuse through an allowed command. Each expected stream rejects before side effects and ends with `error`, `loop.failed` and `session.failed`; if a tool launch was attempted, it also contains `tool.failed`. Negative streams must not contain `tool.completed`, `loop.completed` or `session.completed`.

## Liquid MVP pass/fail (M3)

In addition to the full M3 DoD in `PLAN.md`, the Liquid MVP must prove: every UI/CLI/API mutation records an action; an external-agent mutation can be reverted; the action history can show actor, target, operation and diff/patch; and the workspace can be restored to a previous checkpoint/snapshot.

## AgentPulse as a quality gate

The same metrics AgentPulse reports (rework ratio, first-attempt success rate, cost-per-productive-outcome) are tracked in CI on golden loops once their formulas are decided. The formulas are an open decision until M2 planning.

## CI

- Run on Linux + macOS + Windows.
- Mandatory gates: `cargo fmt --check`, `cargo clippy`, `cargo nextest run` (ignored timing tests run separately), optimized Loop Agent performance tests, the M1 coverage gate, `cargo audit` + `cargo deny`, and the `lychee` docs link + HTML render checks.
- Block merge on any mandatory gate failure, including current-milestone performance checks and planned platform sandbox/parity checks once implemented.
