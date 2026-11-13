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
      expect(keys.label(Command.answerEditor), '⇪⏎');
      expect(keys.menu(Command.newSession), commandN);
      // Platform standards and keys without ⌘ stay as they are.
      expect(keys.label(Command.settings), '⌘,');
      expect(keys.label(Command.dismissQuestion), 'esc');
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

    expect(keys.conflicts(Command.focusInput, liquidN), [Command.newSession]);
    // The ⌘ variant that keeps working conflicts too.
    expect(keys.conflicts(Command.focusInput, commandN), [Command.newSession]);
    // esc stops a run in the input area and dismisses a question.
    expect(
      keys.conflicts(
        Command.dismissQuestion,
        const Keys(LogicalKeyboardKey.escape),
      ),
      isEmpty,
    );
    expect(keys.conflicts(Command.newSession, liquidN), isEmpty);
  });

  test('a Liquid ⌘ chord conflicts with the command whose menu shows the same ⌘ keys', () {
    const liquidCommandN = Keys(
      LogicalKeyboardKey.keyN,
      liquid: true,
      meta: true,
    );
    final keys = ShortcutSettings.liquidKey();

    expect(keys.conflicts(Command.diff, liquidCommandN), [Command.newSession]);
    final taken = keys.change(
      Command.diff,
      liquidCommandN,
      clear: [Command.newSession],
    );
    expect(
      taken.menu(Command.newSession),
      isNull,
      reason: 'the menu bar would take the chord from Diff',
    );
    // The other way round: a ⌘ chord meets the Liquid key chord of another command.
    expect(taken.conflicts(Command.focusInput, commandN), [Command.diff]);
  });

  test('a Page to-do does not compete with the agent Send command', () {
    const commandEnter = Keys(LogicalKeyboardKey.enter, meta: true);
    final keys = ShortcutSettings.liquidKey();

    expect(keys.conflicts(Command.send, commandEnter), isEmpty);
    expect(
      keys.change(Command.send, commandEnter).bindings(Command.toggleToDo),
      keys.bindings(Command.toggleToDo),
      reason: 'the to-do keeps ⌘⏎',
    );
  });

  test('a to-do shortcut cannot take a key that types in the page', () {
    expect(
      reserved(Command.toggleToDo, const Keys(LogicalKeyboardKey.keyA)),
      'typing',
    );
  });

  test(
    'a to-do shortcut cannot take the keys that break a line in the page',
    () {
      expect(
        reserved(Command.toggleToDo, const Keys(LogicalKeyboardKey.enter)),
        'New block or line',
      );
      expect(
        reserved(
          Command.toggleToDo,
          const Keys(LogicalKeyboardKey.enter, shift: true),
        ),
        'New block or line',
      );
    },
  );

  test('a ⌘ chord of the app area conflicts in every area, both ways', () {
    const commandF = Keys(LogicalKeyboardKey.keyF, meta: true);
    const commandOne = Keys(LogicalKeyboardKey.digit1, meta: true);
    final keys = ShortcutSettings.standard();

    expect(keys.conflicts(Command.filterSteps, commandOne), [Command.chat]);
    expect(keys.conflicts(Command.chat, commandF), [Command.filterSteps]);
    expect(
      keys.conflicts(
        Command.filterSteps,
        const Keys(LogicalKeyboardKey.digit1, liquid: true, meta: true),
      ),
      [Command.chat],
      reason: 'the menu bar ignores the Liquid key',
    );
    final taken = keys.change(
      Command.filterSteps,
      commandOne,
      clear: [Command.chat],
    );
    expect(taken.bindings(Command.chat), isEmpty);
    expect(taken.bindings(Command.filterSteps), [commandOne]);
  });

  test('esc may be shared with the app area', () {
    expect(
      ShortcutSettings.standard().conflicts(
        Command.closeStep,
        const Keys(LogicalKeyboardKey.escape),
      ),
      isEmpty,
    );
  });

  test('the default keys of every profile conflict with nothing', () {
    for (final keys in [
      ShortcutSettings.standard(),
      ShortcutSettings.liquidKey(),
    ]) {
      for (final command in Command.values) {
        for (final bound in keys.bindings(command)) {
          expect(
            keys.conflicts(command, bound),
            isEmpty,
            reason: '${keys.base.label} ${command.name} ${bound.label}',
          );
        }
      }
    }
  });

  test('taking keys anyway clears them from the other command', () {
    final keys = ShortcutSettings.liquidKey().change(
      Command.focusInput,
      liquidN,
      clear: [Command.newSession],
    );

    expect(keys.bindings(Command.focusInput), [
      liquidN,
      const Keys(LogicalKeyboardKey.keyL, meta: true),
    ]);
    expect(keys.bindings(Command.newSession), [commandN]);
    expect(keys.label(Command.newSession), '⌘N');
  });

  test('taking keys clears every command that runs them', () {
    const commandEnter = Keys(LogicalKeyboardKey.enter, meta: true);
    final keys = ShortcutSettings.standard();
    final owners = keys.conflicts(Command.newSession, commandEnter);
    expect(owners, unorderedEquals([Command.toggleToDo, Command.answerEditor]));

    final taken = keys.change(Command.newSession, commandEnter, clear: owners);
    expect(taken.conflicts(Command.newSession, commandEnter), isEmpty);
    expect(taken.bindings(Command.toggleToDo), isNot(contains(commandEnter)));
    expect(taken.bindings(Command.answerEditor), isNot(contains(commandEnter)));
  });

  test('resetting a command takes its default keys back from the commands that run them', () {
    final taken = ShortcutSettings.liquidKey().change(
      Command.diff,
      liquidN,
      clear: [Command.newSession],
    );

    final restored = taken.resetTaking(Command.newSession);
    expect(restored.bindings(Command.newSession), [liquidN, commandN]);
    expect(restored.bindings(Command.diff), [
      const Keys(LogicalKeyboardKey.digit3, meta: true),
    ]);
    expect(restored.changed(Command.newSession), isFalse);
  });

  test(
    'resetting with the Liquid key off leaves no conflict once it is on',
    () {
      final taken = ShortcutSettings.liquidKey().change(
        Command.diff,
        liquidN,
        clear: [Command.newSession],
      );

      final on = taken
          .withLiquidKey(false)
          .resetTaking(Command.newSession)
          .withLiquidKey(true);
      expect(on.bindings(Command.newSession), [liquidN, commandN]);
      expect(on.conflicts(Command.newSession, liquidN), isEmpty);
      expect(on.bindings(Command.diff), isNot(contains(liquidN)));
    },
  );

  test('the Custom switch decides whether Liquid key combinations exist', () {
    final off = ShortcutSettings.liquidKey().withLiquidKey(false);
    expect(off.profile, Profile.custom);
    expect(off.base, Profile.liquidKey);
    expect(off.bindings(Command.newSession), [commandN]);
    expect(off.label(Command.answerEditor), '⌘⏎');

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
      reserved(
        Command.dismissQuestion,
        const Keys(LogicalKeyboardKey.arrowDown),
      ),
      'Move through options',
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
    for (final key in [
      LogicalKeyboardKey.numpad1,
      LogicalKeyboardKey.numpadAdd,
      LogicalKeyboardKey.numpadDecimal,
    ]) {
      expect(
        reserved(Command.newSession, Keys(key)),
        'typing',
        reason: 'numpad ${key.keyLabel} types a character',
      );
    }
    expect(
      reserved(
        Command.newSession,
        const Keys(LogicalKeyboardKey.numpad1, meta: true),
      ),
      isNull,
    );
  });

  test('the Liquid key with Shift is kept for capitals, which macOS takes', () {
    expect(
      reserved(
        Command.newSession,
        const Keys(LogicalKeyboardKey.keyN, liquid: true, shift: true),
      ),
      'capitals',
    );
    expect(
      reserved(Command.newSession, liquidN),
      isNull,
      reason: 'the Liquid key alone stays free',
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
