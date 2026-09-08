# M1.2 Executor Startup Evidence

**Current legacy implementation:** This workload measures the existing Ubuntu Executor, not the approved replacement in [SECURITY.md](../../SECURITY.md#accepted-flow-agent-security-target). Its exact mounts, process capacity and cleanup checks remain current-code evidence until coherent migration; they are not future general-isolation guarantees.

This file is the single source for the M1.2 startup workload, evidence and regression policy. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

The evidence is independent of the closed M1.1 matrix and uses schema `flow-m12-executor-startup-v0`. Each sample runs in a fresh measurement child and invokes the deterministic `/bin/echo` Tool once with no arguments or environment and process/thread capacity `8` through the selected and prepared Executor boundary. This is a fixed workload input, not a product default or performance threshold. The exact runtime profile exposes only the fixed runtime objects declared by the ready Executor plus the read-only workspace root.

CI stages the static release output as the root-owned, single-link `/usr/local/libexec/watershed/flow-executor` before reporting begins. Each measurement controller is copied to a separate single-link executable in its temporary installation, outside the measured interval; Cargo's potentially hardlinked example output is not an admitted installed program. The required absolute Executor path is registered and preflighted in the child's isolated configuration before the interval. The measured path then independently selects and prepares the Executor again. The unadjusted `executor_elapsed_ns` interval covers that readiness, canonical policy and capability preparation, transient systemd scope and cgroup creation, the one-shot Executor and Sandbox lifecycle, complete cleanup, and validation of the terminal Tool result and enforcement receipt. Reports retain the configured capacity, relevant systemd/cgroup host metadata, all 30 raw observations plus p50, p95 and maximum. The protocol carries no independent Tool clock, so the evidence does not invent or subtract one.

The fixed Ubuntu 24.04 x64 CI job runs five warmups followed by 30 measured children, one process at a time:

```sh
mkdir -p target/m12-startup
install -d -m 0755 /usr/local/libexec/watershed
install -m 0755 target/x86_64-unknown-linux-musl/release/flow-executor \
  /usr/local/libexec/watershed/flow-executor
cargo run --locked -p flow-agent-core --release \
  --features m12-startup-evidence --example m12_executor_startup \
  -- --executor /usr/local/libexec/watershed/flow-executor \
  > target/m12-startup/m12-executor-startup.jsonl
```

The dedicated artifact is uploaded even when collection fails. A child or lifecycle failure still produces metadata, one terminal workload-failure record and a failed summary before the process exits nonzero.

## Evidence and enforcement

The retained `m12-executor-startup-evidence` artifact from [CI run 33712944225](https://github.com/Open-Equilibrium/watershed/actions/runs/33712944225) covers commit `b54e4e8ddedb79b20efd13b0d069959b7b5891d4`. Its 30 observations used Rust 1.98.0 on the `ubuntu24` runner image `20260831.293.1` with four logical Intel Xeon Platinum 8573C CPUs and 16,765,370,368 bytes of memory, inside the pinned Ubuntu 24.04 contract image. The Executor distribution recorded p50 `51,980,881 ns`, p95 `72,140,948 ns` and maximum `72,296,758 ns`, followed by `complete: true`.

After installed-program admission was added, [CI run 34220164904](https://github.com/Open-Equilibrium/watershed/actions/runs/34220164904) retained 30 observations for `6d74313c2185446999c6c1e51c1e3e086d183871`: p50 `72,375,122 ns`, p95 `82,589,421 ns`, maximum `82,642,268 ns`, and `complete: true`. This run used Rust 1.98.1, the same runner/contract images, four logical AMD EPYC 7763 CPUs and 16,766,414,848 bytes of memory. The changed CPU and toolchain prevent attributing the timing difference solely to admission. Both observations measure the legacy boundary, not the replacement native guard.

CI enforces the deterministic workload, bounded child report, exact successful terminal result, valid isolation receipt, complete report and artifact retention. Timing remains observable regression evidence and a performance KPI; no timing, throughput or memory observation alone fails the build.

Review startup changes against the one-shot architecture target and the retained Executor distribution. Address a clear, maintainable regression without weakening isolation, cleanup or correctness; otherwise record the evidence and architectural tradeoff through the decision flow before changing the target or workload.
