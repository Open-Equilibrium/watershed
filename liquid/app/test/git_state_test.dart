import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/git_state.dart';

import 'fake_backend.dart';

BridgeGitStatus checkout({
  String? branch = 'feature/x',
  bool defaultBranch = false,
  String? provider = 'github',
  bool remote = true,
  bool upstream = true,
  bool changed = false,
  int ahead = 0,
  int behind = 0,
  int? aheadOfDefault,
  BridgeBranchPullRequest? pullRequest,
}) => gitStatusOf(
  branch: branch,
  defaultBranch: defaultBranch,
  provider: provider,
  remote: remote,
  upstream: upstream,
  changes: changed ? [fileOf('a.dart')] : const [],
  ahead: ahead,
  behind: behind,
  aheadOfDefault: aheadOfDefault,
  pullRequest: pullRequest,
);

class SteppedGit extends FakeGit {
  final progress = StreamController<BridgeGitProgress>();

  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) {
    calls.add('run ${action.name} message=$message');
    return progress.stream;
  }
}

class PullFailingGit extends FakeGit {
  @override
  Future<(String, String)> pull() => Future.error('offline');
}

class InitFailingGit extends FakeGit {
  @override
  Future<void> init() => Future.error('no permission');
}

class WatchFailingGit extends FakeGit {
  @override
  Stream<BridgeGitStatus> watch() => Stream.error('gone');
}

