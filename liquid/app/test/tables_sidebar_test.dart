import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/workspace/explorer.dart';
import 'package:liquid/src/workspace/table_page.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  final rows = [
    expenseRow('r1', 'Groceries', '2026-11-01', 'food', 84.2, true),
    expenseRow('r2', 'Electricity', '2026-10-30', 'home', 96, false),
    expenseRow('r3', 'Rent', '2026-10-27', 'home', 950, true),
  ];

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    await tester.pumpAndSettle();
  }

  Finder inSidebar(Finder finder) =>
      find.descendant(of: find.byType(Explorer), matching: finder);
  Finder onTablePage(Finder finder) =>
      find.descendant(of: find.byType(TablePage), matching: finder);

  String table() => kit.fake.tables.single.id;

  testWidgets('Tables lists each Table; a click opens its page', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, inSidebar(find.text('Expenses')));
    expect(kit.workspace.open.value?.table, table());
    final name = tester.widget<TextField>(
      find.byKey(const Key('table page name')),
    );
    expect(name.controller!.text, 'Expenses');
    expect(
      find.descendant(
        of: find.byKey(const Key('shown in')),
        matching: find.text('Expenses'),
      ),
      findsOneWidget,
      reason: 'the View keeps the title it was made with',
    );
    expect(onTablePage(find.text('Rent')), findsOneWidget);
    expect(onTablePage(find.text('3 rows')), findsOneWidget);
  });

  testWidgets('on a Table\'s page, sorting and search change nothing saved', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, inSidebar(find.text('Expenses')));
    final written = kit.fake.applied.length;
    await tap(tester, onTablePage(find.text('Amount')));
    final top = {
      for (final name in ['Groceries', 'Rent'])
        name: tester.getTopLeft(onTablePage(find.text(name))).dy,
    };
    expect(top['Groceries']! < top['Rent']!, isTrue);
    await tester.enterText(find.byKey(const Key('table search field')), 'rent');
    await tester.pump(const Duration(milliseconds: 200));
    await tester.pumpAndSettle();
    expect(onTablePage(find.text('Groceries')), findsNothing);
    expect(onTablePage(find.text('1 of 3 rows')), findsOneWidget);
    expect(kit.fake.applied.length, written);
  });

  testWidgets('a Table is renamed on its page, for all its Views', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, inSidebar(find.text('Expenses')));
    await tap(tester, find.byKey(const Key('table page name')));
    expect(find.text('Renames the Table in its View'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('table page name')), 'Costs');
    await tester.pumpAndSettle();
    expect(kit.fake.tables.single.name, 'Costs');
    expect(inSidebar(find.text('Costs')), findsOneWidget);
    expect(
      kit.fake.views[kit.page]!.single.title,
      'Expenses',
      reason: 'the View keeps its own title',
    );
  });

  testWidgets('Delete Table… asks, deletes, and goes back', (tester) async {
    await kit.show(tester, [TablePart(rows)]);
    await tap(tester, inSidebar(find.text('Expenses')));
    await tap(tester, find.byKey(const Key('Table actions')));
    await tap(tester, find.text('Delete Table…'));
    expect(find.text('Delete the Table “Expenses”?'), findsOneWidget);
    expect(
      find.text('Its 3 rows go with it, and so do its 1 View on Untitled.'),
      findsOneWidget,
    );
    await tap(tester, find.widgetWithText(InkWell, 'Delete Table'));
    expect(kit.fake.tables, isEmpty);
    expect(kit.workspace.open.value?.page, kit.page);
  });

  testWidgets('＋ in Tables makes a Table in the open Page\'s area', (
    tester,
  ) async {
    await kit.show(tester, []);
    await tester.tap(find.byTooltip('New Table'));
    await tester.pumpAndSettle();
    final made = kit.fake.tables.single;
    expect(made.area, kit.workspace.areas.value.single.id);
    expect(kit.workspace.open.value?.table, made.id);
  });

  testWidgets('a section folds and unfolds with a click on its name', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(rows)]);
    expect(inSidebar(find.text('Expenses')), findsOneWidget);
    await tap(tester, find.byKey(const Key('fold Tables')));
    expect(inSidebar(find.text('Expenses')), findsNothing);
    await tap(tester, find.byKey(const Key('fold Tables')));
    expect(inSidebar(find.text('Expenses')), findsOneWidget);
  });

  test('folded sections are kept on this device', () async {
    final dir = await Directory.systemTemp.createTemp('liquid-sidebar');
    addTearDown(() => dir.delete(recursive: true));
    final file = SettingsFile(File('${dir.path}/sidebar.json'));
    final fake = FakeWorkspace();
    final first = WorkspaceState(fake, layout: file);
    await first.start();
    first.toggleFold('tables');
    await pumpEventQueue();
    await first.dispose();
    final again = WorkspaceState(fake, layout: file);
    await again.start();
    expect(again.folded.value, {'tables'});
    await again.dispose();
  });
}
