import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;
  const model = 'anthropic/claude-opus-5-5';

  /// The header's Git action on a pushed branch, with `changed` uncommitted files and `ahead`
  /// commits to push.
  Future<void> show(
    WidgetTester tester, {
    int changed = 2,
    int ahead = 0,
  }) async {
    backend = FakeBackend()
      ..git = gitStatusOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
        changed: changed,
        ahead: ahead,
      )
      ..commitPlanned = BridgeCommitPlan(
        snapshot: 'snapshot',
        files: [
          if (changed > 0) commitFileOf('src/login.ts', added: 4, removed: 1),
          if (changed > 1) commitFileOf('docs/api.md', added: 3),
        ],
        running: const [],
      );
    state = SessionState(backend);
    state.model.value = model;
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

  Future<void> openCompose(WidgetTester tester) async {
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Commit, push & create PR…'));
    await tester.pumpAndSettle();
  }

  Finder button(String label) => find.widgetWithText(InkWell, label);
  String text(WidgetTester tester, String key) =>
      tester.widget<TextField>(find.byKey(Key(key))).controller!.text;

  testWidgets('commits, pushes and creates the reviewed pull request', (
    tester,
  ) async {
    await show(tester);
    await openCompose(tester);

    expect(backend.calls, contains('prepare - -'));
    expect(
      backend.calls,
      contains('write draft $model main src/login.ts,docs/api.md'),
    );
    expect(find.text('Commit, push & create PR'), findsWidgets);
    expect(find.text('acme/api'), findsOneWidget);
    expect(
      text(tester, 'pull-request-title'),
      'Return 429 when logins are blocked',
    );
    expect(
      find.text(
        "Written by claude-opus-5-5 from the branch's commits and the files above. Edit before creating.",
      ),
      findsOneWidget,
    );
    // Nothing runs before the commit has a message.
    expect(
      tester.widget<InkWell>(button('Commit, push & create PR')).onTap,
      isNull,
    );

    await tester.enterText(
      find.byKey(const Key('commit-message')),
      'Document the Retry-After header',
    );
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(button('Commit, push & create PR'));
    await tester.pumpAndSettle();

    expect(backend.committedPaths, ['src/login.ts', 'docs/api.md']);
    expect(backend.committedMessage, 'Document the Retry-After header');
    expect(
      backend.calls,
      containsAllInOrder([
        'push false',
        'plan acme/api main',
        'create pull request',
      ]),
    );
    expect(backend.lastDraft!.title, 'Return 429 when logins are blocked');
    expect(find.text('Commit, push & create PR'), findsNothing);
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('a refused push names what was done and continues from there', (
    tester,
  ) async {
    await show(tester);
    await openCompose(tester);
    await tester.enterText(find.byKey(const Key('commit-message')), 'Document');
    await tester.pump(const Duration(milliseconds: 500));
    backend.transferOutcome = BridgeGitOutcome(
      kind: BridgeGitOutcomeKind.diverged,
      files: const [],
      running: const [],
      detail: '',
      transfer: transferOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
        ahead: 1,
        behind: 1,
      ),
    );

    await tester.tap(button('Commit, push & create PR'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        '✓ Committed 1a2b3c4 · ✕ Not pushed: origin rejected the push · Pull request not created. Remaining: Push, then Create pull request…',
      ),
      findsOneWidget,
    );
    expect(backend.calls, isNot(contains('create pull request')));

    backend
      ..transferOutcome = null
      ..committedMessage = null;
    await tester.tap(button('Push & create PR'));
    await tester.pumpAndSettle();

    expect(backend.committedMessage, isNull);
    expect(backend.calls.where((call) => call == 'push false'), hasLength(2));
    expect(backend.calls, contains('create pull request'));
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('with nothing to commit it pushes and creates', (tester) async {
    await show(tester, changed: 0, ahead: 2);
    await openCompose(tester);

    expect(find.text('Push & create PR'), findsWidgets);
    expect(find.text('2 commits'), findsOneWidget);
    expect(find.text('nothing to commit'), findsOneWidget);
    expect(find.byKey(const Key('commit-message')), findsNothing);
    expect(backend.calls, contains('write draft $model main '));
    expect(
      find.text(
        "Written by claude-opus-5-5 from the branch's 2 commits. Edit before creating.",
      ),
      findsOneWidget,
    );

    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(button('Push & create PR'));
    await tester.pumpAndSettle();

    expect(backend.committedPaths, isNull);
    expect(
      backend.calls,
      containsAllInOrder(['push false', 'create pull request']),
    );
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('a remote that is not on GitHub is refused before any write', (
    tester,
  ) async {
    await show(tester);
    backend.prepared = pullRequestOutcomeOf(
      kind: BridgePullRequestOutcomeKind.notGitHub,
      remote: 'origin',
      host: 'gitlab.com',
    );
    await openCompose(tester);

    expect(find.textContaining('gitlab.com'), findsWidgets);
    expect(find.byKey(const Key('commit-message')), findsNothing);
    expect(backend.calls, isNot(contains('push false')));
    expect(backend.committedPaths, isNull);
  });

  testWidgets('a changed file selection is written for again when done', (
    tester,
  ) async {
    await show(tester);
    await openCompose(tester);

    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('commit-select-docs/api.md')));
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.startsWith('write draft')),
      hasLength(1),
    );
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();

    expect(backend.calls, contains('write draft $model main src/login.ts'));
  });

  testWidgets('is offered only with something to commit or push', (
    tester,
  ) async {
    await show(tester, changed: 0);
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();

    final item = tester.widget<PopupMenuItem<Object>>(
      find.ancestor(
        of: find.text('Commit, push & create PR…'),
        matching: find.byWidgetPredicate((widget) => widget is PopupMenuItem),
      ),
    );
    expect(item.enabled, isFalse);
  });

  testWidgets('a branch that needs a push hands off to the combined dialog', (
    tester,
  ) async {
    await show(tester, ahead: 1);
    backend.planned = pullRequestOutcomeOf(
      kind: BridgePullRequestOutcomeKind.needsPush,
      branch: 'feature/retry-after',
      remote: 'origin',
      ahead: 1,
    );
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Create pull request…'));
    await tester.pumpAndSettle();
    expect(find.text('Push before creating a pull request'), findsOneWidget);

    await tester.tap(button('Commit, push & create PR…'));
    await tester.pumpAndSettle();

    expect(backend.calls, contains('prepare - -'));
    expect(find.byKey(const Key('commit-message')), findsOneWidget);
  });
}
