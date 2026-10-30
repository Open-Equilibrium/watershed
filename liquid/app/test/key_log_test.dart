import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/key_log.dart';

void main() {
  test(
    'describes a key event with what was held, to diagnose the Liquid key',
    () {
      expect(
        describeKeyEvent(
          const KeyDownEvent(
            physicalKey: PhysicalKeyboardKey.keyL,
            logicalKey: LogicalKeyboardKey.keyL,
            character: 'l',
            timeStamp: Duration.zero,
          ),
          pressed: {LogicalKeyboardKey.f18, LogicalKeyboardKey.keyL},
          focus: 'composer',
        ),
        'down logical 0x6c (Key L) physical 0x7000f char "l" '
        'held [F18, Key L] focus composer',
      );
    },
  );
}
