import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_math_fork/flutter_math.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/inline_math.dart';
import 'package:liquid/src/workspace/math_block.dart';
import 'package:liquid/src/workspace/page_editor.dart' show textEdit;
import 'package:liquid/src/workspace/text_block.dart';
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

  test('dollars open math as Pandoc reads them', () {
    for (final (text, math) in [
      (r'The area is $A = \pi r^2$.', [r'A = \pi r^2']),
      (r'Tickets cost $5 and $10.', <String>[]),
      (r'$ x$ and $x $', <String>[]),
      (r'\$x$ and $$x$$', <String>[]),
      (r'$a$5', <String>[]),
      (r'$a$ and $b\$c$', ['a', r'b\$c']),
    ]) {
      expect(
        [for (final match in inlineMath.allMatches(text)) match[1]],
        math,
        reason: text,
      );
    }
  });

  testWidgets('math in a sentence is set until the caret enters it', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart(r'The area is $A = \pi r^2$, so')]);
    final inText = find.descendant(
      of: find.byType(TextBlock),
      matching: find.byType(Math),
    );
    expect(inText, findsOneWidget);
    await tester.tap(block(0));
    await tester.pump();
    final controller = field(tester, 0).controller!;
    controller.selection = const TextSelection.collapsed(offset: 16);
    await tester.pumpAndSettle();
    expect(inText, findsNothing, reason: 'the LaTeX shows');
    controller.selection = const TextSelection.collapsed(offset: 29);
    await tester.pumpAndSettle();
    expect(inText, findsOneWidget);
    expect(kit.fake.texts.values.first, r'The area is $A = \pi r^2$, so');
  });

  testWidgets(r'deleting back to $$ leaves a Text Block as it is', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart(r'$$x')]);
    await tester.tap(block(0));
    await tester.pump();
    await type(tester, r'$$');
    expect(kinds(), [BridgeBlockKind.text]);
  });

  testWidgets('↓ from the Text Block above opens a Math Block', (tester) async {
    await kit.show(tester, [
      const TextPart('Roots'),
      const TextPart('x'),
      const TextPart('Below'),
    ]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.turnIntoMath,
        page: kit.page,
        block: kit.fake.views[kit.page]![1].block,
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(block(0));
    await tester.pump();
    field(tester, 0).controller!.selection = const TextSelection.collapsed(
      offset: 5,
    );
    await tester.pump();
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(tester.widget<TextField>(source).focusNode!.hasFocus, isTrue);
    expect(
      tester.widget<TextField>(source).controller!.selection.baseOffset,
      0,
      reason: 'arriving from above, the caret starts the LaTeX',
    );
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(field(tester, 1).focusNode!.hasFocus, isTrue, reason: 'and leaves');
  });

  testWidgets('a Math Block source opens only after its LaTeX has loaded', (
    tester,
  ) async {
    await kit.show(tester, [TextPart(r'\frac{a}{b}')]);
    kit.fake.deferredKinds = {BridgeQueryKind.math};
    kit.fake.deferred = [];
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.turnIntoMath,
        page: kit.page,
        block: kit.fake.views[kit.page]!.first.block,
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byType(MathBlock));
    await type(tester, 'x');
    expect(source, findsNothing, reason: 'nothing is typed before the load');
    kit.fake.deferredKinds = {};
    for (final release in [...kit.fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
    await tester.tap(find.byType(MathBlock));
    await tester.pumpAndSettle();
    expect(source, findsOneWidget);
  });

  testWidgets('a read during IME composition keeps the composing source', (
    tester,
  ) async {
    await kit.show(tester, [TextPart(r'\frac{a}{b}')]);
    final block = kit.fake.views[kit.page]!.first.block;
    await kit.workspace.apply(
      mutation(BridgeMutationKind.turnIntoMath, page: kit.page, block: block),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byType(MathBlock));
    await tester.pumpAndSettle();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: r'\frac{a}{b}+c',
        selection: TextSelection.collapsed(offset: 13),
        composing: TextRange(start: 11, end: 13),
      ),
    );
    await kit.workspace.apply(
      textEdit(kit.page, block, r'\frac{a}{b}', r'\frac{a}{b}x'),
    );
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(source).controller!.text,
      r'\frac{a}{b}+c',
      reason: 'the composing text survives the read',
    );
  });

  testWidgets('arrow keys stay in the LaTeX while an IME composes', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('Roots'),
      const TextPart('x'),
      const TextPart('Below'),
    ]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.turnIntoMath,
        page: kit.page,
        block: kit.fake.views[kit.page]![1].block,
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byType(MathBlock));
    await tester.pumpAndSettle();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'x+1',
        selection: TextSelection.collapsed(offset: 3),
        composing: TextRange(start: 0, end: 3),
      ),
    );
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(source, findsOneWidget, reason: 'the IME keeps the arrow keys');
  });
}
