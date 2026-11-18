use std::path::PathBuf;

use meta_harness_core::{
    AgentEvent, ApprovalOption, BranchChanges, BranchPullRequest, ChangedFile, ChatItem, Context,
    GitStatus, Harness, ItemDetail, ItemId, ItemKind, ItemStatus, LineCount, Model, ModelChoice,
    ModelOption, ModelUsage, PendingRequest, Provider, PullRequestCheck, PullRequestDetail,
    Question, Request, RequestId, Snapshot, Statistics, ThreadEvent, ThreadSummary, ToolCalls,
    Turn, TurnFile, TurnId, TurnStatistics, TurnStatus, Usage,
};

use crate::api::events::{
    BridgeApproval, BridgeApprovalOption, BridgeChangeStatus, BridgeEvent, BridgeEventKind,
    BridgeFileChange, BridgeItem, BridgeItemKind, BridgeQuestion, BridgeRequest, BridgeUsage,
};
use crate::api::git::{BridgeBranchPullRequest, BridgeGitStatus, BridgePullRequestCheck};
use crate::api::session::{BridgeModelChoice, BridgeModelOption, BridgeThreadSummary};
use crate::api::thread::{
    BridgeContext, BridgeModelUsage, BridgeToolCalls, BridgeTurnStatistics, BridgeTurnStatus,
};
use crate::convert;

fn sequenced(sequence: u64, event: AgentEvent) -> BridgeEvent {
    BridgeEvent::from(ThreadEvent { sequence, event })
}

#[test]
fn a_diff_lists_each_file_with_its_status_and_line_counts() {
    let diff = "\
diff --git a/added.txt b/added.txt
new file mode 100644
--- /dev/null
+++ b/added.txt
@@ -0,0 +1,2 @@
+one
+two
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old.txt b/new.txt
similarity index 80%
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1,2 +1,2 @@
 keep
-was
+is
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
--- a
+++ b
";
    let files = convert::diff_files(diff);
    let listed: Vec<_> = files
        .iter()
        .map(|(file, _)| (file.path.as_str(), file.status, file.added, file.removed))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("added.txt", BridgeChangeStatus::Added, 2, 0),
            ("gone.txt", BridgeChangeStatus::Deleted, 0, 1),
            ("new.txt", BridgeChangeStatus::Renamed, 1, 1),
            ("lib.rs", BridgeChangeStatus::Modified, 1, 1),
        ]
    );
    assert!(files[2].1.starts_with("diff --git a/old.txt b/new.txt\n"));
    assert!(files[2].1.ends_with("+is\n"));
}

#[test]
fn a_patch_file_that_quotes_a_git_header_stays_one_file() {
    let diff = "\
diff --git a/fix.patch b/fix.patch
--- a/fix.patch
+++ b/fix.patch
@@ -0,0 +1 @@
+diff --git a/src/x.rs b/src/x.rs
";
    let files = convert::diff_files(diff);
    let listed: Vec<_> = files
        .iter()
        .map(|(file, _)| (file.path.as_str(), file.added))
        .collect();
    assert_eq!(listed, vec![("fix.patch", 1)]);
}

#[test]
fn a_patch_shows_hidden_characters_written_out_and_names_paths_with_spaces() {
    let diff = "\
diff --git a/a b/c.txt b/a b/c.txt
--- a/a b/c.txt
+++ b/a b/c.txt
@@ -1 +1 @@
-allow\r
+deny\u{202e}\r
";
    let files = convert::diff_files(diff);
    assert_eq!(files[0].0.path, "a b/c.txt");
    assert!(
        files[0].1.ends_with("+deny<U+202E>\r\n"),
        "{:?}",
        files[0].1
    );
    let copied = convert::diff_files(
        "\
diff --git a/src b/x b/dst
similarity index 100%
copy from src
copy to x b/dst
",
    );
    assert_eq!(
        copied[0].0.path, "x b/dst",
        "a copied path keeps all of its name"
    );
}

