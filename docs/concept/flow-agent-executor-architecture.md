# Flow Agent execution and security architecture

Diagrams of the boundaries in the normative [security contract](../../SECURITY.md#flow-agent-security-contract), which owns guarantees and responsibilities; [PROTOCOL.md](../../PROTOCOL.md#m12-executor-protocol) owns the Executor wire and [TESTING.md](../../TESTING.md#native-verification-status) native results. The cases are representative authority paths, not an exploit enumeration.

## 1. Ordinary or invalid request

Only a valid in-scope invocation runs; instructions are context, never authority.

```mermaid
flowchart TD
  Instructions["Editable Workspace AGENTS.md: context, not authority"] --> Input
  Global["Protected global AGENTS.md: context, not authority"] --> Input
  Input["Model requests Tool with parameters"] --> Check{"Available here and valid?"}
  Check -->|No| Reject["Reject before Tool effects"]
  Check -->|Yes| Ready{"Mandatory Flow-owned write protection ready?"}
  Ready -->|No| Stop["Fail before launch"]
  Ready -->|Yes| Intent["Persist intent; retain same Executor after Ready"]
  Intent --> Start["Commit tool.started; send matching Start"]
  Start --> Run["Establish protection; execute trusted Tool"]
  Run --> Result["Validate enforcement receipt; persist bounded result"]
```

## 2. Direct protected write

Native denial, including children; no prompt, escalation or retry.

```mermaid
sequenceDiagram
  participant T as Tool or new helper
  participant G as Flow-owned write protection
  participant P as Protected Flow files
  participant F as Flow Agent
  T->>G: Write, delete or replace protected object
  G-->>T: Denied, protected object unchanged
  T-->>F: Tool result or failure
  Note over F: No permission popup inferred from an OS error
  Note over F: No restart with broader direct-write authority
```

## 3. Compromised Tool

Direct protected writes stay blocked; other host effects and false results are not contained.

```mermaid
flowchart TD
  Tool["Compromised Tool under Flow-owned write protection"] --> FlowWrite["Direct write to protected Flow file"]
  FlowWrite --> Denied["Blocked by mandatory Flow-owned write protection"]
  Tool --> Project["Other host files or network effects"]
  Project --> Authority["Possible within available OS authority"]
  Tool --> Lie["False result returned to Flow"]
  Lie --> Untrusted["Result is not proof of correctness"]
```

## 4. Compromised new helper

The helper inherits the restriction; the trusted-dependency assumption is nevertheless broken.

```mermaid
flowchart TD
  Good["Correct Tool"] --> Launch["Starts a helper or executes project code"]
  Launch --> Bad["Compromised code inherits native restriction"]
  Bad --> Direct["Direct protected Flow write"]
  Direct --> Denied["Blocked"]
  Bad --> Other["Other effects or false output"]
  Other --> Risk["Not generally contained by Flow"]
```

## 5. Independent third-party service

The service acts with its own authority, outside the Tool write guard.

```mermaid
sequenceDiagram
  participant T as Tool inside boundary
  participant S as Independent local application or service
  participant P as Host files including Flow files
  T->>S: Request an action through an allowed integration
  Note over S: Separate process authority, not a newly confined child
  S->>P: Perform action with its own OS rights
  Note over S,P: Flow's Tool write guard does not govern this access
```

## 6. Missing boundary or interrupted execution

No unprotected launch; post-launch uncertainty is neither success nor permission to replay.

```mermaid
flowchart TD
  Prepare["Prepare Tool execution"] --> Guard{"Required protection established?"}
  Guard -->|No| Refuse["No Tool launch and no weaker fallback"]
  Guard -->|Yes| Start["Durable authorization then Tool launch"]
  Start --> Terminal{"Reliable terminal evidence?"}
  Terminal -->|Yes| Record["Record observed outcome"]
  Terminal -->|Missing after dispatch| Unknown["Stop further dispatch; preserve uncertainty"]
  Unknown --> Cleanup["Attempt supported cleanup; report only what is known"]
  Cleanup --> NoReplay["Do not automatically repeat uncertain effects"]
```

## 7. Parallel agents

Run ownership neither serializes shared project edits nor reserves host resources.

```mermaid
flowchart TD
  A["Flow Run A"] --> StoreA["Owned Run A state"]
  B["Flow Run B"] --> StoreB["Owned Run B state"]
  A --> Config["Same-user global configuration authority"]
  B --> Config
  A --> ToolA["Tool A"]
  B --> ToolB["Tool B"]
  ToolA --> Project["Shared writable project files"]
  ToolB --> Project
  Project --> Race["Concurrent edits need separate coordination"]
  ToolA --> Host["Shared CPU, RAM, disk and GPU"]
  ToolB --> Host
```

## 8. External sandbox

Extra containment works only when the nested prerequisites do; no fallback.

```mermaid
flowchart TD
  Outer["Optional deployment-owned sandbox"] --> Flow["Flow Agent and its Executor"]
  Flow --> Ready{"Required inner protection and runtime access available?"}
  Ready -->|No| Refuse["Reject unsupported deployment; no unprotected fallback"]
  Ready -->|Yes| Inner["Flow-owned write protection"]
  Inner --> Tool["Tool operates under both sets of restrictions"]
```

## Protected-set admission and publication

One controller admission precedes Tool launches; Flow publishers stay inside protected directories.

```mermaid
flowchart TD
  Select["Controller selects this home's stores and installed images"] --> Admit["Anchor objects and verify all covered aliases under publication lease"]
  Admit -->|Incomplete, changed or external alias| Refuse["Explain start error; no Tool; no automatic repair"]
  Admit -->|Complete| Prepare["Bind paths as data; remove inherited authority; prepare native restriction"]
  Prepare -->|Not supported or not verified| Refuse
  Prepare -->|Ready and durable launch authorized| Run["Start Tool and inherited helpers"]
  Flow["Trusted Flow publisher"] --> Publish["Append or atomically publish inside existing protected directories"]
  Publish --> Covered["Old and new names remain protected"]
  Run -->|Direct write, alias or move attempt| Block["OS denies operation"]
  Run -->|Ordinary project operation| Host["Tool's trusted-code responsibility"]
```
