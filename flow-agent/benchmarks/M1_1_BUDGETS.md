# M1.1 Execution Budgets and Evidence Matrix

This is the canonical approved M1.1 numeric contract for the maintainer-selected E3 evidence scope and balanced bundle A. Its original counting rules, fixtures and exclusions are fixed by ADR-0107; ADR-0123 adds CV-15, ADR-0124 adds PR-01/PR-02 without adding optimized workloads and ADR-0127 adds CV-16 through CV-18 plus one optimized workload.

## Evidence rules

- Binary units are used. A byte cap counts the bytes identified in the row, not Unicode scalars, allocator capacity or a transport's framing overhead.
- `F:<name>` is a required functional test in the workspace `nextest` gate. OAuth, Responses, authoring and conversation proofs run on Ubuntu, macOS and Windows. Runner proofs run on the enabled Ubuntu and macOS targets; the existing Windows fail-closed proof remains required but is not a positive runner workload. For a byte or count cap `N`, a valid `N` fixture succeeds and `N + 1` fails before unbounded allocation or productive effects. Parameterized rows execute every named field or operation separately. Required malformed, missing, zero, negative and checked-arithmetic cases remain part of the relevant protocol test even when they are not additional numeric selections.
- A deadline test uses a fake monotonic clock unless the row explicitly requires a child process. At `limit - 1 ns` the operation remains eligible; at `limit` it reaches exactly one terminal timeout path; advancing past the limit cannot create another effect or terminal result.
- `P:<name>` is a release-mode, one-process-at-a-time benchmark on the fixed `ubuntu-24.04` x64 runner. Five warmups and 30 measured fresh-child samples produce `target/m11-budgets/m11-budgets.jsonl` with schema `flow-m11-budget-v0`. Timing gates compare the measured p95; RSS gates compare the maximum unadjusted `post-workload VmHWM - pre-workload VmRSS`. A gate passes only when every applicable aggregate is at or below its selected limit; an exceedance is reviewed under `PERFORMANCE.md` before implementation or target changes.
- `P:rss_detection_fixture` is always present in the optimized artifact. It allocates, touches and frees exactly 4 MiB in a fresh child and must report at least 4 MiB minus the documented 512 KiB Linux accounting tolerance. It is measurement validation, not a product RSS gate.
- `F`-only rows are intentionally excluded from maximum-load timing/RSS measurement because they are parser, cardinality or fake-clock safety boundaries rather than performance claims. This is the E3 separation; every explicit p95 or RSS gate still has a `P` row.
- The optimized report records the commit, runner image/version, CPU, logical CPU count, total memory, exact inputs, exclusions, raw samples and aggregates. CI uploads the JSONL artifact even when a budget comparison fails.

The functional proofs run in the workspace `nextest` gate. The optimized reference command is:

```sh
mkdir -p target/m11-budgets
cargo run --locked -p flow-agent-core --release \
  --features m11-budget-evidence --example m11_budgets \
  > target/m11-budgets/m11-budgets.jsonl
```

Run it on the fixed Ubuntu reference runner. A nonzero exit means at least one workload or measurement-validity gate failed; the JSONL report is still complete and must be retained.

## OAuth and authentication

