import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/highlight.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/code_block.dart';
import 'package:liquid/src/workspace/text_block.dart';

import 'page_kit.dart';

/// The approved Code Block mockups.
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

  /// Puts `value` into the focused field, as the text input does.
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

  testWidgets(
    '``` at a Text Block\'s start turns it into a Code Block holding the rest',
    (tester) async {
      await kit.show(tester, [
        const TextPart('Shares round:'),
        const TextPart('share()'),
      ]);
      await tester.tap(block(1));
      await tester.pump();
      await input(
        tester,
        const TextEditingValue(
          text: '```share()',
          selection: TextSelection.collapsed(offset: 3),
        ),
      );

      expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.code]);
      expect(codeOf(1), 'share()');
      expect(kit.fake.languages[kit.fake.views[kit.page]![1].block], isNull);
      expect(codeField(tester).focusNode!.hasFocus, isTrue);
      expect(find.text('Plain text'), findsOneWidget);
    },
  );

  testWidgets('Code in the slash commands turns the empty line into code', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('Shares round:'),
      const TextPart(''),
      const TextPart('After'),
    ]);
    await tester.tap(block(1));
    await tester.pump();
    for (final typed in ['/', '/cod']) {
      await input(
        tester,
        TextEditingValue(
          text: typed,
          selection: TextSelection.collapsed(offset: typed.length),
        ),
      );
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

  testWidgets(
    'typing writes code; ⏎ keeps the indentation, ⇥ and ⇧⇥ indent lines',
    (tester) async {
      await kit.show(tester, [const CodePart('if (total) {\n  return 0;')]);
      await caretInCode(tester, 24);

      await key(tester, LogicalKeyboardKey.enter);
      expect(codeOf(0), 'if (total) {\n  return 0;\n  ');
      await input(
        tester,
        const TextEditingValue(
          text: 'if (total) {\n  return 0;\n  }',
          selection: TextSelection.collapsed(offset: 28),
        ),
      );
      expect(codeOf(0), 'if (total) {\n  return 0;\n  }');

      // The last two lines, selected from inside the second.
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
    },
  );

  testWidgets('the language picker filters and sets the language', (
    tester,
  ) async {
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

  testWidgets('Copy puts the code on the clipboard and shows a check', (
    tester,
  ) async {
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String?;
        }
        return null;
      },
    );
    await kit.show(tester, [const CodePart('share()')]);
    await tester.tap(find.text('Copy'));
    await tester.pump();
    expect(copied, 'share()');
    expect(find.text('Copied'), findsOneWidget);
    await tester.pumpAndSettle(const Duration(seconds: 3));
    expect(find.text('Copy'), findsOneWidget);
  });

  testWidgets(
    'the caret passes into and out of the code; esc leaves it below',
    (tester) async {
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
    },
  );

  testWidgets('⌫ in an empty Code Block turns it into an empty Text Block', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Intro'), const CodePart('')]);
    await caretInCode(tester, 0);
    await key(tester, LogicalKeyboardKey.backspace);

    expect(kinds(), [BridgeBlockKind.text, BridgeBlockKind.text]);
    expect(kit.fake.textsOf(kit.page), ['Intro', '']);
    expect(find.byType(CodeBlock), findsNothing);
    expect(field(tester, 1).focusNode!.hasFocus, isTrue);
  });

  testWidgets('a line typed past the Block\'s width scrolls, never wraps', (
    tester,
  ) async {
    await kit.show(tester, [const CodePart('short')]);
    final line = tester.getSize(code).height;
    await caretInCode(tester, 5);
    final long = 'x' * 300;
    await input(
      tester,
      TextEditingValue(
        text: long,
        selection: TextSelection.collapsed(offset: long.length),
      ),
    );
    expect(codeOf(0), long);
    expect(tester.getSize(code).height, line, reason: 'still one line');
  });

  testWidgets('a lone quote colors no later line as a string', (tester) async {
    await kit.show(tester, [const CodePart('')]);
    final context = tester.element(find.byType(CodeBlock));
    final spans = highlight(
      context,
      "fn f(s: &'a str) {}\nlet n = 1;\nlet c = 'x';",
    ).children!.cast<TextSpan>();
    expect(
      spans.firstWhere((span) => span.text == 'let').style?.color,
      context.ws.synKw,
    );
  });

  testWidgets('code past 24 lines scrolls inside the Block', (tester) async {
    final long = [for (var line = 1; line <= 60; line++) 'line $line'];
    await kit.show(tester, [
      CodePart(long.join('\n')),
      const TextPart('After'),
    ]);
    final height = tester.getSize(find.byType(CodeBlock)).height;
    expect(height, lessThan(30 * 20), reason: '24 lines and the header');
    expect(height, greaterThan(24 * 20));
    expect(find.byType(TextBlock), findsOneWidget, reason: 'the Page goes on');
  });
}
