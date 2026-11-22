import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
    namedRow(4),
  ];

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Future<void> groupBy(WidgetTester tester, String field) async {
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabGroup,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        field: field,
      ),
    );
    await tester.pumpAndSettle();
  }

  List<String> shownTop(WidgetTester tester, List<String> texts) {
    final tops = {
      for (final text in texts)
        if (kit.inTable(find.text(text)).evaluate().isNotEmpty)
          text: tester.getTopLeft(kit.inTable(find.text(text)).first).dy,
    };
    return tops.keys.toList()..sort((a, b) => tops[a]!.compareTo(tops[b]!));
  }

  testWidgets('a grouped Table shows its rows under each option, the empty '
      'group first', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await groupBy(tester, 'category');
    expect(find.byKey(const Key('group header')), findsNWidgets(3));
    expect(
      shownTop(tester, [
        'No Category',
        'Row 4',
        'Food',
        'Groceries',
        'Home',
        'Electricity',
        'Rent',
      ]),
      [
        'No Category',
        'Row 4',
        'Food',
        'Groceries',
        'Home',
        'Electricity',
        'Rent',
      ],
    );
    expect(kit.inTable(find.text('4 rows')), findsOneWidget);
  });

  testWidgets('folding a group hides its rows for everyone', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await groupBy(tester, 'category');
    await tester.tap(
      find.descendant(
        of: find.byKey(const Key('group header')).at(2),
        matching: find.byKey(const Key('fold group')),
      ),
    );
    await tester.pumpAndSettle();
    final folded = view().tabs.first.folded;
    expect([for (final cell in folded) cell.text], ['home']);
    expect(kit.inTable(find.text('Rent')), findsNothing);
    expect(kit.inTable(find.text('Groceries')), findsOneWidget);
    expect(kit.inTable(find.text('4 rows')), findsOneWidget);
  });

  testWidgets('a Checkbox groups into unticked, then ticked', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await groupBy(tester, 'paid');
    expect(shownTop(tester, ['Electricity', 'Row 4', 'Groceries', 'Rent']), [
      'Electricity',
      'Row 4',
      'Groceries',
      'Rent',
    ]);
  });
}
