import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/move.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';

enum SealFault { none, lostAfterLanding, lostUnreadable, flakyOnce, refused }

class LoggedProject extends FakeProject {
  LoggedProject(this.log, this.name);

  final List<String> log;
  final String name;
  SealFault sealFault = SealFault.none;
  var _flaked = false;

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

  Future<void> _finish(String id, String transfer, String? toHost) {
    log.add('$name finish ${toHost ?? 'null'}');
    return super.moveFinish(id, transfer, toHost);
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async {
    if (toHost == null) return _finish(id, transfer, toHost);
    switch (sealFault) {
      case SealFault.none:
        return _finish(id, transfer, toHost);
      case SealFault.lostAfterLanding:
        await _finish(id, transfer, toHost);
        log.add('$name seal lost');
        throw 'the connection dropped';
      case SealFault.lostUnreadable:
        log.add('$name seal lost');
        moveCheckError = 'the source is unreachable';
        throw 'the connection dropped';
      case SealFault.flakyOnce:
        if (_flaked) return _finish(id, transfer, toHost);
        _flaked = true;
        log.add('$name seal lost');
        throw 'the connection dropped';
      case SealFault.refused:
        log.add('$name seal refused');
        throw 'the source is gone';
    }
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

Matcher moveFailure({String? step, String? message}) => throwsA(
  isA<MoveFailure>()
      .having((failure) => failure.step, 'step', step ?? anything)
      .having((failure) => failure.message, 'message', message ?? anything),
);

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

  final unconfirmed = throwsA(
    isA<MoveUnconfirmed>()
        .having((move) => move.session?.id, 'session', 'thread-1')
        .having(
          (move) => move.message,
          'message',
          'The move of “Fix the flaky port test” to studio is not finished yet. Liquid finishes it when This Mac answers.',
        ),
  );

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

  MoveFlow flowOf(MovePlan plan, {ProjectOpener? open}) => MoveFlow(
    open: open ?? (address, root) async => address == studio ? target : source,
    from: (address: null, root: source.root),
    to: (address: studio, root: target.root),
    threadId: 't1',
    plan: plan,
    running: running,
    started: started,
    newTransfer: () => 'transfer-1',
  );

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

  test('a move runs on its own connection to each host, and closes both when it ends', () async {
    final opened = <LoggedProject>[];
    final flow = flowOf(
      planOf(cloneRemote: null),
      open: (address, root) async {
        final own = LoggedProject(log, address == studio ? 'target' : 'source');
        opened.add(own);
        return own;
      },
    );

    await flow.run();

    expect(log, ['source out', 'target in', 'source finish studio']);
    expect(opened.map((own) => own.closed), [true, true]);
  });

  test('a host that cannot be connected fails the move before the source is touched', () async {
    await expectLater(
      flowOf(
        planOf(cloneRemote: null),
        open: (address, root) async {
          if (address == studio) throw 'the host is down';
          return source;
        },
      ).run(),
      moveFailure(
        step: 'connecting',
        message: 'Could not move “Fix the flaky port test” to studio; it stays on This Mac: the host is down.',
      ),
    );
    expect(log, isEmpty);
  });

  test('a move clones and brings the configuration first, then commits and pushes, starts on the target at the root it reports, and only then seals the source', () async {
    target.clonedRoot = '/srv/acme/api';
    final flow = flowOf(planOf(bringing: shown));

    await flow.run();

    expect(target.calls, contains('move in /srv/acme/api'));
    expect(log, [
      'target clone',
      'target configuration',
      'source out',
      'target in',
      'source finish studio',
    ]);
    expect(target.broughtChanges, same(shown));
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
      moveFailure(
        step: 'cloning the project',
        message: 'Could not move “Fix the flaky port test” to studio; it stays on This Mac: T3 could not clone the repository.',
      ),
    );
    expect(log, ['target clone']);
    expect(flow.steps.value.last.state, MoveStepState.failed);
  });

  test('a failed commit and push leaves nothing sealed, since the source releases itself', () async {
    source.moveOutError = 'the push was refused';

    await expectLater(
      flowOf(planOf()).run(),
      moveFailure(
        step: 'committing and pushing',
        message: 'Could not move “Fix the flaky port test” to studio; it stays on This Mac, the clone at /home/me/acme-api remains: the push was refused.',
      ),
    );
    expect(log, ['target clone', 'source out']);
  });

  test(
    'a failed start releases the source, so the session is shown here again',
    () async {
      target.moveInError = 'the branch is already checked out';

      await expectLater(
        flowOf(planOf(cloneRemote: null)).run(),
        moveFailure(
          step: 'starting it on studio',
          message: 'Could not move “Fix the flaky port test” to studio; it stays on This Mac, its commit is pushed: the branch is already checked out.',
        ),
      );
      expect(log, ['source out', 'target in', 'source finish null']);
      expect(target.calls, contains('move abort transfer-1'));
      expect(source.calls, contains('move finish t1 transfer-1 null'));
    },
  );

  test('a seal that keeps failing leaves the move open: the new session stays, nothing is deleted, and the move is unfinished', () async {
    source.sealFault = SealFault.refused;

    await expectLater(flowOf(planOf(cloneRemote: null)).run(), unconfirmed);
    expect(log, [
      'source out',
      'target in',
      ...List.filled(4, 'source seal refused'),
    ]);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  test('a seal whose reply was lost, but which landed, keeps the new session and reports success', () async {
    source.sealFault = SealFault.lostAfterLanding;

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
    source.sealFault = SealFault.flakyOnce;

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
    source.sealFault = SealFault.lostUnreadable;

    await expectLater(flowOf(planOf(cloneRemote: null)).run(), unconfirmed);
    expect(log, ['source out', 'target in', 'source seal lost']);
    expect(target.calls.where((call) => call.startsWith('delete')), isEmpty);
    expect(source.moving?.transfer, 'transfer-1');
  });
}
