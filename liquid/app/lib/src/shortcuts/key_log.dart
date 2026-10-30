import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// One key event as the key log writes it: kind, logical and physical key, character, the keys
/// held and the focused node, to find why some Liquid key shortcuts do nothing (PLAN.md).
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
    'logical 0x${event.logicalKey.keyId.toRadixString(16)} (${event.logicalKey.debugName})',
    'physical 0x${event.physicalKey.usbHidUsage.toRadixString(16)}',
    'char ${character == null ? '-' : '"$character"'}',
    'held [${pressed.map((key) => key.debugName).join(', ')}]',
    'focus ${focus ?? '-'}',
  ].join(' ');
}

/// With `LIQUID_KEY_LOG=1`, writes every key event Flutter receives to stderr; the macOS runner
/// logs the native events beside them.
void logKeysIfAsked(Map<String, String> environment) {
  if (environment['LIQUID_KEY_LOG'] != '1') return;
  HardwareKeyboard.instance.addHandler((event) {
    stderr.writeln(
      'flutter key: ${describeKeyEvent(event, pressed: HardwareKeyboard.instance.logicalKeysPressed, focus: FocusManager.instance.primaryFocus?.debugLabel)}',
    );
    return false;
  });
}
