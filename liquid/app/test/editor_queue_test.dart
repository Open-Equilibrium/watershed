import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

/// What a person typed, as plain lines with a caret: the editor must store exactly this,
/// however late the core answers (ADR-0240).
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
      lines[line] = lines[line].replaceRange(column - 1, column, '');
      column--;
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
      await tester.binding.setSurfaceSize(const Size(1400, 900));
      final fake = FakeWorkspace();
      final workspace = WorkspaceState(fake);
      await workspace.start();
      final page = workspace.open.value!.page;
      workspace.open.value = OpenPage(workspace.open.value!.area, page);
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: AgentShell(workspace: workspace),
        ),
      );
      await tester.pumpAndSettle();
      addTearDown(workspace.dispose);
      fake.delay = () => Duration(milliseconds: random.nextInt(20));

      EditableTextState focused() => tester.state<EditableTextState>(
        find.byWidgetPredicate(
          (widget) => widget is EditableText && widget.focusNode.hasFocus,
          // The first line keeps the keys out of sight until its Text Block took over.
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
            // A composition that may span a change of structure, then its result.
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
          default:
            final letter = String.fromCharCode(97 + random.nextInt(26));
            input(inserted(letter));
            typed.insert(letter);
        }
      }
      switch (end) {
        case _End.stay:
          break;
        case _End.leave:
          await tester.tap(find.byTooltip('Settings'));
        case _End.quit:
          var drained = false;
          workspace.drain().then((_) => drained = true);
          for (var wait = 0; wait < 200 && !drained; wait++) {
            await tester.pump(const Duration(milliseconds: 20));
          }
          expect(drained, isTrue);
          expect(fake.textsOf(page), typed.lines);
          return;
      }
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      expect(fake.textsOf(page), typed.lines);
      if (end == _End.stay) {
        final field = focused();
        expect(field.textEditingValue.text, typed.lines[typed.line]);
        expect(field.textEditingValue.selection.baseOffset, typed.column);
      }
    });
  }
}
