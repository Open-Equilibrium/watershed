# M1.2 Executor Startup Budget

This file is the single source for the M1.2 startup workload, evidence and gate. Protocol, security and platform behavior remain canonical in `PROTOCOL.md`, `SECURITY.md` and `TESTING.md`.

## Workload and method

Reuse `P:runner_four_noop_launches` from `M1_1_BUDGETS.md`: one release-mode sample starts `/usr/bin/true` four times sequentially through the direct M1.1 Tool runner with an empty environment and requires four exact successful terminal results with empty output. Useful Tool work and executable lookup are excluded. The later M1.2 gate replaces only the launch path with four one-shot Default Sandbox Executor invocations, so the p95 difference bounds added Flow Agent protocol, companion-process and Sandbox-backend startup without duplicating useful Tool work.

The fixed Ubuntu 24.04 x64 CI job runs five warmups and 30 measured fresh-child samples, one process at a time:

```sh
cargo run --locked -p flow-agent-core --release --features m11-budget-evidence --example m11_budgets > target/m11-budgets/m11-budgets.jsonl
```

## Direct-runner baseline

GitHub Actions run [32953566534](https://github.com/Open-Equilibrium/watershed/actions/runs/32953566534) measured commit `d48147874dad793f93a44b105fd42198319196cf` on runner image `ubuntu24` version `20260816.277.1`, with four logical CPUs, an Intel Xeon Platinum 8370C and 16,765,378,560 bytes of memory. The `flow-m11-budget-v0` artifact reported 30 passing samples:

| Aggregate | Four launches |
|---|---:|
| p50 | 5.291454 ms |
| p95 | 5.369185 ms |
| maximum | 5.374238 ms |

## Gate

The p95 ceiling and allowed added overhead are pending [D-065](../../docs/decisions/open-decisions.html#d-065). Productive M1.2 Executor implementation remains blocked until that decision is recorded in `docs/adr/ADR-LOG.md`.
