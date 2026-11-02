import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/commit_dialog.dart';
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
  Future<void> show(
    WidgetTester tester, {
    BridgeGitStatus? status,
    FakeBackend? using,
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    backend = (using ?? FakeBackend())
      ..git = status ?? gitStatusOf(changed: 4, defaultBranch: 'main');
    backend.commitPlanned = BridgeCommitPlan(
      snapshot: 'snapshot',
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
      expect(backend.committedSnapshot, 'snapshot');
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

  testWidgets(
    'Enter in the message field keeps a multiline draft without committing',
    (tester) async {
      await show(tester);
      await open(tester);
      await tester.enterText(
        find.byKey(const Key('commit-message')),
        'Subject',
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(backend.committedPaths, isNull);
      expect(find.byKey(const Key('commit-message')), findsOneWidget);
      expect(
        tester.testTextInput.setClientArgs!['inputAction'],
        'TextInputAction.newline',
      );
      await tester.testTextInput.receiveAction(TextInputAction.newline);
      await tester.enterText(
        find.byKey(const Key('commit-message')),
        'Subject\nDetails',
      );
      expect(
        tester
            .widget<TextField>(find.byKey(const Key('commit-message')))
            .controller!
            .text,
        'Subject\nDetails',
      );
      expect(backend.committedPaths, isNull);
    },
  );

  testWidgets(
    'running refusal clears when Git changes while the dialog is open',
    (tester) async {
      await show(tester);
      backend.commitPlanned = BridgeCommitPlan(
        snapshot: 'snapshot',
        files: backend.commitPlanned.files,
        running: const [BridgeRunningAgent(own: false, title: 'Other session')],
      );
      await open(tester);
      await message(tester);
      expect(find.text('Commit on new branch…'), findsNothing);
      backend.commitPlanned = BridgeCommitPlan(
        snapshot: 'snapshot',
        files: backend.commitPlanned.files,
        running: const [],
      );
      backend.git = gitStatusOf(changed: 3);
      await state.git.refresh();
      await tester.pumpAndSettle();
      expect(find.textContaining('Other session'), findsNothing);
      expect(
        tester
            .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
            .onPressed,
        isNotNull,
      );
      expect(find.text('Return 429\n\nExplain the change.'), findsOneWidget);
    },
  );

  testWidgets(
    'turn end refreshes an open commit plan without a status change',
    (tester) async {
      await show(tester);
      await open(tester);
      await message(tester);
      state.status.value = RunStatus.running;
      await tester.pump();
      expect(find.text('Commit on new branch…'), findsNothing);
      final reads = backend.calls.where((call) => call == 'commit plan').length;
      state.status.value = RunStatus.idle;
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call == 'commit plan'),
        hasLength(reads + 1),
      );
      expect(
        tester
            .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
            .onPressed,
        isNotNull,
      );
      expect(find.text('Return 429\n\nExplain the change.'), findsOneWidget);
    },
  );

  testWidgets('detached dialog permits a new branch but disables Commit', (
    tester,
  ) async {
    await show(tester);
    await open(tester);
    await message(tester);
    backend.git = gitStatusOf(branch: null, upstream: null, changed: 4);
    await state.git.refresh();
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Commit'))
          .onPressed,
      isNull,
    );
    expect(find.text('Commit on new branch…'), findsOneWidget);
    expect(
      find.textContaining('Detached HEAD: create a branch'),
      findsOneWidget,
    );
  });

  testWidgets(
    'one pending plan opens one dialog and drops a switched session',
    (tester) async {
      final pending = FakeBackend()..planGate = Completer<BridgeCommitPlan>();
      await show(tester, using: pending);
      final context = tester.element(find.byType(AgentShell));
      final first = showCommitDialog(context, state);
      final second = showCommitDialog(context, state);
      await tester.pump();
      expect(
        backend.calls.where((call) => call == 'commit plan'),
        hasLength(1),
      );
      await state.newSession();
      pending.planGate!.complete(pending.commitPlanned);
      await tester.pumpAndSettle();
      expect(find.text('Commit changes'), findsNothing);
      await Future.wait([first, second]);
    },
  );

  testWidgets('two open requests share a plan and show a single dialog', (
    tester,
  ) async {
    final pending = FakeBackend()..planGate = Completer<BridgeCommitPlan>();
    await show(tester, using: pending);
    final context = tester.element(find.byType(AgentShell));
    final first = showCommitDialog(context, state);
    final second = showCommitDialog(context, state);
    await tester.pump();
    pending.planGate!.complete(pending.commitPlanned);
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call == 'commit plan'), hasLength(1));
    expect(find.text('Commit changes'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await Future.wait([first, second]);
  });

  testWidgets(
    'Initialize waits for loaded status and disables repeated writes',
    (tester) async {
      final pending = FakeBackend()..statusGate = Completer<BridgeGitStatus?>();
      await show(tester, using: pending);
      expect(find.text('Initialize Git'), findsNothing);
      pending.statusGate!.complete(null);
      pending.statusGate = null;
      pending.git = null;
      await tester.pumpAndSettle();
      expect(find.text('Initialize Git'), findsOneWidget);
      pending.initializeGate = Completer<BridgeGitOutcome>();
      final first = state.git.initialize();
      final second = state.git.initialize();
      await tester.pump();
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Initialize Git'),
            )
            .onPressed,
        isNull,
      );
      expect(
        backend.calls.where((call) => call == 'initialize git'),
        hasLength(1),
      );
      pending.initializeGate!.complete(outcomeOf(BridgeGitOutcomeKind.done));
      await Future.wait([first, second]);
      state.clearNotices();
    },
  );

  testWidgets('Edit excludes files and shows the mixed checkbox', (
    tester,
  ) async {
    await show(tester);
    await open(tester);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    expect(find.bySemanticsLabel('All files'), findsOneWidget);
    expect(
      tester
          .getSemantics(find.byKey(const Key('commit-select-notes/todo.md')))
          .label,
      startsWith('notes/todo.md\n'),
    );
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
      snapshot: 'snapshot',
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
      backend.git = gitStatusOf(ahead: 1);
      await state.git.refresh();
      await tester.pumpAndSettle();
      expect(find.byTooltip('Nothing to commit.'), findsOneWidget);
      backend.git = gitStatusOf(changed: 1);
      await state.git.refresh();
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('git-menu')));
      await tester.pumpAndSettle();
      expect(find.text('1 file'), findsOneWidget);
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
      expect(
        tester.widget<Text>(find.text('merging')).style!.color,
        tester.element(find.text('merging')).ws.warning,
      );
      expect(find.text('merging'), findsOneWidget);
      expect(
        find.byTooltip(
          '2 files have conflicts: src/routes/login.ts, docs/api.md. Resolve them in an editor or a terminal; Liquid resolves no conflicts.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets('merge hints distinguish one conflicted file from no conflicts', (
    tester,
  ) async {
    await show(
      tester,
      status: gitStatusOf(merging: true, conflicts: ['a.txt']),
    );
    expect(
      find.byTooltip(
        '1 file has conflicts: a.txt. Resolve them in an editor or a terminal; Liquid resolves no conflicts.',
      ),
      findsOneWidget,
    );
    backend.git = gitStatusOf(merging: true);
    await state.git.refresh();
    await tester.pumpAndSettle();
    expect(
      find.byTooltip(
        'A merge is in progress. Finish it in a terminal; Liquid commits no merges.',
      ),
      findsOneWidget,
    );
  });

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
