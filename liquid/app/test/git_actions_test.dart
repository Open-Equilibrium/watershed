import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> show(WidgetTester tester, BridgeGitStatus status) async {
    backend = FakeBackend()..git = status;
    state = SessionState(backend);
    await state.git.refresh();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: GitActions(state: state, device: 'MacBook'),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> menu(WidgetTester tester) async {
    await tester.pump();
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();
  }

  testWidgets('the next step follows every approved checkout state', (
    tester,
  ) async {
    final cases = [
      (
        gitStatusOf(changed: 4, ahead: 1),
        'Commit',
        '4',
        true,
        'Commit 4 changed files on main.',
      ),
      (
        gitStatusOf(ahead: 1),
        'Push',
        '↑1',
        true,
        'Push 1 commit to origin/main.',
      ),
      (
        gitStatusOf(branch: 'feature/retry-after', upstream: null),
        'Push',
        '',
        true,
        'Push feature/retry-after to origin and track it.',
      ),
      (
        gitStatusOf(behind: 3),
        'Pull',
        '↓3',
        true,
        'Pull 3 commits from origin/main.',
      ),
      (
        gitStatusOf(ahead: 1, behind: 3),
        'Push',
        '',
        false,
        'main and origin/main have diverged. Merge or rebase in a terminal, then push.',
      ),
      (
        gitStatusOf(commit: null, upstream: null),
        'Commit',
        '',
        false,
        'Branch is up to date. Nothing to commit.',
      ),
      (
        gitStatusOf(),
        'Commit',
        '',
        false,
        'Branch is up to date. Nothing to commit.',
      ),
      (
        gitStatusOf(branch: null, upstream: null),
        'Create branch…',
        '',
        true,
        'Detached HEAD: create a branch before committing or pushing.',
      ),
      (
        gitStatusOf(upstream: null, remote: null, changed: 4),
        'Commit',
        '4',
        true,
        'No remote: commits stay in this repository. Push needs a remote added in Git.',
      ),
    ];
    await show(tester, cases.first.$1);
    for (final (status, label, count, enabled, hint) in cases) {
      backend.git = status;
      await state.git.refresh();
      await tester.pumpAndSettle();
      final next = find.byKey(const Key('git-next'));
      expect(
        find.descendant(of: next, matching: find.text(label)),
        findsOneWidget,
      );
      if (count.isNotEmpty) {
        expect(
          find.descendant(of: next, matching: find.text(count)),
          findsOneWidget,
        );
      }
      expect(tester.widget<TextButton>(next).onPressed != null, enabled);
      expect(find.byTooltip(hint), findsOneWidget);
    }
    await state.dispose();
  });

  testWidgets(
    'the Git menu keeps every action visible with counts and hover reasons',
    (tester) async {
      await show(tester, gitStatusOf(changed: 4, ahead: 1));
      await menu(tester);
      for (final label in [
        'main → origin/main',
        'Commit…',
        '4 files',
        'Push',
        '↑1',
        'Pull',
        'up to date',
        'Fetch',
        'origin',
        'Create branch…',
      ]) {
        expect(find.text(label), findsOneWidget);
      }
      final pull = tester.widget<PopupMenuItem<Object>>(
        find.ancestor(
          of: find.text('Pull'),
          matching: find.byWidgetPredicate((widget) => widget is PopupMenuItem),
        ),
      );
      expect(pull.enabled, isFalse);
      expect(
        find.byTooltip('Branch is up to date. Nothing to pull.'),
        findsOneWidget,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      backend.git = gitStatusOf(upstream: null, remote: null);
      await state.git.refresh();
      await menu(tester);
      for (final action in ['Push', 'Pull', 'Fetch']) {
        final item = find.ancestor(
          of: find.text(action),
          matching: find.byWidgetPredicate((widget) => widget is PopupMenuItem),
        );
        expect(tester.widget<PopupMenuItem<Object>>(item).enabled, isFalse);
      }
      expect(find.text('up to date'), findsNothing);
      await state.dispose();
    },
  );

  testWidgets(
    'default push asks once, supports Cancel, Enter and Create branch',
    (tester) async {
      await show(tester, gitStatusOf(ahead: 1, defaultBranch: 'main'));
      Future<void> open() async {
        await tester.tap(find.byKey(const Key('git-next')));
        await tester.pumpAndSettle();
        expect(find.text('Push to main?'), findsOneWidget);
        expect(
          find.text(
            "This pushes 1 commit directly to origin/main, the repository's default branch. You can push it as it is, or create a branch first and push that.",
          ),
          findsOneWidget,
        );
      }

      await open();
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(backend.calls, isNot(contains('push true')));
      await open();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call == 'push true'), hasLength(1));
      await state.dispose();
      await open();
      await tester.tap(find.text('Create branch…'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('branch-search')), findsOneWidget);
      expect(backend.calls.where((call) => call == 'push true'), hasLength(1));
    },
  );

  testWidgets('a merge hides the Commit badge and explains the blocker', (
    tester,
  ) async {
    await show(tester, gitStatusOf(merging: true, changed: 2));
    final next = find.byKey(const Key('git-next'));
    expect(tester.widget<TextButton>(next).onPressed, isNull);
    expect(find.descendant(of: next, matching: find.text('2')), findsNothing);
    await state.dispose();
  });

  testWidgets('push shows its remote and elapsed seconds until Git completes', (
    tester,
  ) async {
    await show(tester, gitStatusOf(ahead: 1));
    backend.transferGate = Completer<BridgeGitOutcome>();
    await tester.tap(find.byKey(const Key('git-next')));
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    expect(find.text('Pushing to origin… 3 s'), findsOneWidget);
    expect(
      tester.widget<TextButton>(find.byKey(const Key('git-next'))).onPressed,
      isNull,
    );
    backend.transferGate!.complete(
      BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.done,
        files: const [],
        running: const [],
        detail: '',
        transfer: transferOf(ahead: 1),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.textContaining('Pushing to'), findsNothing);
    await state.dispose();
  });

  testWidgets(
    'default push confirmation uses the destination and counts from Git',
    (tester) async {
      await show(tester, gitStatusOf(upstream: null));
      backend.transferOutcome = BridgeGitOutcome(
        kind: BridgeGitOutcomeKind.pushDefault,
        files: const [],
        running: const [],
        detail: 'main',
        transfer: transferOf(ahead: 1, firstPush: true),
      );
      await tester.tap(find.byKey(const Key('git-next')));
      await tester.pumpAndSettle();
      expect(find.text('Push to main?'), findsOneWidget);
      expect(
        find.textContaining('This pushes 1 commit directly to origin/main,'),
        findsOneWidget,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(backend.calls, isNot(contains('push true')));
      await state.dispose();
    },
  );

  testWidgets('Fetch runs immediately and the menu keeps fetched time', (
    tester,
  ) async {
    await show(tester, gitStatusOf(upstream: null));
    await menu(tester);
    await tester.tap(find.text('Fetch'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('fetch now'));
    await menu(tester);
    expect(find.textContaining(' · fetched '), findsOneWidget);
    expect(backend.calls.where((call) => call == 'fetch now'), hasLength(1));
    await state.dispose();
  });
}
