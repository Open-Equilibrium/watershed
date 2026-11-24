import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:intl/intl.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final now = DateTime.now();
  String day(int month, int day) =>
      DateFormat('yyyy-MM-dd')
          .format(DateTime(now.year, now.month + month, day));

  Future<void> calendar(WidgetTester tester, List<BridgeRow> rows) async {
    await kit.show(tester, [TablePart(rows)]);
    final view = kit.fake.views[kit.page]!.firstWhere(
      (view) => view.kind == BridgeBlockKind.database,
    );
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setTabLayout,
        page: kit.page,
        view: view.id,
        tab: view.tabs.first.id,
        layout: BridgeLayout.calendar,
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder dayCell(String date) => find.byKey(Key('day $date'));

  Finder textIn(Finder area, String text) =>
      find.descendant(of: area, matching: find.text(text));

  Finder onDay(String date, String text) => textIn(dayCell(date), text);

  final dayList = find.byKey(const Key('day list'));

  testWidgets('rows land on days; No date lists the rest', (tester) async {
    await calendar(tester, [
      expenseRow('r1', 'Groceries', day(0, 3), 'food', 84.2, true),
      expenseRow('r2', 'Rent', day(1, 10), 'home', 950, true),
      namedRow(3),
    ]);
    expect(find.text(DateFormat.yMMMM().format(now)), findsOneWidget);
    expect(onDay(day(0, 3), 'Groceries'), findsOneWidget);
    expect(find.text('Rent'), findsNothing);
    await tap(tester, find.byKey(const Key('next month')));
    expect(onDay(day(1, 10), 'Rent'), findsOneWidget);
    await tap(tester, find.byKey(const Key('this month')));
    expect(onDay(day(0, 3), 'Groceries'), findsOneWidget);
    expect(textIn(find.byKey(const Key('table date')), 'Date'), findsOneWidget);

    await tap(tester, find.text('No date · 1'));
    expect(textIn(dayList, 'Row 3'), findsOneWidget);
  });

  testWidgets('dragging a row to another day changes its date', (tester) async {
    await calendar(tester, [
      expenseRow('r1', 'Groceries', day(0, 3), 'food', 84.2, true),
    ]);
    final from = onDay(day(0, 3), 'Groceries');
    await tester.timedDrag(
      from,
      tester.getCenter(dayCell(day(0, 18))) - tester.getCenter(from),
      const Duration(milliseconds: 400),
    );
    await tester.pumpAndSettle();
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect((set.row, set.field, set.cell?.text), ('r1', 'date', day(0, 18)));
  });

  testWidgets('＋ on a hovered day adds a row with that date', (tester) async {
    final date = day(0, 7);
    await calendar(tester, const []);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(dayCell(date)));
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(Key('add on $date')));
    expect(kit.applied(BridgeMutationKind.addRow), hasLength(1));
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect((set.field, set.cell?.text), ('date', date));
    expect(find.byKey(const Key('row popover')), findsOneWidget);
    await mouse.removePointer();
  });

  testWidgets('an undated row opened from No date stays open on a reload', (
    tester,
  ) async {
    await calendar(tester, [namedRow(3)]);
    await tap(tester, find.text('No date · 1'));
    await tap(tester, textIn(dayList, 'Row 3'));
    expect(find.byKey(const Key('row popover')), findsOneWidget);
    kit.fake.putRows(kit.page, kit.fake.rows.keys.single, [namedRow(3)]);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('row popover')), findsOneWidget);
  });
}
