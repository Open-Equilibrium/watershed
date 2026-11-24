import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));
  final picker = find.byKey(const Key('date picker'));
  Finder inPicker(Finder finder) =>
      find.descendant(of: picker, matching: finder);
  Finder day(String day) => inPicker(find.byKey(ValueKey('day $day')));
  String typed(WidgetTester tester) => tester
      .widget<TextField>(inPicker(find.byType(TextField)))
      .controller!
      .text;

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String text) async {
    await tester.enterText(inPicker(find.byType(TextField)), text);
    await tester.pumpAndSettle();
  }

  List<(String, String?)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.cell!.kind == BridgeCellKind.date ? set.cell!.text : null),
  ];

  Future<void> open(WidgetTester tester, String row) async {
    await tap(tester, cell(row, 'date'));
    await key(tester, LogicalKeyboardKey.enter);
  }

  testWidgets('⏎ opens a date; ↑ and ↓ move by a day', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    expect(typed(tester), 'Nov 1, 2026');
    expect(inPicker(find.text('November 2026')), findsOneWidget);
    expect(inPicker(find.text('Clear')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(typed(tester), 'Oct 31, 2026');
    expect(inPicker(find.text('October 2026')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(typed(tester), 'Nov 2, 2026');
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-11-02')]);
  });

  for (final form in ['10/15/2026', 'Oct 15, 2026', '2026-10-15']) {
    testWidgets('a day typed as $form is written', (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await type(tester, form);
      expect(inPicker(find.text('October 2026')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.enter);
      expect(written(), [('r1', '2026-10-15')]);
    });
  }

  for (final (name, finish) in [
    ('⏎', LogicalKeyboardKey.enter),
    ('⇥', LogicalKeyboardKey.tab),
  ]) {
    testWidgets('$name on a non-day text stays, saying so', (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await type(tester, 'soon');
      await key(tester, finish);
      expect(picker, findsOneWidget);
      expect(typed(tester), 'soon');
      expect(kit.inTable(find.text('“soon” is no date.')), findsOneWidget);
      expect(written(), isEmpty);
    });
  }

  testWidgets('‹ and › show the month before and after', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tap(tester, inPicker(find.byKey(const Key('next month'))));
    expect(inPicker(find.text('December 2026')), findsOneWidget);
    await tap(tester, day('2026-12-24'));
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-12-24')]);
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsOneWidget);
    await tap(tester, inPicker(find.byKey(const Key('previous month'))));
    expect(inPicker(find.text('November 2026')), findsOneWidget);
  });

  testWidgets('typing and esc write nothing; Clear empties', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await type(tester, '2026-10-15');
    await key(tester, LogicalKeyboardKey.escape);
    expect(picker, findsNothing);
    expect(written(), isEmpty);

    await key(tester, LogicalKeyboardKey.enter);
    await tap(tester, inPicker(find.text('Clear')));
    expect(picker, findsNothing);
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect(
      (set.row, set.field, set.cell!.kind),
      ('r1', 'date', BridgeCellKind.empty),
    );

    await tap(tester, cell('r2', 'date'));
    await key(tester, LogicalKeyboardKey.digit1);
    expect(picker, findsOneWidget);
    expect(typed(tester), '1');
  });

  testWidgets('⇥ writes the day typed and moves on', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.tab);
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-11-02')]);
    await key(tester, LogicalKeyboardKey.enter);
    expect(find.byKey(const Key('select picker')), findsOneWidget);
  });

  testWidgets('an open typed day is kept on quit', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await type(tester, '2026-10-15');
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(written(), [('r1', '2026-10-15')]);
  });

  testWidgets('an open non-day is reported on quit, not kept', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r2');
    await type(tester, 'soon');
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(
      kit.workspace.problem.value,
      '“soon” is no date. It was not kept, as the Table closed.',
    );
    expect(written(), isEmpty);
  });

  testWidgets('a click outside writes a day, drops a non-day', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await type(tester, '2026-10-15');
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-10-15')]);
    await open(tester, 'r2');
    await type(tester, 'soon');
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(kit.inTable(find.text('“soon” is no date.')), findsNothing);
    expect(written(), [('r1', '2026-10-15')]);
  });

  testWidgets('weeks and typed days follow the region', (tester) async {
    tester.platformDispatcher.localesTestValue = const [Locale('de', 'DE')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await kit.show(tester, [TablePart(expenses)], app: true);
    await open(tester, 'r1');
    expect(typed(tester), '1. Nov. 2026');
    final mondays = tester.getRect(inPicker(find.text('Mo'))).left;
    final sundays = tester.getRect(inPicker(find.text('So'))).left;
    expect(mondays, lessThan(sundays));
    await type(tester, '15.10.2026');
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r1', '2026-10-15')]);
  });

  for (final form in ['10/15/26', '1/1/12345', '1/1/999999']) {
    testWidgets('a year not of 4 digits is no date', (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await type(tester, form);
      await key(tester, LogicalKeyboardKey.enter);
      expect(picker, findsOneWidget);
      expect(kit.inTable(find.text('“$form” is no date.')), findsOneWidget);
      expect(written(), isEmpty);
    });
  }

  testWidgets('unchanged cells are not written again', (tester) async {
    await kit.show(tester, [
      TablePart([...expenses, namedRow(3)]),
    ]);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.enter);
    await key(tester, LogicalKeyboardKey.enter);
    await tap(tester, day('2026-11-01'));
    await open(tester, 'r3');
    await tap(tester, inPicker(find.text('Clear')));
    expect(picker, findsNothing);
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
  });

  testWidgets('a cell\'s own short-year day is kept', (tester) async {
    await kit.show(tester, [
      TablePart([expenseRow('r1', 'Old', '0500-01-01', 'food', 1, false)]),
    ]);
    await open(tester, 'r1');
    expect(typed(tester), 'Jan 1, 500');
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    await key(tester, LogicalKeyboardKey.tab);
    expect(picker, findsNothing);
    expect(kit.inTable(find.textContaining('is no date')), findsNothing);
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
  });
}
