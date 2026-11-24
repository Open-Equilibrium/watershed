import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
  ];

  final sortButton = find.byKey(const Key('table sort'));
  final filterButton = find.byKey(const Key('table filter'));
  final filterField = find.byKey(const Key('filter field'));
  final filterValue = find.byKey(const Key('filter value'));
  final filterMatch = find.byKey(const Key('filter match'));

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Finder menuText(String text) => find.descendant(
    of: find.byKey(const Key('table query menu')),
    matching: find.text(text),
  );

  List<String> names(WidgetTester tester) {
    final shown = {
      for (final name in ['Groceries', 'Electricity', 'Rent'])
        if (find.text(name).evaluate().isNotEmpty)
          name: tester.getTopLeft(find.text(name).first).dy,
    };
    return shown.keys.toList()..sort((a, b) => shown[a]!.compareTo(shown[b]!));
  }

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Finder shows(String control, String text) =>
      find.descendant(of: find.byKey(Key(control)), matching: find.text(text));

  Future<void> typeValue(WidgetTester tester, String text) async {
    await tester.enterText(filterValue, text);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
  }

  Future<void> filterBy(WidgetTester tester, String field, String value) async {
    await tap(tester, filterButton);
    await tap(tester, filterField);
    await tap(tester, menuText(field));
    await tap(tester, menuText(value));
  }

  Future<void> filterHome(WidgetTester tester) async {
    await filterBy(tester, 'Category', 'Home');
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();
  }

  Future<void> sortElsewhere(WidgetTester tester) async {
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabSort,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        sort: const BridgeSort(field: 'amount', descending: true),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('a sort is kept on the View and orders the rows', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, sortButton);
    await tap(tester, menuText('Amount'));
    expect(
      view().tabs.first.sort,
      const BridgeSort(field: 'amount', descending: false),
    );
    expect(names(tester), ['Groceries', 'Electricity', 'Rent']);
    await tap(tester, menuText('Descending'));
    expect(view().tabs.first.sort?.descending, isTrue);
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    expect(shows('table sort', 'Amount, descending'), findsOneWidget);
    await tap(tester, menuText('Remove the sort'));
    expect(view().tabs.first.sort, isNull);
    expect(shows('table sort', 'Sort'), findsOneWidget);
  });

  testWidgets('a filter keeps its rows and says how many pass', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await filterBy(tester, 'Category', 'Home');
    expect(view().tabs.first.filter?.field, 'category');
    expect(view().tabs.first.filter?.matches, BridgeMatch.is_);
    expect(view().tabs.first.filter?.value?.text, 'home');
    expect(names(tester), ['Electricity', 'Rent']);
    expect(find.text('2 of 3 rows'), findsOneWidget);
    expect(find.text('Category is Home'), findsOneWidget);
    await tap(tester, menuText('Remove the filter'));
    expect(view().tabs.first.filter, isNull);
    expect(find.text('3 rows'), findsOneWidget);
  });

  testWidgets('text is matched as typed, and written on ⏎', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    expect(
      tester.widget<TextField>(filterValue).focusNode!.hasFocus,
      isTrue,
      reason: 'what is typed goes to the value as the menu opens',
    );
    await typeValue(tester, 'ren');
    expect(view().tabs.first.filter?.matches, BridgeMatch.contains);
    expect(names(tester), ['Rent']);
    expect(find.text('Name contains ren'), findsOneWidget);
  });

  testWidgets('esc closes a filter menu after its value is written on ⏎', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    await typeValue(tester, 'ren');
    expect(names(tester), ['Rent']);
    await key(tester, LogicalKeyboardKey.escape);
    expect(find.byKey(const Key('table query menu')), findsNothing);
    expect(names(tester), ['Rent']);
  });

  testWidgets('a number that is none is not written', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    await tap(tester, filterField);
    await tap(tester, menuText('Amount'));
    await typeValue(tester, 'abc');
    expect(view().tabs.first.filter, isNull);
    await typeValue(tester, '96');
    expect(names(tester), ['Electricity']);
  });

  testWidgets('empty needs no value', (tester) async {
    await kit.show(tester, [
      TablePart([...rows, namedRow(4)]),
    ]);
    await tap(tester, filterButton);
    await tap(tester, filterField);
    await tap(tester, menuText('Amount'));
    await tap(tester, filterMatch);
    await tap(tester, menuText('is empty'));
    expect(view().tabs.first.filter?.matches, BridgeMatch.empty);
    expect(find.text('Row 4'), findsOneWidget);
    expect(find.text('1 of 4 rows'), findsOneWidget);
  });

  testWidgets('a sort set elsewhere shows here, and Sort reopens until esc', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await sortElsewhere(tester);
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    expect(shows('table sort', 'Amount, descending'), findsOneWidget);
    await tap(tester, sortButton);
    expect(menuText('Remove the sort'), findsOneWidget);
    await key(tester, LogicalKeyboardKey.escape);
    expect(menuText('Remove the sort'), findsNothing);
  });

  testWidgets('a new row takes the value the filter asks for', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await filterHome(tester);
    await tap(tester, find.text('New'));
    expect(find.text('3 of 4 rows'), findsOneWidget);
  });

  testWidgets('a new row is not given the value a not-is filter excludes', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabFilter,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        filter: BridgeFilter(
          field: 'category',
          matches: BridgeMatch.isNot,
          value: cell(BridgeCellKind.select, text: 'home'),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.text('New'));
    expect(find.text('2 of 4 rows'), findsOneWidget);
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
  });

  testWidgets('a filtered Table keeps its place when the Page changes', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('October'),
      TablePart([
        for (var at = 0; at < 40; at++)
          expenseRow('r$at', 'Row $at', '2026-11-01', 'home', 1, true),
      ]),
    ]);
    await filterHome(tester);
    final scroll = find.descendant(
      of: find.byType(TableBlock),
      matching: find.byWidgetPredicate(
        (widget) =>
            widget is Scrollable && widget.axisDirection == AxisDirection.down,
      ),
    );
    final rowsScroll = tester.state<ScrollableState>(scroll.first).position;
    rowsScroll.jumpTo(120);
    await tester.pumpAndSettle();
    await kit.workspace.apply(
      mutation(BridgeMutationKind.addTextBlock, page: kit.page, text: 'Above'),
    );
    await tester.pumpAndSettle();
    expect(rowsScroll.pixels, 120);
  });

  testWidgets('an emptied value removes the filter and refuses nothing', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.textContaining('This is not'), findsNothing);
    await typeValue(tester, 'ren');
    expect(view().tabs.first.filter, isNotNull);
    await typeValue(tester, '');
    expect(view().tabs.first.filter, isNull);
  });

  testWidgets('deleting a field counts every row, not those shown', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await filterHome(tester);
    await tester.tap(
      kit.inTable(find.text('Amount')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    await tap(tester, find.text('Delete field'));
    expect(find.textContaining('all 3 rows'), findsOneWidget);
  });

  testWidgets('leaving a change unfinished keeps the filter', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    await tap(tester, filterMatch);
    await tap(tester, menuText('is empty'));
    expect(view().tabs.first.filter?.matches, BridgeMatch.empty);
    await tap(tester, filterMatch);
    await tap(tester, menuText('contains'));
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();
    expect(view().tabs.first.filter?.matches, BridgeMatch.empty);
  });

  testWidgets(
    'esc closes the menu, with a choice open or after its value field went',
    (tester) async {
      await kit.show(tester, [TablePart(rows)]);
      await tap(tester, filterButton);
      await tap(tester, filterField);
      await tap(tester, menuText('Category'));
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('table query menu')), findsNothing);
      await tap(tester, filterButton);
      await tap(tester, filterField);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('table query menu')), findsNothing);
    },
  );

  testWidgets('closing a choice gives the keys back to the value', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, filterButton);
    await tap(tester, filterMatch);
    await tap(tester, filterMatch);
    expect(tester.widget<TextField>(filterValue).focusNode!.hasFocus, isTrue);
  });
}
