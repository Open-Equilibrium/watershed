import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/key_log.dart';
import 'package:liquid/src/shortcuts/keymap.dart';

void main() {
  const keyL = KeyDownEvent(
    physicalKey: PhysicalKeyboardKey.keyL,
    logicalKey: LogicalKeyboardKey.keyL,
    character: 'l',
    timeStamp: Duration.zero,
  );

  test(
    'describes a key event with what was held, to diagnose the Liquid key',
    () {
      expect(
        describeKeyEvent(
          keyL,
          pressed: {liquidKey, LogicalKeyboardKey.keyL},
          focus: 'composer',
        ),
        'down 0x6c "L" physical 0x7000f char "l" held [F18, L] focus composer',
        reason: 'labels, not debug names, so a release build reads the same',
      );
    },
  );

  test('logs only while a modifier or the Liquid key is held', () {
    expect(
      logsKey(keyL, {LogicalKeyboardKey.keyL}),
      isFalse,
      reason: 'plain typing, such as a token, stays out of the log',
    );
    expect(logsKey(keyL, {liquidKey, LogicalKeyboardKey.keyL}), isTrue);
    expect(
      logsKey(keyL, {LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.keyL}),
      isTrue,
    );
  });
}
