import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Future<void> pick(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Future<void> pickTable(WidgetTester tester) => pick(
    tester,
    find.descendant(of: find.byType(Dialog), matching: find.text('Expenses')),
  );

  Future<String> secondPage() async {
    final made = await kit.workspace.apply(
      mutation(
        BridgeMutationKind.createPage,
        area: kit.workspace.areas.value.single.id,
        text: 'Review',
      ),
    );
    return made.page!;
  }

  Future<void> choose(WidgetTester tester) async {
    await tester.tap(block(0));
    await tester.pump();
    for (final typed in ['/', '/exist']) {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: typed,
          selection: TextSelection.collapsed(offset: typed.length),
        ),
      );
      await tester.pumpAndSettle();
    }
    await key(tester, LogicalKeyboardKey.enter);
  }

  testWidgets('Existing Table… adds a View with one tab', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    final table = kit.fake.tables.single.id;
    await choose(tester);
    expect(find.text('Add a View of a Table'), findsOneWidget);
    await pickTable(tester);
    expect(find.text('Add a View of “Expenses”'), findsOneWidget);
    await pick(tester, find.byKey(const Key('layout Board')));
    await pick(tester, find.text('Add View'));
    final added = kit.fake.applied.last;
    expect(added.kind, BridgeMutationKind.addTableView);
    expect((added.block, added.text), (table, 'Expenses'));
    expect(
      [for (final tab in added.tabs) (tab.name, tab.layout)],
      [('Board', BridgeLayout.board)],
    );
    expect(
      kit.fake.views[kit.page]!.where((view) => view.block == table),
      hasLength(2),
    );
  });

  testWidgets('a new View may copy the tabs of another, once', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    final table = kit.fake.tables.single.id;
    final review = await secondPage();
    const sort = BridgeSort(field: 'amount', descending: true);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addTableView,
        page: review,
        block: table,
        text: 'Big costs',
        tabs: [tabOf(BridgeLayout.table, name: 'By amount', sort: sort)],
      ),
    );
    await choose(tester);
    await pickTable(tester);
    expect(find.text('Review › Big costs'), findsOneWidget);
    await pick(tester, find.text('Review › Big costs'));
    await pick(tester, find.text('Add View'));
    final added = kit.fake.applied.last;
    expect(added.kind, BridgeMutationKind.addTableView);
    expect(
      [for (final tab in added.tabs) (tab.name, tab.sort)],
      [('By amount', sort)],
    );
  });
}
