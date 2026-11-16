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
import 'package:liquid/src/theme/icons.dart';
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

/// Git that counts the pull request reads.
class _CountingGit extends FakeGit {
  _CountingGit({super.status});

  int reads = 0;

  @override
  Future<BridgePullRequestDetail> pullRequest(int number) {
    reads++;
    return super.pullRequest(number);
  }
}

/// Git whose host refuses every pull request action.
class _RefusingGit extends FakeGit {
  _RefusingGit({super.status});

  @override
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  }) async => throw Exception('host refused');
}

/// A pull request's detail as the host reports it: its checks, the action its row offers and
/// whether it conflicts with its base.
BridgePullRequestDetail detailOf({
  String state = 'open',
  bool draft = false,
  bool conflicting = false,
  List<BridgePullRequestCheck> checks = const [],
  String checksState = 'none',
  String checksSummary = '',
  String? action,
}) => BridgePullRequestDetail(
  number: 7,
  title: 'Add a feature',
  url: 'https://github.com/o/r/pull/7',
  state: state,
  draft: draft,
  base: 'main',
  head: 'feature',
  conflicting: conflicting,
  files: 1,
  added: 1,
  removed: 0,
  checks: checks,
  checksState: checksState,
  checksSummary: checksSummary,
  action: action,
  mergeMethod: 'squash',
  fixPrompt: 'Fix the checks',
  resolvePrompt: 'Resolve the conflicts',
);

BridgePullRequestCheck checkOf(String name, String status, {String? url}) =>
    BridgePullRequestCheck(name: name, status: status, url: url);

