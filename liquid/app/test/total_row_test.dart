import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final menu = find.byKey(const Key('total menu'));
  Finder inMenu(Finder finder) => find.descendant(of: menu, matching: finder);

  testWidgets('a field\'s footer cell picks a total that the View counts', (
    tester,
  ) async {
    await kit.show(tester, [TablePart(expenses)]);
    expect(kit.inTable(find.text('2 rows')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('total date')));
    await tester.pumpAndSettle();
    expect(inMenu(find.text('Count')), findsOneWidget);
    expect(inMenu(find.text('Sum')), findsNothing, reason: 'a date has no sum');
    await tester.tap(find.byKey(const ValueKey('total date')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('total amount')));
    await tester.pumpAndSettle();
    await tester.tap(inMenu(find.text('Sum')));
    await tester.pumpAndSettle();

    final set = kit.applied(BridgeMutationKind.setTabTotal).single;
    expect((set.field, set.total), ('amount', BridgeTotal.sum));
    expect(kit.inTable(find.text('Sum ')), findsNothing);
    expect(kit.inTable(find.textContaining('180.2')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('total amount')));
    await tester.pumpAndSettle();
    await tester.tap(inMenu(find.text('None')));
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.setTabTotal).last.total, isNull);
    expect(kit.inTable(find.textContaining('180.2')), findsNothing);
  });
}
