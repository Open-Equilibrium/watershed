# Testing

Verification strategy. Tests are how the budgets in `PERFORMANCE.md` and the boundaries in `SECURITY.md` become real. Definition of Done (see `AGENTS.md`) requires the relevant tests.

## MVP boundary

Loop Agent MVP tests must not require Watershed-owned project-history/VCS behavior. Tests may run in a normal Git checkout, but Loop Agent does not auto-commit, branch, slice or rewrite project history in the MVP.

## Layers

- **Unit/integration per crate** — standard Rust tests run with `cargo nextest` for deterministic, process-isolated execution (no shared-state leakage between tests) and flaky-test detection.
- **Deterministic FSM tests (Loop Agent)** — given a loop script + fixed inputs, phase/step transitions, instruction loading, connection resolution and the *set* of available tools are asserted. LLM/tool outputs are mocked inputs to deterministic transitions.
- **Golden loops** — recorded end-to-end loops replayed against stub-model/tool responses; event streams and output artifacts are byte-stable and diffed.
- **Event-ordering & transcript persistence (Loop Agent)** — the public JSONL event stream has monotonically increasing per-session `sequence`, the documented event families fire in the expected order, invalid path-like or uppercase `session_id` values are rejected before local-log access, and the local session log (`.loop/sessions/<session_id>.jsonl`, ADR-0037) replays/tails/resumes to the same transcript. This is Loop Agent runtime state, not project VCS/history.
- **Script/schema tests** — valid building-block scripts parse into the expected model; invalid scripts fail with useful diagnostics.
- **Sandbox boundary tests (SECURITY)** — sandbox-negative fixtures attempt prohibited path access, forbidden writes, symlink traversal including create-through-symlink ancestors, disallowed network egress including hostname/DNS attempts, forbidden environment inheritance/secret reads, interpreter misuse through allowed commands, out-of-phase tool use and protected-path access using the default list in `SECURITY.md`; all must be denied by the policy artifact in M0, by the compiled Linux OS policy once enforcement lands, and by macOS policy-artifact parity checks until Seatbelt parity is implemented.
- **Control-plane tests (Meta-Harness)** — Meta-Harness runs headlessly without Liquid; its CLI/API/service surface exposes the session registry, event/transcript streams and AgentPulse queries; at least two agents are represented through one normalized session/event model; shared config resolves to agent-specific runtime config without duplicate per-agent config directories. Liquid integration is exercised against the public API, not a duplicate backend.
- **Config-write audit tests (Meta-Harness)** — every config change yields an audit entry; sensitive changes block on approval.
- **Workspace history & agent-edit tests (Liquid)** — action-history append; revert (incl. compensating actions); diff; external-agent CLI/API mutation; permission-denied; actor/origin attribution; component mutation schema; snapshot/checkpoint restore; workspace event subscription; and a no-hidden-writes test proving every UI/CLI/API/Liquid-AI/external-agent write passes through the mutation pipeline and records an action. Liquid's workspace history is a workspace VCS over its own data, not a project-code VCS.
- **Performance gates** — benchmarks assert the `PERFORMANCE.md` targets/budgets relevant to the current milestone. M1 Loop Agent benchmarks use deterministic fixture/stub-model runs and exclude model latency and tool runtime from Watershed overhead. Failures block release.
- **Coverage gate** — from M1 (first real crate logic), `cargo llvm-cov nextest --workspace --fail-under-lines 95` enforces **≥95% line coverage**; merge blocks below it. Generated/FFI/CLI-arg-glue code may be excluded (`#[coverage(off)]` or llvm-cov ignore config) so the threshold measures meaningful logic, not boilerplate; region/function coverage are tracked as secondary signals (ADR-0022).

## M0 tests / pass-fail checks

M0 is a readiness milestone. It passes when:

- placeholder crates compile;
- CI runs on Linux + macOS;
- `cargo fmt --check` passes;
- `cargo clippy` passes;
- `cargo nextest run` is green (deterministic, process-isolated);
- the dependency-hygiene gate (`cargo audit` + `cargo deny`) passes;
- docs links pass the `lychee` link gate and HTML render checks pass;
- the coverage harness (`cargo llvm-cov`) runs in CI; the **≥95% line-coverage gate is enforced from M1** (ADR-0022), not over the empty M0 scaffold;
- the D-015 fixture suite is specified and deterministic through a stub model: `smoke-loop`, `hello-loop` and sandbox-negative fixtures;
- `smoke-loop` is the minimal first gate: one phase, one tool and one instruction, with the smallest byte-stable golden event stream;
- `hello-loop` is the coverage-driven showcase golden: at least two phases, at least two tools spanning predefined-command vs own-script and read-only vs declared write scope, one allowed-parameter case, at least two phase-bound instructions, data plus trigger/refresh connections, and one subloop definition referenced at least twice to prove recursion, reuse, distinct runtime `loop_id` values and `parent_loop_id`;
- sandbox-negative fixtures cover forbidden write, forbidden network including hostname/DNS attempts, forbidden environment inheritance/secret reads, out-of-phase tool, protected-path access, symlink traversal including create-through-symlink ancestors and interpreter misuse attempts and must be rejected;
- the later M0 scaffold checks in exact golden JSONL stream files that follow the D-015 fixture contract below; executable headless validation with `loop run <name> --emit jsonl` begins when the M1 runtime exists;
- the policy compiler contract has expected-output fixtures for Linux and macOS targets using the `SECURITY.md` policy artifact contract;
- no M0 implementation task requires Codex to choose protocol transport, script schema, sandbox depth, crate layout, CLI shape or D-015 fixture strategy, coverage and invocation contract.

