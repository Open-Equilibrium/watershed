import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/segment_bar.dart';
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

  Future<void> show(
    WidgetTester tester, {
    BridgeStatistics? stats,
    String? figuresError,
    void Function(FakeBackend backend)? setup,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()
      ..stats = stats ?? statistics()
      ..figuresError = figuresError;
    setup?.call(backend);
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

  testWidgets('a failed statistics read says so instead of still reading', (
    tester,
  ) async {
    await show(tester, figuresError: 'the session file is unreadable');
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    expect(inPanel(find.text('No statistics shown')), findsOneWidget);
    expect(
      inPanel(
        find.text(
          'Could not read the statistics: the session file is unreadable',
        ),
      ),
      findsOneWidget,
    );
    expect(inPanel(find.text('Reading the statistics…')), findsNothing);
  });

  testWidgets(
    'the chosen view and breakdown are told, and Enter takes a focused option',
    (tester) async {
      await show(tester);
      await pressMeta(tester, LogicalKeyboardKey.keyU);

      expect(
        tester.getSemantics(inPanel(find.text('Cost')).first),
        isSemantics(isSelected: true),
        reason: 'the chosen option is told, not only drawn',
      );
      expect(
        tester.getSemantics(inPanel(find.text('Tool')).first),
        isSemantics(isSelected: true),
      );
      Focus.of(tester.element(inPanel(find.text('Tokens')).first))
          .requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();

      expect(inPanel(find.text('Tokens per turn')), findsOneWidget);
      expect(
        tester.getSemantics(inPanel(find.text('Tokens')).first),
        isSemantics(isSelected: true),
      );
    },
  );

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

    await tester.tap(inPanel(find.widgetWithText(InkWell, 'Time')));
    await tester.pumpAndSettle();
    expect(inPanel(find.text('17m 53s')), findsOneWidget);
    expect(inPanel(find.text('Waiting for the model')), findsOneWidget);
    expect(inPanel(find.text('9m 48s')), findsOneWidget);
    expect(inPanel(find.text('4m 21s')), findsOneWidget);
    expect(inPanel(find.text('3m 44s')), findsOneWidget);
  });

  testWidgets(
    'a session without timing entries shows a dash for model and tool time',
    (tester) async {
      await show(tester, stats: statistics(timed: false));
      await pressMeta(tester, LogicalKeyboardKey.keyU);
      await tester.tap(inPanel(find.widgetWithText(InkWell, 'Time')));
      await tester.pumpAndSettle();

      expect(inPanel(find.text('Waiting for the model')), findsOneWidget);
      expect(inPanel(find.text('—')), findsWidgets);
      expect(inPanel(find.text('9m 48s')), findsNothing);
    },
  );

  testWidgets(
    'a turn of another branch closes the panel without paging earlier history',
    (tester) async {
      await show(
        tester,
        setup: (backend) => backend
          ..stored = historyOf(
            tree: const [
              BridgeTreeNode(
                id: 'u1',
                parent: null,
                kind: BridgeEntryKind.user,
              ),
              BridgeTreeNode(
                id: 'a1',
                parent: 'u1',
                kind: BridgeEntryKind.assistant,
              ),
            ],
            leaf: 'a1',
          )
          // A chat that fills the view, so no read-ahead reads earlier pages before the tap.
          ..transcripts[''] = BridgeTranscript(
            items: [
              for (var index = 0; index < 40; index++)
                chatItem(
                  'm$index',
                  BridgeItemKind.assistantMessage,
                  text: 'Message $index',
                ),
            ],
            more: true,
            start: 'm0',
          ),
      );
      await pressMeta(tester, LogicalKeyboardKey.keyU);
      await tester.tap(inPanel(find.widgetWithText(InkWell, 'Turn')));
      await tester.pumpAndSettle();
      backend.calls.clear();

      // The second turn's entry is not on the branch the history read gives.
      await tester.tap(inPanel(find.text('Cover the limiter with tests')));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('statistics-panel')), findsNothing);
      expect(
        backend.calls.where((call) => call.startsWith('transcript')),
        isEmpty,
        reason: 'paging cannot bring in an entry of another branch',
      );
    },
  );

  testWidgets(
    'a turn row closes the panel and scrolls the Chat to its prompt',
    (tester) async {
      await show(tester);
      // The second turn's prompt sits on an earlier page than the newest messages.
      backend.transcripts[''] = BridgeTranscript(
        items: [
          for (var index = 0; index < 40; index++)
            chatItem(
              'm$index',
              BridgeItemKind.assistantMessage,
              text: 'Message $index',
            ),
        ],
        more: true,
        start: 'm0',
      );
      backend.transcripts['m0'] = BridgeTranscript(
        items: [
          chatItem(
            'e1',
            BridgeItemKind.userMessage,
            text: 'Add a rate limit to /login',
          ),
          chatItem(
            'e2',
            BridgeItemKind.userMessage,
            text: 'Cover the limiter with tests',
          ),
        ],
        more: false,
        start: 'e1',
      );
      backend.stored = historyOf(
        tree: const [
          BridgeTreeNode(id: 'e1', parent: null, kind: BridgeEntryKind.user),
          BridgeTreeNode(id: 'e2', parent: 'e1', kind: BridgeEntryKind.user),
        ],
        leaf: 'e2',
      );
      await state.loadChat();
      await tester.pumpAndSettle();
      await pressMeta(tester, LogicalKeyboardKey.keyU);
      await tester.tap(inPanel(find.widgetWithText(InkWell, 'Turn')));
      await tester.pumpAndSettle();

      await tester.tap(inPanel(find.text('Cover the limiter with tests')));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('statistics-panel')), findsNothing);
      expect(backend.calls, contains('transcript m0'));
      final chat = tester.getRect(find.byType(ChatView));
      final prompt = tester.getRect(
        find.descendant(
          of: find.byType(ChatView),
          matching: find.text('Cover the limiter with tests'),
        ),
      );
      expect(
        chat.contains(prompt.center),
        isTrue,
        reason: 'the prompt is in view',
      );
    },
  );

  testWidgets('a turn row takes the keyboard, and Enter shows that turn', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);
    await tester.tap(inPanel(find.text('Time')).first);
    await tester.pumpAndSettle();

    Focus.of(
      tester.element(inPanel(find.text('Cover the limiter with tests')).first),
    ).requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('statistics-panel')), findsNothing);
  });

  testWidgets(
    'a breakdown row takes the hover colour while the pointer is over it',
    (tester) async {
      await show(tester);
      await pressMeta(tester, LogicalKeyboardKey.keyU);
      await tester.tap(inPanel(find.widgetWithText(InkWell, 'Turn')));
      await tester.pumpAndSettle();

      final row = inPanel(find.text('Cover the limiter with tests'));
      Color? rowColour() =>
          (tester
                      .widget<AnimatedContainer>(
                        find
                            .ancestor(
                              of: row,
                              matching: find.byType(AnimatedContainer),
                            )
                            .first,
                      )
                      .decoration
                  as BoxDecoration?)
              ?.color;
      final hover = tester.element(row).ws.hover;
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);

      expect(rowColour(), isNull);
      await mouse.moveTo(tester.getCenter(row));
      await tester.pumpAndSettle();
      expect(rowColour(), hover);

      await mouse.moveTo(
        tester.getCenter(inPanel(find.text('Session statistics'))),
      );
      await tester.pumpAndSettle();
      expect(rowColour(), isNull);
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

  testWidgets('an open call of an ended session is drawn as cancelled', (
    tester,
  ) async {
    final base = statistics();
    await show(
      tester,
      stats: BridgeStatistics(
        models: base.models,
        tools: const [
          BridgeToolCalls(
            name: 'bash',
            succeeded: 16,
            failed: 5,
            cancelled: 0,
            open: 2,
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
        turns: base.turns,
        startedMs: base.startedMs,
        endedMs: base.endedMs,
      ),
    );
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    final bars = tester.widgetList<SegmentBar>(
      inPanel(find.byType(SegmentBar)),
    );
    final bash = bars.singleWhere((bar) => bar.segments.first.value == 16);
    expect(
      [for (final segment in bash.segments) segment.value],
      [16, 5, 2, 0],
      reason: 'succeeded, failed, cancelled, running: nothing runs, so none',
    );
  });

  testWidgets('the Tool breakdown times each tool and keys its outcomes', (
    tester,
  ) async {
    await show(tester);
    await pressMeta(tester, LogicalKeyboardKey.keyU);

    expect(inPanel(find.text('3m 53s')), findsOneWidget, reason: 'bash');
    expect(
      inPanel(find.text('—')),
      findsOneWidget,
      reason: 'edit has no timing entry',
    );
    expect(inPanel(find.text('Succeeded')), findsOneWidget);
    expect(inPanel(find.text('Cancelled')), findsOneWidget);
    expect(
      inPanel(find.text('Failed')),
      findsNWidgets(2),
      reason: 'the column header and the legend',
    );
  });

  testWidgets(
    'Ask the agent puts a question in the composer and sends nothing',
    (tester) async {
      await show(tester);
      await pressMeta(tester, LogicalKeyboardKey.keyU);

      await tester.ensureVisible(
        inPanel(find.text('Ask the agent about this session…')),
      );
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
