# Platforms and capabilities

Support is a product-and-capability contract, not a workspace-wide compilation claim. Release 1 requires native execution and verification on the targets below; a target in this table is not evidence that its implementation is available today.

Flow Agent's accepted replacement boundary is [trusted Tools with mandatory native self-protection](SECURITY.md#accepted-flow-agent-security-target) (ADR-0166), not general hostile-Tool containment. ADR-0174 limits first-release protection to this installation's required objects, not additional independent Flow homes. The replacement is integrated; [TESTING.md](TESTING.md#native-verification-status) owns native results and remaining acceptance gaps. Implementation alone is not support proof.

Linux and macOS share the same [public installation guarantee and maintenance limits](SECURITY.md#native-self-protection-and-its-limits) (ADR-0176). Their different [launch mechanics](PROTOCOL.md#process-and-framing-contract) do not establish stronger Linux protection against unconfined host-authority writers.

| Product / capability | Release 1 native targets | Current implementation |
|---|---|---|
| Flow Agent authoring, Fixture execution and provider-only Flows | Linux x86_64; macOS ARM64 | Implemented; productive provider execution is restricted to Ubuntu 24.04 and macOS 27. |
| Flow Agent Tool execution with mandatory self-protection | Linux x86_64; macOS ARM64 | Linux Bubblewrap/seccomp and Mac Seatbelt replacements integrated; native evidence is linked above. Failed admission/readiness prevents launch without fallback. |
| Meta-Harness CLI, service and host-local agent control | Linux x86_64; macOS ARM64; Windows 11 x86_64 in WSL (ADR-0226) | Not implemented. |
| Liquid desktop client | Linux x86_64; macOS ARM64; Windows 11 x86_64 and ARM64 | In development on macOS ARM64: the agent surface for one pi session, not released. Linux and Windows are not implemented. |
| Liquid mobile client | Android ARM64 | Not implemented. |

Linux ARM64 follows after Release 1, and with it Meta-Harness on Windows 11 ARM64 through WSL; the Liquid client runs natively on Windows 11 ARM64 from Release 1. Earlier Windows versions and macOS x86_64 (Intel Macs) are unsupported by every product (ADR-0218). Flow Agent has no native Windows support or future Windows-backend commitment; Meta-Harness reaches Windows 11 only as its Linux build in WSL (ADR-0226). Liquid ships native apps only, with no web client (ADR-0228). Liquid's mobile and headless capabilities retain their separate contracts in its [V-Spec](docs/concept/V-Spec_Liquid.html); desktop support does not certify them.

Ubuntu 24.04 x86_64 and macOS 27 ARM64 are the current concrete Flow verification targets. Other Linux distributions or OS versions do not acquire a support claim from sharing an architecture or passing compilation. Liquid and Meta-Harness must establish their own runtime evidence, not inherit Flow's verification results.

Productive execution, the Default Executor and installation use the same release admission: Ubuntu metadata must contain exactly one `ID=ubuntu` and one `VERSION_ID=24.04`, with unquoted, double-quoted or single-quoted values; macOS must report a dotted numeric version beginning with `27`. Missing, contradictory or malformed release identification is rejected without changing the host. Rust consumers share [the release predicate](flow-agent/release.rs); the standalone installer applies the same finite grammar without executing metadata as shell code.

## Execution and development boundaries

Watershed development targets native Linux x86_64 and macOS ARM64. A Mac can run native development and macOS verification while Linux CI supplies Linux evidence; no local Linux computer is required. Cross-compilation, containers sharing a host kernel, and platform-independent tests do not replace native executor tests on both release targets.

A Linux VM is not the macOS product execution path. Native Apple tools and Metal-based inference must remain possible without placing Flow Agent in a Linux guest; local inference still requires the separately decided trust boundary in [SECURITY.md](SECURITY.md#accepted-post-m11-target-local-inference-and-portable-continuation).

On Windows 11 x86_64, users may access a supported remote host or use a Linux x86_64 environment through WSL. WSL must independently satisfy the Linux executor prerequisites; installing a distribution alone is not a readiness guarantee. This is Linux execution, not native Windows support. A Windows Liquid client does not imply a local Flow Agent process. It embeds no Meta-Harness core; as T3 Code does with its server, it installs, updates and starts the Linux Meta-Harness in a WSL distribution, connects to it over a loopback port and also reaches supported remote hosts. Projects belong in the WSL file system, because Git on a Windows drive is much slower (ADR-0201).

Windows is not a target runtime for agents or local inference, so Meta-Harness has no native Windows version (ADR-0226). The tradeoffs are accepted: agents work with Linux tools, so Windows-native projects such as .NET with Visual Studio, PowerShell scripts or Windows apps are reachable only in part; installing WSL needs administrator rights, and its virtual machine reserves memory; and companies that block WSL run agents on remote hosts, with their own inference server if they want one. A native version is reconsidered only through the decision flow, on shown need.

## Installation prerequisites

On macOS, installation requires Apple's `/usr/bin/osascript -l JavaScript` and its built-in JavaScript/Objective-C bridge for bounded [ACL](GLOSSARY.md#flow-agent-runtime-surfaces) checks on held objects (ADR-0178), including with `--no-default-executor`. Unsafe mutation grants, unavailable queries or missing bridge components stop installation; harmless access and owner/root/current-user grants remain admissible. This requires no compiler, additional runtime or application automation and does not change existing administrator ACLs or host policy.

## Native Executor prerequisites

| Verification host | Required host facilities |
|---|---|
| Ubuntu 24.04 x86_64 | `/usr/bin/bwrap`, a regular root-owned executable without group/other write permission; host policy permitting the backend's user/PID namespaces and seccomp. |
| macOS 27 ARM64 | Apple's `/usr/bin/sandbox-exec`, a regular root-owned executable without group/other write permission, and the selected Seatbelt mechanism. Its deprecated-interface/OS-update risk remains accepted (ADR-0172). |

Host administrators must supply missing prerequisites explicitly. Installation does not update the kernel, change host security policy or start privileged services; no systemd/cgroup service is required. Tool-specific runtimes, libraries, helpers and services remain the Agentic Engineer's responsibility. Follow the [installation steps](README.md#download-installation-on-linux-or-macos), then run `flow executor check` as the intended unprivileged user. Readiness is advisory, not native acceptance or a substitute for per-attempt admission; [PROTOCOL.md](PROTOCOL.md#native-backend-contract) defines backend enforcement.

## Evidence before a support claim

Each native Executor must prove the exact [accepted security contract](SECURITY.md#accepted-flow-agent-security-target), including direct-write protection inherited by Tool children and truthful cancellation/failure reporting. Configuration administration stays manual for the first Flow Agent release. General network containment, exact mount equivalence, preventive process/thread ceilings and hostile crash cleanup are no longer required release guarantees. Ordinary networking is available on both; neither requires runtime-read profiles. Integration supplies no native support proof. [TESTING.md](TESTING.md#m12-transition-and-executor-evidence) owns the historical native baseline, red raw replacement run and outstanding evidence; Windows shared-test success is not native Flow proof. Both native replacement implementations must be verified before the first Flow Agent release. An external sandbox needs separate nested-compatibility evidence; it cannot substitute for Flow's own readiness or establish platform support.
