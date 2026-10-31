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

/// With `LIQUID_KEY_LOG=1`, writes the key events Flutter receives that [logsKey] takes to
/// stderr; the macOS runner logs the native events beside them.
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
