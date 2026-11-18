# Plan

Open implementation work per milestone, with deliverables and a Definition of Done (DoD). Performance architecture and evidence are canonical in `PERFORMANCE.md`.

## MVP boundary

The first MVP is **Flow Agent as a CLI-only harness**. It runs inside normal Git projects but does not own project history, auto-commit, branch management or any Watershed-specific **project-code** VCS engine. This is distinct from Liquid's internal **workspace** action history/VCS (over Liquid's own workspace data), which is in scope for the Liquid MVP (M3), and from the Git integration for agent projects, which T3 Code runs on Meta-Harness's host and Liquid presents, including its per-turn checkpoints (ADR-0282).

## Sequencing and adoption

Build the shared substrate, then Flow Agent, Meta-Harness and Liquid. This dependency/order-of-risk sequence validates execution, then multi-agent orchestration, then the integrated workspace without letting the broadest surface block earlier validation. All three remain independently usable layers of one platform; integration is canonical in `VISION.md`.

The initial adoption wedge is technical teams that need reusable, measurable, and reversible AI-agent workflows. Do not attempt separate adoption motions for all three layers before the Flow Agent and Meta-Harness wedges are validated. Long-term architecture must remain valid whether inference is local or remote: the provisioned core path is offline-capable, each Flow's model/runtime requirements resolve through a device-local Runtime binding, and cross-device continuation creates Conversation branches instead of transferring live host ownership. Canonical integration detail is in `VISION.md`.

## Milestones

### M1.2 — Flow-owned write protection

