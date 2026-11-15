import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/git.dart';
import 'backend.dart';
import 'branch_bases.dart';
import 'git_state.dart';

/// The branch's pull request on GitHub for the PR chip, its summary, the post-push offer and
/// the create dialog (approved pull request mockups §1–5). Status is read when the branch or
/// its upstream changes, when the summary opens, when the window regains focus and on Refresh;
/// never by polling.
class PullRequests {
  PullRequests(this._backend, this._git, this._bases) {
    _git.status.addListener(_statusChanged);
    _statusChanged();
  }

  final SessionBackend _backend;
  final GitState _git;
  final BranchBases _bases;

  /// The branch's latest pull request as last read; kept while a refresh fails.
  final current = ValueNotifier<BridgePullRequest?>(null);

  /// When GitHub last answered for this branch.
  final updated = ValueNotifier<DateTime?>(null);

  /// Why the last refresh failed, which marks [current] stale.
  final failure = ValueNotifier<BridgePullRequestOutcome?>(null);

  /// The completed push the input area offers a pull request for, until dismissed or one
  /// exists.
  final offer = ValueNotifier<BridgeGitTransfer?>(null);

  /// The branch and upstream [current] belongs to.
  String? _key;
  int _reads = 0;

  /// The branch's remote is not on GitHub, as last read.
  bool _notGitHub = false;
  bool _disposed = false;

  String? get _statusKey => switch (_git.status.value) {
    final status? when status.branch != null =>
      '${status.repository}\n${status.rawBranch}\n${status.upstream}',
    _ => null,
  };

  void _statusChanged() {
    final key = _statusKey;
    if (key == _key) return;
    _key = key;
    current.value = null;
    updated.value = null;
    failure.value = null;
    _notGitHub = false;
    if (offer.value?.confirmation.branch != _git.status.value?.rawBranch) {
      offer.value = null;
    }
    unawaited(refresh());
  }

  Future<void>? _refreshing;
  bool _again = false;

  /// Reads the branch's pull request again; a failure keeps the last one, marked stale. A
  /// read asked for meanwhile runs once after the current one instead of queueing.
  Future<void> refresh() {
    if (_refreshing case final refreshing?) {
      _again = true;
      return refreshing;
    }
    return _refreshing = _refresh();
  }

  Future<void> _refresh() async {
    try {
      do {
        _again = false;
        await _readOnce();
      } while (_again && !_disposed);
    } finally {
      _refreshing = null;
    }
  }

  Future<void> _readOnce() async {
    final key = _statusKey;
    final read = ++_reads;
    // Without a pushed upstream there is no pull request to read.
    if (key == null || _git.status.value?.upstream == null) return;
    BridgePullRequestOutcome outcome;
    try {
      outcome = await _backend.pullRequest();
    } on Object {
      outcome = unansweredOutcome;
    }
    if (_disposed || read != _reads || key != _statusKey) return;
    switch (outcome.kind) {
      case BridgePullRequestOutcomeKind.done:
        _notGitHub = false;
        _show(outcome.pullRequest);
      case BridgePullRequestOutcomeKind.notGitHub:
        _notGitHub = true;
        current.value = null;
        updated.value = null;
        failure.value = null;
        offer.value = null;
      default:
        failure.value = outcome;
    }
  }

  void _show(BridgePullRequest? pull) {
    current.value = pull;
    updated.value = DateTime.now();
    failure.value = null;
    if (isOpen(pull)) offer.value = null;
  }

  /// After a push, offers a pull request unless one is open or the remote is not on GitHub.
  Future<void> pushed(BridgeGitTransfer transfer) async {
    await refresh();
    if (!_disposed &&
        !_notGitHub &&
        !isOpen(current.value) &&
        _git.status.value?.rawBranch == transfer.confirmation.branch) {
      offer.value = transfer;
    }
  }

  void dismissOffer() => offer.value = null;

  /// The branch the checked-out branch was created from, as Liquid recorded it.
  Future<String?> recordedBase() async {
    final status = _git.status.value;
    final branch = status?.rawBranch;
    if (status == null || branch == null) return null;
    return _bases.baseOf(status.repository, branch);
  }

  /// The create dialog's plan; without a chosen base, the recorded one.
  Future<BridgePullRequestOutcome> plan({String? target, String? base}) async =>
      _backend.pullRequestPlan(
        target: target,
        base: base ?? await recordedBase(),
      );

  /// The target's branches to choose a base from.
  Future<BridgePullRequestOutcome> bases(String host, String target) =>
      _backend.pullRequestBases(host, target);

  /// The target's branches whose names contain `query`, as GitHub finds them.
  Future<BridgePullRequestOutcome> searchBases(
    String host,
    String target,
    String query,
  ) => _backend.pullRequestBaseSearch(host, target, query);

  /// The title and description the Chat's `model` writes for `plan`'s commits beyond its
  /// base; a later request or [stopWriting] stops it.
  Future<BridgePullRequestText> write(
    String model,
    BridgePullRequestPlan plan,
  ) => _backend.writePullRequestText(
    model: model,
    mergeBase: plan.comparison!.mergeBase,
    headCommit: plan.identity.headCommit,
  );

  /// The title and description the Chat's `model` writes before a commit and push: about
  /// the commits the base branch `base` lacks and the files at `paths` as the commit plan's
  /// `snapshot` holds them; none when there are neither.
  Future<BridgePullRequestText?> writeDraft(
    String model, {
    String? base,
    required String snapshot,
    required List<String> paths,
  }) => _backend.writePullRequestDraftText(
    model: model,
    base: base,
    snapshot: snapshot,
    paths: paths,
  );

  Future<void> stopWriting() => _backend.stopPullRequestText();

