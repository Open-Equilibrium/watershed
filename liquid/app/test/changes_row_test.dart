import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/changes_row.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  Future<void> pump(
    WidgetTester tester,
    BridgeGitStatus status, {
    ValueChanged<DiffScope>? onOpen,
  }) async {
    final state = SessionState(FakeProject(git: FakeGit(status: status)));
    addTearDown(state.dispose);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 280,
              child: ChangesRow(state: state, onOpen: onOpen),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'the branch against its base names the base and counts the branch\'s lines',
    (tester) async {
      await pump(
        tester,
        gitStatusOf(
          branch: 'feature',
          defaultBranch: false,
          changes: [fileOf('a.dart')],
          workingAdded: 2,
          workingRemoved: 1,
          branchBase: 'main',
          branchAdded: 12,
          branchRemoved: 3,
        ),
      );
      expect(find.text('Changes'), findsOneWidget);
      expect(find.text('vs main'), findsOneWidget);
      expect(find.text('+12'), findsOneWidget);
      expect(find.text('−3'), findsOneWidget);
    },
  );

  testWidgets(
    'with no lines on the branch, the uncommitted changes and their lines',
    (tester) async {
      await pump(
        tester,
        gitStatusOf(
          branch: 'feature',
          defaultBranch: false,
          changes: [fileOf('a.dart')],
          workingAdded: 2,
          workingRemoved: 1,
          branchBase: 'main',
        ),
      );
      expect(find.text('Uncommitted'), findsOneWidget);
      expect(find.text('+2'), findsOneWidget);
      expect(find.text('−1'), findsOneWidget);
    },
  );

  testWidgets('a repository with no changes names the branch base at +0 −0', (
    tester,
  ) async {
    await pump(tester, gitStatusOf(branch: 'feature', branchBase: 'main'));
    expect(find.text('vs main'), findsOneWidget);
    expect(find.text('+0'), findsOneWidget);
    expect(find.text('−0'), findsOneWidget);
  });

  testWidgets('a folder outside Git shows no Changes row', (tester) async {
    await pump(tester, gitStatusOf(repository: false));
    expect(find.byKey(const Key('changes-row')), findsNothing);
  });

  testWidgets('a tap opens the Diff at the changes the row names', (
    tester,
  ) async {
    final opened = <DiffScope>[];
    await pump(
      tester,
      gitStatusOf(branch: 'feature', branchBase: 'main'),
      onOpen: opened.add,
    );
    await tester.tap(find.byKey(const Key('changes-row')));
    await pump(
      tester,
      gitStatusOf(workingAdded: 2, workingRemoved: 1),
      onOpen: opened.add,
    );
    await tester.tap(find.byKey(const Key('changes-row')));
    expect(opened.map((scope) => scope.runtimeType), [
      BranchChanges,
      UncommittedChanges,
    ]);
  });
}
