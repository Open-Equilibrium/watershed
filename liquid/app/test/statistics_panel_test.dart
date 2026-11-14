import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/agent/statistics_chart.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

BridgeUsage usageOf({
  int input = 0,
  int output = 0,
  int cacheRead = 0,
  int cacheWrite = 0,
  int cost = 0,
}) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: cacheRead,
  cacheWrite: cacheWrite,
  reasoning: 0,
  costMicroUsd: cost,
);

/// Two turns of one model: 17 min 53 s in all, with 86 tool calls, six of them failed.
BridgeStatistics statistics({bool timed = true}) {
  final first = usageOf(
    input: 200000,
    output: 10000,
    cacheRead: 900000,
    cacheWrite: 20000,
    cost: 700000,
  );
  final second = usageOf(
    input: 12000,
    output: 8300,
    cacheRead: 680000,
    cacheWrite: 21000,
    cost: 570000,
  );
  const model = 'anthropic/claude-sonnet-4-5';
  return statisticsOf(
    models: [
      BridgeModelUsage(
        model: model,
        usage: usageOf(
          input: 212000,
          output: 18300,
          cacheRead: 1580000,
          cacheWrite: 41000,
          cost: 1270000,
        ),
        costByKind: const BridgeCostByKind(
          input: 520000,
          output: 210000,
          cacheRead: 420000,
          cacheWrite: 120000,
        ),
        responses: 41,
        unpriced: 0,
      ),
    ],
    tools: const [
      BridgeToolCalls(
        name: 'bash',
        succeeded: 16,
        failed: 5,
        cancelled: 0,
        open: 0,
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
        entry: 'e1',
        prompt: 'Add a rate limit to /login',
        startedMs: 0,
        endedMs: 201000,
        models: [
          BridgeModelUsage(
            model: model,
            usage: first,
            costByKind: const BridgeCostByKind(
              input: 330000,
              output: 120000,
              cacheRead: 200000,
              cacheWrite: 50000,
            ),
            responses: 20,
            unpriced: 0,
          ),
        ],
        modelMs: timed ? 125000 : null,
        toolMs: timed ? 72000 : null,
        toolCalls: 40,
        failed: 4,
      ),
      BridgeTurnStatistics(
        entry: 'e2',
        prompt: 'Cover the limiter with tests',
        startedMs: 300000,
        endedMs: 1172000,
        models: [
          BridgeModelUsage(
            model: model,
            usage: second,
            costByKind: const BridgeCostByKind(
              input: 190000,
              output: 90000,
              cacheRead: 220000,
              cacheWrite: 70000,
            ),
            responses: 21,
            unpriced: 0,
          ),
        ],
        modelMs: timed ? 463000 : null,
        toolMs: timed ? 189000 : null,
        toolCalls: 46,
        failed: 2,
      ),
    ],
    startedMs: 0,
    endedMs: 7200000,
  );
}

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(WidgetTester tester, {BridgeStatistics? stats}) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()..stats = stats ?? statistics();
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await state.loadChat();
    await tester.pumpAndSettle();
  }

  Future<void> pressMeta(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(key);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Finder inPanel(Finder finder) => find.descendant(
    of: find.byKey(const Key('statistics-panel')),
    matching: finder,
  );

  testWidgets('⌘U opens the statistics, and Escape closes them', (
    tester,
  ) async {
    await show(tester);
    expect(find.byKey(const Key('statistics-panel')), findsNothing);

    await pressMeta(tester, LogicalKeyboardKey.keyU);
    expect(find.byKey(const Key('statistics-panel')), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('statistics-panel')), findsNothing);
  });

  testWidgets('the Cost view leads with the cost and splits it by model', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    expect(
      inPanel(find.text('≈ \$1.27')),
      findsWidgets,
      reason: 'the hero and its model row',
    );
    expect(inPanel(find.text('claude-sonnet-4-5')), findsOneWidget);
    expect(
      inPanel(
        find.textContaining('100% of cost · 1.85 M tokens · 41 responses'),
      ),
      findsOneWidget,
    );
    expect(
      inPanel(find.text('Cache read')),
      findsNWidgets(2),
      reason: 'the totals and the cost by type legend',
    );
  });

  testWidgets('the Cost view splits the cost by type, as the tokens are', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    expect(inPanel(find.text('Cost by type')), findsOneWidget);
    expect(inPanel(find.text('\$0.52')), findsOneWidget, reason: 'input');
    expect(inPanel(find.text('\$0.42')), findsOneWidget, reason: 'cache read');
    expect(inPanel(find.text('\$0.12')), findsOneWidget, reason: 'cache write');
    expect(inPanel(find.text('\$0.21')), findsOneWidget, reason: 'output');
  });

  testWidgets('the Tokens and Time views lead with their own totals', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    await tester.tap(inPanel(find.text('Tokens')));
    await tester.pumpAndSettle();
    expect(inPanel(find.text('1.85 M')), findsWidgets);
    expect(inPanel(find.text('Tokens by type')), findsOneWidget);

    await tester.tap(inPanel(find.text('Time')));
    await tester.pumpAndSettle();
    expect(inPanel(find.text('17 min 53 s')), findsOneWidget);
    expect(inPanel(find.text('Waiting for the model')), findsOneWidget);
    expect(inPanel(find.text('9 min 48 s')), findsOneWidget);
    expect(inPanel(find.text('4 min 21 s')), findsOneWidget);
    expect(inPanel(find.text('3 min 44 s')), findsOneWidget);
  });

  testWidgets(
    'a session without timing entries shows a dash for model and tool time',
    (tester) async {
      await show(tester, stats: statistics(timed: false));
      await pressMeta(tester, LogicalKeyboardKey.keyU);
      await tester.tap(inPanel(find.text('Time')));
      await tester.pumpAndSettle();

      expect(inPanel(find.text('Waiting for the model')), findsOneWidget);
      expect(inPanel(find.text('—')), findsWidgets);
      expect(inPanel(find.text('9 min 48 s')), findsNothing);
    },
  );

  testWidgets('hovering a turn column shows its figures', (tester) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    final chart = inPanel(find.byType(TurnChart));
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    final origin = tester.getTopLeft(chart);
    await mouse.moveTo(origin + const Offset(90, 60));
    await tester.pumpAndSettle();

    expect(find.text('Turn 1'), findsOneWidget);
    expect(find.textContaining('Add a rate limit to /login'), findsWidgets);
    expect(find.textContaining('≈ \$0.70'), findsWidgets);
  });

  testWidgets('the breakdown lists tools, then turns that select nothing yet', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    expect(inPanel(find.text('bash')), findsOneWidget);
    expect(inPanel(find.text('edit')), findsOneWidget);
    expect(inPanel(find.text('21')), findsOneWidget);
    expect(inPanel(find.text('65')), findsOneWidget);

    await tester.tap(inPanel(find.text('Turn')));
    await tester.pumpAndSettle();
    expect(inPanel(find.text('Add a rate limit to /login')), findsOneWidget);
    expect(inPanel(find.text('Cover the limiter with tests')), findsOneWidget);
    expect(inPanel(find.text('2 failed')), findsOneWidget);
  });

  testWidgets(
    'Ask the agent puts a question in the composer and sends nothing',
    (tester) async {
      await show(tester);
      await pressMeta(tester, LogicalKeyboardKey.keyU);

      await tester.tap(inPanel(find.text('Ask the agent about this session…')));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('statistics-panel')), findsNothing);
      expect(
        tester
            .widgetList<EditableText>(find.byType(EditableText))
            .map((field) => field.controller.text),
        anyElement(startsWith('About this session')),
        reason: 'the question waits in the input',
      );
      expect(backend.calls.where((call) => call.startsWith('send')), isEmpty);
    },
  );
}
