import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
  ];

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

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

  testWidgets('the note names the Table and opens it', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    final note = find.byKey(const Key('table note'));
    expect(
      find.descendant(of: note, matching: find.byType(RichText)),
      findsOneWidget,
    );
    expect(
      tester
          .widget<RichText>(
            find.descendant(of: note, matching: find.byType(RichText)),
          )
          .text
          .toPlainText(),
      'Table: Expenses',
    );
    await tap(tester, note);
    expect(kit.workspace.open.value?.table, view().block);
  });

  testWidgets('a click on a column sorts it, then reverses, then clears', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    final amount = kit.inTable(find.text('Amount'));
    await tap(tester, amount);
    expect(view().sort, const BridgeSort(field: 'amount', descending: false));
    expect(names(tester), ['Groceries', 'Electricity', 'Rent']);
    expect(find.byKey(const Key('column sorted')), findsOneWidget);
    await tap(tester, amount);
    expect(view().sort?.descending, isTrue);
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    await tap(tester, amount);
    expect(view().sort, isNull);
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
    expect(view().filter, isNull);
    expect(
      kit.fake.applied.where(
        (applied) => applied.kind == BridgeMutationKind.setViewFilter,
      ),
      isEmpty,
    );
  });

  Future<void> deleteView(WidgetTester tester) async {
    await tap(tester, find.byKey(const Key('View actions')));
    await tap(tester, find.text('Delete View'));
  }

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
    expect(
      kit.fake.views[kit.page]!.any((shown) => shown.block == table),
      false,
    );
    expect(kit.fake.tables.map((table) => table.id), [table]);
  });

  for (final deleteTable in [false, true]) {
    testWidgets('the last View asks first, then deletes '
        '${deleteTable ? 'the Table too' : 'only the View'}', (tester) async {
      await kit.show(tester, [TablePart(rows)]);
      final table = view().block;
      await deleteView(tester);
      expect(find.text('Delete the last View of “Expenses”?'), findsOneWidget);
      expect(
        find.textContaining('The Table and its 3 rows stay in Tables'),
        findsOneWidget,
      );
      await tap(
        tester,
        find.text(deleteTable ? 'Delete Table too' : 'Delete View'),
      );
      expect(
        kit.fake.views[kit.page]!.where((shown) => shown.block == table),
        isEmpty,
      );
      expect(
        kit.fake.tables.map((table) => table.id),
        deleteTable ? isEmpty : [table],
      );
    });
  }

  testWidgets('cancel keeps the last View', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await deleteView(tester);
    await tap(tester, find.text('Cancel'));
    expect(
      kit.fake.views[kit.page]!.where((shown) => shown.block == view().block),
      hasLength(1),
    );
  });

  testWidgets('a View whose Table was deleted says so', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    final table = view().block;
    kit.fake.dropTable(table);
    await tester.pumpAndSettle();
    expect(find.text('This Table was deleted.'), findsOneWidget);
  });
}
