import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/appearance.dart';
import 'package:liquid/src/controls.dart';
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

  testWidgets('Escape and Cancel return the configured dialog result', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: const Scaffold(body: SizedBox()),
      ),
    );
    final context = tester.element(find.byType(Scaffold));
    for (final escape in [false, true]) {
      final result = showDialog<String>(
        context: context,
        builder: (_) => const ActionDialog(
          title: 'Confirm',
          cancelled: 'cancelled',
          actions: [Text('Proceed')],
          children: [Text('Choose an action.')],
        ),
      );
      await tester.pumpAndSettle();
      if (escape) {
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      } else {
        await tester.tap(find.text('Cancel'));
      }
      await tester.pumpAndSettle();
      expect(await result, 'cancelled');
    }
  });

  Future<void> show(
    WidgetTester tester, {
    BridgeGitStatus? git,
    Size size = const Size(1600, 900),
    TextSize textSize = TextSize.standard,
  }) async {
    await tester.binding.setSurfaceSize(size);
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
        builder: (context, child) =>
            UiScale(scale: textSize.scale, child: child!),
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

  testWidgets('the branch chip fits the minimum window with Large text', (
    tester,
  ) async {
    await show(
      tester,
      git: gitStatusOf(upstream: null),
      size: const Size(1050, 900),
      textSize: TextSize.large,
    );
    expect(tester.takeException(), isNull);
    expect(tester.getSize(chipText('main')).width, greaterThan(0));
    expect(chipText('no upstream'), findsOneWidget);

    backend.gitError = 'git could not start: No such file or directory';
    await state.git.refresh();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.text('Git unavailable'), findsOneWidget);
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

  testWidgets('the branch menu opens through its accessibility action', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    try {
      await show(tester, git: gitStatusOf());
      final chip = tester.getSemantics(find.byKey(const Key('branch-chip')));
      expect(chip.getSemanticsData().hasAction(SemanticsAction.tap), isTrue);
      tester.binding.renderViews.single.owner!.semanticsOwner!.performAction(
        chip.id,
        SemanticsAction.tap,
      );
      await tester.pumpAndSettle();
      expect(find.text('feature/token-bucket'), findsOneWidget);
      expect(find.text('origin/release-1.2'), findsOneWidget);
    } finally {
      semantics.dispose();
    }
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
      current: true,
      id: 's1',
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
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.switchConfirmation, (branch: 'main', stash: 's1'));
    await tester.pump(const Duration(seconds: 7));
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

  testWidgets('a changed stash invalidates the open replacement confirmation', (
    tester,
  ) async {
    await show(tester);
    await openMenu(tester);
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    backend.stashed = const BridgeStash(
      current: true,
      id: 'unconfirmed',
      createdMs: 1,
      files: [],
    );
    await state.git.refresh();

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(backend.calls.where((call) => call.startsWith('switch ')), isEmpty);
    expect(
      find.textContaining('The checkout branch or stash changed.'),
      findsOneWidget,
    );
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets(
    'leaving refuses a branch list inconsistent with the displayed source',
    (tester) async {
      await show(tester);
      backend.branches = BridgeBranches(
        branches: [
          branchOf('main'),
          branchOf('other', current: true, stashed: true),
          branchOf('feature/token-bucket'),
        ],
        running: const [],
      );
      backend.stashed = const BridgeStash(
        current: true,
        id: 'other-stash',
        createdMs: 1,
        files: [],
      );
      await openMenu(tester);
      await tester.tap(find.text('feature/token-bucket'));
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();

      expect(
        backend.calls.where((call) => call.startsWith('switch ')),
        isEmpty,
      );
      expect(
        find.textContaining('The checkout branch or stash changed.'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets('leaving confirms the raw branch behind its displayed name', (
    tester,
  ) async {
    const raw = 'feature/\u202e';
    const shown = 'feature/<U+202E>';
    await show(tester, git: gitStatusOf(branch: shown, changed: 1));
    backend.branches = BridgeBranches(
      branches: [
        const BridgeBranch(
          name: raw,
          shown: shown,
          remote: false,
          committedMs: 0,
          current: true,
          stashed: false,
        ),
        branchOf('feature/token-bucket'),
      ],
      running: const [],
    );
    await openMenu(tester);
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    expect(find.text('Leave my changes on $shown'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(backend.switchConfirmation, (branch: raw, stash: null));
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

  testWidgets('a refused Leave retry keeps its current refusal visible', (
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
    backend.outcome = outcomeOf(
      BridgeGitOutcomeKind.wouldOverwrite,
      files: ['config.local'],
    );

    await tester.tap(find.text('Leave them on main and switch'));
    await tester.pumpAndSettle();

    expect(backend.calls.last, 'switch feature/token-bucket leaving changes');
    expect(
      find.text('Could not switch to feature/token-bucket'),
      findsOneWidget,
    );
    expect(find.text('config.local'), findsOneWidget);
    expect(find.text('src/routes/login.ts'), findsNothing);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
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