HTTP body caps count decoded response-body bytes delivered by the HTTP client, before JSON parsing. JSON string caps count decoded UTF-8 value bytes. The callback cap covers the complete HTTP head before field decoding.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| OA-01 | Callback HTTP head: 16 KiB | `F:oauth_callback_http_head_budget`; one valid request head is padded to exactly 16,384 bytes and 16,385 rejects before field decoding. Generated state is compared exactly; callback fields have no separate byte caps inside this envelope. | F-only: parser allocation boundary. |
| OA-02 | User-code body: 64 KiB | `F:oauth_user_code_body_budget`; the production request transport receives the 65,536-byte cap, accepts exact padded JSON and rejects 65,537 bytes before delivery. | F-only: transport and allocation boundary. |
| OA-03 | Device-poll body: 64 KiB | `F:oauth_device_poll_body_budget`; the production poll transport receives the 65,536-byte cap, accepts exact padded JSON and rejects 65,537 bytes before delivery. | F-only: transport and allocation boundary. |
| OA-04 | Token body: 1 MiB | `F:oauth_token_body_budget`; the production exchange transport receives the 1,048,576-byte cap, accepts exact padded JSON and rejects 1,048,577 bytes before delivery. | F-only: transport and allocation boundary. |
| OA-05 | Refresh body: 1 MiB | `F:oauth_refresh_body_budget`; the OA-04 construction on the refresh response path. | F-only: parser allocation boundary. |
| OA-08 | PKCE verifier: 8 KiB | `F:oauth_verifier_field_budget`; exact decoded values at 8,192/8,193 bytes. | F-only: decoded-field boundary. |
| OA-09 | Device-auth id: 8 KiB | `F:oauth_device_authorization_field_budgets`; exact decoded JSON strings at 8,192/8,193 bytes. | F-only: decoded-field boundary. |
| OA-10 | User code: 8 KiB | `F:oauth_device_authorization_field_budgets`; exact decoded JSON strings at 8,192/8,193 bytes. | F-only: decoded-field boundary. |
| OA-11 | Verification URI: 8 KiB | `F:oauth_device_authorization_field_budgets`; exact decoded JSON strings at 8,192/8,193 bytes. | F-only: decoded-field boundary. |
| OA-12 | Account id: 8 KiB | `F:oauth_account_id_field_budget`; exact decoded JWT-claim strings at 8,192/8,193 bytes. | F-only: decoded-field boundary. |
| OA-13 | Access token: 64 KiB | `F:oauth_access_and_id_jwt_token_field_budget`; exact decoded JSON strings at 65,536/65,537 bytes on login and refresh paths. | F-only: secret-field boundary; no secret enters an artifact. |
| OA-14 | Refresh token: 64 KiB | `F:oauth_refresh_token_field_budget`; exact decoded JSON strings at 65,536/65,537 bytes on login and refresh paths. | F-only: secret-field boundary; no secret enters an artifact. |
| OA-15 | ID/JWT token: 64 KiB | `F:oauth_access_and_id_jwt_token_field_budget`; syntactically valid JWT fixtures with exact total string lengths 65,536/65,537 bytes. | F-only: secret-field boundary; no secret enters an artifact. |
| OA-16 | Polling interval: at most 60 s | `F:oauth_poll_interval_budget`; integer and trimmed-string `60` succeed, `61` rejects, `slow_down` accepts 55 + 5 and rejects 60 + 5 with checked addition. | F-only: fake-clock/protocol boundary. |
| OA-17 | `expires_in`: at most 86,400 s | `F:oauth_expiry_budget`; integer `86400` succeeds, `86401` rejects, and millisecond/epoch overflow rejects before replacement. | F-only: arithmetic/protocol boundary. |
| OA-18 | Credential-store lock: 5 s | `F:credential_lock_deadline`; a competing holder exercises 5 s through the deadline convention and proves the prior record is unchanged. | F-only: fake-clock contention lifecycle. |
| OA-19 | HTTP connect deadline: 10 s | `F:auth_connect_deadline`; fake monotonic time keeps a production client request pending through 10 s - 1 ns and observes its sole timeout at 10 s. | F-only: exact production deadline lifecycle. |
| OA-20 | Response-header deadline: 30 s | `F:auth_header_deadline`; fake monotonic time keeps the production request pending through 30 s - 1 ns and observes its sole timeout at 30 s. | F-only: exact production deadline lifecycle. |
| OA-21 | Response-body deadline: 30 s | `F:auth_body_deadline`; after headers, fake monotonic time keeps the production body read pending through 30 s - 1 ns and observes its sole timeout at 30 s. | F-only: exact production deadline lifecycle. |
| OA-22 | Complete authentication request: 60 s | `F:auth_overall_deadline`; body progress cannot extend the production request past the sole exact 60 s fake-time timeout. | F-only: exact production deadline lifecycle. |
| OA-23 | Complete device poll: 15 min | `F:device_poll_overall_deadline`; pending and `slow_down` responses cannot extend 900 s. | F-only: fake-clock protocol lifecycle. |

