import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/whiteboard/board_controller.dart';
import 'package:liquid/src/workspace/media_block.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';
import 'package:liquid/src/workspace/table_block.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/whiteboard_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

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
  const CodePart(this.code);
  final String code;
}

class MediaPart extends Part {
  const MediaPart(this.name, this.bytes);
  final String name;
  final List<int> bytes;
}

class WhiteboardPart extends Part {
  const WhiteboardPart([this.elements = const []]);
  final List<BridgeElement> elements;
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
}) => BridgeCell(
  kind: kind,
  text: text,
  number: number,
  checked: checked,
  files: const [],
);

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

class PageKit {
  late FakeWorkspace fake;
  late WorkspaceState workspace;
  late String page;

  Future<void> show(
    WidgetTester tester,
    List<Part> parts, {
    bool app = false,
    bool fresh = false,
    SessionState? session,
    SessionInfo? info,
    ShortcutController? shortcuts,
    void Function()? before,
    bool pdfRenderer = false,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    if (!pdfRenderer) {
      PdfPages.opener = (_) async => throw const PdfUnreadable();
    }
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
    Future<BridgeApplied> add(
      BridgeMutationKind kind, {
      String? text,
      String? mediaType,
      String? attachment,
    }) async {
      final added = await workspace.apply(
        mutation(
          kind,
          page: page,
          after: after,
          text: text,
          mediaType: mediaType,
          attachment: attachment,
        ),
      );
      after = added.view;
      return added;
    }

    for (final part in parts) {
      switch (part) {
        case TextPart(:final text, :final style):
          final added = await add(BridgeMutationKind.addTextBlock, text: text);
          fake.styles[added.block!] = style;
        case TablePart(:final rows, :final fields):
          final added = await add(
            BridgeMutationKind.addDatabaseBlock,
            text: 'Expenses',
          );
          fake.databases[added.block!] = BridgeDatabase(
            name: 'Expenses',
            fields: fields,
          );
          fake.rows[added.block!] = [...rows];
        case CodePart(:final code):
          final added = await add(BridgeMutationKind.addTextBlock, text: code);
          await workspace.apply(
            mutation(
              BridgeMutationKind.turnIntoCode,
              page: page,
              block: added.block,
            ),
          );
        case MediaPart(:final name, :final bytes):
          final attachment = await fake.addBytes(bytes: bytes);
          await add(
            BridgeMutationKind.addMediaBlock,
            text: name,
            mediaType: mediaTypeOf(name),
            attachment: attachment,
          );
        case WhiteboardPart(:final elements):
          final added = await add(BridgeMutationKind.addWhiteboardBlock);
          fake.whiteboards[added.block!] = BridgeDrawing(
            style: BridgeDrawingStyle.handDrawn,
            elements: [
              for (final (index, element) in elements.indexed)
                withId(element, 'element $index'),
            ],
          );
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
              title: '',
              boundary: boundary,
              kind: BridgeBlockKind.unknown,
              tabs: const [],
            ),
          );
          after = 'unknown-view';
      }
    }
  }

  Finder inTable(Finder finder) =>
      find.descendant(of: find.byType(TableBlock), matching: finder);

  BoardController boardOf(WidgetTester tester) {
    final block = tester.widget<DrawingFrame>(
      find.byType(DrawingFrame, skipOffstage: false).first,
    );
    final board = block.views.boardFor(block.view);
    block.views.releaseBoard(block.view);
    return board;
  }

  Iterable<BridgeMutation> applied(BridgeMutationKind kind) =>
      fake.applied.where((applied) => applied.kind == kind);
}

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
