import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/shortcuts/shortcut_reveal.dart';
import 'package:liquid/src/theme/theme.dart';

void main() {
  Future<void> show(WidgetTester tester, Widget button) => tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: ShortcutReveal(
        liquid: false,
        child: Scaffold(body: Center(child: button)),
      ),
    ),
  );

  Future<void> reveal(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump(const Duration(milliseconds: 600));
  }

  for (final (name, onPressed, enabled) in [
    (
      'an enabled button is announced as a button that can be pressed',
      () {},
      true,
    ),
    (
      'a disabled button is announced as a disabled button without a tap action',
      null,
      false,
    ),
  ]) {
    testWidgets(name, (tester) async {
      final semantics = tester.ensureSemantics();
      await show(tester, PrimaryButton(label: 'Save', onPressed: onPressed));
      expect(
        tester.getSemantics(find.text('Save')),
        matchesSemantics(
          label: 'Save',
          isButton: true,
          hasEnabledState: true,
          isEnabled: enabled,
          hasTapAction: enabled,
        ),
      );
      semantics.dispose();
    });
  }

  testWidgets(
    'a revealed shortcut badge passes the taps on the button under its corner',
    (tester) async {
      var pressed = 0;
      await show(
        tester,
        PrimaryButton(
          label: 'Save',
          shortcut: '⌘S',
          onPressed: () => pressed++,
        ),
      );
      await reveal(tester);

      final button = tester.getRect(find.byType(InkWell));
      await tester.tapAt(Offset(button.right - 4, button.top + 2));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);

      expect(pressed, 1);
    },
  );

  testWidgets(
    'a revealed shortcut keeps a button stretched by its layout at its width',
    (tester) async {
      var pressed = 0;
      await show(
        tester,
        SizedBox(
          width: 200,
          child: PrimaryButton(
            label: 'Save',
            shortcut: '⌘S',
            onPressed: () => pressed++,
          ),
        ),
      );
      await reveal(tester);

      final button = tester.getRect(find.byType(InkWell));
      expect(button.width, 200);
      await tester.tapAt(Offset(button.right - 4, button.center.dy));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);

      expect(pressed, 1);
    },
  );
}
