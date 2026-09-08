# M1.2 Executor Startup Evidence

**Migration status:** The workload/report consumer follows the accepted own-file protection contract in [SECURITY.md](../../SECURITY.md#accepted-flow-agent-security-target). Native integration and replacement measurements remain pending; the retained observations below measure the legacy boundary, not general host containment or the replacement guard.

This file is the single source for the M1.2 startup workload, evidence and regression policy. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

The evidence is independent of the closed M1.1 matrix and uses schema `flow-m12-executor-startup-v0`. Each sample runs in a fresh measurement child and invokes the deterministic `/bin/echo` Tool once with no arguments or environment through the selected and prepared Executor boundary. The exact successful result is exit code `0`, stdout containing one LF and empty stderr. The existing 5,000 ms Tool deadline is a liveness bound, not a performance threshold. Mandatory native protection covers this installation's admitted own-file inventory; there is no runtime profile or process/thread capacity input.

CI builds each host's native release Executor in `target/m12-standard` and measures directly on that host, without a container. Before acceptance and measurement, CI copies the release bytes into a fresh, single-link installation and records its absolute path as `M12_INSTALLED_EXECUTOR`. Each measurement controller is likewise copied to its temporary installation, outside the measured interval; Cargo's potentially hardlinked outputs are not admitted installed programs. The required absolute Executor path is registered and preflighted in the child's isolated configuration before the interval. The measured path then independently selects and prepares the Executor again. The unadjusted `executor_elapsed_ns` interval covers that readiness, canonical invocation and own-file protection preparation, the one-shot Executor/Tool lifecycle through its terminal result, and exact request-hash and policy-digest-bound receipt validation. It does not infer complete hostile-descendant cleanup. The protocol carries no independent Tool clock, so the evidence does not invent or subtract one.

Metadata and aggregate/failure inputs declare `self_protection_required: true`; each measured sample retains receipt-derived `self_protection_active: true` alongside its unadjusted elapsed time. Schema mismatch, missing/inactive guard evidence and legacy child fields reject rather than becoming ignored settings. Every warmup and measured child is validated before its observation is accepted. Reports retain environment metadata, all 30 raw observations plus p50, p95 and maximum; obsolete runtime-profile, process-capacity and systemd/cgroup metadata are not emitted. Report metadata declares the requirement, not a successful protection observation before any child runs.

Each Ubuntu 24.04 x64 and macOS 26 ARM64 CI job runs five warmups followed by 30 measured children, one process at a time. For a local run, set `M12_INSTALLED_EXECUTOR` to the absolute path of your installed native Executor; do not supply a hardlinked Cargo output directly.

```sh
mkdir -p target/m12-startup
cargo run --locked -p flow-agent-core --release \
  --features m12-startup-evidence --example m12_executor_startup \
  -- --executor "$M12_INSTALLED_EXECUTOR" \
  > target/m12-startup/m12-executor-startup.jsonl
```

The dedicated `m12-executor-startup-evidence-${matrix.os}` artifact is uploaded even when collection fails. Metadata identifies the actual OS, architecture, toolchain and runner image; `reference_platform` recognizes both native OS/architecture pairs, while runtime admission checks the supported release. Missing hardware metadata remains unavailable, not inferred. A child or lifecycle failure still produces metadata, one terminal workload-failure record and a failed summary before the process exits nonzero.

## Evidence and enforcement

### Native host observations

Replacement observations remain pending independently for both hosts:

| Host | CI artifact | Observation |
| --- | --- | --- |
| Ubuntu 24.04 x64 | `m12-executor-startup-evidence-ubuntu-24.04` | Pending |
| macOS 26 ARM64 | `m12-executor-startup-evidence-macos-26` | Pending |

Retain each host's raw samples and distribution separately; do not pool them or treat one as evidence for the other. Compare only like-for-like host observations. Compile checks and legacy measurements do not supply replacement timing or native protection evidence.

### Historical container observations

The retained `m12-executor-startup-evidence` artifact from [CI run 33712944225](https://github.com/Open-Equilibrium/watershed/actions/runs/33712944225) covers commit `b54e4e8ddedb79b20efd13b0d069959b7b5891d4`. Its 30 observations used Rust 1.98.0 on the `ubuntu24` runner image `20260831.293.1` with four logical Intel Xeon Platinum 8573C CPUs and 16,765,370,368 bytes of memory, inside the pinned Ubuntu 24.04 contract image. The Executor distribution recorded p50 `51,980,881 ns`, p95 `72,140,948 ns` and maximum `72,296,758 ns`, followed by `complete: true`.

After installed-program admission was added, [CI run 34220164904](https://github.com/Open-Equilibrium/watershed/actions/runs/34220164904) retained 30 observations for `6d74313c2185446999c6c1e51c1e3e086d183871`: p50 `72,375,122 ns`, p95 `82,589,421 ns`, maximum `82,642,268 ns`, and `complete: true`. This run used Rust 1.98.1, the same runner/contract images, four logical AMD EPYC 7763 CPUs and 16,766,414,848 bytes of memory. The changed CPU and toolchain prevent attributing the timing difference solely to admission. Both observations measure the legacy boundary, not the replacement native guard.

After protected-directory inventory admission, [CI run 34227029253](https://github.com/Open-Equilibrium/watershed/actions/runs/34227029253) retained 30 observations for `78d5ca66fabfb80d6ef89616e96ddb2aebeb638e`: p50 `52,383,201 ns`, p95 `72,389,676 ns`, maximum `164,998,986 ns`, and `complete: true`. Rust was 1.98.1, with the same runner/contract images, four logical Intel Xeon Platinum 8573C CPUs and 16,765,378,560 bytes of memory. This precedes combined installed-image alias admission; the changed CPU again prevents attributing differences solely to code. The larger fixed inventory workloads have separate [observations](M1_1_BUDGETS.md#initial-inventory-observations); this startup fixture is not their cost estimate.

### Integrity and regression policy

CI enforces the deterministic workload, bounded child report, exact successful terminal result, valid own-file protection receipt, complete report and artifact retention. Timing remains observable regression evidence and a performance KPI; no timing, throughput or memory observation alone fails the build.

Review startup changes against the one-shot architecture target and the retained Executor distribution. Address a clear, maintainable regression without weakening own-file protection, lifecycle bounds or correctness; otherwise record the evidence and architectural tradeoff through the decision flow before changing the target or workload.