#[test]
fn git_quoted_paths_are_decoded_in_headers_and_renames() {
    let diff = r#"diff --git "a/caf\303\251.txt" "b/caf\303\251.txt"
--- "a/caf\303\251.txt"
+++ "b/caf\303\251.txt"
@@ -1 +1 @@
-one
+two
diff --git "a/tab\tname" "b/tab\tname"
new file mode 100644
--- /dev/null
+++ "b/tab\tname"
@@ -0,0 +1 @@
+x
diff --git "a/q\"uote" "b/q\"uote"
new file mode 100644
--- /dev/null
+++ "b/q\"uote"
@@ -0,0 +1 @@
+x
diff --git "a/back\\slash" "b/back\\slash"
new file mode 100644
--- /dev/null
+++ "b/back\\slash"
@@ -0,0 +1 @@
+x
diff --git a/old.txt "b/n\303\253w.txt"
similarity index 80%
rename from old.txt
rename to "n\303\253w.txt"
--- a/old.txt
+++ "b/n\303\253w.txt"
@@ -1 +1 @@
-was
+is
diff --git "a/x\342\200\256y.txt" "b/x\342\200\256y.txt"
new file mode 100644
--- /dev/null
+++ "b/x\342\200\256y.txt"
@@ -0,0 +1 @@
+x
"#;
    let files = convert::diff_files(diff);
    let listed: Vec<_> = files
        .iter()
        .map(|(file, _)| (file.path.as_str(), file.status, file.added, file.removed))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("café.txt", BridgeChangeStatus::Modified, 1, 1),
            ("tab\tname", BridgeChangeStatus::Added, 1, 0),
            ("q\"uote", BridgeChangeStatus::Added, 1, 0),
            ("back\\slash", BridgeChangeStatus::Added, 1, 0),
            ("nëw.txt", BridgeChangeStatus::Renamed, 1, 1),
            ("x\u{202e}y.txt", BridgeChangeStatus::Added, 1, 0),
        ]
    );
    assert_eq!(files[5].0.shown, "x<U+202E>y.txt");
}

#[test]
fn turn_and_item_events_cross_with_their_turn_item_and_text() {
    let turn = TurnId(2);
    let started = sequenced(
        1,
        AgentEvent::ItemStarted {
            turn,
            item: ItemId(7),
            kind: ItemKind::CommandExecution,
            detail: ItemDetail::Tool {
                name: "Bash".to_owned(),
                target: "ls\u{200b}".to_owned(),
            },
        },
    );
    assert_eq!(
        started,
        BridgeEvent {
            kind: BridgeEventKind::ItemStarted,
            turn: Some(2),
            item: Some(BridgeItem {
                id: 7,
                kind: BridgeItemKind::CommandExecution,
                tool: Some("Bash".to_owned()),
                target: Some("ls<U+200B>".to_owned()),
                ended: false,
                failed: false,
                duration_ms: None,
            }),
            ..crate::convert::event(BridgeEventKind::ItemStarted, 1)
        }
    );

    let delta = sequenced(
        2,
        AgentEvent::ContentDelta {
            item: ItemId(7),
            delta: "hi".to_owned(),
        },
    );
    assert_eq!(delta.kind, BridgeEventKind::ContentDelta);
    assert_eq!(delta.text.as_deref(), Some("hi"));
    assert_eq!(
        delta.item.map(|item| item.kind),
        Some(BridgeItemKind::AssistantMessage)
    );

    let completed = sequenced(
        3,
        AgentEvent::ItemCompleted {
            turn,
            item: ItemId(7),
            kind: ItemKind::AssistantMessage,
            text: "Done".to_owned(),
            status: ItemStatus::Failed,
            duration_ms: Some(5),
        },
    );
    assert_eq!(completed.kind, BridgeEventKind::ItemCompleted);
    assert_eq!(completed.turn, Some(2));
    assert_eq!(completed.text.as_deref(), Some("Done"));
    assert_eq!(
        completed.item,
        Some(BridgeItem {
            id: 7,
            kind: BridgeItemKind::AssistantMessage,
            tool: None,
            target: None,
            ended: true,
            failed: true,
            duration_ms: Some(5),
        })
    );

    let kinds: Vec<_> = [
        AgentEvent::TurnStarted { turn },
        AgentEvent::TurnCompleted {
            turn,
            checkpoint: Some("c".to_owned()),
        },
        AgentEvent::TurnAborted {
            turn,
            checkpoint: None,
        },
        AgentEvent::TurnsChanged,
        AgentEvent::RolledBack,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, event)| {
        let event = sequenced(index as u64 + 4, event);
        (event.kind, event.turn, event.sequence)
    })
    .collect();
    assert_eq!(
        kinds,
        vec![
            (BridgeEventKind::TurnStarted, Some(2), 4),
            (BridgeEventKind::TurnCompleted, Some(2), 5),
            (BridgeEventKind::TurnAborted, Some(2), 6),
            (BridgeEventKind::TurnsChanged, None, 7),
            (BridgeEventKind::RolledBack, None, 8),
        ]
    );
}

