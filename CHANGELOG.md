# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning: [SemVer](https://semver.org/) once releases exist.

## [Unreleased]

### Added

- Product, protocol, security, testing and contributor documentation with OSS governance templates.
- Rust workspace and Flow Agent M1: standalone CLI, canonical events, script registry, deterministic fixtures, local session logs and policy artifacts.
- Contributor harness and mandatory cross-platform CI gates; canonical commands and policy live in `TESTING.md` and `.github/workflows/ci.yml`.

### Changed

- M1 provider context fixed as deterministic, cache-stable `flow-context-v0`, with durable history retained outside the bounded provider projection and post-M1 compaction/retrieval preserved (ADR-0058).
- M1 local events use serial authoritative append before a capacity-one, caller-owned, non-blocking high-watermark notification; receivers replay by sequence and the core owns no arbitrary output transport (ADR-0059, ADR-0062).
- Flow Agent M1 sandboxing uses deterministic in-process policy enforcement; OS isolation remains post-M1.
- The unreleased execution product and identity seams use Flow Agent, `flow-agent*`, `flow-agent-cli`, `flow` and `.flow`; generic Flow workflow, event and context vocabulary remains unchanged (ADR-0073).
- Canonical registry serialization is deterministic UTF-8 JSON of the validated, resolved building-block model.
- CI actions are pinned, Windows is included, and timing-sensitive performance tests run optimized outside coverage.
- Licensing is `AGPL-3.0-only`; contributions use DCO without a CLA.
