# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]

### Added

- Product, protocol, security, testing and contributor documentation with OSS governance templates.
- Rust workspace and Flow Agent M1: standalone CLI, canonical events, script registry, deterministic fixtures, local session logs and policy artifacts.
- Flow Agent M1.1 practical-execution foundations: provider integration, authentication, Conversations, authoring, cancellation and durable recovery.
- Contributor harness and mandatory cross-platform CI gates; canonical commands and policy live in `TESTING.md` and `.github/workflows/ci.yml`.

### Changed

- Before the first release, Flow Agent removed `flow import` and compatibility for automatic flat-session migration, `flow-conversation-entry-v0`, `flow-run-log-record-v0`, `flow-provider-output-v1`, legacy Phase/Step events and Tool observations.
- M1.2 uses a Flow-owned one-shot Executor and official Ubuntu Bubblewrap/seccomp backend with exact descriptor-backed mounts, named runtime-read profiles, readiness before Run reservation and a persisted canonical enforcement receipt. All productive Tool execution, including administrator-owned Custom Executors, is limited to Ubuntu 24.04 x64; other platforms fail closed (ADR-0146, ADR-0160, ADR-0161).
- Controlled Run and Resume returns preserve operation, writer-finalization and ownership-cleanup failures; Drop remains only a best-effort fallback (ADR-0077).
- Dev/CI uses exact Node 24.20.0 LTS and pnpm 11.24.0 pins without adding a product Node runtime (ADR-0080).
- Protocol v0 applies one exclusive JSON container-recursion limit of 128 across wire, constructed-event and canonical-JSON boundaries (ADR-0089).
- The M1 runtime satisfies the architecture-hardening entry criteria in `PLAN.md`; M1.1 provider and general subprocess work must preserve those boundaries (ADR-0079).
- M1 provider context fixed as deterministic, cache-stable `flow-context-v0`, with durable history retained outside the bounded provider projection and post-M1 compaction/retrieval preserved (ADR-0058).
- Provider requests use an opaque Conversation/model-scoped cache key and durably retain optional bounded input, output, cache-read and cache-write token counters without assigning currency cost (ADR-0129).
- Browser authentication deliberately keeps one IPv4 loopback listener and presents the existing device-code command as its fallback when the browser callback cannot complete (ADR-0130).
- Flow Agent no longer exposes Conversation deletion or the unused unbounded Rust session listing; operators remove retained data outside Flow Agent (ADR-0132, ADR-0133).
- `flow reconcile-tool <conversation-id> <run-session-id> --result <file|->` settles exactly one derived uncertain Tool attempt without redispatch or an attempt-id argument (ADR-0134, ADR-0140).
- Productive Tools run without an extra warning or confirmation under their configured Building-Block limits and Executor boundary (ADR-0135).
- OAuth callback input, Responses output items and provider failures follow the reachable bounded contracts selected by ADR-0137–ADR-0139.
- M1 local events use serial authoritative append before a capacity-one, caller-owned, non-blocking high-watermark notification; receivers replay by sequence and the core owns no arbitrary output transport (ADR-0059, ADR-0062).
- M1 is the Flow Agent deterministic runtime foundation: fixture-bounded execution and in-process policy emulation are explicit; real providers and tools belong to M1.1, and OS isolation belongs to M1.2 (ADR-0075, ADR-0076).
- The complete unreleased execution domain uses Flow Agent, Flow, Subflow, `flow.*`, `flow_id`, `flow-context-v0`, `flow-agent*`, `flow-agent-cli`, `flow` and `.flow`, without legacy terminology aliases or vocabulary migration (ADR-0074).
- Canonical registry serialization is deterministic UTF-8 JSON of the validated, resolved building-block model.
- CI actions are pinned, Windows is included, and release-mode performance observations are retained without estimated timing or RSS gates.
- Licensing is `AGPL-3.0-only`; contributions use DCO without a CLA.
