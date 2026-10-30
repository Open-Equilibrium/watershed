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

  test('logs only the Liquid key and ⌘ or ⌃ shortcuts, never typing', () {
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
    const at = KeyDownEvent(
      physicalKey: PhysicalKeyboardKey.keyL,
      logicalKey: LogicalKeyboardKey.keyL,
      character: '@',
      timeStamp: Duration.zero,
    );
    expect(
      logsKey(at, {LogicalKeyboardKey.altLeft, LogicalKeyboardKey.keyL}),
      isFalse,
      reason: '⌥ types characters such as @ on German layouts',
    );
    expect(
      logsKey(at, {
        LogicalKeyboardKey.controlLeft,
        LogicalKeyboardKey.altRight,
        LogicalKeyboardKey.keyL,
      }),
      isFalse,
      reason: 'AltGr reaches Flutter as ⌃⌥ on Windows',
    );
  });
}
