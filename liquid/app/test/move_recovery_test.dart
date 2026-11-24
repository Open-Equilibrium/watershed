import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/session/move_rule.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';
const sealCall = 'move finish thread-1 transfer-1 studio';
const releaseCall = 'move finish thread-1 transfer-1 null';
const aged = Duration(minutes: 6);

BridgeMoving movingAt(DateTime since) => BridgeMoving(
  transfer: 'transfer-1',
  phase: BridgeMovePhase.handed,
  targetAddress: studio,
  targetName: 'studio',
  targetEnvironment: fakeEnvironment,
  sinceMs: since.millisecondsSinceEpoch,
);

BridgeMoving movingSince(Duration age) =>
    movingAt(DateTime.now().subtract(age));

void main() {
  late FakeProject source;
  late FakeProject target;

  setUp(() {
    source = FakeProject(session: FakeSession(id: 'local'));
    target = FakeProject(session: FakeSession(id: 'moved'));
  });

  void listMoving(BridgeMoving moving) => source
    ..moving = moving
    ..threadList = [
      threadOf('thread-1', title: 'Fix the flaky port test', moving: moving),
    ];

  Iterable<String> sourceFinishes() =>
      source.calls.where((call) => call.startsWith('move finish'));

  Future<SessionState> listed(
    BridgeMoving moving, {
    Set<String> running = const {},
    Set<String> started = const {},
    Object? openError,
  }) async {
    listMoving(moving);
    final state = SessionState(source)
      ..runningMoves.addAll(running)
      ..startedMoves.addAll(started)
      ..openProject = (address, root) async {
        if (openError != null) throw openError;
        return address == studio ? target : source;
      };
    await state.addHost(studio, target);
    await state.loadStart();
    await pumpEventQueue();
    return state;
  }

  test('a move this device started is settled at once when listed again, not after five minutes', () async {
    target.landedThread = 'moved-1';

    await listed(movingSince(Duration.zero), started: {'transfer-1'});

    expect(source.calls, contains(sealCall));
  });

  testWidgets(
    'a move another device made is left alone for five minutes, then recovered without a reload',
    (tester) async {
      target.landedThread = 'moved-1';
      final start = DateTime(2026, 11, 18, 12);
      var now = start;
      listMoving(movingAt(start));
      final state = SessionState(source)
        ..clock = (() => now)
        ..openProject = (address, root) async =>
            address == studio ? target : source;
      await state.addHost(studio, target);
      await state.loadStart();

      now = start.add(const Duration(minutes: 4, seconds: 59));
      await tester.pump(const Duration(minutes: 4, seconds: 59));
      expect(source.calls, isNot(contains(sealCall)));
      expect(target.calls, isNot(contains('move abort transfer-1')));

      now = start.add(recoveryAge);
      await tester.pump(const Duration(seconds: 1));
      expect(source.calls, contains(sealCall));
      await state.dispose();
    },
  );

  test('a move is settled only by the host it was made to: a host at the target address with another environment id is unreachable', () async {
    target.landedThread = 'moved-1';
    target.environment = 'env-other';

    await listed(movingSince(aged));

    expect(source.calls, isNot(contains(sealCall)));
    expect(target.calls, isNot(contains('move status transfer-1')));
  });

  test('a target that answers starting keeps a listed move open: it is neither aborted nor released', () async {
    target.startingThread = true;

    await listed(movingSince(aged));

    expect(target.calls, contains('move status transfer-1'));
    expect(target.calls, isNot(contains('move abort transfer-1')));
    expect(source.calls, isNot(contains(releaseCall)));
    expect(source.moving, isNotNull);
  });

  test(
    'a listed move the target landed is sealed on the source, with a notice',
    () async {
      target.landedThread = 'moved-1';

      final state = await listed(movingSince(aged));

      expect(source.calls, contains(sealCall));
      expect(
        state.notice.value?.text,
        'Moved “Fix the flaky port test” to studio.',
      );
      expect(state.notice.value?.level, NoticeLevel.info);
    },
  );

  test('a listed move the target did not land is released, and the session stays where it was', () async {
    final state = await listed(movingSince(aged));

    expect(target.calls, contains('move abort transfer-1'));
    expect(source.calls, contains(releaseCall));
    expect(
      state.notice.value?.text,
      'The move of “Fix the flaky port test” to studio did not happen; the session stayed on This Mac.',
    );
    expect(state.notice.value?.level, NoticeLevel.warning);
  });

  test('a move this device is running is left to its flow', () async {
    target.landedThread = 'moved-1';

    await listed(movingSince(aged), running: {'transfer-1'});

    expect(sourceFinishes(), isEmpty);
  });

  testWidgets(
    'an overlapping recovery scan keeps the five-minute recovery of a young move',
    (tester) async {
      final start = DateTime(2026, 11, 18, 12);
      var now = start;
      final gates = {
        for (var index = 0; index < 3; index++) index: Completer<void>(),
      };
      final slow = _GatedEnvironments(gates);
      BridgeMoving movingOf(String transfer, DateTime since) => BridgeMoving(
        transfer: transfer,
        phase: BridgeMovePhase.handed,
        targetAddress: studio,
        targetName: 'studio',
        targetEnvironment: fakeEnvironment,
        sinceMs: since.millisecondsSinceEpoch,
      );
      final ms = start.millisecondsSinceEpoch;
      source
        ..moving = movingOf('transfer-2', start)
        ..threadList = [
          threadOf(
            'thread-1',
            updatedMs: ms,
            moving: movingOf('transfer-1', start.subtract(aged)),
          ),
          threadOf(
            'thread-2',
            updatedMs: ms - 1,
            moving: movingOf('transfer-2', start),
          ),
          threadOf(
            'thread-3',
            updatedMs: ms - 2,
            moving: movingOf('transfer-3', start.subtract(aged)),
          ),
        ];
      final state = SessionState(source)
        ..clock = (() => now)
        ..openProject = (address, root) async =>
            address == studio ? slow : source;
      await state.addHost(studio, slow);
      int youngChecks() =>
          slow.calls.where((call) => call == 'move status transfer-2').length;

      await state.loadStart();
      await tester.runAsync(pumpEventQueue);
      unawaited(state.recoverMoves());
      await tester.runAsync(pumpEventQueue);
      gates[0]!.complete();
      await tester.runAsync(pumpEventQueue);
      gates[1]!.complete();
      await tester.runAsync(pumpEventQueue);
      gates[2]!.complete();
      await tester.runAsync(pumpEventQueue);

      final before = youngChecks();
      now = start.add(recoveryAge);
      await tester.pump(recoveryAge);
      expect(youngChecks(), greaterThan(before));
      await state.dispose();
    },
  );
}

class _GatedEnvironments extends FakeProject {
  _GatedEnvironments(this.gates);

  final Map<int, Completer<void>> gates;
  var _reads = 0;

  @override
  Future<String> environmentId() async {
    await gates[_reads++]?.future;
    return environment;
  }
}
