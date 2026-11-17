# Meta-Harness

M2 in progress; product and runtime shape: [Meta-Harness V-Spec](../docs/concept/V-Spec_MetaHarness.html), native targets and availability: [PLATFORMS.md](../PLATFORMS.md).

## Layout

- `meta-harness-core/` — the Rust core: the service and its API (`service.rs`, `api.rs`), the client Liquid and `meta` use (`remote.rs`), remote access and pairing (`service/remote.rs`), host links (`hosts.rs`, `tunnel.rs`), the central configuration (`config.rs`, `config/homes.rs`), the pinned T3 Code server and its client protocol (`t3/`, ADR-0281, ADR-0292), and the host, thread and Git model T3 runs (`host.rs`, `thread.rs`, `git.rs`).
- `meta-harness-cli/` — `meta`: the service, the configuration commands, remote access and host links (`hosts.rs`), and the login service (`login.rs`).
- `install.sh` — the bundle's installer.
- `t3.lock` — the pinned T3 release and each archive's SHA-256; `scripts/install-t3.mjs` installs it.

## Install

No release is published yet. A release's `meta-harness-<version>-<platform>.tar.gz` (`darwin-arm64` or `linux-x64`) and `SHA256SUMS` are downloaded in a browser from the [GitHub Releases page](https://github.com/Open-Equilibrium/watershed/releases) and checked before anything is unpacked, as for [Flow Agent](../README.md#download-installation-on-linux-or-macos):

Set `version` to the release's version and `platform` to `darwin-arm64` or `linux-x64` first:

```sh
grep " meta-harness-$version-$platform.tar.gz\$" SHA256SUMS | shasum -a 256 -c -
tar -xzf "meta-harness-$version-$platform.tar.gz"
sh "meta-harness-$version-$platform/install.sh"
```

The installer puts `meta` and the pinned T3 Code into `~/.local/meta-harness` (`--prefix` chooses another folder) without administrator rights and starts the login service (`--no-service` skips it; `meta service install` starts it later). A newer archive installs the same way over the old one, with the same `--prefix`, and restarts the service; `meta uninstall` removes the service and the installation and keeps the data. The service logs to `service.log` in the data folder. On Linux the service keeps running after logout only with lingering ([PLATFORMS.md](../PLATFORMS.md)); on Linux, `sha256sum -c` replaces `shasum -a 256 -c`. `scripts/package-meta-harness.mjs` packs a bundle from a built `meta` and the folder `scripts/install-t3.mjs` unpacks.

## Run from the repository

```sh
t3="$(node scripts/install-t3.mjs "$PWD/target/t3")"
META_HARNESS_T3="$t3" cargo run -p meta-harness-cli -- serve
cargo run -p meta-harness-cli -- status
```

`meta help` lists every command. Without `META_HARNESS_T3`, `meta serve` runs the T3 release installed beside it. The data folder is `META_HARNESS_HOME`, else `~/.meta-harness`.

## Remote access and host links

- `meta remote on [--port N]` listens on this computer's Tailscale addresses, on port 3774 by default, across restarts; `meta remote off` stops it; `meta remote status` shows it.
- `meta invite --device` or `--host` prints a pairing code, valid for five minutes and one use, with the addresses to pair with.
- `meta host agents on|off` accepts agents from other hosts here; it is off until turned on.
- `meta host add ADDRESS CODE [--access LEVEL]` links the host at ADDRESS; `LEVEL` is `read-only`, `approval-required`, `auto-accept-edits`, `auto` (the default) or `full-access`.
- `meta host list`, `meta host remove NAME` (unlinks a host and its T3 link) and `meta host revoke NAME` (refuses a paired client's token from its next connection on) manage them.

A link needs `meta host agents on` on both hosts, then the code from `meta invite --host` on the host to link to, given to `meta host add` on the other. Trust limits: [SECURITY.md](../SECURITY.md#meta-harness-service-and-configuration); mechanism: [V-Spec](../docs/concept/V-Spec_MetaHarness.html#orchestration).

Contract tests run against the pinned release with a scripted ACP agent, through the service: `META_HARNESS_T3=<pinned t3> cargo nextest run --config 'target."cfg(all())".runner = ["node", "<repository>/scripts/run-isolated-rust-test.mjs"]' -p meta-harness-core --test t3_contract`, through the isolation runner as every Rust test (TESTING.md).
