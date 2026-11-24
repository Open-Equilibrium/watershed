import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));
  final plus = find.byKey(const Key('insert row'));
  final grip = find.byKey(const Key('row menu button'));
  final menu = find.byKey(const Key('row menu'));
  const away = Offset(1300, 850);
  Finder inMenu(String text) =>
      find.descendant(of: menu, matching: find.text(text));

  late TestGesture mouse;

  Future<void> hover(WidgetTester tester, Offset at) async {
    await mouse.moveTo(at);
    await tester.pumpAndSettle();
  }

  Future<void> show(WidgetTester tester, [int more = 0]) async {
    await kit.show(tester, [
      TablePart([
        ...expenses,
        for (var number = 3; number < 3 + more; number++) namedRow(number),
      ]),
    ]);
    mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: away);
    addTearDown(mouse.removePointer);
  }

  Future<void> click(WidgetTester tester, Finder finder) async {
    await hover(tester, tester.getCenter(finder));
    await mouse.down(tester.getCenter(finder));
    await mouse.up();
    await tester.pumpAndSettle();
  }

  Future<void> openMenu(WidgetTester tester, String row) async {
    await hover(tester, tester.getCenter(cell(row, 'name')));
    await click(tester, grip);
  }

  Future<void> elsewhere(
    WidgetTester tester,
    BridgeMutationKind kind, {
    String? row,
    String? after,
  }) async {
    final table = kit.fake.views[kit.page]!.single.block;
    await kit.workspace.apply(
      mutation(kind, page: kit.page, block: table, row: row, after: after),
    );
    await tester.pumpAndSettle();
  }

  ScrollController rowsScroll(WidgetTester tester) =>
      tester.widget<ListView>(find.byKey(const Key('table rows'))).controller!;

  List<String?> added() => [
    for (final add in kit.applied(BridgeMutationKind.addRow))
      add.last ? 'last' : add.after,
  ];

  List<String> order() => [
    for (final row in kit.fake.rows[kit.fake.views[kit.page]!.single.block]!)
      row.id,
  ];

  testWidgets('+ inserts a row below, ⌥ and + one above', (tester) async {
    await show(tester);
    await hover(tester, tester.getCenter(cell('r1', 'name')));
    await click(tester, plus);
    expect(added(), ['r1']);
    expect(order()[1], isNot('r2'));
    expect(kit.inTable(find.text('3 rows')), findsOneWidget);
    await hover(tester, tester.getCenter(cell('r1', 'name')));
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await click(tester, plus);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    expect(added(), ['r1', null]);
    expect(order().last, 'r2');
    expect(order()[1], 'r1');
  });

  testWidgets('Insert above and below go next to the row', (tester) async {
    await show(tester);
    await openMenu(tester, 'r2');
    await tap(tester, inMenu('Insert above'));
    expect(menu, findsNothing);
    await openMenu(tester, 'r2');
    await tap(tester, inMenu('Insert below'));
    expect(added(), ['r1', 'r2']);
    expect(order().length, 4);
  });

  testWidgets('Delete row deletes it without asking', (tester) async {
    await show(tester);
    await openMenu(tester, 'r1');
    await tap(tester, inMenu('Delete row'));
    expect(menu, findsNothing);
    expect(find.byType(Dialog), findsNothing);
    expect(kit.applied(BridgeMutationKind.removeRow).single.row, 'r1');
    expect(cell('r1', 'name'), findsNothing);
    expect(kit.inTable(find.text('1 row')), findsOneWidget);
  });

  testWidgets('↑↓ choose in the menu, ⏎ takes, esc closes', (tester) async {
    await show(tester);
    await openMenu(tester, 'r1');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    await openMenu(tester, 'r1');
    for (var step = 0; step < 3; step++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    }
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.removeRow).single.row, 'r1');
    expect(added(), isEmpty);
  });

  testWidgets('a click outside or a Table scroll closes the menu', (
    tester,
  ) async {
    await show(tester, 20);
    await openMenu(tester, 'r1');
    await click(tester, kit.inTable(find.text('Expenses')));
    expect(menu, findsNothing);
    await openMenu(tester, 'r1');
    rowsScroll(tester).jumpTo(40);
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(kit.applied(BridgeMutationKind.addRow), isEmpty);
  });

  testWidgets('a row removed or added elsewhere closes its menu', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester, 'r1');
    await elsewhere(tester, BridgeMutationKind.removeRow, row: 'r1');
    expect(menu, findsNothing);
    await openMenu(tester, 'r2');
    await elsewhere(tester, BridgeMutationKind.addRow);
    expect(menu, findsNothing);
  });
}
