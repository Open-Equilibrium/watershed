import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/worktrees_pane.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/rust/api/worktrees.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;
  final hotfix = worktreeOf('/code/api-hotfix', branch: 'hotfix/login');

  Future<void> show(WidgetTester tester) async {
    backend = FakeBackend()
      ..worktreeList = [
        worktreeOf('/code/api', branch: 'main', main: true, current: true),
        hotfix,
        worktreeOf(
          '/worktrees/api/fix-sync',
          branch: 'fix/sync',
          running: const [BridgeRunningAgent(own: false, title: 'Fix sync')],
        ),
        worktreeOf('/code/api-old', branch: 'release/1.1', locked: true),
        worktreeOf('/code/api-gone', branch: 'spike/cache', missing: true),
      ];
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: WorktreesPane(state: state, device: 'MacBook'),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder button(String label) => find.widgetWithText(InkWell, label);

  testWidgets("lists Git's worktrees and what keeps each", (tester) async {
    await show(tester);

    expect(find.text('/code/api-hotfix'), findsOneWidget);
    expect(find.text('hotfix/login'), findsOneWidget);
    expect(find.text('main checkout · this window'), findsOneWidget);
    expect(find.text('agent running'), findsOneWidget);
    expect(find.text('locked by Git'), findsOneWidget);
    expect(find.text('folder missing'), findsOneWidget);
    // Only the clean, idle, linked worktree offers removal.
    expect(button('Remove…'), findsOneWidget);
  });

  testWidgets('removes a clean worktree after its review and lists again', (
    tester,
  ) async {
    await show(tester);
    backend.reviews[hotfix.path] = removalOf(hotfix, unpushed: 1);

    await tester.tap(button('Remove…'));
    await tester.pumpAndSettle();

    expect(find.text('Remove hotfix/login worktree?'), findsOneWidget);
    expect(
      find.text(
        'Removes /code/api-hotfix on MacBook. Git reports no staged, unstaged, untracked or ignored files to preserve.',
      ),
      findsOneWidget,
    );
    expect(
      find.text(
        'The branch has 1 unpushed commit. The branch and its commits remain in this repository.',
      ),
      findsOneWidget,
    );

    backend.worktreeList = [backend.worktreeList.first];
    await tester.tap(button('Remove worktree'));
    await tester.pumpAndSettle();

    expect(backend.calls, contains('remove /code/api-hotfix'));
    expect(find.text('Remove hotfix/login worktree?'), findsNothing);
    expect(find.text('/code/api-hotfix'), findsNothing);
    // The notice of the removal.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('a worktree with local files is kept and its files named', (
    tester,
  ) async {
    await show(tester);
    backend.reviews[hotfix.path] = removalOf(
      hotfix,
      files: const [
        BridgeWorktreeFile(
          path: 'src/login.ts',
          shown: 'src/login.ts',
          state: BridgeWorktreeFileState.staged,
        ),
        BridgeWorktreeFile(
          path: 'notes.md',
          shown: 'notes.md',
          state: BridgeWorktreeFileState.untracked,
        ),
      ],
    );

    await tester.tap(button('Remove…'));
    await tester.pumpAndSettle();

    expect(find.text('Could not remove this worktree'), findsOneWidget);
    expect(find.textContaining('2 files have local changes.'), findsOneWidget);
    expect(find.text('src/login.ts'), findsOneWidget);
    expect(find.text('untracked'), findsOneWidget);
    expect(tester.widget<InkWell>(button('Remove worktree')).onTap, isNull);
  });

  testWidgets('ignored files keep a worktree too', (tester) async {
    await show(tester);
    backend.reviews[hotfix.path] = removalOf(hotfix, ignored: 142);

    await tester.tap(button('Remove…'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Git ignores 142 files in this folder. Checkpoints and a normal stash do not save them. Move or remove them yourself before removing the worktree.',
      ),
      findsOneWidget,
    );
    expect(button('Open folder'), findsOneWidget);
  });

  testWidgets('a worktree that changed during review is shown again', (
    tester,
  ) async {
    await show(tester);
    backend.reviews[hotfix.path] = removalOf(hotfix);
    await tester.tap(button('Remove…'));
    await tester.pumpAndSettle();
    backend.removed = BridgeWorktreeOutcome(
      kind: BridgeWorktreeOutcomeKind.changed,
      removal: removalOf(
        hotfix,
        files: const [
          BridgeWorktreeFile(
            path: 'late.txt',
            shown: 'late.txt',
            state: BridgeWorktreeFileState.untracked,
          ),
        ],
      ),
      running: const [],
      detail: '',
    );

    await tester.tap(button('Remove worktree'));
    await tester.pumpAndSettle();

    expect(find.text('Could not remove this worktree'), findsOneWidget);
    expect(
      find.textContaining('changed since this dialog opened'),
      findsOneWidget,
    );
    expect(find.text('late.txt'), findsOneWidget);
  });

  Future<void> review(
    WidgetTester tester,
    BridgeWorktreeRemoval removal,
  ) async {
    backend.reviews[hotfix.path] = removal;
    await tester.tap(button('Remove…'));
    await tester.pumpAndSettle();
  }

  testWidgets('says why a worktree is kept when only its state keeps it', (
    tester,
  ) async {
    await show(tester);
    await review(
      tester,
      removalOf(
        worktreeOf('/code/api-hotfix', branch: 'hotfix/login', locked: true),
      ),
    );

    expect(
      find.text(
        'A worktree locked by Git stays locked; Liquid does not force removal.',
      ),
      findsOneWidget,
    );
  });

  testWidgets('keeps a detached worktree whose commits no branch holds', (
    tester,
  ) async {
    await show(tester);
    await review(
      tester,
      removalOf(worktreeOf('/code/api-hotfix'), stranded: 2),
    );

    expect(
      find.text(
        '2 commits are on no branch in this detached worktree and would be lost with it. Create a branch for them first.',
      ),
      findsOneWidget,
    );
    expect(tester.widget<InkWell>(button('Remove worktree')).onTap, isNull);
  });

  testWidgets(
    'a refused or failed removal keeps the worktree and lists again',
    (tester) async {
      await show(tester);
      await review(tester, removalOf(hotfix));
      backend.removed = const BridgeWorktreeOutcome(
        kind: BridgeWorktreeOutcomeKind.agentRunning,
        running: [BridgeRunningAgent(own: false, title: 'Fix sync')],
        detail: '',
      );

      await tester.tap(button('Remove worktree'));
      await tester.pumpAndSettle();

      expect(
        find.text(
          '“Fix sync” is running in this repository. Stop it or wait until it finishes; nothing was removed.',
        ),
        findsOneWidget,
      );

      backend.removed = const BridgeWorktreeOutcome(
        kind: BridgeWorktreeOutcomeKind.failed,
        running: [],
        detail: 'fatal: cannot remove',
      );
      await tester.tap(button('Remove worktree'));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'Could not remove this worktree: “fatal: cannot remove” It stays listed until Git removes it.',
        ),
        findsOneWidget,
      );

      final listed = backend.calls.where((call) => call == 'worktrees').length;
      await tester.tap(button('Cancel'));
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call == 'worktrees'),
        hasLength(listed + 1),
      );
    },
  );

  testWidgets('a folder outside Git has no worktrees to list', (tester) async {
    backend = FakeBackend()..git = null;
    state = SessionState(backend);
    await state.git.refresh();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: WorktreesPane(state: state, device: 'MacBook'),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.text('This folder has no Git repository, so it has no worktrees.'),
      findsOneWidget,
    );
    expect(backend.calls, isNot(contains('worktrees')));
  });

  testWidgets('files hidden from Git are named with how to clear the marks', (
    tester,
  ) async {
    await show(tester);
    await review(
      tester,
      removalOf(
        hotfix,
        files: const [
          BridgeWorktreeFile(
            path: 'config.local',
            shown: 'config.local',
            state: BridgeWorktreeFileState.marked,
          ),
        ],
      ),
    );

    expect(
      find.text(
        '1 file is marked so that Git does not check it for edits, which removing the folder would lose. Clear the marks with git update-index --no-assume-unchanged or --no-skip-worktree, then review again.',
      ),
      findsOneWidget,
    );
    expect(find.textContaining('local changes'), findsNothing);
    expect(find.text('hidden from Git'), findsOneWidget);
  });

  testWidgets('another action using the repository is not called an agent', (
    tester,
  ) async {
    await show(tester);
    await review(tester, removalOf(hotfix));
    backend.removed = const BridgeWorktreeOutcome(
      kind: BridgeWorktreeOutcomeKind.agentRunning,
      running: [],
      detail: '',
    );

    await tester.tap(button('Remove worktree'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Another action is using this repository; try again when it ends. Nothing was removed.',
      ),
      findsOneWidget,
    );
  });
}
