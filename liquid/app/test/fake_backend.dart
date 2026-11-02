import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;

import 'dart:math' as math;

import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/backend.dart';

/// Records commands and lets tests emit events.
class FakeBackend implements SessionBackend {
  final controller = StreamController<BridgeEvent>();
  final calls = <String>[];

  /// The files the shown turns changed; none outside Git.
  List<BridgeFileChange>? changed = const [];
  Map<String, String> patches = const {};
  BridgeHistory stored = historyOf([]);

  /// Histories by the requested leaves, joined by spaces; others get [stored].
  final histories = <String, BridgeHistory>{};
  bool cancelled = false;
  final _stopped = Completer<void>();

  /// Like the bridge's stream, cancelling it finishes only with the next event, which an idle
  /// agent sends only once it is shut down.
  @override
  Stream<BridgeEvent> events() {
    controller.onCancel = () {
      cancelled = true;
      return _stopped.future;
    };
    return controller.stream;
  }

  /// What the agent rejects sends with, like the bridge does.
  String? sendError;

  /// Holds sends until it completes.
  Completer<void>? sendGate;

  @override
  Future<void> send(String message, {required bool followUp}) async {
    calls.add('${followUp ? 'follow-up' : 'send'} $message');
    await sendGate?.future;
    if (sendError case final error?) throw error;
  }

  /// What clear_queue returns.
  BridgeQueue queued = const BridgeQueue(steering: [], followUp: []);

  @override
  Future<BridgeQueue> clearQueue() async {
    calls.add('clear queue');
    return queued;
  }

  /// What aborting fails with.
  Object? abortError;

  @override
  Future<void> abort() async {
    calls.add('abort');
    if (abortError case final error?) throw error;
  }

  @override
  Future<void> setApprovalMode(BridgeApprovalMode mode) async =>
      calls.add('mode ${mode.name}');
  @override
  Future<void> decide(String request, BridgeDecision decision) async =>
      calls.add('decide $request ${decision.name}');
  @override
  Future<void> answerText(String request, String text) async =>
      calls.add('answer $request $text');
  @override
  Future<void> answerConfirm(String request, bool confirmed) async =>
      calls.add('confirm $request $confirmed');
  @override
  Future<void> dismiss(String request) async => calls.add('dismiss $request');

  /// What reading the changes fails with.
  Object? changesError;

  /// Holds change reads until it completes.
  Completer<void>? changesGate;
  int changeReads = 0;

  /// The changes once whitespace is hidden; [changed] when null.
  List<BridgeFileChange>? changedWithoutWhitespace;

  /// The turns of the current branch; by default one that ended.
  List<BridgeTurn> turnList = [turnOf('T1', number: 1)];

  /// Whether the compared checkpoints were removed.
  bool checkpointsRemoved = false;

  /// The checkpoints each read compared, as `from..to`.
  final compared = <String>[];

  /// Whether the project is in a Git repository.
  bool inRepository = true;

  @override
  Future<bool> isInRepository() async => inRepository;

  @override
  Future<List<BridgeTurn>> turns() async {
    await turnsGate?.future;
    return turnList;
  }

  /// Holds turn reads until it completes.
  Completer<void>? turnsGate;

