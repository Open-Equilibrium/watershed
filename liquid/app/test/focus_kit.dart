import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/theme/theme.dart';

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

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

void expectOne(Finder finder) => expect(finder, findsOneWidget);

void expectNone(Finder finder) => expect(finder, findsNothing);

Future<void> pumpApp(WidgetTester tester, Widget home, {Size? size}) async {
  if (size != null) await tester.binding.setSurfaceSize(size);
  await tester.pumpWidget(
    MaterialApp(theme: liquidTheme(Brightness.light), home: home),
  );
  await tester.pumpAndSettle();
}
