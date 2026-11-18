import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/session/move_rule.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';

BridgeMoving movingSince(Duration age, {String transfer = 'transfer-1'}) =>
    BridgeMoving(
      transfer: transfer,
      phase: BridgeMovePhase.handed,
      targetAddress: studio,
      targetName: 'studio',
      targetEnvironment: fakeEnvironment,
      sinceMs: DateTime.now().subtract(age).millisecondsSinceEpoch,
    );

void main() {
  late FakeProject source;
  late FakeProject target;

  setUp(() {
    source = FakeProject(session: FakeSession(id: 'local'));
    target = FakeProject(session: FakeSession(id: 'moved'));
  });

  Future<SessionState> listed(
    BridgeMoving moving, {
    Set<String>? running,
    Set<String>? started,
    Object? openError,
  }) async {
    source
      ..moving = moving
      ..threadList = [
        threadOf('thread-1', title: 'Fix the flaky port test', moving: moving),
      ];
    final state = SessionState(source)
      ..runningMoves.addAll(running ?? const {})
      ..startedMoves.addAll(started ?? const {})
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

    expect(source.calls, contains('move finish thread-1 transfer-1 studio'));
  });

  testWidgets(
    'a move another device made is left alone for five minutes, then recovered without a reload',
    (tester) async {
      target.landedThread = 'moved-1';
      final start = DateTime(2026, 11, 18, 12);
      var now = start;
      final moving = BridgeMoving(
        transfer: 'transfer-1',
        phase: BridgeMovePhase.handed,
        targetAddress: studio,
        targetName: 'studio',
        targetEnvironment: fakeEnvironment,
        sinceMs: start.millisecondsSinceEpoch,
      );
      source
        ..moving = moving
        ..threadList = [
          threadOf(
            'thread-1',
            title: 'Fix the flaky port test',
            moving: moving,
          ),
        ];
      final state = SessionState(source)
        ..clock = (() => now)
        ..openProject = (address, root) async =>
            address == studio ? target : source;
      await state.addHost(studio, target);
      await state.loadStart();

      now = start.add(const Duration(minutes: 4, seconds: 59));
      await tester.pump(const Duration(minutes: 4, seconds: 59));
      expect(
        source.calls,
        isNot(contains('move finish thread-1 transfer-1 studio')),
      );

      now = start.add(recoveryAge);
      await tester.pump(const Duration(seconds: 1));
      expect(source.calls, contains('move finish thread-1 transfer-1 studio'));
      await state.dispose();
    },
  );

  test('a move is settled only by the host it was made to: a host at the target address with another environment id is unreachable', () async {
    target.landedThread = 'moved-1';
    target.environment = 'env-other';

    await listed(movingSince(const Duration(minutes: 6)));

    expect(
      source.calls,
      isNot(contains('move finish thread-1 transfer-1 studio')),
    );
    expect(target.calls, isNot(contains('move status transfer-1')));
  });

  test('a target that answers starting keeps a listed move open: it is neither aborted nor released', () async {
    target.startingThread = true;

    await listed(movingSince(const Duration(minutes: 6)));

    expect(target.calls, contains('move status transfer-1'));
    expect(target.calls, isNot(contains('move abort transfer-1')));
    expect(
      source.calls,
      isNot(contains('move finish thread-1 transfer-1 null')),
    );
    expect(source.moving, isNotNull);
  });

  test(
    'a listed move the target landed is sealed on the source, with a notice',
    () async {
      target.landedThread = 'moved-1';

      final state = await listed(movingSince(const Duration(minutes: 6)));

      expect(source.calls, contains('move finish thread-1 transfer-1 studio'));
      expect(
        state.notice.value?.text,
        'Moved “Fix the flaky port test” to studio.',
      );
      expect(state.notice.value?.level, NoticeLevel.info);
    },
  );

  test('a listed move the target did not land is released, and the session stays where it was', () async {
    final state = await listed(movingSince(const Duration(minutes: 6)));

    expect(target.calls, contains('move abort transfer-1'));
    expect(source.calls, contains('move finish thread-1 transfer-1 null'));
    expect(
      state.notice.value?.text,
      'The move of “Fix the flaky port test” to studio did not happen; the session stayed on This Mac.',
    );
    expect(state.notice.value?.level, NoticeLevel.warning);
  });

  test('a move another device is making is left alone until it is five minutes old', () async {
    target.landedThread = 'moved-1';

    await listed(movingSince(const Duration(minutes: 4, seconds: 59)));

    expect(
      source.calls.where((call) => call.startsWith('move finish')),
      isEmpty,
    );
    expect(
      target.calls.where((call) => call.startsWith('move abort')),
      isEmpty,
    );
  });

  test('a move this device is running is left to its flow', () async {
    target.landedThread = 'moved-1';

    await listed(
      movingSince(const Duration(minutes: 6)),
      running: {'transfer-1'},
    );

    expect(
      source.calls.where((call) => call.startsWith('move finish')),
      isEmpty,
    );
  });

  test('a target that does not answer leaves the session moving', () async {
    final state = await listed(
      movingSince(const Duration(minutes: 6)),
      openError: 'the host is down',
    );

    expect(
      source.calls.where((call) => call.startsWith('move finish')),
      isEmpty,
    );
    expect(state.notice.value, isNull);
  });
}
