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
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/trajectory/compare_view.dart';
import 'package:liquid/src/trajectory/filter_bar.dart';
import 'package:liquid/src/trajectory/overview.dart';
import 'package:liquid/src/trajectory/tree_view.dart';

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
  sections: const ['preamble', 'project_context'],
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
    List<BridgeModel> models = const [],
  }) async {
    await tester.binding.setSurfaceSize(size);
    backend = FakeBackend()
      ..stored = history
      ..modelList = models;
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
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('start')),
        matching: find.text('Start'),
      ),
      findsOneWidget,
    );
    expect(
      find.text('system prompt · AGENTS.md · 2 tools · ≈2.6k'),
      findsOneWidget,
      reason: 'what the first request held before any message',
    );
    expect(find.text('Turn 1'), findsWidgets);
    expect(
      find.text('Add rate limiting · 24.5 s · +4.3k → ≈7.0k'),
      findsOneWidget,
      reason: 'the turn line sums what the turn added to the context',
    );
    expect(find.text('read · src/routes/login.ts'), findsOneWidget);
    expect(find.text('bash · allowed once'), findsOneWidget);
    expect(find.text('+3.4 s'), findsOneWidget);
    expect(find.text('6.1k'), findsOneWidget, reason: 'tokens in');
    expect(
      find.text('+820'),
      findsOneWidget,
      reason: 'a response adds its output',
    );
    expect(find.text('#2'), findsOneWidget, reason: 'the second request');
    expect(
      find.text('214'),
      findsNothing,
      reason: 'thinking moved to the Usage tab',
    );
    for (final label in [
      'Kind',
      'Step',
      'Time',
      'Adds',
      'Context',
      'In',
      'Out',
    ]) {
      expect(find.text(label), findsWidgets, reason: label);
    }
    expect(find.text('Think'), findsNothing);

    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.arrowLeft);
    expect(find.text('Done.'), findsNothing, reason: 'the turn collapsed');
    expect(find.text('···'), findsOneWidget, reason: 'its context track');
    await press(tester, LogicalKeyboardKey.arrowRight);
    expect(find.text('Done.'), findsOneWidget);
  });

  testWidgets('collapses all turns from the header, aligned with the turns', (
    tester,
  ) async {
    await show(
      tester,
      historyOf([
        ...branchA,
        record('u2', 2, BridgeRecordKind.user, 'Again', 30000),
      ]),
    );
    final all = find.byTooltip('Collapse all turns');
    double center(Finder finder) => tester.getCenter(finder).dx;
    expect(
      center(all),
      closeTo(center(find.byKey(const ValueKey('chevron-1'))), 1),
      reason: 'it sits in the column of the turns\' chevrons',
    );
    await tester.tap(all);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-e2')), findsNothing);
    expect(find.byKey(const ValueKey('step-u2')), findsNothing);
    await tester.tap(find.byTooltip('Expand all turns'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-u2')), findsOneWidget);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.tap(find.byKey(const ValueKey('chevron-1')));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('step-u2')),
      findsNothing,
      reason: '⌥-click on a chevron collapses every turn',
    );
    expect(find.bySemanticsLabel('Expand turn 1'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('chevron-2')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-u2')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('step-e2')),
      findsNothing,
      reason: 'a plain click expands only its turn',
    );
  });

  testWidgets('the setup\'s chevron is named as its line is', (tester) async {
    await show(
      tester,
      historyOf([
        record('s0', 0, BridgeRecordKind.message, 'Ready', 0),
        ...branchA,
      ]),
    );
    expect(find.text('Setup'), findsOneWidget);
    expect(find.bySemanticsLabel('Collapse Setup'), findsOneWidget);
  });

  testWidgets('each request shows the window of its own model', (tester) async {
    final second = record(
      'm2',
      1,
      BridgeRecordKind.message,
      'Done',
      300,
      usage: tokens(1300, 20),
      context: saw(3, ['prompt']),
      model: 'faux-2',
    );
    await show(
      tester,
      historyOf([
        record('u1', 1, BridgeRecordKind.user, 'Fix it', 0),
        record(
          'm1',
          1,
          BridgeRecordKind.message,
          'Reading',
          10,
          usage: tokens(1000, 50),
          context: saw(1, ['prompt']),
        ),
        second,
      ]),
      models: [
        faux(200000),
        const BridgeModel(
          provider: 'faux',
          id: 'faux-2',
          name: 'Faux Two',
          contextWindow: 1000000,
        ),
      ],
    );
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.enter);
    await tester.tap(tab('Context'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('1.0k of 200k'),
      findsOneWidget,
      reason: 'faux-1 read it, not the later faux-2',
    );
  });

  testWidgets('a click on a turn line opens its last request\'s context', (
    tester,
  ) async {
    await show(
      tester,
      historyOf([
        record('u1', 1, BridgeRecordKind.user, 'Fix it', 0, added: 400),
        record(
          'm1',
          1,
          BridgeRecordKind.message,
          'Reading',
          10,
          usage: tokens(1000, 50),
          context: saw(1, ['prompt']),
        ),
        record('t1', 1, BridgeRecordKind.tool, 'bash', 200, added: 800),
        record(
          'm2',
          1,
          BridgeRecordKind.message,
          'Done',
          300,
          usage: tokens(1300, 20),
          context: saw(3, ['result of bash']),
        ),
      ]),
    );
    await tester.tap(find.text('Turn 1'));
    await tester.pumpAndSettle();
    expect(
      tab('Request #2 · what this response read · faux-1'),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('step-m1')),
      findsOneWidget,
      reason: 'the line keeps its turn open; its chevron collapses it',
    );
  });

  testWidgets('draws each step\'s context against the model\'s window', (
    tester,
  ) async {
    await show(tester, historyOf(branchA), models: [faux(200000)]);
    expect(
      find.bySemanticsLabel('Context 4.8k of 200k'),
      findsOneWidget,
      reason: 'the prompt takes the size its request measured',
    );
    expect(
      find.bySemanticsLabel('Context ≈5.1k of 200k'),
      findsOneWidget,
      reason: 'the response adds its output, an estimate',
    );
    expect(find.bySemanticsLabel('Context 6.1k of 200k'), findsOneWidget);
    expect(
      tester.getSemantics(find.text('Window')),
      isSemantics(isSelected: true),
    );
    await tester.tap(find.text('Fit'));
    await tester.pumpAndSettle();
    expect(
      tester.getSemantics(find.text('Fit')),
      isSemantics(isSelected: true),
    );

    await show(tester, historyOf(branchA));
    expect(
      find.bySemanticsLabel('Context 4.8k'),
      findsOneWidget,
      reason: 'without a known window the track fits the branch',
    );
    expect(find.text('Window'), findsNothing);
  });

  testWidgets('the inspector shows the request that read a step', (
    tester,
  ) async {
    BridgeStepContext request(int messages) => BridgeStepContext(
      promptChars: 2000,
      sections: const ['preamble', 'project_context'],
      tools: const ['bash'],
      toolChars: 1600,
      messages: 1,
      messageChars: messages,
      newSincePrevious: const [],
    );
    await show(
      tester,
      historyOf([
        record('u1', 1, BridgeRecordKind.user, 'Fix it', 0, added: 400),
        record(
          'm1',
          1,
          BridgeRecordKind.message,
          'Reading',
          10,
          duration: 100,
          usage: tokens(1000, 50),
          context: request(400),
          added: 200,
        ),
        record(
          't1',
          1,
          BridgeRecordKind.tool,
          'bash',
          200,
          duration: 50,
          detail: '{"command":"npm test"}',
          target: 'npm test',
          added: 8000,
        ),
        record(
          'm2',
          1,
          BridgeRecordKind.message,
          'Done',
          300,
          duration: 100,
          usage: tokens(1100, 20, cached: 2000, reasoning: 7, cost: 3000),
          context: request(8800),
          added: 80,
        ),
      ]),
      models: [faux(200000)],
    );
    backend.contexts['m2'] = const BridgeContextDetail(
      sections: [
        BridgeContextPart(name: 'preamble', chars: 1200),
        BridgeContextPart(name: 'project_context', chars: 800),
      ],
      tools: [BridgeContextPart(name: 'bash', chars: 1600)],
      messages: [
        BridgeContextMessage(
          entry: 'u1',
          label: 'prompt · Fix it',
          chars: 400,
          inContext: true,
          fresh: false,
        ),
        BridgeContextMessage(
          entry: 't1',
          label: 'result of bash · npm test',
          chars: 8000,
          inContext: true,
          fresh: true,
        ),
      ],
    );
    for (var step = 0; step < 3; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    await tester.tap(tab('Context'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('context m2'));
    expect(
      find.text('Request #2 · the request that read this result · faux-1'),
      findsOneWidget,
    );
    expect(
      find.text('3.1k of 200k · 2 % · pi compacts at 183.6k'),
      findsOneWidget,
    );
    for (final part in [
      'System prompt ≈300',
      'AGENTS.md ≈200',
      'Tools ≈400',
      'Carried over ≈200',
      'This step ≈2.0k',
    ]) {
      expect(find.text(part), findsOneWidget, reason: part);
    }
    expect(
      find.text(
        'Largest in request #2: result of bash · npm test ≈2.0k · '
        'Tools ≈400 · System prompt ≈300',
      ),
      findsOneWidget,
    );

    await tester.tap(tab('Usage'));
    await tester.pumpAndSettle();
    for (final value in ['3,100', '2,000', '20', '7', '≈ \$0.003']) {
      expect(tab(value), findsOneWidget, reason: value);
    }
  });

  testWidgets('one toolbar holds the view switch, the summary and Compare', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    double row(Finder finder) => tester.getCenter(finder).dy;
    final summary = find.text('branch A · 7 steps · 24.5 s');
    expect(find.text('Fold turns'), findsNothing);
    for (final control in ['Real time', 'Compare']) {
      expect(
        row(find.text(control)),
        closeTo(row(summary), 1),
        reason: control,
      );
    }
    expect(
      tester.getRect(summary).right,
      lessThan(tester.getRect(find.text('Compare')).left),
    );
    expect(find.text('Overview'), findsNothing, reason: 'no header row');
    // The overview's hint is the tooltip of an icon at the end of its time axis.
    const hint =
        'drag a handle to limit the list · click to jump · scroll to zoom';
    expect(find.text(hint), findsNothing);
    final info = tester.getRect(find.byTooltip(hint));
    final tracks = tester.getRect(find.byKey(const Key('overview-tracks')));
    expect(info.left, greaterThanOrEqualTo(tracks.right));
    expect(info.top, closeTo(tracks.top, 1));
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
    await tester.tap(tab('Context'));
    await tester.pumpAndSettle();
    expect(
      find.text('Could not read what the model saw: the session file is gone'),
      findsOneWidget,
    );
  });

  testWidgets('the session tree is compact and collapses to a rail', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    double width() => tester.getSize(find.byType(SessionTreeView)).width;
    expect(width(), 200);
    await tester.tap(find.byTooltip('Collapse the session tree'));
    await tester.pumpAndSettle();
    expect(width(), 24);

    await show(tester, historyOf(branchA), size: narrowest);
    expect(width(), 24, reason: 'below 1280 px it starts collapsed');
    expect(
      find.bySemanticsLabel(RegExp('^Context ')),
      findsWidgets,
      reason: 'which leaves the context track room at the smallest width',
    );
    await tester.tap(find.byTooltip('Expand the session tree'));
    await tester.pumpAndSettle();
    expect(width(), 200);
  });

  testWidgets('tree nodes show each step\'s kind, not by color alone', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    Finder inTree(Finder finder) =>
        find.descendant(of: find.byType(SessionTreeView), matching: finder);
    expect(
      inTree(find.bySemanticsLabel('tool step, failed')),
      findsOneWidget,
      reason: 'npm test failed',
    );
    expect(inTree(find.bySemanticsLabel('response step')), findsNWidgets(3));
    expect(inTree(find.bySemanticsLabel('approval step')), findsOneWidget);
    expect(
      inTree(find.text("I'll look first.")),
      findsOneWidget,
      reason: 'a response shows its first words instead of "Message"',
    );
    expect(inTree(find.text('Message')), findsNothing);
  });

  testWidgets('a click on a tree node jumps to its step, expanding its turn', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    await tester.tap(find.byTooltip('Collapse all turns'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-e5')), findsNothing);
    await tester.tap(find.byKey(const ValueKey('tree-e5')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-e5')), findsOneWidget);
    await press(tester, LogicalKeyboardKey.enter);
    expect(tab('Tool call'), findsOneWidget, reason: 'the step is selected');
    expect(
      tester.getSemantics(find.byKey(const ValueKey('tree-e5'))),
      isSemantics(isSelected: true),
      reason: 'the tree tells which step is selected, not only draws it',
    );
    expect(
      tester.getSemantics(find.byKey(const ValueKey('step-e5'))),
      isSemantics(isSelected: true),
      reason: 'and so does the list',
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
            failed: false,
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

  /// A session of 60 steps in 20 turns, one second apart: a prompt, a response and a tool.
  final long = [
    for (var i = 0; i < 60; i++)
      record(
        's$i',
        i ~/ 3 + 1,
        [
          BridgeRecordKind.user,
          BridgeRecordKind.message,
          BridgeRecordKind.tool,
        ][i % 3],
        'step $i',
        i * 1000,
        duration: i % 3 == 0 ? null : 500,
      ),
  ];

  testWidgets('the range follows the list and its handles limit it', (
    tester,
  ) async {
    await show(tester, historyOf(long));
    String shown() =>
        tester.getSemantics(find.byKey(const Key('range-brush'))).label;
    expect(shown(), startsWith('Showing steps 1–'));
    await tester.drag(
      find.byKey(const ValueKey('step-s3')),
      const Offset(0, -400),
    );
    await tester.pumpAndSettle();
    expect(
      shown(),
      isNot(startsWith('Showing steps 1–')),
      reason: 'scrolling the list moves the range',
    );

    final tracks = tester.getRect(find.byKey(const Key('overview-tracks')));
    await tester.dragFrom(
      tester.getCenter(find.byKey(const Key('range-end'))),
      Offset(
        tracks.right - tester.getCenter(find.byKey(const Key('range-end'))).dx,
        0,
      ),
    );
    await tester.pumpAndSettle();
    await tester.dragFrom(
      tester.getCenter(find.byKey(const Key('range-start'))),
      Offset(
        tracks.left +
            tracks.width * 0.9 -
            tester.getCenter(find.byKey(const Key('range-start'))).dx,
        0,
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('range')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('step-s10')),
      findsNothing,
      reason: 'the handles limit the list to the steps in the range',
    );
    expect(find.byKey(const ValueKey('step-s58')), findsOneWidget);
    await tester.tap(find.byTooltip('Clear the range'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('range')), findsNothing);
  });

  testWidgets('a tree node outside the range clears it to show its step', (
    tester,
  ) async {
    await show(tester, historyOf(long));
    final tracks = tester.getRect(find.byKey(const Key('overview-tracks')));
    await tester.dragFrom(
      tester.getCenter(find.byKey(const Key('range-start'))),
      Offset(
        tracks.left +
            tracks.width * 0.9 -
            tester.getCenter(find.byKey(const Key('range-start'))).dx,
        0,
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-s3')), findsNothing);
    await tester.tap(find.byKey(const ValueKey('tree-s3')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('range')), findsNothing);
    expect(find.byKey(const ValueKey('step-s3')), findsOneWidget);
  });

  testWidgets('dragging the range slowly scrolls the list', (tester) async {
    await show(tester, historyOf(long));
    String shown() =>
        tester.getSemantics(find.byKey(const Key('range-brush'))).label;
    expect(shown(), startsWith('Showing steps 1–'));
    final drag = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('range-brush'))),
    );
    for (var move = 0; move < 100; move++) {
      await drag.moveBy(const Offset(2, 0));
      await tester.pump();
    }
    await drag.up();
    await tester.pumpAndSettle();
    expect(
      shown(),
      isNot(startsWith('Showing steps 1–')),
      reason: 'small moves add up instead of snapping back to the same step',
    );
  });

  testWidgets('a click on the timeline jumps to the step there', (
    tester,
  ) async {
    await show(tester, historyOf(long));
    expect(find.byKey(const ValueKey('step-s50')), findsNothing);
    final tracks = tester.getRect(find.byKey(const Key('overview-tracks')));
    // Step 50 runs from 50 s to 50.5 s of 59.5 s.
    await tester.tapAt(
      Offset(tracks.left + tracks.width * 50.25 / 59.5, tracks.center.dy),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('step-s50')), findsOneWidget);
    await press(tester, LogicalKeyboardKey.enter);
    expect(tab('step 50'), findsOneWidget, reason: 'the step is selected');
  });

  testWidgets('filters dim what does not match and keys move between matches', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    await tester.tap(find.text('Filter'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(of: find.byType(FilterBar), matching: find.text('Tools')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Filter · 1'), findsOneWidget);
    expect(opacity(tester, 'e2'), lessThan(1), reason: 'a response');
    expect(opacity(tester, 'e3'), 1, reason: 'a tool');
    expect(
      find.byKey(const ValueKey('step-e2')),
      findsOneWidget,
      reason: 'what does not match keeps its place',
    );

    // A click on a step gives the list the keyboard.
    await tester.tap(find.byKey(const ValueKey('step-e1')));
    await tester.pumpAndSettle();
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.enter);
    expect(tab('read'), findsOneWidget, reason: 'the first match');
    await press(tester, LogicalKeyboardKey.escape);
    await press(tester, LogicalKeyboardKey.arrowDown);
    await press(tester, LogicalKeyboardKey.enter);
    expect(tab('bash'), findsOneWidget, reason: 'the next match');
    await press(tester, LogicalKeyboardKey.escape);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('filter-text')), 'npm');
    await tester.pumpAndSettle();
    expect(opacity(tester, 'e3'), lessThan(1), reason: 'read has no npm');
    expect(opacity(tester, 'e5'), 1);
    String? focused() => FocusManager.instance.primaryFocus?.debugLabel;
    // The platform hands return in a text field over as its action.
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(focused(), 'trajectory', reason: 'return gives the list the keys');
    await tester.tap(find.byKey(const Key('filter-text')));
    await press(tester, LogicalKeyboardKey.escape);
    expect(focused(), 'trajectory', reason: 'so does esc');
    await press(tester, LogicalKeyboardKey.escape);
    expect(
      find.byType(FilterBar),
      findsNothing,
      reason: 'esc in the list closes the filter row',
    );
    expect(opacity(tester, 'e3'), lessThan(1), reason: 'the filter stays');
    await tester.tap(find.text('Filter · 2'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Clear'));
    await tester.pumpAndSettle();
    expect(opacity(tester, 'e2'), 1);
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

  testWidgets('an opened step keeps its tabs in view in a short window', (
    tester,
  ) async {
    await show(tester, historyOf(branchA), size: shortest);
    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.text('Raw').hitTestable(), findsOneWidget);
  });

  testWidgets('the inspector\'s tabs and the time switch take keys', (
    tester,
  ) async {
    await show(tester, historyOf(branchA));
    for (var step = 0; step < 4; step++) {
      await press(tester, LogicalKeyboardKey.arrowDown);
    }
    await press(tester, LogicalKeyboardKey.enter);
    Focus.of(tester.element(tab('Context'))).requestFocus();
    await tester.pump();
    await press(tester, LogicalKeyboardKey.enter);
    expect(find.textContaining('what the model saw'), findsOneWidget);
    expect(
      tester.getSemantics(tab('Context')),
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
          failed: false,
          id: step.entry,
          parent: index == 0 ? null : branchA[index - 1].entry,
          kind: BridgeEntryKind.other,
          preview: step.label,
        ),
      const BridgeTreeNode(
        failed: false,
        id: 'b1',
        parent: 'e3',
        kind: BridgeEntryKind.user,
        preview: 'Use a token bucket',
      ),
      const BridgeTreeNode(
        failed: false,
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
    await tester.tap(find.text('Filter'));
    await tester.pumpAndSettle();
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();

    await press(tester, LogicalKeyboardKey.keyC);
    expect(backend.calls.last, 'history e6 b2');
    await press(tester, LogicalKeyboardKey.escape);
    expect(
      find.semantics.byFlag(SemanticsFlag.isChecked),
      findsNWidgets(2),
      reason: 'the compared branches are told, not only drawn',
    );
    // The view's keys show while ⌘ is held.
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
    expect(
      find.byType(FilterBar),
      findsOneWidget,
      reason: 'esc left the filter row the comparison hides alone',
    );
    await tester.tap(find.byKey(const ValueKey('tree-e6')));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'history e6');
  });

  testWidgets('shows tool outcomes, the token split and the context tab', (
    tester,
  ) async {
    const saw = BridgeStepContext(
      promptChars: 2310,
      sections: ['preamble'],
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

    await tester.tap(tab('Context'));
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

BridgeModel faux(int window) => BridgeModel(
  provider: 'faux',
  id: 'faux-1',
  name: 'Faux One',
  contextWindow: window,
);

/// `text` inside the open inspector, apart from the ledger's own columns.
Finder tab(String text) => find.descendant(
  of: find.byKey(const Key('inspector')),
  matching: find.text(text),
);