#[test]
fn a_finished_model_request_crosses_with_its_tokens() {
    let event = sequenced(
        9,
        AgentEvent::ModelRequestCompleted {
            turn: TurnId(2),
            usage: Usage {
                input: 3,
                output: 4,
                cache_read: 1,
                cache_write: 2,
                reasoning: 1,
            },
        },
    );
    assert_eq!(event.kind, BridgeEventKind::ModelRequestCompleted);
    assert_eq!(event.turn, Some(2));
    assert_eq!(
        event.usage,
        Some(BridgeUsage {
            input: 3,
            output: 4,
            cache_read: 1,
            cache_write: 2,
            reasoning: 1,
        })
    );
}

#[test]
fn requests_cross_as_approvals_or_questions_and_their_resolution_by_id() {
    let turn = TurnId(2);
    let approval = sequenced(
        1,
        AgentEvent::RequestOpened {
            request: PendingRequest {
                id: RequestId("r1".to_owned()),
                turn,
                request: Request::Approval {
                    kind: "command".to_owned(),
                    prompt: "ls".to_owned(),
                    reason: Some("list".to_owned()),
                    options: vec![
                        ApprovalOption {
                            decision: "accept".to_owned(),
                            label: "Allow".to_owned(),
                            warning: None,
                        },
                        ApprovalOption {
                            decision: "decline".to_owned(),
                            label: "Deny".to_owned(),
                            warning: Some("careful".to_owned()),
                        },
                    ],
                },
            },
        },
    );
    assert_eq!(approval.kind, BridgeEventKind::RequestOpened);
    assert_eq!(approval.turn, Some(2));
    assert_eq!(
        approval.request,
        Some(BridgeRequest {
            id: "r1".to_owned(),
            turn: 2,
            approval: Some(BridgeApproval {
                kind: "command".to_owned(),
                prompt: "ls".to_owned(),
                reason: Some("list".to_owned()),
                options: vec![
                    BridgeApprovalOption {
                        decision: "accept".to_owned(),
                        label: "Allow".to_owned(),
                        warning: None,
                    },
                    BridgeApprovalOption {
                        decision: "decline".to_owned(),
                        label: "Deny".to_owned(),
                        warning: Some("careful".to_owned()),
                    },
                ],
            }),
            questions: Vec::new(),
        })
    );

    let questions = sequenced(
        2,
        AgentEvent::RequestOpened {
            request: PendingRequest {
                id: RequestId("q1".to_owned()),
                turn,
                request: Request::Questions(vec![Question {
                    id: "which".to_owned(),
                    header: "Pick".to_owned(),
                    question: "Which?".to_owned(),
                    options: vec![("a".to_owned(), "The first".to_owned())],
                    multi_select: true,
                    custom: false,
                }]),
            },
        },
    );
    let request = questions.request.expect("the request");
    assert_eq!(request.approval, None);
    assert_eq!(
        request.questions,
        vec![BridgeQuestion {
            id: "which".to_owned(),
            header: "Pick".to_owned(),
            question: "Which?".to_owned(),
            options: vec!["a".to_owned()],
            descriptions: vec!["The first".to_owned()],
            multi_select: true,
            custom: false,
        }]
    );

    let resolved = sequenced(
        3,
        AgentEvent::RequestResolved {
            request: RequestId("r1".to_owned()),
        },
    );
    assert_eq!(resolved.kind, BridgeEventKind::RequestResolved);
    assert_eq!(resolved.request_id.as_deref(), Some("r1"));
    assert_eq!(resolved.turn, None);
}

