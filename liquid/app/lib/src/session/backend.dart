import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/worktrees.dart';
import '../rust/api/history.dart';
import '../rust/api/session.dart';

/// Stored entries the chat reads at a time.
const chatPage = 200;

/// How many recent sessions the start page offers.
const recentLimit = 8;

/// What [SessionState] needs from a running agent session; tests substitute a fake.
abstract interface class SessionBackend {
  Stream<BridgeEvent> events();

  /// Confirms that the first [applied] events of [events] were applied, so the bridge may send
  /// that many more (ADR-0272).
  Future<void> confirmApplied(int applied);
  Future<void> send(String message, {required bool followUp});
  Future<BridgeQueue> clearQueue();
  Future<void> abort();
  Future<void> answerText(String request, String text);
  Future<void> answerConfirm(String request, bool confirmed);
  Future<void> dismiss(String request);
  Future<bool> shutdown();

  /// Whether the project is in a Git repository, so its turns can have checkpoints.
  Future<bool> isInRepository();

  /// The turns of the current branch, oldest first, with what each changed.
  Future<List<BridgeTurn>> turns();

  /// The files that changed from the start of turn `from` to the end of turn `to`.
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  });

  /// One file's unified diff from the start of turn `from` to the end of turn `to`.
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  });
  Future<BridgeHistory> history();

  /// What the session's stored entries count to, on every branch (ADR-0279).
  Future<BridgeStatistics> statistics();

  /// The context window the next request fills and when pi compacts it; null when the agent
  /// reports no window.
  Future<BridgeContext?> context();
  Future<BridgeTranscript> transcript(String? before);
  Future<void> branch(String entry);

  /// What reverting the files to before turn `turn` would change.
  Future<BridgeRevertPlan> revertPlan(String turn);

  /// Reverts the files to before turn `turn` and continues the session from before its
  /// prompt, which returns to the input area; refused once the files left the plan
  /// `confirmation` came with.
  Future<void> revertToTurn(String turn, BridgeRevertConfirmation confirmation);

  /// The sessions with checkpoints in the project's repository.
  Future<List<BridgeCheckpointSession>> checkpointSessions();

  /// Removes every checkpoint in the project's repository.
  Future<void> removeAllCheckpoints();

  /// The file pi stores the open session in, unless it keeps it in memory.
  Future<String?> sessionFile();

  /// The turns with checkpoints of the session stored at `path`.
  Future<int> storedTurns(String path);

  /// Deletes the stored session at `path`, not the open one, with its checkpoints.
  Future<void> deleteSession(String path);

  /// Writes the record of the session stored at `path` into Downloads; returns the folder.
  Future<String> exportRecord({
    required String path,
    required String title,
    required String host,
    required String day,
  });

  /// The project's recent stored sessions, newest first, without the current one.
  Future<List<BridgeStoredSession>> recentSessions();

  /// The open session's title, as the recent sessions are named; null until pi stored one.
  Future<String?> title();

  /// Starts a new session or continues a stored one; `false` when an extension cancelled it.
  Future<bool> newSession();
  Future<bool> switchSession(String path);

  /// The checkout's branch, upstream and uncommitted files; null outside Git.
  Future<BridgeGitStatus?> gitStatus();

  /// The checkout's branches and the agents running there.
  Future<BridgeBranches> gitBranches();

  /// Switches to `branch`; with `leaveChanges`, uncommitted changes stay in the stash of the
  /// branch left.
  Future<BridgeGitOutcome> switchBranch(
    BridgeBranch branch, {
    required bool leaveChanges,
    required String? fromBranch,
    required String? replacingStash,
  });

  /// Creates the branch `name` at the current commit and switches to it; the outcome names the
  /// branch it was created from.
  Future<BridgeGitOutcome> createBranch(String name);

  /// Captures the current whole files and their checkpoint attribution.
  Future<BridgeCommitPlan> commitPlan();

  /// Commits selected files only if they still match the captured tree.
  Future<BridgeGitOutcome> commitFiles(
    String snapshot,
    List<String> paths,
    String message,
    String? branch,
  );

  /// Initializes Git only while no turn runs in this project folder.
  Future<BridgeGitOutcome> initializeGit();

  /// Reads compared tree identities and whole-file changes against HEAD.
  Future<(String, List<BridgeFileChange>)> uncommittedChanges({
    required bool hideWhitespace,
  });

  /// Reads one uncommitted file against HEAD.
  Future<String> uncommittedPatch(String path, {required bool hideWhitespace});

  /// The stashed changes of the branch checked out, if any.
  Future<BridgeStash?> gitStash();

  /// The files and line counts of stash `id`, with the chosen whitespace setting.
  Future<List<BridgeFileChange>> stashChanges(
    String id, {
    required bool hideWhitespace,
  });

  /// One file's unified diff in the stash `id`.
  Future<String> stashPatch(
    String id,
    String path, {
    required bool hideWhitespace,
  });
  Future<BridgeGitOutcome> restoreStash(String id);
  Future<BridgeGitOutcome> discardStash(String id);

  /// Fetches the upstream's remote, or else origin or the only remote; the remote fetched, if
  /// there was one.
  Future<String?> fetch();

  Future<BridgeGitOutcome> push(BridgePushConfirmation? confirmed);
  Future<BridgeGitOutcome> pull();

  /// The pushed branch's pull request plan on GitHub, through the host's gh; `base` is the
  /// recorded branch it was created from, if any.
  Future<BridgePullRequestOutcome> pullRequestPlan({
    String? target,
    String? base,
  });

  /// Creates the reviewed pull request after rechecking its plan.
  Future<BridgePullRequestOutcome> createPullRequest(
    BridgePullRequestDraft draft,
  );

  /// The branch's latest pull request on GitHub, open ones first.
  Future<BridgePullRequestOutcome> pullRequest();

  /// The branches of `target` on GitHub, to choose a base from.
  Future<BridgePullRequestOutcome> pullRequestBases(String host, String target);

  /// The branches of `target` on GitHub whose names contain `query`.
  Future<BridgePullRequestOutcome> pullRequestBaseSearch(
    String host,
    String target,
    String query,
  );

  /// The title and description the Chat's `model` (`provider/id`) writes about the commits
  /// from `mergeBase` to `headCommit`; a later request or [stopPullRequestText] stops it.
  Future<BridgePullRequestText> writePullRequestText({
    required String model,
    required String mergeBase,
    required String headCommit,
  });

  /// As [writePullRequestText], before a commit and push: about the commits the base branch
  /// `base` lacks, as far as Git has it, and the files at `paths` as the commit plan's
  /// `snapshot` holds them; none when there are neither.
  Future<BridgePullRequestText?> writePullRequestDraftText({
    required String model,
    String? base,
    required String snapshot,
    required List<String> paths,
  });

  /// The branch's target, base and any open pull request before it is pushed, for "Commit,
  /// push & create PR".
  Future<BridgePullRequestOutcome> pullRequestPrepared({
    String? target,
    String? base,
  });

  /// Stops the pull request text being written.
  Future<void> stopPullRequestText();

  /// Git's registered worktrees of the repository, the main checkout first.
  Future<List<BridgeWorktree>> worktrees();

  /// What removing the worktree at `path` would lose, or what keeps it.
  Future<BridgeWorktreeRemoval> worktreeRemoval(String path);

  /// Removes the `reviewed` worktree once a recheck finds it unchanged and removable.
  Future<BridgeWorktreeOutcome> removeWorktree(BridgeWorktreeRemoval reviewed);

  Future<List<BridgeModel>> models();

  /// The selected model as `provider/id`.
  Future<String?> currentModel();
  Future<void> setModel(BridgeModel model);
}

