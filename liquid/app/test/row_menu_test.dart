import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'table_kit.dart';

void main() {
  final kit = TableKit();

  Finder cell(String row, String field) =>
      kit.inTable(find.byKey(ValueKey('cell $row $field')));
  final handle = find.byKey(const Key('row handle'));
  final plus = find.byKey(const Key('insert row'));
  final grip = find.byKey(const Key('row menu button'));
  final menu = find.byKey(const Key('row menu'));
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
    await mouse.addPointer(location: const Offset(1300, 850));
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

  /// The rows added, as where they go: after a row, or first.
  List<String?> added() => [
    for (final add in kit.applied(BridgeMutationKind.addRow))
      add.last ? 'last' : add.after,
  ];

  List<String> order() => [
    for (final row in kit.fake.rows[kit.fake.views[kit.page]!.single.block]!)
      row.id,
  ];

  testWidgets('a hovered row shows its handle, left of the Table', (
    tester,
  ) async {
    await show(tester);
    expect(handle, findsNothing);
    await hover(tester, tester.getCenter(cell('r2', 'name')));
    expect(handle, findsOneWidget);
    final row = tester.getRect(cell('r2', 'name'));
    final shown = tester.getRect(handle);
    expect(shown.right, lessThanOrEqualTo(row.left));
    expect(shown.center.dy, moreOrLessEquals(row.center.dy, epsilon: 1));
    await hover(tester, const Offset(1300, 850));
    expect(handle, findsNothing);
  });

  testWidgets('moving onto the handle keeps it; ⋮⋮ opens the row\'s menu', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester, 'r1');
    expect(handle, findsOneWidget);
    expect(inMenu('Insert above'), findsOneWidget);
    expect(inMenu('Insert below'), findsOneWidget);
    expect(inMenu('Delete row'), findsOneWidget);
  });

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
    await tester.tap(inMenu('Insert above'));
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    await openMenu(tester, 'r2');
    await tester.tap(inMenu('Insert below'));
    await tester.pumpAndSettle();
    expect(added(), ['r1', 'r2']);
    expect(order().length, 4);
  });

  testWidgets('Delete row deletes it without asking', (tester) async {
    await show(tester);
    await openMenu(tester, 'r1');
    await tester.tap(inMenu('Delete row'));
    await tester.pumpAndSettle();
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
    // Insert above, Insert below, Delete row.
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
    final rows = tester
        .widget<ListView>(find.byKey(const Key('table rows')))
        .controller!;
    rows.jumpTo(40);
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(kit.applied(BridgeMutationKind.addRow), isEmpty);
  });

  testWidgets('a row removed elsewhere takes its menu along', (tester) async {
    await show(tester);
    final table = kit.fake.views[kit.page]!.single.block;
    await openMenu(tester, 'r1');
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.removeRow,
        page: kit.page,
        block: table,
        row: 'r1',
      ),
    );
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
  });

  testWidgets('after rows move under a still pointer, leaving the Table hides '
      'the handle', (tester) async {
    await show(tester, 4);
    await openMenu(tester, 'r1');
    // The delete lands once the pointer rests on the row under the closed menu.
    final gate = kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.removeRow};
    await click(tester, inMenu('Delete row'));
    gate.complete();
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.removeRow).single.row, 'r1');
    await hover(tester, const Offset(1300, 850));
    expect(handle, findsNothing);
  });

  testWidgets('closing the menu over another row shows that row\'s handle', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester, 'r1');
    await hover(tester, tester.getCenter(cell('r2', 'date')));
    expect(
      tester.getCenter(handle).dy,
      moreOrLessEquals(tester.getCenter(cell('r1', 'name')).dy, epsilon: 1),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(
      tester.getCenter(handle).dy,
      moreOrLessEquals(tester.getCenter(cell('r2', 'name')).dy, epsilon: 1),
    );
  });
}