#[test]
fn approvals_and_questions_write_out_the_hidden_characters_the_chat_shows() {
    let request = convert::request(PendingRequest {
        id: RequestId("r1".to_owned()),
        turn: TurnId(2),
        request: Request::Approval {
            kind: "command".to_owned(),
            prompt: "rm\u{202E}x\nls".to_owned(),
            reason: Some("why\u{2066}".to_owned()),
            options: vec![ApprovalOption {
                decision: "accept".to_owned(),
                label: "Allow\u{200B}".to_owned(),
                warning: Some("careful\u{202E}\nnow".to_owned()),
            }],
        },
    });
    let approval = request.approval.expect("the approval");
    assert_eq!(
        approval.prompt, "rm<U+202E>x\nls",
        "a prompt keeps its line breaks"
    );
    assert_eq!(approval.reason.as_deref(), Some("why<U+2066>"));
    assert_eq!(approval.options[0].label, "Allow<U+200B>");
    assert_eq!(
        approval.options[0].warning.as_deref(),
        Some("careful<U+202E>\nnow")
    );

    let request = convert::request(PendingRequest {
        id: RequestId("q1".to_owned()),
        turn: TurnId(2),
        request: Request::Questions(vec![Question {
            id: "which".to_owned(),
            header: "Pick\u{200B}\nnow".to_owned(),
            question: "Which?\u{202E}".to_owned(),
            options: vec![("a".to_owned(), "The\u{202E} first".to_owned())],
            multi_select: false,
            custom: true,
        }]),
    });
    let question = &request.questions[0];
    assert_eq!(
        question.header, "Pick<U+200B><U+000A>now",
        "a header stays on one line"
    );
    assert_eq!(question.question, "Which?<U+202E>");
    assert_eq!(question.descriptions, ["The<U+202E> first"]);
}

#[test]
fn a_snapshot_carries_its_turns_each_with_its_status_and_its_transcript() {
    let turn = |number, status| Turn {
        number,
        status,
        prompt: format!("prompt {number}"),
        started_ms: Some(10),
        ended_ms: None,
        files: vec![TurnFile {
            path: "a.txt".to_owned(),
            kind: "modified".to_owned(),
            added: 1,
            removed: 0,
        }],
        added: 1,
        removed: 0,
        checkpoint: number == 1,
    };
    let snapshot = convert::snapshot(Snapshot {
        sequence: 9,
        turns: vec![
            turn(1, TurnStatus::Queued),
            turn(2, TurnStatus::Running),
            turn(3, TurnStatus::Completed),
            turn(4, TurnStatus::Aborted),
        ],
        transcript: vec![ChatItem {
            id: ItemId(3),
            turn: TurnId(1),
            kind: ItemKind::UserMessage,
            detail: ItemDetail::None,
            text: "prompt 1".to_owned(),
            status: Some(ItemStatus::Completed),
            duration_ms: None,
        }],
        requests: Vec::new(),
    });
    assert_eq!(snapshot.sequence, 9);
    assert_eq!(
        snapshot.turns.iter().map(|t| t.status).collect::<Vec<_>>(),
        vec![
            BridgeTurnStatus::Queued,
            BridgeTurnStatus::Running,
            BridgeTurnStatus::Completed,
            BridgeTurnStatus::Aborted,
        ]
    );
    assert_eq!(snapshot.turns[0].files, 1);
    assert!(snapshot.turns[0].checkpoint && !snapshot.turns[1].checkpoint);
    assert_eq!(snapshot.transcript[0].turn, 1);
    assert_eq!(snapshot.transcript[0].text, "prompt 1");
    assert_eq!(
        snapshot.transcript[0].item.kind,
        BridgeItemKind::UserMessage
    );
}

