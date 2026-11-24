import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'page_kit.dart';

class _Typed {
  final lines = <String>[''];
  int line = 0;
  int column = 0;

  void insert(String text) {
    lines[line] = lines[line].replaceRange(column, column, text);
    column += text.length;
  }

  void enter() {
    final tail = lines[line].substring(column);
    lines[line] = lines[line].substring(0, column);
    lines.insert(++line, tail);
    column = 0;
  }

  void backspace() {
    if (column > 0) {
      final last = lines[line].substring(0, column).characters.last;
      lines[line] = lines[line].replaceRange(column - last.length, column, '');
      column -= last.length;
    } else if (line > 0) {
      final text = lines.removeAt(line);
      column = lines[--line].length;
      lines[line] += text;
    }
  }
}

enum _End { stay, leave, quit }

void main() {
  for (var seed = 0; seed < 24; seed++) {
    final end = _End.values[seed % 3];
    testWidgets('typing with slow answers stores what was typed, seed $seed, '
        '${end.name}', (tester) async {
      final random = Random(seed);
      final kit = PageKit();
      await kit.show(tester, const []);
      final fake = kit.fake;
      fake.delay = () => Duration(milliseconds: random.nextInt(20));

      EditableTextState focused() => tester.state<EditableTextState>(
        find.byWidgetPredicate(
          (widget) => widget is EditableText && widget.focusNode.hasFocus,
          skipOffstage: false,
        ),
      );
      void input(TextEditingValue value) =>
          tester.testTextInput.updateEditingValue(value);
      TextEditingValue inserted(String text, {TextRange? composing}) {
        final value = focused().textEditingValue;
        final at = value.selection.baseOffset;
        return TextEditingValue(
          text: value.text.replaceRange(at, at, text),
          selection: TextSelection.collapsed(offset: at + text.length),
          composing: composing ?? TextRange.empty,
        );
      }

      final typed = _Typed();
      await tester.tap(find.byKey(const Key('first-line')));
      await tester.pump();
      input(inserted('a'));
      typed.insert('a');
      for (var step = 0; step < 30; step++) {
        await tester.pump(Duration(milliseconds: random.nextInt(15)));
        switch (random.nextInt(10)) {
          case 0 || 1 || 2:
            await tester.sendKeyEvent(LogicalKeyboardKey.enter);
            typed.enter();
          case 3 || 4:
            await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
            typed.backspace();
          case 5:
            final before = focused().textEditingValue;
            final at = before.selection.baseOffset;
            input(
              inserted(
                'ni',
                composing: TextRange(start: at, end: at + 2),
              ),
            );
            await tester.pump(Duration(milliseconds: random.nextInt(15)));
            input(
              TextEditingValue(
                text: before.text.replaceRange(at, at, '你'),
                selection: TextSelection.collapsed(offset: at + 1),
              ),
            );
            typed.insert('你');
          case 6:
            input(inserted('é'));
            typed.insert('é');
          default:
            final letter = String.fromCharCode(97 + random.nextInt(26));
            input(inserted(letter));
            typed.insert(letter);
        }
      }
      if (end == _End.leave) await tester.tap(find.byTooltip('Settings'));
      if (end == _End.quit) {
        var drained = false;
        kit.workspace.drain().then((_) => drained = true);
        for (var wait = 0; wait < 200 && !drained; wait++) {
          await tester.pump(const Duration(milliseconds: 20));
        }
        expect(drained, isTrue);
        expect(fake.textsOf(kit.page), typed.lines);
        return;
      }
      for (var frame = 0; frame < 300; frame++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      await tester.pumpAndSettle();
      expect(fake.textsOf(kit.page), typed.lines);
      if (end == _End.stay) {
        final field = focused();
        expect(field.textEditingValue.text, typed.lines[typed.line]);
        expect(field.textEditingValue.selection.baseOffset, typed.column);
      }
    });
  }
}
