import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/theme/theme.dart';

void main() {
  Future<void> show(WidgetTester tester, Widget button) => tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(body: Center(child: button)),
    ),
  );

  testWidgets(
    'an enabled button is announced as a button that can be pressed',
    (tester) async {
      final semantics = tester.ensureSemantics();
      await show(tester, PrimaryButton(label: 'Save', onPressed: () {}));
      expect(
        tester.getSemantics(find.text('Save')),
        matchesSemantics(
          label: 'Save',
          isButton: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );
      semantics.dispose();
    },
  );

  testWidgets(
    'a disabled button is announced as a disabled button without a tap action',
    (tester) async {
      final semantics = tester.ensureSemantics();
      await show(tester, const PrimaryButton(label: 'Save', onPressed: null));
      expect(
        tester.getSemantics(find.text('Save')),
        matchesSemantics(
          label: 'Save',
          isButton: true,
          hasEnabledState: true,
          isEnabled: false,
          hasTapAction: false,
        ),
      );
      semantics.dispose();
    },
  );
}
