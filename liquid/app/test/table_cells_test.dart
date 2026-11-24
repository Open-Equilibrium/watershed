import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Finder cellOf(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));

  Finder editor() => kit.inTable(
    find.descendant(
      of: find.byKey(const Key('cell editor')),
      matching: find.byType(TextField),
    ),
  );

  List<(String, String, BridgeCell)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.field!, set.cell!),
  ];

  BridgeCell text(String text) => cell(BridgeCellKind.text, text: text);

  Future<void> clickCell(WidgetTester tester, String row, String field) async {
    await tester.tap(cellOf(row, field));
    await tester.pumpAndSettle();
  }

  Future<void> startEdit(WidgetTester tester, String row, String field) async {
    await clickCell(tester, row, field);
    await key(tester, LogicalKeyboardKey.enter);
  }

  List<BridgeRow> namedRows(int from, int to) => [
    for (var number = from; number <= to; number++) namedRow(number),
  ];

  ScrollController rowsScroll(WidgetTester tester) =>
      tester.widget<ListView>(find.byKey(const Key('table rows'))).controller!;

  testWidgets('a click selects a cell and ⌫ clears it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written(), [('r1', 'name', cell(BridgeCellKind.empty))]);
    expect(kit.inTable(find.text('Groceries')), findsNothing);
  });

  testWidgets('the arrows and ⇥ move the selection', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.arrowRight);
    await key(tester, LogicalKeyboardKey.delete);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.delete);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shift);
    await key(tester, LogicalKeyboardKey.tab);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shift);
    await key(tester, LogicalKeyboardKey.delete);
    await clickCell(tester, 'r1', 'amount');
    await key(tester, LogicalKeyboardKey.arrowRight);
    await key(tester, LogicalKeyboardKey.tab);
    expect(written().map((write) => (write.$1, write.$2)), [
      ('r1', 'date'),
      ('r2', 'date'),
      ('r2', 'name'),
    ]);
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      find.descendant(of: cellOf('r2', 'name'), matching: editor()),
      findsOneWidget,
    );
  });

  testWidgets('⏎ keeps a text edit, and esc undoes one', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await startEdit(tester, 'r1', 'name');
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

  testWidgets('a second click edits, and typing replaces it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r2', 'name');
    await clickCell(tester, 'r2', 'name');
    expect(editor(), findsOneWidget);
    await key(tester, LogicalKeyboardKey.escape);
    await key(tester, LogicalKeyboardKey.keyG);
    expect(tester.widget<TextField>(editor()).controller!.text, 'g');
    await key(tester, LogicalKeyboardKey.tab);
    expect(written(), [('r2', 'name', text('g'))]);
  });

  testWidgets(
    'a number edits in the person\'s format, and one that is none stays to be fixed',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await startEdit(tester, 'r1', 'amount');
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
    },
  );

  testWidgets('a checkbox ticks on a click and on Space', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r2', 'paid');
    await key(tester, LogicalKeyboardKey.space);
    expect(written().map((write) => write.$3.checked), [true, false]);
  });

  testWidgets('an edit goes to its row as rows change, and keeps taking keys', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await startEdit(tester, 'r2', 'name');
    await tester.enterText(editor(), 'Power');
    kit.fake.putRows(kit.page, table, [kit.fake.rows[table]![1]]);
    await tester.pumpAndSettle();
    expect(editor(), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r2', 'name', text('Power'))]);
  });

  testWidgets('an open edit is kept on leaving or quitting', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await startEdit(tester, 'r1', 'name');
    await tester.enterText(editor(), 'Market');
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(written(), [('r1', 'name', text('Market'))]);

    await startEdit(tester, 'r2', 'name');
    await tester.enterText(editor(), 'Power');
    await kit.workspace.newPage(kit.workspace.open.value!.area);
    await tester.pumpAndSettle();
    expect(written().last, ('r2', 'name', text('Power')));
  });

  testWidgets('keys typed before the editor still count', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyB);
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(editor()).controller!.text, 'ab');
  });

  testWidgets('an edit of a removed row ends with a notice', (tester) async {
    await kit.show(tester, [
      TablePart([...expenses, namedRow(3)]),
    ]);
    final table = kit.fake.views[kit.page]!.single.block;
    await startEdit(tester, 'r2', 'name');
    await tester.enterText(editor(), 'Power');
    final rows = kit.fake.rows[table]!;
    kit.fake.putRows(kit.page, table, [rows[0], rows[2]]);
    await tester.pumpAndSettle();
    expect(editor(), findsNothing);
    expect(kit.inTable(find.text('Row 3')), findsOneWidget);
    expect(written(), [('r2', 'name', text('Power'))]);
    expect(kit.workspace.problem.value, isNotNull);
  });

  testWidgets('scrolling sideways keeps an edit in view', (tester) async {
    await kit.show(tester, [
      TablePart(
        expenses,
        fields: [
          ...expenseFields,
          for (var number = 1; number < 6; number++)
            BridgeField(
              id: 'f$number',
              name: 'Field $number',
              kind: BridgeFieldKind.text,
              options: const [],
              formula: '',
            ),
        ],
      ),
    ]);
    await startEdit(tester, 'r1', 'amount');
    await tester.enterText(editor(), '7');
    final sideways = tester
        .widget<SingleChildScrollView>(find.byKey(const Key('table columns')))
        .controller!;
    sideways.jumpTo(150);
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(editor()).controller!.text, '7');
    expect(tester.widget<TextField>(editor()).focusNode!.hasFocus, isTrue);
    expect(written(), isEmpty);
    tester.testTextInput.enterText('78');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written().single.$3.number, 78);
  });

  testWidgets('an edit whose row moves out of what was read is still written', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await startEdit(tester, 'r2', 'name');
    await tester.enterText(editor(), 'Power');
    final rows = kit.fake.rows[table]!;
    kit.fake.putRows(kit.page, table, [
      rows[0],
      ...namedRows(10, 159),
      rows[1],
    ]);
    await tester.pumpAndSettle();
    expect(written(), [('r2', 'name', text('Power'))]);
    expect(kit.workspace.problem.value, isNull);
  });

  testWidgets('quick clicks and a held Space tick once each', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.database};
    await tester.tap(cellOf('r2', 'paid'));
    await tester.pump();
    await tester.tap(cellOf('r2', 'paid'));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.space);
    await tester.pump();
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(written().map((write) => write.$3.checked), [true, false, true]);
  });

  testWidgets('an edit of a scrolled-out cell brings it back', (tester) async {
    await kit.show(tester, [TablePart(namedRows(1, 30))]);
    await clickCell(tester, 'r2', 'name');
    final rows = rowsScroll(tester);
    rows.jumpTo(rows.position.maxScrollExtent);
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    expect(editor(), findsOneWidget);
    tester.testTextInput.enterText('Second');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r2', 'name', text('Second'))]);
  });

  testWidgets('an edit of the last row ends when removed', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await startEdit(tester, 'r2', 'name');
    await tester.enterText(editor(), 'Power');
    kit.fake.putRows(kit.page, table, [kit.fake.rows[table]!.first]);
    await tester.pumpAndSettle();
    expect(editor(), findsNothing);
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.backspace);
    expect(
      written().where((write) => write.$1 != 'r2'),
      isEmpty,
      reason: 'the new row comes up unselected',
    );
  });

  testWidgets(
    'a click in the edited cell keeps the edit',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await startEdit(tester, 'r1', 'name');
      await tester.enterText(editor(), 'Market');
      await tester.tapAt(
        tester.getRect(cellOf('r1', 'name')).bottomRight - const Offset(4, 4),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(editor()).controller!.text, 'Market');
      expect(written(), isEmpty);
    },
  );

  testWidgets('a number that is none says so on leaving', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await startEdit(tester, 'r1', 'amount');
    await tester.enterText(editor(), '12x');
    await kit.workspace.newPage(kit.workspace.open.value!.area);
    await tester.pumpAndSettle();
    expect(written(), isEmpty);
    expect(kit.workspace.problem.value, contains('“12x” is no number'));
  });

  testWidgets('a refused tick leaves the next click ticking', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    kit.fake.refuseKinds = {BridgeMutationKind.setCell};
    await clickCell(tester, 'r2', 'paid');
    expect(kit.workspace.problem.value, isNotNull);
    kit.fake.refuseKinds = {};
    await clickCell(tester, 'r2', 'paid');
    expect(written().single.$3.checked, isTrue);
  });
}
