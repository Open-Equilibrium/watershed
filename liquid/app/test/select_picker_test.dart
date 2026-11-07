import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
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

  testWidgets(
    'a second click on the picked cell leaves the options the keys',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.tap(cell('r1', 'category'), kind: PointerDeviceKind.mouse);
      await tester.pumpAndSettle();
      expect(picker, findsOneWidget);
      await key(tester, LogicalKeyboardKey.arrowDown);
      await key(tester, LogicalKeyboardKey.enter);
      expect(chosen(), [('r1', 'home')]);
    },
  );

  testWidgets('scrolling the Table closes the options', (tester) async {
    await kit.show(tester, [
      TablePart([
        ...expenses,
        for (var number = 3; number <= 30; number++) namedRow(number),
      ]),
    ]);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    final rows = tester
        .widget<ListView>(find.byKey(const Key('table rows')))
        .controller!;
    rows.jumpTo(rows.position.maxScrollExtent);
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
  });

  testWidgets('scrolling the Page closes the options', (tester) async {
    await kit.show(tester, [
      TablePart(expenses),
      for (var number = 0; number < 40; number++) TextPart('Line $number'),
    ]);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    final page = tester.state<ScrollableState>(
      find
          .descendant(
            of: find.byType(CustomScrollView),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    page.position.jumpTo(200);
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
  });

  testWidgets(
    'a click in the options beside them keeps the keys',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await open(tester, 'r1');
      await tester.tap(
        inPicker(find.text('Choose an option or create one')),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.arrowDown);
      await key(tester, LogicalKeyboardKey.enter);
      expect(chosen(), [('r1', 'home')]);
    },
  );

  testWidgets(
    'many options scroll inside the window and ↓ keeps the chosen in view',
    (tester) async {
      await kit.show(tester, [
        TablePart(
          expenses,
          fields: [
            for (final field in expenseFields)
              field.id == 'category'
                  ? BridgeField(
                      id: 'category',
                      name: 'Category',
                      kind: BridgeFieldKind.select,
                      options: [
                        for (var number = 1; number <= 60; number++)
                          BridgeOption(
                            id: 'o$number',
                            name: 'Option $number',
                            color: number % 10,
                          ),
                      ],
                    )
                  : field,
          ],
        ),
      ]);
      await open(tester, 'r1');
      final window = tester.getRect(find.byType(MaterialApp));
      expect(window.contains(tester.getRect(picker).bottomLeft), isTrue);
      for (var step = 0; step < 39; step++) {
        await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      }
      await tester.pumpAndSettle();
      final chosenRect = tester.getRect(inPicker(find.text('Option 40')));
      expect(tester.getRect(picker).contains(chosenRect.center), isTrue);
      await key(tester, LogicalKeyboardKey.enter);
      expect(chosen(), [('r1', 'o40')]);
    },
  );

  testWidgets(
    'a select cell near the window bottom opens its options above it',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final bottom = tester.getRect(cell('r1', 'category')).bottom;
      await tester.binding.setSurfaceSize(Size(1400, bottom + 40));
      await tester.pumpAndSettle();
      await open(tester, 'r1');
      final rect = tester.getRect(picker);
      expect(
        rect.bottom,
        lessThanOrEqualTo(tester.getRect(cell('r1', 'category')).top),
      );
      expect(rect.top, greaterThanOrEqualTo(0));
    },
  );

  testWidgets('resizing the window closes a select cell\'s options', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'r1');
    tester.view.physicalSize = tester.view.physicalSize / 2;
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpAndSettle();
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
  });

  testWidgets(
    'a select cell the Table scrolls into view to open hangs its options under it',
    (tester) async {
      await kit.show(tester, [
        TablePart([
          ...expenses,
          for (var number = 3; number <= 30; number++) namedRow(number),
        ]),
      ]);
      await tester.tap(cell('r1', 'category'));
      await tester.pumpAndSettle();
      final rows = tester
          .widget<ListView>(find.byKey(const Key('table rows')))
          .controller!;
      // The cell, partly scrolled out of view, is still there.
      rows.jumpTo(20);
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.keyF);
      expect(rows.offset, 0);
      expect(
        tester.getRect(picker).top,
        moreOrLessEquals(tester.getRect(cell('r1', 'category')).bottom + 4),
      );
    },
  );

  testWidgets(
    'the options choose their side where the Table scrolls their cell to',
    (tester) async {
      await kit.show(tester, [
        TablePart([
          ...expenses,
          for (var number = 3; number <= 30; number++) namedRow(number),
        ]),
      ]);
      await tester.tap(cell('r1', 'category'));
      await tester.pumpAndSettle();
      // Under the cell where it is shown, the options' 132 do not fit; 20 higher they would.
      final bottom = tester.getRect(cell('r1', 'category')).bottom;
      await tester.binding.setSurfaceSize(Size(1400, bottom + 130));
      await tester.pumpAndSettle();
      final rows = tester
          .widget<ListView>(find.byKey(const Key('table rows')))
          .controller!;
      rows.jumpTo(20);
      await tester.pumpAndSettle();
      await key(tester, LogicalKeyboardKey.enter);
      expect(rows.offset, 0);
      expect(
        tester.getRect(picker).bottom,
        lessThanOrEqualTo(tester.getRect(cell('r1', 'category')).top),
      );
    },
  );

  testWidgets(
    'rows added elsewhere that move the cell out of the built rows close its options',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final table = kit.fake.views[kit.page]!.single.block;
      await open(tester, 'r1');
      expect(picker, findsOneWidget);
      // Another device adds rows above it.
      kit.fake.putRows(kit.page, table, [
        for (var number = 3; number <= 80; number++) namedRow(number),
        ...kit.fake.rows[table]!,
      ]);
      await tester.pumpAndSettle();
      expect(picker, findsNothing);
      expect(tester.binding.hasScheduledFrame, isFalse);
    },
  );

  testWidgets(
    'a select cell scrolled far out of view opens its options once revealed',
    (tester) async {
      await kit.show(tester, [
        TablePart([
          ...expenses,
          for (var number = 3; number <= 90; number++) namedRow(number),
        ]),
      ]);
      await tester.tap(cell('r1', 'category'));
      await tester.pumpAndSettle();
      final rows = tester
          .widget<ListView>(find.byKey(const Key('table rows')))
          .controller!;
      rows.jumpTo(rows.position.maxScrollExtent);
      await tester.pumpAndSettle();
      expect(cell('r1', 'category'), findsNothing);
      await key(tester, LogicalKeyboardKey.enter);
      expect(rows.offset, 0);
      expect(picker, findsOneWidget);
      expect(
        tester.getRect(picker).top,
        moreOrLessEquals(tester.getRect(cell('r1', 'category')).bottom + 4),
      );
      await key(tester, LogicalKeyboardKey.arrowDown);
      await key(tester, LogicalKeyboardKey.enter);
      expect(chosen(), [('r1', 'home')]);
    },
  );
}
