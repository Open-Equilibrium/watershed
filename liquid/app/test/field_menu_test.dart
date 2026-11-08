import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'table_kit.dart';

void main() {
  final kit = TableKit();

  final menu = find.byKey(const Key('field menu'));
  Finder inMenu(Finder finder) => find.descendant(of: menu, matching: finder);
  Finder header(String name) => kit.inTable(find.text(name));
  final dialog = find.byType(Dialog);

  Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pumpAndSettle();
  }

  Future<void> open(WidgetTester tester, String name) async {
    await tester.tap(header(name));
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String name) async {
    await tester.enterText(inMenu(find.byType(TextField)), name);
    await tester.pumpAndSettle();
  }

  /// The renames written, as field and name.
  List<(String?, String?)> renamed() => [
    for (final rename in kit.applied(BridgeMutationKind.renameField))
      (rename.field, rename.text),
  ];

  testWidgets('a click on a field\'s header opens its menu', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    expect(menu, findsOneWidget);
    expect(
      tester.widget<TextField>(inMenu(find.byType(TextField))).controller!.text,
      'Category',
    );
    expect(inMenu(find.text('Select')), findsOneWidget);
    expect(inMenu(find.text('Delete field')), findsOneWidget);
  });

  testWidgets('⏎ renames the field and closes the menu', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.enter);
    expect(menu, findsNothing);
    expect(renamed(), [('category', 'Kind')]);
    expect(header('Kind'), findsOneWidget);
  });

  testWidgets('a click outside or on the header again keeps the new name', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    await open(tester, 'Amount');
    await type(tester, 'Sum');
    await open(tester, 'Amount');
    expect(menu, findsNothing);
    expect(renamed(), [('category', 'Kind'), ('amount', 'Sum')]);
  });

  testWidgets('scrolling the Page closes the menu and keeps the new name', (
    tester,
  ) async {
    await kit.show(tester, [
      TablePart(expenses),
      for (var number = 0; number < 40; number++) TextPart('Line $number'),
    ]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    tester
        .state<ScrollableState>(
          find
              .descendant(
                of: find.byType(CustomScrollView),
                matching: find.byType(Scrollable),
              )
              .first,
        )
        .position
        .jumpTo(200);
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(renamed(), [('category', 'Kind')]);
  });

  testWidgets('esc closes the menu and drops the new name', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.escape);
    expect(menu, findsNothing);
    expect(renamed(), isEmpty);
  });

  testWidgets('an empty or unchanged name keeps the old one', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, '  ');
    await key(tester, LogicalKeyboardKey.enter);
    await open(tester, 'Category');
    await key(tester, LogicalKeyboardKey.enter);
    expect(renamed(), isEmpty);
    expect(header('Category'), findsOneWidget);
  });

  testWidgets('Delete field asks first; ⏎ deletes the field', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await tester.tap(inMenu(find.text('Delete field')));
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(find.text('Delete the field “Category”?'), findsOneWidget);
    expect(find.text('Its values in all 2 rows go with it.'), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);
    expect(dialog, findsNothing);
    final removed = kit.applied(BridgeMutationKind.removeField).single;
    expect(removed.field, 'category');
    expect(header('Category'), findsNothing);
  });

  testWidgets('esc in the question keeps the field', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await tester.tap(inMenu(find.text('Delete field')));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.escape);
    expect(dialog, findsNothing);
    expect(kit.applied(BridgeMutationKind.removeField), isEmpty);
    expect(header('Category'), findsOneWidget);
  });

  testWidgets('↓ ⏎ in the menu asks to delete the field', (tester) async {
    await kit.show(tester, [
      TablePart([expenses.first]),
    ]);
    await open(tester, 'Category');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await key(tester, LogicalKeyboardKey.enter);
    expect(find.text('Delete the field “Category”?'), findsOneWidget);
    expect(find.text('Its value in 1 row goes with it.'), findsOneWidget);
  });

  testWidgets('a field removed elsewhere closes its menu', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await open(tester, 'Category');
    await type(tester, 'Kind');
    // Another device removes the field.
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.removeField,
        page: kit.page,
        block: table,
        field: 'category',
      ),
    );
    await tester.pumpAndSettle();
    expect(menu, findsNothing);
    expect(renamed(), isEmpty);
  });

  testWidgets('a rename elsewhere while the menu is open untouched stays', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await open(tester, 'Category');
    // Another device renames the field.
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.renameField,
        page: kit.page,
        block: table,
        field: 'category',
        text: 'Type',
      ),
    );
    await tester.pumpAndSettle();
    await tester.tapAt(const Offset(1300, 850));
    await tester.pumpAndSettle();
    expect(renamed(), [('category', 'Type')]);
    expect(header('Type'), findsOneWidget);
  });

  testWidgets('⏎ renames also with the pointer on Delete field', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(
      location: tester.getCenter(inMenu(find.text('Delete field'))),
    );
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.enter);
    expect(dialog, findsNothing);
    expect(renamed(), [('category', 'Kind')]);
  });

  testWidgets('a name typed is kept when the Page closes or Liquid quits', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(renamed(), [('category', 'Kind')]);

    await open(tester, 'Amount');
    await type(tester, 'Sum');
    await kit.workspace.newPage(kit.workspace.open.value!.area);
    await tester.pumpAndSettle();
    expect(renamed().last, ('amount', 'Sum'));
  });

  testWidgets('a name typed before Delete field stays when the question is '
      'cancelled', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await tester.tap(inMenu(find.text('Delete field')));
    await tester.pumpAndSettle();
    await key(tester, LogicalKeyboardKey.escape);
    expect(kit.applied(BridgeMutationKind.removeField), isEmpty);
    expect(renamed(), [('category', 'Kind')]);
  });

  testWidgets('typing after ↓ goes back to the name, which ⏎ keeps', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await key(tester, LogicalKeyboardKey.arrowDown);
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.enter);
    expect(dialog, findsNothing);
    expect(renamed(), [('category', 'Kind')]);
  });

  testWidgets('a name typed before Delete field is kept when Liquid quits '
      'during the question', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await tester.tap(inMenu(find.text('Delete field')));
    await tester.pumpAndSettle();
    expect(dialog, findsOneWidget);
    await kit.workspace.drain();
    await tester.pumpAndSettle();
    expect(renamed(), [('category', 'Kind')]);
    // Cancelled afterwards, it is not written again.
    await key(tester, LogicalKeyboardKey.escape);
    expect(renamed(), [('category', 'Kind')]);
  });
}
