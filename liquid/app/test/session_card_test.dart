import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

FakeSession dirtySession() {
  final git = FakeGit(
    status: gitStatusOf(
      branch: 'feature',
      defaultBranch: false,
      changes: [
        fileOf('a.dart', added: 2),
        fileOf('b.dart', added: 1, removed: 1),
      ],
      workingAdded: 3,
      workingRemoved: 1,
      pullRequest: pullRequestOf(),
    ),
  );
  return FakeSession()..gitBackend = git;
}

final allDefaultRowsHidden = SessionCardSettings(
  hidden: CardItem.values.where((item) => item.shownByDefault).toSet(),
);

void expectInOrder(WidgetTester tester, List<Finder> rows) {
  for (var index = 0; index < rows.length; index++) {
    expect(rows[index], findsOneWidget);
    if (index > 0) {
      expect(
        tester.getTopLeft(rows[index]).dy,
        greaterThan(tester.getTopLeft(rows[index - 1]).dy),
      );
    }
  }
}

void main() {
  testWidgets('hidden rows leave; Choose lists every row', (tester) async {
    await pumpCard(
      tester,
      session: dirtySession(),
      settings: allDefaultRowsHidden,
    );
    expect(find.byKey(const Key('branch-row')), findsNothing);
    expect(find.byKey(const Key('changes-row')), findsNothing);
    await tester.tap(find.byKey(const Key('choose-rows')));
    await tester.pumpAndSettle();
    expect(find.text('Show in thread details'), findsOneWidget);
    expectInOrder(tester, [
      for (final item in CardItem.values)
        find.widgetWithText(CheckboxMenuButton, item.label),
    ]);
  });

  testWidgets('a press outside a popover card closes it', (tester) async {
    var pressedOutside = 0;
    await pumpCard(
      tester,
      session: dirtySession(),
      beside: false,
      onOutsidePress: () => pressedOutside++,
    );
    await tester.tapAt(const Offset(20, 20));
    await tester.pumpAndSettle();
    expect(pressedOutside, 1);
  });
}
