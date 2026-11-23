import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

BridgeStatistics statistics() {
  const model = 'anthropic/claude-sonnet-4-5';
  BridgeModelUsage modelUsage(
    int responses,
    int input,
    int output,
    int cacheRead,
    int cacheWrite,
  ) => BridgeModelUsage(
    model: model,
    usage: BridgeUsage(
      input: input,
      output: output,
      cacheRead: cacheRead,
      cacheWrite: cacheWrite,
      reasoning: 0,
    ),
    responses: responses,
  );
  return BridgeStatistics(
    models: [modelUsage(41, 212000, 18300, 1580000, 41000)],
    tools: const [
      BridgeToolCalls(
        name: 'bash',
        succeeded: 16,
        failed: 5,
        cancelled: 0,
        open: 0,
        durationMs: 233000,
      ),
      BridgeToolCalls(
        name: 'edit',
        succeeded: 63,
        failed: 1,
        cancelled: 1,
        open: 0,
      ),
    ],
    turns: [
      BridgeTurnStatistics(
        number: 1,
        prompt: 'Add a rate limit to /login',
        startedMs: 0,
        endedMs: 201000,
        models: [modelUsage(20, 200000, 10000, 900000, 20000)],
        toolMs: 72000,
        failed: 4,
      ),
      BridgeTurnStatistics(
        number: 2,
        prompt: 'Cover the limiter with tests',
        startedMs: 300000,
        endedMs: 1172000,
        models: [modelUsage(21, 12000, 8300, 680000, 21000)],
        toolMs: 189000,
        failed: 2,
      ),
    ],
    startedMs: 0,
    endedMs: 7200000,
  );
}

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

void main() {
  late SessionState state;
  late FakeSession session;

  final scrolls = <int?>[];

  Future<void> show(WidgetTester tester, {FakeSession? using}) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    scrolls.clear();
    session = (using ?? FakeSession())..stats = statistics();
    state = SessionState(FakeProject(session: session));
    state.scrollChatTo.addListener(() => scrolls.add(state.scrollChatTo.value));
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await state.open('thread-1');
    await tester.pumpAndSettle();
  }

  Future<void> openPanel(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyU);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Finder inPanel(Finder finder) => find.descendant(
    of: find.byKey(const Key('statistics-panel')),
    matching: finder,
  );

  void expectInPanel(String text) =>
      expect(inPanel(find.text(text)), findsOneWidget);

  testWidgets('⌘U opens the statistics; Escape closes them', (tester) async {
    await show(tester);
    expect(find.byKey(const Key('statistics-panel')), findsNothing);

    await openPanel(tester);
    expect(find.byKey(const Key('statistics-panel')), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('statistics-panel')), findsNothing);
  });

  testWidgets('a failed statistics read says so', (tester) async {
    await show(tester, using: _UnreadableStatistics());
    await openPanel(tester);

    expectInPanel('No statistics shown');
    expectInPanel('Could not read the statistics: unreadable');
    expect(inPanel(find.text('Reading the statistics…')), findsNothing);
  });

  testWidgets('turns are listed; a tap or Enter opens one', (tester) async {
    await show(tester);
    await openPanel(tester);

    await tap(tester, inPanel(find.widgetWithText(InkWell, 'Turn')));
    expectInPanel('Add a rate limit to /login');
    expectInPanel('2 failed');

    await tap(tester, inPanel(find.text('Cover the limiter with tests')));
    expect(find.byKey(const Key('statistics-panel')), findsNothing);
    expect(scrolls, contains(2));

    await openPanel(tester);
    await tap(tester, inPanel(find.widgetWithText(InkWell, 'Turn')));
    scrolls.clear();
    Focus.of(tester.element(inPanel(find.text('Cover the limiter with tests'))))
        .requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('statistics-panel')), findsNothing);
    expect(scrolls, contains(2));
  });

  testWidgets('Ask the agent fills the input and sends nothing', (
    tester,
  ) async {
    await show(tester);
    await openPanel(tester);

    await tester.ensureVisible(
      inPanel(find.text('Ask the agent about this session…')),
    );
    await tap(tester, inPanel(find.text('Ask the agent about this session…')));

    expect(find.byKey(const Key('statistics-panel')), findsNothing);
    expect(
      tester
          .widgetList<EditableText>(find.byType(EditableText))
          .map((field) => field.controller.text),
      anyElement(startsWith('About this session')),
    );
    expect(session.calls, isEmpty);
  });
}

class _UnreadableStatistics extends FakeSession {
  @override
  Future<BridgeStatistics> statistics() async => throw 'unreadable';
}
