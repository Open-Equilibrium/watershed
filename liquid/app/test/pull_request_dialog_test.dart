import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/agent/pull_request_status.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/branch_bases.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';

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
        kind: BridgePullRequestOutcomeKind.signIn,
        host: 'github.com',
      );
      await tester.tap(find.text('Refresh'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Status unavailable'), findsOneWidget);
      expect(find.textContaining('Last known: Open #42 · '), findsOneWidget);
      expect(find.textContaining('Sign in on MacBook'), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(
        tester.getSemantics(find.byKey(const Key('pull-request-chip'))),
        matchesSemantics(
          label: 'Pull request Open #42, status unavailable',
          isButton: true,
          hasTapAction: true,
        ),
      );
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
    await tester.tap(button('Create pull request…'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'plan - -');
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Dismiss'));
    await tester.pumpAndSettle();
    expect(find.textContaining('Pushed 2 commits'), findsNothing);
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('a confirmed push to the default branch offers no pull request', (
    tester,
  ) async {
    await show(tester, status: gitStatusOf(ahead: 1, defaultBranch: 'main'));
    await tester.tap(find.byKey(const Key('git-next')));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('push true'));
    expect(find.textContaining('Pushed 1 commit'), findsNothing);
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets(
    'a failed re-read keeps the plan, says why and holds Create until one succeeds',
    (tester) async {
      await show(tester);
      await openDialog(tester);
      Finder create() => button('Create pull request');
      expect(tester.widget<InkWell>(create()).onTap, isNotNull);

      backend.baseBranches = [nameOf('main'), nameOf('develop')];
      backend.planned = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.signIn,
        host: 'github.com',
      );
      await tester.tap(find.text('main').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('develop'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Sign in on MacBook'), findsOneWidget);
      expect(
        find.text('2 pushed commits · 3 changed files · +12 −1'),
        findsOneWidget,
      );
      expect(tester.widget<InkWell>(create()).onTap, isNull);

      // A read the session did not answer keeps the plan and offers another.
      final gate = backend.pullRequestPlanGate = Completer();
      await tester.tap(button('Check again'));
      await tester.pump();
      gate.completeError(StateError('gone'));
      await tester.pumpAndSettle();
      expect(find.text('Could not read GitHub on MacBook.'), findsOneWidget);
      expect(tester.widget<InkWell>(create()).onTap, isNull);

      backend.pullRequestPlanGate = null;
      backend.planned = pullRequestOutcomeOf(plan: planOf());
      await tester.tap(button('Check again'));
      await tester.pumpAndSettle();
      expect(backend.calls.last, 'plan acme/api main');
      expect(find.textContaining('Could not read GitHub'), findsNothing);
      expect(tester.widget<InkWell>(create()).onTap, isNotNull);
    },
  );

  testWidgets('a first read that fails offers to check again', (tester) async {
    await show(tester);
    final gate = backend.pullRequestPlanGate = Completer();
    await tester.tap(find.byKey(const Key('git-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Create pull request…').last);
    await tester.pump();
    gate.completeError(StateError('gone'));
    await tester.pumpAndSettle();
    expect(find.text('Could not read GitHub on MacBook.'), findsOneWidget);
    backend.pullRequestPlanGate = null;
    await tester.tap(button('Check again'));
    await tester.pumpAndSettle();
    expect(button('Create pull request'), findsOneWidget);
  });

  testWidgets(
    'Create waits for a comparison, and the head shows its hidden characters',
    (tester) async {
      await show(tester);
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          comparison: null,
          comparisonFailed: true,
          head: 'feature/<U+202E>x',
          headBranch: 'feature/\u202ex',
        ),
      );
      await openDialog(tester);
      expect(
        find.text(
          '⚠ GitHub could not compare feature/<U+202E>x with main. Check again, or choose another base.',
        ),
        findsOneWidget,
      );
      expect(
        tester.widget<InkWell>(button('Create pull request')).onTap,
        isNull,
      );
      expect(button('Check again'), findsOneWidget);

      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          comparison: const BridgeComparison(
            commits: 0,
            files: 0,
            added: 0,
            removed: 0,
            url: '',
            mergeBase: '',
          ),
          head: 'feature/<U+202E>x',
          headBranch: 'feature/\u202ex',
        ),
      );
      await tester.tap(button('Check again'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('feature/<U+202E>x has no commits beyond main'),
        findsOneWidget,
      );
      expect(find.textContaining('\u202e'), findsNothing);
    },
  );

  testWidgets(
    'the base list filters, Enter picks the first match, and a long list says so',
    (tester) async {
      await show(tester);
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(base: null, defaultBranch: null, comparison: null),
      );
      await openDialog(tester);
      backend.baseBranches = [nameOf('main'), nameOf('release/1.2')];
      backend.moreBases = true;
      await tester.tap(find.text('Choose a branch…'));
      await tester.pumpAndSettle();
      expect(find.text('Filter branches'), findsOneWidget);
      expect(
        find.textContaining(
          "Liquid has no record of where this branch started, and GitHub's default could not be read.",
        ),
        findsOneWidget,
      );
      await tester.enterText(find.byKey(const Key('base-filter')), 'rel');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(backend.calls.last, 'plan acme/api release/1.2');

      await tester.tap(find.text('Choose a branch…'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('base-filter')), 'hotfix/9');
      await tester.pump();
      expect(
        find.textContaining('No match among the first 2 branches.'),
        findsOneWidget,
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(backend.calls.last, 'plan acme/api hotfix/9');
    },
  );

  testWidgets(
    'a fork without a known parent chooses its target before a base',
    (tester) async {
      await show(tester);
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          targets: const ['bw/api'],
          target: null,
          base: null,
          defaultBranch: null,
          comparison: null,
          parentUnknown: true,
        ),
      );
      await openDialog(tester);
      expect(
        find.textContaining('only the fork itself can be targeted'),
        findsOneWidget,
      );
      await tester.tap(find.text('Choose a branch…'));
      await tester.pumpAndSettle();
      expect(backend.calls, isNot(contains('bases bw/api')));
      expect(
        tester.getSemantics(find.text('Choose a repository…')),
        matchesSemantics(
          label: 'Target: Choose a repository…',
          isButton: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );
    },
  );

  testWidgets('Enter creates unless the description has focus', (tester) async {
    await show(tester);
    await openDialog(tester);
    await tester.tap(find.byKey(const Key('pull-request-description')));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.lastDraft, isNull);
    // The title has focus when the dialog opens.
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await openDialog(tester);
    expect(
      FocusManager.instance.primaryFocus?.context
          ?.findAncestorWidgetOfExactType<TextField>()
          ?.key,
      const Key('pull-request-title'),
    );
    await tester.tap(find.text('Create as draft'));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.lastDraft?.draft, isTrue);
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets(
    'a remote on another host opens its page, and the menu orders Push, PR, Pull',
    (tester) async {
      await show(tester);
      backend.planned = pullRequestOutcomeOf(
        kind: BridgePullRequestOutcomeKind.notGitHub,
        remote: 'origin',
        host: 'gitlab.com',
        page: 'https://gitlab.com/bw/api',
      );
      await openDialog(tester);
      expect(
        find.textContaining('open this repository in its host\'s browser'),
        findsOneWidget,
      );
      expect(button('Open in browser ↗'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('git-menu')));
      await tester.pumpAndSettle();
      double top(String label) => tester.getTopLeft(find.text(label).last).dy;
      expect(top('Push'), lessThan(top('Create pull request…')));
      expect(top('Create pull request…'), lessThan(top('Pull')));
    },
  );

  group("the Chat's model", () {
    TextField field(WidgetTester tester, String key) =>
        tester.widget<TextField>(find.byKey(Key(key)));

    testWidgets('writes the title and description from the pushed commits', (
      tester,
    ) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      await openDialog(tester);

      expect(
        backend.calls,
        contains('write anthropic/claude-opus-5-5 ba5e..c0ffee'),
      );
      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 when logins are blocked',
      );
      expect(
        field(tester, 'pull-request-description').controller!.text,
        'Blocked logins now wait for the window.',
      );
      expect(
        find.text(
          'Written by claude-opus-5-5 from the 2 commits and their diff. Edit before creating.',
        ),
        findsOneWidget,
      );
    });

    testWidgets(
      '⌘Z after the model writes the title does not bring back the title it replaced',
      (tester) async {
        await show(tester);
        state.model.value = 'anthropic/claude-opus-5-5';
        backend.writeGate = Completer();
        await openDialog(tester);
        await tester.pump(const Duration(milliseconds: 600));
        backend.writeGate!.complete(backend.written);
        await tester.pumpAndSettle();
        await tester.pump(const Duration(milliseconds: 600));
        final title = find.byKey(const Key('pull-request-title'));
        await tester.tap(title);
        await undo(tester);
        expect(
          field(tester, 'pull-request-title').controller!.text,
          'Return 429 when logins are blocked',
        );
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('keeps what the user typed while it wrote', (tester) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.writeGate = Completer();
      await openDialog(tester);
      expect(
        find.text(
          '◌ claude-opus-5-5 is writing the title and description… The fields stay editable.',
        ),
        findsOneWidget,
      );
      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 with Retry-After on /login',
      );

      await tester.enterText(
        find.byKey(const Key('pull-request-title')),
        'My title',
      );
      backend.writeGate!.complete(backend.written);
      await tester.pumpAndSettle();

      expect(field(tester, 'pull-request-title').controller!.text, 'My title');
      expect(
        field(tester, 'pull-request-description').controller!.text,
        'Blocked logins now wait for the window.',
      );
    });

    testWidgets('that fails leaves the latest commit and says so', (
      tester,
    ) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.writeError = 'pi failed';
      await openDialog(tester);

      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 with Retry-After on /login',
      );
      expect(
        find.text(
          "Could not write the text with claude-opus-5-5. Filled from the latest commit's subject and body instead; edit before creating.",
        ),
        findsOneWidget,
      );
      expect(button('Create pull request'), findsOneWidget);
    });

    testWidgets('stops when the dialog closes', (tester) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.writeGate = Completer();
      await openDialog(tester);

      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      expect(backend.calls, contains('stop writing'));
    });

    testWidgets('is not asked without one', (tester) async {
      await show(tester);
      await openDialog(tester);

      expect(backend.calls.where((call) => call.startsWith('write')), isEmpty);
      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 with Retry-After on /login',
      );
    });

    testWidgets('writes again for a new base and drops the earlier answer', (
      tester,
    ) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.baseBranches = [nameOf('main'), nameOf('develop')];
      final first = backend.writeGate = Completer();
      await openDialog(tester);

      backend.writeGate = null;
      backend.written = const BridgePullRequestText(
        title: 'For develop',
        body: 'Three commits.',
        commits: 3,
      );
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          base: 'develop',
          comparison: const BridgeComparison(
            commits: 3,
            files: 4,
            added: 20,
            removed: 2,
            url: 'https://github.com/acme/api/compare/develop...feature/retry-after',
            mergeBase: 'de7e',
          ),
        ),
      );
      await tester.tap(find.text('main').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('develop').last);
      await tester.pumpAndSettle();
      first.complete(
        const BridgePullRequestText(
          title: 'For main',
          body: 'Two commits.',
          commits: 2,
        ),
      );
      await tester.pumpAndSettle();

      expect(backend.calls.where((call) => call.startsWith('write')), [
        'write anthropic/claude-opus-5-5 ba5e..c0ffee',
        'write anthropic/claude-opus-5-5 de7e..c0ffee',
      ]);
      expect(
        field(tester, 'pull-request-title').controller!.text,
        'For develop',
      );
      expect(
        find.text(
          'Written by claude-opus-5-5 from the 3 commits and their diff. Edit before creating.',
        ),
        findsOneWidget,
      );
    });

    testWidgets('puts the commit back when a rewrite for a new base fails', (
      tester,
    ) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.baseBranches = [nameOf('main'), nameOf('develop')];
      await openDialog(tester);
      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 when logins are blocked',
      );

      backend.writeError = 'pi failed';
      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          base: 'develop',
          comparison: const BridgeComparison(
            commits: 3,
            files: 4,
            added: 20,
            removed: 2,
            url: 'https://github.com/acme/api/compare/develop...feature/retry-after',
            mergeBase: 'de7e',
          ),
        ),
      );
      await tester.tap(find.text('main').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('develop').last);
      await tester.pumpAndSettle();

      expect(
        field(tester, 'pull-request-title').controller!.text,
        'Return 429 with Retry-After on /login',
      );
      expect(
        find.textContaining('Could not write the text with claude-opus-5-5.'),
        findsOneWidget,
      );
    });

    testWidgets('stops for a base without commits to describe', (tester) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.baseBranches = [nameOf('main'), nameOf('develop')];
      backend.writeGate = Completer();
      await openDialog(tester);

      backend.planned = pullRequestOutcomeOf(
        plan: planOf(
          base: 'develop',
          comparison: const BridgeComparison(
            commits: 0,
            files: 0,
            added: 0,
            removed: 0,
            url: '',
            mergeBase: 'de7e',
          ),
        ),
      );
      await tester.tap(find.text('main').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('develop').last);
      await tester.pumpAndSettle();

      expect(backend.calls, contains('stop writing'));
      expect(find.textContaining('is writing the title'), findsNothing);
    });

    testWidgets('stops when Create is pressed and sends the shown text', (
      tester,
    ) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.writeGate = Completer();
      await openDialog(tester);

      await tester.tap(button('Create pull request'));
      await tester.pumpAndSettle();

      expect(backend.calls, contains('stop writing'));
      expect(backend.lastDraft!.title, 'Return 429 with Retry-After on /login');
      // The notice of the created pull request.
      await tester.pump(const Duration(seconds: 7));
    });

    testWidgets('holds Create briefly after the text changes', (tester) async {
      await show(tester);
      state.model.value = 'anthropic/claude-opus-5-5';
      backend.writeGate = Completer();
      await openDialog(tester);

      backend.writeGate!.complete(backend.written);
      await tester.pump();
      await tester.tap(button('Create pull request'));
      await tester.pump();
      expect(backend.calls, isNot(contains('create pull request')));

      await tester.pump(const Duration(milliseconds: 500));
      await tester.tap(button('Create pull request'));
      await tester.pumpAndSettle();
      expect(backend.lastDraft!.title, 'Return 429 when logins are blocked');
      await tester.pump(const Duration(seconds: 7));
    });
  });
}
