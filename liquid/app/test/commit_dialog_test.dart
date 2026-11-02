import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;
  Future<void> show(WidgetTester tester, {BridgeGitStatus? status}) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    backend = FakeBackend()
      ..git = status ?? gitStatusOf(changed: 4, defaultBranch: 'main');
    backend.commitPlanned = BridgeCommitPlan(
      files: [
        commitFileOf('docs/api.md', turns: [3], added: 3),
        commitFileOf('src/middleware/rateLimit.ts', turns: [1, 3], added: 5),
        commitFileOf(
          'src/routes/login.ts',
          turns: [3],
          edited: true,
          stagedPartly: true,
          added: 4,
          removed: 1,
        ),
        commitFileOf(
          'notes/todo.md',
          status: BridgeChangeStatus.added,
          added: 6,
        ),
      ],
      running: const [],
    );
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: const SessionInfo(
            project: 'acme-api',
            harness: 'pi',
            device: 'MacBook',
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> open(WidgetTester tester) async {
    await tester.tap(find.byKey(const Key('git-next')));
    await tester.pumpAndSettle();
  }

  Future<void> message(WidgetTester tester) async {
    await tester.enterText(
      find.byKey(const Key('commit-message')),
      'Return 429\n\nExplain the change.',
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'commit preselects whole files, attributes them and requires a message',
    (tester) async {
      await show(tester);
      await open(tester);
      expect(find.text('Commit changes'), findsOneWidget);
      expect(find.text('Default branch'), findsOneWidget);
      for (final label in [
        'turn 3',
        'turns 1, 3',
        'turn 3 and you',
        'not this session',
      ]) {
        expect(find.text(label), findsOneWidget);
      }
      expect(
        find.textContaining('The commit takes the whole file as it is now.'),
        findsOneWidget,
      );
      expect(
        tester
            .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
            .onPressed,
        isNull,
      );
      await message(tester);
      await tester.tap(find.widgetWithText(PrimaryButton, 'Commit'));
      await tester.pumpAndSettle();
      expect(backend.committedPaths, [
        'docs/api.md',
        'src/middleware/rateLimit.ts',
        'src/routes/login.ts',
        'notes/todo.md',
      ]);
      expect(backend.committedMessage, 'Return 429\n\nExplain the change.');
      expect(
        state.notices.value.first.text,
        'Committed 4 files on main: “Return 429”',
      );
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets('Edit excludes files and shows the mixed checkbox', (
    tester,
  ) async {
    await show(tester);
    await open(tester);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('commit-select-notes/todo.md')));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Checkbox>(find.byKey(const Key('commit-select-all'))).value,
      isNull,
    );
    expect(find.text('Excluded'), findsOneWidget);
    expect(find.textContaining('3 of 4'), findsOneWidget);
    await tester.tap(find.text('Done'));
    await message(tester);
    await tester.tap(find.widgetWithText(PrimaryButton, 'Commit'));
    await tester.pumpAndSettle();
    expect(backend.committedPaths, [
      'docs/api.md',
      'src/middleware/rateLimit.ts',
      'src/routes/login.ts',
    ]);
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets(
    'Commit on new branch carries the message and files; Back preserves edits',
    (tester) async {
      await show(tester);
      await open(tester);
      await message(tester);
      await tester.tap(find.text('Commit on new branch…'));
      await tester.pumpAndSettle();
      expect(find.text('Commit on a new branch'), findsOneWidget);
      expect(
        find.textContaining(
          'Creates the branch from the current commit on main',
        ),
        findsOneWidget,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.text('Return 429\n\nExplain the change.'), findsOneWidget);
      await tester.tap(find.text('Commit on new branch…'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('commit-branch-name')),
        'feature/retry-after',
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(backend.committedBranch, 'feature/retry-after');
      expect(backend.committedMessage, startsWith('Return 429'));
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets(
    'commit fits a small window with every file and a staged notice',
    (tester) async {
      await show(tester);
      await tester.binding.setSurfaceSize(const Size(1050, 650));
      await tester.pumpAndSettle();
      await open(tester);
      expect(tester.takeException(), isNull);
      expect(find.widgetWithText(PrimaryButton, 'Commit'), findsOneWidget);
    },
  );

  testWidgets('hook failure retains the draft and exposes Git output', (
    tester,
  ) async {
    await show(tester);
    await open(tester);
    await message(tester);
    backend.outcome = outcomeOf(
      BridgeGitOutcomeKind.commitFailed,
      detail: 'eslint: 2 errors',
    );
    await tester.tap(find.widgetWithText(PrimaryButton, 'Commit'));
    await tester.pumpAndSettle();
    expect(find.text('Commit changes'), findsOneWidget);
    expect(find.text('Return 429\n\nExplain the change.'), findsOneWidget);
    expect(
      state.notices.value.first.text,
      contains('nothing was committed. eslint: 2 errors'),
    );
    state.clearNotices();
  });

  testWidgets('a running checkout refuses commit with the revert wording', (
    tester,
  ) async {
    await show(tester);
    backend.commitPlanned = BridgeCommitPlan(
      files: backend.commitPlanned.files,
      running: const [
        BridgeRunningAgent(own: false, title: 'Fix flaky sync test'),
      ],
    );
    await open(tester);
    expect(
      find.text(
        '“Fix flaky sync test” is running in this checkout. Stop it or wait until it finishes, then commit.',
      ),
      findsOneWidget,
    );
    expect(
      tester
          .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
          .onPressed,
      isNull,
    );
  });

  testWidgets(
    'header handles clean, detached HEAD, merge conflicts and the scoped Git menu',
    (tester) async {
      await show(tester, status: gitStatusOf());
      expect(
        find.byTooltip('Branch is up to date. Nothing to commit.'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('git-menu')));
      await tester.pumpAndSettle();
      expect(find.text('main → origin/main'), findsOneWidget);
      expect(find.text('Commit…'), findsOneWidget);
      expect(find.text('Create branch…'), findsOneWidget);
      for (final item in ['Push', 'Pull', 'Fetch']) {
        expect(find.text(item), findsNothing);
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      backend.git = gitStatusOf(branch: null, upstream: null);
      await state.git.refresh();
      await tester.pumpAndSettle();
      expect(find.text('Create branch…'), findsOneWidget);
      await open(tester);
      expect(find.byKey(const Key('branch-search')), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      backend.git = gitStatusOf(
        merging: true,
        conflicts: ['src/routes/login.ts', 'docs/api.md'],
      );
      await state.git.refresh();
      await tester.pumpAndSettle();
      expect(find.text('merging'), findsOneWidget);
      expect(
        find.byTooltip(
          '2 files have conflicts: src/routes/login.ts, docs/api.md. Resolve them in an editor or a terminal; Liquid resolves no conflicts.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets('Initialize Git is explicit and reports when checkpoints begin', (
    tester,
  ) async {
    await show(tester);
    backend.git = null;
    backend.inRepository = false;
    await state.git.refresh();
    await tester.pumpAndSettle();
    expect(backend.calls, isNot(contains('initialize git')));
    expect(find.text('Initialize Git'), findsOneWidget);
    await tester.tap(find.text('Initialize Git'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('initialize git'));
    expect(
      state.notices.value.first.text,
      'Initialized Git. Checkpoints start with the next turn.',
    );
    await tester.pump(const Duration(seconds: 7));
  });
}
