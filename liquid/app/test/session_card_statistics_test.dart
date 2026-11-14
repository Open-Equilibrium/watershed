import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

/// One model that used 1.79 M tokens for $1.27, and a turn of 14 min 32 s with 86 tool calls.
BridgeStatistics sessionStatistics({
  int? modelMs = 588000,
  int? toolMs = 261000,
}) {
  final usage = BridgeUsage(
    input: 212000,
    output: 18300,
    cacheRead: 1580000,
    cacheWrite: 41000,
    reasoning: 0,
    costMicroUsd: 1270000,
  );
  return statisticsOf(
    models: [
      BridgeModelUsage(
        model: 'anthropic/claude-sonnet-4-5',
        costByKind: const BridgeCostByKind(
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
        ),
        usage: usage,
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
        prompt: 'Add a rate limit',
        startedMs: 0,
        endedMs: 872000,
        models: [
          BridgeModelUsage(
            model: 'anthropic/claude-sonnet-4-5',
            costByKind: const BridgeCostByKind(
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
            ),
            usage: usage,
            responses: 41,
            unpriced: 0,
          ),
        ],
        modelMs: modelMs,
        toolMs: toolMs,
        toolCalls: 86,
        failed: 6,
      ),
    ],
  );
}

void main() {
  late FakeBackend backend;
  late SessionState state;
  TestGesture? mouse;
  var addedPointer = false;

  setUp(() {
    mouse = null;
    addedPointer = false;
  });

  tearDown(() async => mouse?.removePointer());

  Future<void> show(
    WidgetTester tester, {
    SessionCardSettings settings = const SessionCardSettings(),
    void Function(FakeBackend backend)? setup,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()..stats = sessionStatistics();
    backend.contextWindow = const BridgeContext(
      tokens: 68200,
      window: 200000,
      compactsAt: 180000,
    );
    setup?.call(backend);
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: info,
          cardSettings: SessionCardSettingsController(settings),
        ),
      ),
    );
    await state.loadChat();
    await tester.pumpAndSettle();
  }

  Finder inCard(Finder finder) => find.descendant(
    of: find.byKey(const Key('session-details-card')),
    matching: finder,
  );

  /// Moves the test's one mouse over `finder` and waits for its tooltip.
  Future<void> hover(WidgetTester tester, Finder finder) async {
    final pointer = mouse ??= await tester.createGesture(
      kind: PointerDeviceKind.mouse,
    );
    if (mouse == pointer && !addedPointer) {
      addedPointer = true;
      await pointer.addPointer(location: Offset.zero);
    }
    await pointer.moveTo(tester.getCenter(finder));
    await tester.pump(const Duration(seconds: 2));
  }

  testWidgets('the figures show the statistics and the context window', (
    tester,
  ) async {
    await show(tester);

    expect(inCard(find.text('Context')), findsOneWidget);
    expect(inCard(find.text('68.2 k / 200 k')), findsOneWidget);
    expect(inCard(find.text('1.85 M')), findsOneWidget);
    expect(inCard(find.text('≈ \$1.27')), findsOneWidget);
    expect(inCard(find.text('14 min 32 s')), findsOneWidget);
    expect(inCard(find.textContaining('6 failed · 86')), findsOneWidget);
    expect(inCard(find.text('Statistics')), findsOneWidget);
  });

  testWidgets('a dash stands for context tokens that are not known', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) =>
          backend.contextWindow = const BridgeContext(window: 200000),
    );

    expect(inCard(find.text('—')), findsWidgets);
    expect(inCard(find.text('68.2 k / 200 k')), findsNothing);
  });

  testWidgets('the cost mark explains the responses without a recorded cost', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) => backend.stats = statisticsOf(
        models: [
          BridgeModelUsage(
            model: 'anthropic/claude-sonnet-4-5',
            costByKind: const BridgeCostByKind(
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
            ),
            usage: BridgeUsage(
              input: 1000,
              output: 100,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              costMicroUsd: 20000,
            ),
            responses: 4,
            unpriced: 2,
          ),
        ],
      ),
    );

    await hover(tester, inCard(find.text('Cost')));

    expect(
      find.textContaining(
        '2 responses without a recorded cost are counted in tokens, not in cost',
      ),
      findsOneWidget,
    );
  });

  testWidgets('an older session shows a dash for model and tool time', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) =>
          backend.stats = sessionStatistics(modelMs: null, toolMs: null),
    );

    expect(inCard(find.text('14 min 32 s')), findsOneWidget);
    expect(inCard(find.text('Model and tools')), findsOneWidget);
    expect(inCard(find.text('Working time')), findsOneWidget);
    expect(find.textContaining('Model 9 min 48 s'), findsNothing);
  });

  testWidgets('the tool calls tooltip names the failing tools', (tester) async {
    await show(tester);

    await hover(tester, inCard(find.text('Tool calls')));

    expect(find.textContaining('Tool calls: 86'), findsOneWidget);
    expect(
      find.textContaining('79 succeeded · 6 failed · 1 cancelled'),
      findsOneWidget,
    );
    expect(find.textContaining('bash 5 failed, edit 1 failed'), findsOneWidget);
  });

  testWidgets('open tool calls count as running while a turn runs', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) => backend.stats = statisticsOf(
        tools: const [
          BridgeToolCalls(
            name: 'bash',
            succeeded: 1,
            failed: 0,
            cancelled: 0,
            open: 2,
          ),
        ],
      ),
    );

    await hover(tester, inCard(find.text('Tool calls')));
    expect(find.textContaining('2 cancelled'), findsOneWidget);
    expect(find.textContaining('running'), findsNothing);

    backend.controller.add(event(BridgeEventKind.turnStarted));
    await tester.pumpAndSettle();
    await hover(tester, inCard(find.text('Tool calls')));
    expect(find.textContaining('2 running'), findsOneWidget);
  });

  testWidgets('the Statistics row opens the statistics panel', (tester) async {
    await show(tester);
    expect(state.statisticsOpen.value, isFalse);

    await tester.tap(inCard(find.text('Statistics')));
    await tester.pumpAndSettle();

    expect(state.statisticsOpen.value, isTrue);
  });

  testWidgets('the figures wait for the first read of the statistics', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) => backend.figuresGate = Completer<void>(),
    );
    await tester.pump();

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsOneWidget);

    backend.figuresGate!.complete();
    await tester.pumpAndSettle();

    expect(inCard(find.text('Tokens')), findsOneWidget);
  });

  testWidgets('hidden rows leave their figures out, the Statistics row too', (
    tester,
  ) async {
    await show(
      tester,
      settings: const SessionCardSettings(hidden: {CardItem.figures}),
    );

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsNothing);
    expect(inCard(find.text('Context')), findsOneWidget);
  });

  testWidgets('a hidden context window leaves its row out', (tester) async {
    await show(
      tester,
      settings: const SessionCardSettings(hidden: {CardItem.context}),
    );

    expect(inCard(find.text('Context')), findsNothing);
    expect(inCard(find.text('Tokens')), findsOneWidget);
  });
}
