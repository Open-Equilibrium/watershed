import 'dart:async';
import 'dart:ui' show SemanticsFlag;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/trajectory/compare_view.dart';
import 'package:liquid/src/trajectory/overview.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

/// The smallest window Liquid allows is 1050 x 640 (MainFlutterWindow.swift). The test font is
/// much wider than the app's, so a test checks one side of it at a time.
const shortest = Size(1600, 640);
const narrowest = Size(1050, 900);

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
  toolChars: 1050,
  messages: messages,
  messageChars: 2780,
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
    target: 'src/routes/login.ts',
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
    decision: BridgeDecision.allowOnce,
  ),
  record(
    'e5',
    1,
    BridgeRecordKind.tool,
    'bash',
    15200,
    duration: 6800,
    detail: '{"command":"npm test"}',
    target: 'npm test',
    failed: true,
  ),
  record('e6', 1, BridgeRecordKind.message, 'Done.', 22000, duration: 2500),
];

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(
    WidgetTester tester,
    BridgeHistory history, {
    Size size = const Size(1600, 900),
  }) async {
    await tester.binding.setSurfaceSize(size);
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
    // The fold button takes return from the keyboard, as a click.
    Focus.of(tester.element(find.text('Fold turns'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.enter);
    expect(
      find.byKey(const ValueKey('step-e2')),
      findsNothing,
      reason: 'the turns folded',
    );
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
    expect(find.text('Tools · 2'), findsOneWidget);
    expect(find.text('New since step 2: result of read'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.keyB);
    expect(backend.calls, contains('branch e3'));
    final composer = tester.widget<TextField>(
      find.byKey(const Key('composer')),
    );
    expect(composer.focusNode?.hasFocus, isTrue);
  });

  testWidgets('branching at the first response hands its prompt back', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.keyB);
    expect(
      backend.calls,
      contains('branch e1'),
      reason: 'pi redoes a response from before its prompt',
    );
    expect(state.notice.value, isNull);
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
    expect(state.notice.value?.level, BridgeLevel.error);
  });

  testWidgets('a history that cannot be read says so once', (tester) async {
    await show(tester, historyOf(branchA));
    backend.historyError = 'the session file is gone';
    for (var stored = 0; stored < 2; stored++) {
      state.stored.value++;
      await tester.pumpAndSettle();
    }
    expect(state.notices.value.map((notice) => notice.text), [
      'Could not read the session history: the session file is gone',
    ]);
    expect(
      find.text('branch A · 7 steps · 24.5 s'),
      findsOneWidget,
      reason: 'the last history stays',
    );
  });

  testWidgets('a branch the agent refuses says why', (tester) async {
    await show(tester, historyOf(branchA));
    backend.branchError = 'the agent has stopped';
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.keyB);
    expect(
      state.notice.value?.text,
      'The session could not branch at step 1: the agent has stopped',
    );
  });

  testWidgets('what the model saw says when it cannot be read', (tester) async {
    await show(tester, historyOf(branchA));
    backend.contextError = 'the session file is gone';
    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    await tester.tap(find.text('Context'));
    await tester.pumpAndSettle();
    expect(
      find.text('Could not read what the model saw: the session file is gone'),
      findsOneWidget,
    );
  });

  testWidgets('a prompt branches before itself, also when already there', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    Future<void> branchAtPrompt() async {
      await tester.tap(find.byKey(const ValueKey('tree-e1')));
      await tester.pumpAndSettle();
      await press(tester, LogicalKeyboardKey.keyB);
    }

    final composer = find.byKey(const Key('composer'));
    for (var round = 0; round < 2; round++) {
      await branchAtPrompt();
      expect(
        tester.widget<TextField>(composer).focusNode?.hasFocus,
        isTrue,
        reason: 'round $round',
      );
    }
    expect(backend.calls.where((call) => call == 'branch e1'), hasLength(2));
    expect(state.notice.value, isNull);
  });

  testWidgets('a jump during a reload reveals its step once loaded', (
    tester,
  ) async {
    final long = [
      for (var index = 0; index < 40; index++)
        record('s$index', 1, BridgeRecordKind.tool, 'read', index * 100),
    ];
    await show(tester, historyOf(branchA));
    backend.histories['s39'] = historyOf(long);
    backend.stored = historyOf(
      branchA,
      tree: [
        ...historyOf(branchA).tree,
        for (final (index, step) in long.indexed)
          BridgeTreeNode(
            id: step.entry,
            parent: index == 0 ? 'e1' : long[index - 1].entry,
            kind: BridgeEntryKind.toolResult,
            preview: 'read',
          ),
      ],
      leaf: 'e6',
    );
    backend.controller.add(event(BridgeEventKind.itemCompleted, item: item(1)));
    await tester.pumpAndSettle();
    backend.gate = Completer();
    backend.controller.add(event(BridgeEventKind.itemCompleted, item: item(2)));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pump();
    backend.gate!.complete();
    backend.gate = null;
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-s39')), findsOneWidget);
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

  testWidgets('says why compare or branch does nothing', (tester) async {
    await show(tester, historyOf(branchA));
    await press(tester, LogicalKeyboardKey.keyC);
    expect(state.notice.value?.text, 'Comparing needs a second branch.');

    backend.controller.add(event(BridgeEventKind.turnStarted));
    await tester.pumpAndSettle();
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.keyB);
    expect(backend.calls, isNot(contains('branch e1')));
    expect(state.notice.value?.text, 'Branch once the agent is idle.');
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('drops a branch asked for while the session changes', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    backend.switchGate = Completer();
    final switching = state.newSession();
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.keyB);
    backend.switchGate!.complete();
    await switching;
    await tester.pumpAndSettle();
    expect(backend.calls, isNot(contains('branch e1')));
    expect(state.notices.value.map((notice) => notice.text), [
      'The session is changing, so the branch was not made.',
    ], reason: 'no error follows the notice');
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('the inspector\'s tabs and the time switch take keys', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    Focus.of(tester.element(find.text('Context'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.textContaining('what the model saw'), findsOneWidget);
    expect(
      tester.getSemantics(find.text('Context')),
      isSemantics(isSelected: true),
      reason: 'the open tab is told, not only drawn',
    );

    // Return presses the focused switch rather than opening the step.
    Focus.of(tester.element(find.text('Equal width'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.enter);
    expect(
      tester.getSemantics(find.text('Equal width')),
      isSemantics(isSelected: true),
    );
    Focus.of(tester.element(find.text('Real time'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.space);
    expect(
      tester.getSemantics(find.text('Real time')),
      isSemantics(isSelected: true),
      reason: 'space presses it too',
    );
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
    await show(
      tester,
      historyOf(branchB, tree: tree, leaf: 'b2'),
      size: shortest,
    );
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
      find.semantics.byFlag(SemanticsFlag.isChecked),
      findsNWidgets(2),
      reason: 'the compared branches are told, not only drawn',
    );
    // The view's keys show while ⌘ is held (ADR-0204).
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 700));
    expect(
      find.text('⌥↑↓ tree · Space select branch · C leave compare'),
      findsOneWidget,
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
    expect(find.text('Duration'), findsOneWidget);
    expect(find.text('Files changed'), findsOneWidget);
    expect(find.text('Tests'), findsOneWidget);
    // A short window scrolls the comparison under the table's header.
    await tester.scrollUntilVisible(
      find.text('Steps 1–3 shared · prompt, message, read'),
      50,
      scrollable: find
          .descendant(
            of: find.byType(CompareView),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    expect(find.text('“Use a token bucket”'), findsOneWidget);
    expect(find.text('only in A'), findsWidgets);

    // The pill leaves the comparison as its key does, also from the keyboard.
    Focus.of(tester.element(find.text('Comparing'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.text('Comparing'), findsNothing);
    await tester.tap(find.byKey(const ValueKey('tree-e6')));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'history e6');
  });

  testWidgets('shows tool outcomes, the token split and the context tab', (
    tester,
  ) async {
    const saw = BridgeStepContext(
      promptChars: 2310,
      tools: ['bash', 'edit'],
      toolChars: 1050,
      messages: 3,
      messageChars: 2780,
      newSincePrevious: ['result of edit'],
    );
    await show(
      tester,
      historyOf([
        record('u1', 1, BridgeRecordKind.user, 'Add rate limiting', 0),
        record(
          'e1',
          1,
          BridgeRecordKind.tool,
          'edit',
          100,
          duration: 9,
          detail: '{"path":"src/login.ts"}',
          target: 'src/login.ts',
          changedFile: 'src/login.ts',
          outcome: const BridgeToolOutcome(added: 9, removed: 2),
        ),
        record(
          'e2',
          1,
          BridgeRecordKind.tool,
          'bash',
          200,
          duration: 900,
          detail: '{"command":"npm test"}',
          target: 'npm test',
          outcome: const BridgeToolOutcome(
            exitCode: 0,
            testsPassed: 14,
            testsFailed: 0,
          ),
        ),
        record(
          'm1',
          1,
          BridgeRecordKind.message,
          'Done.',
          1200,
          duration: 500,
          usage: tokens(6140, 96),
          context: saw,
        ),
      ]),
      size: narrowest,
    );
    backend.contexts['m1'] = const BridgeContextDetail(
      sections: [BridgeContextPart(name: 'AGENTS.md', chars: 920)],
      tools: [BridgeContextPart(name: 'edit', chars: 500)],
      messages: [
        BridgeContextMessage(
          entry: 'e1',
          label: 'result of edit · src/login.ts',
          chars: 2372,
          inContext: true,
          fresh: true,
        ),
      ],
    );
    expect(find.text('+9 −2', findRichText: true), findsOneWidget);
    expect(find.text('exit 0 · 14 ✓', findRichText: true), findsOneWidget);

    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    expect(backend.calls, contains('context m1'));
    expect(find.text('≈ 2,310'), findsOneWidget);
    expect(find.text('≈ 1,050'), findsOneWidget);
    expect(find.text('≈ 2,780'), findsOneWidget);
    expect(find.text('System prompt · AGENTS.md'), findsOneWidget);

    await tester.tap(find.text('Context'));
    await tester.pumpAndSettle();
    expect(find.text('AGENTS.md'), findsOneWidget);
    expect(find.text('1 · result of edit · src/login.ts'), findsOneWidget);
    expect(find.text('new'), findsOneWidget);
  });

  testWidgets('keeps what the model saw only for the latest steps opened', (
    tester,
  ) async {
    // Each response's context lists the whole branch before it, so keeping every one opened
    // would grow with the square of the session.
    await show(
      tester,
      historyOf([
        record('u1', 1, BridgeRecordKind.user, 'Go', 0),
        for (var step = 1; step <= 8; step++)
          record(
            'm$step',
            1,
            BridgeRecordKind.message,
            'Step $step',
            step * 100,
          ),
      ]),
    );
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.enter);
    for (var step = 0; step < 7; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    for (var step = 0; step < 7; step++) {
      await press(tester, LogicalKeyboardKey.arrowUp);
    }
    expect(
      backend.calls.where((call) => call == 'context m1'),
      hasLength(2),
      reason: 'read again after later steps took its place',
    );
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
