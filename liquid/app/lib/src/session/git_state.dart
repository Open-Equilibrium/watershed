import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/history.dart';
import 'backend.dart';
import 'branch_bases.dart';

/// How often Liquid fetches the upstream's remote while a session is open, as T3 Code does.
const fetchEvery = Duration(seconds: 15);

/// The wait before the next fetch after `failures` failed ones in a row: from 30 seconds,
/// doubling up to 15 minutes, as T3 Code's.
Duration fetchWait(int failures) => failures == 0
    ? fetchEvery
    : Duration(seconds: math.min(30 << math.min(failures - 1, 5), 15 * 60));

/// The checkout's Git state for the session header, the sidebar and the Diff tab, read again
/// after each fetch, turn and Git write.
class GitState {
  GitState(this._backend, {required this.notify, BranchBases? bases})
    : _bases = bases ?? BranchBases();

  final SessionBackend _backend;

  /// Where a branch Liquid creates records the branch it was created from.
  final BranchBases _bases;
  final void Function(BridgeLevel level, String text) notify;

  /// The branch and its counts; null outside Git or before the first read.
  final status = ValueNotifier<BridgeGitStatus?>(null);

  /// Whether a status read has completed, distinguishing startup from outside Git.
  final loaded = ValueNotifier(false);

  /// Successful reads, including equal summaries whose contents or agent locks changed.
  final refreshed = ValueNotifier(0);

  /// Whether a Git write is queued or running in this window.
  final writing = ValueNotifier(false);

  /// The explicit network operation and elapsed seconds shown by the split button.
  final progress = ValueNotifier<String?>(null);
  int _writes = 0;
  bool _initializing = false;

  /// Why Git could not be read, such as Git missing on the host.
  final problem = ValueNotifier<String?>(null);

  /// The stashed changes of the branch checked out.
  final stash = ValueNotifier<BridgeStash?>(null);

  Timer? _next;

  /// Counts [watch] and [stop], so a tick of an earlier watch never schedules another.
  int _watches = 0;
  bool _watching = false;
  int _failures = 0;
  DateTime? _fetched;
  String? _fetchedRemote;
  DateTime? get fetched =>
      _fetchedRemote == status.value?.remote ? _fetched : null;
  Future<void>? _fetching;
  String? _fetchRemote;
  bool _fetchRequested = false;

  /// Reads of the state left until the next fetch; the state is read every [fetchEvery]
  /// regardless.
  int _untilFetch = 1;

  /// Counts the reads started, so only the latest one sets the state.
  int _reads = 0;
  Future<void>? _latestRefresh;

  /// Whether a Git write of this window is running, so its own switch is not reported as one
  /// outside Liquid.
  bool _writing = false;
  Future<void> _pendingWrite = Future<void>.value();

  /// What a write of this window tells after its own notice: the stash of the branch it
  /// switched to.
  String? _arrived;

  /// Reads the state now and every [fetchEvery], fetching when due, until [stop].
  void watch() {
    if (_watching) return;
    _watching = true;
    final watch = ++_watches;
    _untilFetch = 1;
    unawaited(refresh());
    _next = Timer(fetchEvery, () => _tick(watch));
  }

  void stop() {
    _watching = false;
    _watches++;
    _next?.cancel();
  }

  Future<void> _tick(int watch) async {
    final upstream = status.value?.upstream;
    if (_fetching == null &&
        progress.value == null &&
        upstream != null &&
        --_untilFetch <= 0) {
      unawaited(_fetch(watch, status.value!.remote!));
    }
    await refresh();
    if (watch == _watches) _next = Timer(fetchEvery, () => _tick(watch));
  }

  /// Shares a fetch for the same remote; a changed destination waits its turn.
  Future<void> _fetch(
    int watch,
    String remote, {
    bool requested = false,
  }) async {
    final pending = _fetching;
    if (pending != null) {
      if (_fetchRemote == remote) {
        _fetchRequested |= requested;
        return pending;
      }
      await pending;
      if (watch != _watches) return;
      final current = status.value?.remote;
      if (current != null) await _fetch(watch, current, requested: requested);
      return;
    }
    _fetchRemote = remote;
    _fetchRequested = requested;
    await (_fetching = _runFetch(watch, remote));
  }

