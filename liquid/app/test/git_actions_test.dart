import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/checkout_pull_request.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/agent/pull_request_rows.dart';
import 'package:liquid/src/agent/session_card_rows.dart';
import 'package:liquid/src/agent/session_menus.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

/// Git whose run streams from the test, so the test decides when each report arrives.
class _GatedGit extends FakeGit {
  _GatedGit({super.status});

  final progress = StreamController<BridgeGitProgress>();

  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) => progress.stream;
}

/// Git whose pull request and publish calls fail with the errors given.
class _FailingGit extends FakeGit {
  _FailingGit({
    super.status,
    this.resolveError,
    this.checkOutError,
    this.publishError,
  });

  final Object? resolveError;
  final Object? checkOutError;
  final Object? publishError;

  @override
  Future<BridgeResolvedPullRequest> resolvePullRequest(String reference) async {
    if (resolveError case final error?) throw error;
    return super.resolvePullRequest(reference);
  }

  @override
  Future<BridgeCheckedOut> checkOutPullRequest(
    String reference, {
    required bool worktree,
  }) async {
    if (checkOutError case final error?) throw error;
    return super.checkOutPullRequest(reference, worktree: worktree);
  }

  @override
  Future<void> publish(
    String provider,
    String repository, {
    required bool public,
  }) async {
    if (publishError case final error?) throw error;
    return super.publish(provider, repository, public: public);
  }
}

/// A checkout on the feature branch, which is not the default branch.
BridgeGitStatus featureOf({
  List<BridgeFileChange> changes = const [],
  int ahead = 0,
  int behind = 0,
  bool upstream = true,
  bool remote = true,
}) => gitStatusOf(
  branch: 'feature',
  defaultBranch: false,
  changes: changes,
  ahead: ahead,
  behind: behind,
  upstream: upstream,
  remote: remote,
);

/// A pull request of the checkout's branch.
BridgeBranchPullRequest prOf({
  String state = 'open',
  bool draft = false,
  String url = 'https://github.com/o/r/pull/7',
}) => BridgeBranchPullRequest(
  number: 7,
  title: 'Add a feature',
  url: url,
  state: state,
  draft: draft,
  base: 'main',
);

/// Records what the browser opener is asked to open, instead of opening it.
List<String> recordOpens() {
  final opened = <String>[];
  final original = runOpener;
  runOpener = (program, arguments) async {
    opened.add(arguments.single);
    return ProcessResult(0, 0, '', '');
  };
  addTearDown(() {
    runOpener = original;
  });
  return opened;
}

/// Pumps the Git action row and the pull request row over `git`.
Future<SessionState> pumpGit(WidgetTester tester, FakeGit git) async {
  final state = SessionState(FakeProject(git: git));
  addTearDown(state.dispose);
  await tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(
        body: Column(
          children: [
            GitActionsControl(state: state),
            PullRequestRows(state: state),
          ],
        ),
      ),
    ),
  );
  await tester.pump();
  return state;
}

