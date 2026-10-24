import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/trajectory/overview.dart';

import 'fake_backend.dart';

const info = SessionInfo(
  title: 'Rate limit for /login',
  project: 'acme-api',
  harness: 'pi',
  device: 'MacBook',
);

BridgeUsage tokens(
  int input,
  int output, {
  int cached = 0,
  int reasoning = 0,
  int cost = 0,
}) => BridgeUsage(
  input: input,
  output: output,
  cacheRead: cached,
  cacheWrite: 0,
  reasoning: reasoning,
  costMicroUsd: cost,
);

BridgeStepContext saw(int messages, List<String> added) => BridgeStepContext(
  promptChars: 2310,
  tools: const ['bash', 'read'],
  messages: messages,
  newSincePrevious: added,
);

/// Branch A of the approved mockup, shortened.
final branchA = [
  record('e1', 1, BridgeRecordKind.user, 'Add rate limiting', 0),
  record(
    'e2',
    1,
    BridgeRecordKind.message,
    "I'll look first.",
    100,
    duration: 2000,
    firstToken: 700,
    usage: tokens(4812, 312),
    context: saw(1, ['prompt']),
  ),
  record(
    'e3',
    1,
    BridgeRecordKind.tool,
    'read',
    2200,
    duration: 18,
    detail: '{"path":"src/routes/login.ts"}',
  ),
  record(
    'e4',
    1,
    BridgeRecordKind.message,
    'There is no limiter yet.',
    2300,
    duration: 3400,
    firstToken: 900,
    usage: tokens(140, 820, cached: 6000, reasoning: 214, cost: 20000),
    context: saw(3, ['result of read']),
  ),
  record(
    'a1',
    1,
    BridgeRecordKind.approval,
    'bash',
    5800,
    duration: 9400,
    detail: 'allow-once',
  ),
  record(
    'e5',
    1,
    BridgeRecordKind.tool,
    'bash',
    15200,
    duration: 6800,
    detail: '{"command":"npm test"}',
    failed: true,
  ),
  record('e6', 1, BridgeRecordKind.message, 'Done.', 22000, duration: 2500),
];

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(WidgetTester tester, BridgeHistory history) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()..stored = history;
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit2);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  double opacity(WidgetTester tester, String entry) => tester
      .widget<Opacity>(
        find
            .ancestor(
              of: find.byKey(ValueKey('step-$entry')),
              matching: find.byType(Opacity),
            )
            .first,
      )
      .opacity;

  testWidgets('shows each step with tokens and timing, grouped by turn', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    expect(find.text('branch A · 7 steps · 24.5 s'), findsOneWidget);
    expect(find.text('Turn 1'), findsWidgets);
    expect(find.text('24.5 s · read 1 · bash 1'), findsOneWidget);
    expect(find.text('read · src/routes/login.ts'), findsOneWidget);
    expect(find.text('bash · allowed once'), findsOneWidget);
    expect(find.text('+3.4 s'), findsOneWidget);
    expect(find.text('6,140'), findsOneWidget);
    expect(find.text('214'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.arrowLeft);
    expect(find.text('Done.'), findsNothing, reason: 'the turn folded');
    await press(tester, LogicalKeyboardKey.arrowRight);
    expect(find.text('Done.'), findsOneWidget);
  });

  testWidgets('opens what the model saw and branches from a step', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.text('What the model saw · 6,140 tokens'), findsOneWidget);
    expect(find.text('900 ms'), findsOneWidget, reason: 'first token');
    expect(
      find.text('6,140 in · 820 out · 214 think · 6,000 cached'),
      findsOneWidget,
    );
    expect(find.text('≈ \$0.02'), findsOneWidget);
    expect(find.text('Tools · bash, read'), findsOneWidget);
    expect(find.text('New since step 2: result of read'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyB);
    expect(backend.calls, contains('branch e3'));
    final composer = tester.widget<TextField>(
      find.byKey(const Key('composer')),
    );
    expect(composer.focusNode?.hasFocus, isTrue);
  });

  testWidgets('a branch that did not move the session keeps the keyboard', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    backend.branchMoves = false;
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.keyB);
    expect(
      backend.calls,
      contains('branch e1'),
      reason: 'a prompt branches at itself',
    );
    final composer = tester.widget<TextField>(
      find.byKey(const Key('composer')),
    );
    expect(composer.focusNode?.hasFocus, isFalse);
    expect(state.notification.value?.$1, BridgeLevel.error);
  });

  testWidgets('the session tree jumps between steps with ⌥↑↓', (tester) async {
    await show(tester, historyOf(branchA));
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pumpAndSettle();
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.text('What the model saw · 4,812 tokens'), findsOneWidget);
  });

  testWidgets('a trackpad swipe zooms the overview instead of focusing', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    final tracks = tester.getCenter(find.byKey(const Key('overview-tracks')));
    final trackpad = await tester.createGesture(
      kind: PointerDeviceKind.trackpad,
    );
    await trackpad.panZoomStart(tracks);
    await trackpad.panZoomUpdate(tracks, pan: const Offset(0, -120));
    await trackpad.panZoomEnd();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('range')), findsNothing);
    expect(tester.widget<Overview>(find.byType(Overview)).window, isNotNull);
  });

  testWidgets('focusing a range dims the steps outside it', (tester) async {
    await show(tester, historyOf(branchA));
    final tracks = tester.getRect(find.byKey(const Key('overview-tracks')));
    await tester.dragFrom(
      tracks.centerLeft + const Offset(1, 0),
      Offset(tracks.width * 0.1, 0),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('range')), findsOneWidget);
    expect(opacity(tester, 'e2'), 1);
    expect(opacity(tester, 'a1'), lessThan(1));

    await press(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('range')), findsNothing);
    expect(opacity(tester, 'a1'), 1);
  });

  testWidgets('compares two branches side by side', (tester) async {
    final branchB = [
      ...branchA.take(3),
      record('b1', 2, BridgeRecordKind.user, 'Use a token bucket', 60000),
      record('b2', 2, BridgeRecordKind.tool, 'bash', 61000, duration: 900),
    ];
    final tree = [
      for (final (index, step) in branchA.indexed)
        BridgeTreeNode(
          id: step.entry,
          parent: index == 0 ? null : branchA[index - 1].entry,
          kind: BridgeEntryKind.other,
          preview: step.label,
        ),
      const BridgeTreeNode(
        id: 'b1',
        parent: 'e3',
        kind: BridgeEntryKind.user,
        preview: 'Use a token bucket',
      ),
      const BridgeTreeNode(
        id: 'b2',
        parent: 'b1',
        kind: BridgeEntryKind.toolResult,
        preview: 'bash',
      ),
    ];
    await show(tester, historyOf(branchB, tree: tree, leaf: 'b2'));
    backend.histories['e6 b2'] = historyOf(
      branchA,
      tree: tree,
      leaf: 'b2',
      others: [branchB],
    );
    expect(find.text('branch B · 5 steps · 1m 01s'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyC);
    expect(backend.calls.last, 'history e6 b2');
    expect(
      find.text('⌥↑↓ tree · Space select branch · C leave compare'),
      findsOneWidget,
    );
    expect(
      find.text('Steps 1–3 shared · prompt, message, read'),
      findsOneWidget,
    );
    expect(find.text('“Use a token bucket”'), findsOneWidget);
    expect(find.text('only in A'), findsWidgets);
    expect(find.text('Duration'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyC);
    await tester.tap(find.byKey(const ValueKey('tree-e6')));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'history e6');
  });

  testWidgets('follows the run live as steps are stored', (tester) async {
    await show(tester, historyOf(branchA.take(1).toList()));
    expect(find.text("I'll look first."), findsNothing);
    backend.stored = historyOf(branchA.take(2).toList());
    backend.controller.add(event(BridgeEventKind.itemCompleted, item: item(1)));
    await tester.pumpAndSettle();
    expect(find.text("I'll look first."), findsOneWidget);
  });
}