/// A session in the embedded Meta-Harness core, reached through the Rust bridge.
class RustSessionBackend implements SessionBackend {
  RustSessionBackend(this._session);

  final AgentSession _session;

  @override
  Stream<BridgeEvent> events() => _session.events();
  @override
  Future<void> confirmApplied(int applied) =>
      _session.confirmApplied(applied: applied);
  @override
  Future<void> send(String message, {required bool followUp}) =>
      _session.send(message: message, followUp: followUp);
  @override
  Future<BridgeQueue> clearQueue() => _session.clearQueue();
  @override
  Future<void> abort() => _session.abort();
  @override
  Future<void> answerText(String request, String text) =>
      _session.answerText(request: request, text: text);
  @override
  Future<void> answerConfirm(String request, bool confirmed) =>
      _session.answerConfirm(request: request, confirmed: confirmed);
  @override
  Future<void> dismiss(String request) => _session.dismiss(request: request);
  @override
  Future<bool> shutdown() => _session.shutdown();
  @override
  Future<bool> isInRepository() => _session.inRepository();
  @override
  Future<List<BridgeTurn>> turns() => _session.turns();
  @override
  Future<BridgeTurnChanges> turnChanges(
    String from,
    String to, {
    required bool hideWhitespace,
  }) =>
      _session.turnChanges(from: from, to: to, hideWhitespace: hideWhitespace);
  @override
  Future<String> turnPatch(
    String from,
    String to,
    String path, {
    required bool hideWhitespace,
  }) => _session.turnPatch(
    from: from,
    to: to,
    path: path,
    hideWhitespace: hideWhitespace,
  );
  @override
  Future<BridgeHistory> history() => _session.history();
  @override
  Future<BridgeStatistics> statistics() => _session.statistics();
  @override
  Future<BridgeContext?> context() => _session.context();
  @override
  Future<BridgeTranscript> transcript(String? before) =>
      _session.transcript(before: before, limit: chatPage);
  @override
  Future<void> branch(String entry) => _session.branch(entry: entry);
  @override
  Future<BridgeRevertPlan> revertPlan(String turn) =>
      _session.revertPlan(turn: turn);
  @override
  Future<void> revertToTurn(
    String turn,
    BridgeRevertConfirmation confirmation,
  ) => _session.revertToTurn(turn: turn, confirmation: confirmation);
  @override
  Future<List<BridgeCheckpointSession>> checkpointSessions() =>
      _session.checkpointSessions();
  @override
  Future<void> removeAllCheckpoints() => _session.removeAllCheckpoints();
  @override
  Future<String?> sessionFile() => _session.sessionFile();
  @override
  Future<int> storedTurns(String path) async =>
      (await _session.storedTurns(path: path)).toInt();
  @override
  Future<void> deleteSession(String path) => _session.deleteSession(path: path);
  @override
  Future<String> exportRecord({
    required String path,
    required String title,
    required String host,
    required String day,
  }) => _session.exportRecord(path: path, title: title, host: host, day: day);
  @override
  Future<List<BridgeStoredSession>> recentSessions() =>
      _session.recentSessions(limit: recentLimit);
  @override
  Future<String?> title() => _session.title();
  @override
  Future<bool> newSession() => _session.newSession();
  @override
  Future<bool> switchSession(String path) => _session.switchSession(path: path);
  @override
  Future<BridgeGitStatus?> gitStatus() => _session.gitStatus();
  @override
  Future<BridgeBranches> gitBranches() => _session.gitBranches();
  @override
  Future<BridgeGitOutcome> switchBranch(
    BridgeBranch branch, {
    required bool leaveChanges,
    required String? fromBranch,
    required String? replacingStash,
  }) => _session.switchBranch(
    name: branch.name,
    remote: branch.remote,
    leaveChanges: leaveChanges,
    fromBranch: fromBranch,
    replacingStash: replacingStash,
  );
  @override
  Future<BridgeGitOutcome> createBranch(String name) =>
      _session.createBranch(name: name);
  @override
  Future<BridgeCommitPlan> commitPlan() => _session.commitPlan();
  @override
  Future<BridgeGitOutcome> commitFiles(
    String snapshot,
    List<String> paths,
    String message,
    String? branch,
  ) => _session.commitFiles(
    snapshot: snapshot,
    paths: paths,
    message: message,
    branch: branch,
  );
  @override
  Future<BridgeGitOutcome> initializeGit() => _session.initializeGit();
  @override
  Future<(String, List<BridgeFileChange>)> uncommittedChanges({
    required bool hideWhitespace,
  }) => _session.uncommittedChanges(hideWhitespace: hideWhitespace);
  @override
  Future<String> uncommittedPatch(
    String path, {
    required bool hideWhitespace,
  }) => _session.uncommittedPatch(path: path, hideWhitespace: hideWhitespace);
  @override
  Future<BridgeStash?> gitStash() => _session.gitStash();
  @override
  Future<List<BridgeFileChange>> stashChanges(
    String id, {
    required bool hideWhitespace,
  }) => _session.stashChanges(id: id, hideWhitespace: hideWhitespace);
  @override
  Future<String> stashPatch(
    String id,
    String path, {
    required bool hideWhitespace,
  }) => _session.stashPatch(id: id, path: path, hideWhitespace: hideWhitespace);
  @override
  Future<BridgeGitOutcome> restoreStash(String id) =>
      _session.restoreStash(id: id);
  @override
  Future<BridgeGitOutcome> discardStash(String id) =>
      _session.discardStash(id: id);
  @override
  Future<String?> fetch() => _session.fetch();
  @override
  Future<BridgeGitOutcome> push(BridgePushConfirmation? confirmed) =>
      _session.push(confirmed: confirmed);
  @override
  Future<BridgeGitOutcome> pull() => _session.pull();
  @override
  Future<BridgePullRequestOutcome> pullRequestPlan({
    String? target,
    String? base,
  }) => _session.pullRequestPlan(target: target, base: base);
  @override
  Future<BridgePullRequestOutcome> createPullRequest(
    BridgePullRequestDraft draft,
  ) => _session.createPullRequest(draft: draft);
  @override
  Future<BridgePullRequestOutcome> pullRequest() => _session.pullRequest();
  @override
  Future<BridgePullRequestOutcome> pullRequestBases(
    String host,
    String target,
  ) => _session.pullRequestBases(host: host, target: target);
  @override
  Future<BridgePullRequestOutcome> pullRequestBaseSearch(
    String host,
    String target,
    String query,
  ) => _session.pullRequestBaseSearch(host: host, target: target, query: query);