## Responses stream

A raw SSE line excludes its LF delimiter and an immediately preceding CR. An assembled event counts decoded `data:` payload bytes plus each SSE-mandated inserted LF. A decoded stream sums the canonical JSON bytes of every accepted non-sentinel decoded event; SSE framing, inserted line feeds and the terminal sentinel do not count. The event count includes every dispatched SSE event, including a terminal sentinel. Each `response.output_item.done` event carries exactly one item.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| RS-01 | Raw SSE line: 256 KiB | `F:responses_line_budget`; valid comment/data lines at 262,144 bytes and the same line plus one byte. | F-only: incremental parser boundary. |
| RS-02 | Assembled event: 1 MiB | `F:responses_event_budget`; multiple individually valid data lines assemble to 1,048,576/1,048,577 bytes. | F-only: parser aggregation boundary. |
| RS-06 | Events: 4,096 | `F:responses_event_count_budget`; 4,096 minimal valid events complete and event 4,097 rejects before dispatch. | F-only: cardinality boundary. |
| RS-08 | Connect deadline: 10 s | `F:responses_connect_deadline`; fake monotonic time keeps a production client request pending through 10 s - 1 ns and observes its sole timeout at 10 s. | F-only: exact production deadline lifecycle. |
| RS-09 | Response-header deadline: 30 s | `F:responses_header_deadline`; fake monotonic time keeps the production request pending through 30 s - 1 ns and observes its sole timeout at 30 s. | F-only: exact production deadline lifecycle. |
| RS-10 | Idle/inter-event deadline: 120 s | `F:responses_idle_deadline`; after one event, fake monotonic time keeps the production stream pending through 120 s - 1 ns and observes its sole timeout at 120 s. | F-only: exact production deadline lifecycle. |
| RS-11 | Complete response deadline: 30 min | `F:responses_overall_deadline`; stream progress cannot extend the production request past the sole exact 1,800 s fake-time timeout. | F-only: exact production deadline lifecycle. |
| RS-12 | Retained provider input: 64 MiB | `F:provider_input_aggregate_budget`; a complete retained-input array at 67,108,864 canonical bytes succeeds and 67,108,865 fails with no later provider or Tool dispatch. | F-only: canonical aggregate boundary. |
| RS-13 | Decoded SSE stream: 32 MiB | `F:responses_decoded_stream_budget_is_checked_before_retention`; accepted non-sentinel events sum to 33,554,432 canonical bytes, then one additional byte rejects before retention. | F-only: decoder aggregate boundary. |
| RS-14 | Definitive provider error message: 4,000 Unicode characters | `F:definitive_http_failure_reports_status_and_bounded_provider_message`; the optional HTTP status is preserved and the provider message is truncated at exactly 4,000 characters. | F-only: persisted diagnostic boundary. |

## Productive dispatch reservation

The concrete Run writer inventories current stored usage and admits the complete applicable envelope before durable attempt intent. Reservation uses checked sums against the 352 MiB/22-segment event stream, 48 MiB context stream, 16 MiB metadata, 5,216 MiB and 131,072-object inventory, and unchanged 5.5 GiB bundle. The bounded route to the next dispatch and its lifecycle closure require 69 maximum-size Event records plus 33 objects totaling 18 MiB.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| PR-01 | Provider dispatch envelope | `F:provider_dispatch_reservation`; exact compiled context bytes/objects, 64 MiB durable provider output in at most four objects, 1,094 Event records, 512 KiB aggregate Run Log/recovery growth and bounded inter-dispatch/lifecycle closure fit at exact remaining capacity; one byte or object beyond any selected limit rejects before intent or request. Maximum six-byte JSON escaping is exercised across the maximum 1,025 UTF-8-safe message deltas of at most 32 KiB each. | F-only: storage-admission invariant. |
| PR-02 | Tool dispatch envelope | `F:tool_dispatch_reservation`; both exact 4 MiB output objects, 71 Event records, 512 KiB aggregate Run Log/recovery growth and bounded inter-dispatch/lifecycle closure fit at exact remaining capacity; one byte or object beyond any selected limit rejects before intent or process launch. | F-only: storage-admission invariant. |

