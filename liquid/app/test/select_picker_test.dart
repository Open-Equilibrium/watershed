import 'package:flutter/gestures.dart';
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

  ScrollController rowsOf(WidgetTester tester) =>
      tester.widget<ListView>(find.byKey(const Key('table rows'))).controller!;

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

  List<Part> manyLines() => [
    for (var number = 0; number < 40; number++) TextPart('Line $number'),
  ];

  void expectClosed() {
    expect(picker, findsNothing);
    expect(chosen(), isEmpty);
  }

  void expectHangingBelowCell(WidgetTester tester) => expect(
    tester.getRect(picker).top,
    moreOrLessEquals(tester.getRect(cell('r1', 'category')).bottom + 4),
  );

  Future<void> clickKeepsKeys(WidgetTester tester, Finder target) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    await tester.tap(target, kind: PointerDeviceKind.mouse);
    await tester.pumpAndSettle();
    expect(picker, findsOneWidget);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    expect(chosen(), [('r1', 'home')]);
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

  testWidgets('a click outside or a resize closes the options', (tester) async {
    await showExpenses(tester);
    await open(tester, 'r1');
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expectClosed();

    await open(tester, 'r1');
    tester.view.physicalSize = tester.view.physicalSize / 2;
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpAndSettle();
    expectClosed();
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

  testWidgets(
    'a second click on the picked cell leaves the options the keys',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) => clickKeepsKeys(tester, cell('r1', 'category')),
  );

  testWidgets(
    'a click in the options beside them keeps the keys',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) => clickKeepsKeys(
      tester,
      inPicker(find.text('Choose an option or create one')),
    ),
  );

  testWidgets('scrolling the Table closes the options', (tester) async {
    await showExpenses(tester, lastRow: 30);
    await open(tester, 'r1');
    expect(picker, findsOneWidget);
    final rows = rowsOf(tester);
    rows.jumpTo(rows.position.maxScrollExtent);
    await tester.pumpAndSettle();
    expectClosed();
  });

  testWidgets(
    'many options scroll inside the window and ↓ keeps the chosen in view',
    (tester) async {
      await showExpenses(
        tester,
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
                    formula: '',
                  )
                : field,
        ],
      );
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

  testWidgets('a select cell scrolled out of view opens its options under it', (
    tester,
  ) async {
    await showExpenses(tester, lastRow: 90);
    await clickCell(tester, 'r1', 'category');
    final rows = rowsOf(tester);
    rows.jumpTo(20);
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.keyF);
    expect(rows.offset, 0);
    expectHangingBelowCell(tester);

    await key(tester, LogicalKeyboardKey.escape);
    rows.jumpTo(rows.position.maxScrollExtent);
    await tester.pumpAndSettle();
    expect(cell('r1', 'category'), findsNothing);
    await key(tester, LogicalKeyboardKey.enter);
    expect(rows.offset, 0);
    expect(picker, findsOneWidget);
    expectHangingBelowCell(tester);
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    expect(chosen(), [('r1', 'home')]);
  });

  testWidgets(
    'rows added elsewhere that move the cell out of the built rows close its options',
    (tester) async {
      await showExpenses(tester);
      final table = kit.fake.views[kit.page]!.single.block;
      await open(tester, 'r1');
      expect(picker, findsOneWidget);
      kit.fake.putRows(kit.page, table, [
        ...namedRows(3, 80),
        ...kit.fake.rows[table]!,
      ]);
      await tester.pumpAndSettle();
      expect(picker, findsNothing);
      expect(tester.binding.hasScheduledFrame, isFalse);
    },
  );

  testWidgets('a cell below the window scrolls into view', (tester) async {
    await kit.show(tester, [TablePart(expenses), ...manyLines()]);
    final bottom = tester.getRect(cell('r1', 'category')).bottom;
    await tester.binding.setSurfaceSize(Size(1400, bottom + 10));
    await tester.pumpAndSettle();
    await clickCell(tester, 'r1', 'category');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    final window = Offset.zero & tester.binding.renderViews.first.size;
    expect(
      window.contains(tester.getRect(cell('r2', 'category')).center),
      isTrue,
    );
    expect(window.contains(tester.getRect(picker).center), isTrue);
    await key(tester, LogicalKeyboardKey.enter);
    expect(chosen(), [('r2', 'food')]);
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
