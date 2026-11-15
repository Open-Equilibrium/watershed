//! Meta-Harness's types as the bridge hands them to Dart. Names, targets and the text the Chat
//! shows cross with hiding characters written out.

use meta_harness_core::{
    AgentEvent, ChatItem, Context, GitStatus, ItemDetail, ItemKind, ItemStatus, ModelChoice,
    PendingRequest, Provider, Question, Request, Snapshot, Statistics, ThreadEvent, ThreadSummary,
    Turn, TurnStatus, Usage, visible, visible_lines, visible_name,
};

use crate::api::events::{
    BridgeApproval, BridgeApprovalOption, BridgeChangeStatus, BridgeEvent, BridgeEventKind,
    BridgeFileChange, BridgeItem, BridgeItemKind, BridgeQuestion, BridgeRequest, BridgeUsage,
};
use crate::api::git::{BridgeBranchPullRequest, BridgeGitStatus};
use crate::api::session::{
    BridgeModel, BridgeModelChoice, BridgeModelOption, BridgeProvider, BridgeThreadSummary,
};
use crate::api::thread::{
    BridgeContext, BridgeModelUsage, BridgeSnapshot, BridgeStatistics, BridgeToolCalls,
    BridgeTranscriptItem, BridgeTurn, BridgeTurnStatistics, BridgeTurnStatus,
};