  /// "Commit, push & create PR"'s target, base and open pull request before any write; without
  /// a chosen base, the recorded one.
  Future<BridgePullRequestOutcome> prepare({
    String? target,
    String? base,
  }) async => _backend.pullRequestPrepared(
    target: target,
    base: base ?? await recordedBase(),
  );

  /// Creates the reviewed pull request; a created or already open one becomes [current] while
  /// the same branch and upstream are checked out.
  Future<BridgePullRequestOutcome> create(BridgePullRequestDraft draft) async {
    final key = _statusKey;
    final outcome = await _backend.createPullRequest(draft);
    if (!_disposed &&
        key == _statusKey &&
        (outcome.kind == BridgePullRequestOutcomeKind.done ||
            outcome.kind == BridgePullRequestOutcomeKind.exists)) {
      // A read that began before creating would show the branch without it.
      _reads++;
      _show(outcome.pullRequest);
    }
    return outcome;
  }

  void dispose() {
    _disposed = true;
    _git.status.removeListener(_statusChanged);
    current.dispose();
    updated.dispose();
    failure.dispose();
    offer.dispose();
  }
}

/// An outcome of `kind` that names nothing.
BridgePullRequestOutcome outcomeOf(BridgePullRequestOutcomeKind kind) =>
    BridgePullRequestOutcome(
      kind: kind,
      bases: const [],
      moreBases: false,
      host: '',
      remote: '',
      branch: '',
      upstream: '',
      repository: '',
      ahead: 0,
      behind: 0,
      first: false,
      page: '',
      detail: '',
    );

/// A plan, creation or read the session did not answer.
final unansweredOutcome = outcomeOf(BridgePullRequestOutcomeKind.failed);

bool isOpen(BridgePullRequest? pull) =>
    pull?.state == BridgePullRequestState.open ||
    pull?.state == BridgePullRequestState.draft;

/// Why a pull request could not be planned, created or read, in plain words; null when it
/// was done. `device` names the host whose gh and Git login are used.
String? pullRequestProblem(
  BridgePullRequestOutcome outcome, {
  required String? device,
}) {
  final on = device == null ? 'on the host' : 'on $device';
  final detail = outcome.detail.isEmpty ? '' : ' “${outcome.detail}”';
  return switch (outcome.kind) {
    BridgePullRequestOutcomeKind.done ||
    BridgePullRequestOutcomeKind.changed ||
    BridgePullRequestOutcomeKind.exists => null,
    BridgePullRequestOutcomeKind.noBranch => 'There is no branch to publish. Create a branch before creating a pull request.',
    BridgePullRequestOutcomeKind.noCommit => 'No committed change can be published. Commit changes first; a checkpoint is not a branch commit.',
    BridgePullRequestOutcomeKind.merging => 'A merge or rebase is in progress. Finish or cancel it in Git before publishing.',
    BridgePullRequestOutcomeKind.noRemote =>
      'This repository has no remote. Add a GitHub remote with Git $on before creating a pull request.',
    BridgePullRequestOutcomeKind.ambiguousRemote ||
    BridgePullRequestOutcomeKind.destinations => 'The fetch and push destinations differ or several remotes could supply this pull request. Resolve the remote in Git first.',
    BridgePullRequestOutcomeKind.notGitHub when outcome.host.isEmpty =>
      '${outcome.remote} is not on GitHub. Pull requests through gh need a GitHub remote.',
    BridgePullRequestOutcomeKind.notGitHub =>
      '${outcome.remote} is on ${outcome.host}, which gh $on has no login for. Pull requests through gh need GitHub; for GitHub Enterprise, sign in with gh auth login --hostname ${outcome.host}, then check again. Otherwise open this repository in its host\'s browser to create the host\'s merge request.',
    BridgePullRequestOutcomeKind.needsPush when outcome.first =>
      '${outcome.branch} is not on ${outcome.remote} yet. Push it before creating a pull request.',
    BridgePullRequestOutcomeKind.needsPush =>
      '${outcome.branch} has ${outcome.ahead == 1 ? '1 commit that is' : '${outcome.ahead} commits that are'} not on ${outcome.remote}. GitHub cannot include them yet.',
    BridgePullRequestOutcomeKind.behind =>
      '${outcome.upstream} has commits ${outcome.branch} lacks. Pull first if it can fast-forward. Otherwise merge or rebase in a terminal, then push. Liquid does not force-push.',
    BridgePullRequestOutcomeKind.headMoved =>
      'GitHub\'s ${outcome.branch} is at another commit than this checkout. Fetch, then pull or push before creating a pull request.',
    BridgePullRequestOutcomeKind.ghMissing =>
      'Could not run gh${outcome.detail.isEmpty ? '' : ': “${outcome.detail}”'}. Install GitHub CLI $on, then check again. Git commits still work.',
    BridgePullRequestOutcomeKind.ghOutdated =>
      'gh $on is too old for Liquid.$detail Update GitHub CLI $on, then check again.',
    BridgePullRequestOutcomeKind.signIn =>
      'Sign in $on with gh auth login${outcome.host == 'github.com' ? '' : ' --hostname ${outcome.host}'}, then check again. Liquid does not ask for the credential.',
    BridgePullRequestOutcomeKind.noAccess =>
      'gh cannot access ${outcome.repository} $on. Check the host\'s login and repository access, then check again.$detail',
    BridgePullRequestOutcomeKind.unreachable =>
      'Could not reach ${outcome.host}.$detail Try again when it is available.',
    BridgePullRequestOutcomeKind.failed => 'Could not read GitHub $on.$detail',
    BridgePullRequestOutcomeKind.notCreated =>
      'GitHub refused the pull request; none was created.$detail',
    BridgePullRequestOutcomeKind.unknown =>
      'GitHub did not return a result. Check for an existing pull request before trying again.$detail',
  };
}