  Future<void> _runFetch(int watch, String remote) async {
    try {
      final fetched = await _backend.fetch();
      if (watch != _watches) return;
      _failures = 0;
      if (fetched) {
        _fetched = DateTime.now();
        _fetchedRemote = remote;
      }
      await refresh();
    } on Object catch (error) {
      if (watch != _watches) return;
      // A streak of failures tells once.
      if (_failures++ == 0 || _fetchRequested) {
        final since = switch (_fetchedRemote == remote ? _fetched : null) {
          final at? =>
            ' The ahead and behind counts are from ${formatClockMs(at.millisecondsSinceEpoch)}.',
          null => '',
        };
        notify(
          _fetchRequested ? BridgeLevel.error : BridgeLevel.warning,
          '${fetchFailure(remote, '$error')}.$since',
        );
      }
    } finally {
      _fetching = null;
      _fetchRemote = null;
      _fetchRequested = false;
      if (watch == _watches) {
        _untilFetch = fetchWait(_failures).inSeconds ~/ fetchEvery.inSeconds;
      }
    }
  }

  /// A click bypasses the background retry wait, without overlapping another fetch.
  Future<void> fetchNow() async {
    final remote = status.value?.remote;
    if (remote == null) return;
    await _fetch(_watches, remote, requested: true);
  }

  Future<BridgeGitOutcome?> push({
    required String device,
    BridgePushConfirmation? confirmed,
  }) => _write(
    'push',
    () => _backend.push(confirmed),
    device: device,
    progressLabel: status.value?.remote == null
        ? 'Pushing'
        : 'Pushing to ${status.value!.remote}',
    done: (_, outcome) {
      final transfer = outcome.transfer!;
      return transfer.firstPush || transfer.gone
          ? 'Pushed ${transfer.branch} to ${transfer.remote}; it now tracks ${transfer.upstream}'
          : 'Pushed ${plural(transfer.ahead, 'commit')} to ${transfer.upstream}';
    },
  );

  Future<BridgeGitOutcome?> pull({required String device}) => _write(
    'pull',
    _backend.pull,
    device: device,
    progressLabel: status.value?.remote == null
        ? 'Pulling'
        : 'Pulling from ${status.value!.remote}',
    done: (_, outcome) {
      final transfer = outcome.transfer!;
      return 'Pulled ${plural(transfer.behind, 'commit')} from ${transfer.upstream}';
    },
  );

  /// Reads the branch, its counts and its stash again.
  Future<void> refresh() => _latestRefresh = _refresh(++_reads);

  Future<void> _refresh(int read) async {
    final before = status.value?.branch;
    try {
      final now = await _backend.gitStatus();
      final stashed = now?.stash;
      // A later read, such as one after a write, wins.
      if (read != _reads) return;
      problem.value = null;
      if (!_sameStatus(status.value, now)) status.value = now;
      loaded.value = true;
      // The same stash on the same tree keeps its value, so the Diff tab keeps its place.
      if (stashed?.id != stash.value?.id ||
          stashed?.current != stash.value?.current) {
        stash.value = stashed;
      }
      refreshed.value++;
      final after = now?.branch;
      if (before == null || after == null || after == before) return;
      if (!_writing) {
        notify(
          BridgeLevel.info,
          'The checkout switched to $after outside Liquid.',
        );
      }
      if (stashed != null) {
        final text =
            '$after has stashed changes from ${formatClockMs(stashed.createdMs)}; the Diff '
            'tab shows them.';
        // A switch of this window tells first that it switched.
        if (_writing) {
          _arrived = text;
        } else {
          notify(BridgeLevel.info, text);
        }
      }
    } on Object catch (error) {
      if (read == _reads) problem.value = '$error';
    }
  }

  /// Reads again and waits until the most recent read has published, which a poll started
  /// meanwhile can overtake; a write settles on it, and a choice reads what it decides on.
  Future<void> refreshSettled() async {
    var pending = refresh();
    while (true) {
      await pending;
      final latest = _latestRefresh!;
      if (identical(pending, latest)) return;
      pending = latest;
    }
  }

