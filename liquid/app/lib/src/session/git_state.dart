import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/git.dart';
import 'backend.dart';

/// What a provider calls a change request: GitLab's merge request, else a pull request.
typedef Terminology = ({String singular, String short});

Terminology terminologyFor(BridgeGitStatus status) =>
    status.provider?.toLowerCase() == 'gitlab'
    ? (singular: 'merge request', short: 'MR')
    : (singular: 'pull request', short: 'PR');

/// What the split button does when pressed, as T3's `resolveQuickAction` decides.
enum QuickKind { runAction, runPull, openPublish, showHint }

@immutable
class QuickAction {
  const QuickAction(this.label, this.kind, {this.action, this.hint});

  final String label;
  final QuickKind kind;
  final BridgeGitAction? action;
  final String? hint;

  bool get disabled => kind == QuickKind.showHint;
}

/// One item of the split button's menu.
enum MenuItem { commit, push, createPullRequest }

/// A finished action's result, shown in place of the button for ten seconds.
typedef GitResult = ({String title, String? description, String? url});

/// What runs: its phase label, the latest hook line, and when it started.
typedef GitRunning = ({String label, String? output, DateTime since});

/// The Git state of the session's checkout, from T3's status stream (ADR-0282).
class GitState {
  GitState(this._git, {required this.onError}) {
    _watch = _git.watch().listen(
      (next) => status.value = next,
      onError: (Object error) => problem.value = 'Git status is unavailable.',
    );
  }

  final GitBackend _git;

  /// Reports a failed action as T3's toast does: a title and a message.
  final void Function(String title, String message) onError;
  late final StreamSubscription<BridgeGitStatus> _watch;

  final status = ValueNotifier<BridgeGitStatus?>(null);
  final problem = ValueNotifier<String?>(null);
  final running = ValueNotifier<GitRunning?>(null);
  final result = ValueNotifier<GitResult?>(null);
  Timer? _resultTimer;

  bool get busy => running.value != null;

  GitBackend get backend => _git;

  /// The split button's action for the current status.
  QuickAction quickAction() => resolveQuickAction(status.value, busy: busy);

  /// The menu's items with why each is disabled, or null when it is enabled.
  List<(MenuItem, String?)> menuItems() =>
      buildMenuItems(status.value, busy: busy);

  /// Runs a stacked action, showing its progress and then its result.
  Future<void> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) async {
    if (busy) return;
    _showResult(null);
    final since = DateTime.now();
    running.value = (
      label: 'Starting source control action...',
      output: null,
      since: since,
    );
    try {
      await for (final step in _git.run(
        action,
        message: message,
        files: files,
        featureBranch: featureBranch,
      )) {
        switch (step.kind) {
          case BridgeGitProgressKind.phase:
            running.value = (label: step.text, output: null, since: since);
          case BridgeGitProgressKind.hook:
            running.value = (
              label: 'Running ${step.text}...',
              output: null,
              since: since,
            );
          case BridgeGitProgressKind.output:
            final current = running.value;
            if (current != null) {
              running.value = (
                label: current.label,
                output: step.text,
                since: since,
              );
            }
          case BridgeGitProgressKind.finished:
            _showResult((
              title: step.text,
              description: step.description,
              url: step.pullRequest,
            ));
          case BridgeGitProgressKind.failed:
            onError('Action failed', step.text);
        }
      }
    } on Object catch (error) {
      onError('Action failed', '$error');
    } finally {
      running.value = null;
    }
  }

  /// Pulls the upstream, as T3's quick action does.
  Future<void> pull() async {
    if (busy) return;
    running.value = (
      label: 'Pulling latest changes...',
      output: null,
      since: DateTime.now(),
    );
    try {
      final (title, description) = await _git.pull();
      _showResult((title: title, description: description, url: null));
    } on Object catch (error) {
      onError('Pull failed', '$error');
    } finally {
      running.value = null;
    }
  }

  /// Initializes Git in the folder.
  Future<void> initialize() async {
    if (busy) return;
    running.value = (
      label: 'Initializing...',
      output: null,
      since: DateTime.now(),
    );
    try {
      await _git.init();
    } on Object catch (error) {
      onError('Git initialization failed', '$error');
    } finally {
      running.value = null;
    }
  }

  void _showResult(GitResult? next) {
    _resultTimer?.cancel();
    result.value = next;
    if (next != null) {
      _resultTimer = Timer(
        const Duration(seconds: 10),
        () => result.value = null,
      );
    }
  }

  void dispose() {
    _resultTimer?.cancel();
    unawaited(_watch.cancel());
  }
}

