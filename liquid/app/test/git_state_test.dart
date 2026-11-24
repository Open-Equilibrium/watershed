import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/git_state.dart';

import 'fake_backend.dart';

const commit = BridgeGitAction.commit;
const commitPush = BridgeGitAction.commitPush;
const push = BridgeGitAction.push;
const createPr = BridgeGitAction.createPullRequest;
const commitPushPr = BridgeGitAction.commitPushPullRequest;

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

    void runs(BridgeGitStatus? status, String label, BridgeGitAction action) =>
        expectQuick(status, label, QuickKind.runAction, action: action);
    void hints(BridgeGitStatus? status, String label, String hint) =>
        expectQuick(status, label, QuickKind.showHint, hint: hint);

    const upToDate = 'Branch is up to date. No action needed.';

    runs(checkout(changed: true), 'Commit, push & PR', commitPushPr);
    hints(null, 'Commit', 'Git status is unavailable.');
    expectQuick(
      checkout(changed: true),
      'Commit',
      QuickKind.showHint,
      hint: 'Git action in progress.',
      busy: true,
    );
    hints(
      checkout(branch: null),
      'Commit',
      'Create and checkout a ref before pushing or opening a pull request.',
    );
    runs(checkout(changed: true, behind: 2), 'Commit', commit);
    runs(
      checkout(changed: true, remote: false, upstream: false),
      'Commit',
      commit,
    );
    runs(
      checkout(changed: true, defaultBranch: true),
      'Commit & push',
      commitPush,
    );
    runs(
      checkout(changed: true, pullRequest: pullRequestOf()),
      'Commit & push',
      commitPush,
    );
    expectQuick(
      checkout(remote: false, upstream: false),
      'Publish repository',
      QuickKind.openPublish,
    );
    runs(checkout(upstream: false, ahead: 2), 'Push & create PR', createPr);
    hints(checkout(upstream: false), 'Push', 'No local commits to push.');
    runs(checkout(ahead: 1, pullRequest: pullRequestOf()), 'Push', push);
    runs(checkout(ahead: 1, defaultBranch: true), 'Push', commitPush);
    runs(checkout(ahead: 1), 'Push & create PR', createPr);
    hints(
      checkout(ahead: 1, behind: 1),
      'Sync ref',
      'Branch has diverged from upstream. Rebase/merge first.',
    );
    expectQuick(checkout(behind: 3), 'Pull', QuickKind.runPull);
    hints(checkout(pullRequest: pullRequestOf()), 'Commit', upToDate);
    runs(checkout(aheadOfDefault: 2), 'Create PR', createPr);
    hints(checkout(), 'Commit', upToDate);
    runs(
      checkout(provider: 'gitlab', upstream: false, ahead: 2),
      'Push & create MR',
      createPr,
    );
    runs(
      checkout(provider: 'gitlab', changed: true),
      'Commit, push & MR',
      commitPushPr,
    );
  });

  test('the menu lists each item with why it is disabled', () {
    const clean = 'Worktree is clean. Make changes before committing.';
    const inProgress = 'Git action in progress.';
    menu(BridgeGitStatus? status, {bool busy = false}) =>
        buildMenuItems(status, busy: busy);

    expect(menu(null), isEmpty);
    expect(menu(checkout(remote: false, upstream: false)), [
      (MenuItem.commit, clean),
    ]);
    expect(menu(checkout(changed: true, ahead: 1)), [
      (MenuItem.commit, null),
      (MenuItem.push, null),
      (MenuItem.createPullRequest, null),
    ]);
    expect(menu(checkout(branch: null, changed: true, ahead: 1)), [
      (MenuItem.commit, null),
      (MenuItem.push, 'Detached HEAD: check out a branch before pushing.'),
      (
        MenuItem.createPullRequest,
        'Detached HEAD: check out a branch before creating a pull request.',
      ),
    ]);
    expect(menu(checkout(changed: true, behind: 1, ahead: 1)), [
      (MenuItem.commit, null),
      (MenuItem.push, 'Branch is behind upstream. Pull/rebase before pushing.'),
      (
        MenuItem.createPullRequest,
        'Branch is behind upstream. Pull/rebase before creating a pull request.',
      ),
    ]);
    expect(menu(checkout(changed: true, pullRequest: pullRequestOf())), [
      (MenuItem.commit, null),
      (MenuItem.push, 'No local commits to push.'),
    ], reason: 'an open pull request leaves out the create item');
    expect(menu(checkout(provider: 'gitlab', changed: true)), [
      (MenuItem.commit, null),
      (MenuItem.push, 'No local commits to push.'),
      (
        MenuItem.createPullRequest,
        'No local commits to include in a merge request.',
      ),
    ]);
    expect(menu(checkout(changed: true, ahead: 1), busy: true), [
      (MenuItem.commit, inProgress),
      (MenuItem.push, inProgress),
      (MenuItem.createPullRequest, inProgress),
    ]);
  });

  test(
    'a run shows its phase, hook and output as they come, then its result',
    () async {
      final git = SteppedGit();
      addTearDown(git.progress.close);
      final state = gitState(git);

      final run = state.run(commitPush, message: 'Fix');
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

      final run = state.run(push);
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

      final first = state.run(commit);
      await state.run(push);
      await state.pull();
      await git.progress.close();
      await first;

      expect(git.calls, ['run commit message=null']);
    },
  );

  test(
    'pull and initialize show their result, and a failure is named',
    () async {
      final git = FakeGit();
      final state = gitState(git);
      await state.pull();
      expect(git.calls, ['pull']);
      expect(state.result.value, (title: 'Pulled', description: 'Up to date'));
      expect(state.running.value, isNull);
      await gitState(FakeGit()..failures['pull'] = 'offline').pull();
      expect(errors, ['Pull failed: offline']);

      await state.initialize();
      expect(git.calls, ['pull', 'init']);
      expect(state.running.value, isNull);
      await gitState(FakeGit()..failures['init'] = 'no permission')
          .initialize();
      expect(errors.last, 'Git initialization failed: no permission');
    },
  );

  test(
    'a checkout that never opened is disposed without unhandled errors',
    () async {
      final errors = <Object>[];
      await runZonedGuarded(() async {
        final state = GitState(
          RustGitBackend(
            Future<GitCheckout>.error(StateError('no repository')),
          ),
          onError: (_, _) {},
        );
        await pumpEventQueue();
        state.dispose();
        await pumpEventQueue();
      }, (error, _) => errors.add(error));
      expect(errors, isEmpty);
    },
  );

  test('a checkout whose status cannot be watched reports that Git status is unavailable', () async {
    final state = gitState(FakeGit()..failures['watch'] = 'gone');
    await pumpEventQueue();

    expect(state.problem.value, 'Git status is unavailable.');
    expect(state.status.value, isNull);
  });
}
