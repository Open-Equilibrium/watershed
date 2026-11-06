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
  }) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    fake = FakeWorkspace();
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
          fake.rows[added.block!] = rows;
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
      findsNWidgets(2),
      reason: 'the title and the Text Block; the Table is no text',
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
}
