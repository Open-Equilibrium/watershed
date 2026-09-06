# Platforms and capabilities

Support is a product-and-capability contract, not a workspace-wide compilation claim. Release 1 requires native execution and verification on the targets below; a target in this table is not evidence that its implementation is available today.

| Product / capability | Release 1 native targets | Current implementation |
|---|---|---|
| Flow Agent authoring, Fixture execution and provider-only Flows | Linux x86_64; macOS ARM64 | Implemented; productive provider execution is restricted to Ubuntu 24.04 and macOS 26. |
| Flow Agent Default Executor and Tool Sandbox | Linux x86_64; macOS ARM64 | Ubuntu 24.04 only. macOS Tool execution fails closed; its native backend and proof remain required before Release 1. |
| Meta-Harness CLI, service and host-local agent control | Linux x86_64; macOS ARM64 | Not implemented. |
| Liquid desktop client | Linux x86_64; macOS ARM64; Windows 11 x86_64 | Not implemented. |

Linux ARM64 and Windows 11 ARM64 are deferred beyond Release 1. Earlier Windows versions are unsupported. Flow Agent and Meta-Harness have no native Windows support or future Windows-backend commitment. Liquid's mobile and headless capabilities retain their separate contracts in its [V-Spec](docs/concept/V-Spec_Liquid.html); desktop support does not certify them.

Ubuntu 24.04 x86_64 and macOS 26 ARM64 are the current concrete Flow verification targets. Other Linux distributions or OS versions do not acquire a support claim from sharing an architecture or passing compilation. Liquid and Meta-Harness must establish their own runtime evidence, not inherit Flow's verification results.

## Execution and development boundaries

Watershed development targets native Linux x86_64 and macOS ARM64. A Mac can run native development and macOS verification while Linux CI supplies Linux evidence; no local Linux computer is required. Cross-compilation, containers sharing a host kernel, and platform-independent tests do not replace native executor tests on both release targets.

A Linux VM is not the macOS product execution path. Native Apple tools and Metal-based inference must remain possible without placing Flow Agent in a Linux guest; local inference still requires the separately decided trust boundary in [SECURITY.md](SECURITY.md#accepted-post-m11-target-local-inference-and-portable-continuation).

On Windows 11 x86_64, users may access a supported remote host or use a Linux x86_64 environment through WSL. WSL must independently satisfy the Linux executor prerequisites; installing a distribution alone is not a readiness guarantee. This is Linux execution, not native Windows support. A Windows Liquid client does not imply a local Flow Agent or Meta-Harness process.

## Evidence before a support claim

Each native Executor must prove its declared filesystem, network, process/thread-capacity and descendant-cleanup guarantees, including cancellation and Executor failure, through the real boundary. Unsupported capabilities fail before Tool launch; no weaker backend, private-API workaround or parity claim is inferred from the platform target. [SECURITY.md](SECURITY.md) owns the invariants; [TESTING.md](TESTING.md) owns verification. Release 1 remains blocked until both required native Executors are implemented and verified.
