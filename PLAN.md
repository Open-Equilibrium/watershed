# Plan

Implementation milestones with deliverables and a Definition of Done (DoD). Performance architecture and evidence are canonical in `PERFORMANCE.md`.

Created: 2026-06-05

## MVP boundary

The first MVP is **Flow Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3), and from the Git integration for agent projects, which Meta-Harness runs and Liquid presents (ADR-0190), including its per-turn checkpoints (ADR-0220).

## Sequencing and adoption

Build the shared substrate, then Flow Agent, Meta-Harness and Liquid. This dependency/order-of-risk sequence validates execution, then multi-agent orchestration, then the integrated workspace without letting the broadest surface block earlier validation. All three remain independently usable layers of one platform; integration is canonical in `VISION.md`.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Flow Agent and Meta-Harness wedges are validated. Long-term architecture must remain valid whether inference is local or remote: the provisioned core path is offline-capable, each Flow's model/runtime requirements resolve through a device-local Runtime binding, and cross-device continuation creates Conversation branches instead of transferring live host ownership. Canonical integration detail is in `VISION.md`.

## Milestones

### M0 — Flow Agent MVP implementation packet + walking skeleton

**Wedge:** Flow Agent execution wedge (developer/open-source credibility).

**Purpose:** establish the implementation packet and walking skeleton needed to build the standalone Flow Agent MVP without inventing architecture.

**Deliverables:**

- Rust workspace and the `core`, `proto`, `flow-agent`, `meta-harness`, and `liquid` scaffold.
- Versioned building-block, event, runtime, session, policy, and sandbox contracts. Canonical owners: `PROTOCOL.md`, `SECURITY.md`, and the Flow Agent V-Spec.
- Deterministic fixtures and expected streams per `TESTING.md`.
- Cross-platform CI, dependency, coverage, link, and render gates per `TESTING.md`, `SECURITY.md`, and `.github/workflows/ci.yml`.

**Decision state:** M0 and M1 are unblocked; accepted decisions are in [`ADR-LOG.md`](docs/adr/ADR-LOG.md).

**DoD:** the scaffold compiles for each implemented product's targets in [PLATFORMS.md](PLATFORMS.md); its canonical contracts and fixtures are sufficient to implement M1 without architectural guesses; and all M0 gates defined by the canonical test, security, and CI sources pass.

### M1 — Flow Agent deterministic runtime foundation

**Status:** Complete.

**Purpose:** establish the deterministic, auditable runtime contracts that later practical execution and OS isolation extend without presenting fixture behavior as productive provider or process execution.

**Deliverables:**

- Strict Flow registry and policy contracts with canonical serialization and bounded recursive resolution.
- Pure deterministic Flow planning, FSM orchestration and exactly-once apply over the fixture executor.
- Fixture/stub execution only when the workspace explicitly selects the canonical fixture profile; other workspaces fail closed before provider or tool side effects.
- Canonical Flow runtime events, append-only session history, replay, tail and resume.
- Deterministic, cache-stable `flow-context-v0` compilation and reproducible context manifests.
- Deterministic in-process policy enforcement/emulation for declared command, parameter, exact-mount and deny-all network decisions.
- Cross-platform functional and coverage gates plus Linux performance evidence defined by `TESTING.md`, `PERFORMANCE.md` and CI.

M1 does not provide a real provider adapter, general external process execution, a complete POSIX shell, OS-enforced isolation, positive network grants, or public session export/delete/prune operations.

**DoD:** a fixture-profile workspace runs multi-phase Flows and Subflows headlessly; planning is side-effect-free; each planned fixture side effect is applied at most once; non-fixture execution fails closed; canonical events and context manifests are persisted and replayed/tailed/resumed without repeating completed side effects; one host-local lease authorizes each active workspace/session pair, direct workspace-marker mutation cannot grant or revoke it, process exit releases it, and controlled returns preserve all operation, writer-finalization, marker-validation and lease-release failures plus valid artifacts; policy-emulation and sandbox-negative tests remain explicit about the absence of an OS boundary; and every M1 test and coverage gate passes with complete performance evidence. Flow Agent remains standalone and has no Watershed-owned project-code VCS behavior.

### M1.1 — Flow Agent practical execution

**Status:** Complete.

#### M1.1 entry criteria — architecture hardening

The M1 deterministic runtime foundation satisfies the following criteria. They remain mandatory boundaries before real provider integration or general subprocess execution begins:

