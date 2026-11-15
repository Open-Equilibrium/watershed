import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeProject project;

  /// Shows an open session with `settings`, its keys as the person configured them.
  Future<void> show(WidgetTester tester, ShortcutSettings settings) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    project = FakeProject();
    final state = SessionState(project);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
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

  /// Presses `key` with ⌘, as the macOS keyboard sends it.
  Future<void> command(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(
      LogicalKeyboardKey.metaLeft,
      platform: 'macos',
    );
    await tester.sendKeyEvent(key, platform: 'macos');
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
    await tester.pumpAndSettle();
  }

  /// Asks the agent a question with one text answer, in place of the composer.
  Future<void> askInput(WidgetTester tester, String text) async {
    project.session.emit(
      event(
        BridgeEventKind.requestOpened,
        1,
        request: questionsOf('q1', [questionOf('name', text)]),
      ),
    );
    await tester.pumpAndSettle();
    // A question takes keys once it could be read.
    await tester.pump(const Duration(milliseconds: 500));
  }

  testWidgets(
    'the Liquid key runs a Liquid command, and its ⌘ key runs it too',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());

      expect(await liquid(tester, LogicalKeyboardKey.keyD), isTrue);
      expect(find.byType(DiffPanel), findsOneWidget);
      await command(tester, LogicalKeyboardKey.keyD);
      expect(find.byType(DiffPanel), findsNothing);
    },
  );

  testWidgets(
    'holding the Liquid key shows its shortcuts and ⌘ no longer does',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());

      await tester.sendKeyDownEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );
      await tester.pump(const Duration(milliseconds: 700));
      expect(find.text('⇪1'), findsNothing);
      await tester.sendKeyUpEvent(
        LogicalKeyboardKey.metaLeft,
        platform: 'macos',
      );

      await tester.sendKeyDownEvent(liquidKey, platform: 'macos');
      await tester.pump(const Duration(milliseconds: 700));
      expect(find.text('⇪1'), findsOneWidget);
      await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
      await tester.pump();
      expect(find.text('⇪1'), findsNothing);
    },
  );

  testWidgets('a changed shortcut replaces its default', (tester) async {
    await show(
      tester,
      ShortcutSettings.liquidKey().change(
        Command.diff,
        const Keys(LogicalKeyboardKey.keyE, liquid: true),
      ),
    );

    await liquid(tester, LogicalKeyboardKey.keyD);
    expect(find.byType(DiffPanel), findsNothing);
    await liquid(tester, LogicalKeyboardKey.keyE);
    expect(find.byType(DiffPanel), findsOneWidget);
  });

  testWidgets('the Standard profile leaves the Liquid key alone', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());

    expect(await liquid(tester, LogicalKeyboardKey.keyD), isFalse);
    expect(find.byType(DiffPanel), findsNothing);
  });

  testWidgets('⏎ answers a question with its typed text, once', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await askInput(tester, 'Commit message');

    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter, platform: 'macos');
    await tester.pumpAndSettle();
    expect(project.session.calls, ['answer q1 name=draft']);
  });

  testWidgets('an input question answers with its rebound key, not with ⏎', (
    tester,
  ) async {
    await show(
      tester,
      ShortcutSettings.standard().change(
        Command.answer,
        const Keys(LogicalKeyboardKey.keyY, meta: true),
      ),
    );
    await askInput(tester, 'Your name');
    await tester.enterText(find.byKey(const Key('answer')), 'Bob');
    // An unbound ⏎ reaches the text field, which reports it as its done action.
    await tester.sendKeyEvent(LogicalKeyboardKey.enter, platform: 'macos');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(project.session.calls, isEmpty);

    await command(tester, LogicalKeyboardKey.keyY);
    expect(project.session.calls, ['answer q1 name=Bob']);
  });
}