fn int(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

/// An event that carries nothing but its kind.
pub(crate) fn event(kind: BridgeEventKind, sequence: i64) -> BridgeEvent {
    BridgeEvent {
        kind,
        sequence,
        turn: None,
        item: None,
        text: None,
        usage: None,
        request: None,
        request_id: None,
    }
}

fn item_kind(kind: ItemKind) -> BridgeItemKind {
    match kind {
        ItemKind::UserMessage => BridgeItemKind::UserMessage,
        ItemKind::AssistantMessage => BridgeItemKind::AssistantMessage,
        ItemKind::Reasoning => BridgeItemKind::Reasoning,
        ItemKind::CommandExecution => BridgeItemKind::CommandExecution,
        ItemKind::FileChange => BridgeItemKind::FileChange,
        ItemKind::DynamicToolCall => BridgeItemKind::DynamicToolCall,
        ItemKind::Error => BridgeItemKind::Error,
    }
}

fn item(
    id: meta_harness_core::ItemId,
    kind: ItemKind,
    detail: &ItemDetail,
    status: Option<ItemStatus>,
    duration_ms: Option<u64>,
) -> BridgeItem {
    let (tool, target) = match detail {
        ItemDetail::None => (None, None),
        ItemDetail::Tool { name, target } => (Some(visible_name(name)), Some(visible_name(target))),
    };
    BridgeItem {
        id: int(id.0),
        kind: item_kind(kind),
        tool,
        target,
        ended: status.is_some(),
        failed: status == Some(ItemStatus::Failed),
        duration_ms: duration_ms.map(int),
    }
}

fn usage(usage: Usage) -> BridgeUsage {
    BridgeUsage {
        input: int(usage.input),
        output: int(usage.output),
        cache_read: int(usage.cache_read),
        cache_write: int(usage.cache_write),
        reasoning: int(usage.reasoning),
    }
}

/// The answer to question `id` as the agent sent its options: each option label the card showed
/// written out becomes the agent's label again; typed text stays as typed.
pub(crate) fn answer_values(questions: &[Question], id: &str, values: Vec<String>) -> Vec<String> {
    let options = questions
        .iter()
        .find(|q| q.id == id)
        .map_or(&[][..], |q| &q.options[..]);
    values
        .into_iter()
        .map(|value| {
            options
                .iter()
                .find(|(label, _)| visible_name(label) == value)
                .map_or(value, |(label, _)| label.clone())
        })
        .collect()
}

pub(crate) fn request(pending: PendingRequest) -> BridgeRequest {
    let (approval, questions) = match pending.request {
        Request::Approval {
            kind,
            prompt,
            reason,
            options,
        } => (
            Some(BridgeApproval {
                kind,
                prompt: visible(&prompt),
                reason: reason.map(|reason| visible(&reason)),
                options: options
                    .into_iter()
                    .map(|o| BridgeApprovalOption {
                        decision: o.decision,
                        label: visible_name(&o.label),
                        warning: o.warning.map(|warning| visible(&warning)),
                    })
                    .collect(),
            }),
            Vec::new(),
        ),
        Request::Questions(questions) => (
            None,
            questions
                .into_iter()
                .map(|q| BridgeQuestion {
                    id: q.id,
                    header: visible_name(&q.header),
                    question: visible(&q.question),
                    descriptions: q.options.iter().map(|(_, d)| visible(d)).collect(),
                    // Written out; `answer_values` gives the agent its own labels back.
                    options: q
                        .options
                        .iter()
                        .map(|(label, _)| visible_name(label))
                        .collect(),
                    multi_select: q.multi_select,
                    custom: q.custom,
                })
                .collect(),
        ),
    };
    BridgeRequest {
        id: pending.id.0,
        turn: int(pending.turn.0),
        approval,
        questions,
    }
}

impl From<ThreadEvent> for BridgeEvent {
    fn from(event: ThreadEvent) -> Self {
        let sequence = int(event.sequence);
        let with = |kind, turn: Option<u64>| BridgeEvent {
            turn: turn.map(int),
            ..crate::convert::event(kind, sequence)
        };
        match event.event {
            AgentEvent::TurnStarted { turn } => with(BridgeEventKind::TurnStarted, Some(turn.0)),
            AgentEvent::TurnCompleted { turn, .. } => {
                with(BridgeEventKind::TurnCompleted, Some(turn.0))
            }
            AgentEvent::TurnAborted { turn, .. } => {
                with(BridgeEventKind::TurnAborted, Some(turn.0))
            }
            AgentEvent::TurnsChanged => with(BridgeEventKind::TurnsChanged, None),
            AgentEvent::RolledBack => with(BridgeEventKind::RolledBack, None),
            AgentEvent::RequestOpened { request: pending } => BridgeEvent {
                turn: Some(int(pending.turn.0)),
                request: Some(request(pending)),
                ..with(BridgeEventKind::RequestOpened, None)
            },
            AgentEvent::RequestResolved { request } => BridgeEvent {
                request_id: Some(request.0),
                ..with(BridgeEventKind::RequestResolved, None)
            },
            AgentEvent::ItemStarted {
                turn,
                item: id,
                kind,
                detail,
            } => BridgeEvent {
                item: Some(item(id, kind, &detail, None, None)),
                ..with(BridgeEventKind::ItemStarted, Some(turn.0))
            },
            AgentEvent::ContentDelta { item: id, delta } => BridgeEvent {
                item: Some(item(
                    id,
                    ItemKind::AssistantMessage,
                    &ItemDetail::None,
                    None,
                    None,
                )),
                text: Some(delta),
                ..with(BridgeEventKind::ContentDelta, None)
            },
            AgentEvent::ItemCompleted {
                turn,
                item: id,
                kind,
                text,
                status,
                duration_ms,
            } => BridgeEvent {
                item: Some(item(id, kind, &ItemDetail::None, Some(status), duration_ms)),
                text: Some(text),
                ..with(BridgeEventKind::ItemCompleted, Some(turn.0))
            },
            AgentEvent::ModelRequestCompleted {
                turn, usage: used, ..
            } => BridgeEvent {
                usage: Some(usage(used)),
                ..with(BridgeEventKind::ModelRequestCompleted, Some(turn.0))
            },
        }
    }
}

pub(crate) fn snapshot(snapshot: Snapshot) -> BridgeSnapshot {
    BridgeSnapshot {
        sequence: int(snapshot.sequence),
        turns: snapshot.turns.into_iter().map(turn).collect(),
        transcript: snapshot
            .transcript
            .into_iter()
            .map(transcript_item)
            .collect(),
        requests: snapshot.requests.into_iter().map(request).collect(),
    }
}

fn transcript_item(chat: ChatItem) -> BridgeTranscriptItem {
    BridgeTranscriptItem {
        item: item(
            chat.id,
            chat.kind,
            &chat.detail,
            chat.status,
            chat.duration_ms,
        ),
        turn: int(chat.turn.0),
        text: chat.text,
    }
}

pub(crate) fn turn(turn: Turn) -> BridgeTurn {
    BridgeTurn {
        number: int(turn.number),
        status: match turn.status {
            TurnStatus::Queued => BridgeTurnStatus::Queued,
            TurnStatus::Running => BridgeTurnStatus::Running,
            TurnStatus::Completed => BridgeTurnStatus::Completed,
            TurnStatus::Aborted => BridgeTurnStatus::Aborted,
        },
        prompt: turn.prompt,
        started_ms: turn.started_ms.map(int),
        ended_ms: turn.ended_ms.map(int),
        files: int(turn.files.len()),
        added: int(turn.added),
        removed: int(turn.removed),
        checkpoint: turn.checkpoint,
    }
}

pub(crate) fn statistics(statistics: Statistics) -> BridgeStatistics {
    let model = |m: meta_harness_core::ModelUsage| BridgeModelUsage {
        model: visible_name(&m.model),
        usage: usage(m.usage),
        responses: int(m.responses),
    };
    BridgeStatistics {
        models: statistics.models.into_iter().map(model).collect(),
        tools: statistics
            .tools
            .into_iter()
            .map(|t| BridgeToolCalls {
                name: visible_name(&t.name),
                succeeded: int(t.succeeded),
                failed: int(t.failed),
                cancelled: int(t.cancelled),
                open: int(t.open),
                duration_ms: t.duration_ms.map(int),
            })
            .collect(),
        turns: statistics
            .turns
            .into_iter()
            .map(|t| BridgeTurnStatistics {
                number: t.entry.parse().unwrap_or_default(),
                prompt: t.prompt,
                started_ms: t.started_ms.map(int),
                ended_ms: t.ended_ms.map(int),
                models: t.models.into_iter().map(model).collect(),
                tool_ms: t.tool_ms.map(int),
                failed: int(t.failed),
            })
            .collect(),
        started_ms: statistics.started_ms.map(int),
        ended_ms: statistics.ended_ms.map(int),
    }
}

pub(crate) fn context(context: Context) -> BridgeContext {
    BridgeContext {
        tokens: context.tokens.map(int),
        window: int(context.window),
        compacts_at: context.compacts_at.map(int),
    }
}

pub(crate) fn provider(provider: Provider) -> BridgeProvider {
    BridgeProvider {
        instance: provider.instance,
        name: visible_name(&provider.name),
        available: provider.available,
        models: provider
            .models
            .into_iter()
            .map(|m| BridgeModel {
                id: m.id,
                name: visible_name(&m.name),
                options: m
                    .options
                    .into_iter()
                    .map(|o| BridgeModelOption {
                        id: o.id,
                        label: o.label,
                        labels: o.choices.iter().map(|(_, l)| l.clone()).collect(),
                        choices: o.choices.into_iter().map(|(id, _)| id).collect(),
                        default: o.default,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(crate) fn model_choice(choice: &ModelChoice) -> BridgeModelChoice {
    BridgeModelChoice {
        instance: choice.instance.clone(),
        model: choice.model.clone(),
        option_ids: choice.options.iter().map(|(id, _)| id.clone()).collect(),
        values: choice.options.iter().map(|(_, v)| v.clone()).collect(),
    }
}

pub(crate) fn to_model_choice(choice: BridgeModelChoice) -> ModelChoice {
    ModelChoice {
        instance: choice.instance,
        model: choice.model,
        options: choice.option_ids.into_iter().zip(choice.values).collect(),
    }
}

pub(crate) fn thread_summary(thread: ThreadSummary) -> BridgeThreadSummary {
    BridgeThreadSummary {
        id: thread.id,
        title: visible_name(&thread.title),
        updated_ms: thread.updated_ms.map(int),
        branch: thread.branch,
        worktree: thread.worktree.map(|p| p.to_string_lossy().into_owned()),
        running: thread.running,
        archived: thread.archived,
    }
}

pub(crate) fn git_status(status: GitStatus) -> BridgeGitStatus {
    BridgeGitStatus {
        repository: status.repository,
        branch: status.branch,
        default_branch: status.default_branch,
        remote: status.remote,
        provider: status.provider,
        changes: status
            .changes
            .into_iter()
            .map(|f| BridgeFileChange {
                shown: visible_name(&f.path),
                path: f.path,
                status: BridgeChangeStatus::Modified,
                added: int(f.added),
                removed: int(f.removed),
            })
            .collect(),
        upstream: status.upstream,
        ahead: int(status.ahead),
        behind: int(status.behind),
        ahead_of_default: status.ahead_of_default.map(int),
        pull_request: status.pull_request.map(|p| BridgeBranchPullRequest {
            number: int(p.number),
            title: visible_name(&p.title),
            url: p.url,
            state: p.state,
            draft: p.draft,
            base: visible_name(&p.base),
        }),
    }
}

/// Splits a unified diff into its files, each with its status, line counts and own patch. A file
/// starts only at a line that begins with a header, so patch text quoting one stays in its file.
pub(crate) fn diff_files(diff: &str) -> Vec<(BridgeFileChange, String)> {
    let starts: Vec<usize> = diff
        .match_indices("diff --git ")
        .map(|(at, _)| at)
        .filter(|&at| at == 0 || diff.as_bytes()[at - 1] == b'\n')
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = starts.get(index + 1).copied().unwrap_or(diff.len());
            let patch = &diff[start..end];
            let section = &patch["diff --git ".len()..];
            let header = section.lines().next().unwrap_or_default();
            let path = path_of(header, section);
            let status = if section.contains("\nnew file mode") {
                BridgeChangeStatus::Added
            } else if section.contains("\ndeleted file mode") {
                BridgeChangeStatus::Deleted
            } else if section.contains("\nrename from ") {
                BridgeChangeStatus::Renamed
            } else {
                BridgeChangeStatus::Modified
            };
            let lines = section.lines().skip_while(|l| !l.starts_with("@@"));
            let (mut added, mut removed) = (0, 0);
            for line in lines {
                if line.starts_with('+') {
                    added += 1;
                } else if line.starts_with('-') {
                    removed += 1;
                }
            }
            (
                BridgeFileChange {
                    shown: visible_name(&path),
                    path,
                    status,
                    added,
                    removed,
                },
                // SECURITY.md: the Diff panel writes out what could hide a change.
                visible_lines(patch),
            )
        })
        .collect()
}

/// The changed file's path: a rename's new name, else the second half of `a/<path> b/<path>`,
/// which also holds for a path with ` b/` in it.
fn path_of(header: &str, section: &str) -> String {
    if let Some(to) = section.lines().find_map(|l| l.strip_prefix("rename to ")) {
        return to.to_owned();
    }
    let half = header.len().saturating_sub(1) / 2;
    match (header.get(..half), header.get(half + 1..)) {
        (Some(a), Some(b)) if a.strip_prefix("a/") == b.strip_prefix("b/") => {
            b.get(2..).unwrap_or_default().to_owned()
        }
        _ => header
            .rsplit_once(" b/")
            .map(|(_, b)| b.to_owned())
            .unwrap_or_default(),
    }
}