1. **Protocol validation ownership:** `proto` alone owns Event-envelope and Event-payload structure validation; `flow-agent-core` delegates it. Runtime retains only stream, sequence, budget and lifecycle invariants, without parallel payload schemas or duplicated `EventType` matches.
2. **Real `core-script` modules:** production uses real Rust modules with explicit visibility/imports, no `include!` assembly and a minimal public API.
3. **Explicit Flow Agent runtime boundaries:** internal structure does not depend primarily on flat `pub use module::*` namespaces or systematic `use super::*`; imports, re-exports and security-critical interfaces are targeted.
4. **Responsibility-based module split:** separate planning/apply; Event construction/persistence; Protocol lifecycle validation; Run reservation/locks; Run bundle inventory; Run reader/replay/tail; Resume orchestration; Context compilation; fixture Tool execution; capability-relative filesystem operations; CLI parsing/dispatch; CLI live streaming; and CLI tail. Do not split by line count alone.
5. **Explicit finite execution-plan IR:** `FlowExecutionPlan` is a typed finite IR. Planning has no provider/Tool adapter and cannot constructively perform side effects; invocation IDs are planned; Apply consumes only the plan; Resume distinguishes pending from completed intents; a second full FSM pass is not a substitute for the IR.
6. **Test architecture:** split large tests along the same responsibilities, including Protocol payload versus lifecycle and Run reservation versus bundle, corruption and Resume. Tests remain behavior-focused and the coverage threshold is unchanged.
7. **CLI architecture:** `main.rs` is only the composition root; argument/usage parsing, dispatch, streaming and tail are separate internal modules, without requiring a product-framework decision.
8. **Completion state:** the criteria are complete in M1; provider/subprocess work must preserve them and remains separately scoped to M1.1.

**Purpose:** turn the deterministic M1 foundation into a useful local Flow Agent without claiming the OS isolation reserved for M1.2.

**Deliverables:**

1. A provider abstraction with at least one real provider adapter.
2. Explicit non-interactive `flow init`, `flow validate` and custom `flow create <kind>` authoring for the four registry kinds: Tool, Instruction, Phase and Flow. Creation has complete preflight, no overwrite and no implicit attachment or discovery.
3. Typed selected-root-Flow input and closed typed runtime values.
4. Parameterized Instructions whose declared `{{name}}` placeholders bind typed Phase input values; explicit output contracts for every Phase.
5. Recursive Phases: a leaf runs the provider loop, while a composite runs only its ordered child Phases and selects its declared result child.
6. Whole successful result handoff between selected sibling Phases by default, with ordered inline forward Transitions using exact typed equality.
7. Declarative Phase loops bounded to 1–32 local iterations and 512 Phase iterations per top-level Flow.
8. Phase-scoped Tool availability: the provider may request an available Tool zero or more times; a Tool reference never invokes it automatically.
9. Provider-requested invocation parameters validated against each Tool's `allowed_parameters`, with canonical bounded Tool results for events and Run Logs.
10. A general bounded external subprocess runner.
11. Predefined command identity and arguments resolved without shell parsing, `PATH` lookup or ambient environment inheritance; productive launch uses the Executor boundary.
12. Own-script execution through one fixed runner with a bounded runtime and no implicit interpreter selection.
13. Timeouts, cancellation and bounded stdout and stderr.
14. Per-Tool Run Log projections plus actual `tool.timed_out` and Tool-failure events.
15. Durable intent around every explicit provider or provider-requested Tool attempt; an uncertain attempt is never relaunched automatically, and `flow reconcile-tool <conversation-id> <run-session-id> --result <file|->` settles exactly one eligible Tool attempt from bounded external evidence.
16. Versioned Conversation trees over linear Runs, including creation, continuation, branching, recovery and paged status, as defined in `PROTOCOL.md`.
17. Explicit `openai-codex` global configuration, browser/device authentication and a protected Flow-owned credential record.
18. Ordered global-home and harness-start Workspace `AGENTS.md` loading as a separate instruction/context channel. Manually referenced Instruction source filenames have no authority or naming semantics and may, for example, be named `SYSTEM.md`.
19. No Watershed-owned project-code VCS behavior.

**DoD:**

- A non-fixture workspace runs a real Flow through a real provider adapter.
- Init, Validate and Create for Tool, Instruction, Phase and Flow satisfy the exact long-flag and delimited-group grammar in `PROTOCOL.md` plus the identity-bound transaction contract; valid output round-trips through registry validation, while failure preserves existing bytes and publishes no partial output.
- Recursive composite/leaf execution, typed Instruction binding, output contracts, whole-result handoff, ordered forward Transitions and both loop bounds are enforced before or during deterministic orchestration.
- At least one predefined command and one own-script Tool use the bounded runner.
- Root input, Phase results, Instruction parameters and provider-requested Tool parameters are typed and validated. Tool declarations make capabilities available but cause no automatic execution.
- Every explicit provider or provider-requested Tool attempt records durable intent; an intent without a committed terminal result is reported as uncertain. Tool reconciliation accepts one canonical bounded result, derives exactly one eligible attempt and never redispatches it.
- Timeout, cancellation, output caps and Tool logs are tested.
- Every Conversation branch remains navigable in one append-only history. Latest-entry continuation and explicit older-entry branching create new Runs from the selected terminal compact snapshot after registry-drift validation, never roll back filesystem or external effects, and retain descendants. Exact two-id recovery consumes the same bounded snapshot model without redispatching durable external results.
- No capability depends on Meta-Harness or Liquid.
- Current gates and security boundaries pass; the fixed M1.1 performance evidence is complete and retained.

