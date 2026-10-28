# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]

### Added

- Product, protocol, security, testing and contributor documentation with OSS governance templates.
- Rust workspace and Flow Agent M1: standalone CLI, canonical events, script registry, deterministic fixtures, local session logs and policy artifacts.
- Flow Agent M1.1 practical-execution foundations: provider integration, authentication, Conversations, authoring, cancellation and durable recovery.
- Contributor harness and mandatory cross-platform CI gates; canonical commands and policy live in `TESTING.md` and `.github/workflows/ci.yml`.
- Liquid's agent surface on macOS, not yet released: the Meta-Harness core runs pi behind an agent-neutral session model with tool approvals, Watershed's pi extension asks before each tool call, and a Flutter client over flutter_rust_bridge shows the chat, Trajectory, Diff and shortcuts with the Liquid key. A Dart build hook compiles the Rust bridge, so building Liquid needs no CocoaPods (ADR-0213).

### Changed

- Liquid clients use Flutter over the Rust core on desktop and Android under binding stability invariants: no Workspace work on the Dart main isolate, bounded delta streams, viewport virtualization, durable local commit before acknowledgement and sync, an established CRDT library and owned disposal (ADR-0182).
- Before the first release, Flow Agent removed `flow import` and compatibility for automatic flat-session migration, `flow-conversation-entry-v0`, `flow-run-log-record-v0`, `flow-provider-output-v1`, legacy Phase/Step events and Tool observations.
- M1.2 replaces broad Tool isolation with trusted Tool invocation and native protection of this installation's Flow-owned files. A shared one-shot Executor lifecycle uses Linux Bubblewrap/seccomp or macOS Seatbelt; Ready/Start, durable intent, bounded results and self-protection receipts remain. Mount declarations, runtime-read profiles, network policy, process/thread capacity and the systemd/cgroup service dependency are removed without compatibility aliases. Version/platform download packaging is implemented; publication and final acceptance remain pending ([status](PLAN.md#m12--flow-agent-os-isolation)).
- Controlled Run and Resume returns preserve operation, writer-finalization and ownership-cleanup failures; Drop remains only a best-effort fallback.
- Dev/CI toolchains and dependencies use current stable pins without adding a product Node runtime; version authorities are linked from `TESTING.md`.
- Protocol v0 applies one exclusive JSON container-recursion limit of 128 across wire, constructed-event and canonical-JSON boundaries.
- The M1 runtime satisfies the architecture-hardening entry criteria in `PLAN.md`; M1.1 provider and general subprocess work must preserve those boundaries.
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
