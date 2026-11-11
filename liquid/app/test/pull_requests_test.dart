import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/branch_bases.dart';
import 'package:liquid/src/session/pull_requests.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_file.dart';

import 'fake_backend.dart';

void main() {
  late Directory dir;
  late FakeBackend backend;
  late BranchBases bases;
  late GitState git;
  late PullRequests pulls;

  File basesFile() => File('${dir.path}/git-bases.json');
  BranchBases stored({String device = 'MacBook'}) =>
      BranchBases(file: SettingsFile(basesFile()), device: device);

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('liquid-pull-requests');
    backend = FakeBackend()..git = gitStatusOf(branch: 'main');
    bases = stored();
    git = GitState(backend, notify: (_, _) {}, bases: bases);
    pulls = PullRequests(backend, git, bases);
    await git.refresh();
  });

  tearDown(() async {
    pulls.dispose();
    await dir.delete(recursive: true);
  });

  test(
    'a branch Liquid creates remembers its base per repository in Liquid data',
    () async {
      await git.create('feature/retry-after');
      backend.git = gitStatusOf(branch: 'feature/retry-after');
      await git.refresh();
      backend.outcome = const BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.commitBranchFailed,
        files: [],
        running: [],
        detail: 'pre-commit refused',
        commit: BridgeCommit(branch: 'fix/typo', count: 0),
      );
      await git.commit(['a.txt'], 'Fix', snapshot: 's', branch: 'fix/typo');

      final reloaded = stored();
      expect(
        await reloaded.baseOf('/work/api/.git', 'feature/retry-after'),
        'main',
      );
      expect(
        await reloaded.baseOf('/work/api/.git', 'fix/typo'),
        'feature/retry-after',
      );
      expect(await reloaded.baseOf('/other/.git', 'fix/typo'), isNull);
      await pulls.plan();
      expect(backend.calls.last, 'plan - main');
    },
  );

  test(
    'base records name this computer by a random id kept in the file',
    () async {
      await BranchBases(file: SettingsFile(basesFile()))
          .record('/work/api/.git', 'feature', 'main');
      final stored = jsonDecode(basesFile().readAsStringSync()) as Map;
      expect(stored['device'], isA<String>());
      expect(stored['device'], isNot(Platform.localHostname));
      // A later start finds the record under the same id.
      expect(
        await BranchBases(file: SettingsFile(basesFile()))
            .baseOf('/work/api/.git', 'feature'),
        'main',
      );
      expect(
        (jsonDecode(basesFile().readAsStringSync()) as Map)['device'],
        stored['device'],
      );
    },
  );

  test(
    'records that named the host name move to the device id on first start',
    () async {
      basesFile().writeAsStringSync(
        jsonEncode({
          'bases': [
            {
              'device': Platform.localHostname,
              'repository': '/work/api/.git',
              'branch': 'feature',
              'base': 'main',
            },
          ],
        }),
      );
      final migrated = BranchBases(file: SettingsFile(basesFile()));
      expect(await migrated.baseOf('/work/api/.git', 'feature'), 'main');
      final stored = jsonDecode(basesFile().readAsStringSync()) as Map;
      expect(stored['device'], isNot(Platform.localHostname));
      expect(
        (stored['bases'] as List).single,
        containsPair('device', stored['device']),
      );
    },
  );

  test(
    'queued branch creations record the branch each one started from',
    () async {
      backend.onCreateBranch = (name) =>
          backend.git = gitStatusOf(branch: name);
      await Future.wait([
        git.create('one'),
        git.create('two'),
        git.commit(['a.txt'], 'Fix', snapshot: 's', branch: 'fix/typo'),
      ]);

      final reloaded = stored();
      expect(await reloaded.baseOf('/work/api/.git', 'one'), 'main');
      expect(await reloaded.baseOf('/work/api/.git', 'two'), 'one');
      expect(await reloaded.baseOf('/work/api/.git', 'fix/typo'), 'two');
    },
  );

  test('a branch created before a hook failed remembers its base', () async {
    backend.outcome = const BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.hookFailed,
      files: [],
      running: [],
      detail: 'post-checkout exited 1',
    );
    await git.create('feature/retry-after');

    expect(
      await stored().baseOf('/work/api/.git', 'feature/retry-after'),
      'main',
    );
  });

  test('base records keep the raw branch behind its displayed name', () async {
    backend.git = gitStatusOf(
      branch: 'release/<U+202E>',
      rawBranch: 'release/\u202e',
    );
    await git.refresh();
    await git.create('fix/\u202e');
    expect(
      await stored().baseOf('/work/api/.git', 'fix/\u202e'),
      'release/\u202e',
    );

    backend.git = gitStatusOf(branch: 'fix/<U+202E>', rawBranch: 'fix/\u202e');
    await git.refresh();
    await pulls.plan();
    expect(backend.calls.last, 'plan - release/\u202e');
  });

  test('the summary keeps its last update when a refresh fails and resets per branch', () async {
    backend.git = gitStatusOf(branch: 'feature/retry-after');
    backend.current = pullRequestOutcomeOf(pullRequest: pullRequestOf());
    await git.refresh();
    await pulls.refresh();
    expect(pulls.current.value?.number, 42);
    final updated = pulls.updated.value;
    expect(updated, isNotNull);

    backend.current = pullRequestOutcomeOf(
      kind: BridgePullRequestOutcomeKind.unreachable,
      host: 'github.com',
      detail: 'offline',
    );
    await pulls.refresh();
    expect(pulls.current.value?.number, 42);
    expect(pulls.updated.value, updated);
    expect(pulls.failure.value?.detail, 'offline');

    backend.git = gitStatusOf(branch: 'main');
    backend.current = pullRequestOutcomeOf();
    await git.refresh();
    await pulls.refresh();
    expect(pulls.current.value, isNull);
    expect(pulls.failure.value, isNull);
  });

  test('a push offers a pull request until dismissed or one exists', () async {
    backend.git = gitStatusOf(branch: 'feature/retry-after', ahead: 2);
    await git.refresh();
    final transfer = transferOf(
      branch: 'feature/retry-after',
      upstream: 'origin/feature/retry-after',
      ahead: 2,
    );
    await pulls.pushed(transfer);
    expect(pulls.offer.value, same(transfer));
    pulls.dismissOffer();
    expect(pulls.offer.value, isNull);

    await pulls.pushed(transfer);
    final created = await pulls.create(
      BridgePullRequestDraft(
        identity: planOf().identity,
        target: 'acme/api',
        base: 'main',
        title: 'Return 429',
        body: '',
        draft: false,
      ),
    );
    expect(created.kind, BridgePullRequestOutcomeKind.done);
    expect(pulls.offer.value, isNull);
    expect(pulls.current.value?.number, 42);

    // A push to a branch whose pull request is open only updates it.
    backend.current = pullRequestOutcomeOf(pullRequest: pullRequestOf());
    await pulls.pushed(transfer);
    expect(pulls.offer.value, isNull);
  });

  test('the offer follows the branch, not how its name displays', () async {
    // Two branches whose names differ only in characters shown the same way.
    final pushed = gitStatusOf(branch: 'fix/a?', rawBranch: 'fix/a\u0007');
    backend.git = pushed;
    await git.refresh();
    final transfer = transferOf(branch: 'fix/a?', upstream: 'origin/fix/a');
    await pulls.pushed(
      BridgeGitTransfer(
        branch: transfer.branch,
        remote: transfer.remote,
        upstream: transfer.upstream,
        ahead: transfer.ahead,
        behind: transfer.behind,
        firstPush: transfer.firstPush,
        gone: transfer.gone,
        destination: transfer.destination,
        confirmation: BridgePushConfirmation(
          branch: 'fix/a\u0007',
          remote: transfer.confirmation.remote,
          reference: transfer.confirmation.reference,
          commit: transfer.confirmation.commit,
          repository: transfer.confirmation.repository,
        ),
      ),
    );
    expect(pulls.offer.value, isNotNull);

    backend.git = gitStatusOf(branch: 'fix/a?', rawBranch: 'fix/a\u0008');
    await git.refresh();
    expect(pulls.offer.value, isNull);
  });

  test(
    'base records from several windows and devices keep each other',
    () async {
      final first = stored();
      final second = stored();
      // The first window read the records before the second wrote its own.
      expect(await first.baseOf('/work/api/.git', 'a'), isNull);
      await second.record('/work/api/.git', 'b', 'main');
      await first.record('/work/api/.git', 'a', 'main');
      await Future.wait([
        for (var index = 0; index < 8; index++)
          (index.isEven ? first : second).record(
            '/work/api/.git',
            'c$index',
            'main',
          ),
      ]);
      final reloaded = stored();
      for (final branch in ['a', 'b', for (var i = 0; i < 8; i++) 'c$i']) {
        expect(await reloaded.baseOf('/work/api/.git', branch), 'main');
      }
      // The same folder on another device is another repository.
      expect(
        await stored(device: 'Mac mini').baseOf('/work/api/.git', 'a'),
        isNull,
      );
      expect(
        dir.listSync().map((entry) => entry.path.split('/').last).toSet(),
        {'git-bases.json', 'git-bases.json.lock'},
      );
    },
  );

  test('a recorded base that cannot name a branch is ignored', () async {
    await basesFile().writeAsString(
      '{"bases":[{"device":"MacBook","repository":"/r","branch":"a","base":"-x"},'
      '{"device":"MacBook","repository":"/r","branch":"b","base":"x/../y"},'
      '{"device":"MacBook","repository":"/r","branch":"c","base":"x\\u0007"},'
      '{"device":"MacBook","repository":"/r","branch":"d","base":"develop"}]}',
    );
    final reloaded = stored();
    for (final branch in ['a', 'b', 'c']) {
      expect(await reloaded.baseOf('/r', branch), isNull, reason: branch);
    }
    expect(await reloaded.baseOf('/r', 'd'), 'develop');
  });

  test('status reads asked for during one run once more after it', () async {
    backend.git = gitStatusOf(
      branch: 'feature/retry-after',
      upstream: 'origin/feature/retry-after',
    );
    await git.refresh();
    await pulls.refresh();
    final reads = backend.pullRequestReads;
    final gate = backend.currentGate = Completer();
    final asked = [pulls.refresh(), pulls.refresh(), pulls.refresh()];
    await pumpEventQueue();
    expect(backend.pullRequestReads, reads + 1);
    backend.currentGate = null;
    backend.current = pullRequestOutcomeOf(pullRequest: pullRequestOf());
    gate.complete(pullRequestOutcomeOf());
    await Future.wait(asked);
    expect(backend.pullRequestReads, reads + 2);
    expect(pulls.current.value?.number, 42);
  });

  test(
    'a remote that is not on GitHub shows and offers no pull request',
    () async {
      backend.git = gitStatusOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
      );
      backend.current = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.notGitHub,
        remote: 'origin',
        host: 'gitlab.com',
      );
      await git.refresh();
      await pulls.pushed(
        transferOf(
          branch: 'feature/retry-after',
          upstream: 'origin/feature/retry-after',
        ),
      );
      expect(pulls.offer.value, isNull);
      expect(pulls.current.value, isNull);
      expect(pulls.failure.value, isNull);
    },
  );

  test(
    'a creation that ends after a branch switch leaves the new branch alone',
    () async {
      backend.git = gitStatusOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
      );
      await git.refresh();
      final gate = backend.createGate = Completer();
      final created = pulls.create(
        BridgePullRequestDraft(
          identity: planOf().identity,
          target: 'acme/api',
          base: 'main',
          title: 'Return 429',
          body: '',
          draft: false,
        ),
      );
      backend.git = gitStatusOf(branch: 'main', upstream: 'origin/main');
      await git.refresh();
      gate.complete(pullRequestOutcomeOf(pullRequest: pullRequestOf()));
      await created;
      expect(pulls.current.value, isNull);
    },
  );

  test('a read that ends after dispose changes nothing', () async {
    backend.git = gitStatusOf(
      branch: 'feature/retry-after',
      upstream: 'origin/feature/retry-after',
    );
    await git.refresh();
    final gate = backend.currentGate = Completer();
    final reading = pulls.refresh();
    await pumpEventQueue();
    pulls.dispose();
    gate.complete(pullRequestOutcomeOf(pullRequest: pullRequestOf()));
    await reading;
    pulls = PullRequests(backend, git, bases);
  });

  test('a closed session reads no more pull requests', () async {
    backend.git = gitStatusOf(
      branch: 'feature/retry-after',
      upstream: 'origin/feature/retry-after',
    );
    final session = SessionState(backend);
    await session.git.refresh();
    await session.pulls.refresh();
    final reads = backend.pullRequestReads;
    await session.dispose();
    backend.git = gitStatusOf(branch: 'main', upstream: 'origin/main');
    await session.git.refresh();
    expect(backend.pullRequestReads, reads);
  });

  test(
    'a branch that only looks like the shown one reads its own pull request',
    () async {
      backend.git = gitStatusOf(
        branch: 'fix/<U+202E>',
        rawBranch: 'fix/<U+202E>',
      );
      await git.refresh();
      await pumpEventQueue();
      final reads = backend.pullRequestReads;
      backend.git = gitStatusOf(
        branch: 'fix/<U+202E>',
        rawBranch: 'fix/\u202e',
      );
      await git.refresh();
      await pumpEventQueue();
      expect(backend.pullRequestReads, reads + 1);
    },
  );
}
