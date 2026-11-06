import 'dart:async';

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

/// A part of a Page to open: a Text Block, a Table or a Block of a kind Liquid does not know.
sealed class _Part {
  const _Part();
}

class _Text extends _Part {
  const _Text(this.text, [this.style = BridgeTextStyle.paragraph]);
  final String text;
  final BridgeTextStyle style;
}

class _Table extends _Part {
  const _Table(this.rows, {this.fields = _fields});
  final List<BridgeRow> rows;
  final List<BridgeField> fields;
}

class _Unknown extends _Part {
  const _Unknown({this.boundary = false});
  final bool boundary;
}

BridgeCell _cell(
  BridgeCellKind kind, {
  String? text,
  double number = 0,
  bool checked = false,
}) => BridgeCell(kind: kind, text: text, number: number, checked: checked);

const _fields = [
  BridgeField(
    id: 'name',
    name: 'Name',
    kind: BridgeFieldKind.text,
    options: [],
  ),
  BridgeField(
    id: 'date',
    name: 'Date',
    kind: BridgeFieldKind.date,
    options: [],
  ),
  BridgeField(
    id: 'category',
    name: 'Category',
    kind: BridgeFieldKind.select,
    options: [
      BridgeOption(id: 'food', name: 'Food', color: 5),
      BridgeOption(id: 'home', name: 'Home', color: 6),
    ],
  ),
  BridgeField(
    id: 'amount',
    name: 'Amount',
    kind: BridgeFieldKind.number,
    options: [],
  ),
  BridgeField(
    id: 'paid',
    name: 'Paid',
    kind: BridgeFieldKind.checkbox,
    options: [],
  ),
];

BridgeRow _row(
  String id,
  String name,
  String day,
  String option,
  double amount,
  bool paid,
) => BridgeRow(
  id: id,
  cells: [
    BridgeFieldCell(
      field: 'name',
      value: _cell(BridgeCellKind.text, text: name),
    ),
    BridgeFieldCell(
      field: 'date',
      value: _cell(BridgeCellKind.date, text: day),
    ),
    BridgeFieldCell(
      field: 'category',
      value: _cell(BridgeCellKind.select, text: option),
    ),
    BridgeFieldCell(
      field: 'amount',
      value: _cell(BridgeCellKind.number, number: amount),
    ),
    BridgeFieldCell(
      field: 'paid',
      value: _cell(BridgeCellKind.checkbox, checked: paid),
    ),
  ],
);

