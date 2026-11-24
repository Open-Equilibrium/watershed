import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
  ];

  List<String> names(WidgetTester tester) {
    final shown = {
      for (final name in ['Groceries', 'Electricity', 'Rent'])
        if (kit.inTable(find.text(name)).evaluate().isNotEmpty)
          name: tester.getTopLeft(kit.inTable(find.text(name)).first).dy,
    };
    return shown.keys.toList()..sort((a, b) => shown[a]!.compareTo(shown[b]!));
  }

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Future<void> deleteView(WidgetTester tester) async {
    await tap(tester, find.byKey(const Key('View actions')));
    await tap(tester, find.text('Delete View'));
  }

  List<BridgeView> viewsOf(String block) =>
      kit.fake.views[kit.page]!.where((shown) => shown.block == block).toList();

  testWidgets('a click on a column sorts it, then reverses, then clears', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    final amount = kit.inTable(find.text('Amount'));
    await tap(tester, amount);
    expect(
      view().tabs.first.sort,
      const BridgeSort(field: 'amount', descending: false),
    );
    expect(names(tester), ['Groceries', 'Electricity', 'Rent']);
    expect(find.byKey(const Key('column sorted')), findsOneWidget);
    await tap(tester, amount);
    expect(view().tabs.first.sort?.descending, isTrue);
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    await tap(tester, amount);
    expect(view().tabs.first.sort, isNull);
    expect(find.byKey(const Key('column sorted')), findsNothing);
  });

  testWidgets('search narrows the rows for now and saves nothing', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table search')));
    await tester.enterText(find.byKey(const Key('table search field')), 'hom');
    await tester.pump(const Duration(milliseconds: 200));
    await tester.pumpAndSettle();
    expect(names(tester), ['Electricity', 'Rent']);
    expect(kit.inTable(find.text('2 of 3 rows')), findsOneWidget);
    expect(view().tabs.first.filter, isNull);
    expect(kit.applied(BridgeMutationKind.setTabFilter), isEmpty);
  });

  testWidgets('deleting a View while another shows the Table asks nothing', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    final table = view().block;
    final other = await kit.workspace.apply(
      mutation(
        BridgeMutationKind.createPage,
        area: kit.workspace.areas.value.single.id,
        text: 'Review',
      ),
    );
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addTableView,
        page: other.page,
        block: table,
        text: 'All costs',
      ),
    );
    await deleteView(tester);
    expect(find.byType(Dialog), findsNothing);
    expect(viewsOf(table), isEmpty);
    expect(kit.fake.tables.map((table) => table.id), [table]);
  });

  for (final (outcome, button, viewGone, tableGone) in [
    ('deletes only the View', 'Delete View', true, false),
    ('deletes the Table too', 'Delete Table too', true, true),
    ('cancel keeps the last View', 'Cancel', false, false),
  ]) {
    testWidgets('the last View asks first, then $outcome', (tester) async {
      await kit.show(tester, [TablePart(rows)]);
      final table = view().block;
      await deleteView(tester);
      expect(find.text('Delete the last View of “Expenses”?'), findsOneWidget);
      expect(
        find.textContaining('The Table and its 3 rows stay in Tables'),
        findsOneWidget,
      );
      await tap(tester, find.text(button));
      expect(viewsOf(table), viewGone ? isEmpty : hasLength(1));
      expect(
        kit.fake.tables.map((table) => table.id),
        tableGone ? isEmpty : [table],
      );
    });
  }

  testWidgets('a View whose Table was deleted says so', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    final table = view().block;
    kit.fake.dropTable(table);
    await tester.pumpAndSettle();
    expect(find.text('This Table was deleted.'), findsOneWidget);
  });
}