void main() {
  late List<String> errors;

  setUp(() => errors = []);

  GitState gitState(GitBackend git) {
    final state = GitState(
      git,
      onError: (title, message) => errors.add('$title: $message'),
    );
    addTearDown(state.dispose);
    return state;
  }

  test('the split button does what T3 does in each state of the checkout', () {
    void expectQuick(
      BridgeGitStatus? status,
      String label,
      QuickKind kind, {
      BridgeGitAction? action,
      String? hint,
      bool busy = false,
    }) {
      final quick = resolveQuickAction(status, busy: busy);
      expect(
        (quick.label, quick.kind, quick.action, quick.hint),
        (label, kind, action, hint),
        reason: label,
      );
    }

    const upToDate = 'Branch is up to date. No action needed.';
    expectQuick(
      checkout(changed: true),
      'Commit, push & PR',
      QuickKind.runAction,
      action: BridgeGitAction.commitPushPullRequest,
    );
    expectQuick(
      null,
      'Commit',
      QuickKind.showHint,
      hint: 'Git status is unavailable.',
    );
    expectQuick(
      checkout(changed: true),
      'Commit',
      QuickKind.showHint,
      hint: 'Git action in progress.',
      busy: true,
    );
    expectQuick(
      checkout(branch: null),
      'Commit',
      QuickKind.showHint,
      hint:
          'Create and checkout a ref before pushing or opening a pull request.',
    );
    expectQuick(
      checkout(changed: true, behind: 2),
      'Commit',
      QuickKind.runAction,
      action: BridgeGitAction.commit,
    );
    expectQuick(
      checkout(changed: true, remote: false, upstream: false),
      'Commit',
      QuickKind.runAction,
      action: BridgeGitAction.commit,
    );
    expectQuick(
      checkout(changed: true, defaultBranch: true),
      'Commit & push',
      QuickKind.runAction,
      action: BridgeGitAction.commitPush,
    );
    expectQuick(
      checkout(changed: true, pullRequest: pullRequestOf()),
      'Commit & push',
      QuickKind.runAction,
      action: BridgeGitAction.commitPush,
    );
    expectQuick(
      checkout(remote: false, upstream: false),
      'Publish repository',
      QuickKind.openPublish,
    );
    expectQuick(
      checkout(upstream: false, ahead: 2),
      'Push & create PR',
      QuickKind.runAction,
      action: BridgeGitAction.createPullRequest,
    );
    expectQuick(
      checkout(upstream: false),
      'Push',
      QuickKind.showHint,
      hint: 'No local commits to push.',
    );
    expectQuick(
      checkout(ahead: 1, pullRequest: pullRequestOf()),
      'Push',
      QuickKind.runAction,
      action: BridgeGitAction.push,
    );
    expectQuick(
      checkout(ahead: 1, defaultBranch: true),
      'Push',
      QuickKind.runAction,
      action: BridgeGitAction.commitPush,
    );
    expectQuick(
      checkout(ahead: 1),
      'Push & create PR',
      QuickKind.runAction,
      action: BridgeGitAction.createPullRequest,
    );
    expectQuick(
      checkout(ahead: 1, behind: 1),
      'Sync ref',
      QuickKind.showHint,
      hint: 'Branch has diverged from upstream. Rebase/merge first.',
    );
    expectQuick(checkout(behind: 3), 'Pull', QuickKind.runPull);
    expectQuick(
      checkout(pullRequest: pullRequestOf()),
      'Commit',
      QuickKind.showHint,
      hint: upToDate,
    );
    expectQuick(
      checkout(aheadOfDefault: 2),
      'Create PR',
      QuickKind.runAction,
      action: BridgeGitAction.createPullRequest,
    );
    expectQuick(checkout(), 'Commit', QuickKind.showHint, hint: upToDate);
    expectQuick(
      checkout(provider: 'gitlab', upstream: false, ahead: 2),
      'Push & create MR',
      QuickKind.runAction,
      action: BridgeGitAction.createPullRequest,
    );
    expectQuick(
      checkout(provider: 'gitlab', changed: true),
      'Commit, push & MR',
      QuickKind.runAction,
      action: BridgeGitAction.commitPushPullRequest,
    );
  });

  test('the menu lists each item with why it is disabled', () {
    const clean = 'Worktree is clean. Make changes before committing.';
    const busy = 'Git action in progress.';
    expect(buildMenuItems(null, busy: false), isEmpty);
    expect(
      buildMenuItems(checkout(remote: false, upstream: false), busy: false),
      [(MenuItem.commit, clean)],
    );
    expect(buildMenuItems(checkout(changed: true, ahead: 1), busy: false), [
      (MenuItem.commit, null),
      (MenuItem.push, null),
      (MenuItem.createPullRequest, null),
    ]);
    expect(
      buildMenuItems(
        checkout(branch: null, changed: true, ahead: 1),
        busy: false,
      ),
      [
        (MenuItem.commit, null),
        (MenuItem.push, 'Detached HEAD: check out a branch before pushing.'),
        (
          MenuItem.createPullRequest,
          'Detached HEAD: check out a branch before creating a pull request.',
        ),
      ],
    );
    expect(
      buildMenuItems(checkout(changed: true, behind: 1, ahead: 1), busy: false),
      [
        (MenuItem.commit, null),
        (
          MenuItem.push,
          'Branch is behind upstream. Pull/rebase before pushing.',
        ),
        (
          MenuItem.createPullRequest,
          'Branch is behind upstream. Pull/rebase before creating a pull request.',
        ),
      ],
    );
    expect(
      buildMenuItems(
        checkout(changed: true, pullRequest: pullRequestOf()),
        busy: false,
      ),
      [(MenuItem.commit, null), (MenuItem.push, 'No local commits to push.')],
      reason: 'an open pull request leaves out the create item',
    );
    expect(
      buildMenuItems(checkout(provider: 'gitlab', changed: true), busy: false),
      [
        (MenuItem.commit, null),
        (MenuItem.push, 'No local commits to push.'),
        (
          MenuItem.createPullRequest,
          'No local commits to include in a merge request.',
        ),
      ],
    );
    expect(buildMenuItems(checkout(changed: true, ahead: 1), busy: true), [
      (MenuItem.commit, busy),
      (MenuItem.push, busy),
      (MenuItem.createPullRequest, busy),
    ]);
  });

  test('the default-branch confirmation words the action as T3 does', () {
    const github = (singular: 'pull request', short: 'PR');
    const gitlab = (singular: 'merge request', short: 'MR');
    final cases = [
      (
        BridgeGitAction.push,
        false,
        github,
        'Push to default ref?',
        'Push to main',
      ),
      (
        BridgeGitAction.commitPush,
        true,
        github,
        'Commit & push to default ref?',
        'Commit & push to main',
      ),
      (
        BridgeGitAction.createPullRequest,
        false,
        github,
        'Push & create PR from default ref?',
        'Push & create PR',
      ),
      (
        BridgeGitAction.commitPushPullRequest,
        true,
        github,
        'Commit, push & create PR from default ref?',
        'Commit, push & create PR',
      ),
      (
        BridgeGitAction.createPullRequest,
        false,
        gitlab,
        'Push & create MR from default ref?',
        'Push & create MR',
      ),
    ];
    for (final (action, includesCommit, terms, title, label) in cases) {
      final copy = defaultBranchCopy(
        action,
        'main',
        includesCommit: includesCommit,
        terms: terms,
      );
      expect(
        (copy.title, copy.continueLabel),
        (title, label),
        reason: '$action',
      );
    }
    expect(
      defaultBranchCopy(
        BridgeGitAction.push,
        'main',
        includesCommit: false,
        terms: github,
      ).description,
      'This action will push local commits on "main". You can continue on this ref '
      'or create a feature ref and run the same action there.',
    );
  });

  test('a feature branch is offered only to actions that commit changes', () {
    for (final (action, changes, offered) in [
      (BridgeGitAction.commit, false, false),
      (BridgeGitAction.push, true, false),
      (BridgeGitAction.createPullRequest, true, false),
      (BridgeGitAction.commitPush, true, true),
      (BridgeGitAction.commitPushPullRequest, true, true),
    ]) {
      expect(
        featureBranchUnavailable(action, 'main', changes: changes),
        offered ? isNull : isNotNull,
        reason: '$action',
      );
    }
  });

  test('elapsed time reads as seconds, then minutes and seconds', () {
    expect(
      [
        for (final seconds in [0, 12, 65]) elapsed(Duration(seconds: seconds)),
      ],
      ['0s', '12s', '1m 5s'],
    );
  });

  test('the state follows the checkout status as it changes', () async {
    final git = FakeGit(status: checkout());
    final state = gitState(git);
    await pumpEventQueue();
    expect(state.quickAction().label, 'Commit');

    git.change(checkout(behind: 2));
    await pumpEventQueue();

    expect(state.quickAction().kind, QuickKind.runPull);
  });

  test(
    'a run shows its phase, hook and output as they come, then its result',
    () async {
      final git = SteppedGit();
      addTearDown(git.progress.close);
      final state = gitState(git);

      final run = state.run(BridgeGitAction.commitPush, message: 'Fix');
      expect(state.running.value?.label, 'Starting source control action...');

      git.progress.add(progressOf(BridgeGitProgressKind.phase, 'Committing'));
      await pumpEventQueue();
      expect(state.running.value?.label, 'Committing');
      git.progress.add(progressOf(BridgeGitProgressKind.hook, 'lint'));
      await pumpEventQueue();
      expect(state.running.value?.label, 'Running lint...');
      git.progress.add(
        progressOf(BridgeGitProgressKind.output, '1 file changed'),
      );
      await pumpEventQueue();
      expect(state.running.value?.output, '1 file changed');

      git.progress.add(
        progressOf(
          BridgeGitProgressKind.finished,
          'Committed',
          description: '1 file',
          pullRequest: 'https://example.com/pr/1',
        ),
      );
      await pumpEventQueue();
      expect(state.result.value, (title: 'Committed', description: '1 file'));

      await git.progress.close();
      await run;
      expect(state.running.value, isNull);
      expect(git.calls, ['run commitPush message=Fix']);
    },
  );

  test(
    'a failed step and a broken stream are action failures, with no result',
    () async {
      final git = SteppedGit();
      addTearDown(git.progress.close);
      final state = gitState(git);

      final run = state.run(BridgeGitAction.push);
      git.progress.add(
        progressOf(BridgeGitProgressKind.failed, 'rejected: non-fast-forward'),
      );
      git.progress.addError('ssh: connection lost');
      await run;

      expect(errors, [
        'Action failed: rejected: non-fast-forward',
        'Action failed: ssh: connection lost',
      ]);
      expect(state.result.value, isNull);
      expect(state.running.value, isNull);
    },
  );

  test(
    'only one action runs at a time: a second action or a pull is ignored',
    () async {
      final git = SteppedGit();
      addTearDown(git.progress.close);
      final state = gitState(git);

      final first = state.run(BridgeGitAction.commit);
      await state.run(BridgeGitAction.push);
      await state.pull();
      await git.progress.close();
      await first;

      expect(git.calls, ['run commit message=null']);
    },
  );

  test('pull shows its result, and a failure is a pull error', () async {
    final git = FakeGit();
    final state = gitState(git);
    await state.pull();
    expect(git.calls, ['pull']);
    expect(state.result.value, (title: 'Pulled', description: 'Up to date'));
    expect(state.running.value, isNull);

    final offline = gitState(PullFailingGit());
    await offline.pull();
    expect(errors, ['Pull failed: offline']);
  });

  test(
    'initialize runs git init, and a failure is an initialization error',
    () async {
      final git = FakeGit();
      final state = gitState(git);
      await state.initialize();
      expect(git.calls, ['init']);
      expect(state.running.value, isNull);

      final denied = gitState(InitFailingGit());
      await denied.initialize();
      expect(errors, ['Git initialization failed: no permission']);
    },
  );

  testWidgets('a result shows for ten seconds, then clears', (tester) async {
    final state = gitState(FakeGit());
    await state.pull();

    expect(state.result.value, isNotNull);
    await tester.pump(const Duration(seconds: 9));
    expect(state.result.value, isNotNull);
    await tester.pump(const Duration(seconds: 1));
    expect(state.result.value, isNull);
  });

  test('a checkout whose status cannot be watched reports that Git status is unavailable', () async {
    final state = gitState(WatchFailingGit());
    await pumpEventQueue();

    expect(state.problem.value, 'Git status is unavailable.');
    expect(state.status.value, isNull);
  });
}
