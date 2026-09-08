# Platforms and capabilities

Support is a product-and-capability contract, not a workspace-wide compilation claim. Release 1 requires native execution and verification on the targets below; a target in this table is not evidence that its implementation is available today.

Flow Agent's accepted replacement boundary is [trusted Tools with mandatory native self-protection](SECURITY.md#accepted-flow-agent-security-target) (ADR-0166), not general hostile-Tool containment. [D-063](docs/decisions/open-decisions.html#d-063) retains only the first-release choice on additional independent Flow homes. The existing Ubuntu Sandbox remains the legacy implementation until a coherent migration; it is not the future cross-platform security promise.

| Product / capability | Release 1 native targets | Current implementation |
|---|---|---|
| Flow Agent authoring, Fixture execution and provider-only Flows | Linux x86_64; macOS ARM64 | Implemented; productive provider execution is restricted to Ubuntu 24.04 and macOS 26. |
| Flow Agent Tool execution with mandatory self-protection | Linux x86_64; macOS ARM64 | Replacement not implemented. Legacy Ubuntu 24.04 Sandbox only; macOS Tool execution fails closed. Both native replacement boundaries require proof before the first Flow Agent release. |
| Meta-Harness CLI, service and host-local agent control | Linux x86_64; macOS ARM64 | Not implemented. |
| Liquid desktop client | Linux x86_64; macOS ARM64; Windows 11 x86_64 | Not implemented. |

Linux ARM64 and Windows 11 ARM64 are deferred beyond Release 1. Earlier Windows versions are unsupported. Flow Agent and Meta-Harness have no native Windows support or future Windows-backend commitment. Liquid's mobile and headless capabilities retain their separate contracts in its [V-Spec](docs/concept/V-Spec_Liquid.html); desktop support does not certify them.

Ubuntu 24.04 x86_64 and macOS 26 ARM64 are the current concrete Flow verification targets. Other Linux distributions or OS versions do not acquire a support claim from sharing an architecture or passing compilation. Liquid and Meta-Harness must establish their own runtime evidence, not inherit Flow's verification results.

## Execution and development boundaries

Watershed development targets native Linux x86_64 and macOS ARM64. A Mac can run native development and macOS verification while Linux CI supplies Linux evidence; no local Linux computer is required. Cross-compilation, containers sharing a host kernel, and platform-independent tests do not replace native executor tests on both release targets.

A Linux VM is not the macOS product execution path. Native Apple tools and Metal-based inference must remain possible without placing Flow Agent in a Linux guest; local inference still requires the separately decided trust boundary in [SECURITY.md](SECURITY.md#accepted-post-m11-target-local-inference-and-portable-continuation).

On Windows 11 x86_64, users may access a supported remote host or use a Linux x86_64 environment through WSL. WSL must independently satisfy the Linux executor prerequisites; installing a distribution alone is not a readiness guarantee. This is Linux execution, not native Windows support. A Windows Liquid client does not imply a local Flow Agent or Meta-Harness process.

## Evidence before a support claim

Each native Executor must prove the exact [accepted security contract](SECURITY.md#accepted-flow-agent-security-target), including direct-write protection inherited by Tool children, controlled configuration changes and truthful cancellation/failure reporting. General network containment, exact mount equivalence, preventive process/thread ceilings and hostile crash cleanup are no longer required release guarantees. ADR-0172 explicitly selects the Mac Seatbelt profile mechanism; no support proof or weaker fallback follows from that approval. [D-063](docs/decisions/open-decisions.html#d-063) owns the remaining protected-inventory choice and [TESTING.md](TESTING.md) owns evidence. Both native replacement implementations must be verified before the first Flow Agent release. An external sandbox needs separate nested-compatibility evidence; it cannot substitute for Flow's own readiness or establish platform support.
