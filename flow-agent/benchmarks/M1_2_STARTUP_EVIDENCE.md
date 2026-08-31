# M1.2 Executor Startup Evidence

This file is the single source for the M1.2 startup workload, evidence and regression policy. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

The evidence is independent of the closed M1.1 matrix and uses schema `flow-m12-executor-startup-v0`. Each sample runs in a fresh measurement child and invokes the deterministic `/bin/echo` Tool once with no arguments or environment through the selected and prepared Executor boundary. The exact runtime profile exposes only the fixed runtime objects declared by the ready Executor plus the read-only workspace root.

The required absolute Executor path is registered and preflighted in the child's isolated configuration before the interval. The measured path then independently selects and prepares the Executor again. The unadjusted `executor_elapsed_ns` interval covers that readiness, canonical policy and capability preparation, the one-shot Executor and Sandbox lifecycle, and validation of the terminal Tool result and enforcement receipt. Reports retain all 30 raw observations plus p50, p95 and maximum. The protocol carries no independent Tool clock, so the evidence does not invent or subtract one.

The fixed Ubuntu 24.04 x64 CI job runs five warmups followed by 30 measured children, one process at a time:

```sh
mkdir -p target/m12-startup
cargo run --locked -p flow-agent-core --release \
  --features m12-startup-evidence --example m12_executor_startup \
  -- --executor "$PWD/target/x86_64-unknown-linux-musl/release/flow-executor" \
  > target/m12-startup/m12-executor-startup.jsonl
```

The dedicated artifact is uploaded even when collection fails. A child or lifecycle failure still produces metadata, one terminal workload-failure record and a failed summary before the process exits nonzero.

## Evidence and enforcement

The first fixed-platform artifact is pending. CI enforces the deterministic workload, bounded child report, exact successful terminal result, valid isolation receipt, complete report and artifact retention. Timing remains observable regression evidence and a performance KPI; no timing, throughput or memory observation alone fails the build.

Review startup changes against the one-shot architecture target and the retained Executor distribution. Address a clear, maintainable regression without weakening isolation, cleanup or correctness; otherwise record the evidence and architectural tradeoff through the decision flow before changing the target or workload.
