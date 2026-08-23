# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]

### Added

- Product, protocol, security, testing and contributor documentation with OSS governance templates.
- Rust workspace and Flow Agent M1: standalone CLI, canonical events, script registry, deterministic fixtures, local session logs and policy artifacts.
- Flow Agent M1.1 practical execution: productive provider and Tool execution, authentication, Conversations, authoring, cancellation and durable recovery; M1.2 remains the separate OS-isolation stage.
- Contributor harness and mandatory cross-platform CI gates; canonical commands and policy live in `TESTING.md` and `.github/workflows/ci.yml`.

### Changed

- M1.2 uses a Flow-owned one-shot Executor protocol and default-installed Sandbox Executor on Ubuntu/macOS; custom integrations remain administrator-owned, while Windows and positive Tool egress are post-MVP (ADR-0146).
- Controlled Run and Resume returns preserve operation, writer-finalization and ownership-cleanup failures; Drop remains only a best-effort fallback (ADR-0077).
- Dev/CI uses exact Node 24.19.0 LTS and pnpm 11.22.0 pins without adding a product Node runtime (ADR-0080).
- Protocol v0 applies one exclusive JSON container-recursion limit of 128 across wire, constructed-event and canonical-JSON boundaries (ADR-0089).
- The M1 runtime satisfies the architecture-hardening entry criteria in `PLAN.md`; M1.1 provider and general subprocess work must preserve those boundaries (ADR-0079).
- M1 provider context fixed as deterministic, cache-stable `flow-context-v0`, with durable history retained outside the bounded provider projection and post-M1 compaction/retrieval preserved (ADR-0058).
- Provider requests use an opaque Conversation/model-scoped cache key and durably retain optional bounded input, output, cache-read and cache-write token counters without assigning currency cost (ADR-0129).
- Browser authentication deliberately keeps one IPv4 loopback listener and presents the existing device-code command as its fallback when the browser callback cannot complete (ADR-0130).
- Flow Agent no longer exposes Conversation deletion or the unused unbounded Rust session listing; operators remove retained data outside Flow Agent (ADR-0132, ADR-0133).
- `flow reconcile-tool <conversation-id> <run-session-id> --result <file|->` settles exactly one derived uncertain Tool attempt without redispatch or an attempt-id argument (ADR-0134, ADR-0140).
- Productive Tools run without an extra warning or confirmation under their configured Building-Block limits; M1.2 remains the OS-isolation boundary (ADR-0135).
- OAuth callback input, Responses output items and provider failures follow the reachable bounded contracts selected by ADR-0137–ADR-0139.
- M1 local events use serial authoritative append before a capacity-one, caller-owned, non-blocking high-watermark notification; receivers replay by sequence and the core owns no arbitrary output transport (ADR-0059, ADR-0062).
- M1 is the Flow Agent deterministic runtime foundation: fixture-bounded execution and in-process policy emulation are explicit; real providers and tools belong to M1.1, and OS isolation belongs to M1.2 (ADR-0075, ADR-0076).
- The complete unreleased execution domain uses Flow Agent, Flow, Subflow, `flow.*`, `flow_id`, `flow-context-v0`, `flow-agent*`, `flow-agent-cli`, `flow` and `.flow`, without legacy terminology aliases or vocabulary migration (ADR-0074); storage migration is defined in [PROTOCOL.md](PROTOCOL.md).
- Canonical registry serialization is deterministic UTF-8 JSON of the validated, resolved building-block model.
- CI actions are pinned, Windows is included, and timing-sensitive performance tests run optimized outside coverage.
- Licensing is `AGPL-3.0-only`; contributions use DCO without a CLA.
