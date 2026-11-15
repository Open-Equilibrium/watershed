import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeSession session;
  late SessionState state;
  final changed = <DiffScope>[];
  var pressedOutside = 0;

  /// Pumps the card beside the chat, over an open session in `worktree` on a feature branch with
  /// two changed files and an open pull request.
  Future<void> pump(
    WidgetTester tester, {
    SessionCardSettings settings = const SessionCardSettings(),
    String? worktree = '/repo/.worktrees/fix-login',
    bool beside = true,
  }) async {
    changed.clear();
    pressedOutside = 0;
    final git = FakeGit(
      status: gitStatusOf(
        branch: 'feature',
        defaultBranch: false,
        changes: [
          fileOf('a.dart', added: 2),
          fileOf('b.dart', added: 1, removed: 1),
        ],
        pullRequest: pullRequestOf(),
      ),
    );
    session = FakeSession()..gitBackend = git;
    state = SessionState(
      FakeProject(session: session, git: git)
        ..threadList = [threadOf('thread-1', worktree: worktree)],
    );
    addTearDown(state.dispose);
    await state.open('thread-1');
    await state.loadStart();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Material(
          child: Align(
            alignment: Alignment.topRight,
            child: SizedBox(
              width: 280,
              height: 600,
              child: SessionCard(
                state: state,
                settings: SessionCardSettingsController(settings),
                onChanges: changed.add,
                beside: beside,
                onOutsidePress: () => pressedOutside++,
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'the card lists the workspace, branch, pull request, Git action and changes rows in T3 order',
    (tester) async {
      await pump(tester);
      expect(find.text('fix-login'), findsOneWidget);
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

  testWidgets('a session with no worktree names the local checkout', (
    tester,
  ) async {
    await pump(tester, worktree: null);
    expect(find.text('Local checkout'), findsOneWidget);
  });

  testWidgets(
    'the Changes row sums the uncommitted lines and opens the Diff panel for them',
    (tester) async {
      await pump(tester);
      expect(find.text('+3'), findsOneWidget);
      expect(find.text('−1'), findsOneWidget);
      await tester.tap(find.byKey(const Key('changes-row')));
      expect(changed.single, isA<UncommittedChanges>());
    },
  );

  testWidgets(
    'the context window row shows the tokens used against the window',
    (tester) async {
      await pump(tester);
      session.contextWindow = const BridgeContext(
        tokens: 45000,
        window: 200000,
      );
      await state.readStatistics();
      await tester.pumpAndSettle();
      expect(find.text('Context window'), findsOneWidget);
      expect(find.text('45.0 k / 200 k'), findsOneWidget);
    },
  );

  testWidgets('a press outside a popover card closes it', (tester) async {
    await pump(tester, beside: false);
    await tester.tapAt(const Offset(20, 20));
    await tester.pumpAndSettle();
    expect(pressedOutside, 1);
  });

  testWidgets(
    'hidden rows leave the card, and with none left it offers Choose what to show',
    (tester) async {
      await pump(
        tester,
        settings: SessionCardSettings(hidden: CardItem.values.toSet()),
      );
      expect(find.byKey(const Key('branch-row')), findsNothing);
      expect(find.byKey(const Key('changes-row')), findsNothing);
      await tester.tap(find.byKey(const Key('choose-rows')));
      await tester.pumpAndSettle();
      expect(find.text('Changes'), findsOneWidget);
    },
  );
}
