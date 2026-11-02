import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/history.dart';
import 'backend.dart';

/// How often Liquid fetches the upstream's remote while a session is open, as T3 Code does
/// (approved Git mockups).
const fetchEvery = Duration(seconds: 15);

/// The wait before the next fetch after `failures` failed ones in a row: from 30 seconds,
/// doubling up to 15 minutes, as T3 Code's.
Duration fetchWait(int failures) => failures == 0
    ? fetchEvery
    : Duration(seconds: math.min(30 << math.min(failures - 1, 5), 15 * 60));

/// The checkout's Git state for the session header, the sidebar and the Diff tab, read again
/// after each fetch, turn and Git write.
class GitState {
  GitState(this._backend, {required this.notify});

  final SessionBackend _backend;
  final void Function(BridgeLevel level, String text) notify;

  /// The branch and its counts; null outside Git or before the first read.
  final status = ValueNotifier<BridgeGitStatus?>(null);

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

  /// Reads of the state left until the next fetch; the state is read every [fetchEvery]
  /// regardless.
  int _untilFetch = 1;

  /// Counts the reads started, so only the latest one sets the state.
  int _reads = 0;

  /// Whether a Git write of this window is running, so its own switch is not reported as one
  /// outside Liquid.
  bool _writing = false;

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
    if (upstream != null && --_untilFetch <= 0) {
      try {
        await _backend.fetch();
        _failures = 0;
        _fetched = DateTime.now();
      } on Object catch (error) {
        // A streak of failures tells once.
        if (_failures++ == 0) {
          final remote = upstream.split('/').first;
          final since = switch (_fetched) {
            final at? =>
              ' The ahead and behind counts are from ${formatClockMs(at.millisecondsSinceEpoch)}.',
            null => '',
          };
          notify(
            BridgeLevel.warning,
            'Could not fetch from $remote: $error.$since',
          );
        }
      }
      _untilFetch = fetchWait(_failures).inSeconds ~/ fetchEvery.inSeconds;
    }
    await refresh();
    if (watch == _watches) _next = Timer(fetchEvery, () => _tick(watch));
  }

  /// Reads the branch, its counts and its stash again.
  Future<void> refresh() async {
    final read = ++_reads;
    final before = status.value?.branch;
    try {
      final now = await _backend.gitStatus();
      final stashed = now == null ? null : await _backend.gitStash();
      // A later read, such as one after a write, wins.
      if (read != _reads) return;
      problem.value = null;
      status.value = now;
      // The same stash keeps its value, so the Diff tab keeps its place in it.
      // The same stash on the same tree keeps its value, so the Diff tab keeps its place.
      if (stashed?.id != stash.value?.id ||
          stashed?.current != stash.value?.current) {
        stash.value = stashed;
      }
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
      done: (now) {
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
  Future<BridgeGitOutcome?> create(String name) => _write(
    'create the branch',
    () => _backend.createBranch(name),
    done: (_) => 'Created and switched to $name.',
  );

  /// Moves the stashed changes `stashed` back into the checkout.
  Future<void> restoreStash(BridgeStash stashed) async {
    final files = stashed.files.length;
    final branch = status.value?.branch;
    await _write(
      'restore',
      () => _backend.restoreStash(stashed.id),
      done: (_) =>
          'Restored $files stashed ${files == 1 ? 'file' : 'files'} on $branch',
    );
  }

  /// Deletes the stashed changes `stashed`.
  Future<void> discardStash(BridgeStash stashed) => _write(
    'discard',
    () => _backend.discardStash(stashed.id),
    done: (_) => 'Discarded the stashed changes.',
  );

  /// Runs the Git write `run`, reports how it ended, `action` naming it, and reads the state
  /// again; returns the outcome, null after an error it reported.
  Future<BridgeGitOutcome?> _write(
    String action,
    Future<BridgeGitOutcome> Function() run, {
    required String Function(BridgeGitStatus? now) done,
  }) async {
    _writing = true;
    try {
      final BridgeGitOutcome outcome;
      try {
        outcome = await run();
      } on Object catch (error) {
        notify(BridgeLevel.error, 'Could not $action: $error');
        // Git may have changed something before it failed.
        await refresh();
        return null;
      }
      await refresh();
      final files = outcome.files.join(', ');
      switch (outcome.kind) {
        case BridgeGitOutcomeKind.done:
          notify(BridgeLevel.info, done(status.value));
        case BridgeGitOutcomeKind.hookFailed:
          notify(
            BridgeLevel.warning,
            '${done(status.value)} A hook failed after: ${outcome.detail}',
          );
        case BridgeGitOutcomeKind.agentRunning:
          notify(
            BridgeLevel.warning,
            'Could not $action: ${whoRuns(outcome.running)}. ${stopFirst(outcome.running)}.',
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
        case BridgeGitOutcomeKind.wouldOverwrite:
      }
      if (_arrived case final arrived?) notify(BridgeLevel.info, arrived);
      return outcome;
    } finally {
      _writing = false;
      _arrived = null;
    }
  }
}

/// Which agents run in the checkout, such as “Fix flaky sync test” is running in this
/// checkout.
String whoRuns(List<BridgeRunningAgent> running) {
  final first = running.firstOrNull;
  final name = switch ((first?.own, first?.title)) {
    (true, _) => 'This session’s agent',
    (_, final title?) => '“$title”',
    _ => 'Another agent session',
  };
  final more = running.length - 1;
  if (more > 0) return '$name and $more more are running in this checkout';
  return first?.own == true
      ? '$name is running'
      : '$name is running in this checkout';
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
