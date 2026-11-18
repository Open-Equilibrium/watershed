import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'keymap.dart';

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

class KeyLog {
  final _logged = <LogicalKeyboardKey>{};

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
