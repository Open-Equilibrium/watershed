import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/keymap.dart';

void main() {
  const liquidN = Keys(LogicalKeyboardKey.keyN, liquid: true);
  const commandN = Keys(LogicalKeyboardKey.keyN, meta: true);
  const commandE = Keys(LogicalKeyboardKey.keyE, meta: true);
  const liquidD = Keys(LogicalKeyboardKey.keyD, liquid: true);
  const liquidE = Keys(LogicalKeyboardKey.keyE, liquid: true);
  const commandD = Keys(LogicalKeyboardKey.keyD, meta: true);

  test('a changed shortcut switches to Custom from its base', () {
    final keys = ShortcutSettings.standard().change(Command.diff, commandE);

    expect(keys.profile, Profile.custom);
    expect(keys.base, Profile.standard);
    expect(keys.bindings(Command.diff), [commandE]);
    expect(keys.changed(Command.diff), isTrue);
    expect(keys.reset(Command.diff).bindings(Command.diff), [commandD]);
    final all = keys.resetAll();
    expect(all.profile, Profile.standard);
    expect(all.changed(Command.diff), isFalse);
  });

  test('conflicts stay within an area, esc is shared, and taking keys', () {
    const esc = Keys(LogicalKeyboardKey.escape);
    const commandOne = Keys(LogicalKeyboardKey.digit1, meta: true);
    const commandEnter = Keys(LogicalKeyboardKey.enter, meta: true);
    const liquidCommandN = Keys(
      LogicalKeyboardKey.keyN,
      liquid: true,
      meta: true,
    );
    const liquidCommandOne = Keys(
      LogicalKeyboardKey.digit1,
      liquid: true,
      meta: true,
    );
    final liquid = ShortcutSettings.liquidKey();
    final standard = ShortcutSettings.standard();

    final cases = <(ShortcutSettings, Command, Keys, Object)>[
      (liquid, Command.focusInput, liquidN, [Command.newSession]),
      (liquid, Command.focusInput, commandN, [Command.newSession]),
      (liquid, Command.dismissQuestion, esc, isEmpty),
      (liquid, Command.newSession, liquidN, isEmpty),
      (standard, Command.stop, esc, isEmpty),
      (liquid, Command.send, commandEnter, isEmpty),
      (liquid, Command.diff, liquidCommandN, [Command.newSession]),
      (standard, Command.toggleToDo, commandOne, [Command.chat]),
      (standard, Command.chat, commandEnter, contains(Command.toggleToDo)),
      (standard, Command.toggleToDo, liquidCommandOne, [Command.chat]),
    ];
    for (final (keys, command, bound, owners) in cases) {
      expect(
        keys.conflicts(command, bound),
        owners,
        reason: '${command.name} ${bound.label}',
      );
    }

    final takenChord = liquid.change(
      Command.diff,
      liquidCommandN,
      clear: [Command.newSession],
    );
    expect(
      takenChord.menu(Command.newSession),
      isNull,
      reason: 'the menu bar would take the chord from Diff',
    );
    expect(takenChord.conflicts(Command.focusInput, commandN), [Command.diff]);

    final takenPair = standard.change(
      Command.toggleToDo,
      commandOne,
      clear: [Command.chat],
    );
    expect(takenPair.bindings(Command.chat), isEmpty);
    expect(takenPair.bindings(Command.toggleToDo), [commandOne]);

    expect(
      liquid.change(Command.send, commandEnter).bindings(Command.toggleToDo),
      liquid.bindings(Command.toggleToDo),
      reason: 'the to-do keeps ⌘⏎',
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

  test('resetting a command takes its default keys back from the others', () {
    final taken = ShortcutSettings.liquidKey().change(
      Command.diff,
      liquidN,
      clear: [Command.newSession],
    );

    final restored = taken.resetTaking(Command.newSession);
    expect(restored.bindings(Command.newSession), [liquidN, commandN]);
    expect(restored.bindings(Command.diff), [commandD]);
    expect(restored.changed(Command.newSession), isFalse);

    final on = taken
        .withLiquidKey(false)
        .resetTaking(Command.newSession)
        .withLiquidKey(true);
    expect(on.bindings(Command.newSession), [liquidN, commandN]);
    expect(on.conflicts(Command.newSession, liquidN), isEmpty);
    expect(on.bindings(Command.diff), isNot(contains(liquidN)));
  });

  test('inactive Liquid key bindings survive a clear and a change', () {
    final custom = ShortcutSettings.liquidKey()
        .change(Command.newSession, liquidE)
        .withLiquidKey(false);
    expect(custom.bindings(Command.newSession), [commandN]);

    final taken = custom.change(
      Command.focusInput,
      commandN,
      clear: [Command.newSession],
    );
    final read = ShortcutSettings.fromJson(taken.toJson())!;
    final on = read.withLiquidKey(true);
    expect(on.bindings(Command.newSession), [liquidE]);
    expect(on.bindings(Command.focusInput), contains(commandN));

    const keyJ = Keys(LogicalKeyboardKey.keyJ, meta: true);
    final edited = custom.change(Command.newSession, keyJ);
    expect(edited.bindings(Command.newSession), [keyJ]);
    expect(edited.withLiquidKey(true).bindings(Command.newSession), [
      keyJ,
      liquidE,
    ]);
  });

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

  test('macOS, fixed-row and typing keys are reserved, while the Liquid key alone is free', () {
    const commandQ = Keys(LogicalKeyboardKey.keyQ, meta: true);
    const shiftEnter = Keys(LogicalKeyboardKey.enter, shift: true);
    const commandLeft = Keys(LogicalKeyboardKey.arrowLeft, meta: true);
    const controlA = Keys(LogicalKeyboardKey.keyA, control: true);
    const commandNumpad1 = Keys(LogicalKeyboardKey.numpad1, meta: true);
    final cases = <(Command, Keys, String?)>[
      (Command.newSession, commandQ, 'Quit'),
      (
        Command.newSession,
        Keys(LogicalKeyboardKey.keyQ, meta: true, liquid: true),
        'Quit',
      ),
      (Command.queue, shiftEnter, 'New line'),
      (Command.newSession, Keys(LogicalKeyboardKey.keyN), 'typing'),
      (
        Command.dismissQuestion,
        Keys(LogicalKeyboardKey.arrowDown),
        'Move through options',
      ),
      (Command.newSession, liquidN, null),
      (
        Command.newSession,
        Keys(LogicalKeyboardKey.keyN, liquid: true, shift: true),
        'capitals',
      ),
      (Command.queue, commandLeft, 'typing'),
      (Command.queue, controlA, 'typing'),
      (Command.toggleToDo, Keys(LogicalKeyboardKey.keyA), 'typing'),
      (Command.toggleToDo, Keys(LogicalKeyboardKey.enter), 'New block or line'),
      (
        Command.toggleToDo,
        Keys(LogicalKeyboardKey.enter, shift: true),
        'New block or line',
      ),
      (Command.newSession, commandNumpad1, null),
      for (final key in [
        LogicalKeyboardKey.numpad1,
        LogicalKeyboardKey.numpadAdd,
        LogicalKeyboardKey.numpadDecimal,
      ])
        (Command.newSession, Keys(key), 'typing'),
    ];
    for (final (command, keys, expected) in cases) {
      expect(
        reserved(command, keys),
        expected,
        reason: '${command.name} ${keys.label}',
      );
    }
  });

  test('a default the user’s own binding takes stays unbound', () {
    const commandU = Keys(LogicalKeyboardKey.keyU, meta: true);
    final json = ShortcutSettings.standard()
        .change(Command.focusInput, commandU)
        .toJson();

    final read = ShortcutSettings.fromJson(json)!;

    expect(read.bindings(Command.focusInput), [commandU]);
    expect(
      read.bindings(Command.statistics),
      isEmpty,
      reason: 'the user’s own ⌘U wins over the Statistics default',
    );
    expect(read.label(Command.statistics), '');
    expect(read.changed(Command.statistics), isTrue);
    expect(read.defaultLabel(Command.statistics), '⌘U');
    expect(read.bindings(Command.diff), [
      commandD,
    ], reason: 'a default that nothing takes keeps its keys');
    expect(ShortcutSettings.fromJson(read.toJson()), read);
  });

  test('settings round-trip JSON and ignore unknown data', () {
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
      const Keys(LogicalKeyboardKey.keyD, liquid: true),
      commandD,
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
  });
}