## Tool runner

Stream caps count raw bytes read from each pipe before UTF-8 classification.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| TR-01 | Stdout: 4 MiB | `F:runner_stdout_budget`; a child emits 4,194,304 bytes, then 4,194,305 bytes, with stderr empty. | `P:runner_dual_stream_caps` also emits exactly 4 MiB on both pipes concurrently. |
| TR-02 | Stderr: 4 MiB | `F:runner_stderr_budget`; the TR-01 pair on stderr with stdout empty. | Covered by `P:runner_dual_stream_caps`. |
| TR-03 | TERM grace: 1 s | `F:runner_term_grace`; a ready child ignores TERM and proves TERM precedes escalation by exactly the deadline convention. | `P:runner_termination`, p95 <= 5 ms, measures one ready child that exits on TERM; the ignored-TERM wait is excluded because the selected second is a safety timeout, not a latency target. |
| TR-04 | Forced reap: 1 s | `F:runner_forced_reap`; an escalated process-group leader plus inherited child exercises the exact wait and one terminal forced-reap failure. | F-only: intentional timeout path. |
| TR-05 | Output drain: 1 s | `F:runner_output_drain`; a descendant retaining both pipe handles forces controller closure at the deadline and one terminal drain failure. | F-only: intentional timeout path; ordinary EOF is covered by the stream benchmark. |
| TR-06 | Exec-vector entries: 2,048 | `F:runner_exec_entry_budget`; exact complete vectors with 2,048/2,049 entries include executable and generated parameter tokens. | F-only: construction boundary. |
| TR-07 | Encoded exec vector: 128 KiB | `F:runner_exec_byte_budget`; exact 131,072/131,073-byte vectors cover strings, terminators and pointer arrays on each supported pointer width. | F-only: construction boundary. |
| TR-08 | Four no-op launches: p95 10 ms | `F:runner_noop_lifecycle`; four direct-exec children each produce exactly one terminal result. | `P:runner_four_noop_launches`, four sequential direct executable launches per sample, p95 <= 10 ms. |
| TR-09 | Termination: p95 5 ms | Covered by TR-03 lifecycle proof. | `P:runner_termination`, p95 <= 5 ms from post-ready TERM request through reaping and EOF. |
| TR-10 | Cancellation: p95 5 ms | `F:runner_cancellation_lifecycle`; one ready child produces one cancelled result and no relaunch. | `P:runner_cancellation`, p95 <= 5 ms from post-ready cancellation through reaping and EOF. |
| TR-11 | Dual-stream collection: p95 100 ms | Covered by TR-01/TR-02 boundary proofs. | `P:runner_dual_stream_caps`, exact 4 MiB stdout plus 4 MiB stderr written concurrently, p95 <= 100 ms. |
| TR-12 | Dual-stream peak RSS growth: 12 MiB | Covered by TR-01/TR-02 boundary proofs. | The same `P:runner_dual_stream_caps` sample has maximum peak growth <= 12 MiB. |

## Authoring

