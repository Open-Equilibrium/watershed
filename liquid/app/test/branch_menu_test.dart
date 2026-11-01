import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(WidgetTester tester, {BridgeGitStatus? git}) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()
      ..git = git ?? gitStatusOf(ahead: 1, changed: 4)
      ..branches = BridgeBranches(
        branches: [
          branchOf('main', current: true),
          branchOf('feature/token-bucket'),
          branchOf('fix/sync-flake', stashed: true),
          branchOf('origin/release-1.2', remote: true),
        ],
        running: const [],
      );
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder chipText(String text) => find.descendant(
    of: find.byKey(const Key('branch-chip')),
    matching: find.text(text),
  );

  Future<void> openMenu(WidgetTester tester) async {
    await tester.tap(find.byKey(const Key('branch-chip')));
    await tester.pumpAndSettle();
  }

  testWidgets('the header and the sidebar show the checkout’s branch', (
    tester,
  ) async {
    await show(tester, git: gitStatusOf(ahead: 1, behind: 3));
    expect(chipText('main'), findsOneWidget);
    expect(find.text('↑1'), findsOneWidget);
    expect(find.text('↓3'), findsOneWidget);
    expect(find.text('pi · main · MacBook'), findsOneWidget);
  });

  testWidgets('a detached HEAD shows its commit, broken Git says so', (
    tester,
  ) async {
    await show(tester, git: gitStatusOf(branch: null, upstream: null));
    expect(chipText('1a2b3c4'), findsOneWidget);

    backend.gitError = 'git could not start: No such file or directory';
    await state.git.refresh();
    await tester.pumpAndSettle();
    expect(find.text('Git unavailable'), findsOneWidget);
  });

  testWidgets('switching with uncommitted changes asks where they go', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester);
    expect(find.textContaining('stashed changes ·'), findsOneWidget);
    expect(
      find.text(
        'You have 4 uncommitted changes; switching asks where they go.',
      ),
      findsOneWidget,
    );
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();

    expect(find.text('Switch branch'), findsWidgets);
    expect(find.text('Leave my changes on main'), findsOneWidget);
    await tester.tap(find.text('Bring my changes to feature/token-bucket'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'switch feature/token-bucket');

    await openMenu(tester);
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    // Leaving them is the default, as in GitHub Desktop.
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'switch feature/token-bucket leaving changes');
    // The notice fades.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('a branch with stashed changes says that leaving replaces them', (
    tester,
  ) async {
    await show(tester);
    backend.stashed = BridgeStash(
      createdMs: DateTime(2026, 11, 1, 10, 12).millisecondsSinceEpoch,
      files: [
        BridgeFileChange(
          path: 'a.txt',
          shown: 'a.txt',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 0,
        ),
      ],
    );
    await state.git.refresh();
    await openMenu(tester);
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    expect(
      find.text(
        '⚠ main already has stashed changes from 10:12 (1 file). Leaving these changes '
        'replaces them.',
      ),
      findsOneWidget,
    );
    expect(find.text('Replace stash and switch'), findsOneWidget);
  });

  testWidgets('without changes a branch switches at once', (tester) async {
    await show(tester, git: gitStatusOf());
    await openMenu(tester);
    await tester.tap(find.text('origin/release-1.2'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'switch origin/release-1.2');
    expect(find.text('Switch branch'), findsNothing);
    // The notice fades.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('Git refusing to bring the changes offers to leave them', (
    tester,
  ) async {
    await show(tester);
    backend.outcome = outcomeOf(
      BridgeGitOutcomeKind.wouldOverwrite,
      files: ['src/routes/login.ts'],
    );
    await openMenu(tester);
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Bring my changes to feature/token-bucket'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(
      find.text('Could not switch to feature/token-bucket'),
      findsOneWidget,
    );
    expect(find.text('src/routes/login.ts'), findsOneWidget);
    backend.outcome = outcomeOf(BridgeGitOutcomeKind.done);
    await tester.tap(find.text('Leave them on main and switch'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'switch feature/token-bucket leaving changes');
    // The notice fades.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('no branch is chosen while an agent runs in the checkout', (
    tester,
  ) async {
    await show(tester);
    backend.branches = BridgeBranches(
      branches: [
        branchOf('main', current: true),
        branchOf('feature/token-bucket'),
      ],
      running: const [
        BridgeRunningAgent(own: false, title: 'Fix flaky sync test'),
      ],
    );
    await openMenu(tester);
    expect(
      find.text(
        '“Fix flaky sync test” is running in this checkout. Stop it or wait until it '
        'finishes, then switch branches.',
      ),
      findsOneWidget,
    );
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call.startsWith('switch')), isEmpty);
  });

  testWidgets('a new name creates a branch unless Git refuses it', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester);
    await tester.enterText(
      find.byKey(const Key('branch-search')),
      'fix..login',
    );
    await tester.pumpAndSettle();
    expect(find.text('No branch matches'), findsOneWidget);
    expect(
      find.text('⚠ Git does not allow “..” in a branch name.'),
      findsOneWidget,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call.startsWith('create')), isEmpty);

    await tester.enterText(
      find.byKey(const Key('branch-search')),
      'feature/retry-after',
    );
    await tester.pumpAndSettle();
    expect(
      find.text(
        'Starts from the current commit on main; your 4 uncommitted changes come along.',
      ),
      findsOneWidget,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'create feature/retry-after');
    // The notice fades.
    await tester.pump(const Duration(seconds: 7));
  });
}
