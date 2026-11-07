import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:intl/intl.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'table_kit.dart';

void main() {
  final kit = TableKit();

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

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  /// The days written into date cells, by row; an emptied cell as none.
  List<(String, String?)> written() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.cell!.kind == BridgeCellKind.date ? set.cell!.text : null),
  ];

  Future<void> open(WidgetTester tester, String row) async {
    await tester.tap(cell(row, 'date'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
  }

  testWidgets('⏎ on a date cell opens its day; ↓ ⏎ writes the next day', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    expect(typed(tester), 'Nov 1, 2026');
    expect(inPicker(find.text('November 2026')), findsOneWidget);
    expect(inPicker(find.text('Clear')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.arrowDown);
    expect(typed(tester), 'Nov 2, 2026');
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-11-02')]);
  });

  testWidgets('↑ goes to the day before, across a month', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.arrowUp);
    expect(typed(tester), 'Oct 31, 2026');
    expect(inPicker(find.text('October 2026')), findsOneWidget);
  });

  for (final form in ['10/15/2026', 'Oct 15, 2026', '2026-10-15']) {
    testWidgets('a day typed as $form is written', (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.enterText(inPicker(find.byType(TextField)), form);
      await tester.pumpAndSettle();
      expect(inPicker(find.text('October 2026')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.enter);
      expect(written(), [('r1', '2026-10-15')]);
    });
  }

  testWidgets('typing on a selected date cell opens its picker with the keys', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r2', 'date'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.digit1);
    expect(picker, findsOneWidget);
    expect(typed(tester), '1');
  });

  testWidgets(
    'a typed text that is no day stays, saying so, and writes nothing',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.enterText(inPicker(find.byType(TextField)), 'soon');
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.enter);
      expect(picker, findsOneWidget);
      expect(typed(tester), 'soon');
      expect(kit.inTable(find.text('“soon” is no date.')), findsOneWidget);
      expect(written(), isEmpty);
    },
  );

  testWidgets(
    'a click on a day writes it and closes; the cell stays selected',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.tap(day('2026-11-20'));
      await tester.pumpAndSettle();
      expect(picker, findsNothing);
      expect(written(), [('r1', '2026-11-20')]);
      await key(tester, LogicalKeyboardKey.enter);
      expect(picker, findsOneWidget);
    },
  );

  testWidgets('‹ and › show the month before and after', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.tap(inPicker(find.byKey(const Key('next month'))));
    await tester.pumpAndSettle();
    expect(inPicker(find.text('December 2026')), findsOneWidget);
    await tester.tap(day('2026-12-24'));
    await tester.pumpAndSettle();
    expect(written(), [('r1', '2026-12-24')]);
    await open(tester, 'r2');
    await tester.tap(inPicker(find.byKey(const Key('previous month'))));
    await tester.pumpAndSettle();
    expect(inPicker(find.text('September 2026')), findsOneWidget);
  });

  testWidgets('Clear empties the cell', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.tap(inPicker(find.text('Clear')));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    final set = kit.applied(BridgeMutationKind.setCell).single;
    expect(
      (set.row, set.field, set.cell!.kind),
      ('r1', 'date', BridgeCellKind.empty),
    );
  });

  testWidgets('esc closes and keeps the day, also with a day typed', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.enterText(inPicker(find.byType(TextField)), '2026-10-15');
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.escape);
    expect(picker, findsNothing);
    expect(written(), isEmpty);
  });

  testWidgets('⇥ writes the day typed and selects the next cell', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.tab);
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-11-02')]);
    // The next cell, Category, opens its options.
    await key(tester, LogicalKeyboardKey.enter);
    expect(find.byKey(const Key('select picker')), findsOneWidget);
  });

  testWidgets('⇥ on a text that is no day stays, saying so', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.enterText(inPicker(find.byType(TextField)), 'soon');
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.tab);
    expect(picker, findsOneWidget);
    expect(kit.inTable(find.text('“soon” is no date.')), findsOneWidget);
    expect(written(), isEmpty);
  });

  testWidgets('a click outside writes the day typed, and drops a text that is '
      'no day', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.enterText(inPicker(find.byType(TextField)), '2026-10-15');
    await tester.pumpAndSettle();
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(written(), [('r1', '2026-10-15')]);
    await open(tester, 'r2');
    await tester.enterText(inPicker(find.byType(TextField)), 'soon');
    await tester.pumpAndSettle();
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(kit.inTable(find.text('“soon” is no date.')), findsNothing);
    expect(written(), [('r1', '2026-10-15')]);
  });

  testWidgets('an empty date cell opens on today\'s month', (tester) async {
    await kit.show(tester, [
      TablePart([...expenses, namedRow(3)]),
    ]);
    await open(tester, 'r3');
    expect(typed(tester), '');
    expect(
      inPicker(find.text(DateFormat.yMMMM('en_US').format(DateTime.now()))),
      findsOneWidget,
    );
  });

  testWidgets('weeks start and days are typed as in the person\'s region', (
    tester,
  ) async {
    tester.platformDispatcher.localesTestValue = const [Locale('de', 'DE')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await kit.show(tester, [TablePart(expenses)], app: true);
    await open(tester, 'r1');
    expect(typed(tester), '1. Nov. 2026');
    final mondays = tester.getRect(inPicker(find.text('Mo'))).left;
    final sundays = tester.getRect(inPicker(find.text('So'))).left;
    expect(mondays, lessThan(sundays));
    await tester.enterText(inPicker(find.byType(TextField)), '15.10.2026');
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    expect(written(), [('r1', '2026-10-15')]);
  });

  testWidgets('in English (US) weeks start on Sunday', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    final sundays = tester.getRect(inPicker(find.text('Su'))).left;
    final mondays = tester.getRect(inPicker(find.text('Mo'))).left;
    expect(sundays, lessThan(mondays));
  });

  for (final form in ['10/15/26', '1/1/12345', '1/1/999999']) {
    testWidgets('a year not of four digits, as in $form, is no date', (
      tester,
    ) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.enterText(inPicker(find.byType(TextField)), form);
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.enter);
      expect(picker, findsOneWidget);
      expect(kit.inTable(find.text('“$form” is no date.')), findsOneWidget);
      expect(written(), isEmpty);
    });
  }

  testWidgets('a picker closed as its cell moves elsewhere drops its reason', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await open(tester, 'r1');
    await tester.enterText(inPicker(find.byType(TextField)), 'soon');
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
    // Another device adds a row above it.
    kit.fake.putRows(kit.page, table, [namedRow(3), ...kit.fake.rows[table]!]);
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(kit.inTable(find.text('“soon” is no date.')), findsNothing);
  });

  testWidgets(
    'the day a cell has, or an empty cell cleared, is not written again',
    (tester) async {
      await kit.show(tester, [
        TablePart([...expenses, namedRow(3)]),
      ]);
      await open(tester, 'r1');
      await key(tester, LogicalKeyboardKey.enter);
      // The cell stays selected; ⏎ opens it again.
      await key(tester, LogicalKeyboardKey.enter);
      await tester.tap(day('2026-11-01'));
      await tester.pumpAndSettle();
      await open(tester, 'r3');
      await tester.tap(inPicker(find.text('Clear')));
      await tester.pumpAndSettle();
      expect(picker, findsNothing);
      expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
    },
  );
}
