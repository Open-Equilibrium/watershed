import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

/// Sort and filter, stored on the View (approved Table View mockups, section 6).
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

  /// The names in the Table, top to bottom.
  List<String> names(WidgetTester tester) {
    final shown = {
      for (final name in ['Groceries', 'Electricity', 'Rent'])
        if (find.text(name).evaluate().isNotEmpty)
          name: tester.getTopLeft(find.text(name).first).dy,
    };
    return shown.keys.toList()..sort((a, b) => shown[a]!.compareTo(shown[b]!));
  }

  BridgeView view() => kit.fake.views[kit.page]!.single;

  testWidgets('a sort is kept on the View and orders the rows', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table sort')));
    await tap(tester, menuText('Amount'));
    expect(view().sort, const BridgeSort(field: 'amount', descending: false));
    expect(names(tester), ['Groceries', 'Electricity', 'Rent']);
    await tap(tester, menuText('Descending'));
    expect(view().sort?.descending, isTrue);
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    expect(find.byKey(const Key('table sort chip')), findsOneWidget);
    await tap(tester, menuText('Remove the sort'));
    expect(view().sort, isNull);
    expect(find.byKey(const Key('table sort chip')), findsNothing);
  });

  testWidgets('a filter keeps its rows and says how many pass', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tap(tester, find.byKey(const Key('filter field')));
    await tap(tester, menuText('Category'));
    await tap(tester, menuText('Home'));
    expect(view().filter?.field, 'category');
    expect(view().filter?.matches, BridgeMatch.is_);
    expect(view().filter?.value?.text, 'home');
    expect(names(tester), ['Electricity', 'Rent']);
    expect(find.text('2 of 3 rows'), findsOneWidget);
    expect(find.text('Category is Home'), findsOneWidget);
    await tap(tester, menuText('Remove the filter'));
    expect(view().filter, isNull);
    expect(find.text('3 rows'), findsOneWidget);
  });

  testWidgets('text is matched as typed, and written on ⏎', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, find.byKey(const Key('table filter')));
    await tester.enterText(find.byKey(const Key('filter value')), 'ren');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(view().filter?.matches, BridgeMatch.contains);
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
    expect(view().filter, isNull);
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
    expect(view().filter?.matches, BridgeMatch.empty);
    expect(find.text('Row 4'), findsOneWidget);
    expect(find.text('1 of 4 rows'), findsOneWidget);
  });

  testWidgets('a sort or filter set elsewhere shows here', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setViewSort,
        page: kit.page,
        view: view().id,
        sort: const BridgeSort(field: 'amount', descending: true),
      ),
    );
    await tester.pumpAndSettle();
    expect(names(tester), ['Rent', 'Electricity', 'Groceries']);
    expect(find.byKey(const Key('table sort chip')), findsOneWidget);
  });

  testWidgets('a chip opens its menu again, and esc closes it', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setViewSort,
        page: kit.page,
        view: view().id,
        sort: const BridgeSort(field: 'amount', descending: true),
      ),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('table sort chip')));
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
}
