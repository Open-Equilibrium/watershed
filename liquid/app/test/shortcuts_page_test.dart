import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'fake_host.dart';

const info = SessionInfo(
  title: 'Rate limit for /login',
  project: 'acme-api',
  harness: 'pi',
  device: 'MacBook',
);

void main() {
  late FakeHost host;
  late MemoryStore store;
  late ShortcutController shortcuts;

  Future<void> show(WidgetTester tester, ShortcutSettings? stored) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    host = FakeHost();
    store = MemoryStore(stored);
    shortcuts = await ShortcutController.load(store, host);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: SessionState(FakeBackend()),
          info: info,
          shortcuts: shortcuts,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> press(
    WidgetTester tester,
    LogicalKeyboardKey key, {
    LogicalKeyboardKey? holding,
  }) async {
    if (holding != null) {
      await tester.sendKeyDownEvent(holding, platform: 'macos');
    }
    await tester.sendKeyEvent(key, platform: 'macos');
    if (holding != null) {
      await tester.sendKeyUpEvent(holding, platform: 'macos');
    }
    await tester.pumpAndSettle();
  }

  Future<void> openSettings(WidgetTester tester) => press(
    tester,
    LogicalKeyboardKey.comma,
    holding: LogicalKeyboardKey.metaLeft,
  );

  testWidgets('⌘, opens Settings over the main area and esc closes it', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.liquidKey());
    await openSettings(tester);

    expect(find.text('Shortcuts'), findsWidgets);
    expect(find.text('⇪N'), findsOneWidget);
    expect(find.text('⌘N'), findsOneWidget);
    expect(find.text('tap ⇪'), findsOneWidget);
    expect(find.text('Trajectory'), findsWidgets);

    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('tap ⇪'), findsNothing);
    expect(find.text('Chat'), findsOneWidget);
  });

  testWidgets('recording new keys switches to Custom and offers a reset', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.tap(find.text('⌘3'));
    await tester.pumpAndSettle();
    expect(find.text('Press keys…'), findsOneWidget);
    await press(
      tester,
      LogicalKeyboardKey.keyD,
      holding: LogicalKeyboardKey.metaLeft,
    );

    expect(shortcuts.value.bindings(Command.diff), [
      const Keys(LogicalKeyboardKey.keyD, meta: true),
    ]);
    expect(store.saved, shortcuts.value);
    expect(find.text('based on Standard'), findsOneWidget);
    expect(find.text('1 shortcut changed'), findsOneWidget);

    await tester.tap(find.text('Reset to ⌘3'));
    await tester.pumpAndSettle();
    expect(shortcuts.value.changed(Command.diff), isFalse);

    await tester.tap(find.text('⌘3'));
    await tester.pumpAndSettle();
    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('Press keys…'), findsNothing, reason: 'esc cancels');
    expect(find.text('⌘3'), findsOneWidget, reason: 'and keeps Settings open');
  });

  testWidgets('keys another action in the area runs ask before they move', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.tap(find.text('⌘L'));
    await tester.pumpAndSettle();
    await press(
      tester,
      LogicalKeyboardKey.keyN,
      holding: LogicalKeyboardKey.metaLeft,
    );
    expect(find.textContaining('already runs'), findsOneWidget);
    expect(shortcuts.value.changed(Command.focusInput), isFalse);

    await tester.tap(find.text('Use anyway, clear New Session'));
    await tester.pumpAndSettle();
    expect(shortcuts.value.bindings(Command.focusInput), [
      const Keys(LogicalKeyboardKey.keyN, meta: true),
    ]);
    expect(shortcuts.value.bindings(Command.newSession), isEmpty);

    await tester.tap(find.text('Reset all to defaults'));
    await tester.pumpAndSettle();
    expect(shortcuts.value, ShortcutSettings.standard());
  });

  testWidgets('choosing a profile applies it at once', (tester) async {
    await show(tester, ShortcutSettings.liquidKey());
    await openSettings(tester);
    expect(host.calls, [true], reason: 'a stored Liquid key is turned on');

    await tester.tap(find.text('Standard'));
    await tester.pumpAndSettle();
    expect(store.saved, ShortcutSettings.standard());
    expect(host.calls, [true, false]);
    expect(find.text('tap ⇪'), findsNothing);

    await tester.tap(find.text('Custom'));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();
    expect(shortcuts.value.liquidKey, isTrue);
    expect(host.calls, [true, false, true]);
  });

  testWidgets('search narrows the list to matching actions and keys', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.enterText(find.byType(TextField), 'branch');
    await tester.pumpAndSettle();
    expect(find.text('Branch from the step'), findsOneWidget);
    expect(find.text('New Session'), findsNothing);

    await tester.enterText(find.byType(TextField), '⌘2');
    await tester.pumpAndSettle();
    expect(find.text('Trajectory'), findsWidgets);
    expect(find.text('Diff'), findsNothing);
  });

  testWidgets('the first start asks for a profile', (tester) async {
    await show(tester, null);

    expect(find.text('How do you want to use the keyboard?'), findsOneWidget);
    await tester.tap(find.text('Use Liquid key'));
    await tester.pumpAndSettle();
    expect(find.text('How do you want to use the keyboard?'), findsNothing);
    expect(store.saved, ShortcutSettings.liquidKey());
    expect(host.calls, [true]);
  });

  testWidgets('esc at the first start keeps Standard', (tester) async {
    await show(tester, null);

    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('How do you want to use the keyboard?'), findsNothing);
    expect(store.saved, ShortcutSettings.standard());
    expect(host.calls, isEmpty);
  });
}
