import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_view.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeBackend backend;

  Future<void> show(WidgetTester tester, ShortcutSettings settings) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: SessionState(backend),
          info: info,
          shortcuts: ShortcutController.memory(settings),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// Presses `key` while the Liquid key is held; whether Liquid took the key.
  Future<bool> liquid(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(liquidKey, platform: 'macos');
    final handled = await tester.sendKeyEvent(key, platform: 'macos');
    await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
    await tester.pumpAndSettle();
    return handled;
  }

  Future<void> hold(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(key, platform: 'macos');
    await tester.pump(const Duration(milliseconds: 700));
  }

  int newSessions() =>
      backend.calls.where((call) => call == 'new session').length;

  testWidgets(
    'the Liquid key runs Liquid commands, and their ⌘ keys still do',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());

      expect(await liquid(tester, LogicalKeyboardKey.keyN), isTrue);
      expect(newSessions(), 1);
      await tester.sendKeyDownEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.keyN, platform: 'macos');
      await tester.sendKeyUpEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );
      await tester.pumpAndSettle();
      expect(newSessions(), 2);

      await liquid(tester, LogicalKeyboardKey.digit3);
      expect(find.byType(DiffView), findsOneWidget);
    },
  );

  testWidgets(
    'holding the Liquid key shows its shortcuts and ⌘ no longer does',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());
      backend.controller.add(event(BridgeEventKind.turnStarted, turn: 1));
      backend.controller.add(
        event(
          BridgeEventKind.requestOpened,
          request: const BridgeRequest(
            id: 'a1',
            approval: BridgeApproval(
              kind: BridgeRequestKind.commandExecution,
              tool: 'bash',
              text: 'git push',
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await hold(tester, LogicalKeyboardKey.metaLeft);
      expect(find.text('⇪⏎'), findsNothing);
      await tester.sendKeyUpEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );

      await hold(tester, liquidKey);
      expect(find.text('⇪⏎'), findsOneWidget);
      expect(find.text('⇪1'), findsOneWidget);
      expect(find.text('esc'), findsWidgets);
      await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
      await tester.pump();
      expect(find.text('⇪⏎'), findsNothing);
    },
  );

  testWidgets('a changed shortcut replaces its default', (tester) async {
    await show(
      tester,
      ShortcutSettings.liquidKey().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyD, liquid: true),
      ),
    );

    await liquid(tester, LogicalKeyboardKey.digit3);
    expect(find.byType(DiffView), findsNothing);
    await liquid(tester, LogicalKeyboardKey.keyD);
    expect(find.byType(DiffView), findsOneWidget);
  });

  testWidgets('the Standard profile leaves the Liquid key alone', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());

    expect(await liquid(tester, LogicalKeyboardKey.keyN), isFalse);
    expect(newSessions(), 0);
    await hold(tester, liquidKey);
    expect(find.text('⌘1'), findsNothing);
    await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
    await hold(tester, LogicalKeyboardKey.metaLeft);
    expect(find.text('⏎'), findsOneWidget, reason: '⌘ reveals the hints');
    expect(
      find.text('⌘1'),
      findsNothing,
      reason: 'the menu bar shows the app-wide keys',
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
  });

  testWidgets('⇪⏎ on an approval allows for the session, not once as well', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.liquidKey());
    backend.controller.add(event(BridgeEventKind.turnStarted, turn: 1));
    backend.controller.add(
      event(
        BridgeEventKind.requestOpened,
        request: const BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.commandExecution,
            tool: 'bash',
            text: 'git push',
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    // An approval takes keys once it could be read (ADR-0207).
    await tester.pump(const Duration(milliseconds: 500));

    await liquid(tester, LogicalKeyboardKey.enter);
    expect(backend.calls.where((call) => call.startsWith('decide')), [
      'decide a1 allowSession',
    ]);
  });

  testWidgets(
    'an input question answers with its keys, not with ⏎ once rebound',
    (tester) async {
      await show(
        tester,
        ShortcutSettings.standard().change(
          Command.answer,
          const Keys(LogicalKeyboardKey.keyY, meta: true),
        ),
      );
      backend.controller.add(
        event(
          BridgeEventKind.userInputRequested,
          request: BridgeRequest(
            id: 'q1',
            question: BridgeQuestion(
              kind: BridgeQuestionKind.input,
              title: 'Your name',
              detail: '',
              options: const [],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('answer')), 'Bob');
      // An unbound ⏎ reaches the text field, which reports it as its done action.
      await tester.sendKeyEvent(LogicalKeyboardKey.enter, platform: 'macos');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.startsWith('answer')), isEmpty);
      await tester.sendKeyDownEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY, platform: 'macos');
      await tester.sendKeyUpEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.startsWith('answer')), [
        'answer q1 Bob',
      ]);
    },
  );

  testWidgets('the Trajectory keys in the status bar follow the shortcuts', (
    tester,
  ) async {
    await show(
      tester,
      ShortcutSettings.standard().change(
        Command.branch,
        const Keys(LogicalKeyboardKey.keyX),
      ),
    );
    await tester.tap(find.text('Trajectory'));
    await tester.pumpAndSettle();

    await hold(tester, LogicalKeyboardKey.metaLeft);
    expect(find.textContaining('X branch'), findsOneWidget);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
  });
}
