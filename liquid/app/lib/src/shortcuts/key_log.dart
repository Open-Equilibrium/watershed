import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'keymap.dart';

/// One key event as the key log writes it: kind, logical key and label, physical key,
/// character, the keys held and the focused node, to find why some Liquid key shortcuts do
/// nothing. Labels, not debug names, so a release build reads the same.
String describeKeyEvent(
  KeyEvent event, {
  required Set<LogicalKeyboardKey> pressed,
  required String? focus,
}) {
  final kind = switch (event) {
    KeyDownEvent() => 'down',
    KeyRepeatEvent() => 'repeat',
    _ => 'up',
  };
  final character = event.character;
  return [
    kind,
    '0x${event.logicalKey.keyId.toRadixString(16)} "${event.logicalKey.keyLabel}"',
    'physical 0x${event.physicalKey.usbHidUsage.toRadixString(16)}',
    'char ${character == null ? '-' : '"$character"'}',
    'held [${pressed.map((key) => key.keyLabel).join(', ')}]',
    'focus ${focus ?? '-'}',
  ].join(' ');
}

/// Whether the log takes `event`: with the Liquid key, or with ⌘ or ⌃ but not ⌥, so plain
/// typing, such as a password or token, stays out of it; ⌥ types characters such as @ on many
/// layouts, and AltGr reaches Flutter as ⌃⌥ on Windows.
bool logsKey(KeyEvent event, Set<LogicalKeyboardKey> pressed) {
  bool held(LogicalKeyboardKey modifier) =>
      pressed.any(LogicalKeyboardKey.expandSynonyms({modifier}).contains);
  if (event.logicalKey == liquidKey || pressed.contains(liquidKey)) return true;
  return (held(LogicalKeyboardKey.meta) || held(LogicalKeyboardKey.control)) &&
      !held(LogicalKeyboardKey.alt);
}

final _modifiers = LogicalKeyboardKey.expandSynonyms({
  LogicalKeyboardKey.meta,
  LogicalKeyboardKey.control,
  LogicalKeyboardKey.alt,
  LogicalKeyboardKey.shift,
});

/// The lines of the key log: [logsKey] takes a modifier or the Liquid key, and a typed key only
/// with its key-down. A typed key's repeats and key-up follow that key-down, so a key typed
/// alone stays out even when ⌘ or ⌃ is held after it. A line names only the modifiers, the
/// Liquid key and the typed keys the log took.
class KeyLog {
  /// The typed keys whose key-down the log took and that are not released yet.
  final _logged = <LogicalKeyboardKey>{};

  /// The line for `event`, or null when the log leaves it out.
  String? line(
    KeyEvent event, {
    required Set<LogicalKeyboardKey> pressed,
    required String? focus,
  }) {
    final key = event.logicalKey;
    final typed = !_modifiers.contains(key);
    final bool logged;
    if (typed && event is! KeyDownEvent) {
      logged = event is KeyUpEvent
          ? _logged.remove(key)
          : _logged.contains(key);
    } else {
      logged = logsKey(event, pressed);
      if (typed) {
        if (logged) {
          _logged.add(key);
        } else {
          _logged.remove(key);
        }
      }
    }
    if (!logged) return null;
    final held = {
      for (final pressedKey in pressed)
        if (_modifiers.contains(pressedKey) ||
            pressedKey == liquidKey ||
            _logged.contains(pressedKey))
          pressedKey,
    };
    return describeKeyEvent(event, pressed: held, focus: focus);
  }
}

/// With `LIQUID_KEY_LOG=1`, writes the lines of [KeyLog] for the key events Flutter receives to
/// stderr; the macOS runner logs the native events beside them.
void logKeysIfAsked(Map<String, String> environment) {
  if (environment['LIQUID_KEY_LOG'] != '1') return;
  final log = KeyLog();
  HardwareKeyboard.instance.addHandler((event) {
    final line = log.line(
      event,
      pressed: HardwareKeyboard.instance.logicalKeysPressed,
      focus: FocusManager.instance.primaryFocus?.debugLabel,
    );
    if (line != null) stderr.writeln('flutter key: $line');
    return false;
  });
}
