import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'table_kit.dart';

void main() {
  final kit = TableKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));
  final picker = find.byKey(const Key('select picker'));
  Finder inPicker(Finder finder) =>
      find.descendant(of: picker, matching: finder);

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  /// The options written into cells, by row.
  List<(String, String?)> chosen() => [
    for (final set in kit.applied(BridgeMutationKind.setCell))
      (set.row!, set.cell!.text),
  ];

  Future<void> open(WidgetTester tester, String row) async {
    await tester.tap(cell(row, 'category'));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.enter);
  }

  testWidgets('⏎ on a select cell opens its options; ↓ ⏎ picks one', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    expect(inPicker(find.text('Food')), findsOneWidget);
    expect(inPicker(find.text('Home')), findsOneWidget);
    expect(
      inPicker(find.text('Choose an option or create one')),
      findsOneWidget,
    );
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    expect(picker, findsNothing);
    expect(chosen(), [('r1', 'home')]);
  });

  testWidgets(
    'typing on a select cell filters its options, ⏎ picks the first',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await tester.tap(cell('r2', 'category'));
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.keyF);
      expect(inPicker(find.text('Food')), findsOneWidget);
      expect(inPicker(find.text('Home')), findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      expect(chosen(), [('r2', 'food')]);
    },
  );

  testWidgets('⏎ on a name no option has creates it in the next color', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
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

  testWidgets('a click picks an option, esc closes and keeps the value', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await key(tester, LogicalKeyboardKey.escape);
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
    await key(tester, LogicalKeyboardKey.enter);
    await tester.tap(inPicker(find.text('Home')));
    await tester.pumpAndSettle();
    expect(chosen(), [('r1', 'home')]);
    expect(picker, findsNothing);
  });

  testWidgets('a click outside closes the options', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
  });

  testWidgets('keys typed before the options take them still filter', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(cell('r1', 'category'));
    await tester.pumpAndSettle();
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
}
