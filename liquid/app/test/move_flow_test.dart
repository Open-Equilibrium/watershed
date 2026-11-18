import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/move.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';

class LoggedProject extends FakeProject {
  LoggedProject(this.log, this.name);

  final List<String> log;
  final String name;

  @override
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  }) {
    log.add('$name out');
    return super.moveOut(
      id,
      transfer: transfer,
      targetAddress: targetAddress,
      targetName: targetName,
      targetEnvironment: targetEnvironment,
      commit: commit,
    );
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) {
    log.add('$name finish ${toHost ?? 'null'}');
    return super.moveFinish(id, transfer, toHost);
  }

  @override
  Future<BridgeHostProject> cloneProject(String remoteUrl, String path) {
    log.add('$name clone');
    return super.cloneProject(remoteUrl, path);
  }

  @override
  Future<void> bringConfiguration(
    ProjectBackend from,
    ConfigurationChanges shown,
  ) {
    log.add('$name configuration');
    return super.bringConfiguration(from, shown);
  }

  @override
  Future<SessionBackend> moveIn(CarriedMove move, {required String root}) {
    log.add('$name in');
    return super.moveIn(move, root: root);
  }
}

class _SealLostLanded extends LoggedProject {
  _SealLostLanded(super.log, super.name);

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async {
    if (toHost == null) return super.moveFinish(id, transfer, toHost);
    await super.moveFinish(id, transfer, toHost);
    log.add('$name seal lost');
    throw 'the connection dropped';
  }
}

class _SealLostUnreadable extends LoggedProject {
  _SealLostUnreadable(super.log, super.name);

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) {
    if (toHost == null) return super.moveFinish(id, transfer, toHost);
    log.add('$name seal lost');
    moveCheckError = 'the source is unreachable';
    return Future.error('the connection dropped');
  }
}

class _SealFlaky extends LoggedProject {
  _SealFlaky(super.log, super.name);

  var _failures = 1;

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) {
    if (toHost != null && _failures-- > 0) {
      log.add('$name seal lost');
      return Future.error('the connection dropped');
    }
    return super.moveFinish(id, transfer, toHost);
  }
}

class _SealRefused extends LoggedProject {
  _SealRefused(super.log, super.name);

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) {
    if (toHost != null) {
      log.add('$name seal refused');
      return Future.error('the source is gone');
    }
    return super.moveFinish(id, transfer, toHost);
  }
}

