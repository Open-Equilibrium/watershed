import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();

  Finder bar() => find.byKey(const Key('power bar'));

  Finder inBar(String text) =>
      find.descendant(of: bar(), matching: find.text(text));

  Future<void> open(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(bar(), findsOneWidget);
  }

  Future<void> type(WidgetTester tester, String text) async {
    await tester.enterText(
      find.descendant(of: bar(), matching: find.byType(TextField)),
      text,
    );
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
  }

  Future<void> enter(WidgetTester tester) async {
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
  }

  testWidgets('⌘K opens the PowerBar over the Page; esc closes it', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('We expect rain on Tuesday.')]);
    await open(tester);
    expect(inBar('Recent'), findsOneWidget);
    expect(inBar('New Page'), findsOneWidget);
    expect(inBar('Arrange'), findsOneWidget);
    expect(inBar('Settings'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(bar(), findsNothing);
  });

  testWidgets('typing searches text and Pages; ⏎ opens the first', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('We expect rain on Tuesday.')]);
    final shared = await kit.workspace.apply(
      mutation(BridgeMutationKind.createArea, text: 'Shared'),
    );
    final trip = await kit.workspace.apply(
      mutation(BridgeMutationKind.createPage, area: shared.area, text: 'Trip'),
    );
    await tester.pumpAndSettle();
    await open(tester);
    await type(tester, 'tues');
    expect(inBar('Text'), findsOneWidget);
    expect(inBar('We expect rain on Tuesday.'), findsOneWidget);
    await tester.enterText(
      find.descendant(of: bar(), matching: find.byType(TextField)),
      'trip',
    );
    await tester.pump();
    expect(
      inBar('We expect rain on Tuesday.'),
      findsNothing,
      reason: 'hits of an earlier query are gone at once',
    );
    await type(tester, 'trip');
    expect(inBar('Pages'), findsOneWidget);
    expect(inBar('Shared'), findsOneWidget, reason: 'the hit’s own area');
    await enter(tester);
    expect(bar(), findsNothing);
    expect(kit.workspace.open.value?.page, trip.page);
    expect(kit.fake.device['recent'], contains(trip.page));
  });

  testWidgets('opened Pages count as recent though the kept list is unread', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('Plan'),
    ], before: () => kit.fake.refuseRemembered = true);
    expect(kit.workspace.recent.value, [kit.page]);
  });

  testWidgets('an opened Table is not a recent Page', (tester) async {
    await kit.show(tester, [TablePart(expenses)]);
    final area = kit.workspace.open.value!.area;
    for (final title in ['Two', 'Three']) {
      final page = await kit.workspace.apply(
        mutation(BridgeMutationKind.createPage, area: area, text: title),
      );
      kit.workspace.open.value = OpenPage(area, page.page!);
      await tester.pumpAndSettle();
    }
    final table = kit.fake.tables.single.id;
    kit.workspace.openTable(table);
    await tester.pumpAndSettle();
    expect(kit.workspace.recent.value, isNot(contains(table)));
    await open(tester);
    for (final title in ['Three', 'Two', 'Untitled']) {
      expect(inBar(title), findsOneWidget, reason: title);
    }
  });

  testWidgets('a text hit opens its Page with the caret in that Block', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Plan')]);
    final area = kit.workspace.open.value!.area;
    final trip = (await kit.workspace.apply(
      mutation(BridgeMutationKind.createPage, area: area, text: 'Trip'),
    )).page!;
    await kit.workspace.apply(
      mutation(BridgeMutationKind.addTextBlock, page: trip, text: 'Intro'),
    );
    final found = await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addTextBlock,
        page: trip,
        after: kit.fake.views[trip]!.first.id,
        text: 'Book the flat in Alfama',
      ),
    );
    await open(tester);
    await type(tester, 'alfa');
    await enter(tester);
    expect(kit.workspace.open.value?.page, trip);
    final focused = tester
        .widgetList<TextField>(find.byType(TextField))
        .where((field) => field.focusNode?.hasFocus ?? false);
    expect(focused.single.controller?.text, 'Book the flat in Alfama');
    expect(found.view, isNotNull);
  });

  testWidgets('/ lists the Blocks, and ⏎ adds one to the Page', (tester) async {
    await kit.show(tester, [const TextPart('Plan')]);
    await open(tester);
    await type(tester, '/');
    expect(
      find.descendant(of: bar(), matching: find.textContaining('Add to ')),
      findsOneWidget,
      reason: 'one heading over all the new Blocks',
    );
    await type(tester, '/white');
    expect(inBar('Whiteboard'), findsOneWidget);
    expect(inBar('Text'), findsNothing);
    await enter(tester);
    final added = kit.applied(BridgeMutationKind.addWhiteboardBlock).single;
    expect(
      (added.page, added.after),
      (kit.page, kit.fake.views[kit.page]!.first.id),
    );
  });

  testWidgets('Existing Table… asks for the Table while the end loads', (
    tester,
  ) async {
    await kit.show(tester, [
      TablePart(expenses),
      for (var at = 0; at < 150; at++) TextPart('Line $at'),
    ]);
    kit.fake.deferredKinds = {BridgeQueryKind.views};
    kit.fake.deferred = [];
    await open(tester);
    await type(tester, '/exist');
    await enter(tester);
    expect(bar(), findsNothing);
    expect(find.text('Add a View of a Table'), findsOneWidget);
  });

  testWidgets('a Block goes after the last View once the focused one left', (
    tester,
  ) async {
    await kit.show(tester, [const TextPart('Plan'), const TextPart('Gone')]);
    final views = kit.fake.views[kit.page]!;
    final gone = views.last.id;
    await tester.tap(find.text('Gone'));
    await tester.pumpAndSettle();
    await kit.workspace.apply(
      mutation(BridgeMutationKind.removeView, page: kit.page, view: gone),
    );
    await tester.pumpAndSettle();
    await open(tester);
    await type(tester, '/white');
    await enter(tester);
    final added = kit.applied(BridgeMutationKind.addWhiteboardBlock).single;
    expect(added.after, views.first.id);
  });

  testWidgets('> lists actions; Arrange turns on Arrange mode', (tester) async {
    await kit.show(tester, [const TextPart('Plan')]);
    await open(tester);
    await type(tester, '>');
    expect(inBar('Only actions your Role allows'), findsOneWidget);
    await type(tester, '>arr');
    await enter(tester);
    expect(find.byKey(const Key('arrange bar')), findsOneWidget);
  });

  testWidgets('arrow keys choose among the results', (tester) async {
    await kit.show(tester, [const TextPart('Plan')]);
    await open(tester);
    await type(tester, '>');
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();
    await enter(tester);
    expect(
      kit.workspace.areas.value
          .map((area) => kit.workspace.pages(area.id).value.length)
          .reduce((a, b) => a + b),
      2,
      reason: 'the second action, New Page, ran',
    );
  });

  testWidgets('a tap of the Liquid key opens it, and the sidebar does too', (
    tester,
  ) async {
    await kit.show(tester, [
      const TextPart('Plan'),
    ], shortcuts: ShortcutController.memory(ShortcutSettings.liquidKey()));
    expect(find.text('⇪K'), findsOneWidget, reason: 'the hint follows keys');
    await tester.sendKeyDownEvent(liquidKey, platform: 'macos');
    await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
    await tester.pumpAndSettle();
    expect(bar(), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape, platform: 'macos');
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(liquidKey, platform: 'macos');
    await tester.sendKeyEvent(LogicalKeyboardKey.keyL, platform: 'macos');
    await tester.sendKeyUpEvent(liquidKey, platform: 'macos');
    await tester.pumpAndSettle();
    expect(bar(), findsNothing, reason: 'the Liquid key held with another');
    await tester.tap(find.text('Search or run a command'));
    await tester.pumpAndSettle();
    expect(bar(), findsOneWidget);
  });
}
