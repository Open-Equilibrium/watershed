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
const releaseCall = 'move finish moving transfer-1 null';
const sealCall = 'move finish moving transfer-1 studio';

FakeProject movingSource({FakeProject? project}) {
  final moving = BridgeMoving(
    transfer: 'transfer-1',
    phase: BridgeMovePhase.handed,
    targetAddress: studio,
    targetName: 'studio',
    targetEnvironment: fakeEnvironment,
    sinceMs: DateTime.now().millisecondsSinceEpoch,
  );
  return (project ?? FakeProject(session: FakeSession(id: 'moving')))
    ..moving = moving
    ..threadList = [
      threadOf('moving', title: 'Fix the flaky port test', moving: moving),
    ];
}

Future<SessionState> openMoving(
  WidgetTester tester,
  FakeProject source, {
  FakeProject? answering,
  bool runningHere = false,
}) async {
  await tester.binding.setSurfaceSize(const Size(1400, 900));
  final state = SessionState(source);
  final remote = answering ?? FakeProject();
  state.openProject = (address, root) async {
    if (address == studio && answering == null) throw 'studio does not answer';
    return address == studio ? remote : source;
  };
  if (runningHere) state.runningMoves.add('transfer-1');
  if (answering != null) await state.addHost(studio, remote);
  await state.loadStart();
  await state.open('moving');
  await tester.pumpWidget(
    LiquidApp(launch: ValueNotifier(Launch.session(state, info))),
  );
  await tester.pump();
  return state;
}

Future<void> tapAndSettle(WidgetTester tester, String text) async {
  await tester.tap(find.text(text));
  await tester.pumpAndSettle();
}

Future<void> cancelMove(WidgetTester tester) async {
  await tapAndSettle(tester, 'Cancel move…');
  await tapAndSettle(tester, 'Cancel move');
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
      await tapAndSettle(tester, 'Try again');

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
      final state = await openMoving(tester, source, answering: remote);

      await cancelMove(tester);

      expect(source.calls, contains(sealCall));
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
      final state = await openMoving(tester, source, answering: remote);

      await cancelMove(tester);

      expect(source.calls, isNot(contains(releaseCall)));
      expect(source.calls, isNot(contains(sealCall)));
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
        answering: FakeProject(),
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

      await tapAndSettle(tester, 'Cancel move…');
      expect(find.text('Cancel the move to studio?'), findsOneWidget);
      expect(
        find.text(
          'The session continues here. If studio already started it, it runs on both hosts until you delete one of them.',
        ),
        findsOneWidget,
      );
      await tapAndSettle(tester, 'Keep waiting');
      expect(source.calls, isNot(contains(releaseCall)));
      expect(find.text('Moving to studio.'), findsOneWidget);

      await cancelMove(tester);
      expect(source.calls, contains(releaseCall));
      expect(find.byType(Composer), findsOneWidget);
      expect(find.text('Moving to studio.'), findsNothing);
      await state.dispose();
    },
  );

  testWidgets(
    'while Liquid settles a move, the input area shows the spinner, not the waiting state',
    (tester) async {
      final state = await openMoving(
        tester,
        movingSource(
          project: HangingCheck(session: FakeSession(id: 'moving')),
        ),
      );

      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
      await state.dispose();
    },
  );
}
