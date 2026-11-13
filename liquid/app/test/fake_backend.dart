import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;

import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/worktrees.dart';
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
  BridgeHistory stored = historyOf();

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

  /// The counts the stream's events were confirmed up to, in order.
  final confirmed = <int>[];

  @override
  Future<void> confirmApplied(int applied) async => confirmed.add(applied);

  /// What the agent rejects sends with, like the bridge does.
  String? sendError;

  /// Holds sends until it completes.
  Completer<void>? sendGate;

  /// Called as each send reaches the agent, which then starts its turn.
  void Function(String message)? onSend;

  @override
  Future<void> send(String message, {required bool followUp}) async {
    calls.add('${followUp ? 'follow-up' : 'send'} $message');
    onSend?.call(message);
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
    final read = ++turnReads;
    await turnsGate?.future;
    if (failTurns || (turnsFailFrom != null && read >= turnsFailFrom!)) {
      throw StateError('no turns');
    }
    return turnList;
  }

  /// Whether turn reads fail, and the first read that fails while earlier ones succeed.
  bool failTurns = false;
  int? turnsFailFrom;

  /// How many turn reads began.
  int turnReads = 0;

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
  Future<BridgeHistory> history() async {
    calls.add('history');
    await gate?.future;
    return stored;
  }

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

  /// Holds the recent-sessions read until it completes, and the read answers the list it began with.
  Completer<void>? recentGate;

  @override
  Future<List<BridgeStoredSession>> recentSessions() async {
    final answer = storedSessions;
    await recentGate?.future;
    return answer;
  }

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

  /// Holds the newest chat page until it completes.
  Completer<void>? newestGate;

  @override
  Future<BridgeTranscript> transcript(String? before) async {
    calls.add('transcript ${before ?? ''}'.trim());
    if (before == null) await newestGate?.future;
    if (before != null) await earlierGate?.future;
    if (transcriptError case final error?) throw error;
    return transcripts[before ?? ''] ??
        const BridgeTranscript(items: [], more: false);
  }

  /// The checkout's Git state; null outside Git.
  BridgeGitStatus? git = gitStatusOf();
  Object? gitError;
  Completer<BridgeGitStatus?>? statusGate;
  Completer<BridgeCommitPlan>? planGate;
  Completer<void>? commitGate;
  Completer<BridgeGitOutcome>? transferGate;
  BridgeGitOutcome? transferOutcome;
  BridgePushConfirmation? lastConfirmation;
  Completer<BridgeGitOutcome>? initializeGate;
  BridgeBranches branches = const BridgeBranches(branches: [], running: []);
  BridgeStash? stashed;
  ({String? branch, String? stash})? switchConfirmation;
  Map<String, Map<String, String>> stashPatches = const {};

  /// What each Git write returns, until it is set otherwise.
  BridgeGitOutcome outcome = outcomeOf(BridgeGitOutcomeKind.done);

  /// What fetching fails with.
  Object? fetchError;

  /// The remote a fetch reports it fetched, when it is not the checkout's.
  String? fetchedRemote;

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
            rawBranch: now.rawBranch,
            commit: now.commit,
            upstream: now.upstream,
            remote: now.remote,
            hasRemotes: now.hasRemotes,
            gone: now.gone,
            ahead: now.ahead,
            behind: now.behind,
            changed: now.changed,
            stash: stashed,
            defaultBranch: now.defaultBranch,
            merging: now.merging,
            conflicts: List.of(now.conflicts),
            repository: now.repository,
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
    // Git reports the branch checked out when it created the branch, unless a test says otherwise.
    final from = createdFrom ?? git?.rawBranch;
    onCreateBranch?.call(name);
    return BridgeGitOutcome(
      kind: outcome.kind,
      files: outcome.files,
      running: outcome.running,
      detail: outcome.detail,
      commit: outcome.commit,
      transfer: outcome.transfer,
      createdFrom: from,
    );
  }

  /// The branch a created branch reports it was created from, when it is not the checkout's.
  String? createdFrom;

  /// Runs when a branch is created, as Git would switch to it.
  void Function(String name)? onCreateBranch;

  BridgeCommitPlan commitPlanned = const BridgeCommitPlan(
    snapshot: 'snapshot',
    files: [],
    running: [],
  );
  String? committedSnapshot;
  List<String>? committedPaths;
  String? committedMessage;
  String? committedBranch;

  /// What committing throws instead of answering, as when Liquid loses Git's answer.
  Object? commitError;

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
    await commitGate?.future;
    if (commitError case final error?) throw error;
    return outcome.kind == BridgeGitOutcomeKind.done && outcome.commit == null
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
  Future<String?> fetch() async {
    calls.add('fetch');
    if (fetchError case final error?) throw error;
    return fetchedRemote ?? git?.remote;
  }

  @override
  Future<BridgeGitOutcome> push(BridgePushConfirmation? confirmed) async {
    calls.add('push ${confirmed != null}');
    lastConfirmation = confirmed;
    if (transferGate case final gate?) return gate.future;
    if (transferOutcome case final outcome?) return outcome;
    final now = git!;
    if (confirmed == null &&
        now.defaultBranch != null &&
        (now.upstream ?? '${now.remote}/${now.branch}') ==
            '${now.remote}/${now.defaultBranch}') {
      return BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.pushDefault,
        files: const [],
        running: const [],
        detail: now.defaultBranch!,
        transfer: transferOf(ahead: now.ahead.toInt()),
      );
    }
    return BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.done,
      files: const [],
      running: const [],
      detail: '',
      transfer: transferOf(
        branch: now.branch!,
        upstream: now.upstream ?? '${now.remote}/${now.branch}',
        ahead: now.ahead.toInt(),
        firstPush: now.upstream == null,
      ),
    );
  }

  @override
  Future<BridgeGitOutcome> pull() async {
    calls.add('pull');
    if (transferGate case final gate?) return gate.future;
    if (transferOutcome case final outcome?) return outcome;
    return BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.done,
      files: const [],
      running: const [],
      detail: '',
      transfer: transferOf(behind: git!.behind.toInt()),
    );
  }

  /// What planning, creating and reading pull requests return, until set otherwise.
  BridgePullRequestOutcome planned = pullRequestOutcomeOf(plan: planOf());
  BridgePullRequestOutcome created = pullRequestOutcomeOf(
    pullRequest: pullRequestOf(),
  );
  BridgePullRequestOutcome current = pullRequestOutcomeOf();
  Completer<BridgePullRequestOutcome>? createGate;
  Completer<BridgePullRequestOutcome>? pullRequestPlanGate;
  Completer<BridgePullRequestOutcome>? currentGate;
  List<BridgeName> baseBranches = const [];
  bool moreBases = false;

  /// What listing bases returns instead of [baseBranches], until cleared.
  BridgePullRequestOutcome? basesOutcome;
  BridgePullRequestDraft? lastDraft;

  @override
  Future<BridgePullRequestOutcome> pullRequestPlan({
    String? target,
    String? base,
  }) async {
    calls.add('plan ${target ?? '-'} ${base ?? '-'}');
    if (pullRequestPlanGate case final gate?) return gate.future;
    return planned;
  }

  @override
  Future<BridgePullRequestOutcome> createPullRequest(
    BridgePullRequestDraft draft,
  ) async {
    calls.add('create pull request');
    lastDraft = draft;
    if (createGate case final gate?) return gate.future;
    return created;
  }

  /// Pull request reads, which follow every status read with an upstream, so they stay out
  /// of [calls].
  int pullRequestReads = 0;

  @override
  Future<BridgePullRequestOutcome> pullRequest() async {
    pullRequestReads++;
    if (currentGate case final gate?) return gate.future;
    return current;
  }

  @override
  Future<BridgePullRequestOutcome> pullRequestBases(
    String host,
    String target,
  ) async {
    calls.add('bases $target');
    return basesOutcome ??
        pullRequestOutcomeOf(bases: baseBranches, moreBases: moreBases);
  }

  /// What a base search answers, until set otherwise: the branches of [baseBranches] that
  /// contain its text. [searchOutcome] replaces the answer and [searchGate] holds it.
  BridgePullRequestOutcome? searchOutcome;
  Completer<BridgePullRequestOutcome>? searchGate;

  @override
  Future<BridgePullRequestOutcome> pullRequestBaseSearch(
    String host,
    String target,
    String query,
  ) async {
    calls.add('search $target $query');
    if (searchGate case final gate?) return gate.future;
    return searchOutcome ??
        pullRequestOutcomeOf(
          bases: [
            for (final name in baseBranches)
              if (name.shown.contains(query)) name,
          ],
        );
  }

  /// What the Chat's model writes for a pull request, until set otherwise; [writeError] fails
  /// it and [writeGate] holds it.
  BridgePullRequestText written = const BridgePullRequestText(
    title: 'Return 429 when logins are blocked',
    body: 'Blocked logins now wait for the window.',
    commits: 2,
  );
  Object? writeError;
  Completer<BridgePullRequestText>? writeGate;

  @override
  Future<BridgePullRequestText> writePullRequestText({
    required String model,
    required String mergeBase,
    required String headCommit,
  }) async {
    calls.add('write $model $mergeBase..$headCommit');
    if (writeGate case final gate?) return gate.future;
    if (writeError case final error?) throw error;
    return written;
  }

  /// Before a commit, the model has nothing to describe.
  bool nothingToDescribe = false;

  @override
  Future<BridgePullRequestText?> writePullRequestDraftText({
    required String model,
    String? base,
    required String snapshot,
    required List<String> paths,
  }) async {
    calls.add('write draft $model ${base ?? '-'} ${paths.join(',')}');
    if (writeGate case final gate?) return gate.future;
    if (writeError case final error?) throw error;
    return nothingToDescribe ? null : written;
  }

  /// What preparing "Commit, push & create PR" returns, until set otherwise.
  BridgePullRequestOutcome prepared = pullRequestOutcomeOf(
    plan: planOf(comparison: null),
  );

  @override
  Future<BridgePullRequestOutcome> pullRequestPrepared({
    String? target,
    String? base,
  }) async {
    calls.add('prepare ${target ?? '-'} ${base ?? '-'}');
    return prepared;
  }

  @override
  Future<void> stopPullRequestText() async => calls.add('stop writing');

  /// Git's worktrees, the reviews of their removal by path, and what removing returns.
  List<BridgeWorktree> worktreeList = const [];
  Map<String, BridgeWorktreeRemoval> reviews = {};
  BridgeWorktreeOutcome? removed;

  /// Holds a removal until it completes with its outcome.
  Completer<BridgeWorktreeOutcome>? removeGate;

  @override
  Future<List<BridgeWorktree>> worktrees() async {
    calls.add('worktrees');
    return worktreeList;
  }

  @override
  Future<BridgeWorktreeRemoval> worktreeRemoval(String path) async {
    calls.add('review $path');
    return reviews[path]!;
  }

  @override
  Future<BridgeWorktreeOutcome> removeWorktree(
    BridgeWorktreeRemoval reviewed,
  ) async {
    calls.add('remove ${reviewed.worktree.path}');
    if (removeGate case final gate?) return gate.future;
    return removed ??
        const BridgeWorktreeOutcome(
          kind: BridgeWorktreeOutcomeKind.done,
          running: [],
          detail: '',
        );
  }

  /// What reverting to before a turn would change.
  BridgeRevertPlan revertPlanned = revertPlanOf();

  /// What a revert fails with.
  Object? revertError;

  /// Holds reverts until it completes.
  Completer<void>? revertGate;

  /// Holds the revert plan until it completes.
  Completer<void>? revertPlanGate;

  @override
  Future<BridgeRevertPlan> revertPlan(String turn) async {
    calls.add('revert plan $turn');
    await revertPlanGate?.future;
    return revertPlanned;
  }

  /// The confirmation the last revert received.
  BridgeRevertConfirmation? revertedWith;

  @override
  Future<void> revertToTurn(
    String turn,
    BridgeRevertConfirmation confirmation,
  ) async {
    calls.add('revert $turn');
    revertedWith = confirmation;
    await revertGate?.future;
    if (revertError case final error?) throw error;
  }

  /// The sessions with checkpoints, and what removing them fails with.
  List<BridgeCheckpointSession> checkpointed = const [];
  Object? removeError;

  /// Holds the checkpoint sessions read until it completes.
  Completer<void>? checkpointGate;

  @override
  Future<List<BridgeCheckpointSession>> checkpointSessions() async {
    await checkpointGate?.future;
    return checkpointed;
  }

  @override
  Future<void> removeAllCheckpoints() async {
    calls.add('remove all checkpoints');
    if (removeError case final error?) throw error;
    checkpointed = const [];
  }

  /// The file of the open session.
  String? openFile = '/sessions/open.jsonl';

  /// Holds the open session's file, as it was when asked, until it completes.
  Completer<void>? sessionFileGate;

  @override
  Future<String?> sessionFile() async {
    final file = openFile;
    await sessionFileGate?.future;
    return file;
  }

  /// Turns with checkpoints by session file.
  Map<String, int> turnCounts = const {};

  /// Holds the turn counts read until it completes.
  Completer<void>? storedTurnsGate;

  @override
  Future<int> storedTurns(String path) async {
    await storedTurnsGate?.future;
    return turnCounts[path] ?? 0;
  }

  /// The folder an export returns, or the error it throws.
  String exportFolder =
      '/Users/ana/Downloads/Rate limit for login – record 2026-11-11';
  Object? exportError;

  @override
  Future<String> exportRecord({
    required String path,
    required String title,
    required String host,
    required String day,
  }) async {
    calls.add('export $path');
    if (exportError case final error?) throw error;
    return exportFolder;
  }

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
  String? checkpoint,
}) => BridgeEvent(
  kind: kind,
  item: item,
  step: step,
  request: request,
  text: text,
  display: display,
  success: success,
  queue: queue,
  checkpoint: checkpoint,
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

/// A session with `tree` (by default a bare root) and the leaf `leaf`.
BridgeHistory historyOf({
  List<BridgeTreeNode>? tree,
  String? leaf,
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
  tree: tree ?? const [BridgeTreeNode(id: 'root', kind: BridgeEntryKind.other)],
  leaf: leaf ?? 'e1',
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
  confirmation: BridgeRevertConfirmation(
    target: 'target',
    checkout: 'checkout',
    branch: branch,
    kept: const [],
    ignored: const [],
    ignoredState: 'ignored',
  ),
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
  String? commit = '1a2b3c4',
  String? branch = 'main',
  String? rawBranch,
  String? upstream = 'origin/main',
  String? remote = 'origin',
  bool? hasRemotes,
  bool gone = false,
  int ahead = 0,
  int behind = 0,
  int changed = 0,
  String? defaultBranch,
  bool merging = false,
  List<String> conflicts = const [],
  String repository = '/work/api/.git',
}) => BridgeGitStatus(
  branch: branch,
  rawBranch: rawBranch ?? branch,
  commit: commit,
  upstream: upstream,
  remote: remote,
  hasRemotes: hasRemotes ?? remote != null,
  gone: gone,
  ahead: ahead,
  behind: behind,
  changed: changed,
  stash: null,
  defaultBranch: defaultBranch,
  merging: merging,
  conflicts: conflicts,
  repository: repository,
);

BridgeGitTransfer transferOf({
  String branch = 'main',
  String remote = 'origin',
  String upstream = 'origin/main',
  int ahead = 0,
  int behind = 0,
  bool firstPush = false,
  bool gone = false,
}) => BridgeGitTransfer(
  branch: branch,
  remote: remote,
  upstream: upstream,
  ahead: ahead,
  behind: behind,
  firstPush: firstPush,
  gone: gone,
  destination: upstream.split('/').skip(1).join('/'),
  confirmation: BridgePushConfirmation(
    branch: branch,
    remote: remote,
    reference: 'refs/heads/${upstream.split('/').skip(1).join('/')}',
    commit: '1a2b3c4',
    repository: 'https://example.invalid/repo.git',
  ),
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
  bool attributionAvailable = true,
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
  attributionAvailable: attributionAvailable,
  edited: edited,
  stagedPartly: stagedPartly,
);

BridgePullRequestOutcome pullRequestOutcomeOf({
  BridgePullRequestOutcomeKind kind = BridgePullRequestOutcomeKind.done,
  BridgePullRequestPlan? plan,
  BridgePullRequest? pullRequest,
  List<BridgeName> bases = const [],
  bool moreBases = false,
  String host = '',
  String remote = '',
  String branch = '',
  String upstream = '',
  String repository = '',
  int ahead = 0,
  int behind = 0,
  bool first = false,
  String page = '',
  String detail = '',
}) => BridgePullRequestOutcome(
  kind: kind,
  plan: plan,
  pullRequest: pullRequest,
  bases: bases,
  moreBases: moreBases,
  page: page,
  host: host,
  remote: remote,
  branch: branch,
  upstream: upstream,
  repository: repository,
  ahead: ahead,
  behind: behind,
  first: first,
  detail: detail,
);

BridgePullRequest pullRequestOf({
  int number = 42,
  String title = 'Return 429 with Retry-After on /login',
  BridgePullRequestState state = BridgePullRequestState.open,
  String head = 'feature/retry-after',
  String base = 'main',
  int passed = 0,
  int failed = 0,
  int pending = 0,
  BridgeReviewDecision? review,
}) => BridgePullRequest(
  number: number,
  title: title,
  url: 'https://github.com/acme/api/pull/$number',
  state: state,
  repository: 'acme/api',
  base: base,
  head: head,
  checksPassed: passed,
  checksFailed: failed,
  checksPending: pending,
  review: review,
);

BridgeName nameOf(String name) => BridgeName(name: name, shown: name);

BridgePullRequestPlan planOf({
  List<String> targets = const ['acme/api'],
  String? target = 'acme/api',
  String head = 'feature/retry-after',
  String? defaultBranch = 'main',
  String? base = 'main',
  BridgeComparison? comparison = const BridgeComparison(
    commits: 2,
    files: 3,
    added: 12,
    removed: 1,
    url: 'https://github.com/acme/api/compare/main...feature/retry-after',
    mergeBase: 'ba5e',
  ),
  bool comparisonFailed = false,
  bool baseMissing = false,
  bool parentUnknown = false,
  BridgePullRequest? open,
  int uncommitted = 0,
  String subject = 'Return 429 with Retry-After on /login',
  String body = 'Blocked login attempts now get 429.',
  String commit = 'c0ffee',
  String headBranch = 'feature/retry-after',
}) => BridgePullRequestPlan(
  identity: BridgePullRequestHead(
    host: 'github.com',
    headRepository: targets.last,
    headBranch: headBranch,
    headCommit: commit,
  ),
  host: 'github.com',
  head: head,
  targets: targets,
  parentUnknown: parentUnknown,
  target: target,
  defaultBranch: defaultBranch == null ? null : nameOf(defaultBranch),
  base: base == null ? null : nameOf(base),
  comparison: comparison,
  comparisonFailed: comparisonFailed,
  baseMissing: baseMissing,
  open: open,
  uncommitted: uncommitted,
  subject: subject,
  body: body,
);

BridgeWorktree worktreeOf(
  String path, {
  String? branch,
  bool main = false,
  bool current = false,
  bool locked = false,
  bool missing = false,
  bool unreadable = false,
  List<BridgeRunningAgent> running = const [],
}) => BridgeWorktree(
  path: path,
  shown: path,
  branch: branch == null ? null : nameOf(branch),
  main: main,
  current: current,
  locked: locked,
  missing: missing,
  unreadable: unreadable,
  running: running,
  sessions: [for (final _ in running) 's'],
);

BridgeWorktreeRemoval removalOf(
  BridgeWorktree worktree, {
  List<BridgeWorktreeFile> files = const [],
  int ignored = 0,
  int unpushed = 0,
  int stranded = 0,
  bool moreMarked = false,
  bool moreIgnored = false,
}) => BridgeWorktreeRemoval(
  worktree: worktree,
  head: 'c0ffee',
  files: files,
  moreFiles: false,
  moreMarked: moreMarked,
  ignored: ignored,
  moreIgnored: moreIgnored,
  unpushed: unpushed,
  stranded: stranded,
  removable:
      files.isEmpty &&
      ignored == 0 &&
      !moreIgnored &&
      stranded == 0 &&
      !moreMarked &&
      !worktree.unreadable &&
      worktree.running.isEmpty &&
      !worktree.main &&
      !worktree.current &&
      !worktree.locked &&
      !worktree.missing,
);
