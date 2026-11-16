import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/diff/turn_scope.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

/// A session in a worktree on a feature branch with two changed files and an open pull request.
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

/// Every row the card can hide: all of them that are shown by default.
final allDefaultRowsHidden = SessionCardSettings(
  hidden: CardItem.values.where((item) => item.shownByDefault).toSet(),
);

void main() {
  testWidgets(
    'the card lists the environment, branch, pull request, Git action and changes rows in T3 order',
    (tester) async {
      await pumpCard(tester, session: dirtySession());
      expect(find.text('Mac mini'), findsOneWidget);
      expect(find.text('feature'), findsOneWidget);
      final rows = [
        find.byKey(const Key('workspace-row')),
        find.byKey(const Key('branch-row')),
        find.text('#7: Add a feature'),
        find.text('Commit & push'),
        find.byKey(const Key('changes-row')),
      ];
      for (var index = 1; index < rows.length; index++) {
        expect(
          tester.getTopLeft(rows[index]).dy,
          greaterThan(tester.getTopLeft(rows[index - 1]).dy),
        );
      }
    },
  );

  testWidgets(
    'the Changes row counts the uncommitted lines and opens the Diff panel for them',
    (tester) async {
      final changed = <DiffScope>[];
      await pumpCard(tester, session: dirtySession(), onChanges: changed.add);
      expect(find.text('+3'), findsOneWidget);
      expect(find.text('−1'), findsOneWidget);
      await tester.tap(find.byKey(const Key('changes-row')));
      expect(changed.single, isA<UncommittedChanges>());
    },
  );

  testWidgets(
    'hidden rows leave the card, and with none left it offers Choose what to show',
    (tester) async {
      await pumpCard(
        tester,
        session: dirtySession(),
        settings: allDefaultRowsHidden,
      );
      expect(find.byKey(const Key('branch-row')), findsNothing);
      expect(find.byKey(const Key('changes-row')), findsNothing);
      await tester.tap(find.byKey(const Key('choose-rows')));
      await tester.pumpAndSettle();
      expect(find.text('Changes'), findsOneWidget);
    },
  );

  testWidgets(
    'the rows menu lists every row in T3 Code order under Show in thread details',
    (tester) async {
      await pumpCard(
        tester,
        session: dirtySession(),
        settings: allDefaultRowsHidden,
      );
      await tester.tap(find.byKey(const Key('choose-rows')));
      await tester.pumpAndSettle();
      expect(find.text('Show in thread details'), findsOneWidget);
      final rows = [
        for (final item in CardItem.values)
          find.widgetWithText(CheckboxMenuButton, item.label),
      ];
      for (var index = 1; index < rows.length; index++) {
        expect(rows[index], findsOneWidget);
        expect(
          tester.getTopLeft(rows[index]).dy,
          greaterThan(tester.getTopLeft(rows[index - 1]).dy),
        );
      }
    },
  );

  testWidgets('Reset to default is off while every row has its default', (
    tester,
  ) async {
    await pumpCard(tester, session: dirtySession());
    await tester.tapAt(
      tester.getCenter(find.byKey(const Key('branch-row'))),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    final reset = find.widgetWithText(MenuItemButton, 'Reset to default');
    expect(tester.widget<MenuItemButton>(reset).onPressed, isNull);
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
