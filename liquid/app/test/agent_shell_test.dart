import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(
  title: 'Rate limit for /login',
  project: 'acme-api',
  harness: 'pi',
  device: 'MacBook',
);

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend();
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
  }

  Future<void> emit(WidgetTester tester, BridgeEvent event) async {
    backend.controller.add(event);
    await tester.pumpAndSettle();
  }

  BridgeRequest approval(String id) => BridgeRequest(
    id: id,
    approval: BridgeApproval(
      kind: BridgeRequestKind.commandExecution,
      tool: 'bash',
      arguments: '{"command":"npm test -- login"}',
      readOnly: false,
    ),
  );

  testWidgets('focuses the input area with ⌘L from any tab', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    final composer = tester.widget<TextField>(
      find.byKey(const Key('composer')),
    );
    expect(composer.focusNode?.hasFocus, isTrue);
  });

  testWidgets('takes the same handed-back text again after it was sent', (
    tester,
  ) async {
    await show(tester);
    final composer = find.byKey(const Key('composer'));
    for (var round = 0; round < 2; round++) {
      await emit(tester, event(BridgeEventKind.inputTextSet, text: 'Retry'));
      expect(
        tester.widget<TextField>(composer).controller?.text,
        'Retry',
        reason: 'round $round',
      );
      await tester.enterText(composer, '');
    }
  });

  testWidgets('decides approvals from the keyboard', (tester) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    for (final (id, keys, decision) in [
      ('a1', [LogicalKeyboardKey.enter], 'allowOnce'),
      (
        'a2',
        [LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.enter],
        'allowSession',
      ),
      ('a3', [LogicalKeyboardKey.escape], 'reject'),
    ]) {
      await emit(
        tester,
        event(BridgeEventKind.requestOpened, request: approval(id)),
      );
      expect(find.text('Run a shell command?'), findsOneWidget);
      expect(find.textContaining('npm test -- login'), findsWidgets);
      for (final key in keys) {
        await tester.sendKeyDownEvent(key);
      }
      for (final key in keys.reversed) {
        await tester.sendKeyUpEvent(key);
      }
      await tester.pump();
      expect(backend.calls.last, 'decide $id $decision');
      await emit(
        tester,
        event(BridgeEventKind.requestResolved, request: BridgeRequest(id: id)),
      );
    }
    expect(find.text('Run a shell command?'), findsNothing);
    expect(find.byKey(const Key('composer')), findsOneWidget);
  });

  testWidgets('sends from the composer and steers a running turn', (
    tester,
  ) async {
    await show(tester);
    await tester.enterText(find.byKey(const Key('composer')), 'Say hi');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(backend.calls, ['prompt Say hi']);
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    await tester.enterText(find.byKey(const Key('composer')), 'Shorter');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(backend.calls.last, 'steer Shorter');
  });

  testWidgets(
    'shows streamed text, tool rows, widgets and the waiting status',
    (tester) async {
      await show(tester);
      await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
      await emit(
        tester,
        event(
          BridgeEventKind.itemStarted,
          turn: 1,
          item: item(1, kind: BridgeItemKind.assistantMessage),
        ),
      );
      await emit(
        tester,
        event(
          BridgeEventKind.contentDelta,
          item: item(1),
          text: 'Now I run the tests.',
        ),
      );
      await emit(
        tester,
        event(
          BridgeEventKind.itemStarted,
          turn: 1,
          item: BridgeItem(
            id: 2,
            kind: BridgeItemKind.commandExecution,
            tool: 'bash',
            arguments: '{"command":"npm test -- login"}',
            failed: false,
          ),
        ),
      );
      await emit(
        tester,
        event(
          BridgeEventKind.widgetChanged,
          display: BridgeDisplay(
            key: 'todo',
            lines: ['✓ limiter', '○ run tests'],
          ),
        ),
      );
      await emit(
        tester,
        event(BridgeEventKind.requestOpened, request: approval('a1')),
      );
      expect(find.text('Now I run the tests.'), findsOneWidget);
      expect(find.text('waiting for approval'), findsOneWidget);
      expect(find.text('todo  ✓ limiter  ○ run tests'), findsOneWidget);
      expect(find.text('● waiting'), findsOneWidget);
    },
  );

  testWidgets('answers a question in place of the composer', (tester) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.select,
            title: 'Pick a window',
            detail: '',
            options: ['Sliding', 'Fixed'],
          ),
        ),
      ),
    );
    expect(find.byKey(const Key('composer')), findsNothing);
    await tester.tap(find.text('Fixed'));
    await tester.pump();
    expect(backend.calls.last, 'answer q1 Fixed');
  });
}
