import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/paste.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();
  String? copied;
  var change = 0;

  setUp(() {
    copied = null;
    writeClipboard = (text) async {
      copied = text;
      change++;
    };
    readClipboard = () async => Pasted(text: copied, change: change);
  });

  List<BridgeView> tables() => [
    for (final view in kit.fake.views[kit.page]!)
      if (view.kind == BridgeBlockKind.database) view,
  ];

  Future<void> viewAction(WidgetTester tester, String action) async {
    await tap(tester, find.byKey(const Key('View actions')));
    await tap(tester, find.text(action));
  }

  Future<void> pasteHere(WidgetTester tester) async {
    await tester.tap(block(0));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  testWidgets('Duplicate puts a new Table with the same tabs right below', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('Below')]);
    final original = tables().single;
    await viewAction(tester, 'Duplicate');
    final duplicated = kit.applied(BridgeMutationKind.duplicateTable).single;
    expect(
      (duplicated.after, duplicated.block, duplicated.text),
      (original.id, original.block, 'Expenses (copy)'),
    );
    expect(tables(), hasLength(2));
    expect(kit.fake.views[kit.page]![1].block, isNot(original.block));
    expect(kit.fake.tables.map((table) => table.name), [
      'Expenses',
      'Expenses (copy)',
    ]);
  });

  testWidgets('Copy, then ⌘V pastes a copy; the chip offers Paste as View', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    final original = tables().single;
    await viewAction(tester, 'Copy');
    expect(copied, 'Expenses');
    await pasteHere(tester);
    final pasted = kit.applied(BridgeMutationKind.duplicateTable).single;
    expect(pasted.block, original.block);
    expect(find.text('Pasted as a copy'), findsOneWidget);
    final copy = tables().last.block;
    await tap(tester, find.text('Paste as View'));
    final added = kit.applied(BridgeMutationKind.addTableView).single;
    expect((added.block, added.text), (original.block, original.title));
    expect(kit.applied(BridgeMutationKind.deleteTable).single.block, copy);
    expect(tables().map((view) => view.block), [
      original.block,
      original.block,
    ]);
    expect(find.text('Pasted as a copy'), findsNothing);
  });

  Future<String> pasteOnEmptyPage(WidgetTester tester) async {
    final area = kit.workspace.open.value!.area;
    final trip = await kit.workspace.apply(
      mutation(BridgeMutationKind.createPage, area: area, text: 'Trip'),
    );
    kit.workspace.open.value = OpenPage(area, trip.page!);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('first-line')));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    return trip.page!;
  }

  testWidgets('⌘V in an empty Page pastes a copied Table, not its name', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await viewAction(tester, 'Copy');
    await pasteOnEmptyPage(tester);
    final pasted = kit.applied(BridgeMutationKind.duplicateTable).single;
    expect(pasted.block, tables().single.block);
    expect(pasted.after, isNull);
    expect(find.text('Pasted as a copy'), findsOneWidget);
    expect(kit.applied(BridgeMutationKind.addTextBlock), isEmpty);
  });

  testWidgets('⌘V in an empty Page pastes other text as text', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await viewAction(tester, 'Copy');
    copied = 'Groceries';
    change++;
    final trip = await pasteOnEmptyPage(tester);
    expect(kit.applied(BridgeMutationKind.duplicateTable), isEmpty);
    expect(kit.fake.textsOf(trip), ['Groceries']);
  });

  testWidgets('the same text copied elsewhere later pastes as text', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    await viewAction(tester, 'Copy');
    copied = 'Expenses';
    change++;
    await pasteHere(tester);
    expect(kit.applied(BridgeMutationKind.duplicateTable), isEmpty);
    expect(field(tester, 0).controller!.text, 'Expenses');
  });

  testWidgets('a copy of a Table deleted since pastes as text', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    await viewAction(tester, 'Copy');
    kit.fake.dropTable(tables().single.block);
    await pasteHere(tester);
    expect(field(tester, 0).controller!.text, 'Expenses');
  });
}