/// The tooltip with a line that reads `line`; a tooltip's lines are separated by newlines.
Finder tooltipLine(String line) => find.byWidgetPredicate(
  (widget) =>
      widget is Tooltip && (widget.message ?? '').split('\n').contains(line),
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
  await tester.pumpAndSettle();
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

  testWidgets('a result stays ten seconds, with no button beside it', (
    tester,
  ) async {
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
    expect(find.text('feature into main'), findsOneWidget);
    expect(find.text('Open PR'), findsNothing);
    await tester.pump(const Duration(seconds: 1));
    expect(find.text('Pull request created'), findsNothing);
    expect(find.text('Push & create PR'), findsOneWidget);
  });

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

  testWidgets(
    'the pull request row shows its state as an icon, and its tooltip names the state',
    (tester) async {
      final git = FakeGit();
      await pumpGit(tester, git);
      for (final (pr, detail, icon, label) in [
        (prOf(), detailOf(), LucideIcons.gitPullRequestArrow, 'Open'),
        (
          prOf(draft: true),
          detailOf(draft: true),
          LucideIcons.gitPullRequestDraft,
          'Draft',
        ),
        (
          prOf(state: 'closed'),
          detailOf(state: 'closed'),
          LucideIcons.gitPullRequestClosed,
          'Closed',
        ),
        (
          prOf(state: 'merged'),
          detailOf(state: 'merged'),
          LucideIcons.gitMerge,
          'Merged',
        ),
      ]) {
        git.detail = detail;
        git.change(gitStatusOf(pullRequest: pr));
        await tester.pumpAndSettle();
        expect(find.byIcon(icon), findsOneWidget, reason: label);
        expect(find.text(label), findsNothing, reason: label);
        expect(tooltipLine(label), findsOneWidget, reason: label);
      }
    },
  );

  testWidgets(
    'the checks badge counts the checks that finished, and its popover lists each check with its details',
    (tester) async {
      final opened = recordOpens();
      await pumpGit(
        tester,
        FakeGit(status: gitStatusOf(pullRequest: prOf()))
          ..detail = detailOf(
            checks: [checkOf('build', 'success'), checkOf('lint', 'failure')],
            checksState: 'failing',
            checksSummary: '1 of 2 failing',
          ),
      );
      expect(find.text('1 failed'), findsOneWidget);

      final git = FakeGit(status: gitStatusOf(pullRequest: prOf()))
        ..detail = detailOf(
          checks: [
            checkOf('build', 'success'),
            checkOf('lint', 'failure', url: 'https://ci.example/lint'),
            checkOf('deploy', 'pending'),
          ],
          checksState: 'failing',
          checksSummary: '1 of 3 failing',
        );
      await pumpGit(tester, git);
      expect(find.text('2/3 checks'), findsOneWidget);

      await tester.tap(find.text('2/3 checks'));
      await tester.pumpAndSettle();
      expect(find.text('Some checks were not successful'), findsOneWidget);
      expect(find.text('lint'), findsOneWidget);
      expect(find.text('deploy'), findsOneWidget);
      expect(find.text('build'), findsNothing, reason: 'finished checks wait');
      await tester.tap(find.text('Show all'));
      await tester.pumpAndSettle();
      expect(find.text('build'), findsOneWidget);
      await tester.tap(find.text('Details'));
      await tester.pumpAndSettle();
      expect(opened, ['https://ci.example/lint']);
    },
  );

  testWidgets('Ready marks a draft ready for review', (tester) async {
    final git = FakeGit(status: gitStatusOf(pullRequest: prOf(draft: true)))
      ..detail = detailOf(draft: true, action: 'ready');
    final state = await pumpGit(tester, git);
    expect(
      find.byTooltip('Mark this pull request as ready for review'),
      findsOneWidget,
    );
    await tester.tap(find.text('Ready'));
    await tester.pumpAndSettle();
    expect(git.calls, ['pr 7 ready merge=null']);
    expect(state.notice.value?.text, 'Marked ready for review');
  });

  testWidgets('Merge asks first, then merges with the repository\'s method', (
    tester,
  ) async {
    final git = FakeGit(status: gitStatusOf(pullRequest: prOf()))
      ..detail = detailOf(action: 'merge');
    await pumpGit(tester, git);
    await tester.tap(find.text('Merge'));
    await tester.pumpAndSettle();
    expect(find.text('Merge pull request?'), findsOneWidget);
    expect(find.text('This merges #7 using squash.'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(git.calls, isEmpty);
    await tester.tap(find.text('Merge'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(PrimaryButton, 'Merge'));
    await tester.pumpAndSettle();
    expect(git.calls, ['pr 7 merge merge=squash']);
  });

  testWidgets(
    'a merge the host refuses says why and leaves the row as it was',
    (tester) async {
      final git = _RefusingGit(status: gitStatusOf(pullRequest: prOf()))
        ..detail = detailOf(action: 'merge');
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('Merge'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(PrimaryButton, 'Merge'));
      await tester.pumpAndSettle();
      expect(
        state.notice.value?.text,
        'Could not merge this pull request. Exception: host refused',
      );
      expect(find.text('Merge'), findsOneWidget);
    },
  );

  testWidgets(
    'Fix checks the pull request out in a worktree and opens the start page with the fix in the composer',
    (tester) async {
      final git = FakeGit(status: gitStatusOf(pullRequest: prOf()))
        ..detail = detailOf(
          action: 'fix',
          checks: [checkOf('lint', 'failure')],
          checksState: 'failing',
        );
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('Fix'));
      await tester.pumpAndSettle();
      expect(git.calls, ['pr worktree https://github.com/o/r/pull/7']);
      expect(state.session, isNull);
      expect(state.worktree.value, '/worktrees/pr');
      expect(state.newWorktree.value, isFalse);
      expect(state.draft.value, 'Fix the checks');
    },
  );

  testWidgets(
    'Resolve puts the conflict prompt in the composer of its worktree',
    (tester) async {
      final git = FakeGit(status: gitStatusOf(pullRequest: prOf()))
        ..detail = detailOf(action: 'resolve', conflicting: true);
      final state = await pumpGit(tester, git);
      await tester.tap(find.text('Resolve'));
      await tester.pumpAndSettle();
      expect(git.calls, ['pr worktree https://github.com/o/r/pull/7']);
      expect(state.draft.value, 'Resolve the conflicts');
    },
  );

  testWidgets('the checks still running are read again after 45 seconds', (
    tester,
  ) async {
    final git = _CountingGit(status: gitStatusOf(pullRequest: prOf()))
      ..detail = detailOf(
        checks: [checkOf('deploy', 'pending')],
        checksState: 'pending',
      );
    final state = await pumpGit(tester, git);
    final reads = git.reads;
    await tester.pump(const Duration(seconds: 44));
    expect(git.reads, reads);
    await tester.pump(const Duration(seconds: 1));
    expect(git.reads, reads + 1);
    await state.dispose();
  });

  testWidgets('the quick action and the options carry T3 Code\'s icons', (
    tester,
  ) async {
    final git = FakeGit(status: featureOf(behind: 3));
    await pumpGit(tester, git);
    expect(find.byIcon(LucideIcons.cloudDownload), findsOneWidget);

    git.change(featureOf(ahead: 1));
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.gitPullRequest), findsOneWidget);
    await tester.tap(find.byTooltip('Git action options'));
    await tester.pumpAndSettle();
    expect(
      find.descendant(
        of: find.widgetWithText(PopupMenuItem<VoidCallback>, 'Push'),
        matching: find.byIcon(LucideIcons.cloudUpload),
      ),
      findsOneWidget,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    git.change(featureOf(upstream: false, remote: false));
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.cloudUpload), findsOneWidget);

    git.change(gitStatusOf(repository: false));
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.gitBranchPlus), findsOneWidget);
  });

  testWidgets(
    'the options name a detached HEAD and a checkout behind its upstream, as T3 Code does',
    (tester) async {
      final git = FakeGit(
        status: gitStatusOf(
          branch: null,
          defaultBranch: false,
          changes: [fileOf('a.dart')],
        ),
      );
      await pumpGit(tester, git);
      await tester.tap(find.byTooltip('Git action options'));
      await tester.pumpAndSettle();
      expect(
        find.text(
          'Detached HEAD: create and check out a branch to enable push and pull request actions.',
        ),
        findsOneWidget,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();

      git.change(featureOf(behind: 3));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Git action options'));
      await tester.pumpAndSettle();
      expect(find.text('Behind upstream. Pull/rebase first.'), findsOneWidget);
    },
  );

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
