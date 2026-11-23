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

  BridgeGitStatus branchOf({int added = 0, int removed = 0}) => gitStatusOf(
    branch: 'feature',
    defaultBranch: false,
    changes: [fileOf('a.dart')],
    workingAdded: 2,
    workingRemoved: 1,
    branchBase: 'main',
    branchAdded: added,
    branchRemoved: removed,
  );

  testWidgets('the row counts lines, opens the Diff, and says so', (
    tester,
  ) async {
    final opened = <DiffScope>[];
    await pump(tester, branchOf(added: 12, removed: 3), onOpen: opened.add);
    expect(find.text('Changes'), findsOneWidget);
    expect(find.text('vs main'), findsOneWidget);
    expect(find.text('+12'), findsOneWidget);
    expect(find.text('−3'), findsOneWidget);

    await tester.tap(find.byKey(const Key('changes-row')));
    await pump(tester, branchOf(), onOpen: opened.add);
    expect(find.text('Uncommitted'), findsOneWidget);
    expect(find.text('+2'), findsOneWidget);
    expect(find.text('−1'), findsOneWidget);
    await tester.tap(find.byKey(const Key('changes-row')));
    expect(opened.map((scope) => scope.runtimeType), [
      BranchChanges,
      UncommittedChanges,
    ]);
  });

  testWidgets('no changes read +0 −0; a non-repo has no row', (tester) async {
    await pump(tester, gitStatusOf(branch: 'feature', branchBase: 'main'));
    expect(find.text('vs main'), findsOneWidget);
    expect(find.text('+0'), findsOneWidget);
    expect(find.text('−0'), findsOneWidget);

    await pump(tester, gitStatusOf(repository: false));
    expect(find.byKey(const Key('changes-row')), findsNothing);
  });
}
