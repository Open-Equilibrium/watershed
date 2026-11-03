import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/branch_bases.dart';
import 'package:liquid/src/session/git_state.dart';
import 'package:liquid/src/session/pull_requests.dart';
import 'package:liquid/src/settings_file.dart';

import 'fake_backend.dart';

void main() {
  late Directory dir;
  late FakeBackend backend;
  late BranchBases bases;
  late GitState git;
  late PullRequests pulls;

  BranchBases stored() =>
      BranchBases(SettingsFile(File('${dir.path}/git-bases.json')));

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
    expect(pulls.failure.value, contains('offline'));

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
}