/// Opens the checkout dialog from a button, as the ref picker does.
Future<SessionState> pumpCheckout(
  WidgetTester tester,
  FakeGit git, {
  String? reference,
}) async {
  final state = SessionState(FakeProject(git: git));
  addTearDown(state.dispose);
  await tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(
        body: Builder(
          builder: (context) => TextButton(
            onPressed: () => showCheckoutPullRequestDialog(
              context,
              state,
              gitStatusOf(),
              reference: reference,
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
  return state;
}

void main() {
  testWidgets(
    'the quick action names the next step and is held where Git cannot act',
    (tester) async {
      final git = FakeGit();
      await pumpGit(tester, git);
      final cases = <(BridgeGitStatus, String, String?)>[
        (featureOf(changes: [fileOf('a.dart')]), 'Commit, push & PR', null),
        (featureOf(ahead: 1), 'Push & create PR', null),
        (featureOf(behind: 3), 'Pull', null),
        (featureOf(upstream: false, remote: false), 'Publish repository', null),
        (featureOf(), 'Commit', 'Branch is up to date. No action needed.'),
        (
          gitStatusOf(branch: null, defaultBranch: false),
          'Commit',
          'Create and checkout a ref before pushing or opening a pull request.',
        ),
      ];
      for (final (status, label, hint) in cases) {
        git.change(status);
        await tester.pumpAndSettle();
        final row = find.ancestor(
          of: find.text(label),
          matching: find.byType(CardRow),
        );
        expect(
          tester.widget<CardRow>(row).onTap != null,
          hint == null,
          reason: label,
        );
        if (hint != null) expect(find.byTooltip(hint), findsOneWidget);
      }
    },
  );

  testWidgets(
    'the Git action options say why an action is unavailable, and run an available one',
    (tester) async {
      final git = FakeGit(status: featureOf());
      final state = await pumpGit(tester, git);
      await tester.tap(find.byTooltip('Git action options'));
      await tester.pumpAndSettle();
      for (final reason in [
        'Worktree is clean. Make changes before committing.',
        'No local commits to push.',
        'No local commits to include in a pull request.',
      ]) {
        expect(find.byTooltip(reason), findsOneWidget);
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      git.change(featureOf(changes: [fileOf('a.dart')], ahead: 1));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Git action options'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Push'));
      await tester.pumpAndSettle();
      expect(git.calls, ['run push message=null files=null feature=false']);
      await state.dispose();
    },
  );

  testWidgets(
    'a running action shows its phase and hook line until it finishes',
    (tester) async {
      final git = _GatedGit(status: featureOf(ahead: 1));
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('Push & create PR'));
      await tester.pump();
      git.progress
        ..add(progressOf(BridgeGitProgressKind.phase, 'Pushing feature'))
        ..add(progressOf(BridgeGitProgressKind.output, 'remote: resolving'));
      await tester.pump();
      expect(find.text('Pushing feature'), findsOneWidget);
      expect(find.text('remote: resolving'), findsOneWidget);
      expect(find.text('Push & create PR'), findsNothing);
      git.progress.add(
        progressOf(
          BridgeGitProgressKind.finished,
          'Pull request created',
          description: 'feature into main',
        ),
      );
      await git.progress.close();
      await tester.pumpAndSettle();
      expect(find.text('Pull request created'), findsOneWidget);
      await state.dispose();
    },
  );

  testWidgets(
    'a result stays ten seconds, and Open PR opens the pull request',
    (tester) async {
      final opened = recordOpens();
      final git = FakeGit(status: featureOf(ahead: 1))
        ..steps = [
          progressOf(
            BridgeGitProgressKind.finished,
            'Pull request created',
            description: 'feature into main',
            pullRequest: 'https://github.com/o/r/pull/9',
          ),
        ];
      await pumpGit(tester, git);
      await tester.tap(find.text('Push & create PR'));
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 9));
      expect(find.text('Pull request created'), findsOneWidget);
      await tester.tap(find.text('Open PR'));
      await tester.pumpAndSettle();
      expect(opened, ['https://github.com/o/r/pull/9']);
      await tester.pump(const Duration(seconds: 1));
      expect(find.text('Pull request created'), findsNothing);
      expect(find.text('Push & create PR'), findsOneWidget);
    },
  );

  testWidgets(
    'an action on the default branch asks first: Abort runs nothing, Continue runs it there',
    (tester) async {
      final git = FakeGit(status: gitStatusOf(ahead: 1));
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('Push'));
      await tester.pumpAndSettle();
      expect(find.text('Push to default ref?'), findsOneWidget);
      expect(find.textContaining('on "main"'), findsOneWidget);
      expect(
        tester
            .widget<PrimaryButton>(
              find.widgetWithText(
                PrimaryButton,
                'Check out feature branch & continue',
              ),
            )
            .onPressed,
        isNull,
      );
      await tester.tap(find.text('Abort'));
      await tester.pumpAndSettle();
      expect(git.calls, isEmpty);
      await tester.tap(find.text('Push'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Push to main'));
      await tester.pumpAndSettle();
      expect(git.calls, [
        'run commitPush message=null files=null feature=false',
      ]);
      await state.dispose();
    },
  );

  testWidgets('a commit on the default branch can take a feature branch', (
    tester,
  ) async {
    final git = FakeGit(
      status: gitStatusOf(ahead: 1, changes: [fileOf('a.dart')]),
    );
    final state = await pumpGit(tester, git);
    await tester.tap(find.text('Commit & push'));
    await tester.pumpAndSettle();
    expect(find.text('Commit & push to default ref?'), findsOneWidget);
    await tester.tap(find.text('Check out feature branch & continue'));
    await tester.pumpAndSettle();
    expect(git.calls, ['run commitPush message=null files=null feature=true']);
    await state.dispose();
  });

  testWidgets('a folder outside Git offers Initialize Git', (tester) async {
    final git = FakeGit(status: gitStatusOf(repository: false));
    await pumpGit(tester, git);
    await tester.tap(find.text('Initialize Git'));
    await tester.pumpAndSettle();
    expect(git.calls, ['init']);
  });

  testWidgets(
    'Publish repository needs owner/repo and creates it with the visibility chosen',
    (tester) async {
      final git = FakeGit(status: featureOf(upstream: false, remote: false));
      await pumpGit(tester, git);
      await tester.tap(find.text('Publish repository'));
      await tester.pumpAndSettle();
      final publish = find.widgetWithText(PrimaryButton, 'Publish');
      expect(tester.widget<PrimaryButton>(publish).onPressed, isNull);
      await tester.enterText(find.byType(TextField), 'acme/api');
      await tester.tap(find.text('Public'));
      await tester.pumpAndSettle();
      await tester.tap(publish);
      await tester.pumpAndSettle();
      expect(git.calls, ['publish github acme/api public=true']);
      expect(find.text('Repository published'), findsOneWidget);
    },
  );

  testWidgets('a failed publish keeps the dialog open and says why', (
    tester,
  ) async {
    await pumpGit(
      tester,
      _FailingGit(
        status: featureOf(upstream: false, remote: false),
        publishError: Exception('offline'),
      ),
    );
    await tester.tap(find.text('Publish repository'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'acme/api');
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(PrimaryButton, 'Publish'));
    await tester.pumpAndSettle();
    expect(find.textContaining('Publish failed.'), findsOneWidget);
  });

  testWidgets(
    'a typed reference resolves after a pause, and Local checks it out in the project folder',
    (tester) async {
      final git = FakeGit();
      final state = await pumpCheckout(tester, git);
      await tester.enterText(find.byType(TextField), 'nope');
      await tester.pump();
      expect(find.textContaining('Use a pull request URL'), findsOneWidget);
      await tester.enterText(find.byType(TextField), '');
      await tester.pump();
      expect(find.textContaining('Paste a pull request URL'), findsOneWidget);
      await tester.enterText(find.byType(TextField), '#42');
      await tester.pump(const Duration(milliseconds: 450));
      await tester.pumpAndSettle();
      expect(find.text('Add a feature'), findsOneWidget);
      await tester.tap(find.widgetWithText(SecondaryButton, 'Local'));
      await tester.pumpAndSettle();
      expect(git.calls, ['resolve 42', 'check out 42 worktree=false']);
      expect(find.text('Add a feature'), findsNothing);
      expect(state.worktree.value, isNull);
      expect(state.newWorktree.value, isFalse);
    },
  );

  testWidgets(
    'Worktree checks the pull request out in its own worktree, which the next thread starts in',
    (tester) async {
      final git = FakeGit();
      final state = await pumpCheckout(tester, git, reference: '42');
      await tester.pump(const Duration(milliseconds: 450));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(PrimaryButton, 'Worktree'));
      await tester.pumpAndSettle();
      expect(git.calls, ['resolve 42', 'check out 42 worktree=true']);
      expect(state.worktree.value, '/worktrees/feature');
      expect(state.newWorktree.value, isFalse);
    },
  );

  testWidgets('a checkout that fails keeps the dialog open with the reason', (
    tester,
  ) async {
    await pumpCheckout(
      tester,
      _FailingGit(checkOutError: Exception('locked')),
      reference: '42',
    );
    await tester.pump(const Duration(milliseconds: 450));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(SecondaryButton, 'Local'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Failed to prepare pull request thread.'),
      findsOneWidget,
    );
    expect(find.text('Checkout pull request'), findsOneWidget);
  });

  testWidgets(
    'a reference that does not resolve says so, and Worktree stays off',
    (tester) async {
      await pumpCheckout(
        tester,
        _FailingGit(resolveError: Exception('offline')),
        reference: '42',
      );
      await tester.pump(const Duration(milliseconds: 450));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('Could not resolve the pull request.'),
        findsOneWidget,
      );
      expect(
        tester
            .widget<PrimaryButton>(
              find.widgetWithText(PrimaryButton, 'Worktree'),
            )
            .onPressed,
        isNull,
      );
    },
  );

  testWidgets('the pull request row names the state the checkout reports', (
    tester,
  ) async {
    final git = FakeGit();
    await pumpGit(tester, git);
    for (final (pr, label) in [
      (prOf(), 'Open'),
      (prOf(draft: true), 'Draft'),
      (prOf(state: 'closed'), 'Closed'),
      (prOf(state: 'merged'), 'Merged'),
    ]) {
      git.change(gitStatusOf(pullRequest: pr));
      await tester.pumpAndSettle();
      expect(find.text(label), findsOneWidget);
    }
  });

  testWidgets(
    'a click opens an https pull request in the browser, and refuses any other link',
    (tester) async {
      final opened = recordOpens();
      final git = FakeGit(status: gitStatusOf(pullRequest: prOf()));
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('#7: Add a feature'));
      await tester.pumpAndSettle();
      expect(opened, ['https://github.com/o/r/pull/7']);
      git.change(gitStatusOf(pullRequest: prOf(url: 'http://example.com/7')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('#7: Add a feature'));
      await tester.pumpAndSettle();
      expect(opened, hasLength(1));
      expect(state.notice.value?.text, contains('not an HTTPS page'));
    },
  );
}
