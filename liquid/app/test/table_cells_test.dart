import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'table_kit.dart';

void main() {
  final kit = TableKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  /// The cells written, as row, field and value.
  List<(String, String, BridgeCell)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.field!, set.cell!),
  ];

  final empty = BridgeCell(
    kind: BridgeCellKind.empty,
    number: 0,
    checked: false,
  );
  BridgeCell text(String text) => BridgeCell(
    kind: BridgeCellKind.text,
    text: text,
    number: 0,
    checked: false,
  );

  Finder editor() => kit.inTable(find.byKey(const Key('cell editor')));

  testWidgets('a click selects a cell and ⌫ clears it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'name'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written(), [('r1', 'name', empty)]);
    expect(kit.inTable(find.text('Groceries')), findsNothing);
  });

  testWidgets('the arrows and ⇥ move the selection', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'name'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.arrowRight);
    await key(tester, LogicalKeyboardKey.delete);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.delete);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shift);
    await key(tester, LogicalKeyboardKey.tab);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shift);
    await key(tester, LogicalKeyboardKey.delete);
    // ⇥ after a row's last field goes on with the next row.
    await tester.tap(cell('r1', 'amount'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.arrowRight);
    await key(tester, LogicalKeyboardKey.tab);
    expect(written().map((write) => (write.$1, write.$2)), [
      ('r1', 'date'),
      ('r2', 'date'),
      ('r2', 'name'),
    ]);
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      find.descendant(of: cell('r2', 'name'), matching: editor()),
      findsOneWidget,
    );
  });

  testWidgets('⏎ edits a text cell; ⏎ keeps the edit, esc undoes it', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'name'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    expect(editor(), findsOneWidget);
    expect(tester.widget<TextField>(editor()).controller!.text, 'Groceries');
    await tester.enterText(editor(), 'Market');
    await key(tester, LogicalKeyboardKey.enter);
    expect(editor(), findsNothing);
    expect(written(), [('r1', 'name', text('Market'))]);
    expect(kit.inTable(find.text('Market')), findsOneWidget);

    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), 'Bakery');
    await key(tester, LogicalKeyboardKey.escape);
    expect(editor(), findsNothing);
    expect(written(), hasLength(1));
    expect(kit.inTable(find.text('Market')), findsOneWidget);
  });

  testWidgets('a second click edits, and typing replaces the value', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r2', 'name'));
    await tester.pumpAndSettle();
    await tester.tap(cell('r2', 'name'));
    await tester.pumpAndSettle();
    expect(editor(), findsOneWidget);
    await key(tester, LogicalKeyboardKey.escape);
    await key(tester, LogicalKeyboardKey.keyG);
    expect(tester.widget<TextField>(editor()).controller!.text, 'g');
    await key(tester, LogicalKeyboardKey.tab);
    expect(written(), [('r2', 'name', text('g'))]);
  });

  testWidgets('a number edits in the person\'s format, and one that is none '
      'stays to be fixed', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'amount'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    expect(tester.widget<TextField>(editor()).controller!.text, '84.2');
    await tester.enterText(editor(), '42.5x');
    await key(tester, LogicalKeyboardKey.enter);
    expect(editor(), findsOneWidget);
    expect(kit.inTable(find.text('“42.5x” is no number.')), findsOneWidget);
    expect(written(), isEmpty);
    await tester.enterText(editor(), '1,042.5');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written().single.$3.number, 1042.5);
    expect(written().single.$3.kind, BridgeCellKind.number);
  });

  testWidgets('a checkbox ticks on a click and on Space', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r2', 'paid'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.space);
    expect(written().map((write) => write.$3.checked), [true, false]);
  });

  testWidgets('esc leaves the table', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'name'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.escape);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written(), isEmpty);
  });
}