  @override
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  }) async {
    changeReads++;
    compared.add('$from..$to');
    await changesGate?.future;
    if (changesError case final error?) throw error;
    final files = hideWhitespace
        ? changedWithoutWhitespace ?? changed
        : changed;
    // A failed turn has no checkpoint at the end the comparison needs, as in the core.
    String? failureOf(String id) =>
        turnList.where((turn) => turn.id == id).firstOrNull?.failure;
    final missing =
        failureOf(to) != null || (from == to && failureOf(from) != null);
    if (missing && files != null) {
      return const BridgeTurnChanges(
        state: BridgeChangesState.removed,
        files: [],
      );
    }
    return BridgeTurnChanges(
      state: files == null
          ? BridgeChangesState.outsideGit
          : checkpointsRemoved
          ? BridgeChangesState.removed
          : BridgeChangesState.shown,
      files: checkpointsRemoved ? const [] : files ?? const [],
    );
  }

  /// What reading a patch fails with, like the bridge does.
  String? patchError;

  /// Holds patch reads until it completes.
  Completer<void>? patchGate;

  /// The files whose patch was read, in order.
  final patchReads = <String>[];

  /// Patches of particular comparisons, as `from..to`; others come from [patches].
  Map<String, Map<String, String>> turnPatches = const {};

  @override
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  }) async {
    patchReads.add(hideWhitespace ? '$path without whitespace' : path);
    await patchGate?.future;
    if (patchError case final error?) throw error;
    return turnPatches['$from..$to']?[path] ?? patches[path] ?? '';
  }

  @override
  Future<BridgeHistory> history(List<String> leaves) async {
    final key = leaves.join(' ');
    calls.add('history $key'.trim());
    await gate?.future;
    if (historyError case final error?) throw error;
    return histories[key] ?? stored;
  }

  Object? historyError;

  /// Holds history reads until it completes.
  Completer<void>? gate;

  /// Whether [branch] moves the session's leaf, as a successful branch does.
  bool branchMoves = true;
  Object? branchError;

  /// The project's stored sessions, the models and the selected one, as pi reports them.
  List<BridgeStoredSession> storedSessions = const [];
  List<BridgeModel> modelList = const [];
  String? model;

  /// Whether an extension cancels session switches.
  bool switchCancelled = false;

  @override
  Future<List<BridgeStoredSession>> recentSessions() async => storedSessions;

  /// The open session's title, as pi stored it.
  String? sessionTitle;

  @override
  Future<String?> title() async => sessionTitle;

  /// Holds session switches until it completes.
  Completer<void>? switchGate;

  @override
  Future<bool> newSession() async {
    calls.add('new session');
    await switchGate?.future;
    if (!switchCancelled) openFile = '/sessions/new.jsonl';
    return !switchCancelled;
  }

  @override
  Future<bool> switchSession(String path) async {
    calls.add('switch $path');
    return !switchCancelled;
  }

  @override
  Future<List<BridgeModel>> models() async => modelList;

  @override
  Future<String?> currentModel() async => model;

  @override
  Future<void> setModel(BridgeModel next) async {
    calls.add('model ${next.provider}/${next.id}');
    model = '${next.provider}/${next.id}';
  }

  /// Transcript pages by their `before` entry; the newest page has the key ''.
  final transcripts = <String, BridgeTranscript>{};

  /// What reading the chat fails with.
  Object? transcriptError;

  /// Holds reads of earlier chat pages until it completes.
  Completer<void>? earlierGate;

  @override
  Future<BridgeTranscript> transcript(String? before) async {
    calls.add('transcript ${before ?? ''}'.trim());
    if (before != null) await earlierGate?.future;
    if (transcriptError case final error?) throw error;
    return transcripts[before ?? ''] ??
        const BridgeTranscript(items: [], more: false);
  }

  /// Context details by response entry.
  final contexts = <String, BridgeContextDetail>{};
  Object? contextError;

  @override
  Future<BridgeContextDetail?> context(String entry) async {
    calls.add('context $entry');
    if (contextError case final error?) throw error;
    return contexts[entry];
  }

  /// The checkout's Git state; null outside Git.
  BridgeGitStatus? git = gitStatusOf();
  Object? gitError;
  Completer<BridgeGitStatus?>? statusGate;
  Completer<BridgeCommitPlan>? planGate;
  Completer<BridgeGitOutcome>? initializeGate;
  BridgeBranches branches = const BridgeBranches(branches: [], running: []);
  BridgeStash? stashed;
  ({String? branch, String? stash})? switchConfirmation;
  Map<String, Map<String, String>> stashPatches = const {};

  /// What each Git write returns, until it is set otherwise.
  BridgeGitOutcome outcome = outcomeOf(BridgeGitOutcomeKind.done);

  /// What fetching fails with.
  Object? fetchError;

  int statusReads = 0;

  @override
  Future<BridgeGitStatus?> gitStatus() async {
    statusReads++;
    if (statusGate case final gate?) return gate.future;
    if (gitError case final error?) throw error;
    final now = git;
    return now == null
        ? null
        : BridgeGitStatus(
            branch: now.branch,
            commit: now.commit,
            upstream: now.upstream,
            ahead: now.ahead,
            behind: now.behind,
            changed: now.changed,
            stash: stashed,
            defaultBranch: now.defaultBranch,
            merging: now.merging,
            conflicts: List.of(now.conflicts),
          );
  }

  @override
  Future<BridgeBranches> gitBranches() async => branches;

  @override
  Future<BridgeGitOutcome> switchBranch(
    BridgeBranch branch, {
    required bool leaveChanges,
    required String? fromBranch,
    required String? replacingStash,
  }) async {
    calls.add('switch ${branch.name}${leaveChanges ? ' leaving changes' : ''}');
    switchConfirmation = (branch: fromBranch, stash: replacingStash);
    return outcome;
  }

  @override
  Future<BridgeGitOutcome> createBranch(String name) async {
    calls.add('create $name');
    return outcome;
  }

  BridgeCommitPlan commitPlanned = const BridgeCommitPlan(
    snapshot: 'snapshot',
    files: [],
    running: [],
  );
  String? committedSnapshot;
  List<String>? committedPaths;
  String? committedMessage;
  String? committedBranch;

  @override
  Future<BridgeCommitPlan> commitPlan() async {
    calls.add('commit plan');
    return planGate?.future ?? commitPlanned;
  }

  @override
  Future<BridgeGitOutcome> commitFiles(
    String snapshot,
    List<String> paths,
    String message,
    String? branch,
  ) async {
    committedSnapshot = snapshot;
    committedPaths = paths;
    committedMessage = message;
    committedBranch = branch;
    return outcome.kind == BridgeGitOutcomeKind.done
        ? BridgeGitOutcome(
            kind: BridgeGitOutcomeKind.done,
            files: [],
            running: [],
            detail: '',
            commit: BridgeCommit(
              branch: branch ?? git?.branch ?? 'main',
              subject: message.split('\n').first,
              count: paths.length,
            ),
          )
        : outcome;
  }

  @override
  Future<BridgeGitOutcome> initializeGit() async {
    calls.add('initialize git');
    if (initializeGate case final gate?) return gate.future;
    git = gitStatusOf();
    inRepository = true;
    return outcome;
  }

  String uncommittedRevision = 'uncommitted';

  @override
  Future<(String, List<BridgeFileChange>)> uncommittedChanges({
    required bool hideWhitespace,
  }) async {
    calls.add(
      'uncommitted changes${hideWhitespace ? ' without whitespace' : ''}',
    );
    return (
      uncommittedRevision,
      List<BridgeFileChange>.of(
        hideWhitespace
            ? changedWithoutWhitespace ?? changed ?? []
            : changed ?? [],
      ),
    );
  }

  @override
  Future<String> uncommittedPatch(
    String path, {
    required bool hideWhitespace,
  }) async {
    calls.add('uncommitted patch $path');
    final patch = patches[path] ?? '';
    await patchGate?.future;
    return patch;
  }

  @override
  Future<BridgeStash?> gitStash() async => stashed;

  @override
  Future<List<BridgeFileChange>> stashChanges(
    String id, {
    required bool hideWhitespace,
  }) async => hideWhitespace
      ? changedWithoutWhitespace ?? stashed?.files ?? const []
      : stashed?.files ?? const [];

  @override
  Future<String> stashPatch(
    String id,
    String path, {
    required bool hideWhitespace,
  }) async {
    calls.add('stash patch $path');
    await patchGate?.future;
    return stashPatches[id]?[path] ?? patches[path] ?? '';
  }

  @override
  Future<BridgeGitOutcome> restoreStash(String id) async {
    calls.add('restore stash $id');
    return outcome;
  }

  @override
  Future<BridgeGitOutcome> discardStash(String id) async {
    calls.add('discard stash $id');
    return outcome;
  }

  @override
  Future<bool> fetch() async {
    calls.add('fetch');
    if (fetchError case final error?) throw error;
    return true;
  }

  /// What reverting to before a turn would change.
  BridgeRevertPlan revertPlanned = revertPlanOf();

  /// What a revert fails with.
  Object? revertError;

  @override
  Future<BridgeRevertPlan> revertPlan(String turn) async {
    calls.add('revert plan $turn');
    return revertPlanned;
  }

  @override
  Future<void> revertToTurn(String turn) async {
    calls.add('revert $turn');
    if (revertError case final error?) throw error;
  }

  /// The sessions with checkpoints, and what removing them fails with.
  List<BridgeCheckpointSession> checkpointed = const [];
  Object? removeError;

  @override
  Future<List<BridgeCheckpointSession>> checkpointSessions() async =>
      checkpointed;

  @override
  Future<void> removeAllCheckpoints() async {
    calls.add('remove all checkpoints');
    if (removeError case final error?) throw error;
    checkpointed = const [];
  }

  /// The file of the open session.
  String? openFile = '/sessions/open.jsonl';

  @override
  Future<String?> sessionFile() async => openFile;

  /// Turns with checkpoints by session file.
  Map<String, int> turnCounts = const {};

  @override
  Future<int> storedTurns(String path) async => turnCounts[path] ?? 0;

  @override
  Future<void> deleteSession(String path) async {
    calls.add('delete $path');
    if (removeError case final error?) throw error;
    storedSessions = [
      for (final session in storedSessions)
        if (session.path != path) session,
    ];
  }

  /// Moves the leaf like pi: to the entry, or before it for a prompt.
  @override
  Future<void> branch(String entry) async {
    calls.add('branch $entry');
    if (branchError case final error?) throw error;
    if (branchMoves) {
      final node = stored.tree.where((node) => node.id == entry).firstOrNull;
      stored = BridgeHistory(
        tree: stored.tree,
        leaf: node?.kind == BridgeEntryKind.user ? node?.parent : entry,
        trajectories: stored.trajectories,
        usage: stored.usage,
      );
    }
  }

  @override
  Future<bool> shutdown() async {
    calls.add('shutdown');
    if (!_stopped.isCompleted) _stopped.complete();
    return true;
  }
}

