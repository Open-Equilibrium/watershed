import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_math_fork/flutter_math.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/math_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final source = find.byKey(const Key('math source'));

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];
  String latexOf(int index) =>
      kit.fake.texts[kit.fake.views[kit.page]![index].block]!;

  Future<void> type(WidgetTester tester, String text) async {
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> math(WidgetTester tester, String latex) async {
    await kit.show(tester, [TextPart(latex)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.turnIntoMath,
        page: kit.page,
        block: kit.fake.views[kit.page]!.first.block,
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('/math turns an empty line into a Math Block that sets its '
      'LaTeX as it is typed', (tester) async {
    await kit.show(tester, [const TextPart('Roots'), const TextPart('')]);
    await tester.tap(block(1));
    await tester.pump();
    await type(tester, '/');
    await type(tester, '/math');
    expect(find.byKey(const ValueKey('choice Math')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);

    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.math]);
    expect(tester.widget<TextField>(source).focusNode!.hasFocus, isTrue);
    await type(tester, r'\frac{a}{b}');
    expect(latexOf(1), r'\frac{a}{b}');
    expect(
      find.descendant(of: find.byType(MathBlock), matching: find.byType(Math)),
      findsOneWidget,
    );

    await key(tester, LogicalKeyboardKey.escape);
    expect(source, findsNothing, reason: 'esc closes the LaTeX');
    await tester.tap(find.byType(MathBlock));
    await tester.pumpAndSettle();
    expect(source, findsOneWidget, reason: 'a click opens it again');
  });

  testWidgets(r'$$ at the start of an empty Text Block turns it into math', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('')]);
    await tester.tap(block(0));
    await tester.pump();
    await type(tester, r'$');
    await type(tester, r'$$');
    expect(kinds(), [BridgeBlockKind.math]);
    expect(latexOf(0), '');
  });

  testWidgets('LaTeX that cannot be set shows as written with the reason; '
      '⌫ in an empty Math Block turns it back into text', (tester) async {
    await math(tester, r'\fracc{1}{2}');
    expect(find.byKey(const Key('math error')), findsOneWidget);
    expect(find.textContaining(r'\fracc'), findsWidgets);

    await tester.tap(find.byType(MathBlock));
    await tester.pumpAndSettle();
    await type(tester, '');
    await key(tester, LogicalKeyboardKey.backspace);
    expect(kinds(), [BridgeBlockKind.text]);
  });
}
