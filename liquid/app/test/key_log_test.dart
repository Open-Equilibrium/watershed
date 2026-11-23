import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/shortcuts/key_log.dart';
import 'package:liquid/src/shortcuts/keymap.dart';

KeyDownEvent down(
  PhysicalKeyboardKey physical,
  LogicalKeyboardKey logical, [
  String? character,
]) => KeyDownEvent(
  physicalKey: physical,
  logicalKey: logical,
  character: character,
  timeStamp: Duration.zero,
);

KeyUpEvent release(PhysicalKeyboardKey physical, LogicalKeyboardKey logical) =>
    KeyUpEvent(
      physicalKey: physical,
      logicalKey: logical,
      timeStamp: Duration.zero,
    );

KeyRepeatEvent repeat(
  PhysicalKeyboardKey physical,
  LogicalKeyboardKey logical,
) => KeyRepeatEvent(
  physicalKey: physical,
  logicalKey: logical,
  character: 'a',
  timeStamp: Duration.zero,
);

void main() {
  final keyL = down(PhysicalKeyboardKey.keyL, LogicalKeyboardKey.keyL, 'l');
  final keyA = down(PhysicalKeyboardKey.keyA, LogicalKeyboardKey.keyA, 'a');
  final ctrl = down(
    PhysicalKeyboardKey.controlLeft,
    LogicalKeyboardKey.controlLeft,
  );

  test('a typed key stays out of the log when a modifier is held after it', () {
    final log = KeyLog();
    String? logged(KeyEvent event, Set<LogicalKeyboardKey> pressed) =>
        log.line(event, pressed: pressed, focus: null);

    expect(
      logged(keyA, {LogicalKeyboardKey.keyA}),
      isNull,
      reason: 'plain typing stays out',
    );
    expect(
      logged(ctrl, {LogicalKeyboardKey.keyA, LogicalKeyboardKey.controlLeft}),
      contains('held [Control Left]'),
      reason: 'the modifier itself is logged, and A was not logged, so it is not listed as held',
    );
    expect(
      logged(repeat(PhysicalKeyboardKey.keyA, LogicalKeyboardKey.keyA), {
        LogicalKeyboardKey.keyA,
        LogicalKeyboardKey.controlLeft,
      }),
      isNull,
      reason: 'a repeat follows its key-down',
    );
    expect(
      logged(release(PhysicalKeyboardKey.keyA, LogicalKeyboardKey.keyA), {
        LogicalKeyboardKey.controlLeft,
      }),
      isNull,
      reason: 'releasing A while Ctrl is held logs nothing',
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
    final at = down(PhysicalKeyboardKey.keyL, LogicalKeyboardKey.keyL, '@');
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
