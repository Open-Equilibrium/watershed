import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

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

Future<void> undo(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
  await tester.pumpAndSettle();
}

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
