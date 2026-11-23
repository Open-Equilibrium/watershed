import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/code_block.dart';

import 'card_harness.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final code = find.descendant(
    of: find.byType(CodeBlock),
    matching: find.byKey(const Key('code')),
  );
  TextField codeField(WidgetTester tester) => tester.widget<TextField>(code);

  List<BridgeBlockKind> kinds() => [
    for (final view in kit.fake.views[kit.page]!) view.kind,
  ];
  String codeOf(int index) =>
      kit.fake.texts[kit.fake.views[kit.page]![index].block]!;

  TextEditingValue valueOf(String text, [int? offset]) => TextEditingValue(
    text: text,
    selection: TextSelection.collapsed(offset: offset ?? text.length),
  );

  Future<void> input(WidgetTester tester, TextEditingValue value) async {
    tester.testTextInput.updateEditingValue(value);
    await tester.pumpAndSettle();
  }

  Future<void> caretInCode(WidgetTester tester, int offset) async {
    await tester.tap(code);
    await tester.pump();
    codeField(tester).controller!.selection = TextSelection.collapsed(
      offset: offset,
    );
    await tester.pump();
  }

  testWidgets('``` at a Text Block start makes a Code Block', (tester) async {
    await kit.show(tester, [
      const TextPart('Shares round:'),
      const TextPart('share()'),
    ]);
    await tester.tap(block(1));
    await tester.pump();
    await input(tester, valueOf('```share()', 3));

    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.code]);
    expect(codeOf(1), 'share()');
    expect(kit.fake.languages[kit.fake.views[kit.page]![1].block], isNull);
    expect(codeField(tester).focusNode!.hasFocus, isTrue);
    expect(find.text('Plain text'), findsOneWidget);
  });

  testWidgets('/code turns an empty line into a Code Block', (tester) async {
    await kit.show(tester, [
      const TextPart('Shares round:'),
      const TextPart(''),
      const TextPart('After'),
    ]);
    await tester.tap(block(1));
    await tester.pump();
    for (final typed in ['/', '/cod']) {
      await input(tester, valueOf(typed));
    }
    expect(find.byKey(const ValueKey('choice Code')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);

    expect(kinds(), [
      BridgeBlockKind.text,
      BridgeBlockKind.code,
      BridgeBlockKind.text,
    ]);
    expect(codeOf(1), '');
    expect(codeField(tester).focusNode!.hasFocus, isTrue);
  });

  testWidgets('⏎ keeps the indentation; ⇥ and ⇧⇥ indent lines', (tester) async {
    await kit.show(tester, [const CodePart('if (total) {\n  return 0;')]);
    await caretInCode(tester, 24);

    await key(tester, LogicalKeyboardKey.enter);
    expect(codeOf(0), 'if (total) {\n  return 0;\n  ');
    await input(tester, valueOf('if (total) {\n  return 0;\n  }', 28));
    expect(codeOf(0), 'if (total) {\n  return 0;\n  }');

    codeField(tester).controller!.selection = const TextSelection(
      baseOffset: 16,
      extentOffset: 28,
    );
    await tester.pump();
    await key(tester, LogicalKeyboardKey.tab);
    expect(codeOf(0), 'if (total) {\n    return 0;\n    }');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await key(tester, LogicalKeyboardKey.tab);
    await key(tester, LogicalKeyboardKey.tab);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    expect(codeOf(0), 'if (total) {\nreturn 0;\n}');
    expect(codeField(tester).focusNode!.hasFocus, isTrue, reason: 'no move');
  });

  testWidgets('the language picker filters and sets it', (tester) async {
    await kit.show(tester, [const CodePart('let x = 1;')]);
    await tester.tap(find.text('Plain text'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('language search')), 'type');
    await tester.pumpAndSettle();
    expect(find.text('TypeScript'), findsOneWidget);
    expect(find.text('Rust'), findsNothing);
    await key(tester, LogicalKeyboardKey.enter);

    final block = kit.fake.views[kit.page]![0].block;
    expect(kit.fake.languages[block], 'typescript');
    expect(find.byKey(const Key('language search')), findsNothing);
    expect(find.text('TypeScript'), findsOneWidget, reason: 'in the header');
  });

  testWidgets('Copy copies the code and shows a check', (tester) async {
    final copied = recordClipboard(tester);
    await kit.show(tester, [const CodePart('share()')]);
    await tester.tap(find.text('Copy'));
    await tester.pump();
    expect(copied(), 'share()');
    expect(find.text('Copied'), findsOneWidget);
    await tester.pumpAndSettle(const Duration(seconds: 3));
    expect(find.text('Copy'), findsOneWidget);
  });

  testWidgets('the caret moves in and out of code; esc leaves', (tester) async {
    await kit.show(tester, [
      const TextPart('Shares round:'),
      const CodePart('a\nb'),
      const TextPart('Reports'),
    ]);
    await tester.tap(block(1));
    await tester.pump();
    field(tester, 1).controller!.selection = const TextSelection.collapsed(
      offset: 0,
    );
    await tester.pump();
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(codeField(tester).focusNode!.hasFocus, isTrue);
    expect(codeField(tester).controller!.selection.baseOffset, 3);

    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(field(tester, 1).focusNode!.hasFocus, isTrue);
    expect(field(tester, 1).controller!.selection.baseOffset, 0);

    await caretInCode(tester, 0);
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(field(tester, 0).focusNode!.hasFocus, isTrue);
    expect(field(tester, 0).controller!.selection.baseOffset, 13);

    await caretInCode(tester, 1);
    await key(tester, LogicalKeyboardKey.escape);
    expect(field(tester, 1).focusNode!.hasFocus, isTrue);
    expect(field(tester, 1).controller!.selection.baseOffset, 0);
  });

  testWidgets('⌫ in an empty Code Block makes a Text Block', (tester) async {
    await kit.show(tester, [const TextPart('Intro'), const CodePart('')]);
    await caretInCode(tester, 0);
    await key(tester, LogicalKeyboardKey.backspace);

    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
    expect(find.byType(CodeBlock), findsNothing);
    expect(field(tester, 1).focusNode!.hasFocus, isTrue);
  });
}
