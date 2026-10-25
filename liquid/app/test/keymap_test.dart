import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/keymap.dart';

void main() {
  const liquidN = Keys(LogicalKeyboardKey.keyN, liquid: true);
  const commandN = Keys(LogicalKeyboardKey.keyN, meta: true);
  const commandD = Keys(LogicalKeyboardKey.keyD, meta: true);

  test(
    'the Liquid key profile adds ⇪ to Liquid commands and keeps their ⌘ keys',
    () {
      final keys = ShortcutSettings.liquidKey();

      expect(keys.bindings(Command.newSession), [liquidN, commandN]);
      expect(keys.label(Command.newSession), '⇪N');
      expect(keys.label(Command.allowSession), '⇪⏎');
      expect(keys.menu(Command.newSession), commandN);
      // Platform standards and keys without ⌘ stay as they are.
      expect(keys.label(Command.settings), '⌘,');
      expect(keys.label(Command.reject), 'esc');
      expect(keys.label(Command.queue), '⌥⏎');
    },
  );

  test('the Standard profile uses ⌘ only', () {
    final keys = ShortcutSettings.standard();

    expect(keys.bindings(Command.newSession), [commandN]);
    expect(keys.label(Command.diff), '⌘3');
    expect(keys.liquidKey, isFalse);
  });

  test(
    'changing a shortcut switches to Custom based on the profile it came from',
    () {
      final keys = ShortcutSettings.standard().change(Command.diff, commandD);

      expect(keys.profile, Profile.custom);
      expect(keys.base, Profile.standard);
      expect(keys.bindings(Command.diff), [commandD]);
      expect(keys.changed(Command.diff), isTrue);
      expect(keys.reset(Command.diff).bindings(Command.diff), [
        const Keys(LogicalKeyboardKey.digit3, meta: true),
      ]);
      final all = keys.resetAll();
      expect(all.profile, Profile.standard);
      expect(all.changed(Command.diff), isFalse);
    },
  );

  test('a conflict is another command in the same area with the same keys', () {
    final keys = ShortcutSettings.liquidKey();

    expect(keys.conflict(Command.focusInput, liquidN), Command.newSession);
    // The ⌘ variant that keeps working conflicts too.
    expect(keys.conflict(Command.focusInput, commandN), Command.newSession);
    // esc stops a run in the input area and rejects in an approval.
    expect(
      keys.conflict(Command.reject, const Keys(LogicalKeyboardKey.escape)),
      isNull,
    );
    expect(keys.conflict(Command.newSession, liquidN), isNull);
  });

  test('taking keys anyway clears them from the other command', () {
    final keys = ShortcutSettings.liquidKey().change(
      Command.focusInput,
      liquidN,
      clear: Command.newSession,
    );

    expect(keys.bindings(Command.focusInput), [
      liquidN,
      const Keys(LogicalKeyboardKey.keyL, meta: true),
    ]);
    expect(keys.bindings(Command.newSession), [commandN]);
    expect(keys.label(Command.newSession), '⌘N');
  });

  test(
    'Custom can turn the Liquid key off and choosing a profile starts over',
    () {
      final custom = ShortcutSettings.liquidKey().withLiquidKey(false);

      expect(custom.profile, Profile.custom);
      expect(custom.liquidKey, isFalse);
      final standard = custom.choose(Profile.standard);
      expect(standard, ShortcutSettings.standard());
      expect(custom.choose(Profile.liquidKey), ShortcutSettings.liquidKey());
    },
  );

  test(
    'settings survive a JSON round trip and ignore what they do not know',
    () {
      final keys = ShortcutSettings.liquidKey()
          .change(
            Command.diff,
            const Keys(LogicalKeyboardKey.keyD, liquid: true),
          )
          .change(Command.focusInput, null);

      expect(ShortcutSettings.fromJson(keys.toJson()), keys);
      final json = keys.toJson()
        ..['changes'] = <String, Object?>{
          ...(keys.toJson()['changes'] as Map),
          'gone': 'meta+0x6e',
          'diff': 'not keys',
        };
      final read = ShortcutSettings.fromJson(json)!;
      expect(read.bindings(Command.diff), [
        const Keys(LogicalKeyboardKey.digit3, liquid: true),
        const Keys(LogicalKeyboardKey.digit3, meta: true),
      ]);
      expect(read.bindings(Command.focusInput), [
        const Keys(LogicalKeyboardKey.keyL, meta: true),
      ]);
      expect(ShortcutSettings.fromJson({'profile': 'unknown'}), isNull);
    },
  );
}
