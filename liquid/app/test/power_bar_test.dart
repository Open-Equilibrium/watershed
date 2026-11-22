import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
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
    final area = kit.workspace.open.value!.area;
    final trip = await kit.workspace.apply(
      mutation(BridgeMutationKind.createPage, area: area, text: 'Trip'),
    );
    await open(tester);
    await type(tester, 'tues');
    expect(inBar('Text'), findsOneWidget);
    expect(inBar('We expect rain on Tuesday.'), findsOneWidget);
    await type(tester, 'trip');
    expect(inBar('Pages'), findsOneWidget);
    await enter(tester);
    expect(bar(), findsNothing);
    expect(kit.workspace.open.value?.page, trip.page);
    expect(kit.fake.device['recent'], contains(trip.page));
  });

  testWidgets('/ lists the Blocks, and ⏎ adds one to the Page', (tester) async {
    await kit.show(tester, [const TextPart('Plan')]);
    await open(tester);
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
}
