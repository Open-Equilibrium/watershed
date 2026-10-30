import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/shortcuts/shortcuts_page.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'fake_host.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeHost host;
  late MemoryStore store;
  late ShortcutController shortcuts;
  late FakeBackend backend;

  Future<void> show(WidgetTester tester, ShortcutSettings? stored) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    backend = FakeBackend();
    host = FakeHost();
    store = MemoryStore(stored);
    shortcuts = await ShortcutController.load(store, host);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: SessionState(backend),
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
    expect(find.text('Fold or unfold the folder'), findsOneWidget);
    expect(find.text('Collapse or expand the turn'), findsOneWidget);
    expect(find.text('Collapse all turns'), findsOneWidget);

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

  testWidgets('recording the keys a command already has changes nothing', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.tap(find.text('⌘3'));
    await tester.pumpAndSettle();
    await press(
      tester,
      LogicalKeyboardKey.digit3,
      holding: LogicalKeyboardKey.metaLeft,
    );
    expect(find.text('Press keys…'), findsNothing, reason: 'recording ends');
    expect(shortcuts.value, ShortcutSettings.standard(), reason: 'no Custom');
  });

  testWidgets('keys that macOS or typing need are refused', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.tap(find.text('⌘N'));
    await tester.pumpAndSettle();
    await press(
      tester,
      LogicalKeyboardKey.keyQ,
      holding: LogicalKeyboardKey.metaLeft,
    );
    expect(find.text('⌘Q is kept for Quit.'), findsOneWidget);
    expect(find.textContaining('Use anyway'), findsNothing);
    await press(tester, LogicalKeyboardKey.keyN);
    expect(find.text('N is for typing here.'), findsOneWidget);
    expect(shortcuts.value.changed(Command.newSession), isFalse);
  });

  testWidgets('leaving the recording row stops recording', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.tap(find.text('⌘3'));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(TextField));
    await tester.pumpAndSettle();
    expect(find.text('Press keys…'), findsNothing);
    await press(tester, LogicalKeyboardKey.keyX);
    expect(shortcuts.value.changed(Command.diff), isFalse);
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

  testWidgets(
    'says inside Settings why the Liquid key could not be turned on',
    (tester) async {
      await show(tester, ShortcutSettings.standard());
      host.failure = PlatformException(
        code: 'approval',
        message: 'macOS needs your approval in System Settings → General → Login Items.',
      );
      await openSettings(tester);
      await tester.tap(find.text('Liquid key'));
      await tester.pumpAndSettle();
      final reason = find.textContaining(
        'Could not turn the Liquid key on: macOS needs your approval',
      );
      expect(shortcuts.value.profile, Profile.standard);
      expect(
        find.descendant(of: find.byType(ShortcutsPage), matching: reason),
        findsOneWidget,
      );

      host.failure = null;
      await tester.tap(find.text('Custom'));
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: find.byType(ShortcutsPage), matching: reason),
        findsNothing,
        reason: 'choosing a profile again clears the reason',
      );
    },
  );

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

  testWidgets('lists every key that never changes', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);
    for (final fixed in Fixed.values) {
      await tester.enterText(find.byType(TextField), fixed.label);
      await tester.pumpAndSettle();
      // A row of the list, not the search field that holds the same words.
      expect(
        find.byWidgetPredicate(
          (widget) => widget is Text && widget.data == fixed.label,
        ),
        findsOneWidget,
        reason: fixed.name,
      );
    }
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

  testWidgets('a question during the first start leaves the choice open', (
    tester,
  ) async {
    await show(tester, null);
    const asks = 'How do you want to use the keyboard?';
    backend.controller.add(
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.confirm,
            title: 'Sure?',
            detail: '',
            options: const [],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text(asks), findsNothing, reason: 'the question goes first');
    expect(store.saved, isNull, reason: 'nobody chose a profile');

    backend.controller.add(
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q1'),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text(asks), findsOneWidget);
  });

  testWidgets('esc at the first start keeps Standard', (tester) async {
    await show(tester, null);

    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('How do you want to use the keyboard?'), findsNothing);
    expect(store.saved, ShortcutSettings.standard());
    expect(host.calls, isEmpty);
  });

  group('a change that would replace a kept Custom profile', () {
    final kept = ShortcutSettings.liquidKey().change(
      Command.diff,
      const Keys(LogicalKeyboardKey.keyD, liquid: true),
    );
    const commandD = Keys(LogicalKeyboardKey.keyD, meta: true);

    Future<void> recordDiff(WidgetTester tester) async {
      await openSettings(tester);
      await tester.tap(find.text('⌘3'));
      await tester.pumpAndSettle();
      await press(
        tester,
        LogicalKeyboardKey.keyD,
        holding: LogicalKeyboardKey.metaLeft,
      );
    }

    testWidgets('asks first, and ⏎ or esc keep everything', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      for (final key in [LogicalKeyboardKey.enter, LogicalKeyboardKey.escape]) {
        await recordDiff(tester);
        expect(find.text('Replace the Custom profile?'), findsOneWidget);
        expect(
          find.text(
            'Changing Diff to ⌘D starts a new Custom profile from Standard. '
            'This replaces your current Custom profile, which has 1 changed '
            'shortcut.',
          ),
          findsOneWidget,
        );
        expect(shortcuts.value.profile, Profile.standard, reason: 'not yet');

        await press(tester, key);
        expect(find.text('Replace the Custom profile?'), findsNothing);
        expect(shortcuts.value, kept.choose(Profile.standard), reason: '$key');
        expect(find.text('Shortcuts'), findsWidgets, reason: 'Settings stay');
        if (key == LogicalKeyboardKey.escape) continue;
        await press(tester, LogicalKeyboardKey.escape);
      }

      // Settings are still open after esc.
      await tester.tap(find.text('⌘3'));
      await tester.pumpAndSettle();
      await press(
        tester,
        LogicalKeyboardKey.keyD,
        holding: LogicalKeyboardKey.metaLeft,
      );
      expect(find.text('Replace the Custom profile?'), findsOneWidget);
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
      expect(
        find.text('Replace the Custom profile?'),
        findsNothing,
        reason: 'leaving the question keeps Custom too',
      );
      expect(shortcuts.value, kept.choose(Profile.standard));
    });

    for (final byKeyboard in [false, true]) {
      testWidgets(
        'replaces it ${byKeyboard ? 'with Tab and ⏎' : 'on a click'}',
        (tester) async {
          await show(tester, kept.choose(Profile.standard));
          await recordDiff(tester);
          if (byKeyboard) {
            await press(tester, LogicalKeyboardKey.tab);
            await press(tester, LogicalKeyboardKey.enter);
          } else {
            await tester.tap(find.text('Replace Custom'));
            await tester.pumpAndSettle();
          }
          expect(shortcuts.value.profile, Profile.custom);
          expect(shortcuts.value.base, Profile.standard);
          expect(shortcuts.value.bindings(Command.diff), [commandD]);
          expect(shortcuts.value.changes.length, 1);
        },
      );
    }

    testWidgets('a profile chosen while it asks ends the question', (
      tester,
    ) async {
      await show(tester, kept.choose(Profile.standard));
      await recordDiff(tester);
      expect(find.text('Replace the Custom profile?'), findsOneWidget);
      await tester.tap(find.text('Custom'));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      expect(find.byType(ErrorWidget), findsNothing);
      expect(find.text('Replace the Custom profile?'), findsNothing);
      expect(find.text('Press keys…'), findsNothing, reason: 'recording ends');
      expect(shortcuts.value, kept, reason: 'the kept Custom profile is back');
    });

    testWidgets('only the question shows, not an earlier refusal', (
      tester,
    ) async {
      await show(tester, kept.choose(Profile.standard));
      await openSettings(tester);
      await tester.tap(find.text('⌘3'));
      await tester.pumpAndSettle();
      await press(
        tester,
        LogicalKeyboardKey.keyQ,
        holding: LogicalKeyboardKey.metaLeft,
      );
      expect(find.text('⌘Q is kept for Quit.'), findsOneWidget);
      await press(
        tester,
        LogicalKeyboardKey.keyD,
        holding: LogicalKeyboardKey.metaLeft,
      );
      expect(find.text('Replace the Custom profile?'), findsOneWidget);
      expect(find.text('⌘Q is kept for Quit.'), findsNothing);
    });

    testWidgets('does not ask when nothing would be lost', (tester) async {
      await show(
        tester,
        ShortcutSettings.liquidKey()
            .choose(Profile.custom)
            .choose(Profile.standard),
      );
      await recordDiff(tester);
      expect(find.text('Replace the Custom profile?'), findsNothing);
      expect(shortcuts.value.bindings(Command.diff), [commandD]);
    });
  });
}