#[test]
fn statistics_cross_with_each_model_tool_and_turn() {
    let usage = Usage {
        input: 5,
        output: 6,
        cache_read: 7,
        cache_write: 8,
        reasoning: 9,
    };
    let statistics = convert::statistics(Statistics {
        models: vec![ModelUsage {
            model: "gpt\u{200b}".to_owned(),
            usage,
            responses: 2,
        }],
        tools: vec![ToolCalls {
            name: "Bash".to_owned(),
            succeeded: 3,
            failed: 1,
            cancelled: 0,
            open: 1,
            duration_ms: Some(40),
        }],
        turns: vec![TurnStatistics {
            entry: "4".to_owned(),
            prompt: "First line".to_owned(),
            started_ms: Some(100),
            ended_ms: Some(200),
            models: vec![ModelUsage {
                model: "m".to_owned(),
                usage,
                responses: 1,
            }],
            tool_ms: Some(30),
            failed: 1,
        }],
        started_ms: Some(100),
        ended_ms: Some(200),
    });
    assert_eq!(statistics.models[0].model, "gpt<U+200B>");
    assert_eq!(statistics.models[0].responses, 2);
    assert_eq!(statistics.models[0].usage.reasoning, 9);
    assert_eq!(
        statistics.tools,
        vec![BridgeToolCalls {
            name: "Bash".to_owned(),
            succeeded: 3,
            failed: 1,
            cancelled: 0,
            open: 1,
            duration_ms: Some(40),
        }]
    );
    assert_eq!(
        statistics.turns,
        vec![BridgeTurnStatistics {
            number: 4,
            prompt: "First line".to_owned(),
            started_ms: Some(100),
            ended_ms: Some(200),
            models: vec![BridgeModelUsage {
                model: "m".to_owned(),
                usage: BridgeUsage {
                    input: 5,
                    output: 6,
                    cache_read: 7,
                    cache_write: 8,
                    reasoning: 9,
                },
                responses: 1,
            }],
            tool_ms: Some(30),
            failed: 1,
        }]
    );
    assert_eq!(
        (statistics.started_ms, statistics.ended_ms),
        (Some(100), Some(200))
    );
}

#[test]
fn the_context_window_crosses_with_what_is_known_of_it() {
    assert_eq!(
        convert::context(Context {
            tokens: Some(10),
            window: 100,
            compacts_at: Some(80),
        }),
        BridgeContext {
            tokens: Some(10),
            window: 100,
            compacts_at: Some(80),
        }
    );
}

#[test]
fn providers_and_model_choices_cross_with_their_options_both_ways() {
    let provider = convert::provider(Provider {
        instance: "scripted".to_owned(),
        name: "Scripted\u{200b}".to_owned(),
        available: true,
        models: vec![Model {
            id: "m1".to_owned(),
            name: "Model 1".to_owned(),
            options: vec![ModelOption {
                id: "effort".to_owned(),
                label: "Effort".to_owned(),
                choices: vec![
                    ("low".to_owned(), "Low".to_owned()),
                    ("high".to_owned(), "High".to_owned()),
                ],
                default: Some("low".to_owned()),
            }],
        }],
    });
    assert_eq!(provider.name, "Scripted<U+200B>");
    assert!(provider.available);
    assert_eq!(
        provider.models[0].options[0],
        BridgeModelOption {
            id: "effort".to_owned(),
            label: "Effort".to_owned(),
            choices: vec!["low".to_owned(), "high".to_owned()],
            labels: vec!["Low".to_owned(), "High".to_owned()],
            default: Some("low".to_owned()),
        }
    );

    let choice = BridgeModelChoice {
        instance: "scripted".to_owned(),
        model: "m1".to_owned(),
        option_ids: vec!["effort".to_owned()],
        values: vec!["high".to_owned()],
    };
    let core: ModelChoice = convert::to_model_choice(choice.clone()).expect("matching pairs");
    assert_eq!(core.options, vec![("effort".to_owned(), "high".to_owned())]);
    assert_eq!(convert::model_choice(&core), choice);
}

