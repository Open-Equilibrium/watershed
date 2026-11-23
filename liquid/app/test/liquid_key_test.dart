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
const macos = 'macos';

void main() {
  late FakeProject project;

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

  Future<bool> chord(
    WidgetTester tester,
    LogicalKeyboardKey modifier,
    LogicalKeyboardKey key,
  ) async {
    await tester.sendKeyDownEvent(modifier, platform: macos);
    final handled = await tester.sendKeyEvent(key, platform: macos);
    await tester.sendKeyUpEvent(modifier, platform: macos);
    await tester.pumpAndSettle();
    return handled;
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key, platform: macos);
    await tester.pumpAndSettle();
  }

  Future<void> askInput(WidgetTester tester, String text) async {
    project.session.emit(
      event(
        BridgeEventKind.requestOpened,
        1,
        request: questionsOf('q1', [questionOf('name', text)]),
      ),
    );
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 500));
  }

  testWidgets(
    'the Liquid key runs a Liquid command, and its ⌘ key runs it too',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());

      expect(await chord(tester, liquidKey, LogicalKeyboardKey.keyD), isTrue);
      expect(find.byType(DiffPanel), findsOneWidget);
      await chord(tester, LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.keyD);
      expect(find.byType(DiffPanel), findsNothing);
    },
  );

  testWidgets(
    'holding the Liquid key shows its shortcuts and ⌘ no longer does',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());

      await tester.sendKeyDownEvent(
        LogicalKeyboardKey.metaLeft,
        platform: macos,
      );
      await tester.pump(const Duration(milliseconds: 700));
      expect(find.text('⇪1'), findsNothing);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: macos);

      await tester.sendKeyDownEvent(liquidKey, platform: macos);
      await tester.pump(const Duration(milliseconds: 700));
      expect(find.text('⇪1'), findsOneWidget);
      await tester.sendKeyUpEvent(liquidKey, platform: macos);
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
    await chord(tester, liquidKey, LogicalKeyboardKey.keyD);
    expect(find.byType(DiffPanel), findsNothing);
    await chord(tester, liquidKey, LogicalKeyboardKey.keyE);
    expect(find.byType(DiffPanel), findsOneWidget);
  });

  testWidgets('the Standard profile leaves the Liquid key alone', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());

    expect(await chord(tester, liquidKey, LogicalKeyboardKey.keyD), isFalse);
    expect(find.byType(DiffPanel), findsNothing);
  });

  testWidgets('⏎ answers a question with its typed text, once', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await askInput(tester, 'Commit message');

    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    await press(tester, LogicalKeyboardKey.enter);
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
    await press(tester, LogicalKeyboardKey.enter);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(project.session.calls, isEmpty);

    await chord(tester, LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.keyY);
    expect(project.session.calls, ['answer q1 name=Bob']);
  });
}