final _expenses = [
  _row('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
  _row('r2', 'Electricity', '2026-10-30', 'home', 96, false),
];

BridgeRow _named(int number) => BridgeRow(
  id: 'r$number',
  cells: [
    BridgeFieldCell(
      field: 'name',
      value: _cell(BridgeCellKind.text, text: 'Row $number'),
    ),
  ],
);

void main() {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  /// Opens a Page of `parts` in Liquid, or in a bare frame unless `app`.
  Future<void> show(
    WidgetTester tester,
    List<_Part> parts, {
    bool app = false,
    void Function()? before,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    fake = FakeWorkspace();
    before?.call();
    workspace = WorkspaceState(fake);
    await workspace.start();
    page = workspace.open.value!.page;
    String? after;
    for (final part in parts) {
      switch (part) {
        case _Text(:final text, :final style):
          final added = await workspace.apply(
            mutation(
              BridgeMutationKind.addTextBlock,
              page: page,
              after: after,
              text: text,
            ),
          );
          after = added.view;
          fake.styles[added.block!] = style;
        case _Table(:final rows, :final fields):
          final added = await workspace.apply(
            mutation(
              BridgeMutationKind.addDatabaseBlock,
              page: page,
              after: after,
              text: 'Expenses',
            ),
          );
          after = added.view;
          fake.databases[added.block!] = BridgeDatabase(
            name: 'Expenses',
            fields: fields,
          );
          fake.rows[added.block!] = [...rows];
        case _Unknown(:final boundary):
          final views = fake.views[page] ??= [];
          final at = after == null
              ? 0
              : views.indexWhere((view) => view.id == after) + 1;
          views.insert(
            at,
            BridgeView(
              id: 'unknown-view',
              block: 'unknown-block',
              boundary: boundary,
              kind: BridgeBlockKind.unknown,
            ),
          );
          after = 'unknown-view';
      }
    }
    workspace.open.value = OpenPage(workspace.open.value!.area, page);
    await tester.pumpWidget(
      app
          ? LiquidApp(launch: const Launch.problem(null), workspace: workspace)
          : MaterialApp(
              theme: liquidTheme(Brightness.light),
              home: AgentShell(workspace: workspace),
            ),
    );
    await tester.pumpAndSettle();
    addTearDown(workspace.dispose);
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

  Finder inTable(Finder finder) =>
      find.descendant(of: find.byType(TableBlock), matching: finder);

  testWidgets('a Table shows its name, fields and rows', (tester) async {
    await show(tester, [const _Text('October'), _Table(_expenses)]);
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
      expect(inTable(find.text(text)), findsOneWidget, reason: text);
    }
    expect(
      inTable(find.byIcon(LucideIcons.check)),
      findsOneWidget,
      reason: 'one ticked checkbox',
    );
    expect(
      tester.getRect(inTable(find.text('96'))).right,
      greaterThan(tester.getRect(inTable(find.text('Amount'))).right - 12),
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
      await show(tester, [
        _Table([for (var number = 1; number <= 2000; number++) _named(number)]),
        const _Text('Below'),
      ]);
      expect(inTable(find.text('2,000 rows')), findsOneWidget);
      expect(inTable(find.text('Row 1')), findsOneWidget);
      expect(inTable(find.text('Row 40')), findsNothing);
      final rows = fake.asked.where(
        (query) => query.kind == BridgeQueryKind.rows,
      );
      expect(rows, isNotEmpty);
      expect(rows.every((query) => query.limit <= 100), isTrue);
      expect(rows.every((query) => query.start == 0), isTrue);
      expect(
        tester.getSize(find.byType(TableBlock)).height,
        lessThan(32 + 11 * 34 + 2 * 40),
        reason: 'about ten rows high',
      );

      await tester.scrollUntilVisible(
        inTable(find.text('Row 150')),
        300,
        scrollable: find.descendant(
          of: find.byKey(const Key('table rows')),
          matching: find.byType(Scrollable),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        fake.asked.any(
          (query) => query.kind == BridgeQueryKind.rows && query.start == 100,
        ),
        isTrue,
        reason: 'the next window, once it scrolls in',
      );
      expect(inTable(find.text('2,000 rows')), findsOneWidget);
      expect(find.text('Below'), findsOneWidget, reason: 'the Page goes on');
    },
  );

  testWidgets(
    'a wide Table builds the fields in view, keeping the first one in view',
    (tester) async {
      await show(tester, [
        _Table(
          _expenses,
          fields: [
            _fields.first,
            for (var number = 1; number < 300; number++)
              BridgeField(
                id: 'f$number',
                name: 'Field $number',
                kind: BridgeFieldKind.text,
                options: const [],
              ),
          ],
        ),
      ]);
      expect(inTable(find.text('Field 1')), findsOneWidget);
      expect(inTable(find.text('Field 299')), findsNothing);

      await tester.drag(inTable(find.text('Field 2')), const Offset(-60000, 0));
      await tester.pumpAndSettle();
      expect(inTable(find.text('Field 299')), findsOneWidget);
      expect(inTable(find.text('Field 2')), findsNothing);
      expect(
        inTable(find.text('Groceries')),
        findsOneWidget,
        reason: 'the first field stays in view',
      );
      expect(
        tester.getRect(inTable(find.text('Name'))).left,
        lessThan(tester.getRect(inTable(find.text('Field 299'))).left),
      );
    },
  );

  testWidgets('a new Table with only its Name field shows its header', (
    tester,
  ) async {
    await show(tester, [
      _Table(_expenses, fields: [_fields.first]),
    ]);
    expect(tester.getSize(inTable(find.text('Name'))).height, greaterThan(0));
    expect(
      tester.getRect(inTable(find.text('Name'))).bottom,
      lessThan(tester.getRect(inTable(find.text('Groceries'))).top),
    );
  });

  testWidgets('a Table follows its rows as they change', (tester) async {
    await show(tester, [_Table(_expenses)]);
    final table = fake.views[page]!.single.block;
    fake.putRows(page, table, [..._expenses, _named(3)]);
    await tester.pumpAndSettle();
    expect(inTable(find.text('Row 3')), findsOneWidget);
    expect(inTable(find.text('3 rows')), findsOneWidget);
  });

  for (final (locale, number, day) in [
    (const Locale('de', 'DE'), '84,2', '1. Nov. 2026'),
    (const Locale('en', 'GB'), '84.2', '1 Nov 2026'),
    (const Locale('es', 'MX'), '84.2', '1 nov 2026'),
    // Material knows Tibetan, but intl has no formats for it.
    (const Locale('bo'), '84.2', 'Nov 1, 2026'),
  ]) {
    testWidgets('a person in $locale reads numbers and days as there', (
      tester,
    ) async {
      tester.platformDispatcher.localesTestValue = [locale];
      addTearDown(tester.platformDispatcher.clearLocalesTestValue);
      await show(tester, [_Table(_expenses)], app: true);
      expect(inTable(find.text(number)), findsOneWidget);
      expect(inTable(find.text(day)), findsOneWidget);
    });
  }

  testWidgets('a person who writes Serbian in Latin reads days in Latin', (
    tester,
  ) async {
    tester.platformDispatcher.localesTestValue = [
      Locale.fromSubtags(
        languageCode: 'sr',
        scriptCode: 'Latn',
        countryCode: 'RS',
      ),
    ];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await show(tester, [_Table(_expenses)], app: true);
    expect(inTable(find.textContaining('nov')), findsOneWidget);
    expect(inTable(find.textContaining('нов')), findsNothing);
  });

  testWidgets('a language Cupertino does not know reads in English', (
    tester,
  ) async {
    // Material knows Pashto, but Cupertino, whose texts macOS's text menu shows, does not.
    tester.platformDispatcher.localesTestValue = const [Locale('ps', 'AF')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    await show(tester, [_Table(_expenses)], app: true);
    final context = tester.element(find.byType(TableBlock));
    expect(CupertinoLocalizations.of(context), isNotNull);
    expect(inTable(find.text('Nov 1, 2026')), findsOneWidget);
  });

  testWidgets('a number shows every digit it holds', (tester) async {
    BridgeRow amount(String id, double number) => BridgeRow(
      id: id,
      cells: [
        BridgeFieldCell(
          field: 'amount',
          value: _cell(BridgeCellKind.number, number: number),
        ),
      ],
    );
    await show(tester, [
      _Table([
        amount('r1', 0.0004),
        amount('r2', 1.23456),
        amount('r3', 1234567),
        amount('r4', -2.5e-7),
        amount('r5', 1 / 3000),
        amount('r6', 1e-20),
        amount('r7', 1e20),
        amount('r8', 84),
        // Another replica's or a damaged document may hold what the core refuses.
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
      expect(inTable(find.text(text)), findsOneWidget, reason: text);
    }
  });

  testWidgets('a Table whose fields are all removed still shows its rows', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses, fields: const [])]);
    expect(tester.takeException(), isNull);
    expect(inTable(find.text('Expenses')), findsOneWidget);
    expect(inTable(find.text('2 rows')), findsOneWidget);
  });

  testWidgets(
    'a Block of a kind Liquid does not know keeps its place as a placeholder',
    (tester) async {
      await show(tester, [
        const _Text('Above'),
        const _Unknown(boundary: true),
        const _Text('Below'),
      ]);
      expect(
        find.text('This Block needs a newer version of Liquid.'),
        findsOneWidget,
      );
      expect(find.byKey(const Key('section-divider')), findsOneWidget);
      expect(find.byType(TextBlock), findsNWidgets(2));
      expect(
        fake.asked.where(
          (query) =>
              query.kind == BridgeQueryKind.text &&
              query.block == 'unknown-block',
        ),
        isEmpty,
        reason: 'never read as text',
      );
    },
  );

  for (final other in [_Table(_expenses), const _Unknown()]) {
    final name = other is _Table ? 'a Table' : 'a placeholder';

    testWidgets('the caret passes $name', (tester) async {
      await show(tester, [const _Text('Above'), other, const _Text('Below')]);
      await caret(tester, 1, 0);
      await key(tester, LogicalKeyboardKey.arrowUp);
      expect(field(tester, 0).focusNode!.hasFocus, isTrue);
      expect(field(tester, 0).controller!.selection.baseOffset, 5);

      await key(tester, LogicalKeyboardKey.arrowRight);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
      expect(field(tester, 1).controller!.selection.baseOffset, 0);
    });

    testWidgets('⌫ below $name joins nothing', (tester) async {
      await show(tester, [const _Text('Above'), other, const _Text('Below')]);
      await caret(tester, 1, 0);
      await key(tester, LogicalKeyboardKey.backspace);
      expect(fake.textsOf(page), ['Above', 'Below']);
      expect(
        fake.applied.where(
          (applied) => applied.kind == BridgeMutationKind.joinTextBlock,
        ),
        isEmpty,
      );
      expect(workspace.problem.value, isNull);
      expect(field(tester, 1).focusNode!.hasFocus, isTrue);
    });
  }

  testWidgets('a numbered list after a Table starts at 1 again', (
    tester,
  ) async {
    await show(tester, [
      const _Text('One', BridgeTextStyle.numbered),
      _Table(_expenses),
      const _Text('Two', BridgeTextStyle.numbered),
    ]);
    expect(find.text('1.'), findsNWidgets(2));
    expect(find.text('2.'), findsNothing);
    final table = fake.views[page]![1].block;
    expect(
      fake.asked.where(
        (query) => query.kind == BridgeQueryKind.text && query.block == table,
      ),
      isEmpty,
      reason: 'the run ends at the Table, which is never read as text',
    );
  });

  Iterable<BridgeMutation> applied(BridgeMutationKind kind) =>
      fake.applied.where((applied) => applied.kind == kind);

  testWidgets('+ New adds a row after the last one and shows it', (
    tester,
  ) async {
    await show(tester, [
      _Table([for (var number = 1; number <= 20; number++) _named(number)]),
    ]);
    await tester.tap(inTable(find.text('New')));
    await tester.pumpAndSettle();
    expect(applied(BridgeMutationKind.addRow).single.after, 'r20');
    final table = fake.views[page]!.single.block;
    expect(fake.rows[table], hasLength(21));
    expect(inTable(find.text('21 rows')), findsOneWidget);
    final rows = tester.widget<ListView>(find.byKey(const Key('table rows')));
    expect(rows.controller!.offset, rows.controller!.position.maxScrollExtent);
    expect(rows.controller!.offset, greaterThan(0));
  });

  testWidgets('+ New in an empty Table adds its first row', (tester) async {
    await show(tester, [const _Table([])]);
    expect(inTable(find.textContaining('rows')), findsNothing);
    await tester.tap(inTable(find.text('New')));
    await tester.pumpAndSettle();
    expect(applied(BridgeMutationKind.addRow).single.after, isNull);
    expect(inTable(find.text('1 row')), findsOneWidget);
  });

  final panel = find.byKey(const Key('field panel'));

  testWidgets(
    '+ in the header adds a field of the chosen kind after the last',
    (tester) async {
      await show(tester, [_Table(_expenses)]);
      await tester.tap(inTable(find.byKey(const Key('add field'))));
      await tester.pumpAndSettle();
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
      final added = applied(BridgeMutationKind.addField).single;
      expect(added.after, 'paid');
      expect(added.text, 'Tip');
      expect(added.fieldKind, BridgeFieldKind.number);
      expect(panel, findsNothing);
      expect(inTable(find.text('Tip')), findsOneWidget);
      final sideways = tester
          .widget<SingleChildScrollView>(
            inTable(find.byType(SingleChildScrollView)),
          )
          .controller!;
      expect(sideways.offset, greaterThan(0), reason: 'the new field shows');
      expect(sideways.offset, sideways.position.maxScrollExtent);
    },
  );

  testWidgets('a field without a name takes its kind\'s, and a click adds it', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    await tester.tap(inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
    await tester.tap(find.descendant(of: panel, matching: find.text('Date')));
    await tester.pumpAndSettle();
    final added = applied(BridgeMutationKind.addField).single;
    expect(added.text, 'Date');
    expect(added.fieldKind, BridgeFieldKind.date);
    expect(panel, findsNothing);
  });

  testWidgets('esc closes the field panel and adds nothing', (tester) async {
    await show(tester, [_Table(_expenses)]);
    await tester.tap(inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
    expect(applied(BridgeMutationKind.addField), isEmpty);
  });

  testWidgets('⏎ pressed again while a field is added adds it once', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    await tester.tap(inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
    final gate = fake.gate = Completer<void>();
    fake.gatedKinds = {BridgeMutationKind.addField};
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(applied(BridgeMutationKind.addField), hasLength(1));
  });

  testWidgets(
    'a refused field keeps the panel open, a click outside closes it',
    (tester) async {
      await show(tester, [_Table(_expenses)]);
      await tester.tap(inTable(find.byKey(const Key('add field'))));
      await tester.pumpAndSettle();
      fake.refuseKinds = {BridgeMutationKind.addField};
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(panel, findsOneWidget);
      expect(workspace.problem.value, isNotNull);
      await tester.tapAt(const Offset(1300, 850));
      await tester.pumpAndSettle();
      expect(panel, findsNothing);
    },
  );

  testWidgets('a Table without fields offers + to add one', (tester) async {
    await show(tester, [_Table(_expenses, fields: const [])]);
    await tester.tap(inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    final added = applied(BridgeMutationKind.addField).single;
    expect(added.after, isNull);
    expect(inTable(find.text('Text')), findsOneWidget);
  });

  testWidgets('two quick + New add their rows in order', (tester) async {
    await show(tester, [_Table(_expenses)]);
    final gate = fake.gate = Completer<void>();
    fake.gatedKinds = {BridgeMutationKind.addRow};
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    final table = fake.rows[fake.views[page]!.single.block]!;
    expect(table, hasLength(4));
    final adds = applied(BridgeMutationKind.addRow).toList();
    expect(adds.first.after, 'r2');
    expect(adds.last.after, table[2].id, reason: 'after the row just added');
  });

  testWidgets('scrolling a Table sideways closes its field panel', (
    tester,
  ) async {
    await show(tester, [
      _Table(
        _expenses,
        fields: [
          ..._fields,
          for (var number = 1; number < 10; number++)
            BridgeField(
              id: 'f$number',
              name: 'Field $number',
              kind: BridgeFieldKind.text,
              options: const [],
            ),
        ],
      ),
    ]);
    final sideways = tester
        .widget<SingleChildScrollView>(
          inTable(find.byType(SingleChildScrollView)),
        )
        .controller!;
    sideways.jumpTo(sideways.position.maxScrollExtent);
    await tester.pumpAndSettle();
    await tester.tap(inTable(find.byKey(const Key('add field'))));
    await tester.pumpAndSettle();
    expect(panel, findsOneWidget);
    sideways.jumpTo(0);
    await tester.pumpAndSettle();
    expect(panel, findsNothing);
  });

  testWidgets('+ New right after a row was added goes after that row', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    // The Table reads its rows again only after the second click.
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database, BridgeQueryKind.rows};
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    await tester.pump();
    expect(applied(BridgeMutationKind.addRow), hasLength(1));
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    final table = fake.rows[fake.views[page]!.single.block]!;
    final adds = applied(BridgeMutationKind.addRow).toList();
    expect(adds, hasLength(2));
    expect(adds.last.after, table[2].id, reason: 'after the row just added');
  });

  testWidgets('a quick second + New shows its own row', (tester) async {
    await show(tester, [
      _Table([for (var number = 1; number <= 20; number++) _named(number)]),
    ]);
    // The first add's read waits until the second add is on its way.
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database};
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    fake.deferred = null;
    final gate = fake.gate = Completer<void>();
    fake.gatedKinds = {BridgeMutationKind.addRow};
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(inTable(find.text('21 rows')), findsOneWidget);
    fake.gate = null;
    gate.complete();
    await tester.pumpAndSettle();
    expect(inTable(find.text('22 rows')), findsOneWidget);
    final rows = tester.widget<ListView>(find.byKey(const Key('table rows')));
    expect(rows.controller!.offset, (22 - 10) * 34.0);
  });

  testWidgets('+ New before the rows are counted adds after the last', (
    tester,
  ) async {
    late List<Completer<void>> reads;
    await show(
      tester,
      [_Table(_expenses)],
      before: () {
        reads = fake.deferred = [];
        fake.deferredKinds = {BridgeQueryKind.rows};
      },
    );
    expect(inTable(find.text('New')), findsOneWidget);
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    fake.deferred = null;
    while (reads.isNotEmpty) {
      reads.removeAt(0).complete();
      await tester.pump();
    }
    await tester.pumpAndSettle();
    expect(applied(BridgeMutationKind.addRow).single.after, 'r2');
  });

  testWidgets('a quick second field goes after the field just added', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database};
    for (var time = 0; time < 2; time++) {
      await tester.tap(inTable(find.byKey(const Key('add field'))));
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
    }
    fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    final adds = applied(BridgeMutationKind.addField).toList();
    expect(adds, hasLength(2));
    final fields = fake.databases[fake.views[page]!.single.block]!.fields;
    expect(adds.last.after, fields[5].id, reason: 'after the field just added');
  });

  testWidgets('a field removed before it was read leaves + working', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    final table = fake.views[page]!.single.block;
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database};
    Future<void> add() async {
      await tester.tap(inTable(find.byKey(const Key('add field'))));
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
    }

    await add();
    // Another device removes it before this one read it.
    fake.databases[table] = const BridgeDatabase(
      name: 'Expenses',
      fields: _fields,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    await add();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    expect(fake.databases[table]!.fields, hasLength(6));
  });

  testWidgets(
    'a row removed elsewhere while + New is read leaves + New working',
    (tester) async {
      await show(tester, [_Table(_expenses)]);
      final table = fake.views[page]!.single.block;
      final reads = fake.deferred = [];
      fake.deferredKinds = {BridgeQueryKind.database};
      await tester.tap(inTable(find.text('New')));
      await tester.pump();
      // Before this device reads its new row, another one removes the first row.
      fake.rows[table]!.removeAt(0);
      fake.deferred = null;
      for (final read in reads) {
        read.complete();
      }
      await tester.pumpAndSettle();
      expect(inTable(find.text('2 rows')), findsOneWidget);
      // A later change, then the other device removes the row this one added.
      fake.putRows(page, table, [...fake.rows[table]!]);
      await tester.pumpAndSettle();
      fake.putRows(page, table, [fake.rows[table]!.first]);
      await tester.pumpAndSettle();
      await tester.tap(inTable(find.text('New')));
      await tester.pumpAndSettle();
      expect(workspace.problem.value, isNull);
      expect(fake.rows[table], hasLength(2));
    },
  );

  testWidgets('a failed read keeps + New after the row just added', (
    tester,
  ) async {
    await show(tester, [_Table(_expenses)]);
    final table = fake.views[page]!.single.block;
    final reads = fake.deferred = [];
    fake.deferredKinds = {BridgeQueryKind.database};
    await tester.tap(inTable(find.text('New')));
    await tester.pump();
    final added = fake.rows[table]!.last.id;
    // The reads after the add fail, as when the bridge is briefly gone.
    fake.refuseQueries = 'gone';
    fake.deferred = null;
    for (final read in reads) {
      read.complete();
    }
    await tester.pumpAndSettle();
    fake.putRows(page, table, [...fake.rows[table]!]);
    await tester.pumpAndSettle();
    fake.refuseQueries = null;
    await tester.tap(inTable(find.text('New')));
    await tester.pumpAndSettle();
    expect(applied(BridgeMutationKind.addRow).last.after, added);
  });
}
