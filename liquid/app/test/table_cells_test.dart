import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_cells.dart';
import 'package:liquid/src/workspace/text_block.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  List<(String, String, BridgeCell)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.field!, set.cell!),
  ];

  final empty = BridgeCell(
    kind: BridgeCellKind.empty,
    number: 0,
    checked: false,
    files: const [],
  );
  BridgeCell text(String text) => BridgeCell(
    kind: BridgeCellKind.text,
    text: text,
    number: 0,
    checked: false,
    files: const [],
  );

  Future<void> clickCell(WidgetTester tester, String row, String field) async {
    await tester.tap(cell(row, field));
    await tester.pumpAndSettle();
  }

  List<BridgeRow> namedRows(int from, int to) => [
    for (var number = from; number <= to; number++) namedRow(number),
  ];

  Finder editor() => kit.inTable(
    find.descendant(
      of: find.byKey(const Key('cell editor')),
      matching: find.byType(TextField),
    ),
  );

  testWidgets('a click selects a cell and ⌫ clears it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written(), [('r1', 'name', empty)]);
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
      find.descendant(of: cell('r2', 'name'), matching: editor()),
      findsOneWidget,
    );
  });

  testWidgets('⏎ keeps a text edit, and esc undoes one', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
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
      await clickCell(tester, 'r1', 'amount');
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
    },
  );

  testWidgets('a checkbox ticks on a click and on Space', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r2', 'paid');
    await key(tester, LogicalKeyboardKey.space);
    expect(written().map((write) => write.$3.checked), [true, false]);
  });

  testWidgets('esc leaves the table', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.escape);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written(), isEmpty);
  });

  testWidgets('an edit goes to its cell as rows change', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), 'Power');
    kit.fake.putRows(kit.page, table, [kit.fake.rows[table]![1]]);
    await tester.pumpAndSettle();
    if (editor().evaluate().isNotEmpty) {
      await key(tester, LogicalKeyboardKey.enter);
    }
    expect(written(), [('r2', 'name', text('Power'))]);
  });

  testWidgets('an open edit is kept on leaving or quitting', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), 'Market');
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(written(), [('r1', 'name', text('Market'))]);

    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
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
    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), 'Power');
    final rows = kit.fake.rows[table]!;
    kit.fake.putRows(kit.page, table, [rows[0], rows[2]]);
    await tester.pumpAndSettle();
    expect(editor(), findsNothing);
    expect(kit.inTable(find.text('Row 3')), findsOneWidget);
    expect(written(), [('r2', 'name', text('Power'))]);
    expect(kit.workspace.problem.value, isNotNull);
  });

  testWidgets(
    'a click in the edited cell keeps the edit',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await clickCell(tester, 'r1', 'name');
      await key(tester, LogicalKeyboardKey.enter);
      await tester.enterText(editor(), 'Market');
      await tester.tapAt(
        tester.getRect(cell('r1', 'name')).bottomRight - const Offset(4, 4),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(editor()).controller!.text, 'Market');
      expect(written(), isEmpty);
    },
  );

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
            ),
        ],
      ),
    ]);
    await clickCell(tester, 'r1', 'amount');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), '7');
    final sideways = tester
        .widget<SingleChildScrollView>(
          kit.inTable(find.byType(SingleChildScrollView)),
        )
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

  testWidgets('an edit of the last row ends when removed', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
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

  testWidgets('an edit whose row moves out of what was read is still written', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
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

  testWidgets('an edit keeps taking keys after its row moves', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await clickCell(tester, 'r2', 'name');
    await key(tester, LogicalKeyboardKey.enter);
    kit.fake.putRows(kit.page, table, [kit.fake.rows[table]![1]]);
    await tester.pumpAndSettle();
    tester.testTextInput.enterText('Power');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r2', 'name', text('Power'))]);
  });

  testWidgets('quick clicks and a held Space tick once each', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.database};
    await tester.tap(cell('r2', 'paid'));
    await tester.pump();
    await tester.tap(cell('r2', 'paid'));
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
    final rows = tester
        .widget<ListView>(find.byKey(const Key('table rows')))
        .controller!;
    rows.jumpTo(rows.position.maxScrollExtent);
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    expect(editor(), findsOneWidget);
    tester.testTextInput.enterText('Second');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r2', 'name', text('Second'))]);
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

  testWidgets('a number that is none says so as its row moves', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await clickCell(tester, 'r2', 'amount');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), '-');
    final rows = kit.fake.rows[table]!;
    kit.fake.putRows(kit.page, table, [
      rows[0],
      ...namedRows(10, 159),
      rows[1],
    ]);
    await tester.pumpAndSettle();
    expect(written(), isEmpty);
    expect(kit.workspace.problem.value, contains('“-” is no number'));
  });

  testWidgets('a number that is none says so on leaving', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await clickCell(tester, 'r1', 'amount');
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(editor(), '12x');
    await kit.workspace.newPage(kit.workspace.open.value!.area);
    await tester.pumpAndSettle();
    expect(written(), isEmpty);
    expect(kit.workspace.problem.value, contains('“12x” is no number'));
  });

  testWidgets('esc puts the caret at the start of the Text Block below', (
    tester,
  ) async {
    await kit.show(tester, [
      TablePart(expenses),
      const TextPart('Paid by card.'),
    ]);
    await clickCell(tester, 'r1', 'name');
    await key(tester, LogicalKeyboardKey.escape);
    final below = tester.widget<TextField>(
      find.descendant(
        of: find.byType(TextBlock),
        matching: find.byType(TextField),
      ),
    );
    expect(below.focusNode!.hasFocus, isTrue);
    expect(below.controller!.selection.baseOffset, 0);
  });

  testWidgets('the arrows bring the selected cell into view', (tester) async {
    await kit.show(tester, [TablePart(namedRows(1, 30))]);
    await clickCell(tester, 'r1', 'name');
    for (var step = 0; step < 15; step++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    }
    await tester.pumpAndSettle();
    final rows = tester
        .widget<ListView>(find.byKey(const Key('table rows')))
        .controller!;
    expect(rows.offset, (16 - 10) * 34.0);
    await key(tester, LogicalKeyboardKey.backspace);
    expect(written().single.$1, 'r16');
  });

  testWidgets('an empty checkbox cell shows an unticked box', (tester) async {
    await kit.show(tester, [
      TablePart([...expenses, namedRow(3)]),
    ]);
    await clickCell(tester, 'r1', 'paid');
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();
    for (final row in ['r1', 'r3']) {
      final box = find.descendant(
        of: cell(row, 'paid'),
        matching: find.byType(CellCheckbox),
      );
      expect(box, findsOneWidget, reason: row);
      expect(tester.widget<CellCheckbox>(box).checked, isFalse, reason: row);
    }
  });
}
