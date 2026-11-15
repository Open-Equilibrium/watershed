import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/branch_menu.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

/// A checkout whose status stream fails, as a broken repository does.
class _BrokenGit extends FakeGit {
  @override
  Stream<BridgeGitStatus> watch() => Stream.error(Exception('no repository'));
}

void main() {
  late FakeSession session;
  late FakeGit git;
  late SessionState state;

  /// Opens a session on the feature branch, with its refs, and shows the branch row. The
  /// session's turns are `turns`.
  Future<void> pump(
    WidgetTester tester, {
    FakeGit? checkout,
    List<BridgeTurn> turns = const [],
  }) async {
    git =
        checkout ??
        FakeGit(status: gitStatusOf(branch: 'feature', defaultBranch: false));
    git.refList = [
      refOf('main', isDefault: true),
      refOf('feature', current: true),
      refOf('fix', worktree: '/worktrees/fix'),
      refOf('origin/fix', remote: 'origin'),
    ];
    session = FakeSession()
      ..gitBackend = git
      ..current = snapshotOf(turns: turns)
      ..turnList = turns;
    state = SessionState(FakeProject(session: session, git: git));
    addTearDown(state.dispose);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(width: 280, child: BranchRow(state: state)),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> openPicker(WidgetTester tester) async {
    await tester.tap(find.byKey(const Key('branch-chevron')));
    await tester.pumpAndSettle();
  }

  testWidgets('the branch name and its chevron both open the ref picker', (
    tester,
  ) async {
    await pump(tester);
    await tester.tap(find.byKey(const Key('branch-name')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('branch-search')), findsOneWidget);
    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    await openPicker(tester);
    expect(find.byKey(const Key('branch-search')), findsOneWidget);
  });

  testWidgets(
    'the ref picker names each ref, and a search narrows the list and offers a new ref',
    (tester) async {
      await pump(tester);
      await openPicker(tester);
      for (final detail in ['current', 'default', 'worktree', 'remote']) {
        expect(find.text(detail), findsOneWidget);
      }
      await tester.enterText(find.byKey(const Key('branch-search')), 'fea');
      await tester.pumpAndSettle();
      expect(find.text('main'), findsNothing);
      expect(find.text('Create new ref "fea"'), findsOneWidget);
    },
  );

  testWidgets('Enter switches to the first ref the search lists', (
    tester,
  ) async {
    await pump(tester);
    await openPicker(tester);
    await tester.enterText(find.byKey(const Key('branch-search')), 'main');
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(git.calls, ['switch main']);
  });

  testWidgets('a remote ref switches to the local branch of its name', (
    tester,
  ) async {
    await pump(tester);
    await openPicker(tester);
    await tester.tap(find.text('origin/fix'));
    await tester.pumpAndSettle();
    expect(git.calls, ['switch fix']);
  });

  testWidgets(
    'a ref checked out in a worktree opens that worktree in the session',
    (tester) async {
      await pump(tester);
      await openPicker(tester);
      await tester.tap(find.text('fix'));
      await tester.pumpAndSettle();
      expect(session.calls, ['branch fix worktree=/worktrees/fix']);
    },
  );

  testWidgets(
    'a switch with uncommitted changes asks first, and Cancel keeps the branch',
    (tester) async {
      await pump(tester);
      git.change(
        gitStatusOf(
          branch: 'feature',
          defaultBranch: false,
          changes: [fileOf('a.dart')],
        ),
      );
      await tester.pumpAndSettle();
      await openPicker(tester);
      await tester.tap(find.text('main'));
      await tester.pumpAndSettle();
      expect(find.text('Switch to main?'), findsOneWidget);
      await tester.tap(find.widgetWithText(SecondaryButton, 'Cancel'));
      await tester.pumpAndSettle();
      expect(git.calls, isEmpty);
      await openPicker(tester);
      await tester.tap(find.text('main'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(PrimaryButton, 'Switch branch'));
      await tester.pumpAndSettle();
      expect(git.calls, ['switch main']);
    },
  );

  testWidgets(
    'a new name creates and checks out a ref, its spaces made dashes',
    (tester) async {
      await pump(tester);
      await openPicker(tester);
      await tester.enterText(
        find.byKey(const Key('branch-search')),
        'new feature',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Create new ref "new-feature"'));
      await tester.pumpAndSettle();
      expect(git.calls, ['create new-feature']);
    },
  );

  testWidgets('Git refusing a new ref says why', (tester) async {
    await pump(tester);
    await openPicker(tester);
    await tester.enterText(find.byKey(const Key('branch-search')), 'taken');
    await tester.pumpAndSettle();
    git.refError = Exception('ref exists');
    await tester.tap(find.text('Create new ref "taken"'));
    await tester.pumpAndSettle();
    expect(
      state.notice.value?.text,
      contains('Failed to create and switch ref.'),
    );
  });

  testWidgets(
    'while a turn runs the picker offers no switch and says to stop it',
    (tester) async {
      await pump(tester, turns: [turnOf(1, status: BridgeTurnStatus.running)]);
      await openPicker(tester);
      expect(
        find.text('Stop the current turn to switch worktrees.'),
        findsOneWidget,
      );
      await tester.tap(find.text('main'));
      await tester.pumpAndSettle();
      expect(git.calls, isEmpty);
      expect(find.byKey(const Key('branch-search')), findsOneWidget);
    },
  );

  testWidgets(
    'a pull request reference typed in the picker offers its checkout',
    (tester) async {
      await pump(tester);
      await openPicker(tester);
      await tester.enterText(find.byKey(const Key('branch-search')), '#42');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Checkout pull request'));
      await tester.pump(const Duration(milliseconds: 450));
      await tester.pumpAndSettle();
      expect(find.text('Add a feature'), findsOneWidget);
      expect(git.calls, ['resolve 42']);
    },
  );

  testWidgets('a ref list Git cannot read says so and closes the picker', (
    tester,
  ) async {
    await pump(tester);
    git.refError = Exception('busy');
    await openPicker(tester);
    expect(state.notice.value?.text, contains('Could not list the refs:'));
    expect(find.byKey(const Key('branch-search')), findsNothing);
  });

  testWidgets(
    'the branch row opens the ref picker through its accessibility action',
    (tester) async {
      await pump(tester);
      final semantics = tester.ensureSemantics();
      await tester.pumpAndSettle();
      tester.semantics.tap(find.semantics.byLabel('Branch feature'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('branch-search')), findsOneWidget);
      semantics.dispose();
    },
  );

  testWidgets('right-clicking the branch name offers to copy it', (
    tester,
  ) async {
    await pump(tester);
    await tester.tap(
      find.byKey(const Key('branch-name')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    expect(find.text('Copy branch name'), findsOneWidget);
  });

  testWidgets('a checkout Git cannot read says so in the branch row', (
    tester,
  ) async {
    await pump(tester, checkout: _BrokenGit());
    expect(find.text('Git unavailable'), findsOneWidget);
  });
}
