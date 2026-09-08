# Native Flow-file protection proposal

**Mac mechanism and overlap policy selected (ADR-0172); product implementation pending.** The accepted guarantees and exclusions belong to [SECURITY.md](../../SECURITY.md#accepted-flow-agent-security-target). This document specifies the checked Seatbelt design and the remaining additional-home choice in D-063. [TESTING.md](../../TESTING.md#authorized-mac-feasibility-evaluation-adr-0167) owns reproducible commands, native observations and their limits. The existing Linux implementation and fail-closed Mac product behavior remain unchanged.

## Recommendation and user consequences

Implement the selected native macOS profile mechanism as one part of a checked launch boundary, not as a path-only rule. Retain the short-lived Default Executor. Do not require a Linux VM, administrator privileges, a permanent service or an additional product runtime on Mac. `sandbox-exec` is supplied by macOS; Python is only the development probe's driver.

The boundary combines protected directories/program names, complete alias admission, inherited-handle removal, ancestor-move denial and terminal-channel isolation. An invalid installation must produce an actionable start error, never an automatic repair or an unprotected Tool. ADR-0169 accepts the alias and ancestor-move restrictions and reconfirms independent-service responsibility; ADR-0170 accepts the terminal-access compatibility cost under [SECURITY.md](../../SECURITY.md#native-self-protection-and-its-limits). ADR-0172 accepts the deprecated Apple-interface maintenance risk. D-063 retains only the proposed coverage of additional independent Flow homes, not reapproval of the current home's required protection.

Ordinary non-interactive shell commands and project builds remain the intended use case. Tools receive bounded input/output pipes, not the human review terminal. The proposed terminal-device denial can break programs requiring direct terminal access, interactive password prompts or terminal-user-interface libraries. It is not a general promise that every Mac application works. Native GUI automation through independent services and Metal compatibility require their own evidence; neither is established by a C compilation test.

## Native App Sandbox comparison

The practical comparison is now separate from the earlier documentation-only assessment. [TESTING.md](../../TESTING.md#authorized-mac-feasibility-evaluation-adr-0167) owns exact commands, source revisions and hosted ARM64 evidence. No Windows result is used as Mac proof. No production Executor, administrator service or release-signing identity is involved.

### What Apple recommends

The [SDK manual mirror for `sandbox-exec`](https://keith.github.io/xcode-man-pages/sandbox-exec.1.html) explicitly marks the command deprecated and recommends App Sandbox. The consulted sources do not supply a detailed deprecation rationale, a removal date or evidence that the old mechanism is inherently insecure. Do not turn deprecation into those claims. Apple's [current App Sandbox overview](https://developer.apple.com/documentation/security/app-sandbox) documents the supported entitlement-based resource boundary; App Sandbox is required for Mac App Store distribution, not limited to that distribution channel.

The models differ: the profile candidate starts from host access and subtracts protected writes and terminal access. App Sandbox starts with constrained app authority and adds resource grants. The experiment enables `com.apple.security.app-sandbox` on a locally signed native app and inheritance on its bundled helper. Neither Mac configuration uses seccomp, which is a Linux system-call filter, or a Linux VM. A supported app model is not a drop-in supported replacement for every custom command profile.

### Configurations and fair controls

- **Unprotected:** the same native helper performs the dangerous operations successfully. This establishes reachable effects, not protection.
- **Checked profile:** the previously evaluated profile plus alias admission and descriptor removal. The retained raw-profile failures remain in the original probe.
- **Minimal App Sandbox:** signed app and bundled helper, app-sandbox entitlement, helper inheritance, and user-selected read/write entitlement without an actual selection. Its own synthetic app container is the positive control; refusing an unselected project is expected, not a product defect.
- **App Sandbox with project exception:** add Apple's documented temporary absolute-path read/write exception for one synthetic project directory. This is a fixed grant in the locally signed experiment, not a real file-selection or persistent-bookmark workflow.
- **App Sandbox with executable-write permission:** retain that project exception and additionally enable `com.apple.security.files.user-selected.executable`; report its effect separately rather than assuming a failed build workflow cannot be configured correctly.
- **App Sandbox with runtime-read permission:** retain the project and executable-write grants and add read-only access to the already installed Node/npm runtime. This separates runtime-access denial from npm-script compatibility; the runtime is not copied or re-signed.

The [Apple file-access guide](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox) describes container access, recursive folder selection and executable-write permission. Its restriction on execution outside app/container locations is specifically stated for **user-selected-file entitlements**, not proof that all host execution is impossible under every App Sandbox configuration. The [helper guide](https://developer.apple.com/documentation/xcode/embedding-a-helper-tool-in-a-sandboxed-app) supplies bundled-helper inheritance and local ad-hoc signing. The [temporary-exception reference](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/AppSandboxTemporaryExceptionEntitlements.html) documents the separate path grant used here. Temporary exceptions are not established as a suitable release design merely because they work locally.

### Observed security and compatibility

The initial complete five-way comparison contained 133 observations; the npm extension adds three workloads per configuration and a sixth runtime-read configuration. Exact revisions and outcome counts belong to [TESTING.md](../../TESTING.md#authorized-mac-feasibility-evaluation-adr-0167). All unprotected mutation/control operations succeeded. No table entry is a general application-certification claim. Both original project-exception variants produced the same functional outcomes below.

| Case | Checked profile | App Sandbox with project exception |
|---|---|---|
| Write ordinary project file | Allowed | Allowed |
| Write/delete/replace separate Flow file; change its metadata or mapped bytes | Explicit OS denial | Explicit OS denial |
| New-session child and bundled helper write separate Flow file | Explicit OS denial after helper startup | Explicit OS denial after helper startup |
| Existing external helper writes separate Flow file | Starts, then write denied | Starts, then write denied; minimal App Sandbox could not start this helper |
| Symlink pointing to separate Flow file; create new hardlink from it | Denied | Denied |
| Existing hardlink in the writable project pointing to Flow file | Layout rejected before launch | Write succeeds without that admission check |
| Deliberately passed writable protected handle | Removed before launch | Write succeeds if deliberately passed |
| Flow file underneath the writable project directory | Still denied by explicit protected subdirectory rule | Writable through the containing directory grant |
| Move parent of separate protected directory | Denied | Denied |
| Write controller's synthetic terminal device | Denied | Denied |
| `/bin/sh` small noninteractive workload | Completes | Completes |
| `/usr/bin/git --version`; `xcrun clang --version`; build through `xcrun` | Complete | Exit 1: `xcrun: error: cannot be used within an App Sandbox.` |
| Directly resolved Xcode Git version and new synthetic repository | Complete | Complete |
| Direct compiler with explicit SDK and linker location | Builds native program | Builds native program |
| Run that newly built program | Completes | Denied despite the successful build, both without and with executable-write permission |

The hardlink and handle results are **not App Sandbox escape claims**: granting an existing alias or passing already-open authority is a launcher/layout problem. App Sandbox also needs checked launch conditions. Conversely, the checked profile's successful rejection is not an OS-only result. Neither experiment promises whole-host safety or control over independent services.

The nested-directory case is an overlap check, not evidence that ordinary projects contain implicit Flow configuration. [PROTOCOL.md](../../PROTOCOL.md#configuration-and-accepted-post-m11-target-invariants) owns the global-only authority; its [storage contract](../../PROTOCOL.md#local-run-storage-and-m11-conversation-trees) places runtime state in the global home. Workspace `AGENTS.md` is instruction/context input, not technical configuration or an automatically protected Flow store. A grant limited to a project separate from Flow storage avoids this specific overlap. A grant to the whole user home can include `~/.flow` and the Mac platform store, however. The current [home resolver](../../flow-agent/flow-agent-core/src/runtime/session_store.rs) validates an explicit absolute `FLOW_AGENT_HOME` without comparing it with the Workspace; separation is not yet a universal admission check. An App Sandbox design must define overlap rejection or another supported protection method before claiming “all granted files except Flow's own.” The experiment neither proves alternative layouts impossible nor approves a changed storage contract.

`xcrun` is Apple's tool locator/launcher. Its refusal does not mean Git and compilers cannot run: the controller resolved existing Xcode tool locations before sandboxing, and the actual programs then ran without copying or re-signing them. However, arbitrary build scripts may invoke the refused launcher internally. The successful tiny direct build does not establish compatibility with those scripts, Xcode projects or every installed development tool.

The generated program's launch returned `EPERM` in both project-exception variants; its file existed and the compiler exited 0. Adding executable-write permission to the app did not fix this compiler/helper workload. The experiment does not identify the precise quarantine/signature/helper-entitlement cause or prove that every supported execution arrangement is impossible. A container-based build/output arrangement would be a different, untested workflow, not transparent compatibility with arbitrary project commands.

The npm extension tests real offline `prebuild`, `build` and `postbuild` scripts with verified outputs. Original App Sandbox variants refused Node startup; granting read access to the installed runtime allowed Node/npm to start. Explicit project selection then corrected the app's initial container directory. The resulting comparison is:

| `npm run build` workload | Checked profile | App Sandbox with project, executable-write and runtime-read grants |
|---|---|---|
| Generate JavaScript, execute it in a Node child, verify output and complete lifecycle | Completes; synthetic Flow write denied | Completes; synthetic Flow write denied |
| Compile and run native program through `xcrun` | Completes; synthetic Flow write denied | Fails at `xcrun`, exit 1; protected write not reached |
| Compile through direct compiler/SDK/linker paths, then run output | Completes; synthetic Flow write denied | Compilation succeeds; new program launch returns `EPERM`; protected write not reached |

All three unprotected controls completed and performed their deliberate writes. The final six-configuration matrix contains 178 observations; its successful completion does not turn the measured App restrictions or raw-profile counterexamples into passing security tests. A general claim that npm cannot work in App Sandbox would be false.

No React/Vite/Next.js project, dependency installation or Python Tool workload has been tested; Python only drives the experiment outside the sandbox. These need their own interpreter, dependency, cache and service access. Native compiler/helper restrictions matter only when a workflow uses them. Build scripts execute project/dependency code, which remains part of the Engineer's trust responsibility even when npm or Python itself is trusted.

### User experience, terminal interaction and remaining proof

For the profile candidate, ordinary host paths remain the default; installation admission establishes the protected exceptions. For App Sandbox, the user or installer must establish usable resource grants. A genuine folder-selection/bookmark workflow, grant overlap checks and stable signed distribution remain untested. Re-signing a test app with a fixture-specific absolute path is **not** a proposed per-user installation procedure.

Programs that may need a private interactive terminal include `sudo` password prompts, `ssh`/`scp` password or key-passphrase prompts, a `git commit` that opens an editor, Vim/Nano, `less`, `fzf` and interactive coding-agent interfaces. These are compatibility examples, not individually tested failures. A simple stdin question is different from opening a terminal device. Noninteractive modes such as `git commit -m ...` may avoid that editor interaction, but do not certify hooks, authentication or every child process. Never solve password handling by forwarding secrets into Tool input/logs. A private Tool terminal separated from human approval would be a different design requiring approval and native tests, not an automatic fallback.

Both approaches stay native, with no VM-induced Linux/host split. The probe records individual elapsed times, but different cold/warm states, fixed ordering and failed workloads preclude an overhead ranking. No latency, memory, GPU or all-workload performance conclusion follows. Metal/ML inference, GUI automation, real file-picker/bookmark grants, release signatures/notarization, downloaded-package quarantine and other macOS versions remain untested. The profile's prior 29-case publication/parent-exit/later-terminal matrix was not fully repeated for App Sandbox; do not imply evidence parity there.

ADR-0172 selects the checked profile for the broad host-tool use case, accepting its deprecated-interface maintenance risk. App Sandbox remains comparison evidence, not a second selected release backend; reconsidering it would require a separate grant/layout/compatibility decision. Neither the comparison nor mechanism approval establishes release readiness.

## Codex CLI approach and reuse boundary

Source inspection on 2026-09-08 covers published [Codex CLI 0.153.4](https://github.com/openai/codex/releases/tag/rust-v0.153.4), commit `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`, and current main `d6489472f3c15e87d2d7763a5fde033545c530f8`. The [official security guide](https://learn.chatgpt.com/docs/agent-approvals-security) and [permission guide](https://learn.chatgpt.com/docs/permissions) describe OS sandboxing separately from approval policy. Codex CLI uses **Seatbelt through `/usr/bin/sandbox-exec` on macOS**, not seccomp or entitlement-based App Sandbox. Its use does not reverse Apple's deprecation or certify Watershed's contract.

The release's [launcher implementation](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/sandboxing/src/seatbelt.rs) constructs a profile from filesystem and network permissions, binds path parameters and invokes the absolute system executable with `-p`, `-D` and the command argument vector. It supports narrower read-only exceptions within writable roots, protects their ancestors against relocation, and treats policy-preparation failures as errors. This is OS enforcement around programs, not a checker that recognizes safe shell command names.

| Concern | Codex CLI | Proposed Flow-specific profile | Evaluated App Sandbox |
|---|---|---|---|
| Scope | Configurable broader filesystem/network policy plus separate approvals | Narrow mandatory Flow-owned direct-write and review-channel protection; Tool effects otherwise trusted | Restricted app authority plus explicit resource grants |
| Overlapping directories | Read-only exceptions under writable roots and ancestor protections | Same OS mechanism can express protected Flow objects inside a writable parent | Tested containing-directory grant also allowed the nested Flow file |
| Children | Base profile permits process creation; children inherit the sandbox | Must inherit own-file protection, even after parent exit | Bundled/inherited and external helper behavior measured separately |
| Terminals | Base policy supports created private pseudo-terminals using a scoped extension | Direct terminal-device denial accepted for the first release; private Tool terminal not implemented | Tested controller-terminal access denied; no completed Flow review UI |
| Authority expansion | Approval and full-access modes are configurable product policies | Never lift own-file protection; configuration proposals use controller-owned publication | No automatic unsandboxed fallback is approved |
| Integration cost | A larger permission engine with platform and tool compatibility rules | Smaller contract, but still requires anchored admission, handle hygiene, safe review and native tests | App packaging, entitlements, resource grants and host-workflow adaptations |
| Maintenance | Uses the deprecated profile command | Shares that OS-interface risk; OS updates require revalidation | Documented Apple model, but no transparent arbitrary-host-tool compatibility demonstrated |

The release [base profile](https://github.com/openai/codex/blob/3d2ee51ca2d5db578f328aa75e20aa22c0197c9a/codex-rs/sandboxing/src/seatbelt_base_policy.sbpl) includes specific Node/Python system-query allowances and private pseudo-terminal rules. Its terminal access is not an unrestricted grant to other sessions' terminals. These are useful design references, not native proof for Flow's approval channel. Codex protects selected metadata directories such as `.git`, `.agents` and `.codex`; that is not a rule protecting every `AGENTS.md` filename.

**Test distinction:** Watershed's native probes exercise the same OS mechanism with Watershed's own profile and launcher checks. Neither the Codex CLI binary nor Codex's generated profiles/test suite were run on the Mac runner. Source inspection is not execution evidence, and no claim is made that Codex establishes Flow's complete protected-object/alias/publication invariant. No Codex source was incorporated into Watershed.

**Recommendation:** reuse the architectural pattern, not the complete permission product. Retain Flow's one-shot Default Executor, generate only the approved narrow policy, and independently prove Flow-specific invariants. A broader Codex-like sandbox would add a different product contract and maintenance work without removing the shared deprecation risk. Copying upstream code would require a separate dependency/licensing review; it is unnecessary for evaluating the mechanism. Neither native approach has a measured general performance advantage here; both avoid a Linux guest, while their different permissions primarily affect compatibility.

## Overlapping homes and editable instructions

ADR-0172 allows deliberate home/installation placement inside a Tool-writable directory with a clear warning. Identify the Flow objects that remain protected and the parent-folder moves/deletion consequently unavailable to Tools. The nested-path profile test demonstrates this primitive; safe production admission is still required. An outside hardlink or incomplete inventory remains a start error, not a warning-only exception.

The corrected [instruction-file rule](../../SECURITY.md#native-self-protection-and-its-limits) keeps the global `AGENTS.md` protected with the whole Flow home and only Workspace-local instructions editable. Editable local instructions can steer future requests; they cannot expand available Tools or turn Tool output into consent. No per-filename hole inside the global home is required.

## Complete proposed protected set

Resolve the following from trusted controller/installation configuration before Tool launch, never from model output or a Tool-supplied exclusion. Anchor canonical objects using the existing no-follow filesystem discipline; do not later resolve authority again through mutable path text.

| Object class | Proposed coverage and update rule |
|---|---|
| Selected `FLOW_AGENT_HOME` | Entire home: global Flow configuration, registry, runtime history, context/objects, global `AGENTS.md`, locks and staging files, including future publications. Protect directory ancestry; local instructions outside the protected store have no Flow-owned write restriction. |
| Additional Flow homes | Optional expansion still awaiting D-063: an explicit administrator-owned set fixed before execution. The default shared home already covers concurrent Runs using that home; there is no whole-disk search or automatic promise for undisclosed independent homes. If this expansion is selected, every home needing mutual protection must be admitted before starting those Tools. |
| Platform credential/selection store | Entire Flow-owned platform directory, including `credentials.json`, `executor.json`, locks and staging names. These are outside the selected home today; their locations remain canonical in [PROTOCOL.md](../../PROTOCOL.md). The inventory inspects metadata, never credential contents. |
| Installed program objects | Current `flow`, the official sibling `flow-executor` and any explicitly selected Custom Executor, including every covered name of the same file. Reject missing or unresolved objects. Custom code still belongs to the trusted installation, not third-party certification. |
| Path ancestry | Prevent Tools from removing or renaming any canonical ancestor of a protected directory or program. Do not prohibit ordinary unrelated child-file edits in those ancestors. Moving a parent folder containing Flow data is consequently unavailable to Tools. |
| Review channel | Controller owns and pins its terminal before running Tools. No terminal/approval handles are inherited. Deny direct access to the supported Mac terminal-device namespace, including later-created terminals; protect `/dev/tty` and console paths as well as the selected device. This prevents a different Run's new terminal from becoming an unguarded direct entry point. |

Undisclosed installations, project files, backups and exported copies are not automatically protected objects. Changing the admitted home/image set is manual installation maintenance, not one of the automated configuration fields. Do not claim that an already-running Tool acquires protection for a newly admitted outside directory: its native policy is immutable. Maintenance must account for previously started helpers; bounded waiting is not proof that all of them ended.

## General invariant, not command-name filtering

At admission, every regular protected file's total hardlink count must equal the number of its distinct names covered by the protected set. Count an overlapping root only once. Reject external aliases, unsupported file types, incomplete scans, changed identities and inability to establish the complete set. Ordinary project hardlinks remain allowed.

This admits a legitimate temporary hardlink **inside** a protected directory without permitting an alias outside it. It does not relax the runtime's existing storage-format, ownership, durability or incomplete-publication checks. A protected leftover stage is not automatically a valid completed authoring operation.

The invariant is preserved during execution because:

1. All admitted names are covered by native write denial; children inherit that denial across ordinary spawn, session changes and parent exit.
2. Tools cannot create new hardlinks from protected objects, change their metadata, replace them or move their protected ancestry outside the policy.
3. Flow-owned publishers create, append, link and rename only within the already protected directory set. Replacement and newly created files remain covered by directory policy; no per-file policy refresh is needed.
4. No writable protected handle, directory capability or review handle reaches the Tool. Only explicitly constructed Tool I/O is inherited. A pre-opened writable handle defeats the path-only profile and must be removed before execution.

Existing Flow processes must coordinate initial admission with their protected-file publication operations using an installation-owned lease and anchored identities. A scan racing a publisher is retried within a bounded admission attempt or rejected; it is not a valid snapshot. Do not serialize entire Runs or ordinary project work behind that admission lease. Do not rescan all retained history for every Tool call: retain one verified controller admission and preserve it through checked publisher operations. Measure the cost of the actual scanner and contention during integration; the small fixture scan supplies no large-history performance claim.

Unconfined external writers, privileged actors and controller/OS compromise are the existing trust exclusions, not problems solved by the lease. Cooperative locks alone never constrain a malicious Tool. The native restriction is what preserves the established set against Tool code.

```mermaid
flowchart TD
  Select["Controller selects homes, platform store and installed images"] --> Admit["Anchor objects and verify all covered aliases under publication lease"]
  Admit -->|Incomplete, changed or external alias| Refuse["Explain start error; no Tool; no automatic repair"]
  Admit -->|Complete| Prepare["Bind paths as data; remove inherited authority; prepare native restriction"]
  Prepare -->|Not supported or not verified| Refuse
  Prepare -->|Ready and durable launch authorized| Run["Start Tool and inherited helpers"]
  Flow["Trusted Flow publisher"] --> Publish["Append or atomically publish inside existing protected directories"]
  Publish --> Covered["Old and new names remain protected"]
  Run -->|Direct write, alias or move attempt| Block["OS denies operation"]
  Run -->|Ordinary project operation| Host["Tool's trusted-code responsibility"]
```

## Launch, publication and failure protocol

- **Prepare:** initialize required Flow-owned directories through trusted setup, acquire admission, verify identities/alias coverage and build an immutable protected set. Missing prerequisites or a path that cannot be represented safely are explicit readiness failures. The candidate uses fixed profile structure and parameter-bound path data; shell interpolation and embedding untrusted path text as profile source are unnecessary.
- **Start:** the existing controller/one-shot Executor seam retains durable intent, `Ready`/`Start` ordering, bounded transport and exact request binding. Install the native restriction and remove all unintended descriptors before the first Tool instruction. Launch trusted enforcement code with a separate sanitized environment; apply Engineer-admitted Tool environment values only inside the established boundary, never to the enforcement launcher's loader or startup hooks. The replacement protocol must identify the exact applied protected set without pretending the old broad-isolation receipt still describes it.
- **Publish:** preserve existing create-only hardlink publication, append, staged-file replacement and staged-directory rename. Both temporary and final names stay protected. A failed cleanup can leave a protected stage and an honestly incomplete outcome; it cannot silently move that stage to a writable project directory.
- **Install:** finalize program publication before a new installation starts Tools. Current installation does not upgrade existing programs. An external hardlink to a selected program is a readiness error; an interrupted install must be finalized or deliberately repaired by its owner. This proposal does not add an automatic updater or change system/kernel policy.
- **Finish/crash:** terminate further dispatch and retain truthful effects/uncertainty evidence. The restriction stays on a surviving helper; protection must not depend on the Executor remaining alive. Do not promise that arbitrary hostile descendants have all been killed. Never redispatch an uncertain effect.

## Configuration review integration

The [approved ADR-0168 workflow](../../SECURITY.md#configuration-and-migration-boundary) is normative, not another proposal here. Its implementation must use separate controller-owned input, discard pre-review buffered input, safely render untrusted output and show an unmistakable controller review of the exact change. Render control and bidirectional formatting characters visibly rather than executing or visually hiding them. A Tool's stdout, a literal `yes`, an escape sequence or a forged attention event must never count as consent. A native terminal-open denial is necessary evidence, not a completed approval UI or transaction test.

```mermaid
sequenceDiagram
  participant T as Tool or surviving helper
  participant G as Native restriction
  participant F as Flow controller
  participant U as Local owner
  T-->>F: Tool output and explicit proposal
  Note over F: Output is untrusted data, not consent
  F->>F: Finish Tool; validate proposal; stop further dispatch
  T->>G: Open current or later review terminal
  G-->>T: Direct access denied; no inherited review handle
  F->>U: Controller-owned exact-change review
  alt Explicit valid consent within five minutes
    U-->>F: Approve this proposal
    F->>F: Revalidate version and scope; atomic publication for future Runs
  else Refusal, silence, cancellation or lost channel
    F->>F: Reject and end requesting Run; no retained approval queue
  end
```

An Engineer-authorized independent GUI/automation service may act with its own authority, including on a terminal. That remains delegated host authority, not direct Tool-channel isolation. Do not advertise secure human approval against a compromised controller, terminal application or such a service. If a deployment needs that stronger guarantee, this proposal is insufficient; a separately trusted approval surface would require another decision.

## Finite evidence and release acceptance

The executable matrix partitions file access, alias admission, publication, inherited authority, helper lifetime, review-device access and ordinary host-program compatibility. It deliberately includes unprotected controls. It is not an exhaustive list of commands or a mathematical proof that Apple's private implementation has no defects.

| Evidence boundary | What constitutes completion |
|---|---|
| Native feasibility | Existing direct-write/link/handle controls plus multi-root publication, metadata/mapped writes, ordinary project writes/hardlinks, child inheritance, parent exit and current/later terminal tests pass on native ARM64 macOS. The raw path-only profile remains a recorded counterexample. |
| Product admission | Implement anchored discovery and coordinated publication, then run real runtime/installer concurrency and interruption tests. The fixture scanner is deliberately single-owner; its passing tests do **not** prove race-safe production admission. |
| Configuration transaction | Implement ADR-0168 and test actual CLI review, pre-buffered/forged output, wrong/stale/reused consent, exact scope, rejection/timeout/channel loss, simultaneous publishers and each crash boundary. No mock interaction may be reported as native end-to-end consent evidence. |
| Distribution | Build the actual download package; verify checksum failure paths, clean installation, quarantine/Gatekeeper behavior and the chosen release signing/notarization process on macOS. The local generated C binary's signature proves none of these. No release identity or credential is acquired by the probe. |
| Release support | Resolve the additional-home inventory choice, publish the tested OS range, retain Linux x86_64 and macOS ARM64 native gates and full lifecycle observations. The Mac mechanism and mandatory restrictions are selected, not verified product behavior. Unsupported or failed readiness has no weaker fallback. OS updates require revalidation; a deprecated interface can force a compatibility update or explicit refusal. |

The implementation and distribution rows are **release gates**, not tests completed by this design. Do not select a release OS range solely from one hosted runner patch version. The [native comparison](#native-app-sandbox-comparison) establishes specific App Sandbox successes and restrictions, not arbitrary host-program compatibility; [D-063](../decisions/open-decisions.html#d-063) retains the additional-home choice. A passing feasibility matrix and an approved mechanism are not the claim that it is the only possible design or already release-ready.
