# Meta-Harness

M2 in progress; product and runtime shape: [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html), native targets and availability: [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — the Rust core: the service and its API (`service.rs`, `api.rs`), the client Liquid and `meta` use (`remote.rs`), the central configuration (`config.rs`, `config/homes.rs`), the pinned T3 Code server and its client protocol (`t3/`, ADR-0281, ADR-0292), and the host, thread and Git model T3 runs (`host.rs`, `thread.rs`, `git.rs`).
- `meta-harness-cli/` — `meta`: the service, the configuration commands and the login service (`login.rs`).
- `install.sh` — the bundle's installer; `scripts/package-meta-harness.mjs` packs a bundle.
- `t3.lock` — the pinned T3 release and each archive's SHA-256; `scripts/install-t3.mjs` installs it.

## Run from the repository

```sh
t3="$(node scripts/install-t3.mjs "$PWD/target/t3")"
META_HARNESS_T3="$t3" cargo run -p meta-harness-cli -- serve
cargo run -p meta-harness-cli -- status
```

`meta help` lists every command. Without `META_HARNESS_T3`, `meta serve` runs the T3 release installed beside it. The data folder is `META_HARNESS_HOME`, else `~/.meta-harness`.

Contract tests run against the pinned release with a scripted ACP agent, through the service: `META_HARNESS_T3=<pinned t3> cargo nextest run --config 'target."cfg(all())".runner = ["node", "<repository>/scripts/run-isolated-rust-test.mjs"]' -p meta-harness-core --test t3_contract`, through the isolation runner as every Rust test (TESTING.md).