**Open:** verify the accepted boundary natively, prove the prebuilt installation contract and complete [repository review and closeout](AGENTS.md#session-workflow). [TESTING.md](TESTING.md#native-verification-status) owns native and downloaded-bundle acceptance results, [PERFORMANCE.md](PERFORMANCE.md#flow-agent) native measurements, [SECURITY.md](SECURITY.md#flow-agent-security-contract) the security and installation decisions and [PROTOCOL.md](PROTOCOL.md#m12-executor-protocol) the wire contract. Configuration administration stays manual for the first Flow Agent release; deferred work is in the [later roadmap](#later-flow-agent-roadmap).

**DoD:** Verify the accepted boundary on both native Flow Agent targets in [PLATFORMS.md](PLATFORMS.md), prove the prebuilt installation contract, and complete the canonical test, performance and review gates. Cross-compilation, historical isolation tests and mechanism experiments do not prove the replacement. Meta-Harness and Liquid never acquire ownership of Flow's Executor or Tool protection.

### M2 — Meta-Harness MVP

**Wedge:** Meta-Harness orchestration wedge — run Flow Agent and external agents durably, observably and together across harnesses, model providers and devices, leaving governance to each harness (ADR-0252). Emphasize transparent, self-hostable, AGPL-aligned control; do not frame this as a monetization step.

M2 delivers Meta-Harness as a **self-contained, host-scoped headless control plane** with CLI/API/service surfaces — usable without Liquid. Each instance controls only CLI agents on its own host. Its public API is one command set over a private local connection on the host and an authenticated network connection remotely (ADR-0185). Liquid integrates later as a client of one or more instances. Full product/runtime detail: [`docs/concept/V-Spec_MetaHarness.html`](docs/concept/V-Spec_MetaHarness.html).

**Deliverables:**

- Meta-Harness CLI run, session, schedule and metrics commands.
- Host links through the Liquid account and an end-to-end relay (ADR-0285); Tailscale carries them until then (ADR-0299).
- API/protocol surface for Liquid and BYOA: session registry, live event and transcript streams, artifact/log/handoff queries, config read/write proposals, agent schedule/trigger control, answers to a harness's own approvals and questions, revert.
- Central configuration for Flow Agent, which receives only its global authority and explicit Flows rather than a project-local config layer; device-local Runtime bindings remain owned by the execution host. Liquid edits the configuration across devices after sync and approved mockups.
- Control plane: session registry, routing, task state, attention state and agent schedule/trigger skeleton.
- Reference scenario for schedules: a scheduled agent keeps a news feed as a database page in Liquid and changes it only with the approval its Workspace Role requires, as `disler/pi-durable-news-agent` does with its own app.
- Host-local Meta-Harness agent executor that rejects cross-host agent-process control (on T3 Code: ADR-0280 to ADR-0291).
- Adapters: Flow Agent (via its public runtime surfaces) + at least one external CLI adapter.
- Orchestration tools for agents: T3 Code's own, across hosts through its peer links (ADR-0253, ADR-0288, [V-Spec](docs/concept/V-Spec_MetaHarness.html#orchestration)): an agent starts, messages, reads, steers and stops subagents of any harness and model, on other agent hosts too, and hosts link through the Liquid account (ADR-0285); Claude Code joins Codex as a T3 provider.
- Event/transcript ingestion from agents; artifact/log/handoff indexing (logs, structured summaries, host-provided diffs, handoff packs, checkpoints).
- **No rich standalone GUI**; Meta-Harness ships as CLI plus login service (ADR-0286).

**DoD:** monitor, steer and configure at least two different CLI agents on one agent host from one control surface, with both represented through one normalized session/event model; reject attempts to claim or control a process on another host; run without Liquid; expose the public API/protocol over both connections; integrate Flow Agent through its public runtime surfaces (not its internals); resolve shared config without duplicated per-agent config directories for the same capability; require approval plus an audit record for every sensitive config change; and let a session on one host start, message, steer and stop a subagent of another harness on another agent host through the orchestration tools.

### M3 — Liquid (staged standalone workspace product)

**Wedge:** prove safe human/agent co-editing of local-first Workspace state with attributed, reviewable and reversible History. Liquid remains useful without agents, network or a hosted service. Do not build a generic Notion clone.

The target product is canonical in [`docs/concept/V-Spec_Liquid.html`](docs/concept/V-Spec_Liquid.html). M3 is staged so workspace fundamentals prove value before sync, community extensions and broad integrations. Chat between people and a calendar follow after Release 1 (ADR-0236). Review leftovers are tracked as GitHub issues [#61](https://github.com/Open-Equilibrium/watershed/issues/61)–[#67](https://github.com/Open-Equilibrium/watershed/issues/67).

#### Liquid agent surface — early slice

The macOS Liquid app runs its agent sessions on a pinned T3 Code server through the Meta-Harness service (ADR-0215, ADR-0280). Open:

- Say in the Diff panel exactly what it covers (ADR-0222), copying T3 Code's wording where it has one.
- Project scripts in the session card, once Liquid has a terminal (ADR-0295).
- Android follows with host links (ADR-0285).

#### M3a — Local Workspace foundation

- KPI review of the M3a Workspace (second round, 2026-11-16): no Blocker or Major left. Minors fixed where their code is touched: `Text("")` stored as a value, open failures reported as a wrong key, a failed commit reusing its writer's sequence, number separators parsed loosely, quick ticks matched by value, a false warning after typing during a join or after other writes that change nothing (a number typed as before, spaces in an empty cell, `---` twice), modified keys right after ⏎, a reread lost after a failed read, numbered-list style reads, quitting before a second write that follows an await, a crash-durability test after an unclean exit, `SECURITY.md` promising the macOS keychain refuses other programs (it asks the person), a failed delete of the old path-named key entry blocking every open, the key's hex copy left in memory, change counts kept for resources no one follows, the slash command menu adding only Tables, `FakeWorkspace` re-implementing the core with no Dart test against the real bridge ([D-110](docs/decisions/open-decisions.html#d-110)), bridge tests repeating core tests, the read window's size and the Page a mutation touches known outside the core, and `SplitTextBlock`'s bridge doc missing the start-of-Block rule.
- Linux, Windows and Android follow with their clients, each keeping the database key in its operating system's keystore (ADR-0216); `liquid-core`'s CI, which runs Automerge on Linux and macOS, adds Windows and Android with them (ADR-0238).
- Delivery order, one PR each: Views on several Pages ([D-112](docs/decisions/open-decisions.html#d-112)), with Board, Calendar, grouping, copy and the deletion cascade; Arrange mode; PowerBar, with its FTS5 index; Connections and formulas, with the Chart and Formula Blocks that depend on them; Automations; Roles and protection; History, revert and Restore version; Sources, Page icons, Markdown import and export, and Settings, History and Automations as dialogs.
- Input-to-presented-frame latency while typing in a long Page ([PERFORMANCE.md](PERFORMANCE.md#liquid)) is measured in the app with the History slice, which groups keystrokes.
- A missing replica file is reported, not opened as a new Workspace, after its mockup (ADR-0249); purge of deleted content comes with the History slice, before sync (ADR-0250); the core refuses writes past its size caps (ADR-0251), which must also bound a Page's document as a whole: a drawing's caps leave its text and erased ink uncounted, and several Blocks add up.
- Left from the Media Block (ADR-0305), approved 2026-11-17: a pasted link becomes a link in its line, as in Microsoft Loop, with inline marks: Liquid's link icon and the link text the clipboard carries, else the link's host, fetching nothing; hovering shows the address with Edit, which changes the title and address, and Copy link, and a click opens the default browser. Embeds come with the built-in browser (M3b); paste on Windows, Linux and Android needs a clipboard reader each, as macOS has.
- Left from the Whiteboard Block (ADR-0306): undo with History, as for every Block; sticky notes, images on the board, arrows that stay attached to shapes, GoodNotes' circle to lasso, converting ink to a text box (low priority) and a pen stroke, or Draw in the first line, that starts a Whiteboard on an empty Page; a file going into a file cell shows no progress yet.
- Left from annotating (ADR-0307): Save as… with the notes written into a copy of the PDF, text selection and search in a document, annotating on a phone, a PDF over 100 MiB (it shows as a file), and a PDF renderer for Windows (Windows.Data.Pdf), Android (PdfRenderer) and Linux, decided with their clients.
- Left from the Table View so far: keys to insert and delete rows on a selected cell follow with their own mockup (Keyboard first); changing an option's name or color, once the core can; several sorts and filter conditions; Arrange mode changes a Table's fixed height.
- Left from the Explorer and Pages slice for later ones: toggles, inline marks and indentation in Text Blocks, the Section outline on hover, the Workspace switcher, Page icons, templates, the other Blocks, Existing Block… and Page templates in slash commands, Block handles that select, move and delete a whole Block, Liquid's own texts in the person's language (ADR-0246) and the device-local cache of View heights (client architecture rule 5).
- A Page title keystroke rereads the area's whole Page tree; fixing it needs the bridge's change events to tell a rename apart (ADR-0216 rule 3), so it is decided together with a virtualized Explorer.
- Tables (ADR-0244), after the Table View: field formats for decimals, thousands, percent and currency; the link-to-row, formula and button fields follow with Connections, Formulas and Automations; finding a new field's kind from its first value, with a core mutation that changes a field's kind; with Formulas, calculated fields, the total row, live values in Text Blocks and the Calculation Block; with Views on several Pages, whether a row opens as a Page.
- Database and drawing bounds (ADR-0243, ADR-0306, ADR-0307): sync must keep a Database's reads within 500 fields and options, and a drawing's within its element and point caps, per page and in all, when replicas add them concurrently; loading fields in pages or asynchronously may replace the Database caps.
- Date cells hold days only (ADR-0242); the Inbox and tasks slice decides how a time is stored, as the wall clock or as an instant with its zone, as Notion does.
- The app's structure (ADR-0239, ADR-0241): rail, tabs with splits, Chats as Blocks, the Inbox and Templates; mockups for implementation and their place in this order follow. Calls and meetings come after the core works.
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
- ⏎ at the start of a line that shares its sort key with the line above gives that line a new key first (ADR-0303). ⏎ inside such a line moves its tail past the whole tied group today, across another Block or Section; how that split is placed is decided with this work.
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

### Deferred Flow Agent features

Later milestones may add typed projections or mappings between Flow, Phase, Instruction, subflow, Tool and addressable artifact inputs/outputs; backward edges or an expression language; general retry/fallback policies; and automatic Tool imports referenced by Instructions. Dynamic proposals to add a Phase, child Phase or Flow at run time—including one-run versus persisted approval and an operator opt-out—also remain deferred. A durable ordered Conversation-status inventory may replace the query-only CV-03 admission bound when larger inventories are required; it must define transaction, recovery and consistency rules without replacing Conversation authority. Any future routing or proposal surface requires a finite schema, permissions, complete allowed/rejected matrix, provenance, replay and approval decision before enablement. The M1.1 routing boundary is canonical in `PROTOCOL.md`.

### Local inference and portable continuation

Accepted targets ([Flow Agent V-Spec](docs/concept/V-Spec_FlowAgent.html#accepted-target)): a provider-neutral local-inference path that runs the complete local core path on a provisioned host with network interfaces disabled, and a public, versioned Portable continuation contract whose imported terminal checkpoint creates a child Run on another device. Open decisions: provisioning, model/runtime evidence and resource admission ([D-059](docs/decisions/open-decisions.html#d-059)), the local inference process trust boundary ([D-061](docs/decisions/open-decisions.html#d-061)), archive, transfer, fencing and tree navigation ([D-058](docs/decisions/open-decisions.html#d-058)), archive authenticity ([D-062](docs/decisions/open-decisions.html#d-062)) and offline approvals and revocation ([D-060](docs/decisions/open-decisions.html#d-060)).

### Tool-initiated configuration changes

**Deferred until after the first Flow Agent release (ADR-0173).** The purpose is to let Tools request limited configuration changes without direct access to Flow files. Manual administration is sufficient initially and avoids building a proposal protocol and approval UI before the core execution boundary is ready.

**Prior discussion:** The approved direction was Flow-applied, scope-limited proposals with default denial, optional local-owner approval or preconfigured limited permission. The initial scope was model/context-budget settings, not credentials, executable selection or Tool permissions. Review after Tool completion, bounded waiting, exact non-reusable consent and changes affecting future Runs were intended to avoid self-escalation and accidental repeated changes. Revisit the useful scope, unattended operation, review experience and conflict/recovery handling when scheduling this feature; retain no detailed protocol or diagram in advance.

### Protection of other independent Flow installations

**Discussion only; no feature commitment (ADR-0174).** The first release protects this installation's required objects under [SECURITY.md](SECURITY.md#flow-owned-write-protection-and-its-limits). Multiple Runs can share that home. Earlier discussion considered an administrator-listed set of other homes: it could protect deliberately separate installations from each other's Tools, but adds discovery, publication coordination and maintenance complexity. First establish whether users need separate homes instead of shared-home Runs; decide whether to add this protection before specifying it. No automatic whole-disk discovery or future delivery is approved.

## AgentPulse — last roadmap item

Low priority; scheduled after every other roadmap entry. Meta-Harness computes and stores AgentPulse v0 metrics — rework ratio, first-attempt success and cost-per-productive-outcome — and exposes them through its CLI/API. The formulas remain [D-010](docs/decisions/open-decisions.html#d-010).
