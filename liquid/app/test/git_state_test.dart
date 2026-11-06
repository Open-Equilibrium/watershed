import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/git_state.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late GitState git;
  late List<(BridgeLevel, String)> notices;

  setUp(() {
    backend = FakeBackend();
    notices = [];
    git = GitState(
      backend,
      notify: (level, text) => notices.add((level, text)),
    );
  });

  test('a completed commit reports failed inspection as a warning', () async {
    backend.outcome = const BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.done,
      files: [],
      running: [],
      detail: 'Could not inspect the completed commit: unreadable index',
      commit: BridgeCommit(branch: 'main', subject: null, count: 1),
    );
    final result = await git.commit(['a.txt'], 'Subject', snapshot: 'snapshot');
    expect(result!.kind, BridgeGitOutcomeKind.done);
    expect(notices.single, (
      BridgeLevel.warning,
      'Committed 1 file on main. Could not inspect the completed commit: unreadable index',
    ));
  });

  test('hook changes report only files left uncommitted', () async {
    backend.outcome = BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.hookChanged,
      files: const ['a.txt'],
      running: const [],
      detail: '',
      commit: const BridgeCommit(branch: 'main', subject: 'Subject', count: 1),
    );
    await git.commit(['a.txt'], 'Subject', snapshot: 'snapshot');
    expect(notices.single, (
      BridgeLevel.warning,
      'Committed 1 file on main: “Subject” A hook also changed a.txt, which stays uncommitted.',
    ));
  });

  test(
    'equal fresh status objects do not notify but real changes do',
    () async {
      var changes = 0;
      git.status.addListener(() => changes++);
      await git.refresh();
      await git.refresh();
      expect(changes, 1);
      backend.git = gitStatusOf(changed: 2);
      await git.refresh();
      expect(changes, 2);
    },
  );

  test('new branch failure says nothing was committed and names the created branch', () async {
    backend.outcome = const BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.commitBranchFailed,
      files: [],
      running: [],
      detail: 'hook refused',
      commit: BridgeCommit(branch: 'feat', subject: null, count: 0),
    );
    await git.commit(
      ['a.txt'],
      'Subject',
      snapshot: 'snapshot',
      branch: 'feat',
    );
    expect(
      notices.single.$2,
      'Created and switched to feat; nothing was committed. hook refused',
    );
    expect(notices.single.$2, isNot(contains('null')));
  });

  test(
    'several hook-changed files stay uncommitted after the success notice',
    () async {
      backend.outcome = const BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.hookChanged,
        files: ['a.txt', 'b.txt'],
        running: [],
        detail: '',
        commit: BridgeCommit(branch: 'main', subject: 'Subject', count: 2),
      );
      await git.commit(['a.txt', 'b.txt'], 'Subject', snapshot: 'snapshot');
      expect(
        notices.single.$2,
        'Committed 2 files on main: “Subject” A hook also changed a.txt, b.txt, which stay uncommitted.',
      );
    },
  );

  int fetches() => backend.calls.where((call) => call == 'fetch').length;

  test(
    'push and pull notices report the actual destination and captured counts',
    () async {
      await git.refresh();
      backend.transferOutcome = BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.done,
        files: const [],
        running: const [],
        detail: '',
        transfer: transferOf(ahead: 1),
      );
      await git.push(device: 'MacBook');
      expect(git.fetched, isNotNull);
      backend.transferOutcome = BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.done,
        files: const [],
        running: const [],
        detail: '',
        transfer: transferOf(
          branch: 'feature/retry-after',
          upstream: 'origin/feature/retry-after',
          firstPush: true,
        ),
      );
      await git.push(device: 'MacBook');
      backend.transferOutcome = BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.done,
        files: const [],
        running: const [],
        detail: '',
        transfer: transferOf(behind: 3),
      );
      await git.pull(device: 'MacBook');
      expect(notices.map((notice) => notice.$2), [
        'Pushed 1 commit to origin/main',
        'Pushed feature/retry-after to origin; it now tracks origin/feature/retry-after',
        'Pulled 3 commits from origin/main',
      ]);
    },
  );

  test('push rejection, sign-in and pull refusals use the approved recovery wording', () async {
    await git.refresh();
    for (final ahead in [0, 1]) {
      backend.transferOutcome = BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.diverged,
        files: const [],
        running: const [],
        detail: '',
        transfer: transferOf(ahead: ahead, behind: 3),
      );
      await git.push(device: 'MacBook');
    }
    backend.transferOutcome = BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.signInFailed,
      files: const [],
      running: const [],
      detail: 'Authentication failed',
      transfer: transferOf(),
    );
    await git.push(device: 'MacBook');
    backend.transferOutcome = outcomeOf(
      BridgeGitOutcomeKind.wouldOverwrite,
      files: ['src/routes/login.ts'],
    );
    await git.pull(device: 'MacBook');
    backend.transferOutcome = outcomeOf(
      BridgeGitOutcomeKind.agentRunning,
      running: [
        const BridgeRunningAgent(own: false, title: 'Fix flaky sync test'),
      ],
    );
    await git.pull(device: 'MacBook');
    expect(
      notices[0].$2,
      startsWith(
        'Could not push: origin/main has 3 commits that main lacks. Pull them, then push.',
      ),
    );
    expect(
      notices[1].$2,
      startsWith(
        'Could not push: main and origin/main have diverged, 1 commit here and 3 there. Merge or rebase in a terminal, then push.',
      ),
    );
    expect(
      notices[2].$2,
      startsWith(
        'Could not push: Git could not sign in to origin. Liquid uses the Git login of MacBook; sign in there with git or gh, then push again.',
      ),
    );
    expect(notices[2].$2, contains('Nothing was pushed.'));
    expect(notices[2].$2, contains('“Authentication failed”'));
    expect(
      notices[3].$2,
      startsWith(
        'Could not pull: Git would overwrite your uncommitted changes in src/routes/login.ts. Commit them first.',
      ),
    );
    expect(
      notices[4].$2,
      'Could not pull: “Fix flaky sync test” is running in this checkout. Stop it or wait until it finishes. Nothing was pulled.',
    );
    expect(notices.map((notice) => notice.$1), everyElement(BridgeLevel.error));
  });

  test(
    'unsupported push destinations use the preflight error notice',
    () async {
      backend.transferOutcome = outcomeOf(
        BridgeGitOutcomeKind.preflightFailed,
        detail: 'This remote pushes to other or several repositories. Push it with Git.',
      );
      await git.push(device: 'MacBook');
      expect(notices.single, (
        BridgeLevel.error,
        'Could not push: This remote pushes to other or several repositories. Push it with Git. Nothing was pushed.',
      ));
    },
  );

  test('a transfer failure shows Git output and states that its effects are uncertain', () async {
    backend.transferOutcome = outcomeOf(
      BridgeGitOutcomeKind.transferFailed,
      detail: 'Could not resolve host: github.com',
    );
    await git.push(device: 'MacBook');
    expect(notices.single.$2, startsWith('Could not push: Git failed.'));
    expect(notices.single.$2, contains('“Could not resolve host: github.com”'));
    expect(
      notices.single.$2,
      contains('Git or its hooks may have made changes.'),
    );
  });

  testWidgets(
    'a requested Fetch shares a pending background fetch and reports its failure',
    (tester) async {
      final pending = _PendingFetchBackend();
      final state = GitState(
        pending,
        notify: (level, text) => notices.add((level, text)),
      );
      state.watch();
      await tester.pump();
      await tester.pump(fetchEvery);
      var finished = false;
      final requested = state.fetchNow().then((_) => finished = true);
      await tester.pump();
      expect(finished, isFalse);
      pending.fetched.completeError('offline');
      await tester.pump();
      await requested;
      expect(pending.calls.where((call) => call == 'fetch'), hasLength(1));
      expect(notices.single.$1, BridgeLevel.error);
      expect(
        notices.single.$2,
        contains('Could not fetch from origin: “offline”'),
      );
      state.stop();
    },
  );

  testWidgets('a requested Fetch queues behind a fetch for another remote', (
    tester,
  ) async {
    for (final stopped in [false, true]) {
      final pending = _QueuedFetchBackend();
      final state = GitState(
        pending,
        notify: (level, text) => notices.add((level, text)),
      );
      addTearDown(state.stop);
      state.watch();
      await tester.pump();
      await tester.pump(fetchEvery);
      expect(pending.remotes, ['origin']);
      pending.git = gitStatusOf(remote: 'other', upstream: 'other/main');
      await state.refresh();
      var finished = false;
      final requested = state.fetchNow().then((_) => finished = true);
      final shared = state.fetchNow();
      await tester.pump();
      expect(finished, isFalse);
      expect(pending.remotes, ['origin']);
      if (stopped) state.stop();
      pending.fetches.first.complete(true);
      await tester.pump();
      if (!stopped) {
        expect(pending.remotes, ['origin', 'other']);
        expect(finished, isFalse);
        expect(state.fetched, isNull);
        pending.fetches.last.completeError('offline');
        await tester.pump();
        expect(notices.single.$1, BridgeLevel.error);
        expect(
          notices.single.$2,
          contains('Could not fetch from other: “offline”'),
        );
      } else {
        expect(pending.remotes, ['origin']);
      }
      await requested;
      await shared;
      state.stop();
      notices.clear();
    }
  });

  testWidgets('background fetch waits while a push or pull runs', (
    tester,
  ) async {
    for (final push in [true, false]) {
      final state = GitState(
        backend,
        notify: (level, text) => notices.add((level, text)),
      );
      await state.refresh();
      backend.transferGate = Completer<BridgeGitOutcome>();
      state.watch();
      final writing = push
          ? state.push(device: 'MacBook')
          : state.pull(device: 'MacBook');
      await tester.pump();
      expect(
        state.progress.value,
        isNotNull,
        reason: 'transfer starts before the fetch tick',
      );
      await tester.pump(fetchEvery);
      expect(fetches(), 0, reason: push ? 'push' : 'pull');
      backend.transferGate!.complete(
        BridgeGitOutcome(
          kind: BridgeGitOutcomeKind.done,
          files: const [],
          running: const [],
          detail: '',
          transfer: transferOf(),
        ),
      );
      await tester.pump();
      await writing;
      state.stop();
    }
  });

  test(
    'preflight and fetch failures guarantee that nothing was transferred',
    () async {
      for (final push in [true, false]) {
        final action = push ? 'push' : 'pull';
        final past = push ? 'pushed' : 'pulled';
        for (final kind in [
          BridgeGitOutcomeKind.preflightFailed,
          BridgeGitOutcomeKind.fetchFailed,
        ]) {
          backend.transferOutcome = BridgeGitOutcome(
            kind: kind,
            files: const [],
            running: const [],
            detail: 'offline',
            transfer: transferOf(),
          );
          await (push
              ? git.push(device: 'MacBook')
              : git.pull(device: 'MacBook'));
          expect(
            git.fetched,
            isNull,
            reason: 'failed fetches do not refresh the fetched time',
          );
          expect(notices.last.$1, BridgeLevel.error);
          expect(notices.last.$2, startsWith('Could not $action:'));
          expect(notices.last.$2, contains('Nothing was $past.'));
          expect(notices.last.$2, isNot(contains('may have made changes')));
          if (kind == BridgeGitOutcomeKind.fetchFailed) {
            expect(
              notices.last.$2,
              contains('Could not fetch from origin: “offline”'),
            );
          }
        }
      }
    },
  );

  testWidgets('a pending fetch leaves checkout polling active', (tester) async {
    final pending = _PendingFetchBackend();
    final state = GitState(
      pending,
      notify: (level, text) => notices.add((level, text)),
    );
    addTearDown(state.stop);
    state.watch();
    await tester.pump();
    await tester.pump(fetchEvery);
    expect(pending.calls.where((call) => call == 'fetch'), hasLength(1));
    pending.git = gitStatusOf(branch: 'other', changed: 2);
    pending.stashed = stashOf();
    for (var tick = 0; tick < 3; tick++) {
      await tester.pump(fetchEvery);
      expect(state.status.value?.branch, 'other');
      expect(state.status.value?.changed, 2);
      expect(state.stash.value?.id, 's1');
    }
    expect(pending.calls.where((call) => call == 'fetch'), hasLength(1));
    state.stop();
    pending.fetched.completeError('fetch ended after stop');
    await tester.pump();
    expect(
      notices.where((notice) => notice.$2.contains('Could not fetch')),
      isEmpty,
    );
  });

  testWidgets('overlapping writes each publish their switch and arrival', (
    tester,
  ) async {
    final pending = _PendingWritesBackend();
    final state = GitState(
      pending,
      notify: (level, text) => notices.add((level, text)),
    );
    await state.refresh();
    Future<BridgeGitOutcome?> switchTo(String name) => state.switchTo(
      branchOf(name),
      leaveChanges: false,
      fromBranch: null,
      replacingStash: null,
    );
    final first = switchTo('one');
    final second = switchTo('two');
    await tester.pump();
    pending.writes['one']!.complete();
    await tester.pump();
    await first;
    pending.writes['two']!.complete();
    await tester.pump();
    await second;
    expect(notices.map((notice) => notice.$2), [
      'Switched to one.',
      'one has stashed changes from 10:41; the Diff tab shows them.',
      'Switched to two.',
      'two has stashed changes from 10:41; the Diff tab shows them.',
    ]);
  });

  for (final kind in [
    BridgeGitOutcomeKind.done,
    BridgeGitOutcomeKind.hookFailed,
  ]) {
    test(
      'a $kind switch with a failed refresh names the destination',
      () async {
        await git.refresh();
        backend.git = gitStatusOf(branch: 'feature');
        backend.gitError = 'status read failed';
        backend.outcome = BridgeGitOutcome(
          kind: kind,
          files: const [],
          running: const [],
          detail: kind == BridgeGitOutcomeKind.hookFailed ? 'hook refused' : '',
        );

        await git.switchTo(
          branchOf('feature'),
          leaveChanges: false,
          fromBranch: null,
          replacingStash: null,
        );

        expect(git.problem.value, 'status read failed');
        expect(notices.single, (
          kind == BridgeGitOutcomeKind.done
              ? BridgeLevel.info
              : BridgeLevel.warning,
          kind == BridgeGitOutcomeKind.done
              ? 'Switched to feature.'
              : 'Switched to feature. A hook failed after: hook refused',
        ));
      },
    );
  }

  testWidgets('a write waits for the latest read to publish its own switch', (
    tester,
  ) async {
    final pending = _PendingStatusBackend();
    final state = GitState(
      pending,
      notify: (level, text) => notices.add((level, text)),
    );
    await state.refresh();
    pending.hold = true;
    var completed = false;
    final write = state.switchTo(
      branchOf('feature'),
      leaveChanges: false,
      fromBranch: null,
      replacingStash: null,
    )..then((_) => completed = true);
    await tester.pump();
    expect(pending.reads, hasLength(1));
    final later = state.refresh();
    expect(pending.reads, hasLength(2));
    pending.reads[0].complete(gitStatusOf(branch: 'feature'));
    await tester.pump();
    expect(
      completed,
      isFalse,
      reason: 'the superseding read has not published',
    );
    expect(notices, isEmpty);
    pending.reads[1].complete(gitStatusOf(branch: 'feature'));
    await tester.pump();
    await write;
    await later;
    expect(state.status.value?.branch, 'feature');
    expect(notices, [(BridgeLevel.info, 'Switched to feature.')]);
  });

  test(
    'an external switch cannot pair another branch stash with the status',
    () async {
      final switching = _SwitchingBackend();
      final state = GitState(switching, notify: (_, _) {});

      await state.refresh();

      expect(state.status.value?.branch, 'main');
      expect(state.stash.value?.id, 'main-stash');
    },
  );

  testWidgets('fetches every 15 seconds and waits longer after failures', (
    tester,
  ) async {
    git.watch();
    await tester.pump();
    expect(git.status.value?.branch, 'main');
    await tester.pump(const Duration(seconds: 15));
    expect(fetches(), 1);

    backend.fetchError = 'Could not resolve host: github.com';
    await tester.pump(const Duration(seconds: 15));
    expect(fetches(), 2);
    expect(notices.single.$1, BridgeLevel.warning);
    expect(notices.single.$2, contains('Could not fetch from origin'));
    // The wait after a failure doubles from 30 seconds; a streak tells once.
    await tester.pump(const Duration(seconds: 29));
    expect(fetches(), 2);
    await tester.pump(const Duration(seconds: 1));
    expect(fetches(), 3);
    final reads = backend.statusReads;
    await tester.pump(const Duration(seconds: 60));
    expect(fetches(), 4);
    expect(notices, hasLength(1));
    expect(
      backend.statusReads - reads,
      4,
      reason: 'the state is read every 15 seconds while fetching waits',
    );

    backend.fetchError = null;
    await tester.pump(const Duration(seconds: 120));
    expect(fetches(), 5);
    await tester.pump(const Duration(seconds: 15));
    expect(fetches(), 6, reason: 'a fetch that works again resets the wait');

    git.stop();
  });

  testWidgets('a branch without upstream fetches nothing but stays current', (
    tester,
  ) async {
    backend.git = gitStatusOf(upstream: null);
    git.watch();
    await tester.pump(const Duration(seconds: 15));
    expect(fetches(), 0);
    backend.git = gitStatusOf(upstream: null, changed: 2);
    await tester.pump(const Duration(seconds: 15));
    expect(git.status.value?.changed, 2);
    git.stop();
  });

  test('a branch that only looks like the shown one is a new status', () async {
    backend.git = gitStatusOf(
      branch: 'fix/<U+202E>',
      rawBranch: 'fix/<U+202E>',
    );
    await git.refresh();
    backend.git = gitStatusOf(branch: 'fix/<U+202E>', rawBranch: 'fix/\u202e');
    await git.refresh();
    expect(git.status.value?.rawBranch, 'fix/\u202e');
  });

  test('the same stash keeps its value across reads', () async {
    backend.stashed = BridgeStash(
      current: true,
      id: 's1',
      createdMs: 0,
      files: [change('a.txt')],
    );
    await git.refresh();
    final first = git.stash.value;
    backend.stashed = BridgeStash(
      current: true,
      id: 's1',
      createdMs: 0,
      files: [change('a.txt')],
    );
    await git.refresh();
    expect(identical(git.stash.value, first), isTrue);
  });

  test('a hook failing after the switch is a warning, not a failure', () async {
    await git.refresh();
    backend.git = gitStatusOf(branch: 'other');
    backend.outcome = outcomeOf(
      BridgeGitOutcomeKind.hookFailed,
      detail: 'git-lfs was not found on your path',
    );
    await git.switchTo(
      branchOf('other'),
      leaveChanges: false,
      fromBranch: null,
      replacingStash: null,
    );
    expect(notices.single, (
      BridgeLevel.warning,
      'Switched to other. A hook failed after: git-lfs was not found on your path',
    ));
    expect(git.status.value?.branch, 'other');
  });

  test('says when the checkout switched outside Liquid', () async {
    await git.refresh();
    backend.git = gitStatusOf(branch: 'feature');
    await git.refresh();
    expect(
      notices.single.$2,
      'The checkout switched to feature outside Liquid.',
    );
  });

  test('back on a branch with stashed changes says so', () async {
    backend.git = gitStatusOf(branch: 'feature');
    await git.refresh();
    backend.git = gitStatusOf();
    backend.stashed = BridgeStash(
      current: true,
      id: 's1',
      createdMs: DateTime(2026, 11, 1, 10, 41).millisecondsSinceEpoch,
      files: [change('a.txt')],
    );

    await git.switchTo(
      branchOf('main'),
      leaveChanges: false,
      fromBranch: null,
      replacingStash: null,
    );

    expect(notices.map((notice) => notice.$2), [
      'Switched to main.',
      'main has stashed changes from 10:41; the Diff tab shows them.',
    ]);
  });

  test('switching tells where the changes went', () async {
    backend.git = gitStatusOf(changed: 4);
    await git.refresh();
    backend.git = gitStatusOf(branch: 'feature/token-bucket');

    await git.switchTo(
      branchOf('feature/token-bucket'),
      leaveChanges: true,
      fromBranch: 'main',
      replacingStash: null,
    );

    expect(backend.calls.last, 'switch feature/token-bucket leaving changes');
    expect(
      notices.single.$2,
      'Switched to feature/token-bucket. Your 4 changes are stashed on main.',
    );
    expect(git.status.value?.branch, 'feature/token-bucket');
  });

  test(
    'an agent running in the checkout is named and nothing changes',
    () async {
      await git.refresh();
      backend.outcome = outcomeOf(
        BridgeGitOutcomeKind.agentRunning,
        running: [
          const BridgeRunningAgent(own: false, title: 'Fix flaky sync test'),
        ],
      );

      await git.restoreStash(stashOf());

      expect(notices.single, (
        BridgeLevel.warning,
        'Could not restore: “Fix flaky sync test” is running in this checkout. '
            'Stop it or wait until it finishes.',
      ));
    },
  );

  test(
    'a restore that conflicts or that Git refuses keeps the stash',
    () async {
      backend.stashed = BridgeStash(
        current: true,
        id: 's1',
        createdMs: 0,
        files: [change('a.txt')],
      );
      await git.refresh();
      backend.outcome = outcomeOf(
        BridgeGitOutcomeKind.conflicts,
        files: ['src/routes/login.ts'],
      );
      await git.restoreStash(stashOf());
      backend.outcome = outcomeOf(
        BridgeGitOutcomeKind.wouldOverwrite,
        files: ['src/routes/login.ts'],
      );
      await git.restoreStash(stashOf());

      expect(notices.map((notice) => notice.$2), [
        'Restored with conflicts in src/routes/login.ts; Git kept the stash. Resolve them in '
            'an editor, then discard the stash.',
        'Could not restore: Git would overwrite your uncommitted changes in '
            'src/routes/login.ts. The stash stays; commit or discard those changes, then '
            'restore.',
      ]);
      expect(git.stash.value, isNotNull);
    },
  );

  test('a restore into a checkout with changes says what to do', () async {
    backend.git = gitStatusOf(changed: 2);
    await git.refresh();
    backend.outcome = outcomeOf(BridgeGitOutcomeKind.uncommitted);

    await git.restoreStash(stashOf());

    expect(notices.single, (
      BridgeLevel.warning,
      'Could not restore: the checkout has 2 uncommitted changes. Commit or discard them, '
          'then restore; the stash stays.',
    ));
  });

  test('restoring reports its files and reads the stash again', () async {
    backend.stashed = BridgeStash(
      current: true,
      id: 's1',
      createdMs: 0,
      files: [change('a.txt'), change('b.txt')],
    );
    await git.refresh();
    final stashed = git.stash.value!;
    backend.stashed = null;

    await git.restoreStash(stashed);

    expect(notices.single, (
      BridgeLevel.info,
      'Restored 2 stashed files on main',
    ));
    expect(git.stash.value, isNull);
  });

  test('Git that cannot be read is a problem, outside Git none', () async {
    backend.gitError = 'git could not start: No such file or directory';
    await git.refresh();
    expect(git.problem.value, contains('git could not start'));

    backend.gitError = null;
    backend.git = null;
    await git.refresh();
    expect(git.problem.value, isNull);
    expect(git.status.value, isNull);
  });

  test('describes branch names Git does not allow', () {
    expect(branchNameProblem('feature/retry-after'), isNull);
    expect(
      branchNameProblem('fix..login'),
      'Git does not allow “..” in a branch name.',
    );
    expect(
      branchNameProblem('a b'),
      'Git does not allow spaces in a branch name.',
    );
    expect(
      branchNameProblem('-x'),
      'Git does not allow a branch name to start with “-”.',
    );
    expect(
      branchNameProblem('x.lock'),
      'Git does not allow a branch name to end with “.lock”.',
    );
  });
}

