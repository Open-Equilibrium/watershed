import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/paste.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  String? copied;

  setUp(() {
    copied = null;
    writeClipboard = (text) async => copied = text;
    readClipboard = () async => Pasted(text: copied);
  });

  List<BridgeView> tables() => [
    for (final view in kit.fake.views[kit.page]!)
      if (view.kind == BridgeBlockKind.database) view,
  ];

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Future<void> viewAction(WidgetTester tester, String action) async {
    await tap(tester, find.byKey(const Key('View actions')));
    await tap(tester, find.text(action));
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
    await tester.tap(block(0));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
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

  testWidgets('text copied elsewhere pastes as text', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    await viewAction(tester, 'Copy');
    copied = 'Expenses, by month';
    await tester.tap(block(0));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await key(tester, LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.duplicateTable), isEmpty);
    expect(field(tester, 0).controller!.text, 'Expenses, by month');
  });
}
