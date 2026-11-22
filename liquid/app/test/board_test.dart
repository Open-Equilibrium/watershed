import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/cell_pickers.dart';
import 'package:liquid/src/workspace/table_cells.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
  ];

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Future<void> board(WidgetTester tester, List<BridgeRow> rows) async {
    await kit.show(tester, [TablePart(rows)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabLayout,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        layout: BridgeLayout.board,
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder column(String name) => find.ancestor(
    of: find.descendant(
      of: find.byKey(const Key('column header')),
      matching: find.text(name),
    ),
    matching: find.byKey(const Key('board column')),
  );

  Finder card(String name) => find.ancestor(
    of: find.text(name),
    matching: find.byKey(const Key('board card')),
  );

  List<BridgeMutation> cellsSet() =>
      kit.applied(BridgeMutationKind.setCell).toList();

  testWidgets('a Board shows a column per option, and the empty one once it '
      'holds a card', (tester) async {
    await board(tester, rows);
    expect(find.byKey(const Key('board column')), findsNWidgets(2));
    expect(
      find.descendant(of: column('Home'), matching: find.text('Rent')),
      findsOneWidget,
    );
    expect(find.text('No Category'), findsNothing);
    expect(find.byKey(const Key('table group')), findsOneWidget);
    kit.fake.putRows(kit.page, view().block, [...rows, namedRow(4)]);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('board column')), findsNWidgets(3));
    final empty = tester.getTopLeft(column('No Category')).dx;
    expect(empty, lessThan(tester.getTopLeft(column('Food')).dx));
  });

  testWidgets('a card shows its first field and the other filled fields, '
      'but not the grouped one', (tester) async {
    await board(tester, rows);
    final groceries = card('Groceries');
    expect(
      find.descendant(of: groceries, matching: find.text('84.2')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: groceries, matching: find.text('Food')),
      findsNothing,
    );
  });

  testWidgets('dragging a card to another column sets its value', (
    tester,
  ) async {
    await board(tester, rows);
    await tester.timedDrag(
      card('Groceries'),
      tester.getCenter(column('Home')) - tester.getCenter(card('Groceries')),
      const Duration(milliseconds: 400),
    );
    await tester.pumpAndSettle();
    final set = cellsSet().single;
    expect((set.row, set.field, set.cell?.text), ('r1', 'category', 'home'));
  });

  testWidgets('a card dropped above another in its column moves its row', (
    tester,
  ) async {
    await board(tester, rows);
    await tester.timedDrag(
      card('Rent'),
      tester.getCenter(card('Electricity')) -
          tester.getCenter(card('Rent')) -
          const Offset(0, 10),
      const Duration(milliseconds: 400),
    );
    await tester.pumpAndSettle();
    final moved = kit.applied(BridgeMutationKind.moveRow).single;
    expect((moved.row, moved.after, moved.last), ('r3', null, false));
    expect(cellsSet(), isEmpty);
  });

  testWidgets('New at a column’s foot adds a row with that value', (
    tester,
  ) async {
    await board(tester, rows);
    await tap(
      tester,
      find.descendant(of: column('Food'), matching: find.text('New')),
    );
    final added = kit.applied(BridgeMutationKind.addRow).single;
    final set = cellsSet().single;
    expect(
      (set.row, set.field, set.cell?.text),
      (kit.fake.rows[view().block]!.last.id, 'category', 'food'),
    );
    expect(added.last, isTrue);
  });

  testWidgets('a card opens its row beside it; esc closes it', (tester) async {
    await board(tester, rows);
    await tap(tester, card('Groceries'));
    final popover = find.byKey(const Key('row popover'));
    expect(popover, findsOneWidget);
    expect(
      find.descendant(of: popover, matching: find.text('Amount')),
      findsOneWidget,
    );
    await tester.enterText(
      find.descendant(of: popover, matching: find.byType(TextField)).first,
      'Food market',
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    final set = cellsSet().single;
    expect((set.row, set.field, set.cell?.text), ('r1', 'name', 'Food market'));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(popover, findsNothing);
  });

  testWidgets('a row’s ··· deletes it', (tester) async {
    await board(tester, rows);
    await tap(tester, card('Rent'));
    await tap(tester, find.byKey(const Key('row popover actions')));
    await tap(tester, find.text('Delete row'));
    expect(kit.applied(BridgeMutationKind.removeRow).single.row, 'r3');
  });

  testWidgets('a row’s fields change in its popover with the Table’s '
      'pickers', (tester) async {
    await board(tester, rows);
    await tap(tester, card('Electricity'));
    final popover = find.byKey(const Key('row popover'));
    await tap(tester, find.byKey(const Key('pick category')));
    await tap(tester, find.text('Food').last);
    await tap(tester, find.byKey(const Key('pick date')));
    await tester.enterText(
      find.descendant(
        of: find.byType(DatePicker),
        matching: find.byType(TextField),
      ),
      '2026-11-15',
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    await tap(
      tester,
      find.descendant(
        of: popover,
        matching: find.byWidgetPredicate(
          (widget) =>
              widget is CellView && widget.kind == BridgeFieldKind.checkbox,
        ),
      ),
    );
    final amount = find
        .descendant(of: popover, matching: find.byType(TextField))
        .at(1);
    await tester.enterText(amount, '120');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(
      [
        for (final set in cellsSet())
          (
            set.field,
            switch (set.cell?.kind) {
              BridgeCellKind.checkbox => set.cell!.checked,
              BridgeCellKind.number => set.cell!.number,
              _ => set.cell?.text,
            },
          ),
      ],
      [
        ('category', 'food'),
        ('date', '2026-11-15'),
        ('paid', true),
        ('amount', 120.0),
      ],
    );
  });

  testWidgets('a Board without a Select field offers to add one', (
    tester,
  ) async {
    await kit.show(tester, [
      TablePart(const [], fields: [expenseFields.first]),
    ]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabLayout,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        layout: BridgeLayout.board,
      ),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.text('Add a Select field'));
    final field = kit.applied(BridgeMutationKind.addField).single;
    expect((field.text, field.fieldKind), ('Status', BridgeFieldKind.select));
    expect(view().tabs.first.group, isNotNull);
  });
}
