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
  final picker = find.byKey(const Key('select picker'));
  Finder inPicker(Finder finder) =>
      find.descendant(of: picker, matching: finder);

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  List<(String, String?)> chosen() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.cell!.text),
  ];

  Future<void> clickCell(WidgetTester tester, String row, String field) async {
    await tester.tap(cell(row, field));
    await tester.pumpAndSettle();
  }

  List<BridgeRow> namedRows(int from, int to) => [
    for (var number = from; number <= to; number++) namedRow(number),
  ];

  Future<void> open(WidgetTester tester, String row) async {
    await clickCell(tester, row, 'category');
    await key(tester, LogicalKeyboardKey.enter);
  }

  Future<void> showExpenses(
    WidgetTester tester, {
    int lastRow = 2,
    List<BridgeField> fields = expenseFields,
  }) => kit.show(tester, [
    TablePart([...expenses, ...namedRows(3, lastRow)], fields: fields),
  ]);

  void expectClosed() {
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
  }

  testWidgets('⏎ opens a select cell; ↓ ⏎ picks an option', (tester) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    for (final option in ['Food', 'Home', 'Choose an option or create one']) {
      expect(inPicker(find.text(option)), findsOneWidget);
    }
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsNothing);
    expect(chosen(), [('r1', 'home')]);
  });

  testWidgets(
    'picking the option a cell holds writes nothing and warns of nothing',
    (tester) async {
      await showExpenses(tester);
      await open(tester, 'r1');
      await key(tester, LogicalKeyboardKey.enter);
      expect(picker, findsNothing);
      expect(chosen(), isEmpty);
      expect(kit.workspace.problem.value, isNull);
    },
  );

  testWidgets('typing filters options, ⏎ picks the first', (tester) async {
    await showExpenses(tester);
    await clickCell(tester, 'r2', 'category');
    await key(tester, LogicalKeyboardKey.keyF);
    expect(inPicker(find.text('Food')), findsOneWidget);
    expect(inPicker(find.text('Home')), findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    expect(chosen(), [('r2', 'food')]);
  });

  testWidgets('⏎ on a new name creates it in the next color', (tester) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    await tester.enterText(inPicker(find.byType(TextField)), 'Travel');
    await tester.pumpAndSettle();
    expect(inPicker(find.text('Create')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);
    final added = kit.applied(BridgeMutationKind.addOption).single;
    expect((added.field, added.text, added.color), ('category', 'Travel', 7));
    final travel = kit
        .fake
        .databases[kit.fake.views[kit.page]!.single.block]!
        .fields
        .firstWhere((field) => field.id == 'category')
        .options
        .last;
    expect(chosen(), [('r1', travel.id)]);
    expect(kit.inTable(find.text('Travel')), findsOneWidget);
  });

  testWidgets('a click picks an option; esc keeps the value', (tester) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.escape);
    expectClosed();
    await key(tester, LogicalKeyboardKey.enter);
    await tester.tap(inPicker(find.text('Home')));
    await tester.pumpAndSettle();
    expect(chosen(), [('r1', 'home')]);
    expect(picker, findsNothing);
  });

  testWidgets('keys typed before the options still filter', (tester) async {
    await showExpenses(tester);
    await clickCell(tester, 'r1', 'category');
    await tester.sendKeyEvent(LogicalKeyboardKey.keyH);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyO);
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<TextField>(inPicker(find.byType(TextField)))
          .controller!
          .text,
      'ho',
    );
  });

  testWidgets('⇥ in the options keeps the value, selects next', (tester) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.tab);
    expect(picker, findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    await tester.enterText(
      kit.inTable(
        find.descendant(
          of: find.byKey(const Key('cell editor')),
          matching: find.byType(TextField),
        ),
      ),
      '12',
    );
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      [
        for (final set in kit.applied(BridgeMutationKind.setCell))
          (set.row, set.field, set.cell!.number),
      ],
      [('r1', 'amount', 12.0)],
    );
  });
}
