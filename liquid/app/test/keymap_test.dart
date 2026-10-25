import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/keymap.dart';

void main() {
  const liquidN = Keys(LogicalKeyboardKey.keyN, liquid: true);
  const commandN = Keys(LogicalKeyboardKey.keyN, meta: true);
  const commandD = Keys(LogicalKeyboardKey.keyD, meta: true);
  const liquidD = Keys(LogicalKeyboardKey.keyD, liquid: true);

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

  test('the Custom switch decides whether Liquid key combinations exist', () {
    final off = ShortcutSettings.liquidKey().withLiquidKey(false);
    expect(off.profile, Profile.custom);
    expect(off.base, Profile.liquidKey);
    expect(off.bindings(Command.newSession), [commandN]);
    expect(off.label(Command.allowSession), '⌘⏎');

    final on = ShortcutSettings.standard().withLiquidKey(true);
    expect(on.base, Profile.standard);
    expect(on.bindings(Command.newSession), [liquidN, commandN]);
  });

  test('Custom is kept while another profile is active', () {
    final custom = ShortcutSettings.liquidKey().change(Command.diff, liquidD);

    final standard = custom.choose(Profile.standard);
    expect(standard.liquidKey, isFalse);
    expect(standard.bindings(Command.diff), [
      const Keys(LogicalKeyboardKey.digit3, meta: true),
    ]);
    expect(standard.choose(Profile.custom), custom);
    expect(
      ShortcutSettings.liquidKey().choose(Profile.custom).base,
      Profile.liquidKey,
      reason: 'a first Custom profile starts from the active one',
    );
  });

  test('resetting all never turns the Liquid key on or off', () {
    final changed = ShortcutSettings.liquidKey().change(Command.diff, liquidD);
    expect(changed.resetAll(), ShortcutSettings.liquidKey());

    final off = changed.withLiquidKey(false).resetAll();
    expect(off.profile, Profile.custom);
    expect(off.liquidKey, isFalse);
    expect(off.changed(Command.diff), isFalse);
  });

  test('keys macOS, a fixed row or typing use are reserved', () {
    expect(
      reserved(
        Command.newSession,
        const Keys(LogicalKeyboardKey.keyQ, meta: true),
      ),
      'Quit',
    );
    expect(
      reserved(Command.branch, const Keys(LogicalKeyboardKey.arrowDown)),
      'Previous or next step',
    );
    expect(
      reserved(
        Command.queue,
        const Keys(LogicalKeyboardKey.enter, shift: true),
      ),
      'New line',
    );
    expect(
      reserved(Command.newSession, const Keys(LogicalKeyboardKey.keyN)),
      'typing',
    );
    expect(
      reserved(Command.branch, const Keys(LogicalKeyboardKey.keyX)),
      isNull,
      reason: 'the Trajectory has no text field',
    );
    expect(reserved(Command.newSession, liquidN), isNull);
    expect(
      reserved(
        Command.newSession,
        const Keys(LogicalKeyboardKey.keyQ, meta: true, liquid: true),
      ),
      'Quit',
      reason: 'the menu ignores the Liquid key',
    );
    expect(
      reserved(
        Command.queue,
        const Keys(LogicalKeyboardKey.arrowLeft, meta: true),
      ),
      'typing',
      reason: '⌘← moves the cursor',
    );
    expect(
      reserved(
        Command.queue,
        const Keys(LogicalKeyboardKey.keyA, control: true),
      ),
      'typing',
      reason: '⌃A moves the cursor',
    );
  });

  test(
    'settings survive a JSON round trip and ignore what they do not know',
    () {
      final keys = ShortcutSettings.liquidKey()
          .change(Command.diff, liquidD)
          .change(Command.focusInput, null);

      expect(ShortcutSettings.fromJson(keys.toJson()), keys);
      final json = keys.toJson();
      final custom = {...json['custom']! as Map<String, Object?>};
      custom['changes'] = <String, Object?>{
        ...custom['changes']! as Map<String, Object?>,
        'gone': ['meta+0x6e'],
        'diff': 'not keys',
        'chat': ['meta+0x31', 'meta+0x31'],
      };
      final read = ShortcutSettings.fromJson({...json, 'custom': custom})!;
      expect(read.bindings(Command.diff), [
        const Keys(LogicalKeyboardKey.digit3, liquid: true),
        const Keys(LogicalKeyboardKey.digit3, meta: true),
      ]);
      expect(read.bindings(Command.focusInput), [
        const Keys(LogicalKeyboardKey.keyL, meta: true),
      ]);
      expect(read.bindings(Command.chat), [
        const Keys(LogicalKeyboardKey.digit1, meta: true),
      ], reason: 'keys listed twice would run the command twice');
      expect(ShortcutSettings.fromJson({...json, 'version': 2}), isNull);
      expect(
        ShortcutSettings.fromJson({'version': 1, 'profile': 'custom'}),
        isNull,
      );
      expect(
        ShortcutSettings.fromJson({'version': 1, 'profile': 'unknown'}),
        isNull,
      );
    },
  );
}