BridgeEvent event(
  BridgeEventKind kind, {
  BridgeItem? item,
  BridgeStep? step,
  BridgeRequest? request,
  String? text,
  BridgeDisplay? display,
  bool? success,
  BridgeQueue? queue,
}) => BridgeEvent(
  kind: kind,
  item: item,
  step: step,
  request: request,
  text: text,
  display: display,
  success: success,
  queue: queue,
);

BridgeItem item(
  int id, {
  BridgeItemKind? kind,
  String? tool,
  bool failed = false,
  int? durationMs,
  int? added,
  int? removed,
}) => BridgeItem(
  id: id,
  kind: kind,
  tool: tool,
  failed: failed,
  durationMs: durationMs,
  added: added,
  removed: removed,
);

BridgeUsage usage(int input, int output, int cost) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
  costMicroUsd: cost,
);

BridgeRecord record(
  String entry,
  int turn,
  BridgeRecordKind kind,
  String label,
  int start, {
  int? duration,
  int? firstToken,
  String detail = '',
  String target = '',
  String? changedFile,
  BridgeDecision? decision,
  BridgeUsage? usage,
  BridgeStepContext? context,
  bool failed = false,
  BridgeToolOutcome outcome = const BridgeToolOutcome(),
  int added = 0,
  String? model,
}) => BridgeRecord(
  entry: entry,
  turn: turn,
  kind: kind,
  label: label,
  detail: detail,
  target: target,
  changedFile: changedFile,
  decision: decision,
  startMs: start,
  durationMs: duration,
  firstTokenMs: firstToken,
  model: model ?? (kind == BridgeRecordKind.message ? 'faux-1' : ''),
  usage: usage,
  stopReason: '',
  failed: failed,
  context: context,
  outcome: outcome,
  addedChars: added,
);