  @override
  Future<BridgePullRequestText> writePullRequestText({
    required String model,
    required String mergeBase,
    required String headCommit,
  }) => _session.writePullRequestText(
    model: model,
    mergeBase: mergeBase,
    headCommit: headCommit,
  );

  @override
  Future<BridgePullRequestText?> writePullRequestDraftText({
    required String model,
    String? base,
    required String snapshot,
    required List<String> paths,
  }) => _session.writePullRequestDraftText(
    model: model,
    base: base,
    snapshot: snapshot,
    paths: paths,
  );

  @override
  Future<BridgePullRequestOutcome> pullRequestPrepared({
    String? target,
    String? base,
  }) => _session.pullRequestPrepared(target: target, base: base);

  @override
  Future<void> stopPullRequestText() => _session.stopPullRequestText();

  @override
  Future<List<BridgeWorktree>> worktrees() => _session.worktrees();

  @override
  Future<BridgeWorktreeRemoval> worktreeRemoval(String path) =>
      _session.worktreeRemoval(path: path);

  @override
  Future<BridgeWorktreeOutcome> removeWorktree(
    BridgeWorktreeRemoval reviewed,
  ) => _session.removeWorktree(reviewed: reviewed);
  @override
  Future<List<BridgeModel>> models() => _session.models();
  @override
  Future<String?> currentModel() => _session.currentModel();
  @override
  Future<void> setModel(BridgeModel model) =>
      _session.setModel(provider: model.provider, id: model.id);
}
