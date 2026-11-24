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

  final menu = find.byKey(const Key('field menu'));
  Finder inMenu(Finder finder) => find.descendant(of: menu, matching: finder);
  Finder header(String name) => kit.inTable(find.text(name));
  final dialog = find.byType(Dialog);

  Finder chevron(String name) => find
      .descendant(
        of: find.ancestor(of: header(name), matching: find.byType(Row)).first,
        matching: find.byKey(const Key('column menu')),
      )
      .first;

  Future<void> open(WidgetTester tester, String name) =>
      tap(tester, chevron(name));

  Future<void> type(WidgetTester tester, String name) async {
    await tester.enterText(inMenu(find.byType(TextField)), name);
    await tester.pumpAndSettle();
  }

  List<(String?, String?)> renamed() => [
    for (final rename in kit.applied(BridgeMutationKind.renameField))
      (rename.field, rename.text),
  ];

  testWidgets('the chevron or right-click opens a column menu', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await tester.tap(header('Category'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    expect(menu, findsOneWidget);
    await key(tester, LogicalKeyboardKey.escape);
    expect(menu, findsNothing);
    await open(tester, 'Category');
    expect(menu, findsOneWidget);
    expect(
      tester.widget<TextField>(inMenu(find.byType(TextField))).controller!.text,
      'Category',
    );
    expect(inMenu(find.text('Select')), findsOneWidget);
    expect(inMenu(find.text('Delete field')), findsOneWidget);
  });

  testWidgets('⏎ renames; an empty or same name keeps it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, '  ');
    await key(tester, LogicalKeyboardKey.enter);
    await open(tester, 'Category');
    await key(tester, LogicalKeyboardKey.enter);
    expect(renamed(), isEmpty);
    expect(header('Category'), findsOneWidget);

    await open(tester, 'Category');
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.enter);
    expect(menu, findsNothing);
    expect(renamed(), [('category', 'Kind')]);
    expect(header('Kind'), findsOneWidget);
  });

  testWidgets('esc drops a name; a click outside keeps it', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await type(tester, 'Kind');
    await key(tester, LogicalKeyboardKey.escape);
    expect(menu, findsNothing);
    expect(renamed(), isEmpty);
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

  testWidgets('Delete field asks first; ⏎ deletes the field', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    await open(tester, 'Category');
    await tap(tester, inMenu(find.text('Delete field')));
    expect(menu, findsNothing);
    expect(find.text('Delete the field “Category”?'), findsOneWidget);
    expect(find.text('Its values in all 2 rows go with it.'), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);
    expect(dialog, findsNothing);
    final removed = kit.applied(BridgeMutationKind.removeField).single;
    expect(removed.field, 'category');
    expect(header('Category'), findsNothing);
  });

  testWidgets('a field removed elsewhere closes its menu', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await open(tester, 'Category');
    await type(tester, 'Kind');
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

  testWidgets('an untouched menu keeps a rename elsewhere', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final table = kit.fake.views[kit.page]!.single.block;
    await open(tester, 'Category');
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

  testWidgets('a typed name is kept on close or quit', (tester) async {
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
}
