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

  test('a typed key stays out of the log when a modifier is held after it', () {
    const keyA = KeyDownEvent(
      physicalKey: PhysicalKeyboardKey.keyA,
      logicalKey: LogicalKeyboardKey.keyA,
      character: 'a',
      timeStamp: Duration.zero,
    );
    const ctrl = KeyDownEvent(
      physicalKey: PhysicalKeyboardKey.controlLeft,
      logicalKey: LogicalKeyboardKey.controlLeft,
      timeStamp: Duration.zero,
    );
    const aRepeat = KeyRepeatEvent(
      physicalKey: PhysicalKeyboardKey.keyA,
      logicalKey: LogicalKeyboardKey.keyA,
      character: 'a',
      timeStamp: Duration.zero,
    );
    const aUp = KeyUpEvent(
      physicalKey: PhysicalKeyboardKey.keyA,
      logicalKey: LogicalKeyboardKey.keyA,
      timeStamp: Duration.zero,
    );
    final log = KeyLog();
    expect(
      log.line(keyA, pressed: {LogicalKeyboardKey.keyA}, focus: null),
      isNull,
      reason: 'plain typing stays out',
    );
    final modifier = log.line(
      ctrl,
      pressed: {LogicalKeyboardKey.keyA, LogicalKeyboardKey.controlLeft},
      focus: null,
    );
    expect(modifier, isNotNull, reason: 'the modifier itself is logged');
    expect(
      modifier,
      contains('held [Control Left]'),
      reason: 'A was not logged, so it is not listed as held',
    );
    expect(
      log.line(
        aRepeat,
        pressed: {LogicalKeyboardKey.keyA, LogicalKeyboardKey.controlLeft},
        focus: null,
      ),
      isNull,
      reason: 'a repeat follows its key-down',
    );
    expect(
      log.line(aUp, pressed: {LogicalKeyboardKey.controlLeft}, focus: null),
      isNull,
      reason: 'releasing A while Ctrl is held logs nothing',
    );
  });

  test('a logged key keeps its repeats and key-up in the log', () {
    const keyA = KeyDownEvent(
      physicalKey: PhysicalKeyboardKey.keyA,
      logicalKey: LogicalKeyboardKey.keyA,
      character: '\u0001',
      timeStamp: Duration.zero,
    );
    const aUp = KeyUpEvent(
      physicalKey: PhysicalKeyboardKey.keyA,
      logicalKey: LogicalKeyboardKey.keyA,
      timeStamp: Duration.zero,
    );
    final log = KeyLog();
    final pressed = {LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.keyA};
    expect(log.line(keyA, pressed: pressed, focus: null), isNotNull);
    expect(
      log.line(aUp, pressed: {LogicalKeyboardKey.controlLeft}, focus: null),
      isNotNull,
      reason: 'the key-up of a logged key is logged',
    );
  });

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
