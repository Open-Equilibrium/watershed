import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

Future<({FakeGit git, SessionState state})> openCommit(
  WidgetTester tester,
  List<BridgeFileChange> changes,
) async {
  final git = FakeGit(
    status: gitStatusOf(
      branch: 'feature',
      defaultBranch: false,
      changes: changes,
    ),
  );
  final state = SessionState(FakeProject(git: git));
  await tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(body: GitActionsControl(state: state)),
    ),
  );
  await tester.pump();
  await tap(tester, find.byTooltip('Git action options'));
  await tap(tester, find.text('Commit'));
  return (git: git, state: state);
}

Finder fileRow(String path) =>
    find.ancestor(of: find.text(path), matching: find.byType(Row)).first;

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'Commit lists the changed files and commits them all, the message optional',
    (tester) async {
      final (:git, :state) = await openCommit(tester, [
        fileOf('lib/a.dart', added: 3),
        fileOf('b.md', added: 1, removed: 1),
      ]);
      expect(find.text('Commit changes'), findsOneWidget);
      expect(find.text('lib/a.dart'), findsOneWidget);
      expect(find.text('+4 / -1'), findsOneWidget);
      await tap(tester, find.widgetWithText(PrimaryButton, 'Commit'));
      expect(git.calls, ['run commit message=null files=null feature=false']);
      await state.dispose();
    },
  );

  testWidgets(
    'a file left out is not committed, and with none left both commits are off',
    (tester) async {
      final (:git, :state) = await openCommit(tester, [
        fileOf('a.dart'),
        fileOf('b.dart'),
      ]);
      await tap(tester, find.widgetWithText(GhostButton, 'Edit'));
      await tap(tester, find.byType(Checkbox).first);
      expect(find.text('Excluded'), findsNWidgets(2));
      expect(
        tester
            .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<SecondaryButton>(
              find.widgetWithText(SecondaryButton, 'Commit on new branch'),
            )
            .onPressed,
        isNull,
      );
      await tap(
        tester,
        find.descendant(of: fileRow('b.dart'), matching: find.byType(Checkbox)),
      );
      expect(find.text('Excluded'), findsOneWidget);
      await tap(tester, find.widgetWithText(GhostButton, 'Done'));
      expect(find.text('(1 of 2)'), findsOneWidget);
      await tap(tester, find.widgetWithText(PrimaryButton, 'Commit'));
      expect(git.calls, ['run commit message=null files=b.dart feature=false']);
      await state.dispose();
    },
  );

  testWidgets('Commit on new branch commits there with the message typed', (
    tester,
  ) async {
    final (:git, :state) = await openCommit(tester, [fileOf('a.dart')]);
    await tester.enterText(find.byType(TextField), 'Fix login');
    await tap(
      tester,
      find.widgetWithText(SecondaryButton, 'Commit on new branch'),
    );
    expect(git.calls, ['run commit message=Fix login files=null feature=true']);
    await state.dispose();
  });
}