M0 fails if any of those decisions remain implicit.

## D-015 fixture contract

The M0 scaffold checks in fixture registry roots and expected JSONL stream files; M1 executes them. Each fixture registry root uses the Loop Agent V-Spec v0 container: recursive `.yaml`/`.yml` files, one YAML 1.2 document and one top-level block per file, with identity from block `id`/`name`. This docs packet fixes the suite, coverage dimensions, determinism requirements and invocation contract; the scaffold owns the literal fixture payload files. Expected streams use the `PROTOCOL.md` v0 envelope, fixed lowercase path-safe `session_id` tokens, fixed fixture IDs, fixed fixture timestamps and monotonic per-session `sequence`, serialized with `PROTOCOL.md` canonical event JSONL bytes. Coverage assertions use the public payload fields in `PROTOCOL.md` (`loop_definition_id`, `tool_kind`, `read_scope`, `write_scope`, `allowed_parameters`, `network_access`, `instruction_ids`, `connection_ids` and `connection_kinds`), not fixture-private names. Connection coverage asserts the `PROTOCOL.md` pairing rule: `connection_ids[i]` and `connection_kinds[i]` describe the same registry-resolved Step `connection_refs[i]` entry. Script-definition assertions are separate from golden event-stream assertions: the `hello-loop` source fixture must include one predefined-command tool using the v0 `command.command_id` plus literal `command.argv` shape and one allowed parameter with a typed, constrained value, and one own-script tool using the v0 `script_runtime: posix-sh` and `script_body` fields from the Loop Agent V-Spec. Those fields are not public event payload fields unless `PROTOCOL.md` later adds them.

- `smoke-loop`: the checked-in stream uses this event-type order: `session.started`, `loop.started`, `phase.entered`, `step.started`, `message.delta`, `message.completed`, `tool.started`, `tool.completed`, `step.completed`, `loop.completed`, `session.completed`. Payloads identify one phase, one instruction and one read-only predefined-command tool with no network or writes by those protocol fields.
- `hello-loop`: starts and ends with the same session/loop lifecycle shape as `smoke-loop`; includes at least two `phase.entered` events, at least two tool runs covering predefined-command and own-script tools by public `tool_kind`, read-only and declared write scope, one allowed-parameter case, at least one `tool.progress`, at least two phase-scoped instruction payloads, data and trigger/refresh connection ids/kinds using the protocol payload fields above, and one subloop definition reused at least twice. Each subloop invocation has a distinct runtime `loop_id`, the same payload `loop_definition_id` and `parent_loop_id` set to the containing runtime loop id.
- Sandbox-negative fixtures: tiny loops for forbidden write, forbidden network including hostname/DNS attempts, forbidden environment inheritance/secret reads, out-of-phase tool, protected-path access, symlink traversal including create-through-symlink ancestors and interpreter misuse through an allowed command. Each expected stream rejects before side effects and ends with `error`, `loop.failed` and `session.failed`; if a tool launch was attempted, it also contains `tool.failed`. Negative streams must not contain `tool.completed`, `loop.completed` or `session.completed`.

## Liquid MVP pass/fail (M3)

In addition to the full M3 DoD in `PLAN.md`, the Liquid MVP must prove: every UI/CLI/API mutation records an action; an external-agent mutation can be reverted; the action history can show actor, target, operation and diff/patch; and the workspace can be restored to a previous checkpoint/snapshot.

## AgentPulse as a quality gate

The same metrics AgentPulse reports (rework ratio, first-attempt success rate, cost-per-productive-outcome) are tracked in CI on golden loops once their formulas are decided. The formulas are an open decision until M2 planning.

## CI

- Run on Linux + macOS.
- Mandatory gates: `cargo fmt --check`, `cargo clippy`, `cargo nextest run` (deterministic, process-isolated tests), `cargo audit` + `cargo deny` (dependency hygiene, see `SECURITY.md`), and the `lychee` docs link gate + HTML render check (`pnpm run docs:render-check`; all M0, ADR-0021, ADR-0043, ADR-0045); plus **≥95% line coverage** via `cargo llvm-cov nextest --workspace --fail-under-lines 95` from M1 (ADR-0022).
- Block merge on: failing tests, line coverage below 95% (from M1), failing dependency-hygiene gates (`cargo audit`/`cargo deny`), failing Linux sandbox boundary tests once enforcement lands, missed macOS policy-artifact parity checks, missed performance gates for the current milestone, and docs link (`lychee`)/HTML validation failures.
- M0 CI starts with compile + `cargo fmt --check` + `cargo clippy` + `cargo nextest` unit tests + `cargo audit`/`cargo deny` + `lychee` docs link validation + `pnpm run docs:render-check`; M1 must add the ≥95% coverage gate, D-015 golden-loop diffs, the Loop Agent budgets in `PERFORMANCE.md`, Linux sandbox-negative enforcement tests and macOS policy-artifact parity checks.
