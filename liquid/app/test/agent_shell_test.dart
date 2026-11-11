import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_view.dart';
import 'package:liquid/src/trajectory/tree_view.dart';
import 'package:liquid/src/trajectory/trajectory_view.dart';
import 'package:liquid/src/agent/action_area.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/worktrees.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/theme/logo.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/theme/tokens.g.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';

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

  /// Holds ⌘ long enough for the contextual shortcuts to show.
  Future<void> holdCommand(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 700));
  }

  Future<void> emit(WidgetTester tester, BridgeEvent event) async {
    backend.controller.add(event);
    await tester.pumpAndSettle();
  }

  BridgeEvent confirm(String id) => event(
    BridgeEventKind.userInputRequested,
    request: BridgeRequest(
      id: id,
      question: BridgeQuestion(
        kind: BridgeQuestionKind.confirm,
        title: 'Deploy now?',
        detail: '',
        options: const [],
      ),
    ),
  );

  testWidgets('a turn\'s end in the chat offers to revert its files', (
    tester,
  ) async {
    await show(tester);
    BridgeTreeNode node(String id, String? parent, BridgeEntryKind kind) =>
        BridgeTreeNode(
          failed: false,
          id: id,
          parent: parent,
          kind: kind,
          preview: id,
        );
    backend
      ..stored = historyOf(
        [],
        tree: [
          node('c1', null, BridgeEntryKind.custom),
          node('u1', 'c1', BridgeEntryKind.user),
          node('a1', 'u1', BridgeEntryKind.assistant),
          node('c2', 'a1', BridgeEntryKind.custom),
        ],
        leaf: 'c2',
      )
      ..transcripts[''] = BridgeTranscript(
        items: [
          chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
          chatItem('a1', BridgeItemKind.assistantMessage, text: 'Done'),
        ],
        more: false,
      )
      ..turnList = [
        turnOf(
          'T1',
          number: 1,
          startEntry: 'c1',
          endEntry: 'c2',
          added: 2,
          removed: 1,
        ),
      ]
      ..revertPlanned = revertPlanOf(
        files: [revertedFile('a.txt', BridgeChangeStatus.modified)],
      );
    await state.loadChat();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Revert files to before this turn…'));
    await tester.pumpAndSettle();
    expect(find.text('Revert to before turn 1?'), findsOneWidget);
    await tester.tap(find.text('Revert and branch'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('revert T1'));
  });

  testWidgets(
    'a question that arrives while a revert is prepared keeps the keys',
    (tester) async {
      await show(tester);
      BridgeTreeNode node(String id, String? parent, BridgeEntryKind kind) =>
          BridgeTreeNode(
            failed: false,
            id: id,
            parent: parent,
            kind: kind,
            preview: id,
          );
      backend
        ..stored = historyOf(
          [],
          tree: [
            node('c1', null, BridgeEntryKind.custom),
            node('u1', 'c1', BridgeEntryKind.user),
            node('a1', 'u1', BridgeEntryKind.assistant),
            node('c2', 'a1', BridgeEntryKind.custom),
          ],
          leaf: 'c2',
        )
        ..transcripts[''] = BridgeTranscript(
          items: [
            chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
            chatItem('a1', BridgeItemKind.assistantMessage, text: 'Done'),
          ],
          more: false,
        )
        ..turnList = [
          turnOf(
            'T1',
            number: 1,
            startEntry: 'c1',
            endEntry: 'c2',
            added: 2,
            removed: 1,
          ),
        ]
        ..revertPlanned = revertPlanOf(
          files: [revertedFile('a.txt', BridgeChangeStatus.modified)],
        )
        ..revertPlanGate = Completer();
      await state.loadChat();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Revert files to before this turn…'));
      await tester.pump();
      await emit(tester, confirm('q1'));
      backend.revertPlanGate!.complete();
      await tester.pumpAndSettle();
      expect(find.text('Revert to before turn 1?'), findsNothing);
      expect(backend.calls, isNot(contains('revert T1')));
    },
  );

  testWidgets(
    'the input area floats at the bottom of each tab\'s main column',
    (tester) async {
      await show(tester);
      await tester.pumpAndSettle();
      Future<void> openTab(LogicalKeyboardKey digit) async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
        await tester.sendKeyEvent(digit);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
        await tester.pumpAndSettle();
      }

      final composer = find.byKey(const Key('composer'));
      final area = find.byType(ActionArea);
      await tester.enterText(composer, 'draft');
      double center(Finder finder) => tester.getCenter(finder).dx;
      expect(
        center(area),
        closeTo(center(find.byType(ChatView)), 1),
        reason: 'in the reading width with the messages',
      );
      expect(
        tester.getRect(find.byType(ChatView)).bottom,
        greaterThan(tester.getRect(area).bottom),
        reason: 'the content runs on under the input area',
      );

      await openTab(LogicalKeyboardKey.digit2);
      final trajectory = tester.getRect(find.byType(TrajectoryView));
      final tree = tester.getRect(find.byType(SessionTreeView));
      expect(
        center(area),
        closeTo((trajectory.left + tree.left) / 2, 1),
        reason: 'over the column left of the session tree',
      );
      expect(trajectory.bottom, greaterThan(tester.getRect(area).bottom));
      expect(
        tester.widget<TextField>(composer).controller?.text,
        'draft',
        reason: 'the input area moves with the tab and keeps its text',
      );

      await openTab(LogicalKeyboardKey.digit3);
      expect(center(area), closeTo(center(find.byType(DiffView)), 1));
    },
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

  testWidgets('heads the sidebar with the Liquid mark and name', (
    tester,
  ) async {
    await show(tester);
    await tester.pumpAndSettle();
    final sidebar = find.byType(Sidebar);
    final mark = find.descendant(
      of: sidebar,
      matching: find.byType(LiquidMark),
    );
    final name = find.descendant(of: sidebar, matching: find.text('Liquid'));
    expect(tester.getSize(mark), const Size.square(18));
    expect(tester.getCenter(mark).dy, closeTo(tester.getCenter(name).dy, 1));
    expect(tester.getRect(mark).right, lessThan(tester.getRect(name).left));
    expect(
      tester.getRect(name).bottom,
      lessThan(tester.getRect(find.text('Agent sessions')).top),
      reason: 'above the sessions',
    );
  });

  testWidgets('the Diff tab counts the files the shown turn changed', (
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

  testWidgets('keeps the indentation and line ends typed while a send fails', (
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
    await tester.enterText(composer, '    nested_call()\n');
    backend.sendGate!.complete();
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(composer).controller?.text,
      'first\n\n    nested_call()\n',
      reason: 'the typed text comes back exactly as it was typed',
    );
  });

  testWidgets(
    'keeps the formatting of the same text typed again while it was sent',
    (tester) async {
      await show(tester);
      backend
        ..sendGate = Completer()
        ..sendError = 'busy';
      final composer = find.byKey(const Key('composer'));
      await tester.enterText(composer, 'nested_call()');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      await tester.enterText(composer, '    nested_call()\n');
      backend.sendGate!.complete();
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(composer).controller?.text,
        '    nested_call()\n',
        reason: 'the message is not doubled, and its retyped form keeps its whitespace',
      );
    },
  );

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

  testWidgets(
    '⌘Z after a sent message does not bring the message back',
    (tester) async {
      await show(tester);
      final composer = find.byKey(const Key('composer'));
      await tester.enterText(composer, 'Say hi');
      await tester.pump(const Duration(milliseconds: 600));
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump(const Duration(milliseconds: 600));
      await tester.tap(composer);
      await undo(tester);
      expect(tester.widget<TextField>(composer).controller?.text, '');
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets(
    '⌘Z after text handed back to the composer does not bring back what it replaced',
    (tester) async {
      await show(tester);
      final composer = find.byKey(const Key('composer'));
      await tester.enterText(composer, 'Plan');
      await tester.pump(const Duration(milliseconds: 600));
      await emit(tester, event(BridgeEventKind.inputTextSet, text: 'Retry'));
      expect(
        tester.widget<TextField>(composer).controller?.text,
        'Retry\n\nPlan',
      );
      await tester.pump(const Duration(milliseconds: 600));
      await tester.tap(composer);
      await undo(tester);
      expect(
        tester.widget<TextField>(composer).controller?.text,
        'Retry\n\nPlan',
      );
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets(
    'typing before the next frame after text is handed back keeps the typed text',
    (tester) async {
      await show(tester);
      final composer = find.byKey(const Key('composer'));
      await tester.enterText(composer, 'Plan');
      await tester.tap(composer);
      state.draft.value = 'Retry';
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: 'Retry\n\nPlanx',
          selection: TextSelection.collapsed(offset: 12),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(composer).controller?.text,
        'Retry\n\nPlanx',
      );
    },
  );

  testWidgets(
    'a hand-back that changes no text keeps the undo history',
    (tester) async {
      await show(tester);
      final composer = find.byKey(const Key('composer'));
      await tester.enterText(composer, 'Hello');
      await tester.pump(const Duration(milliseconds: 600));
      await tester.enterText(composer, 'Hello there');
      await tester.pump(const Duration(milliseconds: 600));
      state.draft.value = 'Hello there';
      await tester.pump(const Duration(milliseconds: 600));
      await tester.tap(composer);
      await undo(tester);
      expect(tester.widget<TextField>(composer).controller?.text, 'Hello');
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets('marks the branch in the rebuilt chat and reads earlier items', (
    tester,
  ) async {
    await show(tester);
    backend.stored = historyOf(
      [],
      tree: [
        const BridgeTreeNode(
          failed: false,
          id: 'u1',
          kind: BridgeEntryKind.user,
          preview: 'Go',
        ),
        const BridgeTreeNode(
          failed: false,
          id: 'a1',
          parent: 'u1',
          kind: BridgeEntryKind.assistant,
          preview: 'Looked',
        ),
        const BridgeTreeNode(
          failed: false,
          id: 'a2',
          parent: 'a1',
          kind: BridgeEntryKind.assistant,
          preview: 'Done',
        ),
        const BridgeTreeNode(
          failed: false,
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

    await tester.drag(
      find
          .descendant(
            of: find.byType(ChatView),
            matching: find.byType(ListView),
          )
          .first,
      const Offset(0, 4000),
    );
    await tester.pumpAndSettle();
    expect(backend.calls, contains('transcript u1'));
  });

  testWidgets('a short newest page reads earlier messages without scrolling', (
    tester,
  ) async {
    await show(tester);
    backend.transcripts[''] = BridgeTranscript(
      items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
      more: true,
      start: 'u2',
    );
    backend.transcripts['u2'] = BridgeTranscript(
      items: [chatItem('u1', BridgeItemKind.userMessage, text: 'First')],
      more: false,
      start: 'u1',
    );
    await state.loadChat();
    await tester.pumpAndSettle();
    expect(
      find.text('First'),
      findsOneWidget,
      reason: 'the newest page fits the viewport, so nothing can scroll',
    );
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

  testWidgets('the notification list closes for a question, a click or esc', (
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
    await emit(tester, confirm('q1'));
    expect(
      find.text('Notifications'),
      findsNothing,
      reason: 'it would hide the question',
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

  testWidgets('takes no click or key before a question could be read', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, confirm('q1'));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.tap(find.text('Yes'));
    await tester.tap(find.text('Dismiss'));
    await tester.pump();
    expect(backend.calls.where((call) => call.contains('q1')), isEmpty);
    await tester.pump(const Duration(milliseconds: 500));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'confirm q1 true');
  });

  testWidgets(
    'counts the wait for an answer from its first frame, however late',
    (tester) async {
      await show(tester);
      backend.controller.add(confirm('q1'));
      await tester.idle();
      // The card is built and laid out now, but its frame, slow to lay out, ends 20 s later.
      tester.binding.buildOwner!.buildScope(tester.binding.rootElement!);
      tester.binding.rootPipelineOwner.flushLayout();
      expect(find.text('Yes', skipOffstage: false), findsOneWidget);
      await tester.pump(const Duration(seconds: 20));
      await tester.tap(find.text('Yes'));
      await tester.pump();
      expect(
        backend.calls.where((call) => call.startsWith('confirm')),
        isEmpty,
      );
      await tester.pump(const Duration(milliseconds: 500));
      await tester.tap(find.text('Yes'));
      expect(backend.calls.last, 'confirm q1 true');
    },
  );

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

  testWidgets('typing in the input area never runs a tab\'s own keys', (
    tester,
  ) async {
    final diffSettings = DiffSettingsController();
    await show(tester, diffSettings: diffSettings);
    await tester.pumpAndSettle();
    final composer = find.byKey(const Key('composer'));
    Future<void> openTab(LogicalKeyboardKey digit) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(digit);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
    }

    await openTab(LogicalKeyboardKey.digit2);
    await tester.tap(composer);
    await tester.pump();
    for (final key in [
      LogicalKeyboardKey.keyC,
      LogicalKeyboardKey.keyB,
      LogicalKeyboardKey.space,
    ]) {
      await tester.sendKeyEvent(key);
    }
    await tester.pumpAndSettle();
    expect(state.notice.value, isNull, reason: 'c did not compare');
    expect(
      backend.calls.where((call) => call.startsWith('branch')),
      isEmpty,
      reason: 'b did not branch',
    );

    await openTab(LogicalKeyboardKey.digit3);
    expect(find.byType(DiffView), findsOneWidget);
    await tester.tap(composer);
    await tester.pump();
    final before = diffSettings.value;
    await tester.sendKeyEvent(LogicalKeyboardKey.keyU);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    await tester.pumpAndSettle();
    expect(diffSettings.value, before, reason: 'u and w typed, not switched');
  });

  testWidgets('the session header names the project and the model', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()..model = 'faux/faux-1';
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await tester.pumpAndSettle();
    Finder inHeader(String text) => find.descendant(
      of: find.byKey(const Key('session-header')),
      matching: find.text(text),
    );
    expect(inHeader('acme-api'), findsOneWidget);
    expect(inHeader('faux-1'), findsOneWidget);
  });

  testWidgets(
    'the PR chip sits between the branch chip and the Git action and refreshes on focus',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      backend = FakeBackend()
        ..git = gitStatusOf(
          branch: 'feature/retry-after',
          upstream: 'origin/feature/retry-after',
        )
        ..current = pullRequestOutcomeOf(pullRequest: pullRequestOf());
      state = SessionState(backend);
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: AgentShell(state: state, info: info),
        ),
      );
      await tester.pumpAndSettle();
      double left(Finder finder) => tester
          .getTopLeft(
            find.descendant(
              of: find.byKey(const Key('session-header')),
              matching: finder,
            ),
          )
          .dx;
      expect(
        left(find.byKey(const Key('branch-chip'))),
        lessThan(left(find.byKey(const Key('pull-request-chip')))),
      );
      expect(
        left(find.byKey(const Key('pull-request-chip'))),
        lessThan(left(find.byKey(const Key('git-next')))),
      );
      int reads() => backend.pullRequestReads;
      final before = reads();
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      expect(reads(), before + 1);
      state.git.stop();
    },
  );

  testWidgets('messages start at the top and flow down', (tester) async {
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
      event(BridgeEventKind.contentDelta, item: item(1), text: 'Pong!'),
    );
    final chat = tester.getRect(find.byType(ChatView));
    expect(
      tester.getRect(find.text('Pong!')).top - chat.top,
      lessThan(80),
      reason: 'the first message sits at the top, not above the input',
    );
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

  testWidgets('removing a repeated queued message keeps the others in order', (
    tester,
  ) async {
    await show(tester);
    const queue = BridgeQueue(
      steering: [],
      followUp: ['build', 'test', 'build'],
    );
    backend.queued = queue;
    await emit(tester, event(BridgeEventKind.queueChanged, queue: queue));
    await tester.tap(find.byKey(const Key('unqueue 2')));
    await tester.pumpAndSettle();
    expect(backend.calls, [
      'clear queue',
      'follow-up build',
      'follow-up test',
    ], reason: 'the last build leaves; the first one keeps its place');
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
      await emit(tester, confirm('q1'));
      expect(find.text('Now I run the tests.'), findsOneWidget);
      expect(find.text('todo  ✓ limiter  ○ run tests'), findsOneWidget);
      expect(find.text('● waiting'), findsOneWidget);
    },
  );

  testWidgets('the post-push offer keeps the last diff line in view', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()
      ..git = gitStatusOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
        ahead: 2,
      )
      ..changed = [
        BridgeFileChange(
          path: 'long.ts',
          shown: 'long.ts',
          status: BridgeChangeStatus.added,
          added: 200,
          removed: 0,
        ),
      ]
      ..patches = {
        'long.ts': [
          '@@ -0,0 +1,200 @@',
          for (var line = 1; line <= 200; line++) '+line $line',
        ].join('\n'),
      };
    state = SessionState(backend);
    await state.git.refresh();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, parseDiff: parsePatch),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('git-next')));
    await tester.pumpAndSettle();
    expect(find.text('Create pull request…'), findsOneWidget);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();

    final first = find.text('line 1', findRichText: true);
    await tester.drag(first, const Offset(0, -10000));
    await tester.pumpAndSettle();
    expect(
      tester.getBottomLeft(find.text('line 200', findRichText: true)).dy,
      lessThanOrEqualTo(tester.getTopLeft(find.byType(WidgetStrip)).dy),
    );
    await tester.pump(const Duration(seconds: 7));
    state.git.stop();
  });

  group('a question that closes a running dialog', () {
    Future<void> showBranch(WidgetTester tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      backend = FakeBackend()
        ..git = gitStatusOf(branch: 'feature/retry-after')
        ..worktreeList = [
          worktreeOf('/code/api', branch: 'main', main: true, current: true),
          worktreeOf('/code/api-hotfix', branch: 'hotfix/login'),
        ];
      backend.reviews['/code/api-hotfix'] = removalOf(
        backend.worktreeList.last,
      );
      state = SessionState(backend);
      await state.git.refresh();
      await state.pulls.refresh();
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: AgentShell(state: state, info: info),
        ),
      );
      await tester.pumpAndSettle();
    }

    Iterable<String> notices() => state.notices.value.map((n) => n.text);

    testWidgets('reports a pull request creation that failed meanwhile', (
      tester,
    ) async {
      await showBranch(tester);
      await tester.tap(find.byKey(const Key('git-menu')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Create pull request…').last);
      await tester.pumpAndSettle();
      final gate = backend.createGate = Completer();
      await tester.tap(find.widgetWithText(InkWell, 'Create pull request'));
      await tester.pump();

      await emit(tester, confirm('q1'));
      expect(find.text('Create pull request'), findsNothing);
      gate.complete(
        pullRequestOutcomeOf(
          kind: BridgePullRequestOutcomeKind.notCreated,
          detail: 'Validation Failed (HTTP 422)',
        ),
      );
      await tester.pumpAndSettle();
      expect(notices(), [
        'GitHub refused the pull request; none was created. “Validation Failed (HTTP 422)”',
      ]);
      state.git.stop();
    });

    testWidgets('reports a worktree removal that failed meanwhile', (
      tester,
    ) async {
      await showBranch(tester);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.comma);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Worktrees'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(InkWell, 'Remove…'));
      await tester.pumpAndSettle();
      final gate = backend.removeGate = Completer();
      await tester.tap(find.widgetWithText(InkWell, 'Remove worktree'));
      await tester.pump();

      await emit(tester, confirm('q1'));
      expect(find.text('Remove hotfix/login worktree?'), findsNothing);
      gate.complete(
        const BridgeWorktreeOutcome(
          kind: BridgeWorktreeOutcomeKind.failed,
          running: [],
          detail: 'fatal: validation failed',
        ),
      );
      await tester.pumpAndSettle();
      expect(notices(), [
        'Could not remove the worktree at /code/api-hotfix: “fatal: validation failed” It stays listed until Git removes it.',
      ]);
      state.git.stop();
    });

    testWidgets('warns of a worktree removal that changed meanwhile', (
      tester,
    ) async {
      await showBranch(tester);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.comma);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Worktrees'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(InkWell, 'Remove…'));
      await tester.pumpAndSettle();
      final gate = backend.removeGate = Completer();
      await tester.tap(find.widgetWithText(InkWell, 'Remove worktree'));
      await tester.pump();

      await emit(tester, confirm('q1'));
      expect(find.text('Remove hotfix/login worktree?'), findsNothing);
      gate.complete(
        const BridgeWorktreeOutcome(
          kind: BridgeWorktreeOutcomeKind.changed,
          running: [],
          detail: '',
        ),
      );
      await tester.pumpAndSettle();
      expect(notices(), [
        '/code/api-hotfix changed since this dialog opened. Nothing was removed.',
      ]);
      expect(state.notices.value.single.level, BridgeLevel.warning);
      await tester.pump(const Duration(seconds: 7));
      state.git.stop();
    });
  });

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
    // A question takes answers once it could be read.
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

  testWidgets('names extension statuses but not Watershed\'s own', (
    tester,
  ) async {
    await show(tester);
    for (final (key, text) in [
      ('watershed.revert', 'Waiting for a revert in this checkout to finish'),
      ('todo', '2 left'),
      // An extension's key that only looks like Watershed's still shows.
      ('watershed.other', 'busy'),
    ]) {
      await emit(
        tester,
        event(
          BridgeEventKind.statusChanged,
          display: BridgeDisplay(key: key, lines: const []),
          text: text,
        ),
      );
    }
    expect(
      find.text(
        'Waiting for a revert in this checkout to finish · todo 2 left · '
        'watershed.other busy',
      ),
      findsOneWidget,
    );
  });

  testWidgets('fits many extension statuses', (tester) async {
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

  testWidgets('filtering after scrolling shows the option Enter picks', (
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
            title: 'Pick an option',
            detail: '',
            options: [
              for (var number = 0; number < 100; number++)
                'Option ${number.toString().padLeft(2, '0')}',
            ],
          ),
        ),
      ),
    );
    for (var step = 0; step < 50; step++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    }
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('filter')), 'Option');
    await tester.pumpAndSettle();
    expect(
      find.text('Option 00').hitTestable(),
      findsOneWidget,
      reason: 'the highlighted first match is in view',
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'answer q1 Option 00');
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

  testWidgets('fits a tool with a 64-character name on its row', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    final tool = 'mcp_${'x' * 60}';
    await emit(
      tester,
      event(
        BridgeEventKind.itemStarted,
        item: item(7, kind: BridgeItemKind.dynamicToolCall, tool: tool),
      ),
    );
    await emit(
      tester,
      event(
        BridgeEventKind.itemCompleted,
        item: item(
          7,
          kind: BridgeItemKind.dynamicToolCall,
          tool: tool,
          durationMs: 40,
        ),
      ),
    );
    expect(tester.takeException(), isNull);
    expect(find.text('40 ms'), findsOneWidget, reason: 'the outcome stays');
  });

  testWidgets('shows contextual shortcuts only while Cmd alone is held', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(
      tester,
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.editor,
            title: 'Commit message',
            detail: '',
            options: const [],
          ),
        ),
      ),
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
        'T turn · ↑↓ file · ←→ folder · ⌥↑↓ hunk · U unified/split · W whitespace',
      ),
      findsOneWidget,
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
  });

  testWidgets('underlines only the open tab, in the accent', (tester) async {
    await show(tester);
    await tester.pumpAndSettle();
    Color underline(String tab) =>
        (tester
                    .widget<Container>(
                      find
                          .ancestor(
                            of: find.text(tab),
                            matching: find.byType(Container),
                          )
                          .first,
                    )
                    .decoration!
                as BoxDecoration)
            .border!
            .bottom
            .color;
    expect(underline('Chat'), WsPalette.light.accent);
    expect(underline('Trajectory'), Colors.transparent);
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
