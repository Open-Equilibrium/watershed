# Meta-Harness

Planned for M2; see the [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html). The Liquid agent-surface slice in [PLAN.md](../PLAN.md) builds its core first, embedded in Liquid (ADR-0184).

Native targets and current availability follow [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — the Rust core: starts the pinned T3 Code server and speaks its client protocol (ADR-0281, ADR-0292); `host.rs` holds the server and projects, `thread.rs` a followed thread, `git.rs` Git through T3, `t3/` the server, protocol client and thread projection.
- `t3.lock` — the pinned T3 release and each archive's SHA-256; `scripts/install-t3.mjs` installs it.

Contract tests run against that release with a scripted ACP agent: `META_HARNESS_T3=<pinned t3> cargo test -p meta-harness-core --test t3_contract`.
