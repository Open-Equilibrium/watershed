import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/page_icon.dart';

import 'page_kit.dart';

Finder inSidebar(Finder finder) =>
    find.descendant(of: find.byType(Sidebar), matching: finder);

void main() {
  testWidgets('a Page takes an emoji or an icon beside its title, shown in the '
      'Explorer and the breadcrumb, and loses it on Remove', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    List<String?> icons() => [
      for (final mutation in kit.fake.applied)
        if (mutation.kind == BridgeMutationKind.setPageIcon) mutation.text,
    ];

    await tester.tap(find.byKey(const Key('add page icon')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('icon search')), 'mon');
    await tester.pumpAndSettle();
    expect(find.text('🏠'), findsNothing);
    await tester.tap(find.text('💰').last);
    await tester.pumpAndSettle();
    expect(icons(), ['💰']);
    expect(inSidebar(find.text('💰')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('page-header')),
        matching: find.text('💰'),
      ),
      findsOneWidget,
    );

    await tester.tap(find.byKey(const Key('page icon')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('icon Recent 💰')), findsOneWidget);
    await tester.tap(find.text('Icons'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('icon Icons lucide:rocket')));
    await tester.pumpAndSettle();
    expect(icons().last, 'lucide:rocket');

    await tester.tap(find.byKey(const Key('page icon')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    expect(icons().last, '');
    expect(inSidebar(find.text('💰')), findsNothing);
  });

  testWidgets('without an icon the title lines up with the Blocks', (
    tester,
  ) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    expect(
      tester.getTopLeft(find.byKey(const Key('page-title'))).dx,
      tester.getTopLeft(block(0)).dx,
    );
  });

  testWidgets('without an icon the breadcrumb shows the plain Page symbol', (
    tester,
  ) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    expect(
      find.descendant(
        of: find.byKey(const Key('page-header')),
        matching: find.byType(PageIcon),
      ),
      findsOneWidget,
    );
  });
}
