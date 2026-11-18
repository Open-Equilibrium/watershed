import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';
import '../rust/api/git.dart';
import 'backend.dart';

typedef Terminology = ({String singular, String short});

Terminology terminologyFor(BridgeGitStatus status) =>
    status.provider?.toLowerCase() == 'gitlab'
    ? (singular: 'merge request', short: 'MR')
    : (singular: 'pull request', short: 'PR');

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

enum MenuItem { commit, push, createPullRequest }

typedef GitResult = ({String title, String? description});

typedef GitRunning = ({String label, String? output, DateTime since});

class GitState {
  GitState(this._git, {required this.onError}) {
    _watch = _git.watch().listen(
      (next) => status.value = next,
      onError: (Object error) => problem.value = 'Git status is unavailable.',
    );
  }

  final GitBackend _git;

  final void Function(String title, String message) onError;
  late final StreamSubscription<BridgeGitStatus> _watch;

  final status = ValueNotifier<BridgeGitStatus?>(null);
  final problem = ValueNotifier<String?>(null);
  final running = ValueNotifier<GitRunning?>(null);
  final result = ValueNotifier<GitResult?>(null);
  Timer? _resultTimer;

  bool get busy => running.value != null;

  GitBackend get backend => _git;

  QuickAction quickAction() => resolveQuickAction(status.value, busy: busy);

  List<(MenuItem, String?)> menuItems() =>
      buildMenuItems(status.value, busy: busy);

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
            _showResult((title: step.text, description: step.description));
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

  Future<void> pull() async {
    if (busy) return;
    running.value = (
      label: 'Pulling latest changes...',
      output: null,
      since: DateTime.now(),
    );
    try {
      final (title, description) = await _git.pull();
      _showResult((title: title, description: description));
    } on Object catch (error) {
      onError('Pull failed', '$error');
    } finally {
      running.value = null;
    }
  }

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
    _git.close();
  }
}

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

String elapsed(Duration duration) {
  final seconds = duration.inSeconds;
  if (seconds < 60) return '${seconds}s';
  return '${seconds ~/ 60}m ${seconds % 60}s';
}

class NoCheckout implements GitBackend {
  const NoCheckout();

  static final _error = StateError(
    'No agent host answers, so there is no checkout.',
  );

  static Never _unanswered() => throw _error;

  @override
  Future<BridgeGitStatus> status() async => _unanswered();
  @override
  Stream<BridgeGitStatus> watch() => Stream.error(_error);
  @override
  Future<BridgeRefs> refs(
    String query, {
    int cursor = 0,
    int limit = 50,
  }) async => _unanswered();
  @override
  Future<void> switchTo(String name) async => _unanswered();
  @override
  Future<void> createBranch(String name) async => _unanswered();
  @override
  Future<void> init() async => _unanswered();
  @override
  Future<(String, String)> pull() async => _unanswered();
  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) => Stream.error(_error);
  @override
  Future<List<BridgeFileChange>> uncommittedChanges({
    required bool hideWhitespace,
  }) async => _unanswered();
  @override
  Future<String> uncommittedPatch(
    String path, {
    required bool hideWhitespace,
  }) async => _unanswered();
  @override
  Future<BridgeResolvedPullRequest> resolvePullRequest(
    String reference,
  ) async => _unanswered();
  @override
  Future<BridgeCheckedOut> checkOutPullRequest(
    String reference, {
    required bool worktree,
  }) async => _unanswered();
  @override
  Future<void> publish(
    String provider,
    String repository, {
    required bool public,
  }) async => _unanswered();
  @override
  Future<List<BridgeFileChange>> branchChanges({
    required bool hideWhitespace,
  }) async => _unanswered();
  @override
  Future<String> branchPatch(
    String path, {
    required bool hideWhitespace,
  }) async => _unanswered();
  @override
  Future<BridgePullRequestDetail> pullRequest(int number) async =>
      _unanswered();
  @override
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  }) async => _unanswered();
  @override
  Future<BridgeCheckedOut> pullRequestWorktree(String url) async =>
      _unanswered();
  @override
  void close() {}
}
