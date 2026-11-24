import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final addField = kit.inTable(find.byKey(const Key('add field')));
  final newRow = kit.inTable(find.text('New'));
  final panel = find.byKey(const Key('field panel'));

  List<BridgeRow> namedRows(int count) => [
    for (var number = 1; number <= count; number++) namedRow(number),
  ];

  List<BridgeField> textFields(int last) => [
    for (var number = 1; number <= last; number++)
      BridgeField(
        id: 'f$number',
        name: 'Field $number',
        kind: BridgeFieldKind.text,
        options: const [],
        formula: '',
      ),
  ];

  ScrollController sidewaysOf(WidgetTester tester) => tester
      .widget<SingleChildScrollView>(find.byKey(const Key('table columns')))
      .controller!;

  ScrollController rowsScroll(WidgetTester tester) =>
      tester.widget<ListView>(find.byKey(const Key('table rows'))).controller!;

  List<BridgeField> shownFields() =>
      kit.fake.databases[kit.fake.views[kit.page]!.single.block]!.fields;

  Future<void> openPanel(WidgetTester tester) => tap(tester, addField);

  Completer<void> gateOn(BridgeMutationKind kind) {
    kit.fake.gatedKinds = {kind};
    return kit.fake.gate = Completer<void>();
  }

  Future<void> releaseGate(WidgetTester tester, Completer<void> gate) async {
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
  }

  List<Completer<void>> holdDatabaseReads() {
    kit.fake.deferredKinds = {BridgeQueryKind.database};
    return kit.fake.deferred = [];
  }

  Future<void> releaseReads(
    WidgetTester tester,
    List<Completer<void>> reads,
  ) async {
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
  }

  Future<Completer<void>> addThenReopen(WidgetTester tester) async {
    await openPanel(tester);
    final gate = gateOn(BridgeMutationKind.addField);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await openPanel(tester);
    return gate;
  }

  testWidgets('a Table shows its name, fields and rows', (tester) async {
    await kit.show(tester, [const TextPart('October'), TablePart(expenses)]);
    for (final text in [
      'Expenses',
      'Name',
      'Date',
      'Category',
      'Amount',
      'Paid',
      'Groceries',
      'Electricity',
      'Nov 1, 2026',
      'Oct 30, 2026',
      'Food',
      'Home',
      '84.2',
      '96',
      '2 rows',
    ]) {
      expect(kit.inTable(find.text(text)), findsOneWidget, reason: text);
    }
    expect(
      kit.inTable(find.byIcon(LucideIcons.check)),
      findsOneWidget,
      reason: 'one ticked checkbox',
    );
    expect(
      tester.getRect(kit.inTable(find.text('96'))).right,
      greaterThan(tester.getRect(kit.inTable(find.text('Amount'))).right - 12),
      reason: 'numbers sit right-aligned',
    );
    expect(
      find.byType(TextField),
      findsNWidgets(3),
      reason:
          'the title, the Text Block and the Table\'s name; no cell is text',
    );
  });

  testWidgets(
    'a long Table reads its rows in windows and scrolls inside a fixed height',
    (tester) async {
      await kit.show(tester, [
        TablePart(namedRows(2000)),
        const TextPart('Below'),
      ]);
      expect(kit.inTable(find.text('2,000 rows')), findsOneWidget);
      expect(kit.inTable(find.text('Row 1')), findsOneWidget);
      expect(kit.inTable(find.text('Row 40')), findsNothing);
      final rows = kit.fake.asked.where(
        (query) => query.kind == BridgeQueryKind.rows,
      );
      expect(rows, isNotEmpty);
      expect(rows.every((query) => query.limit <= 100), isTrue);
      expect(rows.every((query) => query.start == 0), isTrue);
      expect(
        tester.getSize(find.byType(TableBlock)).height,
        lessThan(32 + 34 + 40 + 11 * 34 + 2 * 40),
        reason: 'a title, the tabs, the setup line, and about ten rows',
      );

      await tester.scrollUntilVisible(
        kit.inTable(find.text('Row 150')),
        300,
        scrollable: find.descendant(
          of: find.byKey(const Key('table rows')),
          matching: find.byType(Scrollable),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        kit.fake.asked.any(
          (query) => query.kind == BridgeQueryKind.rows && query.start == 100,
        ),
        isTrue,
        reason: 'the next window, once it scrolls in',
      );
      expect(kit.inTable(find.text('2,000 rows')), findsOneWidget);
      expect(find.text('Below'), findsOneWidget, reason: 'the Page goes on');
    },
  );

  testWidgets(
    'a wide Table builds the fields in view, keeping the first one in view',
    (tester) async {
      await kit.show(tester, [
        TablePart(expenses, fields: [expenseFields.first, ...textFields(299)]),
      ]);
      expect(kit.inTable(find.text('Field 1')), findsOneWidget);
      expect(kit.inTable(find.text('Field 299')), findsNothing);

      await tester.drag(
        kit.inTable(find.text('Field 2')),
        const Offset(-60000, 0),
      );
      await tester.pumpAndSettle();
      expect(kit.inTable(find.text('Field 299')), findsOneWidget);
      expect(kit.inTable(find.text('Field 2')), findsNothing);
      expect(
        kit.inTable(find.text('Groceries')),
        findsOneWidget,
        reason: 'the first field stays in view',
      );
      expect(
        tester.getRect(kit.inTable(find.text('Name'))).left,
        lessThan(tester.getRect(kit.inTable(find.text('Field 299'))).left),
      );
    },
  );

  testWidgets(
    '⌘Z in a View\'s title after a rename elsewhere does not bring back what was typed',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final name = kit.inTable(find.byType(TextField));
      await tester.tap(name);
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: 'Expenses x',
          selection: TextSelection.collapsed(offset: 10),
        ),
      );
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 600));
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pumpAndSettle();
      final view = kit.fake.views[kit.page]!.single.id;
      await kit.workspace.apply(
        mutation(
          BridgeMutationKind.renameView,
          page: kit.page,
          view: view,
          text: 'Trips',
        ),
      );
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 600));
      await tester.tap(name);
      await undo(tester);
      expect(tester.widget<TextField>(name).controller?.text, 'Trips');
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets('a Table follows its rows as they change', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    kit.fake.putRows(kit.page, table, [...expenses, namedRow(3)]);
    await tester.pumpAndSettle();
    expect(kit.inTable(find.text('Row 3')), findsOneWidget);
    expect(kit.inTable(find.text('3 rows')), findsOneWidget);
  });

  testWidgets('a number shows every digit it holds', (tester) async {
    const digits = [
      (0.0004, '0.0004'),
      (1.23456, '1.23456'),
      (1234567.0, '1,234,567'),
      (-2.5e-7, '-0.00000025'),
      (1 / 3000, '0.0003333333333333333'),
      (1e-20, '0.00000000000000000001'),
      (1e20, '100000000000000000000'),
      (84.0, '84'),
      (double.negativeInfinity, '-∞'),
      (double.nan, 'NaN'),
    ];
    BridgeRow amount(String id, double number) => BridgeRow(
      id: id,
      cells: [
        BridgeFieldCell(
          field: 'amount',
          value: cell(BridgeCellKind.number, number: number),
        ),
      ],
      computed: const [],
    );
    await kit.show(tester, [
      TablePart([for (final (number, text) in digits) amount(text, number)]),
    ]);
    for (final (_, text) in digits) {
      expect(kit.inTable(find.text(text)), findsOneWidget, reason: text);
    }
  });

  testWidgets('an unknown Block stays as a placeholder', (tester) async {
    await kit.show(tester, [
      const TextPart('Above'),
      const UnknownPart(boundary: true),
      const TextPart('Below'),
    ]);
    expect(
      find.text('This Block needs a newer version of Liquid.'),
      findsOneWidget,
    );
    expect(find.byKey(const Key('section-divider')), findsOneWidget);
    expect(find.byType(TextBlock), findsNWidgets(2));
    expect(
      kit.fake.asked.where(
        (query) =>
            query.kind == BridgeQueryKind.text &&
            query.block == 'unknown-block',
      ),
      isEmpty,
      reason: 'never read as text',
    );
  });

  for (final other in [TablePart(expenses), const UnknownPart()]) {
    final name = other is TablePart ? 'a Table' : 'a placeholder';

    testWidgets('the caret passes $name, and ⌫ below it joins nothing', (
      tester,
    ) async {
      await kit.show(tester, [
        const TextPart('Above'),
        other,
        const TextPart('Below'),
      ]);
      await tester.tap(block(1));
      await tester.pump();
      field(tester, 1).controller!.selection = const TextSelection.collapsed(
        offset: 0,
      );
      await tester.pump();
      await key(tester, LogicalKeyboardKey.arrowUp);
      expect(field(tester, 0).focusNode!.hasFocus, isTrue);
      expect(field(tester, 0).controller!.selection.baseOffset, 5);

      await key(tester, LogicalKeyboardKey.arrowRight);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
      expect(field(tester, 1).controller!.selection.baseOffset, 0);

      await key(tester, LogicalKeyboardKey.backspace);
      expect(kit.fake.textsOf(kit.page), ['Above', 'Below']);
      expect(kit.applied(BridgeMutationKind.joinTextBlock), isEmpty);
      expect(kit.workspace.problem.value, isNull);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
    });
  }

  testWidgets('+ New adds a row after the last and shows it', (tester) async {
    await kit.show(tester, [TablePart(namedRows(20))]);
    await tap(tester, newRow);
    final add = kit.applied(BridgeMutationKind.addRow).single;
    expect((add.last, add.after), (true, null), reason: 'the core places it');
    final table = kit.fake.views[kit.page]!.single.block;
    expect(kit.fake.rows[table], hasLength(21));
    expect(kit.inTable(find.text('21 rows')), findsOneWidget);
    final rows = rowsScroll(tester);
    expect(rows.offset, rows.position.maxScrollExtent);
    expect(rows.offset, greaterThan(0));
  });

  testWidgets(
    '+ in the header adds a field of the chosen kind after the last',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await openPanel(tester);
      expect(panel, findsOneWidget);
      for (final kind in ['Text', 'Number', 'Select', 'Date', 'Checkbox']) {
        expect(
          find.descendant(of: panel, matching: find.text(kind)),
          findsOneWidget,
          reason: kind,
        );
      }
      await tester.enterText(
        find.descendant(of: panel, matching: find.byType(TextField)),
        'Tip',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      final added = kit.applied(BridgeMutationKind.addField).single;
      expect(added.last, isTrue);
      expect(added.text, 'Tip');
      expect(added.fieldKind, BridgeFieldKind.number);
      expect(panel, findsNothing);
      expect(kit.inTable(find.text('Tip')), findsOneWidget);
      final sideways = sidewaysOf(tester);
      expect(sideways.offset, greaterThan(0), reason: 'the new field shows');
      expect(sideways.offset, sideways.position.maxScrollExtent);
    },
  );

  testWidgets('a nameless field takes its kind and is added', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    await tap(tester, find.descendant(of: panel, matching: find.text('Date')));
    final added = kit.applied(BridgeMutationKind.addField).single;
    expect(added.text, 'Date');
    expect(added.fieldKind, BridgeFieldKind.date);
    expect(panel, findsNothing);
  });

  testWidgets('⏎ pressed again while a field is added adds it once', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    final gate = gateOn(BridgeMutationKind.addField);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await releaseGate(tester, gate);
    expect(kit.applied(BridgeMutationKind.addField), hasLength(1));
  });

  testWidgets(
    'a Table without fields still shows its rows and offers + to add one',
    (tester) async {
      await kit.show(tester, [TablePart(expenses, fields: const [])]);
      expect(tester.takeException(), isNull);
      expect(kit.inTable(find.text('Expenses')), findsOneWidget);
      expect(kit.inTable(find.text('2 rows')), findsOneWidget);
      await openPanel(tester);
      await key(tester, LogicalKeyboardKey.enter);
      expect(kit.applied(BridgeMutationKind.addField).single.last, isTrue);
      expect(kit.inTable(find.text('Text')), findsOneWidget);
    },
  );

  testWidgets('a Group menu with many Select fields scrolls to the last one', (
    tester,
  ) async {
    await kit.show(tester, [
      TablePart(
        const [],
        fields: [
          for (var number = 1; number <= 20; number++)
            BridgeField(
              id: 'f$number',
              name: 'Choice $number',
              kind: BridgeFieldKind.select,
              options: const [],
              formula: '',
            ),
        ],
      ),
    ]);
    await tester.tap(find.byKey(const Key('table group')));
    await tester.pumpAndSettle();
    final menu = find.byKey(const Key('table query menu'));
    final last = find.descendant(of: menu, matching: find.text('Choice 20'));
    await tester.scrollUntilVisible(
      last,
      40,
      scrollable: find.descendant(of: menu, matching: find.byType(Scrollable)),
    );
    expect(tester.takeException(), isNull);
    await tester.tap(last);
    await tester.pumpAndSettle();
    expect(
      find.descendant(
        of: find.byKey(const Key('table group')),
        matching: find.text('Choice 20'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('two quick + New add both rows last', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final gate = gateOn(BridgeMutationKind.addRow);
    await tester.tap(newRow);
    await tester.pump();
    await tester.tap(newRow);
    await tester.pump();
    await releaseGate(tester, gate);
    final table = kit.fake.rows[kit.fake.views[kit.page]!.single.block]!;
    expect(table, hasLength(4));
    expect(kit.applied(BridgeMutationKind.addRow).map((add) => add.last), [
      true,
      true,
    ]);
    expect(table.take(2).map((row) => row.id), ['r1', 'r2']);
  });

  testWidgets(
    'a field or a row removed before its read leaves + and + New working',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final table = kit.fake.views[kit.page]!.single.block;
      final reads = holdDatabaseReads();
      Future<void> add() async {
        await openPanel(tester);
        await key(tester, LogicalKeyboardKey.enter);
      }

      await add();
      kit.fake.databases[table] = const BridgeDatabase(
        name: 'Expenses',
        fields: expenseFields,
      );
      kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
      await tester.pump();
      await releaseReads(tester, reads);
      await add();
      expect(kit.workspace.problem.value, isNull);
      expect(shownFields(), hasLength(6));

      final rowReads = holdDatabaseReads();
      await tap(tester, newRow);
      await tester.pump();
      kit.fake.rows[table]!.removeAt(0);
      await releaseReads(tester, rowReads);
      expect(kit.inTable(find.text('2 rows')), findsOneWidget);
      kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
      await tester.pumpAndSettle();
      kit.fake.putRows(kit.page, table, [kit.fake.rows[table]!.first]);
      await tester.pumpAndSettle();
      await tap(tester, newRow);
      expect(kit.workspace.problem.value, isNull);
      expect(kit.fake.rows[table], hasLength(2));
    },
  );

  testWidgets('a reopened panel adds its field and stays', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final gate = await addThenReopen(tester);
    final answered = kit.fake.answered = Completer<void>();
    await releaseGate(tester, gate);
    expect(panel, findsOneWidget, reason: 'stays while the add is answered');
    kit.fake.answered = null;
    answered.complete();
    await tester.pumpAndSettle();
    expect(panel, findsOneWidget, reason: 'the reopened panel stays');
    await key(tester, LogicalKeyboardKey.enter);
    expect(shownFields(), hasLength(7));
  });

  testWidgets('a field added while another is on its way', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final gate = await addThenReopen(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await releaseGate(tester, gate);
    expect(shownFields(), hasLength(7));
    expect(panel, findsNothing);
  });
}
