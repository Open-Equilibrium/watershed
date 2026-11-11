import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Paints [top] after a box filled with [bottom], so [top] shows on it.
PaintPattern paintsAbove(Color bottom, Color top) {
  bool fills(Object? paint, Color color) =>
      paint is Paint && paint.color.toARGB32() == color.toARGB32();
  bool box(Symbol method) => method == #drawRect || method == #drawRRect;
  return paints
    ..something(
      (method, arguments) => box(method) && fills(arguments[1], bottom),
    )
    ..something((method, arguments) => box(method) && fills(arguments[1], top));
}

/// ⌘Z, as the macOS keyboard sends it.
Future<void> undo(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
  await tester.pumpAndSettle();
}

/// Presses Tab until the keyboard is on something inside [target].
Future<void> tabTo(WidgetTester tester, Finder target) async {
  final inside = find.descendant(of: target, matching: find.byType(Focus));
  for (var press = 0; press < 30; press++) {
    final focus = FocusManager.instance.primaryFocus?.context;
    if (focus != null && inside.evaluate().contains(focus)) return;
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pumpAndSettle();
  }
  fail('Tab never reached $target');
}