class _SwitchingBackend extends FakeBackend {
  _SwitchingBackend() {
    stashed = const BridgeStash(
      current: true,
      id: 'main-stash',
      createdMs: 0,
      files: [],
    );
  }

  @override
  Future<BridgeGitStatus?> gitStatus() async {
    final before = await super.gitStatus();
    git = gitStatusOf(branch: 'other');
    stashed = const BridgeStash(
      current: true,
      id: 'other-stash',
      createdMs: 0,
      files: [],
    );
    return before;
  }
}

class _PendingStatusBackend extends FakeBackend {
  bool hold = false;
  final reads = <Completer<BridgeGitStatus?>>[];

  @override
  Future<BridgeGitStatus?> gitStatus() {
    if (!hold) return super.gitStatus();
    final read = Completer<BridgeGitStatus?>();
    reads.add(read);
    return read.future;
  }
}

BridgeStash stashOf() =>
    const BridgeStash(current: true, id: 's1', createdMs: 0, files: []);

BridgeFileChange change(String path) => BridgeFileChange(
  path: path,
  shown: path,
  status: BridgeChangeStatus.modified,
  added: 1,
  removed: 0,
);

class _PendingFetchBackend extends FakeBackend {
  final fetched = Completer<bool>();

  @override
  Future<bool> fetch() {
    calls.add('fetch');
    return fetched.future;
  }
}

class _QueuedFetchBackend extends FakeBackend {
  final remotes = <String?>[];
  final fetches = <Completer<bool>>[];

  @override
  Future<bool> fetch() {
    remotes.add(git?.remote);
    final fetched = Completer<bool>();
    fetches.add(fetched);
    return fetched.future;
  }
}

class _PendingWritesBackend extends FakeBackend {
  final writes = {'one': Completer<void>(), 'two': Completer<void>()};

  @override
  Future<BridgeGitOutcome> switchBranch(
    BridgeBranch branch, {
    required bool leaveChanges,
    required String? fromBranch,
    required String? replacingStash,
  }) async {
    await writes[branch.name]!.future;
    git = gitStatusOf(branch: branch.name);
    stashed = BridgeStash(
      current: true,
      id: branch.name,
      createdMs: DateTime(2026, 11, 1, 10, 41).millisecondsSinceEpoch,
      files: const [],
    );
    return outcome;
  }
}
