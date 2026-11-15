import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

/// Holds its statistics until `gate` completes, so the rows can be seen waiting for them.
class GatedStatistics extends FakeSession {
  final gate = Completer<void>();

  @override
  Future<BridgeStatistics> statistics() async {
    await gate.future;
    return super.statistics();
  }
}

/// One turn of 14m 32s with 86 tool calls, 6 of them failed, on a model that used 1.85 M tokens.
BridgeStatistics sessionStatistics({int? toolMs = 261000}) => BridgeStatistics(
  models: [
    BridgeModelUsage(
      model: 'anthropic/claude-sonnet-4-5',
      usage: BridgeUsage(
        input: 212000,
        output: 18300,
        cacheRead: 1580000,
        cacheWrite: 41000,
        reasoning: 0,
      ),
      responses: 41,
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
      number: 1,
      prompt: 'Add a rate limit',
      startedMs: 0,
      endedMs: 872000,
      models: const [],
      toolMs: toolMs,
      failed: 6,
    ),
  ],
);

FakeSession sessionWith({
  BridgeContext? window = const BridgeContext(
    tokens: 68200,
    window: 200000,
    compactsAt: 180000,
  ),
  int? toolMs = 261000,
}) => FakeSession()
  ..stats = sessionStatistics(toolMs: toolMs)
  ..contextWindow = window;

void main() {
  late SessionState state;
  // The test's one mouse, which moves over the rows and waits for their tooltips.
  TestGesture? mouse;
  setUp(() => mouse = null);

  Future<void> show(
    WidgetTester tester,
    FakeSession session, {
    SessionCardSettings settings = const SessionCardSettings(),
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    state = SessionState(FakeProject(session: session));
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
    await state.open('thread-1');
    await tester.pumpAndSettle();
  }

  Finder inCard(Finder finder) => find.descendant(
    of: find.byKey(const Key('session-card-frame')),
    matching: finder,
  );

  /// Moves the mouse over `finder` and waits for its tooltip.
  Future<void> hover(WidgetTester tester, Finder finder) async {
    if (mouse == null) {
      final pointer = mouse = await tester.createGesture(
        kind: PointerDeviceKind.mouse,
      );
      addTearDown(pointer.removePointer);
      await pointer.addPointer(location: Offset.zero);
    }
    await mouse!.moveTo(tester.getCenter(finder));
    await tester.pump(const Duration(seconds: 2));
  }

  testWidgets(
    'the rows show the tokens, working time, tool calls and context window',
    (tester) async {
      await show(tester, sessionWith());

      expect(inCard(find.text('Context window')), findsOneWidget);
      expect(inCard(find.text('68.2 k / 200 k')), findsOneWidget);
      expect(inCard(find.text('1.85 M')), findsOneWidget);
      expect(inCard(find.text('14m 32s')), findsOneWidget);
      expect(inCard(find.textContaining('6 failed · 86')), findsOneWidget);
      expect(inCard(find.text('Statistics')), findsOneWidget);
    },
  );

  testWidgets(
    'a dash stands for what is not known, and the cost row says T3 reports none',
    (tester) async {
      await show(
        tester,
        sessionWith(window: const BridgeContext(window: 200000), toolMs: null),
      );

      expect(inCard(find.text('T3 Code reports no cost')), findsOneWidget);
      expect(
        inCard(find.text('—')),
        findsNWidgets(4),
        reason: 'context tokens, tool time, model time and cost',
      );
    },
  );

  testWidgets(
    'the tool calls tooltip names the outcomes and the failing tools',
    (tester) async {
      await show(tester, sessionWith());

      await hover(tester, inCard(find.text('Tool calls')));

      expect(find.textContaining('Tool calls: 86'), findsOneWidget);
      expect(
        find.textContaining('79 succeeded · 6 failed · 1 cancelled'),
        findsOneWidget,
      );
      expect(
        find.textContaining('bash 5 failed, edit 1 failed'),
        findsOneWidget,
      );
    },
  );

  testWidgets('open tool calls count as running once a turn runs', (
    tester,
  ) async {
    final session = FakeSession()
      ..stats = BridgeStatistics(
        models: const [],
        tools: const [
          BridgeToolCalls(
            name: 'bash',
            succeeded: 1,
            failed: 0,
            cancelled: 0,
            open: 2,
          ),
        ],
        turns: const [],
      );
    await show(tester, session);

    await hover(tester, inCard(find.text('Tool calls')));
    expect(find.textContaining('2 cancelled'), findsOneWidget);

    session
      ..turnList = [
        turnOf(1, status: BridgeTurnStatus.running, checkpoint: false),
      ]
      ..emit(event(BridgeEventKind.turnStarted, 1, turn: 1));
    await tester.pumpAndSettle();
    await hover(tester, inCard(find.text('Tool calls')));
    expect(find.textContaining('2 running'), findsOneWidget);
  });

  testWidgets('the Statistics row opens the statistics panel', (tester) async {
    await show(tester, sessionWith());
    expect(state.statisticsOpen.value, isFalse);

    await tester.tap(inCard(find.text('Statistics')));
    await tester.pumpAndSettle();

    expect(state.statisticsOpen.value, isTrue);
  });

  testWidgets('the rows wait for the first read of the statistics', (
    tester,
  ) async {
    final session = GatedStatistics()..contextWindow = null;
    await show(tester, session);

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsOneWidget);

    session.gate.complete();
    await tester.pumpAndSettle();

    expect(inCard(find.text('Tokens')), findsOneWidget);
  });

  testWidgets('hidden statistics leave their rows and the Statistics row out', (
    tester,
  ) async {
    await show(
      tester,
      sessionWith(),
      settings: const SessionCardSettings(hidden: {CardItem.statistics}),
    );

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsNothing);
    expect(inCard(find.text('Context window')), findsOneWidget);
  });

  testWidgets('a statistics row explains itself when the keyboard focuses it', (
    tester,
  ) async {
    await show(tester, sessionWith());

    Focus.of(tester.element(inCard(find.text('Tokens')))).requestFocus();
    await tester.pump();
    await tester.pump();

    expect(find.textContaining('Cache read 1.58 M'), findsOneWidget);
  });
}