/// A session whose current branch is `records`, as one tree path.
BridgeHistory historyOf(
  List<BridgeRecord> records, {
  List<BridgeTreeNode>? tree,
  String? leaf,
  List<List<BridgeRecord>> others = const [],
  BridgeUsage usage = const BridgeUsage(
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    costMicroUsd: 0,
  ),
}) => BridgeHistory(
  usage: usage,
  tree:
      tree ??
      [
        const BridgeTreeNode(
          failed: false,
          id: 'root',
          kind: BridgeEntryKind.other,
          preview: '',
        ),
        for (final (index, record) in records.indexed)
          BridgeTreeNode(
            failed: record.failed,
            id: record.entry,
            parent: index == 0 ? 'root' : records[index - 1].entry,
            kind: switch (record.kind) {
              BridgeRecordKind.user => BridgeEntryKind.user,
              BridgeRecordKind.message => BridgeEntryKind.assistant,
              BridgeRecordKind.tool => BridgeEntryKind.toolResult,
              BridgeRecordKind.approval => BridgeEntryKind.approval,
            },
            preview: record.label,
          ),
      ],
  leaf: leaf ?? records.lastOrNull?.entry ?? 'e1',
  trajectories: [
    for (final branch in [records, ...others])
      BridgeTrajectory(
        records: branch,
        durationMs: branch.fold(
          0,
          (end, record) =>
              math.max(end, record.startMs + (record.durationMs ?? 0)),
        ),
      ),
  ],
);

