import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/agent/pull_request.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/branch_bases.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;

  /// The header's PR chip and Git action, with the post-push offer below them.
  Future<void> show(
    WidgetTester tester, {
    BridgeGitStatus? status,
    BranchBases? bases,
  }) async {
    backend = FakeBackend()
      ..git = status ?? gitStatusOf(branch: 'feature/retry-after');
    state = SessionState(backend, bases: bases);
    await state.git.refresh();
    await state.pulls.refresh();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: Column(
            children: [
              Row(
                children: [
                  PullRequestChip(state: state, device: 'MacBook'),
                  GitActions(state: state, device: 'MacBook'),
                ],
              ),
              const Spacer(),
              PushedOffer(state: state, device: 'MacBook'),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> openDialog(WidgetTester tester) async {
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();
    // The menu's entry, above a post-push offer.
    await tester.tap(find.text('Create pull request…').last);
    await tester.pumpAndSettle();
  }

  Finder button(String label) => find.widgetWithText(InkWell, label);

  testWidgets(
    'the dialog prefills the latest commit and creates only what was reviewed',
    (tester) async {
      final bases = BranchBases();
      await bases.record('/work/api/.git', 'feature/retry-after', 'main');
      await show(tester, bases: bases);
      await openDialog(tester);

      expect(backend.calls, contains('plan - main'));
      expect(find.text('acme/api'), findsOneWidget);
      expect(find.text('feature/retry-after'), findsOneWidget);
      expect(find.text('created from main'), findsOneWidget);
      expect(
        find.text('2 pushed commits · 3 changed files · +12 −1'),
        findsOneWidget,
      );
      expect(
        find.text('Creates the pull request on GitHub through gh on MacBook.'),
        findsOneWidget,
      );
      final title = find.byKey(const Key('pull-request-title'));
      expect(
        tester.widget<TextField>(title).controller!.text,
        'Return 429 with Retry-After on /login',
      );
      expect(button('Create pull request'), findsOneWidget);

      await tester.enterText(title, '');
      await tester.pump();
      expect(
        tester.widget<InkWell>(button('Create pull request')).onTap,
        isNull,
      );
      await tester.enterText(title, 'Return 429 on /login');
      await tester.tap(find.text('Create as draft'));
      await tester.pump();
      await tester.tap(button('Create draft pull request'));
      await tester.pumpAndSettle();

      final draft = backend.lastDraft!;
      expect(draft.title, 'Return 429 on /login');
      expect(draft.body, 'Blocked login attempts now get 429.');
      expect(
        (draft.target, draft.base, draft.draft),
        ('acme/api', 'main', true),
      );
      expect(draft.identity.headCommit, 'c0ffee');
      expect(find.text('Create pull request'), findsNothing);
      expect(state.pulls.current.value?.number, 42);
      expect(find.text('Open #42'), findsOneWidget);
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets(
    'a fork preselects its parent, and an open pull request is shown instead',
    (tester) async {
      await show(tester);
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          targets: const ['acme/api', 'bw/api'],
          head: 'bw:feature/retry-after',
        ),
      );
      await openDialog(tester);
      expect(find.text('parent of bw/api'), findsOneWidget);
      expect(find.text('bw:feature/retry-after'), findsOneWidget);
      expect(
        find.textContaining('Pushes still go only to origin'),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      backend.planned = pullRequestOutcomeOf(
        plan: planOf(open: pullRequestOf()),
      );
      await openDialog(tester);
      expect(find.text('Pull request #42 already exists'), findsOneWidget);
      expect(
        find.textContaining('is open from feature/retry-after to main'),
        findsOneWidget,
      );
      expect(find.text('Open in browser ↗'), findsOneWidget);
      expect(button('Create pull request'), findsNothing);
    },
  );

  testWidgets(
    'unpushed commits hand off to Push and gh problems say what to do',
    (tester) async {
      await show(
        tester,
        status: gitStatusOf(branch: 'feature/retry-after', ahead: 2),
      );
      backend.planned = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.needsPush,
        branch: 'feature/retry-after',
        remote: 'origin',
        ahead: 2,
      );
      await openDialog(tester);
      expect(find.text('Push before creating a pull request'), findsOneWidget);
      expect(
        find.text(
          'feature/retry-after has 2 commits that are not on origin. GitHub cannot include them yet.',
        ),
        findsOneWidget,
      );
      await tester.tap(button('Push…'));
      await tester.pumpAndSettle();
      expect(backend.calls, contains('push false'));

      backend.planned = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.signIn,
        host: 'github.com',
      );
      await openDialog(tester);
      expect(
        find.text(
          'Sign in on MacBook with gh auth login, then check again. Liquid does not ask for the credential.',
        ),
        findsOneWidget,
      );
      backend.planned = pullRequestOutcomeOf(plan: planOf());
      await tester.tap(button('Check again'));
      await tester.pumpAndSettle();
      expect(button('Create pull request'), findsOneWidget);
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets(
    'a refused or changed creation keeps the text and says what happened',
    (tester) async {
      await show(tester);
      await openDialog(tester);
      final title = find.byKey(const Key('pull-request-title'));
      await tester.enterText(title, 'Kept title');

      backend.created = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.notCreated,
        detail: 'Validation Failed (HTTP 422)',
      );
      await tester.tap(button('Create pull request'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining(
          'GitHub refused the pull request; none was created.',
        ),
        findsOneWidget,
      );
      expect(tester.widget<TextField>(title).controller!.text, 'Kept title');

      backend.created = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.changed,
        plan: planOf(commit: 'beef'),
      );
      await tester.tap(button('Create pull request'));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'feature/retry-after changed since this dialog opened. Review the updated commits before creating the pull request.',
        ),
        findsOneWidget,
      );
      expect(tester.widget<TextField>(title).controller!.text, 'Kept title');

      backend.created = pullRequestOutcomeOf(pullRequest: pullRequestOf());
      final gate = backend.createGate = Completer();
      await tester.tap(button('Create pull request'));
      await tester.pump();
      expect(find.textContaining('Creating pull request…'), findsOneWidget);
      gate.complete(backend.created);
      await tester.pumpAndSettle();
      expect(backend.lastDraft!.identity.headCommit, 'beef');
      await tester.pump(const Duration(seconds: 7));
    },
  );

  testWidgets('without a known base the user chooses one from GitHub', (
    tester,
  ) async {
    await show(tester);
    backend.planned = pullRequestOutcomeOf(
      plan: planOf(base: null, defaultBranch: null, comparison: null),
    );
    backend.baseBranches = [nameOf('main'), nameOf('release/1.2')];
    await openDialog(tester);
    expect(find.text('Choose a branch…'), findsOneWidget);
    expect(
      find.textContaining('Choose a base and enter a title'),
      findsOneWidget,
    );
    expect(tester.widget<InkWell>(button('Create pull request')).onTap, isNull);

    backend.planned = pullRequestOutcomeOf(plan: planOf(base: 'release/1.2'));
    await tester.tap(find.text('Choose a branch…'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('bases acme/api'));
    await tester.tap(find.text('release/1.2').last);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('plan acme/api release/1.2'));
    expect(
      tester.widget<InkWell>(button('Create pull request')).onTap,
      isNotNull,
    );
  });

  testWidgets(
    'the PR chip summarizes checks and review and refreshes when opened',
    (tester) async {
      await show(tester);
      backend.current = pullRequestOutcomeOf(
        pullRequest: pullRequestOf(
          passed: 2,
          failed: 1,
          review: BridgeReviewDecision.changesRequested,
        ),
      );
      await state.pulls.refresh();
      await tester.pumpAndSettle();
      final reads = backend.pullRequestReads;

      await tester.tap(find.text('Open #42'));
      await tester.pumpAndSettle();
      expect(backend.pullRequestReads, reads + 1);
      expect(
        find.text('Return 429 with Retry-After on /login'),
        findsOneWidget,
      );
      expect(find.text('feature/retry-after → main'), findsOneWidget);
      expect(
        find.text('Checks: 1 failed, 2 passed · Review: changes requested'),
        findsOneWidget,
      );
      expect(find.text('Open in browser ↗'), findsOneWidget);
      expect(find.textContaining('Updated from GitHub at '), findsOneWidget);

      backend.current = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.unreachable,
        host: 'github.com',
      );
      await tester.tap(find.text('Refresh'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Status unavailable'), findsOneWidget);
      expect(find.textContaining('Last known: Open #42 · '), findsOneWidget);
    },
  );

  testWidgets('a push offers a pull request above the input area', (
    tester,
  ) async {
    await show(
      tester,
      status: gitStatusOf(
        branch: 'feature/retry-after',
        upstream: 'origin/feature/retry-after',
        ahead: 2,
      ),
    );
    await tester.tap(find.byKey(const Key('git-next')));
    await tester.pumpAndSettle();
    expect(
      find.text('Pushed 2 commits to origin/feature/retry-after'),
      findsOneWidget,
    );
    await tester.tap(find.text('Create pull request…'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'plan - -');
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Dismiss'));
    await tester.pumpAndSettle();
    expect(find.textContaining('Pushed 2 commits'), findsNothing);
    await tester.pump(const Duration(seconds: 7));
  });
}
