# M1.2 Executor Startup Evidence

This file is the single source for the M1.2 startup workload, evidence and regression policy. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

The direct-runner baseline is independent of the closed M1.1 matrix and uses schema `flow-m12-startup-baseline-v0`. Each sample runs in a fresh measurement child and performs exactly one direct Tool invocation. That invocation starts the fixed no-op child with an empty environment; the child returns one bounded `flow-m12-noop-tool-v0` result containing its independently measured `tool_runtime_ns`.

The outer `runner_elapsed_ns` interval begins immediately before direct-runner handoff and ends only after process launch, terminal classification, reap and stdout/stderr drain have completed. Reports retain all 30 raw pairs and independent p50, p95 and maximum distributions for `runner_elapsed_ns` and `tool_runtime_ns`. Values are unadjusted; no Tool-runtime subtraction or derived overhead value is used.

The fixed Ubuntu 24.04 x64 CI job runs five warmups followed by 30 measured children, one process at a time:

```sh
mkdir -p target/m12-startup
cargo run --locked -p flow-agent-core --release \
  --features m12-startup-evidence --example m12_executor_startup \
  > target/m12-startup/m12-direct-runner-baseline.jsonl
```

The dedicated artifact is uploaded even when collection fails. A child or lifecycle failure still produces metadata, one terminal workload-failure record and a failed summary before the process exits nonzero.

## Evidence and enforcement

The first fixed-runner artifact is pending. CI enforces the deterministic workload, bounded child report, exact successful terminal result, complete report and artifact retention. Timing remains observable regression evidence and a performance KPI; ADR-0158 rejects an estimated absolute startup hard-fail number, so the report records `p95_limit_ns: null` and no timing observation alone fails the build.

Review startup changes against the one-shot architecture target and the retained runner/tool distributions. Address a clear, maintainable regression without weakening isolation, cleanup or correctness; otherwise record the evidence and architectural tradeoff through the decision flow before changing the target or workload.