BridgeTranscriptItem chatItem(
  String entry,
  BridgeItemKind kind, {
  String text = '',
  String? tool,
}) => BridgeTranscriptItem(
  entry: entry,
  kind: kind,
  text: text,
  tool: tool,
  failed: false,
);

/// A revert plan of `files`, refused while `running` agents run.
BridgeRevertPlan revertPlanOf({
  List<BridgeRevertFile> files = const [],
  List<BridgeRunningAgent> running = const [],
  String? branch = 'main',
  BridgeChangesState state = BridgeChangesState.shown,
}) => BridgeRevertPlan(
  state: state,
  files: files,
  running: running,
  branch: branch,
);

/// A file a revert changes as `status` says.
BridgeRevertFile revertedFile(
  String path,
  BridgeChangeStatus status, {
  bool edited = false,
  int? modifiedMs,
}) => BridgeRevertFile(
  path: path,
  shown: path,
  status: status,
  edited: edited,
  modifiedMs: modifiedMs,
  ignored: false,
  kept: false,
);

/// A turn of the current branch; it ended unless `ended` says otherwise.
BridgeTurn turnOf(
  String id, {
  required int number,
  String prompt = 'Add rate limiting',
  bool ended = true,
  int startedMs = 1793527200000,
  String? failure,
  int? files,
  int? added,
  int? removed,
  String? startEntry,
  String? endEntry,
}) => BridgeTurn(
  id: id,
  startEntry: startEntry ?? '$id-start',
  endEntry: ended ? endEntry ?? '$id-end' : null,
  number: number,
  promptEntry: '$id-prompt',
  prompt: prompt,
  startedMs: startedMs,
  endedMs: ended ? startedMs + 120000 : null,
  ended: ended && failure == null,
  failure: failure,
  files: files ?? (added == null ? null : 1),
  added: added,
  removed: removed,
);

BridgeGitStatus gitStatusOf({
  String? branch = 'main',
  String? upstream = 'origin/main',
  int ahead = 0,
  int behind = 0,
  int changed = 0,
  String? defaultBranch,
  bool merging = false,
  List<String> conflicts = const [],
}) => BridgeGitStatus(
  branch: branch,
  commit: '1a2b3c4',
  upstream: upstream,
  ahead: ahead,
  behind: behind,
  changed: changed,
  stash: null,
  defaultBranch: defaultBranch,
  merging: merging,
  conflicts: conflicts,
);

BridgeBranch branchOf(
  String name, {
  bool remote = false,
  bool current = false,
  bool stashed = false,
}) => BridgeBranch(
  name: name,
  shown: name,
  remote: remote,
  committedMs: 0,
  current: current,
  stashed: stashed,
);

BridgeGitOutcome outcomeOf(
  BridgeGitOutcomeKind kind, {
  List<String> files = const [],
  List<BridgeRunningAgent> running = const [],
  String detail = '',
}) => BridgeGitOutcome(
  kind: kind,
  files: files,
  running: running,
  detail: detail,
);

BridgeCommitFile commitFileOf(
  String path, {
  List<int> turns = const [],
  bool edited = false,
  bool stagedPartly = false,
  BridgeChangeStatus status = BridgeChangeStatus.modified,
  int added = 0,
  int removed = 0,
}) => BridgeCommitFile(
  change: BridgeFileChange(
    path: path,
    shown: path,
    status: status,
    added: added,
    removed: removed,
  ),
  turns: Int64List.fromList(turns),
  edited: edited,
  stagedPartly: stagedPartly,
);
