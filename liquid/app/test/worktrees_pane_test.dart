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
}