Definition and registry sizes count raw file bytes. Generated maximum-size definitions are real one-block UTF-8 YAML documents; semantically ignored trailing YAML comments provide exact byte padding. A maximum registry has 1,024 valid one-block entries padded to exactly 16 MiB total, with no file above 128 KiB.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| AU-01 | Definition file: 128 KiB | `F:authoring_definition_budget`; Create and Validate accept a valid 131,072-byte definition and reject 131,073 while reading. | `P:authoring_max_definition_transaction` uses the valid 128 KiB Tool definition. |
| AU-02 | Registry bytes: 16 MiB | `F:authoring_registry_byte_budget`; Validate accepts 16,777,216 total bytes and rejects one additional byte before retaining more source. | `P:authoring_max_registry_validate` uses the exact 16 MiB registry. |
| AU-03 | Registry entries: 1,024 | `F:authoring_registry_entry_budget`; Validate accepts 1,024 minimal entries and rejects entry 1,025. | `P:authoring_max_registry_validate` uses exactly 1,024 entries. |
| AU-04 | Maximum-definition transaction: p95 25 ms | `F:authoring_transaction_roundtrip`; stage, sync, no-replace publish, reload and semantic round-trip preserve exact bytes. | `P:authoring_max_definition_transaction`, p95 <= 25 ms. |
| AU-05 | Maximum-definition transaction RSS: 4 MiB | Covered by AU-04. | The same `P:authoring_max_definition_transaction` sample has maximum peak growth <= 4 MiB. |
| AU-06 | Init: p95 50 ms | `F:authoring_init_transaction`; one empty workspace reaches the complete durable initialized state and recovers each transition. | `P:authoring_init`, one empty workspace and default registry root per sample, p95 <= 50 ms. |
| AU-07 | Init RSS: 4 MiB | Covered by AU-06. | The same `P:authoring_init` sample has maximum peak growth <= 4 MiB. |
| AU-08 | Maximum-registry Validate: p95 2 s | Covered by AU-01 through AU-03. | `P:authoring_max_registry_validate`, exact 1,024-entry/16 MiB registry, p95 <= 2 s. |
| AU-09 | Maximum-registry Validate RSS: 32 MiB | Covered by AU-01 through AU-03. | The same `P:authoring_max_registry_validate` sample has maximum peak growth <= 32 MiB. |

## Conversations and Run Logs

A canonical-record cap counts canonical JSON bytes without the JSONL LF. Rotation counts stored bytes including each LF and rotates before an append would exceed 16 MiB. A scan quantum admits a complete next record only when both its record count and stored bytes remain within the selected limits. A page admits complete records in source order until the next record would exceed either the count or canonical output limit.

