# M1.2 Executor Startup Evidence

The workload/report consumer follows the accepted Flow-owned write protection contract in [SECURITY.md](../../SECURITY.md#flow-agent-security-contract). Linux and Mac replacement observations are available below; [TESTING.md](../../TESTING.md#native-verification-status) owns native acceptance status.

This file is the single source for the M1.2 startup workload, evidence and regression policy. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

The evidence is independent of the closed M1.1 matrix and uses schema `flow-m12-executor-startup-v0`. Each sample runs in a fresh measurement child and invokes the deterministic `/bin/echo` Tool once with no arguments or environment through the selected and prepared Executor. The exact successful result is exit code `0`, stdout containing one LF and empty stderr. The existing 5,000 ms Tool deadline is a liveness bound, not a performance threshold. Mandatory Flow-owned write protection covers this installation's admitted Flow-owned file inventory; there is no runtime profile or process/thread capacity input.

CI builds each host's native release Executor in `target/m12-standard` and measures directly on that host, without a container. Before acceptance and measurement, CI copies the release bytes into a fresh, single-link installation and records its absolute path as `M12_INSTALLED_EXECUTOR`. Each measurement controller is likewise copied to its temporary installation, outside the measured interval; Cargo's potentially hardlinked outputs are not admitted installed programs. The required absolute Executor path is registered and preflighted in the child's isolated configuration before the interval. The measured path then independently selects and prepares the Executor again. The unadjusted `executor_elapsed_ns` interval covers that readiness, canonical invocation and Flow-owned write protection preparation, the one-shot Executor/Tool lifecycle through its terminal result, and exact request-hash and policy-digest-bound receipt validation. It does not infer complete hostile-descendant cleanup. The protocol carries no independent Tool clock, so the evidence does not invent or subtract one.

Metadata and aggregate/failure inputs declare `self_protection_required: true`; each measured sample retains receipt-derived `self_protection_active: true` alongside its unadjusted elapsed time. Schema mismatch, missing/inactive guard evidence and legacy child fields reject rather than becoming ignored settings. Every warmup and measured child is validated before its observation is accepted. Reports retain environment metadata, all 30 raw observations plus p50, p95 and maximum; obsolete runtime-profile, process-capacity and systemd/cgroup metadata are not emitted. Report metadata declares the requirement, not a successful protection observation before any child runs.

Each Ubuntu 24.04 x64 and macOS 27 ARM64 CI job runs five warmups followed by 30 measured children, one process at a time. For a local run, set `M12_INSTALLED_EXECUTOR` to the absolute path of your installed native Executor; do not supply a hardlinked Cargo output directly.

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

Each host keeps its latest report. Both use Rust 1.99.0, five warmups and 30 samples, all with active protection and `complete: true`; retained raw payloads and recomputed aggregates agree:

| Host | CI artifact | Revision / native job | Artifact | p50 (ns) | p95 (ns) | Maximum (ns) |
|---|---|---|---:|---:|---:|---:|
| Ubuntu 24.04 x64 | `m12-executor-startup-evidence-ubuntu-24.04` | `2b59f10`, [Linux 110523397292](https://github.com/Open-Equilibrium/watershed/actions/runs/36908028880/job/110523397292) | `11185539504` | 51,728,796 | 51,897,403 | 52,003,125 |
| macOS 27 ARM64 | `m12-executor-startup-evidence-xcode-27` | `2b59f10`, [Mac 110523397025](https://github.com/Open-Equilibrium/watershed/actions/runs/36908028880/job/110523397025) | `11186380547` | 324,787,542 | 438,812,250 | 463,842,959 |

The Linux job uses image `20260927.320.1` and four logical Intel Xeon 6973P-C CPUs with 16,765,374,464 bytes of memory. The Mac job uses image `20260928.0222.1` and three logical CPUs; CPU model and memory are unavailable. Retain each host's raw samples and distribution separately; do not pool them or treat one as evidence for the other. Hardware, image and toolchain differences prevent attributing changes solely to code. Compile checks and legacy container measurements do not supply replacement timing or Flow-owned write protection evidence. [TESTING.md](../../TESTING.md#native-verification-status) owns tested-tree bindings and native acceptance.

### Integrity and regression policy

CI enforces the deterministic workload, bounded child report, exact successful terminal result, valid Flow-owned write protection receipt, complete report and artifact retention. Timing remains observable regression evidence and a performance KPI; no timing, throughput or memory observation alone fails the build.

Review startup changes against the one-shot architecture target and the retained Executor distribution. Address a clear, maintainable regression without weakening Flow-owned write protection, lifecycle bounds or correctness; otherwise record the evidence and architectural tradeoff through the decision flow before changing the target or workload.
