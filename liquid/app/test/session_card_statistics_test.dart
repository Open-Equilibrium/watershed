import 'dart:async';

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

class GatedStatistics extends FakeSession {
  final gate = Completer<void>();

  @override
  Future<BridgeStatistics> statistics() async {
    await gate.future;
    return super.statistics();
  }
}

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

const contextShown = SessionCardSettings(shown: {CardItem.context});

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

  testWidgets('rows show tokens, time, tool calls and context', (tester) async {
    await show(tester, sessionWith(), settings: contextShown);

    expect(inCard(find.text('Context')), findsOneWidget);
    expect(inCard(find.text('68.2 k / 200 k')), findsOneWidget);
    expect(inCard(find.text('1.85 M')), findsOneWidget);
    expect(inCard(find.text('14m 32s')), findsOneWidget);
    expect(inCard(find.textContaining('6 failed · 86')), findsOneWidget);
    expect(inCard(find.text('Statistics')), findsOneWidget);
  });

  testWidgets('the Statistics row opens the statistics panel', (tester) async {
    await show(tester, sessionWith());
    expect(state.statisticsOpen.value, isFalse);

    await tester.tap(inCard(find.text('Statistics')));
    await tester.pumpAndSettle();

    expect(state.statisticsOpen.value, isTrue);
  });

  testWidgets('rows wait for the first statistics read', (tester) async {
    final session = GatedStatistics()..contextWindow = null;
    await show(tester, session);

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsOneWidget);

    session.gate.complete();
    await tester.pumpAndSettle();

    expect(inCard(find.text('Tokens')), findsOneWidget);
  });

  testWidgets('hidden statistics leave their rows out', (tester) async {
    await show(
      tester,
      sessionWith(),
      settings: const SessionCardSettings(
        hidden: {CardItem.statistics},
        shown: {CardItem.context},
      ),
    );

    expect(inCard(find.text('Tokens')), findsNothing);
    expect(inCard(find.text('Statistics')), findsNothing);
    expect(inCard(find.text('Context')), findsOneWidget);
  });

  testWidgets('a focused statistics row explains itself', (tester) async {
    await show(tester, sessionWith());

    Focus.of(tester.element(inCard(find.text('Tokens')))).requestFocus();
    await tester.pump();
    await tester.pump();

    expect(find.textContaining('Cache read 1.58 M'), findsOneWidget);
  });
}
