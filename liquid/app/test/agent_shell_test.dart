import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/icons.dart';
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

  /// Holds ⌘ long enough for the contextual shortcuts to show (ADR-0204).
  Future<void> holdCommand(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 700));
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

  testWidgets('marks the branch in the rebuilt chat and reads earlier items', (
    tester,
  ) async {
    await show(tester);
    backend.stored = historyOf(
      [],
      tree: [
        const BridgeTreeNode(
          id: 'u1',
          kind: BridgeEntryKind.user,
          preview: 'Go',
        ),
        const BridgeTreeNode(
          id: 'a1',
          parent: 'u1',
          kind: BridgeEntryKind.assistant,
          preview: 'Looked',
        ),
        const BridgeTreeNode(
          id: 'a2',
          parent: 'a1',
          kind: BridgeEntryKind.assistant,
          preview: 'Done',
        ),
        const BridgeTreeNode(
          id: 'b1',
          parent: 'a1',
          kind: BridgeEntryKind.user,
          preview: 'Instead',
        ),
      ],
      leaf: 'b1',
    );
    backend.transcripts[''] = BridgeTranscript(
      items: [
        for (var index = 0; index < 30; index++)
          chatItem('u1', BridgeItemKind.assistantMessage, text: 'Line $index'),
        chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
        chatItem('b1', BridgeItemKind.userMessage, text: 'Instead'),
      ],
      more: true,
      start: 'u1',
    );
    await state.loadChat();
    await tester.pumpAndSettle();
    expect(find.text('continues after step 2'), findsOneWidget);
    expect(find.text('A: 1 more'), findsOneWidget);

    await tester.drag(find.byType(ListView).first, const Offset(0, 4000));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('transcript u1'));
  });

  testWidgets('shows notifications in the status bar', (tester) async {
    await show(tester);
    Future<void> notify(BridgeLevel level, String text) => emit(
      tester,
      event(
        BridgeEventKind.notification,
        display: BridgeDisplay(key: '', level: level),
        text: text,
      ),
    );
    final status = find.byKey(const Key('status'));
    String shown() => tester.widget<Text>(status).data!;

    await notify(BridgeLevel.info, 'Tests passed');
    expect(shown(), 'Tests passed');
    await tester.pump(const Duration(seconds: 7));
    expect(shown(), contains('idle'), reason: 'information fades');

    await notify(BridgeLevel.warning, 'lint: 2 warnings');
    await notify(BridgeLevel.error, 'Could not branch: compacting');
    await tester.pump(const Duration(seconds: 7));
    expect(shown(), 'Could not branch: compacting', reason: 'errors stay');
    expect(find.byIcon(LucideIcons.triangleAlert), findsOneWidget);
    expect(find.text('· 2 more'), findsOneWidget);

    await tester.tap(status);
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsOneWidget);
    expect(find.text('lint: 2 warnings'), findsOneWidget);
    await tester.tap(find.text('Clear all'));
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsNothing);
    expect(shown(), contains('idle'));

    await notify(BridgeLevel.error, 'Again');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(shown(), contains('idle'), reason: 'Esc dismisses');
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

  testWidgets('sends, steers and queues from the composer', (tester) async {
    await show(tester);
    final composer = find.byKey(const Key('composer'));
    await tester.enterText(composer, 'Say hi');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(backend.calls, ['send Say hi']);
    expect(find.text('Send'), findsOneWidget);

    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    expect(find.text('Steer'), findsOneWidget, reason: 'Enter steers a run');
    expect(
      find.text('⌥⏎ queue'),
      findsNothing,
      reason: 'shown while ⌘ is held',
    );
    await holdCommand(tester);
    expect(find.text('⌥⏎ queue'), findsOneWidget);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.enterText(composer, 'Shorter');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(backend.calls.last, 'send Shorter');
    await tester.enterText(composer, 'Also docs');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pump();
    expect(backend.calls.last, 'follow-up Also docs');

    await emit(
      tester,
      event(
        BridgeEventKind.queueChanged,
        queue: const BridgeQueue(steering: [], followUp: ['Also docs']),
      ),
    );
    expect(find.text('Queued: Also docs'), findsOneWidget);
    backend.queued = const BridgeQueue(steering: [], followUp: ['Also docs']);
    await tester.tap(find.byKey(const Key('unqueue 0')));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'clear queue');

    await tester.tap(find.text('Stop'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'abort');
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
      expect(
        find.byIcon(LucideIcons.clock),
        findsOneWidget,
        reason: 'a tool waiting for approval shows a clock with its text',
      );
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

  testWidgets('shows the whole call before it is allowed', (tester) async {
    await show(tester);
    final command = [
      for (var part = 0; part < 30; part++) 'rm -rf /tmp/build-$part',
    ].join(' && ');
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.commandExecution,
            tool: 'bash',
            arguments: '{"command":"$command"}',
            readOnly: false,
          ),
        ),
      ),
    );
    expect(find.text(command), findsOneWidget, reason: 'the command itself');
    expect(find.textContaining('more lines'), findsOneWidget);
    await tester.drag(
      find.ancestor(
        of: find.text(command),
        matching: find.byType(SingleChildScrollView),
      ),
      const Offset(0, -2000),
    );
    await tester.pumpAndSettle();
    expect(
      find.textContaining('more lines'),
      findsNothing,
      reason: 'nothing hidden at the end',
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'decide a1 allowOnce');
  });

  testWidgets('filters a long selection and picks from the keyboard', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.select,
            title: 'Pick a deployment target',
            detail: '',
            options: [
              'staging-eu',
              'staging-us',
              'production-eu',
              'production-us',
              for (var number = 1; number <= 10; number++) 'preview-$number',
            ],
          ),
        ),
      ),
    );
    expect(find.text('14 / 14'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('filter')), 'PROD');
    await tester.pump();
    expect(find.text('2 / 14'), findsOneWidget);
    expect(find.text('staging-eu'), findsNothing);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'answer q1 production-us');
  });

  testWidgets('edits a multi-line answer and sends it with ⌘Enter', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.editor,
            title: 'Edit the commit message',
            detail: 'fix: limit /login',
            options: const [],
          ),
        ),
      ),
    );
    await holdCommand(tester);
    expect(find.text('⏎ new line'), findsOneWidget);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    final editor = find.byKey(const Key('answer'));
    await tester.enterText(editor, 'fix: limit /login\n\nAdd a limiter.');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    expect(backend.calls.last, 'answer q1 fix: limit /login\n\nAdd a limiter.');
  });

  testWidgets('marks a tool done when only its outcome changes', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    await emit(
      tester,
      event(
        BridgeEventKind.itemStarted,
        turn: 1,
        item: item(5, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    expect(find.byIcon(LucideIcons.loader), findsOneWidget);
    await emit(
      tester,
      event(
        BridgeEventKind.itemCompleted,
        turn: 1,
        item: item(
          5,
          kind: BridgeItemKind.commandExecution,
          tool: 'bash',
          durationMs: 40,
        ),
      ),
    );
    expect(find.byIcon(LucideIcons.check), findsOneWidget);
    expect(find.text('40 ms'), findsOneWidget);
  });

  testWidgets('shows contextual shortcuts only while Cmd alone is held', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    await emit(
      tester,
      event(BridgeEventKind.requestOpened, request: approval('a1')),
    );
    expect(find.text('⌘⏎'), findsNothing, reason: 'hidden by default');
    expect(
      find.text('⌘1'),
      findsNothing,
      reason: 'app-wide ones live in menus',
    );

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 300));
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await tester.pump(const Duration(milliseconds: 700));
    expect(find.text('⌘⏎'), findsNothing, reason: 'a shortcut was used');
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 700));
    expect(find.text('⌘⏎'), findsOneWidget);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
    expect(find.text('⌘⏎'), findsNothing);
  });

  testWidgets('puts app-wide shortcuts into the macOS menu bar', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    final labels = <String>[];
    void collect(Object? items) {
      for (final item in (items as List?) ?? const []) {
        final map = item as Map;
        if (map['label'] case final String label) labels.add(label);
        collect(map['children']);
      }
    }

    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.menu,
      (call) async {
        if (call.method == 'Menu.setMenus') {
          collect((call.arguments as Map)['0']);
        }
        return null;
      },
    );
    await show(tester);
    await tester.pump();
    debugDefaultTargetPlatformOverride = null;
    expect(labels, containsAll(['Chat', 'Trajectory', 'Diff', 'Focus Input']));
  });
}