  /// The checkout's branches and the agents running there.
  Future<BridgeBranches> branches() => _backend.gitBranches();

  /// Switches to `branch`, leaving uncommitted changes in the stash of the branch left or
  /// bringing them along; returns how it ended, null after an error it reported.
  Future<BridgeGitOutcome?> switchTo(
    BridgeBranch branch, {
    required bool leaveChanges,
    required String? fromBranch,
    required String? replacingStash,
  }) {
    final left = status.value;
    return _write(
      'switch',
      () => _backend.switchBranch(
        branch,
        leaveChanges: leaveChanges,
        fromBranch: fromBranch,
        replacingStash: replacingStash,
      ),
      done: (now, _) {
        final name = now?.branch ?? branch.shown;
        final changes = left?.changed ?? 0;
        final count = changes == 1 ? '1 change is' : '$changes changes are';
        return leaveChanges && changes > 0 && left?.branch != null
            ? 'Switched to $name. Your $count stashed on ${left!.branch}.'
            : 'Switched to $name.';
      },
    );
  }

  /// Creates the branch `name` at the current commit and switches to it.
  Future<BridgeGitOutcome?> create(String name) async {
    // Read when the write starts, after the writes queued before it.
    BridgeGitStatus? from;
    final outcome = await _write('create the branch', () {
      from = status.value;
      return _backend.createBranch(name);
    }, done: (_, _) => 'Created and switched to $name.');
    // Git created the branch, also when a hook failed after.
    if ({
      BridgeGitOutcomeKind.done,
      BridgeGitOutcomeKind.hookFailed,
    }.contains(outcome?.kind)) {
      await _recordBase(from, name);
    }
    return outcome;
  }

  /// Remembers the branch `from` was on as the base of the new branch `name`.
  Future<void> _recordBase(BridgeGitStatus? from, String name) async {
    final base = from?.rawBranch;
    if (from == null || base == null) return;
    try {
      await _bases.record(from.repository, name, base);
    } on Object catch (error) {
      notify(
        BridgeLevel.warning,
        'Could not remember that $name was created from ${from.branch}: $error',
      );
    }
  }

  Future<BridgeCommitPlan>? _planRead;
  bool _openingCommit = false;

  /// Shares only an in-flight capture; each later read captures the checkout again.
  Future<BridgeCommitPlan> commitPlan() => _planRead ??= _readPlan();

  Future<BridgeCommitPlan> _readPlan() async {
    try {
      return await _backend.commitPlan();
    } finally {
      _planRead = null;
    }
  }

  /// Opens at most one commit dialog, including its initial status and plan reads.
  Future<void> openCommitPlan(
    Future<void> Function(BridgeCommitPlan) show,
  ) async {
    if (_openingCommit) return;
    _openingCommit = true;
    try {
      await refresh();
      final plan = await commitPlan();
      await show(plan);
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not read uncommitted changes: $error');
    } finally {
      _openingCommit = false;
    }
  }

  /// Commits the selected files only while they match the dialog's snapshot.
  Future<BridgeGitOutcome?> commit(
    List<String> paths,
    String message, {
    required String snapshot,
    String? branch,
  }) async {
    BridgeGitStatus? from;
    final outcome = await _write(
      'commit',
      () {
        from = status.value;
        return _backend.commitFiles(snapshot, paths, message, branch);
      },
      done: (now, outcome) {
        final commit = outcome.commit!;
        final text =
            'Committed ${plural(commit.count, 'file')} on ${commit.branch}';
        return commit.subject == null ? '$text.' : '$text: “${commit.subject}”';
      },
    );
    // The branch exists once Git created it, even when its commit failed.
    if (branch != null &&
        {
          BridgeGitOutcomeKind.done,
          BridgeGitOutcomeKind.hookChanged,
          BridgeGitOutcomeKind.commitBranchFailed,
        }.contains(outcome?.kind)) {
      await _recordBase(from, branch);
    }
    return outcome;
  }