void main() {
  late List<String> log;
  late LoggedProject source;
  late LoggedProject target;
  late Set<String> running;
  late Set<String> started;

  setUp(() {
    log = [];
    source = LoggedProject(log, 'source');
    target = LoggedProject(log, 'target');
    running = <String>{};
    started = <String>{};
  });

  final shown = FakeConfigurationChanges([
    const BridgeConfigChange(
      kind: BridgeConfigKind.skill,
      name: 'review',
      direction: BridgeDirection.add,
    ),
  ]);

  MovePlan planOf({
    bool running = false,
    bool commit = true,
    String? cloneRemote = 'git@github.com:acme/api.git',
    ConfigurationChanges? bringing,
  }) => MovePlan(
    title: 'Fix the flaky port test',
    sourceName: 'This Mac',
    targetName: 'studio',
    branch: 'fix/ports',
    uncommitted: 3,
    running: running,
    commit: commit,
    root: '/home/me/acme-api',
    cloneRemote: cloneRemote,
    bringing: bringing,
  );

  MoveFlow flowOf(MovePlan plan) => MoveFlow(
    open: (address, root) async => address == studio ? target : source,
    from: (address: null, root: source.root),
    to: (address: studio, root: target.root),
    threadId: 't1',
    plan: plan,
    running: running,
    started: started,
    newTransfer: () => 'transfer-1',
  );

  test('a cloned project moves in at the root the target reports', () async {
    target.clonedRoot = '/srv/acme/api';
    await flowOf(planOf()).run();
    expect(target.calls, contains('move in /srv/acme/api'));
  });

  test('a move out names the environment id of the host it moves to', () async {
    target.environment = 'env-studio';

    await flowOf(planOf(cloneRemote: null)).run();

    expect(source.moveOutEnvironment, 'env-studio');
  });

  test('a move-in error is not a refusal: a target that answers starting keeps the move waiting, never released', () async {
    target.moveInError = 'The moved messages are not all delivered yet, so the session has not finished moving; try again.';
    target.startingThread = true;

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(isA<MoveUnconfirmed>()),
    );

    expect(
      source.moving?.transfer,
      'transfer-1',
      reason: 'the move stays open',
    );
    expect(target.calls, isNot(contains('move abort transfer-1')));
    expect(source.calls, isNot(contains('move finish t1 transfer-1 null')));
  });

  test('a move-in error after the target landed seals the source and returns the session', () async {
    target.moveInError = 'Meta-Harness failed the move-in.';
    target.landedThread = 'moved-1';

    final session = await flowOf(planOf(cloneRemote: null)).run();

    expect(session.id, target.session.id);
    expect(source.calls, contains('move finish t1 transfer-1 studio'));
  });

  test('a move-in error the target does not list is aborted, and the source released', () async {
    target.moveInError = 'This project has no Git remote that T3 can resolve, so the session cannot move here.';

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(isA<MoveFailure>()),
    );

    expect(target.calls, contains('move abort transfer-1'));
    expect(source.calls, contains('move finish t1 transfer-1 null'));
  });

  test('a move runs on its own connection to each host, and closes both when it ends', () async {
    final opened = <LoggedProject>[];
    final flow = MoveFlow(
      open: (address, root) async {
        final own = LoggedProject(log, address == studio ? 'target' : 'source');
        opened.add(own);
        return own;
      },
      from: (address: null, root: '/work/acme-api'),
      to: (address: studio, root: '/home/me/acme-api'),
      threadId: 't1',
      plan: planOf(cloneRemote: null),
      running: running,
      started: started,
      newTransfer: () => 'transfer-1',
    );

    await flow.run();

    expect(log, ['source out', 'target in', 'source finish studio']);
    expect(opened.map((own) => own.closed), [true, true]);
  });

  test('a host that cannot be connected fails the move before the source is touched', () async {
    final flow = MoveFlow(
      open: (address, root) async {
        if (address == studio) throw 'the host is down';
        return source;
      },
      from: (address: null, root: '/work/acme-api'),
      to: (address: studio, root: '/home/me/acme-api'),
      threadId: 't1',
      plan: planOf(cloneRemote: null),
      running: running,
      started: started,
      newTransfer: () => 'transfer-1',
    );

    await expectLater(
      flow.run(),
      throwsA(
        isA<MoveFailure>()
            .having((failure) => failure.step, 'step', 'connecting')
            .having(
              (failure) => failure.message,
              'message',
              'Could not move “Fix the flaky port test” to studio; it stays on This Mac: the host is down.',
            ),
      ),
    );
    expect(log, isEmpty);
  });

  test('a move clones and brings the configuration first, then commits and pushes, starts on the target, and only then seals the source', () async {
    final flow = flowOf(planOf(bringing: shown));

    await flow.run();

    expect(log, [
      'target clone',
      'target configuration',
      'source out',
      'target in',
      'source finish studio',
    ]);
    expect(flow.steps.value.map((step) => step.text), [
      'Cloned the project to /home/me/acme-api on studio',
      'Brought the configuration to studio',
      'Committed 3 files and pushed fix/ports to its remote',
      'Started the session on studio',
    ]);
    expect(flow.steps.value.map((step) => step.state).toSet(), {
      MoveStepState.done,
    });
  });

  test('the configuration brought is the change set that was shown', () async {
    await flowOf(planOf(bringing: shown, cloneRemote: null)).run();

    expect(target.broughtChanges, same(shown));
  });

  test(
    'a running turn ends before the move, and the steps say so first',
    () async {
      final flow = flowOf(planOf(running: true, cloneRemote: null));

      await flow.run();

      expect(flow.steps.value.first.text, 'The current turn ended');
      expect(log, ['source out', 'target in', 'source finish studio']);
    },
  );

  test('a failed clone leaves the source untouched, and the failure names the step and says the session stayed here', () async {
    target.cloneError = 'T3 could not clone the repository';
    final flow = flowOf(planOf());

    await expectLater(
      flow.run(),
      throwsA(
        isA<MoveFailure>()
            .having((failure) => failure.step, 'step', 'cloning the project')
            .having(
              (failure) => failure.message,
              'message',
              'Could not move “Fix the flaky port test” to studio; it stays on This Mac: T3 could not clone the repository.',
            ),
      ),
    );
    expect(log, ['target clone']);
    expect(flow.steps.value.last.state, MoveStepState.failed);
  });

  test('a failed commit and push leaves nothing sealed, since the source releases itself', () async {
    source.moveOutError = 'the push was refused';
    final flow = flowOf(planOf());

    await expectLater(
      flow.run(),
      throwsA(
        isA<MoveFailure>()
            .having((failure) => failure.step, 'step', 'committing and pushing')
            .having(
              (failure) => failure.message,
              'message',
              'Could not move “Fix the flaky port test” to studio; it stays on This Mac, the clone at /home/me/acme-api remains: the push was refused.',
            ),
      ),
    );
    expect(log, ['target clone', 'source out']);
  });

  test('a failed push after the configuration was brought says the configuration is already there', () async {
    source.moveOutError = 'the push was refused';

    await expectLater(
      flowOf(planOf(bringing: shown, cloneRemote: null)).run(),
      throwsA(
        isA<MoveFailure>().having(
          (failure) => failure.message,
          'message',
          'Could not move “Fix the flaky port test” to studio; it stays on This Mac, the configuration is already brought to studio: the push was refused.',
        ),
      ),
    );
  });

  test('a failed push with Leave on This Mac is named as pushing', () async {
    source.moveOutError = 'the push was refused';

    await expectLater(
      flowOf(planOf(commit: false, cloneRemote: null)).run(),
      throwsA(
        isA<MoveFailure>().having((failure) => failure.step, 'step', 'pushing'),
      ),
    );
  });

  test(
    'a failed start releases the source, so the session is shown here again',
    () async {
      target.moveInError = 'the branch is already checked out';
      final flow = flowOf(planOf(cloneRemote: null));

      await expectLater(
        flow.run(),
        throwsA(
          isA<MoveFailure>()
              .having(
                (failure) => failure.step,
                'step',
                'starting it on studio',
              )
              .having(
                (failure) => failure.message,
                'message',
                'Could not move “Fix the flaky port test” to studio; it stays on This Mac, its commit is pushed: the branch is already checked out.',
              ),
        ),
      );
      expect(log, ['source out', 'target in', 'source finish null']);
    },
  );

  test('a seal that keeps failing leaves the move open: the new session stays, nothing is deleted, and the move is unfinished', () async {
    source = _SealRefused(log, 'source');

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(
        isA<MoveUnconfirmed>()
            .having(
              (unconfirmed) => unconfirmed.session?.id,
              'session',
              'thread-1',
            )
            .having(
              (unconfirmed) => unconfirmed.message,
              'message',
              'The move of “Fix the flaky port test” to studio is not finished yet. Liquid finishes it when This Mac answers.',
            ),
      ),
    );
    expect(log, [
      'source out',
      'target in',
      'source seal refused',
      'source seal refused',
      'source seal refused',
      'source seal refused',
    ]);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  test('a seal whose reply was lost, but which landed, keeps the new session and reports success', () async {
    source = _SealLostLanded(log, 'source');

    final session = await flowOf(planOf(cloneRemote: null)).run();

    expect(session.id, 'thread-1');
    expect(log, [
      'source out',
      'target in',
      'source finish studio',
      'source seal lost',
    ]);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  test('a move-in whose reply was lost, but which landed, takes the landed session', () async {
    target.moveInLostReplies = 1;
    target.lostRepliesLanded = true;

    final session = await flowOf(planOf(cloneRemote: null)).run();

    expect(session.id, 'thread-1');
    expect(log, ['source out', 'target in', 'source finish studio']);
    expect(running, isEmpty);
    expect(source.moving, isNull);
  });

  test('a move-in whose reply was lost, and that never landed, is aborted and the source released', () async {
    target.moveInLostReplies = 1;

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(isA<MoveFailure>()),
    );

    expect(log, ['source out', 'target in', 'source finish null']);
    expect(target.calls, contains('move abort transfer-1'));
    expect(source.moving, isNull);
    expect(running, isEmpty);
  });
  test('a move-in whose reply stays lost, and that never landed, is aborted and released', () async {
    target.moveInLostReplies = 3;

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(isA<MoveFailure>()),
    );

    expect(log, ['source out', 'target in', 'source finish null']);
    expect(target.calls, contains('move abort transfer-1'));
    expect(source.moving, isNull);
    expect(running, isEmpty);
  });
  test('a move-out whose reply was lost is settled by the rule: the target has not landed, so the move is aborted and the source released', () async {
    source.moveOutLostAfterHandover = true;

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(isA<MoveFailure>()),
    );

    expect(log, ['source out', 'source finish null']);
    expect(target.calls, contains('move abort transfer-1'));
    expect(source.calls, contains('move finish t1 transfer-1 null'));
  });

  test('a seal that fails while the source is reachable and has not moved is tried again', () async {
    source = _SealFlaky(log, 'source');

    final session = await flowOf(planOf(cloneRemote: null)).run();

    expect(session.id, 'thread-1');
    expect(log, [
      'source out',
      'target in',
      'source seal lost',
      'source finish studio',
    ]);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
    expect(running, isEmpty);
    expect(source.moving, isNull);
  });

  test('a seal whose reply was lost, when the source cannot be read, keeps the new session, leaves the source alone, and says the move is unconfirmed', () async {
    source = _SealLostUnreadable(log, 'source');

    await expectLater(
      flowOf(planOf(cloneRemote: null)).run(),
      throwsA(
        isA<MoveUnconfirmed>()
            .having(
              (unconfirmed) => unconfirmed.session?.id,
              'session',
              'thread-1',
            )
            .having(
              (unconfirmed) => unconfirmed.message,
              'message',
              'The move of “Fix the flaky port test” to studio is not finished yet. Liquid finishes it when This Mac answers.',
            ),
      ),
    );
    expect(log, ['source out', 'target in', 'source seal lost']);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
    expect(source.moving?.transfer, 'transfer-1');
  });

  test('the seal names the transfer that move out began', () async {
    await flowOf(planOf(cloneRemote: null)).run();

    expect(source.calls, contains('move finish t1 transfer-1 studio'));
  });

  test(
    'a release after a failed start names the transfer that move out began',
    () async {
      target.moveInError = 'the branch is already checked out';

      await expectLater(
        flowOf(planOf(cloneRemote: null)).run(),
        throwsA(isA<MoveFailure>()),
      );

      expect(source.calls, contains('move finish t1 transfer-1 null'));
    },
  );
}
