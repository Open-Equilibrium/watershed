import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Future<void> pick(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

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
    final line = block(0);
    await tester.tap(line);
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
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
  }

  testWidgets('Existing Table… adds a View of a chosen Table with no setup', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    final table = kit.fake.tables.single.id;
    await choose(tester);
    expect(find.text('Add a View of a Table'), findsOneWidget);
    await pick(
      tester,
      find.descendant(of: find.byType(Dialog), matching: find.text('Expenses')),
    );
    final added = kit.fake.applied.last;
    expect(added.kind, BridgeMutationKind.addTableView);
    expect(
      (added.block, added.text, added.sort, added.filter),
      (table, 'Expenses', null, null),
    );
    expect(
      kit.fake.views[kit.page]!.where((view) => view.block == table),
      hasLength(2),
    );
  });

  testWidgets('a new View may copy the setup of another, once', (tester) async {
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
        sort: sort,
      ),
    );
    await choose(tester);
    await pick(
      tester,
      find.descendant(of: find.byType(Dialog), matching: find.text('Expenses')),
    );
    expect(find.text('Add a View of “Expenses”'), findsOneWidget);
    expect(find.text('Amount, descending'), findsOneWidget);
    expect(
      find.descendant(of: find.byType(Dialog), matching: find.text('Review')),
      findsOneWidget,
    );
    await pick(tester, find.byKey(const Key('setup 0')));
    await pick(tester, find.text('Add View'));
    final added = kit.fake.applied.last;
    expect((added.kind, added.sort), (BridgeMutationKind.addTableView, sort));
  });

  testWidgets('cancel adds no View', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    await choose(tester);
    await pick(tester, find.text('Cancel'));
    expect(
      kit.fake.applied.where(
        (applied) => applied.kind == BridgeMutationKind.addTableView,
      ),
      isEmpty,
    );
  });
}