  /// Creates Git explicitly, once at a time, after status confirmed this folder is outside it.
  Future<void> initialize() async {
    if (_initializing ||
        !loaded.value ||
        status.value != null ||
        writing.value) {
      return;
    }
    _initializing = true;
    try {
      await _write(
        'initialize Git',
        _backend.initializeGit,
        done: (_, _) =>
            'Initialized Git. Checkpoints start with the next turn.',
      );
    } finally {
      _initializing = false;
    }
  }

  /// Moves the stashed changes `stashed` back into the checkout.
  Future<void> restoreStash(BridgeStash stashed) async {
    final files = stashed.files.length;
    final branch = status.value?.branch;
    await _write(
      'restore',
      () => _backend.restoreStash(stashed.id),
      done: (_, _) => 'Restored ${plural(files, 'stashed file')} on $branch',
    );
  }

  /// Deletes the stashed changes `stashed`.
  Future<void> discardStash(BridgeStash stashed) => _write(
    'discard',
    () => _backend.discardStash(stashed.id),
    done: (_, _) => 'Discarded the stashed changes.',
  );

  /// Runs the Git write `run`, reports how it ended, `action` naming it, and reads the state
  /// again; returns the outcome, null after an error it reported.
  Future<BridgeGitOutcome?> _write(
    String action,
    Future<BridgeGitOutcome> Function() run, {
    String? device,
    String? progressLabel,
    required String Function(BridgeGitStatus? now, BridgeGitOutcome outcome)
    done,
  }) async {
    _writes++;
    writing.value = true;
    final previous = _pendingWrite;
    final finished = Completer<void>();
    _pendingWrite = finished.future;
    await previous;
    _writing = true;
    Timer? elapsed;
    if (progressLabel != null) {
      var seconds = 0;
      void tick() => progress.value = '$progressLabel… $seconds s';
      tick();
      elapsed = Timer.periodic(const Duration(seconds: 1), (timer) {
        seconds = timer.tick;
        tick();
      });
    }
    try {
      final BridgeGitOutcome outcome;
      try {
        outcome = await run();
      } on Object catch (error) {
        notify(
          BridgeLevel.error,
          'Could not $action: $error${nothingTransferred(action)}',
        );
        // Git may have changed something before it failed.
        await refreshSettled();
        return null;
      }
      if (outcome.transfer case final transfer?
          when transfer.remote != '.' &&
              {
                BridgeGitOutcomeKind.done,
                BridgeGitOutcomeKind.pushDefault,
                BridgeGitOutcomeKind.diverged,
                BridgeGitOutcomeKind.transferFailed,
              }.contains(outcome.kind)) {
        _fetched = DateTime.now();
        _fetchedRemote = transfer.remote;
      }
      await refreshSettled();
      final now = problem.value == null ? status.value : null;
      final files = outcome.files.join(', ');
      switch (outcome.kind) {
        case BridgeGitOutcomeKind.pushDefault:
          // The caller opens the approved confirmation dialog; no push ran.
          break;
        case BridgeGitOutcomeKind.diverged:
          final transfer = outcome.transfer!;
          final explanation = transfer.ahead > 0
              ? divergedHint(
                  transfer.branch,
                  transfer.upstream,
                  ahead: transfer.ahead,
                  behind: transfer.behind,
                )
              : behindHint(transfer.branch, transfer.upstream, transfer.behind);
          notify(
            BridgeLevel.error,
            'Could not $action: $explanation${nothingTransferred(action)}${outcome.detail.isEmpty ? '' : ' “${outcome.detail}”'}',
          );
        case BridgeGitOutcomeKind.signInFailed:
          notify(
            BridgeLevel.error,
            'Could not $action: Git could not sign in to ${outcome.transfer!.remote}. Liquid uses the Git login of $device; sign in there with git or gh, then $action again. Nothing was ${action == 'push' ? 'pushed' : 'pulled'}. “${outcome.detail}”',
          );
        case BridgeGitOutcomeKind.fetchFailed:
          notify(
            BridgeLevel.error,
            'Could not $action: ${fetchFailure(outcome.transfer!.remote, outcome.detail)}.${nothingTransferred(action)}',
          );
        case BridgeGitOutcomeKind.preflightFailed:
          notify(
            BridgeLevel.error,
            'Could not $action: ${outcome.detail}${nothingTransferred(action)}',
          );
        case BridgeGitOutcomeKind.transferFailed:
          notify(
            BridgeLevel.error,
            'Could not $action: Git failed. “${outcome.detail}” Git or its hooks may have made changes.',
          );
        case BridgeGitOutcomeKind.done:
          final text = done(now, outcome);
          notify(
            outcome.detail.isEmpty ? BridgeLevel.info : BridgeLevel.warning,
            outcome.detail.isEmpty ? text : '$text ${outcome.detail}',
          );
        case BridgeGitOutcomeKind.hookFailed:
          notify(
            BridgeLevel.warning,
            '${done(now, outcome)} A hook failed after: ${outcome.detail}',
          );
        case BridgeGitOutcomeKind.mergeInProgress:
          notify(
            progressLabel == null ? BridgeLevel.warning : BridgeLevel.error,
            progressLabel == null
                ? mergeHint(status.value)
                : 'Could not $action: ${mergeHint(status.value)}${nothingTransferred(action)}',
          );
        case BridgeGitOutcomeKind.commitBranchFailed:
          notify(
            BridgeLevel.error,
            'Created and switched to ${outcome.commit!.branch}; nothing was committed. ${outcome.detail}',
          );
        case BridgeGitOutcomeKind.commitFailed:
          notify(
            BridgeLevel.error,
            'Could not commit: nothing was committed. ${outcome.detail}',
          );
        case BridgeGitOutcomeKind.hookChanged:
          notify(
            BridgeLevel.warning,
            '${done(now, outcome)} A hook also changed $files, which ${outcome.files.length == 1 ? 'stays' : 'stay'} uncommitted.',
          );
        case BridgeGitOutcomeKind.agentRunning:
          notify(
            progressLabel == null ? BridgeLevel.warning : BridgeLevel.error,
            'Could not $action: ${whoRuns(outcome.running)}. ${stopFirst(outcome.running)}.${nothingTransferred(action)}',
          );
        case BridgeGitOutcomeKind.conflicts:
          notify(
            BridgeLevel.warning,
            'Restored with conflicts in $files; Git kept the stash. Resolve them in an '
            'editor, then discard the stash.',
          );
        case BridgeGitOutcomeKind.uncommitted:
          final changed = status.value?.changed ?? 0;
          final changes = changed == 1
              ? '1 uncommitted change'
              : '$changed uncommitted changes';
          notify(
            BridgeLevel.warning,
            'Could not $action: the checkout has $changes. Commit or discard them, then '
            'restore; the stash stays.',
          );
        case BridgeGitOutcomeKind.branchMoved:
          notify(
            BridgeLevel.warning,
            'Could not restore: the branch has new commits since these changes were left. '
            'Restore them with Git; the stash stays.',
          );
        case BridgeGitOutcomeKind.submoduleChanged:
          notify(
            BridgeLevel.warning,
            'Could not $action: Git cannot stash the new commit of $files. Commit it, or '
            'bring the changes along.',
          );
        // A switch Git refuses asks in a dialog what to do.
        case BridgeGitOutcomeKind.wouldOverwrite when action == 'restore':
          notify(
            BridgeLevel.warning,
            'Could not restore: Git would overwrite your uncommitted changes in $files. The '
            'stash stays; commit or discard those changes, then restore.',
          );
        case BridgeGitOutcomeKind.wouldOverwrite when action == 'pull':
          notify(
            BridgeLevel.error,
            'Could not pull: Git would overwrite your uncommitted changes in $files. Commit them first. Nothing was pulled.',
          );
        case BridgeGitOutcomeKind.wouldOverwrite:
      }
      if (_arrived case final arrived?) notify(BridgeLevel.info, arrived);
      return outcome;
    } finally {
      elapsed?.cancel();
      progress.value = null;
      _writing = false;
      writing.value = --_writes > 0;
      _arrived = null;
      finished.complete();
    }
  }
}