#[test]
fn a_model_choice_whose_option_ids_and_values_differ_in_number_is_refused() {
    let choice = |option_ids: &[&str], values: &[&str]| BridgeModelChoice {
        instance: "scripted".to_owned(),
        model: "m1".to_owned(),
        option_ids: option_ids.iter().map(|id| (*id).to_owned()).collect(),
        values: values.iter().map(|value| (*value).to_owned()).collect(),
    };
    assert!(convert::to_model_choice(choice(&["effort", "speed"], &["high"])).is_err());
    assert!(convert::to_model_choice(choice(&["effort"], &["high", "fast"])).is_err());
}

#[test]
fn a_thread_summary_crosses_with_its_title_and_worktree() {
    let summary = convert::thread_summary(ThreadSummary {
        id: "t1".to_owned(),
        title: "Fix\u{200b} bug".to_owned(),
        updated_ms: Some(5),
        branch: Some("main".to_owned()),
        worktree: Some(PathBuf::from("/wt")),
        running: true,
        archived: false,
        harness: Some(Harness::ClaudeCode),
        harness_known: true,
        moving: None,
    });
    assert_eq!(
        summary,
        BridgeThreadSummary {
            id: "t1".to_owned(),
            title: "Fix<U+200B> bug".to_owned(),
            updated_ms: Some(5),
            branch: Some("main".to_owned()),
            worktree: Some("/wt".to_owned()),
            running: true,
            archived: false,
            movable: true,
            harness_known: true,
            moving: None,
        }
    );
    let other = convert::thread_summary(ThreadSummary {
        id: "t2".to_owned(),
        title: "Other".to_owned(),
        updated_ms: None,
        branch: None,
        worktree: None,
        running: false,
        archived: false,
        harness: None,
        harness_known: true,
        moving: None,
    });
    assert!(!other.movable, "a thread of another harness cannot move");
    assert!(other.harness_known, "its service reported the harness");

    let older = convert::thread_summary(ThreadSummary {
        id: "t3".to_owned(),
        title: "Older".to_owned(),
        updated_ms: None,
        branch: None,
        worktree: None,
        running: false,
        archived: false,
        harness: None,
        harness_known: false,
        moving: None,
    });
    assert!(!older.movable);
    assert!(!older.harness_known, "an older service reported no harness");
}

#[test]
fn a_git_status_crosses_with_its_changes_counts_and_pull_request() {
    let status = convert::git_status(GitStatus {
        repository: true,
        branch: Some("feat".to_owned()),
        remote: true,
        provider: Some("GitHub".to_owned()),
        changes: vec![ChangedFile {
            path: "a\u{200b}.txt".to_owned(),
            added: 2,
            removed: 1,
        }],
        upstream: true,
        ahead: 1,
        ahead_of_default: Some(3),
        working_tree_lines: LineCount {
            added: 5,
            removed: 3,
        },
        branch_changes: Some(BranchChanges {
            base: Some("origin/main\u{202e}".to_owned()),
            lines: LineCount {
                added: 9,
                removed: 4,
            },
        }),
        pull_request: Some(BranchPullRequest {
            number: 42,
            title: "Add it".to_owned(),
            url: "https://example.invalid/42".to_owned(),
            state: "open".to_owned(),
            draft: true,
            base: "main".to_owned(),
        }),
        ..GitStatus::default()
    });
    assert_eq!(
        status,
        BridgeGitStatus {
            repository: true,
            branch: Some("feat".to_owned()),
            default_branch: false,
            remote: true,
            provider: Some("GitHub".to_owned()),
            changes: vec![BridgeFileChange {
                path: "a\u{200b}.txt".to_owned(),
                shown: "a<U+200B>.txt".to_owned(),
                status: BridgeChangeStatus::Modified,
                added: 2,
                removed: 1,
            }],
            working_added: 5,
            working_removed: 3,
            branch_base: Some("origin/main<U+202E>".to_owned()),
            branch_added: 9,
            branch_removed: 4,
            upstream: true,
            ahead: 1,
            behind: 0,
            ahead_of_default: Some(3),
            pull_request: Some(BridgeBranchPullRequest {
                number: 42,
                title: "Add it".to_owned(),
                url: "https://example.invalid/42".to_owned(),
                state: "open".to_owned(),
                draft: true,
                base: "main".to_owned(),
            }),
        }
    );
}

