# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]

### Added

- Product, protocol, security, testing and contributor documentation with OSS governance templates.
- Rust workspace and Flow Agent M1: standalone CLI, canonical events, script registry, deterministic fixtures, local session logs and policy artifacts.
- Flow Agent M1.1 practical-execution foundations: provider integration, authentication, Conversations, authoring, cancellation and durable recovery.
- Contributor harness and mandatory cross-platform CI gates; canonical commands and policy live in `TESTING.md` and `.github/workflows/ci.yml`.
- Liquid's agent surface on macOS, not yet released: the Meta-Harness core runs pi behind an agent-neutral session model, Watershed's pi extension marks each turn's checkpoints, and a Flutter client over flutter_rust_bridge shows the chat, Trajectory, Diff and shortcuts with the Liquid key. A Dart build hook compiles the Rust bridge, so building Liquid needs no CocoaPods (ADR-0213).
- Liquid's Git integration for the agent's project on macOS: per-turn checkpoints with their Diff and revert, branches and stashes, whole-file commits, push, fast-forward pull and fetch, GitHub pull requests whose text the Chat's model writes, and removing worktrees in Settings (ADR-0220, ADR-0234).
- Liquid's local Workspace on macOS: the `liquid-core` mutation pipeline over Automerge, an encrypted SQLite replica with its key in the keychain, and Pages with Text Blocks, Sections and Tables whose rows, fields and cells can be edited (ADR-0237, ADR-0238).

### Changed

- Meta-Harness leaves governance to each harness: Watershed's own tool approvals are removed, and Liquid shows only the approvals a harness itself asks for (ADR-0252).
- Liquid clients use Flutter over the Rust core on desktop and Android under binding stability invariants: no Workspace work on the Dart main isolate, bounded delta streams, viewport virtualization, durable local commit before acknowledgement and sync, an established CRDT library and owned disposal (ADR-0182).
- Before the first release, Flow Agent removed `flow import` and compatibility for automatic flat-session migration, `flow-conversation-entry-v0`, `flow-run-log-record-v0`, `flow-provider-output-v1`, legacy Phase/Step events and Tool observations.
- M1.2 replaces broad Tool isolation with trusted Tool invocation and native protection of this installation's Flow-owned files. A shared one-shot Executor lifecycle uses Linux Bubblewrap/seccomp or macOS Seatbelt; Ready/Start, durable intent, bounded results and Flow-owned write protection receipts remain. Mount declarations, runtime-read profiles, network policy, process/thread capacity and the systemd/cgroup service dependency are removed without compatibility aliases. Version/platform download packaging is implemented; publication and final acceptance remain pending ([status](PLAN.md#m12--flow-owned-write-protection)).
- Controlled Run and Resume returns preserve operation, writer-finalization and ownership-cleanup failures; Drop remains only a best-effort fallback.
- Dev/CI toolchains and dependencies use current stable pins without adding a product Node runtime; version authorities are linked from `TESTING.md`.
- Protocol v0 applies one exclusive JSON container-recursion limit of 128 across wire, constructed-event and canonical-JSON boundaries.
- The M1 runtime satisfies the [architecture boundaries](docs/architecture.md#architecture-boundaries); M1.1 provider and general subprocess work must preserve those boundaries.
- M1 provider context fixed as deterministic, cache-stable `flow-context-v0`, with durable history retained outside the bounded provider projection and post-M1 compaction/retrieval preserved.
- Provider requests use an opaque Conversation/model-scoped cache key and durably retain optional bounded input, output, cache-read and cache-write token counters without assigning currency cost.
- Browser authentication deliberately keeps one IPv4 loopback listener and presents the existing device-code command as its fallback when the browser callback cannot complete.
- Flow Agent no longer exposes Conversation deletion or the unused unbounded Rust session listing; operators remove retained data outside Flow Agent.
- `flow reconcile-tool <conversation-id> <run-session-id> --result <file|->` settles exactly one derived uncertain Tool attempt without redispatch or an attempt-id argument.
- Productive Tools run without an extra warning or confirmation under their configured Building-Block limits and Executor boundary (ADR-0135).
- OAuth callback input, Responses output items and provider failures follow the reachable bounded contracts in `PROTOCOL.md`.
- M1 local events use serial authoritative append before a capacity-one, caller-owned, non-blocking high-watermark notification; receivers replay by sequence and the core owns no arbitrary output transport.
- M1 is the Flow Agent deterministic runtime foundation: fixture-bounded execution and in-process policy emulation are explicit; real providers and tools belong to M1.1, and OS isolation belongs to M1.2.
- The complete unreleased execution domain uses Flow Agent, Flow, Subflow, `flow.*`, `flow_id`, `flow-context-v0`, `flow-agent*`, `flow-agent-cli`, `flow` and `.flow`, without legacy terminology aliases or vocabulary migration.
- Canonical registry serialization is deterministic UTF-8 JSON of the validated, resolved building-block model.
- CI actions are pinned; native gates follow [TESTING.md](TESTING.md), and retained performance observations follow [PERFORMANCE.md](PERFORMANCE.md).
- Licensing is `AGPL-3.0-only`; contributions use DCO without a CLA.
