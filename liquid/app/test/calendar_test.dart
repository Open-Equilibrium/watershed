import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:intl/intl.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final now = DateTime.now();
  String day(int month, int day) =>
      DateFormat('yyyy-MM-dd')
          .format(DateTime(now.year, now.month + month, day));

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Future<void> calendar(WidgetTester tester, List<BridgeRow> rows) async {
    await kit.show(tester, [TablePart(rows)]);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabLayout,
        page: kit.page,
        view: view().id,
        tab: view().tabs.first.id,
        layout: BridgeLayout.calendar,
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder dayCell(String date) => find.byKey(Key('day $date'));

  Finder onDay(String date, String text) =>
      find.descendant(of: dayCell(date), matching: find.text(text));

  testWidgets('a Calendar places rows on their days, a month at a time', (
    tester,
  ) async {
    await calendar(tester, [
      expenseRow('r1', 'Groceries', day(0, 3), 'food', 84.2, true),
      expenseRow('r2', 'Rent', day(1, 10), 'home', 950, true),
    ]);
    expect(find.text(DateFormat.yMMMM().format(now)), findsOneWidget);
    expect(onDay(day(0, 3), 'Groceries'), findsOneWidget);
    expect(find.text('Rent'), findsNothing);
    await tap(tester, find.byKey(const Key('next month')));
    expect(onDay(day(1, 10), 'Rent'), findsOneWidget);
    await tap(tester, find.byKey(const Key('this month')));
    expect(onDay(day(0, 3), 'Groceries'), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('table date')),
        matching: find.text('Date'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('a day shows three rows and “+N more”, which lists them all', (
    tester,
  ) async {
    await calendar(tester, [
      for (var number = 1; number <= 5; number++)
        expenseRow('r$number', 'Item $number', day(0, 12), 'food', 1, false),
    ]);
    expect(onDay(day(0, 12), 'Item 3'), findsOneWidget);
    expect(onDay(day(0, 12), 'Item 4'), findsNothing);
    await tap(tester, onDay(day(0, 12), '+2 more'));
    expect(
      find.descendant(
        of: find.byKey(const Key('day list')),
        matching: find.text('Item 5'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('dragging a row to another day changes its date', (tester) async {
    await calendar(tester, [
      expenseRow('r1', 'Groceries', day(0, 3), 'food', 84.2, true),
    ]);
    await tester.timedDrag(
      onDay(day(0, 3), 'Groceries'),
      tester.getCenter(dayCell(day(0, 18))) -
          tester.getCenter(onDay(day(0, 3), 'Groceries')),
      const Duration(milliseconds: 400),
    );
    await tester.pumpAndSettle();
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect((set.row, set.field, set.cell?.text), ('r1', 'date', day(0, 18)));
  });

  testWidgets('＋ on a hovered day adds a row with that date', (tester) async {
    await calendar(tester, const []);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(dayCell(day(0, 7))));
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(Key('add on ${day(0, 7)}')));
    expect(kit.applied(BridgeMutationKind.addRow), hasLength(1));
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect((set.field, set.cell?.text), ('date', day(0, 7)));
    expect(find.byKey(const Key('row popover')), findsOneWidget);
    await mouse.removePointer();
  });

  testWidgets('a month with more rows than it reads says so', (tester) async {
    await calendar(tester, [
      for (var number = 1; number <= 1001; number++)
        expenseRow('r$number', 'Item $number', day(0, 12), 'food', 1, false),
    ]);
    expect(find.text('1,000 of 1,001 rows shown'), findsOneWidget);
  });

  testWidgets('No date lists the rows without one', (tester) async {
    await calendar(tester, [
      expenseRow('r1', 'Groceries', day(0, 3), 'food', 84.2, true),
      namedRow(2),
    ]);
    await tap(tester, find.text('No date · 1'));
    expect(
      find.descendant(
        of: find.byKey(const Key('day list')),
        matching: find.text('Row 2'),
      ),
      findsOneWidget,
    );
  });
}
