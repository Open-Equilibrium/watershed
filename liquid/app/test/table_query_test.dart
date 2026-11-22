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

  Future<void> filterHome(WidgetTester tester) async {
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Category'));
    await tap(tester, menuText('Home'));
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();
  }

  testWidgets('a sort is kept on the View and orders the rows', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table sort')));
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
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Category'));
    await tap(tester, menuText('Home'));
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
    await tap(tester, find.byKey(const Key('table filter')));
    await tester.enterText(find.byKey(const Key('filter value')), 'ren');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(view().tabs.first.filter?.matches, BridgeMatch.contains);
    expect(names(tester), ['Rent']);
    expect(find.text('Name contains ren'), findsOneWidget);
  });

  testWidgets('a number that is none is not written', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Amount'));
    await tester.enterText(find.byKey(const Key('filter value')), 'abc');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(view().tabs.first.filter, isNull);
    await tester.enterText(find.byKey(const Key('filter value')), '96');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(names(tester), ['Electricity']);
  });

  testWidgets('empty needs no value', (tester) async {
    await kit.show(tester, [
      TablePart([...rows, namedRow(4)]),
    ]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Amount'));
    await tap(tester, find.byKey(const Key('filter match')));
    await tap(tester, menuText('is empty'));
    expect(view().tabs.first.filter?.matches, BridgeMatch.empty);
    expect(find.text('Row 4'), findsOneWidget);
    expect(find.text('1 of 4 rows'), findsOneWidget);
  });

  testWidgets('a sort or filter set elsewhere shows here', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
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
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    expect(shows('table sort', 'Amount, descending'), findsOneWidget);
  });

  testWidgets('Sort opens its menu again, and esc closes it', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
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
    await tap(tester, find.byKey(const Key('table sort')));
    expect(menuText('Remove the sort'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(menuText('Remove the sort'), findsNothing);
  });

  testWidgets('a new row takes the value the filter asks for', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Category'));
    await tap(tester, menuText('Home'));
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();
    await tap(tester, find.text('New'));
    expect(find.text('3 of 4 rows'), findsOneWidget);
  });

  testWidgets('a row inserted from its handle stays under the filter', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await filterHome(tester);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(find.text('Rent')));
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('insert row')));
    expect(find.text('3 of 4 rows'), findsOneWidget);
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

  testWidgets('what is typed goes to the value as the menu opens', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    final value = tester.widget<TextField>(
      find.byKey(const Key('filter value')),
    );
    expect(value.focusNode!.hasFocus, isTrue);
  });

  testWidgets('an emptied value removes the filter and refuses nothing', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.textContaining('This is not'), findsNothing);
    await tester.enterText(find.byKey(const Key('filter value')), 'ren');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(view().tabs.first.filter, isNotNull);
    await tester.enterText(find.byKey(const Key('filter value')), '');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
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
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter match')));
    await tap(tester, menuText('is empty'));
    final saved = view().tabs.first.filter;
    expect(saved?.matches, BridgeMatch.empty);
    await tap(tester, find.byKey(const Key('filter match')));
    await tap(tester, menuText('contains'));
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();
    expect(view().tabs.first.filter?.matches, BridgeMatch.empty);
  });

  testWidgets('esc closes the menu after its value field went', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Category'));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('table query menu')), findsNothing);
  });

  testWidgets('esc closes the menu while a choice is open', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('table query menu')), findsNothing);
  });

  testWidgets('closing a choice gives the keys back to the value', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter match')));
    await tap(tester, find.byKey(const Key('filter match')));
    final value = tester.widget<TextField>(
      find.byKey(const Key('filter value')),
    );
    expect(value.focusNode!.hasFocus, isTrue);
  });
}