/// Which agents run `where`, such as “Fix flaky sync test” is running in this checkout.
String whoRuns(
  List<BridgeRunningAgent> running, {
  String where = 'in this checkout',
}) {
  final first = running.firstOrNull;
  final name = switch ((first?.own, first?.title)) {
    (true, _) => 'This session’s agent',
    (_, final title?) => '“$title”',
    _ => 'Another agent session',
  };
  final more = running.length - 1;
  if (more > 0) return '$name and $more more are running $where';
  return first?.own == true ? '$name is running' : '$name is running $where';
}

/// What to do before trying again while `running` run.
String stopFirst(List<BridgeRunningAgent> running) => running.length > 1
    ? 'Stop them or wait until they finish'
    : 'Stop it or wait until it finishes';

/// Why Git refuses `name` as a branch name, for its most common rules; null when none of them
/// applies, which leaves the decision to Git.
String? branchNameProblem(String name) {
  const notIn = {'..': '“..”', '@{': '“@{”', '//': '“//”'};
  for (final MapEntry(:key, :value) in notIn.entries) {
    if (name.contains(key)) {
      return 'Git does not allow $value in a branch name.';
    }
  }
  if (name.contains(RegExp(r'\s'))) {
    return 'Git does not allow spaces in a branch name.';
  }
  if (RegExp(r'[~^:?*\[\\]').firstMatch(name) case final bad?) {
    return 'Git does not allow “${bad[0]}” in a branch name.';
  }
  for (final start in ['-', '/', '.']) {
    if (name.startsWith(start)) {
      return 'Git does not allow a branch name to start with “$start”.';
    }
  }
  for (final end in ['.lock', '/', '.']) {
    if (name.endsWith(end)) {
      return 'Git does not allow a branch name to end with “$end”.';
    }
  }
  return null;
}

