import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/media_block.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/text_block.dart';
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

class CodePart extends Part {
  const CodePart(this.code, [this.language = '']);
  final String code;
  final String language;
}

class MediaPart extends Part {
  const MediaPart(this.name, this.bytes);
  final String name;
  final List<int> bytes;
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

/// A Page opened over a fake Workspace, for the Workspace tests.
class PageKit {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  /// Opens a Page of `parts` in Liquid, or in a bare frame unless `app`.
  ///
  /// A `fresh` Page stays the new blank Page of a first start, which takes the keys in its
  /// title; a `session` shows that agent session instead, beside the sidebar's Pages.
  Future<void> show(
    WidgetTester tester,
    List<Part> parts, {
    bool app = false,
    bool fresh = false,
    SessionState? session,
    SessionInfo? info,
    ShortcutController? shortcuts,
    void Function()? before,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    fake = FakeWorkspace();
    before?.call();
    workspace = WorkspaceState(fake);
    await workspace.start(show: session == null);
    if (session == null) {
      page = workspace.open.value!.page;
      await _add(parts);
      if (!fresh) {
        workspace.open.value = OpenPage(workspace.open.value!.area, page);
      }
    }
    await tester.pumpWidget(
      app
          ? LiquidApp(
              launch: ValueNotifier(const Launch.problem(null)),
              workspace: workspace,
            )
          : MaterialApp(
              theme: liquidTheme(Brightness.light),
              home: AgentShell(
                state: session,
                info: info,
                workspace: workspace,
                shortcuts: shortcuts,
              ),
            ),
    );
    await tester.pumpAndSettle();
    addTearDown(workspace.dispose);
  }

  Future<void> _add(List<Part> parts) async {
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
        case CodePart(:final code, :final language):
          final added = await workspace.apply(
            mutation(
              BridgeMutationKind.addTextBlock,
              page: page,
              after: after,
              text: code,
            ),
          );
          after = added.view;
          final turn = mutation(
            BridgeMutationKind.turnIntoCode,
            page: page,
            block: added.block,
          );
          await workspace.apply(turn);
          if (language.isNotEmpty) fake.languages[added.block!] = language;
        case MediaPart(:final name, :final bytes):
          final attachment = await fake.addBytes(bytes: bytes);
          final added = await workspace.apply(
            mutation(
              BridgeMutationKind.addMediaBlock,
              page: page,
              after: after,
              text: name,
              mediaType: mediaTypeOf(name),
              attachment: attachment,
            ),
          );
          after = added.view;
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
  }

  Finder inTable(Finder finder) =>
      find.descendant(of: find.byType(TableBlock), matching: finder);

  Iterable<BridgeMutation> applied(BridgeMutationKind kind) =>
      fake.applied.where((applied) => applied.kind == kind);
}

/// The text field of the Text Block at `index`.
Finder block(int index) => find.descendant(
  of: find.byType(TextBlock).at(index),
  matching: find.byType(TextField),
);

TextField field(WidgetTester tester, int index) =>
    tester.widget<TextField>(block(index));

Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyEvent(key);
  await tester.pumpAndSettle();
}