/// T3's `resolveQuickAction`.
QuickAction resolveQuickAction(BridgeGitStatus? status, {required bool busy}) {
  if (busy) {
    return const QuickAction(
      'Commit',
      QuickKind.showHint,
      hint: 'Git action in progress.',
    );
  }
  if (status == null) {
    return const QuickAction(
      'Commit',
      QuickKind.showHint,
      hint: 'Git status is unavailable.',
    );
  }
  final terms = terminologyFor(status);
  final changes = status.changes.isNotEmpty;
  final openPr = status.pullRequest?.state == 'open';
  final ahead = status.ahead > 0;
  final behind = status.behind > 0;
  final defaultDelta = (status.aheadOfDefault ?? status.ahead) > 0;
  final defaultRef = status.defaultBranch;
  const upToDate = QuickAction(
    'Commit',
    QuickKind.showHint,
    hint: 'Branch is up to date. No action needed.',
  );
  QuickAction pushOrCreate() => openPr || defaultRef
      ? QuickAction(
          'Push',
          QuickKind.runAction,
          action: defaultRef
              ? BridgeGitAction.commitPush
              : BridgeGitAction.push,
        )
      : QuickAction(
          'Push & create ${terms.short}',
          QuickKind.runAction,
          action: BridgeGitAction.createPullRequest,
        );
  if (status.branch == null) {
    return QuickAction(
      'Commit',
      QuickKind.showHint,
      hint:
          'Create and checkout a ref before pushing or opening a ${terms.singular}.',
    );
  }
  if (changes) {
    if (behind || (!status.upstream && !status.remote)) {
      return const QuickAction(
        'Commit',
        QuickKind.runAction,
        action: BridgeGitAction.commit,
      );
    }
    if (openPr || defaultRef) {
      return const QuickAction(
        'Commit & push',
        QuickKind.runAction,
        action: BridgeGitAction.commitPush,
      );
    }
    return QuickAction(
      'Commit, push & ${terms.short}',
      QuickKind.runAction,
      action: BridgeGitAction.commitPushPullRequest,
    );
  }
  if (!status.upstream) {
    if (!status.remote) {
      return const QuickAction('Publish repository', QuickKind.openPublish);
    }
    if (!ahead) {
      return openPr
          ? upToDate
          : const QuickAction(
              'Push',
              QuickKind.showHint,
              hint: 'No local commits to push.',
            );
    }
    return pushOrCreate();
  }
  if (ahead && behind) {
    return const QuickAction(
      'Sync ref',
      QuickKind.showHint,
      hint: 'Branch has diverged from upstream. Rebase/merge first.',
    );
  }
  if (behind) return const QuickAction('Pull', QuickKind.runPull);
  if (ahead) return pushOrCreate();
  if (openPr) return upToDate;
  if (defaultDelta && !defaultRef) {
    return QuickAction(
      'Create ${terms.short}',
      QuickKind.runAction,
      action: BridgeGitAction.createPullRequest,
    );
  }
  return upToDate;
}

/// T3's `buildMenuItems` with `getMenuActionDisabledReason`.
List<(MenuItem, String?)> buildMenuItems(
  BridgeGitStatus? status, {
  required bool busy,
}) {
  if (status == null) return const [];
  final terms = terminologyFor(status);
  final branch = status.branch != null;
  final openPr = status.pullRequest?.state == 'open';
  final behind = status.behind > 0;
  final ahead = status.ahead > 0;
  final reachable = status.upstream || status.remote;
  String? reason(String? why) => busy ? 'Git action in progress.' : why;
  final commit = reason(
    status.changes.isEmpty
        ? 'Worktree is clean. Make changes before committing.'
        : null,
  );
  if (!status.remote) return [(MenuItem.commit, commit)];
  final push = reason(switch (()) {
    _ when !branch => 'Detached HEAD: check out a branch before pushing.',
    _ when behind => 'Branch is behind upstream. Pull/rebase before pushing.',
    _ when !reachable => 'Add an "origin" remote before pushing.',
    _ when !ahead => 'No local commits to push.',
    _ => null,
  });
  if (openPr) return [(MenuItem.commit, commit), (MenuItem.push, push)];
  final create = reason(switch (()) {
    _ when !branch =>
      'Detached HEAD: check out a branch before creating a ${terms.singular}.',
    _ when !reachable =>
      'Add an "origin" remote before creating a ${terms.singular}.',
    _ when (status.aheadOfDefault ?? status.ahead) <= 0 =>
      'No local commits to include in a ${terms.singular}.',
    _ when behind =>
      'Branch is behind upstream. Pull/rebase before creating a ${terms.singular}.',
    _ => null,
  });
  return [
    (MenuItem.commit, commit),
    (MenuItem.push, push),
    (MenuItem.createPullRequest, create),
  ];
}

/// The default-branch confirmation's title, description and continue label, as T3 words them.
({String title, String description, String continueLabel}) defaultBranchCopy(
  BridgeGitAction action,
  String branch, {
  required bool includesCommit,
  required Terminology terms,
}) {
  final suffix =
      ' on "$branch". You can continue on this ref or create a feature ref and run the same '
      'action there.';
  final pushes =
      action == BridgeGitAction.push || action == BridgeGitAction.commitPush;
  if (pushes) {
    return includesCommit
        ? (
            title: 'Commit & push to default ref?',
            description: 'This action will commit and push changes$suffix',
            continueLabel: 'Commit & push to $branch',
          )
        : (
            title: 'Push to default ref?',
            description: 'This action will push local commits$suffix',
            continueLabel: 'Push to $branch',
          );
  }
  return includesCommit
      ? (
          title: 'Commit, push & create ${terms.short} from default ref?',
          description:
              'This action will commit, push, and create a ${terms.singular}$suffix',
          continueLabel: 'Commit, push & create ${terms.short}',
        )
      : (
          title: 'Push & create ${terms.short} from default ref?',
          description:
              'This action will push local commits and create a ${terms.singular}$suffix',
          continueLabel: 'Push & create ${terms.short}',
        );
}

/// Why "Check out feature branch & continue" is unavailable, as T3 says it; null when it is.
String? featureBranchUnavailable(
  BridgeGitAction action,
  String branch, {
  required bool changes,
}) {
  if (!changes) {
    return "There are no changes to commit, so a feature branch can't be named. Create a branch "
        'yourself or continue on $branch.';
  }
  if (action == BridgeGitAction.push ||
      action == BridgeGitAction.createPullRequest) {
    return "This action doesn't commit your changes, so a feature branch can't be named. "
        'Create a branch yourself or continue on $branch.';
  }
  return null;
}

/// Seconds as T3 shows elapsed time: `12s`, `1m 5s`.
String elapsed(Duration duration) {
  final seconds = duration.inSeconds;
  if (seconds < 60) return '${seconds}s';
  return '${seconds ~/ 60}m ${seconds % 60}s';
}
