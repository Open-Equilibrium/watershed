import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'keymap.dart';

/// One key event as the key log writes it: kind, logical key and label, physical key,
/// character, the keys held and the focused node, to find why some Liquid key shortcuts do
/// nothing (PLAN.md). Labels, not debug names, so a release build reads the same.
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

/// Whether the log takes `event`: only while ⌘, ⌃, ⌥ or the Liquid key is held, so plain
/// typing, such as a password or token, never lands in it.
bool logsKey(KeyEvent event, Set<LogicalKeyboardKey> pressed) =>
    event.logicalKey == liquidKey ||
    pressed.any(
      (key) =>
          key == liquidKey ||
          LogicalKeyboardKey.expandSynonyms({
            LogicalKeyboardKey.meta,
            LogicalKeyboardKey.control,
            LogicalKeyboardKey.alt,
          }).contains(key),
    );

/// With `LIQUID_KEY_LOG=1`, writes the key events Flutter receives while a modifier or the
/// Liquid key is held to stderr; the macOS runner logs the native events beside them.
void logKeysIfAsked(Map<String, String> environment) {
  if (environment['LIQUID_KEY_LOG'] != '1') return;
  HardwareKeyboard.instance.addHandler((event) {
    final pressed = HardwareKeyboard.instance.logicalKeysPressed;
    if (logsKey(event, pressed)) {
      stderr.writeln(
        'flutter key: ${describeKeyEvent(event, pressed: pressed, focus: FocusManager.instance.primaryFocus?.debugLabel)}',
      );
    }
    return false;
  });
}
