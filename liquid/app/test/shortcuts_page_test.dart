import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/settings_page.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';
import 'fake_host.dart';

const info = SessionInfo(project: 'acme-api');
const metaD = Keys(LogicalKeyboardKey.keyD, meta: true);
const metaE = Keys(LogicalKeyboardKey.keyE, meta: true);
const metaN = Keys(LogicalKeyboardKey.keyN, meta: true);
const replaceAsk = 'Replace the Custom profile?';
const clearNew = 'Use anyway, clear New Session';

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
    await pumpApp(
      tester,
      AgentShell(state: state, info: info, shortcuts: shortcuts),
    );
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

  Future<void> record(
    WidgetTester tester,
    String label,
    LogicalKeyboardKey key,
  ) async {
    await tapText(tester, label);
    await pressMeta(tester, key);
  }

  Future<void> recordDiff(WidgetTester tester) async {
    await openSettings(tester);
    await record(tester, '⌘D', LogicalKeyboardKey.keyE);
  }

  final recording = find.text('Press keys…');

  testWidgets('recording keys moves to Custom, offers reset', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await tapText(tester, '⌘D');
    expect(recording, findsOneWidget);
    await pressMeta(tester, LogicalKeyboardKey.keyE);

    expect(shortcuts.value.bindings(Command.diff), [metaE]);
    expect(store.saved, shortcuts.value);
    expect(find.text('based on Standard'), findsOneWidget);
    expect(find.text('1 shortcut changed'), findsOneWidget);

    await tapText(tester, 'Reset to ⌘D');
    expect(shortcuts.value.changed(Command.diff), isFalse);

    await tapText(tester, '⌘D');
    await press(tester, LogicalKeyboardKey.escape);
    expect(recording, findsNothing, reason: 'esc cancels');
    expect(find.text('⌘D'), findsOneWidget, reason: 'and keeps Settings open');
  });

  testWidgets('keys another action in the area ask first', (tester) async {
    await show(tester, ShortcutSettings.standard());
    await openSettings(tester);

    await record(tester, '⌘L', LogicalKeyboardKey.keyN);
    expect(find.textContaining('already runs'), findsOneWidget);
    expect(shortcuts.value.changed(Command.focusInput), isFalse);

    await tapText(tester, clearNew);
    expect(shortcuts.value.bindings(Command.focusInput), [metaN]);
    expect(shortcuts.value.bindings(Command.newSession), isEmpty);

    await tester.ensureVisible(find.text('Reset all to defaults'));
    await tapText(tester, 'Reset all to defaults');
    expect(shortcuts.value, ShortcutSettings.standard());
  });

  testWidgets(
    'resetting a command whose keys another command took asks first',
    (tester) async {
      await show(tester, ShortcutSettings.liquidKey());
      await openSettings(tester);
      await tapText(tester, '⇪D');
      await press(tester, LogicalKeyboardKey.keyN, holding: liquidKey);
      await tapText(tester, clearNew);

      await tapText(tester, 'Reset to ⇪N');
      expect(find.textContaining('already runs Diff'), findsOneWidget);
      expect(shortcuts.value.bindings(Command.newSession), [
        metaN,
      ], reason: 'nothing changes before the user agrees');

      await tapText(tester, 'Use anyway, clear Diff');
      expect(shortcuts.value.bindings(Command.newSession), [
        const Keys(LogicalKeyboardKey.keyN, liquid: true),
        metaN,
      ]);
      expect(shortcuts.value.bindings(Command.diff), [metaD]);
    },
  );

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

    testWidgets('asks first, and ⏎ or esc keep everything', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      for (final key in [LogicalKeyboardKey.enter, LogicalKeyboardKey.escape]) {
        await recordDiff(tester);
        expect(find.text(replaceAsk), findsOneWidget);
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
        expect(find.text(replaceAsk), findsNothing);
        expect(shortcuts.value, kept.choose(Profile.standard), reason: '$key');
        expect(find.text('Shortcuts'), findsWidgets, reason: 'Settings stay');
        if (key == LogicalKeyboardKey.enter) {
          await press(tester, LogicalKeyboardKey.escape);
        }
      }

      await record(tester, '⌘D', LogicalKeyboardKey.keyE);
      expect(find.text(replaceAsk), findsOneWidget);
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
      expect(
        find.text(replaceAsk),
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
          expect(shortcuts.value.bindings(Command.diff), [metaE]);
          expect(shortcuts.value.changes.length, 1);
        },
      );
    }

    testWidgets('a profile chosen while it asks ends it', (tester) async {
      await show(tester, kept.choose(Profile.standard));
      await recordDiff(tester);
      expect(find.text(replaceAsk), findsOneWidget);
      await tapText(tester, 'Custom');
      expect(tester.takeException(), isNull);
      expect(find.byType(ErrorWidget), findsNothing);
      expect(find.text(replaceAsk), findsNothing);
      expect(recording, findsNothing, reason: 'recording ends');
      expect(shortcuts.value, kept, reason: 'the kept Custom profile is back');
    });

    testWidgets('does not ask when nothing would be lost', (tester) async {
      await show(
        tester,
        ShortcutSettings.liquidKey()
            .choose(Profile.custom)
            .choose(Profile.standard),
      );
      await recordDiff(tester);
      expect(find.text(replaceAsk), findsNothing);
      expect(shortcuts.value.bindings(Command.diff), [metaE]);
    });
  });
}
