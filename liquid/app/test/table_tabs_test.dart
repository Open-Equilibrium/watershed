import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_tabs.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final bigCosts = tabOf(
    BridgeLayout.table,
    name: 'Big costs',
    sort: const BridgeSort(field: 'amount', descending: true),
  );

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  BridgeView view() => kit.fake.views[kit.page]!.firstWhere(
    (view) => view.kind == BridgeBlockKind.database,
  );

  Finder tab(String name) =>
      find.descendant(of: find.byType(TabStrip), matching: find.text(name));

  Finder shows(String control, String text) =>
      find.descendant(of: find.byKey(Key(control)), matching: find.text(text));

  Finder menuText(String text) => find.descendant(
    of: find.byKey(const Key('table query menu')),
    matching: find.text(text),
  );

  Future<String> addTab(BridgeTab tab) async {
    final added = await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addTab,
        page: kit.page,
        view: view().id,
        after: view().tabs.last.id,
        tabs: [tab],
      ),
    );
    return added.tab!;
  }

  testWidgets('a View shows its tabs, and a tab its own sort', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await addTab(bigCosts);
    await tester.pumpAndSettle();
    expect(tab('Table'), findsOneWidget);
    expect(shows('table sort', 'Sort'), findsOneWidget);
    await tap(tester, tab('Big costs'));
    expect(shows('table sort', 'Amount, descending'), findsOneWidget);
    expect(kit.fake.device['tab/${view().id}'], view().tabs.last.id);
  });

  testWidgets('the last used tab shows when the Page opens again', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final big = await addTab(bigCosts);
    final area = kit.workspace.open.value!.area;
    final other = await kit.workspace.apply(
      mutation(BridgeMutationKind.createPage, area: area, text: 'Other'),
    );
    kit.workspace.open.value = OpenPage(area, other.page!);
    await tester.pumpAndSettle();
    kit.fake.device['tab/${view().id}'] = big;
    kit.workspace.open.value = OpenPage(area, kit.page);
    await tester.pumpAndSettle();
    expect(shows('table sort', 'Amount, descending'), findsOneWidget);
  });

  testWidgets('＋ adds a tab of the chosen layout, named after it', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tap(tester, find.byKey(const Key('add tab')));
    expect(menuText('New tab'), findsOneWidget);
    await tap(tester, menuText('Board'));
    final added = kit.applied(BridgeMutationKind.addTab).single;
    expect(added.after, view().tabs.first.id);
    expect(
      [for (final tab in added.tabs) (tab.name, tab.layout)],
      [('Board', BridgeLayout.board)],
    );
    expect(view().tabs.map((tab) => tab.name), ['Table', 'Board']);
    expect(kit.fake.device['tab/${view().id}'], view().tabs.last.id);
  });

  testWidgets('the active tab opens its menu: rename, layout, duplicate', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tap(tester, tab('Table'));
    expect(menuText('Delete tab'), findsNothing, reason: 'the only tab');
    await tap(tester, menuText('Calendar'));
    expect(view().tabs.single.layout, BridgeLayout.calendar);
    await tap(tester, tab('Table'));
    await tap(tester, menuText('Rename'));
    await tester.enterText(find.byKey(const Key('tab name')), 'Due dates');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(view().tabs.single.name, 'Due dates');
    await tap(tester, tab('Due dates'));
    await tap(tester, menuText('Duplicate tab'));
    expect(view().tabs.map((tab) => (tab.name, tab.layout)), [
      ('Due dates', BridgeLayout.calendar),
      ('Due dates', BridgeLayout.calendar),
    ]);
  });

  testWidgets('Delete tab removes it and shows its neighbor', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final board = await addTab(tabOf(BridgeLayout.board));
    await tester.pumpAndSettle();
    await tap(tester, tab('Board'));
    await tester.tap(tab('Board'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tap(tester, menuText('Delete tab'));
    expect(view().tabs.map((tab) => tab.id), isNot(contains(board)));
    expect(kit.fake.device['tab/${view().id}'], view().tabs.single.id);
  });

  testWidgets('a tab dragged onto another takes its place', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await addTab(tabOf(BridgeLayout.board));
    await tester.pumpAndSettle();
    final first = view().tabs.first.id;
    await tester.timedDrag(
      tab('Board'),
      tester.getCenter(tab('Table')) - tester.getCenter(tab('Board')),
      const Duration(milliseconds: 300),
    );
    await tester.pumpAndSettle();
    final moved = kit.applied(BridgeMutationKind.moveTab).single;
    expect(moved.after, isNull);
    expect(view().tabs.last.id, first);
  });

  testWidgets('Group sets the tab’s group from Select and Checkbox fields', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tap(tester, find.byKey(const Key('table group')));
    expect(menuText('Group by'), findsOneWidget);
    expect(menuText('Name'), findsNothing);
    await tap(tester, menuText('Category'));
    expect(view().tabs.single.group, 'category');
    expect(shows('table group', 'Category'), findsOneWidget);
  });
}
