import 'dart:async';

import 'package:flutter/cupertino.dart';
import 'package:flutter/gestures.dart';
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

  const shopped = BridgeDatabase(
    name: 'Expenses',
    fields: [
      ...expenseFields,
      BridgeField(
        id: 'shop',
        name: 'Shop',
        kind: BridgeFieldKind.text,
        options: [],
      ),
    ],
  );

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
      ),
  ];

  ScrollController sidewaysOf(WidgetTester tester) => tester
      .widget<SingleChildScrollView>(find.byKey(const Key('table columns')))
      .controller!;

  Future<void> openPanel(WidgetTester tester) async {
    await tester.tap(kit.inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
  }

  Finder block(int index) => find.descendant(
    of: find.byType(TextBlock).at(index),
    matching: find.byType(TextField),
  );
  TextField field(WidgetTester tester, int index) =>
      tester.widget<TextField>(block(index));

  Future<void> caret(WidgetTester tester, int index, int offset) async {
    await tester.tap(block(index));
    await tester.pump();
    field(tester, index).controller!.selection = TextSelection.collapsed(
      offset: offset,
    );
    await tester.pump();
  }

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
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

  testWidgets('a Table with only its Name shows its header', (tester) async {
    await kit.show(tester, [
      TablePart(expenses, fields: [expenseFields.first]),
    ]);
    expect(
      tester.getSize(kit.inTable(find.text('Name'))).height,
      greaterThan(0),
    );
    expect(
      tester.getRect(kit.inTable(find.text('Name'))).bottom,
      lessThan(tester.getRect(kit.inTable(find.text('Groceries'))).top),
    );
  });

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

  for (final (locale, number, day) in [
    (const Locale('de', 'DE'), '84,2', '1. Nov. 2026'),
    (const Locale('en', 'GB'), '84.2', '1 Nov 2026'),
    (const Locale('es', 'MX'), '84.2', '1 nov 2026'),
    (const Locale('bo'), '84.2', 'Nov 1, 2026'),
  ]) {
    testWidgets('a person in $locale reads numbers and days as there', (
      tester,
    ) async {
      tester.platformDispatcher.localesTestValue = [locale];
      addTearDown(tester.platformDispatcher.clearLocalesTestValue);
      await kit.show(tester, [TablePart(expenses)], app: true);
      expect(kit.inTable(find.text(number)), findsOneWidget);
      expect(kit.inTable(find.text(day)), findsOneWidget);
    });
  }

  testWidgets('Serbian in Latin reads days in Latin', (tester) async {
    tester.platformDispatcher.localesTestValue = [
      Locale.fromSubtags(
        languageCode: 'sr',
        scriptCode: 'Latn',
        countryCode: 'RS',
      ),
    ];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await kit.show(tester, [TablePart(expenses)], app: true);
    expect(kit.inTable(find.textContaining('nov')), findsOneWidget);
    expect(kit.inTable(find.textContaining('нов')), findsNothing);
  });

  testWidgets('a language Cupertino lacks reads in English', (tester) async {
    tester.platformDispatcher.localesTestValue = const [Locale('ps', 'AF')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await kit.show(tester, [TablePart(expenses)], app: true);
    final context = tester.element(find.byType(TableBlock));
    expect(CupertinoLocalizations.of(context), isNotNull);
    expect(kit.inTable(find.text('Nov 1, 2026')), findsOneWidget);
  });

  testWidgets('a number shows every digit it holds', (tester) async {
    BridgeRow amount(String id, double number) => BridgeRow(
      id: id,
      cells: [
        BridgeFieldCell(
          field: 'amount',
          value: cell(BridgeCellKind.number, number: number),
        ),
      ],
    );
    await kit.show(tester, [
      TablePart([
        amount('r1', 0.0004),
        amount('r2', 1.23456),
        amount('r3', 1234567),
        amount('r4', -2.5e-7),
        amount('r5', 1 / 3000),
        amount('r6', 1e-20),
        amount('r7', 1e20),
        amount('r8', 84),
        amount('r9', double.negativeInfinity),
        amount('r10', double.nan),
      ]),
    ]);
    for (final text in [
      '0.0004',
      '1.23456',
      '1,234,567',
      '-0.00000025',
      '0.0003333333333333333',
      '0.00000000000000000001',
      '100000000000000000000',
      '84',
      '-∞',
      'NaN',
    ]) {
      expect(kit.inTable(find.text(text)), findsOneWidget, reason: text);
    }
  });

  testWidgets('a Table without fields still shows its rows', (tester) async {
    await kit.show(tester, [TablePart(expenses, fields: const [])]);
    expect(tester.takeException(), isNull);
    expect(kit.inTable(find.text('Expenses')), findsOneWidget);
    expect(kit.inTable(find.text('2 rows')), findsOneWidget);
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

    testWidgets('the caret passes $name', (tester) async {
      await kit.show(tester, [
        const TextPart('Above'),
        other,
        const TextPart('Below'),
      ]);
      await caret(tester, 1, 0);
      await key(tester, LogicalKeyboardKey.arrowUp);
      expect(field(tester, 0).focusNode!.hasFocus, isTrue);
      expect(field(tester, 0).controller!.selection.baseOffset, 5);

      await key(tester, LogicalKeyboardKey.arrowRight);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
      expect(field(tester, 1).controller!.selection.baseOffset, 0);
    });

    testWidgets('⌫ below $name joins nothing', (tester) async {
      await kit.show(tester, [
        const TextPart('Above'),
        other,
        const TextPart('Below'),
      ]);
      await caret(tester, 1, 0);
      await key(tester, LogicalKeyboardKey.backspace);
      expect(kit.fake.textsOf(kit.page), ['Above', 'Below']);
      expect(
        kit.fake.applied.where(
          (applied) => applied.kind == BridgeMutationKind.joinTextBlock,
        ),
        isEmpty,
      );
      expect(kit.workspace.problem.value, isNull);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
    });
  }

  testWidgets('a numbered list after a Table starts at 1', (tester) async {
    await kit.show(tester, [
      const TextPart('One', BridgeTextStyle.numbered),
      TablePart(expenses),
      const TextPart('Two', BridgeTextStyle.numbered),
    ]);
    expect(find.text('1.'), findsNWidgets(2));
    expect(find.text('2.'), findsNothing);
    final table = kit.fake.views[kit.page]![1].block;
    expect(
      kit.fake.asked.where(
        (query) => query.kind == BridgeQueryKind.text && query.block == table,
      ),
      isEmpty,
      reason: 'the run ends at the Table, which is never read as text',
    );
  });

  testWidgets('+ New adds a row after the last and shows it', (tester) async {
    await kit.show(tester, [TablePart(namedRows(20))]);
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pumpAndSettle();
    final add = kit.applied(BridgeMutationKind.addRow).single;
    expect((add.last, add.after), (true, null), reason: 'the core places it');
    final table = kit.fake.views[kit.page]!.single.block;
    expect(kit.fake.rows[table], hasLength(21));
    expect(kit.inTable(find.text('21 rows')), findsOneWidget);
    final rows = tester.widget<ListView>(find.byKey(const Key('table rows')));
    expect(rows.controller!.offset, rows.controller!.position.maxScrollExtent);
    expect(rows.controller!.offset, greaterThan(0));
  });

  testWidgets('+ New in an empty Table adds its first row', (tester) async {
    await kit.show(tester, [const TablePart([])]);
    expect(kit.inTable(find.textContaining('rows')), findsNothing);
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.addRow).single.last, isTrue);
    expect(kit.inTable(find.text('1 row')), findsOneWidget);
  });

  final panel = find.byKey(const Key('field panel'));

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
    await tester.tap(find.descendant(of: panel, matching: find.text('Date')));
    await tester.pumpAndSettle();
    final added = kit.applied(BridgeMutationKind.addField).single;
    expect(added.text, 'Date');
    expect(added.fieldKind, BridgeFieldKind.date);
    expect(panel, findsNothing);
  });

  testWidgets('esc closes the field panel and adds nothing', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
    expect(kit.applied(BridgeMutationKind.addField), isEmpty);
  });

  testWidgets('scrolling the Page closes the field panel', (tester) async {
    await kit.show(tester, [
      TablePart(expenses),
      for (var number = 0; number < 40; number++) TextPart('Line $number'),
    ]);
    await openPanel(tester);
    tester
        .state<ScrollableState>(
          find
              .descendant(
                of: find.byType(CustomScrollView),
                matching: find.byType(Scrollable),
              )
              .first,
        )
        .position
        .jumpTo(200);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets('resizing the window closes the field panel', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    tester.view.physicalSize = tester.view.physicalSize / 2;
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets(
    'a click beside a field name keeps its keys',
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      await openPanel(tester);
      await tester.tap(
        find.descendant(of: panel, matching: find.text('Kind')),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(
        kit.applied(BridgeMutationKind.addField).single.fieldKind,
        BridgeFieldKind.number,
      );
    },
  );

  testWidgets('⏎ pressed again while a field is added adds it once', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.addField), hasLength(1));
  });

  testWidgets('a refused field keeps the panel; click closes', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await openPanel(tester);
    kit.fake.refuseKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(panel, findsOneWidget);
    expect(kit.workspace.problem.value, isNotNull);
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets('a Table without fields offers + to add one', (tester) async {
    await kit.show(tester, [TablePart(expenses, fields: const [])]);
    await openPanel(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.addField).single.last, isTrue);
    expect(kit.inTable(find.text('Text')), findsOneWidget);
  });

  testWidgets('two quick + New add both rows last', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addRow};
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pump();
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    final table = kit.fake.rows[kit.fake.views[kit.page]!.single.block]!;
    expect(table, hasLength(4));
    expect(kit.applied(BridgeMutationKind.addRow).map((add) => add.last), [
      true,
      true,
    ]);
    expect(table.take(2).map((row) => row.id), ['r1', 'r2']);
  });

  testWidgets('a sideways scroll closes the field panel', (tester) async {
    await kit.show(tester, [
      TablePart(expenses, fields: [...expenseFields, ...textFields(9)]),
    ]);
    final sideways = sidewaysOf(tester);
    sideways.jumpTo(sideways.position.maxScrollExtent);
    await tester.pumpAndSettle();
    await openPanel(tester);
    expect(panel, findsOneWidget);
    sideways.jumpTo(0);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets('a quick second + New shows its own row', (tester) async {
    await kit.show(tester, [TablePart(namedRows(20))]);
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.database};
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pump();
    kit.fake.deferred = null;
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addRow};
    await tester.tap(kit.inTable(find.text('New')));
    await tester.pump();
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(kit.inTable(find.text('21 rows')), findsOneWidget);
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(kit.inTable(find.text('22 rows')), findsOneWidget);
    final rows = tester.widget<ListView>(find.byKey(const Key('table rows')));
    expect(rows.controller!.offset, (22 - 10) * 34.0);
  });

  testWidgets('a field removed before read leaves + working', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    final reads = kit.fake.deferred = [];
    kit.fake.deferredKinds = {BridgeQueryKind.database};
    Future<void> add() async {
      await openPanel(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
    }

    await add();
    kit.fake.databases[table] = const BridgeDatabase(
      name: 'Expenses',
      fields: expenseFields,
    );
    kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
    await tester.pump();
    kit.fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    await add();
    expect(kit.workspace.problem.value, isNull);
    expect(kit.fake.databases[table]!.fields, hasLength(6));
  });

  testWidgets(
    'a row removed elsewhere while + New is read leaves + New working',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final table = kit.fake.views[kit.page]!.single.block;
      final reads = kit.fake.deferred = [];
      kit.fake.deferredKinds = {BridgeQueryKind.database};
      await tester.tap(kit.inTable(find.text('New')));
      await tester.pump();
      kit.fake.rows[table]!.removeAt(0);
      kit.fake.deferred = null;
      for (final read in reads) {
        read.complete();
      }
      await tester.pumpAndSettle();
      expect(kit.inTable(find.text('2 rows')), findsOneWidget);
      kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
      await tester.pumpAndSettle();
      kit.fake.putRows(kit.page, table, [kit.fake.rows[table]!.first]);
      await tester.pumpAndSettle();
      await tester.tap(kit.inTable(find.text('New')));
      await tester.pumpAndSettle();
      expect(kit.workspace.problem.value, isNull);
      expect(kit.fake.rows[table], hasLength(2));
    },
  );

  testWidgets('a reopened panel adds its field and stays', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);

    await openPanel(tester);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await openPanel(tester);
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(panel, findsOneWidget, reason: 'the reopened panel stays');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    final fields =
        kit.fake.databases[kit.fake.views[kit.page]!.single.block]!.fields;
    expect(fields, hasLength(7));
  });

  testWidgets('a field added while another is on its way', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);

    await openPanel(tester);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await openPanel(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    final fields =
        kit.fake.databases[kit.fake.views[kit.page]!.single.block]!.fields;
    expect(fields, hasLength(7));
    expect(panel, findsNothing);
  });

  testWidgets('+, + New and field kinds are buttons', (tester) async {
    final semantics = tester.ensureSemantics();
    await kit.show(tester, [TablePart(expenses)]);
    expect(
      tester.getSemantics(kit.inTable(find.byKey(const Key('add field')))),
      matchesSemantics(label: 'Add field', isButton: true, hasTapAction: true),
    );
    expect(
      tester.getSemantics(kit.inTable(find.text('New'))),
      matchesSemantics(label: 'New', isButton: true, hasTapAction: true),
    );
    await openPanel(tester);
    expect(
      tester.getSemantics(
        find.descendant(of: panel, matching: find.text('Text')),
      ),
      matchesSemantics(
        label: 'Text',
        isButton: true,
        hasTapAction: true,
        hasSelectedState: true,
        isSelected: true,
      ),
    );
    semantics.dispose();
  });

  testWidgets(
    'a row this device added and one removed elsewhere leave the rows where the person reads',
    (tester) async {
      await kit.show(tester, [TablePart(namedRows(20))]);
      final table = kit.fake.views[kit.page]!.single.block;
      final reads = kit.fake.deferred = [];
      kit.fake.deferredKinds = {BridgeQueryKind.database};
      await tester.tap(kit.inTable(find.text('New')));
      await tester.pump();
      kit.fake.rows[table]!.removeAt(0);
      kit.fake.deferred = null;
      for (final read in reads) {
        read.complete();
      }
      await tester.pumpAndSettle();
      final rows = tester
          .widget<ListView>(find.byKey(const Key('table rows')))
          .controller!;
      rows.jumpTo(0);
      await tester.pumpAndSettle();
      kit.fake.putRows(kit.page, table, [
        ...kit.fake.rows[table]!,
        namedRow(99),
      ]);
      await tester.pumpAndSettle();
      expect(rows.offset, 0, reason: 'the rows stay where the person reads');
    },
  );

  testWidgets('the field panel closes when + leaves the view', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await openPanel(tester);
    expect(panel, findsOneWidget);
    kit.fake.databases[table] = shopped;
    kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets('a reopened panel stays while its field is added', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);

    await openPanel(tester);
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await openPanel(tester);
    final answered = kit.fake.answered = Completer<void>();
    kit.fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    kit.fake.answered = null;
    answered.complete();
    await tester.pumpAndSettle();
    expect(panel, findsOneWidget);
  });

  testWidgets(
    'a refused field crossed by one added elsewhere closes the panel whose + left',
    (tester) async {
      await kit.show(tester, [TablePart(expenses)]);
      final table = kit.fake.views[kit.page]!.single.block;
      await openPanel(tester);
      final gate = kit.fake.gate = Completer<void>();
      kit.fake.gatedKinds = {BridgeMutationKind.addField};
      kit.fake.refuseKinds = {BridgeMutationKind.addField};
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      kit.fake.databases[table] = shopped;
      kit.fake.putRows(kit.page, table, [...kit.fake.rows[table]!]);
      await tester.pumpAndSettle();
      kit.fake.gate = null;
      gate.complete();
      await tester.pumpAndSettle();
      expect(kit.workspace.problem.value, isNotNull);
      expect(panel, findsNothing);
    },
  );
}