#[test]
fn a_pull_request_crosses_with_its_checks_action_and_prompts_with_hidden_characters_written_out() {
    let detail = convert::pull_request_detail(PullRequestDetail {
        number: 42,
        title: "Add\u{200b}it".to_owned(),
        url: "https://example.invalid/42".to_owned(),
        state: "open".to_owned(),
        draft: false,
        base: "main".to_owned(),
        head: "feature".to_owned(),
        conflicting: false,
        files: 3,
        lines: LineCount {
            added: 12,
            removed: 4,
        },
        checks: vec![PullRequestCheck {
            name: "build\u{202e}".to_owned(),
            status: "failure".to_owned(),
            description: Some("exit\u{200b}1".to_owned()),
            url: None,
        }],
        checks_state: "failing".to_owned(),
        checks_summary: "1 of 1 failing".to_owned(),
        action: Some("fix".to_owned()),
        merge_method: "merge".to_owned(),
    });
    assert_eq!(
        (detail.number, detail.files, detail.added, detail.removed),
        (42, 3, 12, 4)
    );
    assert_eq!(detail.title, "Add<U+200B>it");
    assert_eq!(
        detail.checks,
        vec![BridgePullRequestCheck {
            name: "build<U+202E>".to_owned(),
            status: "failure".to_owned(),
            description: Some("exit<U+200B>1".to_owned()),
            url: None,
        }]
    );
    assert_eq!(detail.action.as_deref(), Some("fix"));
    assert!(detail.fix_prompt.contains("titled `Add<U+200B>it`"));
    assert!(
        detail
            .fix_prompt
            .ends_with("\nFailing checks:\n> build<U+202E>: exit<U+200B>1")
    );
    assert!(detail.resolve_prompt.starts_with(
        "PR #42 (https://example.invalid/42) conflicts with its base branch `main`. Its branch `feature` is"
    ));
}

#[test]
fn a_question_s_options_show_hidden_characters_and_answer_with_the_agent_s_labels() {
    let questions = vec![Question {
        id: "pick".to_owned(),
        header: "Pick".to_owned(),
        question: "Which?".to_owned(),
        options: vec![
            ("keep\u{202e}".to_owned(), String::new()),
            ("drop".to_owned(), String::new()),
        ],
        multi_select: true,
        custom: true,
    }];
    let shown = convert::request(PendingRequest {
        id: RequestId("r".to_owned()),
        turn: TurnId(1),
        request: Request::Questions(questions.clone()),
    });
    assert_eq!(shown.questions[0].options, vec!["keep<U+202E>", "drop"]);
    assert_eq!(
        convert::answer_values(
            &questions,
            "pick",
            vec!["keep<U+202E>".to_owned(), "drop".to_owned()],
            false
        ),
        vec!["keep\u{202e}", "drop"]
    );
    assert_eq!(
        convert::answer_values(&questions, "pick", vec!["keep<U+202E>".to_owned()], true),
        vec!["keep<U+202E>"]
    );
}
