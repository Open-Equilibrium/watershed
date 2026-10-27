import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/action_area.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(
    WidgetTester tester, {
    DiffSettingsController? diffSettings,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend();
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, diffSettings: diffSettings),
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

  BridgeRequest approval(String id, {int? item}) => BridgeRequest(
    id: id,
    approval: BridgeApproval(
      kind: BridgeRequestKind.commandExecution,
      tool: 'bash',
      text: 'npm test -- login',
      item: item,
    ),
  );

  testWidgets(
    'names the session in the header and sidebar as the start page does',
    (tester) async {
      await show(tester);
      await tester.pumpAndSettle();
      expect(find.text('New session'), findsNWidgets(2));

      await tester.enterText(
        find.byKey(const Key('composer')),
        'Add a limiter\nto /login',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(
        find.text('Add a limiter'),
        findsNWidgets(2),
        reason: 'the first line of the first prompt, at once',
      );

      backend.sessionTitle = 'Rate limit for /login';
      await emit(tester, event(BridgeEventKind.turnStarted));
      await emit(tester, event(BridgeEventKind.turnCompleted));
      expect(
        find.text('Rate limit for /login'),
        findsNWidgets(2),
        reason: 'the name pi stored',
      );

      backend.sessionTitle = null;
      await state.newSession();
      await tester.pumpAndSettle();
      expect(find.text('New session'), findsNWidgets(2));
    },
  );

  testWidgets('the Diff tab counts the files the latest run changed', (
    tester,
  ) async {
    await show(tester);
    await tester.pumpAndSettle();
    final count = find.byKey(const Key('changed-files'));
    expect(count, findsNothing, reason: 'no run yet');

    backend.changed = [
      for (final path in ['a.txt', 'b.txt'])
        BridgeFileChange(
          path: path,
          shown: path,
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 1,
        ),
    ];
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(tester, event(BridgeEventKind.turnCompleted));
    expect(
      find.descendant(of: count, matching: find.text('2')),
      findsOneWidget,
    );

    backend.changed = const [];
    await state.newSession();
    await tester.pumpAndSettle();
    expect(count, findsNothing, reason: 'a new session has no run yet');
  });

  testWidgets(
    'counts changes as the whitespace choice says, kept or switched',
    (tester) async {
      await show(
        tester,
        diffSettings: DiffSettingsController(
          const DiffSettings(hideWhitespace: true),
        ),
      );
      await tester.pumpAndSettle();
      BridgeFileChange change(String path) => BridgeFileChange(
        path: path,
        shown: path,
        status: BridgeChangeStatus.modified,
        added: 1,
        removed: 1,
      );
      backend
        ..changed = [change('code.ts'), change('spaced.ts')]
        ..changedWithoutWhitespace = [change('code.ts')];
      await emit(tester, event(BridgeEventKind.turnStarted));
      await emit(tester, event(BridgeEventKind.turnCompleted));
      expect(
        find.descendant(
          of: find.byKey(const Key('changed-files')),
          matching: find.text('1'),
        ),
        findsOneWidget,
        reason: 'the kept choice counts before the Diff tab opens',
      );

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byKey(const Key('changed-files')),
          matching: find.text('2'),
        ),
        findsOneWidget,
        reason: 'W shows whitespace changes again',
      );
    },
  );

  testWidgets('says why the agent stopped', (tester) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.exited,
        success: false,
        text: 'Error: No models available',
      ),
    );
    expect(find.textContaining('Error: No models available'), findsOneWidget);
  });

  testWidgets('keeps a failed agent\'s whole output readable in the list', (
    tester,
  ) async {
    await show(tester);
    const output =
        '(node:4242) ExperimentalWarning: Type stripping is an experimental feature\n'
        'Error: Failed to load extension /w/watershed.ts\n'
        "  Cannot find module '@earendil-works/pi-coding-agent'";
    await emit(
      tester,
      event(BridgeEventKind.exited, success: false, text: output),
    );
    await tester.tap(find.byKey(const Key('status')));
    await tester.pumpAndSettle();
    final entry = find.descendant(
      of: find.byType(ListView),
      matching: find.textContaining('ExperimentalWarning'),
    );
    expect(find.byType(SelectableText), findsNothing);

    await tester.tap(entry.first);
    await tester.pumpAndSettle();
    expect(
      tester.widget<SelectableText>(find.byType(SelectableText)).data,
      output,
      reason: 'the whole output shows, selectable',
    );

    Focus.of(tester.element(entry.first)).requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      find.byType(SelectableText),
      findsNothing,
      reason: '⏎ on the entry closes it',
    );
  });

  testWidgets('the mode menu explains each mode and what Allow edits risks', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Approval mode'));
    await tester.pumpAndSettle();
    expect(
      find.text('Asks before every call that is not read-only.'),
      findsOneWidget,
    );
    expect(find.text('Changes files without asking.'), findsOneWidget);
    expect(
      find.text(
        'Any file you can change, including settings that run commands, such '
        'as Git settings Liquid uses for the diff.',
      ),
      findsOneWidget,
    );
    expect(find.text('Runs every call without asking.'), findsOneWidget);
    // On the canvas, as the mockup shows menus, the risk text stays readable (AA).
    final menu = tester.widget<Material>(
      find
          .ancestor(
            of: find.text('Runs every call without asking.'),
            matching: find.byType(Material),
          )
          .last,
    );
    expect(
      (menu.color, menu.surfaceTintColor),
      (const Color(0xFFFFFFFF), Colors.transparent),
    );
  });

  testWidgets('focuses the input area with ⌘L from any tab', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
    // The tab opens and takes the keys before the next key arrives.
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    final composer = tester.widget<TextField>(
      find.byKey(const Key('composer')),
    );
    expect(composer.focusNode?.hasFocus, isTrue);
  });

  testWidgets('keeps what the user typed when a message comes back', (
    tester,
  ) async {
    await show(tester);
    backend
      ..sendGate = Completer()
      ..sendError = 'busy';
    final composer = find.byKey(const Key('composer'));
    await tester.enterText(composer, 'first');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.enterText(composer, 'second');
    backend.sendGate!.complete();
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(composer).controller?.text,
      'first\n\nsecond',
      reason: 'the failed message comes back before the typed text',
    );
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

  testWidgets('the notification list closes for a request, a click or esc', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.notification,
        display: BridgeDisplay(key: '', level: BridgeLevel.warning),
        text: 'lint: 2 warnings',
      ),
    );
    final status = find.byKey(const Key('status'));
    Future<void> open() async {
      await tester.tap(status);
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsOneWidget);
    }

    await open();
    await tester.tapAt(const Offset(800, 200));
    await tester.pumpAndSettle();
    expect(
      find.text('Notifications'),
      findsNothing,
      reason: 'a click elsewhere',
    );

    await open();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsNothing, reason: 'esc');

    await open();
    await emit(
      tester,
      event(BridgeEventKind.requestOpened, request: approval('a1')),
    );
    expect(
      find.text('Notifications'),
      findsNothing,
      reason: 'it would hide the call',
    );
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('opens and clears the notifications from the keyboard', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.notification,
        display: BridgeDisplay(key: '', level: BridgeLevel.warning),
        text: 'lint: 2 warnings',
      ),
    );
    Focus.of(tester.element(find.byKey(const Key('status')))).requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsOneWidget);
    Focus.of(tester.element(find.text('Clear all'))).requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsNothing);
  });

  testWidgets('decides approvals from the keyboard', (tester) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
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
      // An approval takes keys once it could be read (ADR-0207).
      await tester.pump(const Duration(milliseconds: 500));
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

  testWidgets('drops the draft hint before what a session allowance covers', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'edit',
            text: '{}',
          ),
        ),
      ),
    );
    final label = find.text('Allow edit for this session');
    expect(
      tester.renderObject<RenderParagraph>(label).didExceedMaxLines,
      isFalse,
      reason: 'the button says all it covers',
    );
    expect(find.text('your draft is kept'), findsNothing);
  });

  testWidgets('takes no click or key before a decision could be read', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(BridgeEventKind.requestOpened, request: approval('a1')),
    );
    await tester.tap(find.text('Allow once'));
    await tester.pump();
    expect(backend.calls.where((call) => call.startsWith('decide')), isEmpty);
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Allow once'));
    expect(backend.calls.last, 'decide a1 allowOnce');
    await emit(
      tester,
      event(BridgeEventKind.requestResolved, request: BridgeRequest(id: 'a1')),
    );

    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.confirm,
            title: 'Deploy now?',
            detail: '',
            options: const [],
          ),
        ),
      ),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.tap(find.text('Yes'));
    await tester.tap(find.text('Dismiss'));
    await tester.pump();
    expect(backend.calls.where((call) => call.contains('q1')), isEmpty);
    await tester.pump(const Duration(milliseconds: 500));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'confirm q1 true');
  });

  testWidgets('keeps a key held from before out of a question field', (
    tester,
  ) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA);
    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.input,
            title: 'Name',
            detail: '',
            options: const [],
          ),
        ),
      ),
    );
    await tester.pump(const Duration(milliseconds: 500));
    expect(
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.keyA),
      isTrue,
      reason: 'the repeat never reaches the field',
    );
    expect(await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA), isTrue);
    expect(
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyB),
      isFalse,
      reason: 'a key pressed for the question types',
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.keyB);
  });

  testWidgets('says what allowing a call for the session covers', (
    tester,
  ) async {
    await show(tester);
    final long = 'mcp__${'long_server_name_' * 3}__run_ta';
    for (final (kind, tool, label) in [
      (
        BridgeRequestKind.commandExecution,
        'bash',
        'Allow this command for this session',
      ),
      (BridgeRequestKind.fileChange, 'edit', 'Allow edit for this session'),
      (
        BridgeRequestKind.tool,
        long,
        'Allow ${long.substring(0, 23)}… for this session',
      ),
      // Cut by characters, never inside one.
      (
        BridgeRequestKind.tool,
        '${'a' * 22}😀xyz',
        'Allow ${'a' * 22}😀… for this session',
      ),
    ]) {
      await emit(
        tester,
        event(
          BridgeEventKind.requestOpened,
          request: BridgeRequest(
            id: tool,
            approval: BridgeApproval(kind: kind, tool: tool, text: '{}'),
          ),
        ),
      );
      expect(find.text(label), findsOneWidget);
      await emit(
        tester,
        event(
          BridgeEventKind.requestResolved,
          request: BridgeRequest(id: tool),
        ),
      );
    }
  });

  testWidgets('sends, steers and queues from the composer', (tester) async {
    await show(tester);
    final composer = find.byKey(const Key('composer'));
    await tester.enterText(composer, 'Say hi');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(backend.calls, ['send Say hi']);
    expect(
      find.text('Start'),
      findsOneWidget,
      reason: 'a session without a conversation starts with its first message',
    );

    await emit(tester, event(BridgeEventKind.turnStarted));
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
      await emit(tester, event(BridgeEventKind.turnStarted));
      await emit(
        tester,
        event(
          BridgeEventKind.itemStarted,
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
          item: BridgeItem(
            id: 2,
            kind: BridgeItemKind.commandExecution,
            tool: 'bash',
            target: 'npm test -- login',
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
        event(BridgeEventKind.requestOpened, request: approval('a1', item: 2)),
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
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Fixed'));
    await tester.pump();
    expect(backend.calls.last, 'answer q1 Fixed');
  });

  testWidgets('keeps a long question and its answers in the input area', (
    tester,
  ) async {
    await show(tester);
    final long = [for (var part = 0; part < 40; part++) 'line $part'].join(' ');
    BridgeEvent ask(String id, BridgeQuestionKind kind, String detail) => event(
      BridgeEventKind.userInputRequested,
      request: BridgeRequest(
        id: id,
        question: BridgeQuestion(
          kind: kind,
          title: 'Question: $long',
          detail: detail,
          options: const ['Sliding', 'Fixed'],
        ),
      ),
    );
    final area = find.byType(ActionArea);
    void inArea(Finder finder) => expect(
      tester.getRect(area).intersect(tester.getRect(finder)),
      tester.getRect(finder),
      reason: '$finder',
    );

    await emit(tester, ask('q1', BridgeQuestionKind.confirm, 'Detail: $long'));
    inArea(find.text('Yes'));
    inArea(find.text('No'));
    // A question takes answers once it could be read (ADR-0209).
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('No'));
    expect(backend.calls.last, 'confirm q1 false');
    await emit(
      tester,
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q1'),
      ),
    );

    await emit(tester, ask('q2', BridgeQuestionKind.select, ''));
    inArea(find.text('Fixed'));
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Fixed'));
    expect(backend.calls.last, 'answer q2 Fixed');
  });

  testWidgets('fits many extension statuses and a long tool name', (
    tester,
  ) async {
    await show(tester);
    for (var key = 0; key < 8; key++) {
      await emit(
        tester,
        event(
          BridgeEventKind.statusChanged,
          display: BridgeDisplay(key: 'extension-$key', lines: const []),
          text: 'indexing workspace files ($key)',
        ),
      );
    }
    // Model APIs allow tool names of up to 64 characters.
    final tool = 'mcp__${'long_server_name_' * 3}__run_ta';
    expect(tool.length, 64);
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.tool,
            tool: tool,
            text: '{}',
          ),
        ),
      ),
    );
    expect(find.text('Call $tool?'), findsOneWidget);
  });

  /// Scrolls an approval's code to `line`, which is built only once it scrolls into view.
  Future<void> revealLine(WidgetTester tester, String line) =>
      tester.scrollUntilVisible(
        find.text(line),
        20,
        scrollable: find
            .descendant(
              of: find.byType(ActionArea),
              matching: find.byType(Scrollable),
            )
            .first,
      );

  testWidgets('shows a file change as its path and changed lines', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'edit',
            text: '{"edits":[],"path":"src/routes/login.ts"}',
            edit: BridgeEdit(
              path: 'src/routes/login.ts',
              wholeFile: false,
              changes: [
                BridgeEditChange(
                  removed: "router.post('/login', login);",
                  added:
                      'const limit = rateLimit({ max: 5 });\n'
                      "router.post('/login', limit, login);",
                ),
              ],
            ),
          ),
        ),
      ),
    );
    expect(find.text('src/routes/login.ts'), findsOneWidget);
    expect(find.text("− router.post('/login', login);"), findsOneWidget);
    await revealLine(tester, '+ const limit = rateLimit({ max: 5 });');
    await revealLine(tester, "+ router.post('/login', limit, login);");
    expect(find.textContaining('"edits"'), findsNothing);
    await emit(
      tester,
      event(BridgeEventKind.requestResolved, request: BridgeRequest(id: 'a1')),
    );

    // A file's last line break ends its last line; it adds no empty one.
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a2',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'write',
            text: '{}',
            edit: BridgeEdit(
              path: 'notes.md',
              wholeFile: true,
              changes: [BridgeEditChange(removed: '', added: 'one\ntwo\n')],
            ),
          ),
        ),
      ),
    );
    expect(find.text('Replaces the whole file'), findsOneWidget);
    await revealLine(tester, '+ two');
    expect(find.text('+ '), findsNothing);
    expect(
      find.textContaining('more lines'),
      findsNothing,
      reason: 'two is last',
    );
  });

  testWidgets('marks a line break that ends only one side of a replacement', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'edit',
            text: '{}',
            edit: BridgeEdit(
              path: 'install.sh',
              wholeFile: false,
              changes: [
                // Would turn the commented line below into a command.
                BridgeEditChange(
                  removed: '# disabled: ',
                  added: '# disabled: \n',
                ),
                BridgeEditChange(removed: 'keep\n', added: 'kept\n'),
                // Would join the line above with the next one.
                BridgeEditChange(removed: '# on error\n', added: ''),
              ],
            ),
          ),
        ),
      ),
    );
    expect(find.text('− # disabled: '), findsOneWidget);
    await revealLine(tester, '+ # disabled: ↵');
    final line = tester.widget<RichText>(
      find.text('+ # disabled: ↵', findRichText: true),
    );
    TextSpan? mark;
    line.text.visitChildren((span) {
      if (span is TextSpan && span.text == '↵') mark = span;
      return true;
    });
    expect(
      mark?.style?.color,
      tester.element(find.byType(ActionArea)).ws.textFaint,
    );
    await revealLine(tester, '+ kept');
    expect(
      find.text('− keep'),
      findsOneWidget,
      reason: 'a line break both sides end with is not marked',
    );
    await revealLine(tester, '− # on error↵');
  });

  testWidgets('says when a write empties the whole file', (tester) async {
    await show(tester);
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'write',
            text: '{}',
            edit: BridgeEdit(
              path: '/Users/me/.ssh/authorized_keys',
              wholeFile: true,
              changes: [BridgeEditChange(removed: '', added: '')],
            ),
          ),
        ),
      ),
    );
    expect(find.text('Empties the whole file'), findsOneWidget);
  });

  testWidgets('builds only the lines of a large file change that show', (
    tester,
  ) async {
    await show(tester);
    final content = [for (var line = 0; line < 5000; line++) 'line $line'];
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.fileChange,
            tool: 'write',
            text: '{}',
            edit: BridgeEdit(
              path: 'big.txt',
              wholeFile: true,
              changes: [
                BridgeEditChange(removed: '', added: content.join('\n')),
              ],
            ),
          ),
        ),
      ),
    );
    expect(find.text('Replaces the whole file'), findsOneWidget);
    expect(
      find.text('+ line 4999'),
      findsNothing,
      reason: 'a line far below is built once it scrolls into view',
    );
    expect(find.textContaining('more lines'), findsOneWidget);
  });

  testWidgets(
    'counts the lines below a file change by its lines, however they wrap',
    (tester) async {
      await show(tester);
      final lines = [
        for (var line = 0; line < 6; line++) '$line${'x' * 400}',
        for (var line = 0; line < 100; line++) 'short $line',
      ];
      await emit(
        tester,
        event(
          BridgeEventKind.requestOpened,
          request: BridgeRequest(
            id: 'a1',
            approval: BridgeApproval(
              kind: BridgeRequestKind.fileChange,
              tool: 'write',
              text: '{}',
              edit: BridgeEdit(
                path: 'notes.md',
                wholeFile: true,
                changes: [
                  BridgeEditChange(removed: '', added: lines.join('\n')),
                ],
              ),
            ),
          ),
        ),
      );
      // Lines leave the tree once they scroll out of view, so the view is found once.
      final code = tester.getRect(
        find
            .ancestor(
              of: find.text('notes.md'),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      Future<void> expectExactHint() async {
        await tester.pumpAndSettle();
        final shown = lines.lastIndexWhere((line) {
          final text = find.text('+ $line');
          return text.evaluate().isNotEmpty &&
              tester.getRect(text).top < code.bottom;
        });
        final below = lines.length - 1 - shown;
        expect(find.text('↓ $below more lines'), findsOneWidget);
      }

      await expectExactHint();
      await tester.dragFrom(code.center, const Offset(0, -300));
      await expectExactHint();
    },
  );

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
            text: command,
          ),
        ),
      ),
    );
    expect(find.text(command), findsOneWidget, reason: 'the command itself');
    expect(find.textContaining('more lines'), findsOneWidget);
    final view = tester.getRect(
      find
          .ancestor(of: find.text(command), matching: find.byType(Scrollable))
          .first,
    );
    expect(
      tester.getRect(find.textContaining('more lines')).overlaps(view),
      isFalse,
      reason: 'the hint covers none of the call',
    );
    await tester.drag(
      find
          .ancestor(of: find.text(command), matching: find.byType(Scrollable))
          .first,
      const Offset(0, -2000),
    );
    await tester.pumpAndSettle();
    expect(
      find.textContaining('more lines'),
      findsNothing,
      reason: 'nothing hidden at the end',
    );
    await tester.pump(const Duration(milliseconds: 500));
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
    await tester.pump(const Duration(milliseconds: 500));
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

  testWidgets('shows the lines a file change added and removed', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    for (final (id, tool, added, removed) in [
      (5, 'edit', 9, 2),
      (6, 'write', 38, 0),
    ]) {
      await emit(
        tester,
        event(
          BridgeEventKind.itemStarted,
          item: item(id, kind: BridgeItemKind.fileChange, tool: tool),
        ),
      );
      await emit(
        tester,
        event(
          BridgeEventKind.itemCompleted,
          item: item(
            id,
            kind: BridgeItemKind.fileChange,
            tool: tool,
            durationMs: 9,
            added: added,
            removed: removed,
          ),
        ),
      );
    }
    expect(find.text('+9 −2', findRichText: true), findsOneWidget);
    expect(find.text('+38', findRichText: true), findsOneWidget);
    expect(find.text('9 ms'), findsNothing, reason: 'the lines say more');
  });

  testWidgets('shows thinking without promising to open it', (tester) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(
      tester,
      event(
        BridgeEventKind.itemStarted,
        item: item(4, kind: BridgeItemKind.reasoning),
      ),
    );
    final thinking = find.ancestor(
      of: find.text('Thinking'),
      matching: find.byType(ItemTile),
    );
    expect(
      find.descendant(of: thinking, matching: find.byType(Icon)),
      findsNothing,
      reason: 'a chevron would suggest it opens',
    );
  });

  testWidgets('marks only the call an approval asks about as waiting', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    // pi starts both calls, the first is allowed and the second asks.
    for (final id in [5, 6]) {
      await emit(
        tester,
        event(
          BridgeEventKind.itemStarted,
          item: item(id, kind: BridgeItemKind.commandExecution, tool: 'bash'),
        ),
      );
    }
    await emit(
      tester,
      event(
        BridgeEventKind.requestOpened,
        request: BridgeRequest(
          id: 'a2',
          approval: BridgeApproval(
            kind: BridgeRequestKind.commandExecution,
            tool: 'bash',
            text: 'pwd',
            item: 6,
          ),
        ),
      ),
    );
    expect(find.text('waiting for approval'), findsOneWidget);
  });

  testWidgets('marks a tool done when only its outcome changes', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(
      tester,
      event(
        BridgeEventKind.itemStarted,
        item: item(5, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    expect(find.byIcon(LucideIcons.loader), findsOneWidget);
    await emit(
      tester,
      event(
        BridgeEventKind.itemCompleted,
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

    await emit(
      tester,
      event(
        BridgeEventKind.itemStarted,
        item: item(6, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    await emit(
      tester,
      event(
        BridgeEventKind.itemCompleted,
        item: item(
          6,
          kind: BridgeItemKind.commandExecution,
          tool: 'bash',
          durationMs: 84213,
        ),
      ),
    );
    expect(find.text('1m 24s'), findsOneWidget, reason: 'as the Trajectory');
  });

  testWidgets('shows contextual shortcuts only while Cmd alone is held', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
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

  testWidgets('the status bar names the Diff tab\'s keys while Cmd is held', (
    tester,
  ) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(
      tester.getSemantics(find.text('Diff').first),
      isSemantics(isSelected: true),
      reason: 'the open tab is told, not only drawn',
    );
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 700));
    expect(
      find.text(
        '↑↓ file · ←→ folder · ⌥↑↓ hunk · U unified/split · W whitespace',
      ),
      findsOneWidget,
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
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

  testWidgets('stops the agent before the app exits', (tester) async {
    await show(tester);
    AppExitResponse? response;
    unawaited(
      tester.binding.handleRequestAppExit().then((answer) => response = answer),
    );
    await tester.pump();
    expect(
      response,
      AppExitResponse.exit,
      reason: 'an idle agent lets Liquid quit',
    );
    expect(backend.calls, contains('shutdown'));
  });
}