/// Why the current merge prevents an explicit commit.
String mergeHint(BridgeGitStatus? status) {
  final conflicts = status?.conflicts ?? const <String>[];
  if (conflicts.isEmpty) {
    return 'A merge is in progress. Finish it in a terminal; Liquid commits no merges.';
  }
  return '${conflicts.length} ${conflicts.length == 1 ? 'file has' : 'files have'} conflicts: ${conflicts.join(', ')}. Resolve them in an editor or a terminal; Liquid resolves no conflicts.';
}

/// How to commit while HEAD names a commit rather than a branch.
const detachedCommitHint =
    'Detached HEAD: create a branch before committing or pushing.';

/// Generated bridge lists compare by identity; compare only status values before publishing.
bool _sameStatus(BridgeGitStatus? a, BridgeGitStatus? b) =>
    identical(a, b) ||
    (a != null &&
        b != null &&
        a.branch == b.branch &&
        a.rawBranch == b.rawBranch &&
        a.repository == b.repository &&
        a.commit == b.commit &&
        a.upstream == b.upstream &&
        a.remote == b.remote &&
        a.hasRemotes == b.hasRemotes &&
        a.gone == b.gone &&
        a.ahead == b.ahead &&
        a.behind == b.behind &&
        a.changed == b.changed &&
        a.defaultBranch == b.defaultBranch &&
        a.merging == b.merging &&
        listEquals(a.conflicts, b.conflicts) &&
        a.stash?.id == b.stash?.id &&
        a.stash?.current == b.stash?.current);

/// The Git action messages shared by the split button and transfer notices.
const noRemoteHint =
    'No remote: commits stay in this repository. Push needs a remote added in Git.';
const noUpstreamRemoteHint = 'No upstream: set one in Git to push this branch.';
const noUpstreamHint = 'This branch has no upstream. Push it first.';

String fetchFailure(String remote, String detail) =>
    'Could not fetch from $remote: “$detail”';

String behindHint(String branch, String upstream, int behind) =>
    '$upstream has ${plural(behind, 'commit')} that $branch lacks. Pull them, then push.';

String divergedHint(
  String branch,
  String upstream, {
  int? ahead,
  int? behind,
}) =>
    '$branch and $upstream have diverged${ahead == null ? '' : ', ${plural(ahead, 'commit')} here and $behind there'}. Merge or rebase in a terminal, then push.';

String nothingTransferred(String action) => switch (action) {
  'push' => ' Nothing was pushed.',
  'pull' => ' Nothing was pulled.',
  _ => '',
};