**Explicit post-M1.1 Flow Agent work:** later milestones may add typed projections or mappings between Flow, Phase, Instruction, subflow, Tool and addressable artifact inputs/outputs; backward edges or an expression language; general retry/fallback policies; and automatic Tool imports referenced by Instructions. Dynamic proposals to add a Phase, child Phase or Flow at run time—including one-run versus persisted approval and an operator opt-out—also remain deferred. A durable ordered Conversation-status inventory may replace the query-only CV-03 admission bound when larger inventories are required; it must define transaction, recovery and consistency rules without replacing Conversation authority. Any future routing or proposal surface requires a finite schema, permissions, complete allowed/rejected matrix, provenance, replay and approval decision before enablement. The M1.1 routing boundary is canonical in `PROTOCOL.md`.

#### Accepted post-M1.1 architecture corrections

These architecture corrections are accepted; implementation status is recorded in the canonical contracts:

- Use `FLOW_AGENT_HOME` (default `~/.flow`) as the sole Global Flow configuration authority. Project-specific behavior is an explicitly authored/selected Flow; no Workspace-local technical config is discovered or merged. Global and harness-start Workspace `AGENTS.md` remain a separate instruction/context channel without configuration authority. This correction is implemented in M1.1.
- Add a provider-neutral local-inference path so a provisioned host can run the complete local core path with network interfaces disabled. Portable requirements and device-local Runtime bindings remain separate; provisioning, model/runtime evidence and resource admission remain [D-059](docs/decisions/open-decisions.html#d-059), with the local inference process trust boundary in [D-061](docs/decisions/open-decisions.html#d-061).
- Add a public, versioned Portable continuation contract. A verified terminal checkpoint imported on another device creates a child Run, rebinds destination authority and capabilities, and never replays completed effects. Disconnected continuations branch; exact in-flight recovery remains distinct. Archive, transfer, fencing and tree-navigation details remain [D-058](docs/decisions/open-decisions.html#d-058), archive authenticity in [D-062](docs/decisions/open-decisions.html#d-062), and offline approvals/revocation in [D-060](docs/decisions/open-decisions.html#d-060).

No implementation or release claim follows from a documentation decision alone. Each remaining target requires its red behavior tests, finite security and compatibility contract, implementation and ordinary repository gates in a separately scoped change.

### M1.2 — Flow Agent OS isolation

**Status:** The native self-protection replacement and version/platform download packaging are implemented; [TESTING.md](TESTING.md#native-verification-status) owns native and downloaded-bundle acceptance results and their limits, while [PERFORMANCE.md](PERFORMANCE.md#flow-agent) links native measurements. [SECURITY.md](SECURITY.md#accepted-flow-agent-security-target) owns the approved security and installation decisions. Configuration administration remains manual for the first Flow Agent release; deferred work and discussion context belong to the [later roadmap](#later-flow-agent-roadmap). The final native gate and [repository review and closeout](AGENTS.md#session-workflow) remain incomplete. This entry updates status, not the separate three-workstream development plan.

**Current implementation:** One shared one-shot lifecycle uses Linux Bubblewrap/seccomp or macOS Seatbelt to protect this installation's Flow-owned objects. Tool commands and typed parameters, Ready/Start, durable intent, bounded results and self-protection receipts replace broad mount/profile/network/process-capacity policy. Schemas reject removed requirements rather than ignoring them. The wire contract is canonical in [PROTOCOL.md](PROTOCOL.md#m12-executor-protocol); [TESTING.md](TESTING.md#m12-transition-and-executor-evidence) owns executable evidence.

**Contract:** The [security contract](SECURITY.md#accepted-flow-agent-security-target) and [execution/security architecture](docs/concept/flow-agent-executor-architecture.md) define guarantees and Engineer responsibilities. The unreleased migration adds no compatibility aliases.

**DoD:** Verify the accepted boundary on both native Flow Agent targets in [PLATFORMS.md](PLATFORMS.md), prove the prebuilt installation contract, and complete the canonical test, performance and review gates. Cross-compilation, historical isolation tests and mechanism experiments do not prove the replacement. Meta-Harness and Liquid never acquire ownership of Flow's Executor or Tool protection.

### M2 — Meta-Harness MVP

**Wedge:** Meta-Harness orchestration wedge — run Flow Agent and external agents durably, observably and together across harnesses, model providers and devices, leaving governance to each harness (ADR-0252). Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained, host-scoped headless control plane** with CLI/API/service surfaces — usable without Liquid. Each instance controls only CLI agents on its own host. Its public API is one command set over a private local connection on the host and an authenticated network connection remotely (ADR-0185). Liquid integrates later as a client of one or more instances. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI (headless user/admin: run/session/config/metrics commands).
- Local service in its own process, serving that API over both connections; who starts it is [D-074](docs/decisions/open-decisions.html#d-074). Liquid then connects to it like any client and drops its embedded core (ADR-0215).
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, agent schedule/trigger control, answers to a harness's own approvals and questions, revert.
- Central configuration model that resolves shared Watershed building blocks to each target's documented config surface. Flow Agent receives only its global authority and explicit Flows rather than a project-local config layer; device-local Runtime bindings remain owned by the execution host.
- Control plane: session registry, routing, task state, attention state and agent schedule/trigger skeleton.
- Reference scenario for schedules: a scheduled agent keeps a news feed as a database page in Liquid and changes it only with the approval its Workspace Role requires, as `disler/pi-durable-news-agent` does with its own app.
- Host-local Meta-Harness agent executor that rejects cross-host agent-process control (backend integration: [D-048](docs/decisions/open-decisions.html#d-048)).
- Adapters: Flow Agent (via its public runtime surfaces) + at least one external CLI adapter.
- Orchestration tools for agents as MCP tools (ADR-0253, [V-Spec](docs/concept/V-Spec_MetaHarness.html#orchestration)): an agent starts, messages, reads, steers and stops subagents of any harness and model, on other agent hosts too, and hosts link by a pairing the person completes; Claude Code joins pi and Codex as an adapter.
- Event/transcript ingestion from agents; artifact/log/handoff indexing (logs, structured summaries, host-provided diffs, handoff packs, checkpoints).
- Policy-gated configuration writes with audit trail and review flow.
- **No rich standalone GUI**; M2 packaging, including whether it adds a small status screen after the headless controller works, remains D-021.

**DoD:** monitor, steer and configure at least two different CLI agents on one agent host from one control surface, with both represented through one normalized session/event model; reject attempts to claim or control a process on another host; run without Liquid; expose the public API/protocol over both connections; integrate Flow Agent through its public runtime surfaces (not its internals); resolve shared config without duplicated per-agent config directories for the same capability; require approval plus an audit record for every sensitive config change; and let a session on one host start, message, steer and stop a subagent of another harness on another agent host through the orchestration tools.

### M3 — Liquid (staged standalone workspace product)

**Wedge:** prove safe human/agent co-editing of local-first Workspace state with attributed, reviewable and reversible History. Liquid remains useful without agents, network or a hosted service. Do not build a generic Notion clone.

The target product is canonical in [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html). M3 is staged so workspace fundamentals prove value before sync, community extensions and broad integrations. Chat between people and a calendar follow after Release 1 (ADR-0236).

#### Liquid agent surface — early slice

A macOS Liquid app, built ahead of M2 and M3a, that controls pi through an embedded Meta-Harness core and makes agent runs inspectable (ADR-0184).

- Embedded Meta-Harness core in Rust with a pi adapter (`pi --mode rpc`) behind an agent-neutral in-process session/event model, mapped to the PROTOCOL.md v0 families with service mode (ADR-0195); service mode, remote access and Android follow with multi-device.
- pi session files are the only session store; Liquid-specific records are custom pi entries (ADR-0189).
- Agent view: send a prompt, follow the response live, steer and abort; pi dialogs, status and widgets.
- Trajectory tab, live and retrospective: steps, tokens, timing and what the model saw; branch from a step and compare both branches side by side.
- Diff view of a run's changes in the agent's Git project, unified or split and with whitespace changes hidden on request (ADR-0190).
- Shared [design guidelines](docs/design/guidelines.md) and tokens, plus approved mockups of the agent view, Trajectory tab and Diff tab before their implementation.
- CI: `flutter analyze` and `flutter test` on Linux as required checks; a macOS app build on a macOS runner becomes required once it runs reliably.
- Out of scope: Trajectory search and attribution of entries to the extension that produced them. The agent-surface mockup's context-window display follows in a later step.

**DoD (macOS):**

1. The app starts a pi session through the embedded core; a user can send a prompt, follow the response live, steer and abort.
2. pi dialogs (`select`, `confirm`, `input`, `editor`) appear and their answers reach pi; a cancelled dialog never leaves pi waiting; status and widgets render.
3. The Trajectory tab shows each step with tokens, timing and what the model saw, live and after the run, and looks the same after an app restart, rebuilt from the pi session file alone.
4. Branching from a step creates a second branch that can be compared side by side, while the original stays unchanged.
5. The Diff tab shows a run's changes against the project state before the run, as Git sees them under the project's settings (ADR-0210).
6. The UI never sees pi's protocol: CI tests drive a scripted fake agent behind the same session/event model, without model calls or credentials; one manual check uses real pi.
7. A long token stream crosses the bridge as small deltas, the Dart main isolate never waits for Rust, and closing a view releases its streams.
8. Quitting the app ends every pi process; a crashed pi process shows an error instead of hanging.
9. Comparable observations for token streaming and long Trajectories are recorded under [PERFORMANCE.md](PERFORMANCE.md#liquid), without thresholds.

**Closeout:** green CI plus one final round of the five KPI reviews on the final candidate, then Autoreview, Clawpatch and doc_sync as AGENTS.md orders them. A review is clean without Blocker or Major findings; Minor findings are fixed when trivial and otherwise listed under the follow-ups below.

**Follow-ups:**

- Keep Liquid key profiles consistent with the native mapping when disabling fails.
- Keep stored Liquid key profiles disabled on unsupported hosts.
- Preserve both Appearance selections when they change before the next frame.
- Close the cheap record gaps (ADR-0222): show the nested tool calls pi records, and say in the Diff and Trajectory tabs exactly what they cover, after an approved mockup.
- On a Mac keyboard with the Liquid key on, ⇪1, ⇪2 and ⇪3 work, but ⇪L and ⇪N do nothing and type nothing either (hardware test, 2026-10-28), while widget tests with the same keys pass. Next: run Liquid from a terminal with `LIQUID_KEY_LOG=1`, press ⇪1, ⇪L and ⇪N, and compare the native and Flutter lines for the letters ([liquid/README.md](liquid/README.md#run)).
- Closeout review leftovers (2026-10-30, delta review 2026-10-31), Minor findings and decided follow-ups:
  - Code: one helper for estimating tokens from characters (inspector and context track), one saved-settings controller with one way to report a failed save for Appearance, Diff and Shortcuts, one step-kind color mapping for tags, timeline and tree, one line-height helper in the Trajectory view, compare mode's segment shifting beside `compareBranches`, `SettingsPage` in its own file apart from the shortcut recorder, tokens instead of the overview's raw widths, and dropping the unused `Branch.lane` and `BranchMarker.lane`.
  - Robustness and tests: coalescing the agent stream while the UI falls behind (client architecture rule 4); a paused-time test of the forwarder's 50 ms window end to end; both key logs can miss a Liquid key release that another app, hotkey or panel takes, and then log typing until the next press.
  - UI: keyboard and screen-reader access to the timeline's range handles; while comparing, return, ⌘F and the Filter button still change the hidden inspector and filter row; mockup details still open: esc in the list closing the filter row after the range and the inspector, under the label "Close the step"; a jump outside a limiting range clearing it rather than moving it there; the Filter button's accent while filters are on, the inspector tabs' accent underline, the legend's share of the request, the collapsed tree's label.
  - Docs: trim ADR-0228 and ADR-0233 to their decisions and links, and move the two rules found only in the log (a refused change is undone on its device, the CLI/API returns a ready reference for every change) to their canonical docs.
  - CI follow-ups (2026-10-31): the timing-dependent macOS test `runner_term_grace` failed once (its 100 ms deadline can send TERM before the shell sets its trap); the detached-helper test in `tests/test_self_protection_inventory.py` hangs the suite when its case fails before release; `tests/test_m1_validation_contract.py` fails once no decision is open; a pull request update starts a push and a pull request run, which together can request more than the five macOS runners.
  - Final delta review (2026-10-31), Minor: keep `path`/`file_path` whole only as a bounded string for file-changing tools; trajectory.rs's comment on parallel tools, the duplicate path argument names in pi.rs and the session test's unquoted fixture path; the navigation mockup's "outside the range" for timeline clicks; a refused Liquid key removal tells the user only an exit status.
  - Clawpatch on the checkpoint branch (2026-11-01), predating it: test fixtures can write an inherited `GIT_INDEX_FILE`; the linked-FIFO test in `tests/stored.rs` can hang; the trajectory drops superseded compaction summaries from context details and can leave the preceding response outside the freshness window; the supervisor loses a question's deadline under event backpressure; `ready.rs` does not enforce the startup event limit; pending bridge queries block Stop and answers; a tool that settles unfinished loses its partial output.
  - Revert reviews (2026-11-01): open a session file that may be a pipe without waiting and refuse what is no plain file, in `stored.rs` and `History::from_file`.
  - Session removal reviews (2026-11-01), Minor: listing refs is capped at 4 MiB of Git output, so past about 45,000 checkpoint refs listing and removing them fail; a turn's end that takes over two seconds after a stop refuses deleting the session just left until retried; a session open but idle in another window can be deleted; a refusal cannot name a running turn whose name lock it could not take; a non-folder entry under `worktrees/` fails removing all with an unclear I/O error; a turn waiting while all checkpoints are removed says it waits for a revert; the row menu's button sits beside the sidebar's session card rather than inside it; a name file can change its inode under concurrent starts, so a running session's name can drop from the refusal while its locks still hold.
- A tool row in the chat overflows with a 64-character tool name (found 2026-11-08).
- At the smallest window, 1050 x 640, the session tree starts collapsed, so the Trajectory's Step column keeps about 190 px and the context track shows; the comparison still shows no step rows until scrolled.

#### Liquid Git integration — next slice

Completes the Git integration for the agent's Git project (ADR-0190, ADR-0191): branches, commits, push, pull requests and worktrees, plus per-turn checkpoints that the Diff tab shows and a user can revert the files to (ADR-0220, ADR-0234; [mockups](docs/design/mockups/liquid-checkpoints.html)). Meta-Harness runs the host's `git` and `gh`; Liquid provides the UI. Pull requests in order: checkpoints and the Diff tab on them (done), revert to a turn (done), deleting a session and removing all checkpoints (done), the branch in the header with a background fetch, switching and creating branches and stashing changes as GitHub Desktop does (done; [mockups](docs/design/mockups/liquid-git-branches.html)), whole-file commits with checkpoint attribution, the uncommitted Diff scope and explicit Git initialization (done), push, fast-forward pull and Fetch on request (done), then pull requests and worktrees ([mockups](docs/design/mockups/liquid-git-prs-worktrees.html), approved): pull requests part 1, creating one for a pushed branch through the host's `gh`, forks, the PR chip and its summary and the offer after a push (done); the title and description written by the Chat's model (done); “Commit, push & create PR” (done); then worktrees: listing and removing them in Settings › Worktrees (done), the rest with Chats (ADR-0239), which they need: creating one from a new Chat's composer, its location and the last-Chat choice, the folder chip and moving an idle Chat.

- Follow-up (2026-11-03), pull requests part 1: the current session header has no ··· menu, so at narrow widths the branch chip, PR chip and Git action keep their places until the Chat header brings it; choosing a base lists the target's first 500 branches on GitHub, and another is taken only as typed; GitHub's comparison counts at most 300 files and 250 commits; an SSH host alias or an HTTPS port in a remote URL is refused, not resolved to its GitHub host; commits pushed from elsewhere between the recheck and the creation request join the new pull request unreviewed; base records name the device by its host name, which can change; a fork its parent's organization owns is compared as `owner:branch`, which GitHub may resolve to the parent's own branch of that name (creation names `head_repo`); the post-Push offer is matched by the displayed branch name, so switching to a branch that displays the same keeps it (creation still rechecks the branch).
- Follow-up (2026-11-06), Clawpatch on the pull request text branch, predating it: push destinations compare SSH URLs without their user names (`diff/sync.rs`); a merge in progress is detected by resolving `MERGE_HEAD` rather than reading its record; `api/workspace.rs` follows lock-file links on targets other than Unix; two core tests share the folder `watershed-no-git-<pid>`. Clawpatch on the combined dialog's branch (2026-11-06), predating it: a confirmed default-branch push is not bound to the resolved repository, and symbolic push refs are not resolved before that check (`diff/sync.rs`); a turn without its end checkpoint counts its changes as edits outside the session in the commit dialog's attribution (`checkpoint/git.rs`). Clawpatch on the worktrees branch (2026-11-06), predating it: a fast-forward pull can overwrite ignored local files (`diff/sync.rs`); the pull request text can leave out submodule updates (`diff/pull_text.rs`); the bounded-title test fixture lacks the late prompt (`tests/stored.rs`). Worktree removal reviews (2026-11-06): the status walks every file inside ignored folders, so a huge ignored tree can reach the time limit, and one helper could find the checkout locks both for the list and for `checkouts()`.
- Follow-up (2026-11-03): stash restoration skips deleted paths in `diff/revert.rs::repositories_in_the_way`; reject stashed deletions inside nested repositories before mutation, preserving their files, index and stash.

#### M3a — Local Workspace foundation

- Local replica as the only interactive store: SQLite with SQLCipher and FTS5 (ADR-0237).
- The ADR-0238 spike passed on macOS: Automerge converges on every case and refuses Role-denied changes identically, with Loro measured alongside ([evidence](liquid/benchmarks/CRDT_SPIKE_EVIDENCE.md)); `liquid-core`'s CI runs Automerge on Linux too, and Windows and Android follow with their clients.
- Delivery order, one PR each: `liquid-core` object model, mutation pipeline, Actions and History over Automerge, starting with areas, Pages, Text Blocks and Views, to which each later slice adds its objects (done); `liquid-storage` with SQLite, SQLCipher and FTS5 and the durable commit (done); the Workspace bridge calls (ADR-0216), with the database key in the operating system's keystore (done on macOS; Linux, Windows and Android follow with their clients); Explorer, Pages, Text Blocks and Sections (done); the other built-in Blocks, starting with the Database Block (its core with text, number, select, date and checkbox fields and indexed rows and its bridge calls done; the Table View after its [mockups](docs/design/mockups/liquid-database-table.html), showing Tables on a Page, adding them through slash commands and adding their fields and rows, and selecting and editing text, number, checkbox, select and date cells, the field menu and the row handle and its menu done, with sort and filter following in the Views slice, which stores them on the View; file, link-to-row, formula and button fields follow with attachments, Connections, Formulas and Automations); Views, copy and the deletion cascade; Arrange mode; PowerBar, with its FTS5 index; Connections and formulas; Automations; Roles and protection; History, revert and Restore version; Sources, Page icons, Markdown import and export, and Settings, History and Automations as dialogs.
- Each loaded Page keeps its View order as an index until it is closed. ⏎ places the new Text Block right after the current one, so writing at a Page's end keeps sort keys short, while writing above existing text lengthens them as the probe measured, to 204 characters after 1,000 lines ([evidence](liquid/benchmarks/WORKSPACE_CORE_EVIDENCE.md#probe-of-the-long-pages-snapshot)). The Flutter editor has no workload yet: input-to-presented-frame latency while typing in a long Page ([PERFORMANCE.md](PERFORMANCE.md#liquid)) is measured in the app with the History slice, which groups keystrokes.
- KPI review of the M3a Workspace (2026-11-08, `main` @ ec214200, all five KPIs, the UX one in the built macOS app): fixes in this order, one PR each — typing onto a selected cell losing its first key (macOS selects a focused field's text), empty checkbox cells drawing no box, the sidebar's invisible focus and hover, the slash command menu's placement and its opening on a quickly typed "/ta", then shared menu parts and the picker and field panel out of `table_block.dart`, one placement routine in `liquid-core`, `liquid-bridge`'s `api/workspace.rs` split into replica, subscriptions and messages, and one shared Workspace test setup; then the KPIs again. Decided with it: ADR-0249 (a missing replica file, after its mockup), ADR-0250 (purge, with History) and ADR-0251 (size caps). Minors fixed where their code is touched: tied sort keys under `Place::After`, a cap on a stored sort key's length (with the `midpoint` fix below), `Text("")` stored as a value, open failures reported as a wrong key, the folder lock released before the database closes, a poisoned lock reused with uncommitted changes, a failed commit reusing its writer's sequence, number separators parsed loosely, quick ticks matched by value, a false warning after typing during a join, modified keys right after ⏎, a reread lost after a failed read, numbered-list style reads, quitting before a second write that follows an await, and a crash-durability test after an unclean exit.
- Left from the Table View so far: keys to insert and delete rows on a selected cell follow with their own mockup (Keyboard first).
- Left from the Explorer and Pages slice for later ones: toggles, inline marks and indentation in Text Blocks, the Section outline on hover, the Workspace switcher, Page icons, templates, the other Blocks, Existing Block… and Page templates in slash commands, Block handles that select, move and delete a whole Block, Liquid's own texts in the person's language (ADR-0246) and the device-local cache of View heights (client architecture rule 5).
- Runtime review of the Database bridge (2026-11-03), predating it: `sort_key::midpoint` recurses once per digit a key shares with its neighbor and copies the suffix while unwinding, so inserting again and again into one gap grows keys whose next key costs stack depth and quadratic copying; it becomes a risk only after hundreds of thousands of such inserts. Generate the fraction iteratively, with a long-key regression test.
- Tables (ADR-0244), after the Table View: finding a new field's kind from its first value, with a core mutation that changes a field's kind; with Formulas, calculated fields, the total row, live values in Text Blocks and the Calculation Block; with the Views slice, whether a row opens as a Page.
- Database bounds (ADR-0243): sync must keep a Database's reads within 500 fields and options when replicas add them concurrently; loading fields in pages or asynchronously may replace the caps.
- Date cells hold days only (ADR-0242); the Inbox and tasks slice decides how a time is stored, as the wall clock or as an instant with its zone, as Notion does.
- Review leftovers of the Explorer and Pages slice (2026-11-03), Minor: a Page reopened while its earlier queue writes enables its fields before the refreshed text and Views arrive; a refused text edit leaves later edits at offsets that assumed it; a refused rename restores the cached title, which can lag an earlier successful one; the to-do box cannot be reached or ticked from the keyboard.
- The app's structure (ADR-0239, ADR-0241): rail, tabs with splits, Chats as Blocks, the Inbox and Templates; mockups for implementation and their place in this order follow, after the change of `tokens.css` and the app to Notion's scale (ADR-0241). Calls and meetings come after the core works.
- Flutter clients over the Rust core follow the [client architecture](docs/concept/liquid-client-architecture.md) invariants (ADR-0182).
- Pages, Blocks, Views, Sources, Connections, Automations, Roles, History and settings.
- Explorer with Pages and Sources as peer navigation entries; Page templates create use-case compositions rather than new Page Types.
- PowerBar for finding Workspace objects, creating Blocks and invoking permitted Workspace actions.
- Blank Page and deterministic first-action creation: typing creates a Text Block, `/` opens slash commands, paste/drop/drawing select matching Blocks, and text shortcuts format inside Text Blocks.
- Content-first flow followed by explicit responsive Arrange mode. Mobile retains every Block; complex Views open focused surfaces. Safe compact OS widgets remain post-MVP.
- Built-in Text, Database, Chart, Code, Formula, Media/Embed and Whiteboard Blocks.
- Synchronized multi-Page Views over one canonical Block. Duplicate creates a new Block; deleting the last View deletes the Block, its Connections and owned Automations in one reversible Action, while independent dependent Automations are disabled for repair.
- Connections/formulas for direct logic and Workspace Automations for when/if/then logic.
- Allow-only Roles with default-denied unlisted access; explicit deny rules are post-MVP.
- One mutation pipeline and History for every M3a write surface, including UI, Connections/formulas, Automations and import. History and revert follow ADR-0235.

**M3a DoD:** offline, a user can create and navigate Pages/Sources, act immediately on a blank Page, edit and connect Blocks, reuse a Block through synchronized Views, arrange the Page responsively, assign a Role, run a simple Automation and recover the tested deletion cascade through History.

#### M3b — Apps and agent actions

- Restricted local JavaScript/TypeScript App Runtime with declarative UI, resource limits and explicit capabilities; WASM is a later extension.
- App Blocks and agents in Chats (ADR-0239), including permitted agent actions in the PowerBar.
- Web-link Sources, read-only and loaded only when opened, in a built-in browser (ADR-0236).
- Reviewing agent changes: what an agent changed is marked on the Page and filterable in History, from Automerge's attributed changes (ADR-0238), and the person can approve or revert it at any time; an approved mockup comes first.
- App SDK for AI/user-built Workspace-local Apps, with versioned state, Views and typed App actions.
- Workspace CLI/API (D-027) exposes compact typed reads, mutation proposals/execution and permitted App actions to agents.
- Liquid AI uses the same Role, capability, mutation and History boundaries.
- Liquid AI, CLI/API, App and Meta-Harness integrations join the same permissioned mutation/History pipeline.
- Optional Meta-Harness projections show friendly harness/adapter-configuration/location/session choices and separate config management while preserving instance identity, freshness and authority. Flow Agent selection requires an explicit Flow from its global authority; other adapters retain their documented selection model. Live commands route to the owning instance; a future portable checkpoint continued elsewhere appears as a new child Run/branch, not the same live process.

**M3b DoD:** a user can prompt Liquid AI to build an interactive App over connected Block data, use it without build/deploy/CI knowledge, inspect or restore its versions through History, share it within the Workspace and allow an agent to invoke only selected App actions through the CLI/API.

#### M3c — Central sync, mobile and headless execution

- Central Sync Server with resumable star-topology exchange, merging with Automerge 3 (ADR-0238); a concurrent edit of one field shows both values, after an approved mockup.
- Authorized user devices sync complete Workspaces and continue offline from local replicas.
- An optional headless replica receives only explicitly enabled Workspaces and exposes the same Role/mutation boundary to Automations, schedules and agents on its host while user devices are offline.
- Sync applies received actions through the same permissioned mutation/History pipeline.
- Sync, headless replicas and Meta-Harness remain separate logical roles even when one hosted deployment co-locates them.

**M3c DoD:** laptop, phone and an opted-in headless replica converge through the Sync Server; disconnecting any replica never blocks its local work; reconnection resumes without silent loss; a server agent can act only within its Workspace and Role.

#### M3d — Extension ecosystem and MCP adapters

- Versioned Block SDK and signed, sandboxed Block Registry for reusable community Block Types; no arbitrary third-party native code.
- MCP adapter maps external MCP capabilities to the same typed App actions and Role/capability boundary, with or without a visible App View.
- Import/export paths let integrations become Apps for users and compact CLI actions for agents without exposing MCP concepts in normal UX.

**M3d DoD:** an installed Block Registry package is isolated, upgradeable and responsive; an external MCP integration can be permissioned once, represented as an App or headless integration, and invoked through the same action contract from Liquid or an authorized agent.

## Later Flow Agent roadmap

These entries retain scope and decision context, not detailed future-feature specifications or release commitments.

### Tool-initiated configuration changes

**Deferred until after the first Flow Agent release (ADR-0173).** The purpose is to let Tools request limited configuration changes without direct access to Flow files. Manual administration is sufficient initially and avoids building a proposal protocol and approval UI before the core execution boundary is ready.

**Prior discussion:** The approved direction was Flow-applied, scope-limited proposals with default denial, optional local-owner approval or preconfigured limited permission. The initial scope was model/context-budget settings, not credentials, executable selection or Tool permissions. Review after Tool completion, bounded waiting, exact non-reusable consent and changes affecting future Runs were intended to avoid self-escalation and accidental repeated changes. Revisit the useful scope, unattended operation, review experience and conflict/recovery handling when scheduling this feature; retain no detailed protocol or diagram in advance.

### Protection of other independent Flow installations

**Discussion only; no feature commitment (ADR-0174).** The first release protects this installation's required objects under [SECURITY.md](SECURITY.md#native-self-protection-and-its-limits). Multiple Runs can share that home. Earlier discussion considered an administrator-listed set of other homes: it could protect deliberately separate installations from each other's Tools, but adds discovery, publication coordination and maintenance complexity. First establish whether users need separate homes instead of shared-home Runs; decide whether to add this protection before specifying it. No automatic whole-disk discovery or future delivery is approved.

## AgentPulse — last roadmap item

Low priority; scheduled after every other roadmap entry. Meta-Harness computes and stores AgentPulse v0 metrics — rework ratio, first-attempt success and cost-per-productive-outcome — and exposes them through its CLI/API. The formulas remain [D-010](docs/decisions/open-decisions.html#d-010).
