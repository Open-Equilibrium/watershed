import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/page_icon.dart';

import 'focus_kit.dart';
import 'page_kit.dart';

void expectOne(Finder finder) => expect(finder, findsOneWidget);

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

  testWidgets('Add icon shows while keyboard focus is on it', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    await kit.workspace.rename(kit.page, 'October');
    await tester.pumpAndSettle();
    final add = find.byKey(const Key('add page icon'));
    double opacity() => tester
        .widget<AnimatedOpacity>(
          find.ancestor(of: add, matching: find.byType(AnimatedOpacity)),
        )
        .opacity;
    expect(opacity(), 0);
    await tabTo(tester, add);
    expect(opacity(), 1);
  });

  testWidgets('the icon picker names its icons, marks the chosen one and its '
      'tabs', (tester) async {
    final semantics = tester.ensureSemantics();
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    await kit.workspace.setPageIcon(kit.page, '💰');
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('page icon')));
    await tester.pumpAndSettle();

    expect(find.byTooltip('Change icon'), findsOneWidget);
    expect(
      tester.getSemantics(find.bySemanticsLabel('money')),
      isSemantics(
        label: 'money',
        isButton: true,
        isSelected: true,
        hasTapAction: true,
      ),
    );
    expect(
      tester.getSemantics(find.bySemanticsLabel('chart')),
      isSemantics(
        label: 'chart',
        isButton: true,
        isSelected: false,
        hasTapAction: true,
      ),
    );
    expect(
      tester.getSemantics(find.bySemanticsLabel('Emoji')),
      isSemantics(
        label: 'Emoji',
        isButton: true,
        isSelected: true,
        hasTapAction: true,
      ),
    );
    await tester.tap(find.text('Icons'));
    await tester.pumpAndSettle();
    expect(
      tester.getSemantics(find.bySemanticsLabel('rocket')),
      isSemantics(label: 'rocket', isButton: true, hasTapAction: true),
    );
    semantics.dispose();
  });

  testWidgets(
    'an icon that cannot be saved shows the notice and nothing else',
    (tester) async {
      final kit = PageKit();
      await kit.show(tester, const [TextPart('October')]);
      kit.fake.refuseKinds = {BridgeMutationKind.setPageIcon};
      await tester.tap(find.byKey(const Key('add page icon')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('💰').last);
      await tester.pumpAndSettle();
      expectOne(find.textContaining('Liquid did not save this change'));
    },
  );
}
