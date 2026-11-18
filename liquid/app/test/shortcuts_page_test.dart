import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/settings_page.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'fake_host.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeHost host;
  late MemoryStore store;
  late ShortcutController shortcuts;
  late FakeProject project;

  Future<void> show(WidgetTester tester, ShortcutSettings? stored) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    project = FakeProject();
    host = FakeHost();
    store = MemoryStore(stored);
    shortcuts = await ShortcutController.load(store, host);
    final state = SessionState(project);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, shortcuts: shortcuts),
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

  Future<void> pressMeta(WidgetTester tester, LogicalKeyboardKey key) =>
      press(tester, key, holding: LogicalKeyboardKey.metaLeft);

  Future<void> tapText(WidgetTester tester, String text) async {
    await tester.tap(find.text(text));
    await tester.pumpAndSettle();
  }

  Future<void> openSettings(WidgetTester tester) =>
      pressMeta(tester, LogicalKeyboardKey.comma);

  testWidgets('⌘, opens Settings and esc closes it', (tester) async {
    await show(tester, ShortcutSettings.liquidKey());
    await openSettings(tester);

    expect(find.text('Shortcuts'), findsWidgets);
    expect(find.text('⇪N'), findsOneWidget);
    expect(find.text('⌘N'), findsOneWidget);
    expect(find.text('tap ⇪'), findsOneWidget);
    expect(find.text('Fold or unfold the folder'), findsOneWidget);

    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('tap ⇪'), findsNothing);
    expect(find.text('Chat'), findsOneWidget);
  });

  testWidgets('a default taken by the user’s ⌘U is unbound', (tester) async {
    await show(
      tester,
      ShortcutSettings.fromJson(
        ShortcutSettings.standard()
            .change(
              Command.focusInput,
              const Keys(LogicalKeyboardKey.keyU, meta: true),
            )
            .toJson(),
      ),
    );
    await openSettings(tester);

    expect(find.text('⌘U'), findsOneWidget, reason: 'only the user’s own ⌘U');
    expect(find.text('Reset to ⌘U'), findsOneWidget);
  });

  testWidgets('recording keys moves to Custom, offers reset', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘D');
    expect(find.text('Press keys…'), findsOneWidget);
    await pressMeta(tester, LogicalKeyboardKey.keyE);

    expect(shortcuts.value.bindings(Command.diff), [
      const Keys(LogicalKeyboardKey.keyE, meta: true),
    ]);
    expect(store.saved, shortcuts.value);
    expect(find.text('based on Standard'), findsOneWidget);
    expect(find.text('1 shortcut changed'), findsOneWidget);

    await tapText(tester, 'Reset to ⌘D');
    expect(shortcuts.value.changed(Command.diff), isFalse);

    await tapText(tester, '⌘D');
    await press(tester, LogicalKeyboardKey.escape);
    expect(find.text('Press keys…'), findsNothing, reason: 'esc cancels');
    expect(find.text('⌘D'), findsOneWidget, reason: 'and keeps Settings open');
  });

  testWidgets('keys another action in the area ask first', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘L');
    await pressMeta(tester, LogicalKeyboardKey.keyN);
    expect(find.textContaining('already runs'), findsOneWidget);
    expect(shortcuts.value.changed(Command.focusInput), isFalse);

    await tapText(tester, 'Use anyway, clear New Session');
    expect(shortcuts.value.bindings(Command.focusInput), [
      const Keys(LogicalKeyboardKey.keyN, meta: true),
    ]);
    expect(shortcuts.value.bindings(Command.newSession), isEmpty);

    await tester.ensureVisible(find.text('Reset all to defaults'));
    await tapText(tester, 'Reset all to defaults');
    expect(shortcuts.value, ShortcutSettings.standard());
  });

  testWidgets('Reset names every command it takes keys from', (tester) async {
    final settings = ShortcutSettings.liquidKey()
        .change(
          Command.diff,
          const Keys(LogicalKeyboardKey.keyN, liquid: true),
          clear: [Command.newSession],
        )
        .change(
          Command.focusInput,
          const Keys(LogicalKeyboardKey.keyN, meta: true),
          clear: [Command.newSession],
        );
    await show(tester, settings);
    await openSettings(tester);
    await tapText(tester, 'Reset to ⇪N');
    expect(find.textContaining('already runs Diff'), findsOneWidget);
    expect(find.textContaining('already runs Focus Input'), findsOneWidget);

    await tapText(tester, 'Use anyway, clear Diff and Focus Input');
    expect(
      shortcuts.value.conflicts(
        Command.newSession,
        const Keys(LogicalKeyboardKey.keyN, liquid: true),
      ),
      isEmpty,
    );
    expect(
      shortcuts.value.bindings(Command.focusInput),
      isNot(contains(const Keys(LogicalKeyboardKey.keyN, meta: true))),
    );
  });

  testWidgets(
    'recording keys a Liquid key combination runs asks while the key is off',
    (tester) async {
      final settings = ShortcutSettings.standard().change(
        Command.newSession,
        const Keys(LogicalKeyboardKey.keyE, meta: true, liquid: true),
      );
      await show(tester, settings);
      await openSettings(tester);
      await tapText(tester, '⌘U');
      await pressMeta(tester, LogicalKeyboardKey.keyE);
      expect(find.textContaining('already runs New Session'), findsOneWidget);
      expect(find.text('Use anyway, clear New Session'), findsOneWidget);
      expect(shortcuts.value.changed(Command.statistics), isFalse);
    },
  );

  testWidgets('changing the reveal key hides the hints', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await tester.sendKeyDownEvent(
      LogicalKeyboardKey.metaLeft,
      platform: 'macos',
    );
    await tester.pump(const Duration(milliseconds: 700));
    expect(find.byType(KeyBadge), findsWidgets);

    await shortcuts.change(ShortcutSettings.liquidKey());
    await tester.pumpAndSettle();
    expect(
      find.byType(KeyBadge),
      findsNothing,
      reason:
          'the hints shown for ⌘ do not stay once the Liquid key reveals them',
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
  });

  testWidgets(
    'resetting a command whose keys another command took asks first',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());
      await openSettings(tester);
      await tapText(tester, '⇪D');
      await press(tester, LogicalKeyboardKey.keyN, holding: liquidKey);
      await tapText(tester, 'Use anyway, clear New Session');

      await tapText(tester, 'Reset to ⇪N');
      expect(find.textContaining('already runs Diff'), findsOneWidget);
      expect(shortcuts.value.bindings(Command.newSession), [
        const Keys(LogicalKeyboardKey.keyN, meta: true),
      ], reason: 'nothing changes before the user agrees');

      await tapText(tester, 'Use anyway, clear Diff');
      expect(shortcuts.value.bindings(Command.newSession), [
        const Keys(LogicalKeyboardKey.keyN, liquid: true),
        const Keys(LogicalKeyboardKey.keyN, meta: true),
      ]);
      expect(shortcuts.value.bindings(Command.diff), [
        const Keys(LogicalKeyboardKey.keyD, meta: true),
      ]);
    },
  );

  testWidgets('a profile chosen during a conflict cancels it', (tester) async {
    await show(
      tester,
      ShortcutSettings.standard()
          .change(
            Command.focusInput,
            const Keys(LogicalKeyboardKey.keyN, meta: true),
            clear: [Command.newSession],
          )
          .choose(Profile.standard),
    );
    await openSettings(tester);
    await tapText(tester, '⌘D');
    await pressMeta(tester, LogicalKeyboardKey.keyN);
    expect(find.text('Use anyway, clear New Session'), findsOneWidget);

    await tapText(tester, 'Custom');
    expect(
      find.text('Use anyway, clear New Session'),
      findsNothing,
      reason: 'in Custom, Focus Input owns ⌘N, so the old owner is stale',
    );
    expect(shortcuts.value.bindings(Command.diff), [
      const Keys(LogicalKeyboardKey.keyD, meta: true),
    ]);
    expect(shortcuts.value.bindings(Command.focusInput), [
      const Keys(LogicalKeyboardKey.keyN, meta: true),
    ]);
  });

  testWidgets('recording the current keys changes nothing', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘D');
    await pressMeta(tester, LogicalKeyboardKey.keyD);
    expect(find.text('Press keys…'), findsNothing, reason: 'recording ends');
    expect(shortcuts.value, ShortcutSettings.standard(), reason: 'no Custom');
  });

  testWidgets('keys that macOS or typing need are refused', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘N');
    await pressMeta(tester, LogicalKeyboardKey.keyQ);
    expect(find.text('⌘Q is kept for Quit.'), findsOneWidget);
    expect(find.textContaining('Use anyway'), findsNothing);
    await press(tester, LogicalKeyboardKey.keyN);
    expect(find.text('N is for typing here.'), findsOneWidget);
    expect(shortcuts.value.changed(Command.newSession), isFalse);
  });

  testWidgets('the Liquid key with Shift is refused by macOS', (tester) async {
    await show(tester, ShortcutSettings.liquidKey());
    await openSettings(tester);
    await tapText(tester, '⇪N');
    await tester.sendKeyDownEvent(liquidKey, platform: 'macos');
    await press(
      tester,
      LogicalKeyboardKey.keyM,
      holding: LogicalKeyboardKey.shiftLeft,
    );
    await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
    await tester.pumpAndSettle();
    expect(find.text('⇧⇪M is kept for capitals.'), findsOneWidget);
    expect(shortcuts.value.changed(Command.newSession), isFalse);
  });

  testWidgets('leaving the recording row stops recording', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘D');
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

    await tapText(tester, 'Standard');
    expect(store.saved, ShortcutSettings.standard());
    expect(host.calls, [true, false]);
    expect(find.text('tap ⇪'), findsNothing);

    await tapText(tester, 'Custom');
    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();
    expect(shortcuts.value.liquidKey, isTrue);
    expect(host.calls, [true, false, true]);
  });

  testWidgets('says why the Liquid key could not be turned on', (tester) async {
    await show(tester, ShortcutSettings.standard());
    host.failure = PlatformException(
      code: 'approval',
      message: 'macOS needs your approval in System Settings → General → Login Items.',
    );
    await openSettings(tester);
    await tapText(tester, 'Liquid key');
    final reason = find.textContaining(
      'Could not turn the Liquid key on: macOS needs your approval',
    );
    expect(shortcuts.value.profile, Profile.standard);
    expect(
      find.descendant(of: find.byType(SettingsPage), matching: reason),
      findsOneWidget,
    );
    expect(
      find.byTooltip('Dismiss'),
      findsNothing,
      reason: 'the Liquid key keeps its own notice, not the save notice',
    );

    host.failure = null;
    await tapText(tester, 'Custom');
    expect(
      find.descendant(of: find.byType(SettingsPage), matching: reason),
      findsNothing,
      reason: 'choosing a profile again clears the reason',
    );
  });

  testWidgets('search narrows the list to matching actions and keys', (
    tester,
  ) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tester.enterText(find.byType(TextField), 'whitespace');
    await tester.pumpAndSettle();
    expect(find.text('Hide or show whitespace changes'), findsOneWidget);
    expect(find.text('New Session'), findsNothing);

    await tester.enterText(find.byType(TextField), '⌘D');
    await tester.pumpAndSettle();
    expect(find.text('Diff'), findsWidgets);
    expect(find.text('New Session'), findsNothing);
  });

  testWidgets('lists every key that never changes', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);
    for (final fixed in Fixed.values) {
      await tester.enterText(find.byType(TextField), fixed.label);
      await tester.pumpAndSettle();
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
    await tapText(tester, 'Use Liquid key');
    expect(find.text('How do you want to use the keyboard?'), findsNothing);
    expect(store.saved, ShortcutSettings.liquidKey());
    expect(host.calls, [true]);
  });

  testWidgets('a question at the first start leaves it open', (tester) async {
    await show(tester, null);
    const asks = 'How do you want to use the keyboard?';
    project.session.emit(
      event(
        BridgeEventKind.requestOpened,
        901,
        request: questionsOf('q1', [questionOf('q1', 'Sure?')]),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text(asks), findsNothing, reason: 'the question goes first');
    expect(store.saved, isNull, reason: 'nobody chose a profile');

    project.session.emit(
      event(BridgeEventKind.requestResolved, 900, requestId: 'q1'),
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
      const Keys(LogicalKeyboardKey.keyE, liquid: true),
    );
    const commandE = Keys(LogicalKeyboardKey.keyE, meta: true);

    Future<void> recordDiff(WidgetTester tester) async {
      await openSettings(tester);
      await tapText(tester, '⌘D');
      await pressMeta(tester, LogicalKeyboardKey.keyE);
    }

    testWidgets('asks first, and ⏎ or esc keep everything', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      for (final key in [LogicalKeyboardKey.enter, LogicalKeyboardKey.escape]) {
        await recordDiff(tester);
        expect(find.text('Replace the Custom profile?'), findsOneWidget);
        expect(
          find.text(
            'Changing Diff to ⌘E starts a new Custom profile from Standard. '
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

      await tapText(tester, '⌘D');
      await pressMeta(tester, LogicalKeyboardKey.keyE);
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
            await tapText(tester, 'Replace Custom');
          }
          expect(shortcuts.value.profile, Profile.custom);
          expect(shortcuts.value.base, Profile.standard);
          expect(shortcuts.value.bindings(Command.diff), [commandE]);
          expect(shortcuts.value.changes.length, 1);
        },
      );
    }

    testWidgets('a profile chosen while it asks ends it', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      await recordDiff(tester);
      expect(find.text('Replace the Custom profile?'), findsOneWidget);
      await tapText(tester, 'Custom');
      expect(tester.takeException(), isNull);
      expect(find.byType(ErrorWidget), findsNothing);
      expect(find.text('Replace the Custom profile?'), findsNothing);
      expect(find.text('Press keys…'), findsNothing, reason: 'recording ends');
      expect(shortcuts.value, kept, reason: 'the kept Custom profile is back');
    });

    testWidgets('another profile than asked under ends it', (tester) async {
      await show(tester, kept.choose(Profile.liquidKey));
      await openSettings(tester);
      await tapText(tester, '⇪D');
      await press(tester, LogicalKeyboardKey.keyE, holding: liquidKey);
      expect(find.text('Replace the Custom profile?'), findsOneWidget);

      await tapText(tester, 'Standard');
      expect(find.text('Replace the Custom profile?'), findsNothing);
      expect(find.text('Replace Custom'), findsNothing);
      expect(find.text('Press keys…'), findsNothing, reason: 'recording ends');
      expect(shortcuts.value.profile, Profile.standard);
      expect(shortcuts.value.bindings(Command.diff), [
        const Keys(LogicalKeyboardKey.keyD, meta: true),
      ], reason: 'nothing is replaced, so no command loses its keys');
    });

    testWidgets('only the question shows, not an old refusal', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      await openSettings(tester);
      await tapText(tester, '⌘D');
      await pressMeta(tester, LogicalKeyboardKey.keyQ);
      expect(find.text('⌘Q is kept for Quit.'), findsOneWidget);
      await pressMeta(tester, LogicalKeyboardKey.keyE);
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
      expect(shortcuts.value.bindings(Command.diff), [commandE]);
    });
  });
}
