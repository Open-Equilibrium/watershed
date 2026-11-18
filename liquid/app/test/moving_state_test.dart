import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/action_area.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';
const info = SessionInfo(project: 'acme-api');

BridgeMoving movingTo(String targetAddress, {String targetName = 'studio'}) =>
    BridgeMoving(
      transfer: 'transfer-1',
      phase: BridgeMovePhase.handed,
      targetAddress: targetAddress,
      targetName: targetName,
      targetEnvironment: fakeEnvironment,
      sinceMs: DateTime.now().millisecondsSinceEpoch,
    );

FakeProject movingSource({String targetAddress = studio}) =>
    FakeProject(session: FakeSession(id: 'moving'))
      ..moving = movingTo(targetAddress)
      ..threadList = [
        threadOf(
          'moving',
          title: 'Fix the flaky port test',
          moving: movingTo(targetAddress),
        ),
      ];

Future<SessionState> openMoving(
  WidgetTester tester,
  FakeProject source, {
  bool targetAnswers = false,
  bool runningHere = false,
  FakeProject? target,
}) async {
  await tester.binding.setSurfaceSize(const Size(1400, 900));
  final state = SessionState(source);
  final remote = target ?? FakeProject();
  state.openProject = (address, root) async {
    if (address == studio && !targetAnswers) throw 'studio does not answer';
    return address == studio ? remote : source;
  };
  if (runningHere) state.runningMoves.add('transfer-1');
  if (targetAnswers) await state.addHost(studio, remote);
  await state.loadStart();
  await state.open('moving');
  await tester.pumpWidget(
    LiquidApp(launch: ValueNotifier(Launch.session(state, info))),
  );
  await tester.pump();
  return state;
}

class HangingCheck extends FakeProject {
  HangingCheck({super.session});

  @override
  Future<BridgeMoveCheck> moveCheck(String id) =>
      Completer<BridgeMoveCheck>().future;
}

void main() {
  testWidgets(
    'Try again reloads the listing, so a move that has since finished stops showing as moving',
    (tester) async {
      final source = movingSource();
      final state = await openMoving(tester, source);

      source.threadList = [
        threadOf('moving', title: 'Fix the flaky port test'),
      ];
      await tester.tap(find.text('Try again'));
      await tester.pumpAndSettle();

      expect(find.byType(Composer), findsOneWidget);
      expect(find.text('Moving to studio.'), findsNothing);
      await state.dispose();
    },
  );

  testWidgets(
    'a moving session whose target does not answer says it waits, offers Try again and Cancel move, and drops the composer',
    (tester) async {
      final state = await openMoving(tester, movingSource());

      expect(find.text('Moving to studio.'), findsOneWidget);
      expect(
        find.text('studio does not answer, so this session waits for it.'),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsOneWidget);
      expect(find.text('Cancel move…'), findsOneWidget);
      expect(find.byType(Composer), findsNothing);
      expect(
        find.text('Moving'),
        findsOneWidget,
        reason: 'the sidebar row says Moving',
      );
      await state.dispose();
    },
  );

  testWidgets(
    'cancelling a move whose target already landed finishes it there: the session opens on the target, with a notice',
    (tester) async {
      final source = movingSource();
      final remote = FakeProject()..landedThread = 'moved-1';
      final state = await openMoving(
        tester,
        source,
        targetAnswers: true,
        target: remote,
      );

      await tester.tap(find.text('Cancel move…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel move'));
      await tester.pumpAndSettle();

      expect(source.calls, contains('move finish moving transfer-1 studio'));
      expect(
        state.notice.value?.text,
        'studio already had this session, so the move finished.',
      );
      expect(state.sessionAddress, studio);
      await state.dispose();
    },
  );

  testWidgets(
    'cancelling a move whose target is still delivering it does not release the session; the cancel waits',
    (tester) async {
      final source = movingSource();
      final remote = FakeProject()
        ..landedThread = 'moved-1'
        ..undelivered = true;
      final state = await openMoving(
        tester,
        source,
        targetAnswers: true,
        target: remote,
      );

      await tester.tap(find.text('Cancel move…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel move'));
      await tester.pumpAndSettle();

      expect(
        source.calls,
        isNot(contains('move finish moving transfer-1 null')),
      );
      expect(
        source.calls,
        isNot(contains('move finish moving transfer-1 studio')),
      );
      expect(
        state.notice.value?.text,
        'studio is still taking the session, so the move cannot be cancelled yet.',
      );
      await state.dispose();
    },
  );

  testWidgets(
    'a move to a target that answers says Moving to it with a spinner',
    (tester) async {
      final state = await openMoving(
        tester,
        movingSource(),
        targetAnswers: true,
        runningHere: true,
      );

      expect(find.text('Moving to studio…'), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
      await state.dispose();
    },
  );

  testWidgets(
    'cancelling a move asks first, and confirming releases the session here',
    (tester) async {
      final source = movingSource();
      final state = await openMoving(tester, source);

      await tester.tap(find.text('Cancel move…'));
      await tester.pumpAndSettle();
      expect(find.text('Cancel the move to studio?'), findsOneWidget);
      expect(
        find.text(
          'The session continues here. If studio already started it, it runs on both hosts until you delete one of them.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Keep waiting'));
      await tester.pumpAndSettle();
      expect(
        source.calls,
        isNot(contains('move finish moving transfer-1 null')),
      );
      expect(find.text('Moving to studio.'), findsOneWidget);

      await tester.tap(find.text('Cancel move…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel move'));
      await tester.pumpAndSettle();
      expect(source.calls, contains('move finish moving transfer-1 null'));
      expect(find.byType(Composer), findsOneWidget);
      expect(find.text('Moving to studio.'), findsNothing);
      await state.dispose();
    },
  );

  testWidgets(
    'a move another device made, whose target answers, waits without a spinner until it is five minutes old',
    (tester) async {
      final state = await openMoving(
        tester,
        movingSource(),
        targetAnswers: true,
      );

      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.text('Try again'), findsOneWidget);
      expect(
        find.text(
          'Liquid waits five minutes after the move started before it finishes it.',
        ),
        findsOneWidget,
      );
      await state.dispose();
    },
  );

  testWidgets(
    'while Liquid settles a move, the input area shows the spinner, not the waiting state',
    (tester) async {
      final source = HangingCheck(session: FakeSession(id: 'moving'))
        ..moving = movingTo(studio)
        ..threadList = [
          threadOf(
            'moving',
            title: 'Fix the flaky port test',
            moving: movingTo(studio),
          ),
        ];
      final state = await openMoving(tester, source);

      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
      await state.dispose();
    },
  );
}
