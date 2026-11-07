import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

/// A part of a Page to open: a Text Block, a Table or a Block of a kind Liquid does not know.
sealed class Part {
  const Part();
}

class TextPart extends Part {
  const TextPart(this.text, [this.style = BridgeTextStyle.paragraph]);
  final String text;
  final BridgeTextStyle style;
}

class TablePart extends Part {
  const TablePart(this.rows, {this.fields = expenseFields});
  final List<BridgeRow> rows;
  final List<BridgeField> fields;
}

class UnknownPart extends Part {
  const UnknownPart({this.boundary = false});
  final bool boundary;
}

BridgeCell cell(
  BridgeCellKind kind, {
  String? text,
  double number = 0,
  bool checked = false,
}) => BridgeCell(kind: kind, text: text, number: number, checked: checked);

const expenseFields = [
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

BridgeRow expenseRow(
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
      value: cell(BridgeCellKind.text, text: name),
    ),
    BridgeFieldCell(
      field: 'date',
      value: cell(BridgeCellKind.date, text: day),
    ),
    BridgeFieldCell(
      field: 'category',
      value: cell(BridgeCellKind.select, text: option),
    ),
    BridgeFieldCell(
      field: 'amount',
      value: cell(BridgeCellKind.number, number: amount),
    ),
    BridgeFieldCell(
      field: 'paid',
      value: cell(BridgeCellKind.checkbox, checked: paid),
    ),
  ],
);

final expenses = [
  expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
  expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
];

BridgeRow namedRow(int number) => BridgeRow(
  id: 'r$number',
  cells: [
    BridgeFieldCell(
      field: 'name',
      value: cell(BridgeCellKind.text, text: 'Row $number'),
    ),
  ],
);

/// A Page with Tables opened over a fake Workspace, for the Table tests.
class TableKit {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  /// Opens a Page of `parts` in Liquid, or in a bare frame unless `app`.
  Future<void> show(
    WidgetTester tester,
    List<Part> parts, {
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
        case TextPart(:final text, :final style):
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
        case TablePart(:final rows, :final fields):
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
        case UnknownPart(:final boundary):
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

  Finder inTable(Finder finder) =>
      find.descendant(of: find.byType(TableBlock), matching: finder);

  Iterable<BridgeMutation> applied(BridgeMutationKind kind) =>
      fake.applied.where((applied) => applied.kind == kind);
}
