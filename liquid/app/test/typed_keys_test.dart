import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'table_kit.dart';

/// On desktop, Flutter selects a single-line field's text when it takes focus; a key typed
/// onto a cell reaches the field before it does, and what the person types next goes in
/// where the field's selection is, as macOS text input puts it.
void main() {
  final kit = TableKit();
  final macOS = TargetPlatformVariant.only(TargetPlatform.macOS);

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));

  /// The focused field, as macOS text input sees it.
  EditableTextState focused(WidgetTester tester) => tester.state(
    find.byWidgetPredicate(
      (widget) => widget is EditableText && widget.focusNode.hasFocus,
    ),
  );

  /// Types the first character onto the selected cell, lets its field take the keys, and
  /// types the rest where the field's selection is.
  Future<void> type(
    WidgetTester tester,
    LogicalKeyboardKey first,
    String rest,
  ) async {
    await tester.sendKeyEvent(first);
    await tester.pumpAndSettle();
    final field = focused(tester);
    final value = field.textEditingValue;
    field.userUpdateTextEditingValue(
      value.replaced(value.selection, rest),
      SelectionChangedCause.keyboard,
    );
    await tester.pumpAndSettle();
  }

  Future<void> select(WidgetTester tester, String row, String field) async {
    await tester.tap(cell(row, field));
    await tester.pumpAndSettle();
  }

  List<(String?, String?, BridgeCell?)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row, set.field, set.cell),
  ];

  testWidgets(
    'typing onto a number or text cell keeps every key',
    variant: macOS,
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await select(tester, 'r1', 'amount');
      await type(tester, LogicalKeyboardKey.digit4, '2.5');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      await select(tester, 'r2', 'name');
      await type(tester, LogicalKeyboardKey.keyP, 'ower');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(
        [
          for (final (row, _, cell) in written())
            (row, cell?.number, cell?.text),
        ],
        [('r1', 42.5, null), ('r2', 0.0, 'power')],
      );
    },
  );

  testWidgets(
    'typing onto a select or date cell keeps every key in its picker',
    variant: macOS,
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await select(tester, 'r1', 'category');
      await type(tester, LogicalKeyboardKey.keyH, 'om');
      expect(focused(tester).textEditingValue.text, 'hom');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await select(tester, 'r1', 'date');
      await type(tester, LogicalKeyboardKey.digit1, '0/15/2026');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(written().single.$3?.text, '2026-10-15');
    },
  );

  testWidgets(
    'a field menu and a date picker open with the caret after their text',
    variant: macOS,
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await tester.tap(kit.inTable(find.text('Category')));
      await tester.pumpAndSettle();
      expect(
        focused(tester).textEditingValue.selection,
        const TextSelection.collapsed(offset: 'Category'.length),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await select(tester, 'r1', 'date');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(
        focused(tester).textEditingValue.selection,
        const TextSelection.collapsed(offset: 'Nov 1, 2026'.length),
      );
    },
  );
}
