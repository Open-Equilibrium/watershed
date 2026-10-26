import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/agent/start_page.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(
    WidgetTester tester, {
    void Function()? setUp,
    Size size = const Size(1600, 900),
  }) async {
    await tester.binding.setSurfaceSize(size);
    backend = FakeBackend()
      ..modelList = const [
        BridgeModel(provider: 'faux', id: 'faux-1', name: 'Faux One'),
        BridgeModel(provider: 'faux', id: 'faux-2', name: 'Faux Two'),
      ]
      ..model = 'faux/faux-1';
    setUp?.call();
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> newSessionKey(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  testWidgets('offers recent sessions on the start page and continues one', (
    tester,
  ) async {
    final twoHoursAgo = DateTime.now()
        .subtract(const Duration(hours: 2, minutes: 5))
        .millisecondsSinceEpoch;
    await show(
      tester,
      setUp: () => backend.storedSessions = [
        BridgeStoredSession(
          path: '/sessions/a.jsonl',
          title: 'Rate limit for /login',
          modifiedMs: twoHoursAgo,
        ),
      ],
    );
    expect(find.text('New agent session'), findsOneWidget);
    expect(find.text('Recent in acme-api'), findsOneWidget);
    expect(find.text('continue · 2 h'), findsOneWidget);
    expect(find.text('Start'), findsOneWidget);
    expect(find.text(localDevice), findsOneWidget);

    await tester.tap(find.text('Rate limit for /login'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('switch /sessions/a.jsonl'));
    expect(
      backend.calls.last,
      'history',
      reason: 'the chat of the continued session is read',
    );
  });

  testWidgets('starts a new session with Cmd+N only while idle', (
    tester,
  ) async {
    await show(tester);
    await newSessionKey(tester);
    expect(backend.calls, contains('new session'));

    backend.calls.clear();
    backend.controller.add(event(BridgeEventKind.turnStarted, turn: 1));
    await tester.pumpAndSettle();
    await newSessionKey(tester);
    expect(backend.calls, isNot(contains('new session')));
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (BridgeLevel.info, 'Stop the agent before starting a new session.'),
    );
    await tester.pump(const Duration(seconds: 7));
    expect(state.notice.value, isNull, reason: 'information fades');
  });

  testWidgets('picks the model on the start page', (tester) async {
    await show(tester);
    expect(find.text('Faux One'), findsOneWidget);
    await tester.tap(find.text('Faux One'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Faux Two').last);
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'model faux/faux-2');
    expect(find.text('Faux Two'), findsOneWidget);
  });

  testWidgets('sizes each chip to its label', (tester) async {
    await show(tester);
    // The chips in the input area, not the project's name in the sidebar.
    double width(String label) => tester
        .getSize(
          find
              .ancestor(
                of: find.descendant(
                  of: find.byType(StartChips),
                  matching: find.text(label),
                ),
                matching: find.byType(Container),
              )
              .first,
        )
        .width;
    expect(
      width('acme-api'),
      lessThan(width('Faux One')),
      reason: 'compact chips, as the approved mockup shows',
    );
  });

  testWidgets('keeps Start reachable with long names in a narrow window', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    backend = FakeBackend()
      ..modelList = const [
        BridgeModel(
          provider: 'faux',
          id: 'faux-1',
          name: 'A model with a remarkably long display name',
        ),
      ]
      ..model = 'faux/faux-1';
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: const SessionInfo(
            project: 'a-project-with-a-rather-long-directory-name',
            harness: 'pi',
            device: 'MacBook',
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull, reason: 'nothing overflows');
    await tester.enterText(find.byKey(const Key('composer')), 'Go');
    await tester.tap(find.text('Start'));
    await tester.pump();
    expect(backend.calls, contains('send Go'));
  });

  testWidgets('scrolls a full list of recent sessions in a small window', (
    tester,
  ) async {
    final now = DateTime.now().millisecondsSinceEpoch;
    await show(
      tester,
      size: const Size(800, 500),
      setUp: () => backend.storedSessions = [
        for (var number = 1; number <= 8; number++)
          BridgeStoredSession(
            path: '/sessions/$number.jsonl',
            title: 'Session $number',
            modifiedMs: now,
          ),
      ],
    );
    expect(tester.takeException(), isNull, reason: 'nothing overflows');
    await tester.scrollUntilVisible(
      find.text('Session 8'),
      40,
      scrollable: find
          .ancestor(
            of: find.text('Session 1'),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    await tester.tap(find.text('Session 8'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('switch /sessions/8.jsonl'));
  });
}