| ID | Selected contract | Exact fixture and functional proof | Optimized proof or E3 exclusion |
|---|---|---|---|
| CV-01 | Canonical record: 256 KiB | `F:conversation_record_budget`; canonical records at 262,144/262,145 bytes, before LF. | F-only: record parser boundary. |
| CV-02 | Rotation segment: 16 MiB | `F:conversation_rotation_budget`; appends fill exactly 16,777,216 stored bytes and the next complete record starts a new segment. | F-only: storage lifecycle boundary. |
| CV-03 | Scan quantum records/entries: 4,096 | `F:conversation_scan_count_budget`; exactly 4,096 minimal complete records are processed and record 4,097 remains for the next quantum. `F:conversation_status_inventory_count_budget` accepts exactly 4,096 aggregate directory entries, including irrelevant and migration-only entries, and rejects entry 4,097 before summary or migration work. | Covered by `P:conversation_migration_quantum` and `P:conversation_replay_quantum`. |
| CV-04 | Scan quantum bytes: 16 MiB | `F:conversation_scan_byte_budget`; exactly 16,777,216 stored bytes are processed and the next complete record remains for the next quantum. | `P:conversation_migration_quantum`, `P:conversation_replay_quantum` and `P:conversation_history_validation_quantum` each process exactly 16 MiB. |
| CV-05 | Status/projection page records: 100 | `F:conversation_page_count_budget_and_human_truncation_notice`; a 100-record page succeeds and record 101 remains behind the cursor. | `P:conversation_status_page` and `P:run_log_projection_page` use exactly 100 records. |
| CV-06 | Status/projection output: 1 MiB | `F:conversation_page_byte_budget`; a status page with 100 largest valid records stays within 1,048,576 bytes, while canonical projection output is exactly 1,048,576 bytes and the next record remains behind the cursor. | `P:conversation_status_page` uses exactly 100 largest valid status records within the ceiling; `P:run_log_projection_page` uses exactly 100 records and 1 MiB canonical output. |
| CV-07 | Status/projection memory: 16 MiB | Page boundary behavior is covered by CV-05/CV-06. | `P:conversation_status_page` and `P:run_log_projection_page` each require maximum peak growth <= 16 MiB. |
| CV-08 | Migration/replay buffer: 1 MiB | `F:conversation_io_buffer_budget`; instrumented operations never issue a read/write buffer above 1,048,576 bytes across multi-quantum input. | F-only: implementation allocation boundary; quantum RSS is gated separately. |
| CV-09 | Migration/replay/history-validation memory: 64 MiB | Quantum and buffer behavior is covered by CV-03/CV-04/CV-08. | `P:conversation_migration_quantum`, `P:conversation_replay_quantum` and `P:conversation_history_validation_quantum` each require maximum peak growth <= 64 MiB. |
| CV-10 | Eight synchronized Run Log appends: p95 10 ms | `F:run_log_append_durability`; eight 576-byte canonical records are individually appended and synchronized, then replayed. | `P:run_log_eight_sync_appends`, the same eight records per sample, p95 <= 10 ms. |
| CV-11 | Fixed status/projection pages: p95 100 ms | Page behavior is covered by CV-05/CV-06. | `P:conversation_status_page`, exactly 100 largest valid records within 1 MiB, and `P:run_log_projection_page`, exactly 100 records and 1 MiB, each have p95 <= 100 ms and are subject to CV-07. |
| CV-12 | 16 MiB migration quantum: p95 1.25 s; replay/history-validation quantum: p95 1 s | Quantum behavior is covered by CV-03/CV-04/CV-08. | `P:conversation_migration_quantum`, `P:conversation_replay_quantum` and `P:conversation_history_validation_quantum` each process exactly 16 MiB and are subject to CV-09. Migration p95 <= 1.25 s; replay and history-validation p95 <= 1 s. |
| CV-13 | History-validation scratch: 1 KiB per complete entry plus 16 MiB work reserve | `F:conversation_history_scratch_budget`; instrumentation proves every admitted peak remains at or below `entries * 1,024 + 16,777,216`, and insufficient space fails before effects. | F-only: storage-accounting safety boundary. |
| CV-14 | History validation: at most O(n log n) | `F:conversation_history_work_budget`; comparison and I/O-pass instrumentation remains within the selected closed formula for boundary and adversarial-order fixtures. | F-only: algorithmic implementation boundary; CV-12 gates elapsed quantum time. |
| CV-15 | Status summary and transaction: 4 KiB each | `F:conversation_status_rejects_oversized_and_unknown_summaries` rejects an oversized persisted summary; `F:conversation_status_reads_only_its_bounded_summary` proves status uses only the named summary and never retained history or Run Logs. | `P:conversation_status_page` reads exactly 100 bounded summaries; its time and RSS remain gated by CV-07/CV-11. |
| CV-16 | In-memory replay output: 64 MiB | `F:in_memory_replay_accepts_exact_output_limit_and_rejects_one_byte_over`; exactly 67,108,864 canonical bytes succeed and one byte beyond returns typed `ReplayOutputLimitExceeded { limit_bytes }`. | F-only: public API boundary; large CLI replay uses callback streaming. |
| CV-17 | Callback-streaming full-Run replay: exactly 352 MiB across 22 segments, p95 13 s | `F:streaming_replay_emits_large_segmented_jsonl_without_returning_it` proves validated record streaming, byte identity and empty returned output above CV-16. | `P:conversation_full_run_streaming_replay` validates and hashes exactly 369,098,752 bytes across 22 segments, p95 <= 13 s. |
| CV-18 | Callback-streaming full-Run replay: maximum peak RSS growth 256 MiB | Streaming behavior is covered by CV-17. | `P:conversation_full_run_streaming_replay` requires maximum peak growth <= 256 MiB while processing the same exact workload without retaining complete output. |

## Completion criterion

The approved matrix is finite: 72 selected-contract rows, one named functional proof for every row, 15 named optimized workloads including the RSS detection fixture, and no implicit “similar case” requirement. Evidence implementation completes every named functional proof and passes every applicable gate in the optimized Ubuntu artifact before M1.1 product behavior begins. A later example outside this closed set requires a new decision; it cannot silently expand the closeout gate.
