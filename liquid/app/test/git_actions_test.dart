import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/checkout_pull_request.dart';
import 'package:liquid/src/agent/git_actions.dart';
import 'package:liquid/src/agent/pull_request_rows.dart';
import 'package:liquid/src/agent/session_menus.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

class _ScriptedGit extends FakeGit {
  _ScriptedGit({
    super.status,
    this.resolveError,
    this.checkOutError,
    this.publishError,
  });

  final Object? resolveError;
  final Object? checkOutError;
  final Object? publishError;
  Object? actionError;
  int reads = 0;
  bool gated = false;
  final progress = StreamController<BridgeGitProgress>();

  @override
  Stream<BridgeGitProgress> run(
    BridgeGitAction action, {
    String? message,
    List<String>? files,
    bool featureBranch = false,
  }) => gated
      ? progress.stream
      : super.run(
          action,
          message: message,
          files: files,
          featureBranch: featureBranch,
        );

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

  @override
  Future<void> runPullRequestAction(
    int number,
    String action, {
    String? mergeMethod,
  }) async {
    if (actionError case final error?) throw error;
    return super.runPullRequestAction(number, action, mergeMethod: mergeMethod);
  }

  @override
  Future<BridgePullRequestDetail> pullRequest(int number) {
    reads++;
    return super.pullRequest(number);
  }
}

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

_ScriptedGit _prGit(BridgePullRequestDetail detail, {bool draft = false}) =>
    _ScriptedGit(
      status: gitStatusOf(pullRequest: prOf(draft: draft)),
    )..detail = detail;

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

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyEvent(key);
  await tester.pumpAndSettle();
}

VoidCallback? onPressed(WidgetTester tester, Finder finder) =>
    tester.widget<PrimaryButton>(finder).onPressed;

Future<SessionState> pumpIn(
  WidgetTester tester,
  FakeGit git,
  Widget Function(SessionState state) body,
) async {
  final state = SessionState(FakeProject(git: git));
  addTearDown(state.dispose);
  await tester.pumpWidget(
    MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(body: body(state)),
    ),
  );
  await tester.pumpAndSettle();
  return state;
}

Future<SessionState> pumpGit(WidgetTester tester, FakeGit git) => pumpIn(
  tester,
  git,
  (state) => Column(
    children: [
      GitActionsControl(state: state),
      PullRequestRows(state: state),
    ],
  ),
);

Future<SessionState> pumpCheckout(
  WidgetTester tester,
  FakeGit git, {
  String? reference,
}) async {
  final state = await pumpIn(
    tester,
    git,
    (state) => Builder(
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
  );
  await tap(tester, find.text('Open'));
  if (reference != null) {
    await tester.pump(const Duration(milliseconds: 450));
    await tester.pumpAndSettle();
  }
  return state;
}

void main() {
  testWidgets(
    'the Git action options say why an action is unavailable, and run one',
    (tester) async {
      final git = FakeGit(status: featureOf());
      final state = await pumpGit(tester, git);
      await tap(tester, find.byTooltip('Git action options'));
      for (final reason in [
        'Worktree is clean. Make changes before committing.',
        'No local commits to push.',
        'No local commits to include in a pull request.',
      ]) {
        expect(find.byTooltip(reason), findsOneWidget);
      }
      await press(tester, LogicalKeyboardKey.escape);
      git.change(featureOf(changes: [fileOf('a.dart')], ahead: 1));
      await tester.pumpAndSettle();
      await tap(tester, find.byTooltip('Git action options'));
      await tap(tester, find.text('Push'));
      expect(git.calls, ['run push message=null files=null feature=false']);
      await state.dispose();
    },
  );

  testWidgets(
    'an action on the default branch asks first: Abort runs nothing, Continue runs it there',
    (tester) async {
      final git = FakeGit(status: gitStatusOf(ahead: 1));
      final state = await pumpGit(tester, git);
      await tap(tester, find.text('Push'));
      expect(find.text('Push to default ref?'), findsOneWidget);
      expect(find.textContaining('on "main"'), findsOneWidget);
      expect(
        onPressed(
          tester,
          find.widgetWithText(
            PrimaryButton,
            'Check out feature branch & continue',
          ),
        ),
        isNull,
      );
      await tap(tester, find.text('Abort'));
      expect(git.calls, isEmpty);
      await tap(tester, find.text('Push'));
      await tap(tester, find.text('Push to main'));
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
    await tap(tester, find.text('Commit & push'));
    expect(find.text('Commit & push to default ref?'), findsOneWidget);
    await tap(tester, find.text('Check out feature branch & continue'));
    expect(git.calls, ['run commitPush message=null files=null feature=true']);
    await state.dispose();
  });

  testWidgets('a folder outside Git offers Initialize Git', (tester) async {
    final git = FakeGit(status: gitStatusOf(repository: false));
    await pumpGit(tester, git);
    await tap(tester, find.text('Initialize Git'));
    expect(git.calls, ['init']);
  });

  testWidgets(
    'Publish repository needs owner/repo and creates it with the visibility chosen',
    (tester) async {
      final git = FakeGit(status: featureOf(upstream: false, remote: false));
      await pumpGit(tester, git);
      await tap(tester, find.text('Publish repository'));
      final publish = find.widgetWithText(PrimaryButton, 'Publish');
      expect(onPressed(tester, publish), isNull);
      await tester.enterText(find.byType(TextField), 'acme/api');
      await tap(tester, find.text('Public'));
      await tap(tester, publish);
      expect(git.calls, ['publish github acme/api public=true']);
      expect(find.text('Repository published'), findsOneWidget);
    },
  );

  testWidgets('a failed publish keeps the dialog open and says why', (
    tester,
  ) async {
    await pumpGit(
      tester,
      _ScriptedGit(
        status: featureOf(upstream: false, remote: false),
        publishError: Exception('offline'),
      ),
    );
    await tap(tester, find.text('Publish repository'));
    await tester.enterText(find.byType(TextField), 'acme/api');
    await tester.pumpAndSettle();
    await tap(tester, find.widgetWithText(PrimaryButton, 'Publish'));
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
      await tap(tester, find.widgetWithText(SecondaryButton, 'Local'));
      expect(git.calls, ['resolve 42', 'check out 42 worktree=false']);
      expect(find.text('Add a feature'), findsNothing);
      expect((state.worktree.value, state.newWorktree.value), (null, false));
    },
  );

  testWidgets(
    'Worktree checks the pull request out in its own worktree, which the next thread starts in',
    (tester) async {
      final git = FakeGit();
      final state = await pumpCheckout(tester, git, reference: '42');
      await tap(tester, find.widgetWithText(PrimaryButton, 'Worktree'));
      expect(git.calls, ['resolve 42', 'check out 42 worktree=true']);
      expect(
        (state.worktree.value, state.newWorktree.value),
        ('/worktrees/feature', false),
      );
    },
  );

  testWidgets(
    'a reference that does not resolve says so, and Worktree stays off',
    (tester) async {
      await pumpCheckout(
        tester,
        _ScriptedGit(resolveError: Exception('offline')),
        reference: '42',
      );
      expect(
        find.textContaining('Could not resolve the pull request.'),
        findsOneWidget,
      );
      expect(
        onPressed(tester, find.widgetWithText(PrimaryButton, 'Worktree')),
        isNull,
      );
    },
  );

  testWidgets('a checkout that fails keeps the dialog open with the reason', (
    tester,
  ) async {
    await pumpCheckout(
      tester,
      _ScriptedGit(checkOutError: Exception('locked')),
      reference: '42',
    );
    await tap(tester, find.widgetWithText(SecondaryButton, 'Local'));
    expect(
      find.textContaining('Failed to prepare pull request thread.'),
      findsOneWidget,
    );
    expect(find.text('Checkout pull request'), findsOneWidget);
  });

  testWidgets(
    'the checks badge counts the checks that finished, and its popover lists each check with its details',
    (tester) async {
      final opened = recordOpens();
      await pumpGit(
        tester,
        _prGit(
          detailOf(
            checks: [checkOf('build', 'success'), checkOf('lint', 'failure')],
            checksState: 'failing',
            checksSummary: '1 of 2 failing',
          ),
        ),
      );
      expect(find.text('1 failed'), findsOneWidget);

      await pumpGit(
        tester,
        _prGit(
          detailOf(
            checks: [
              checkOf('build', 'success'),
              checkOf('lint', 'failure', url: 'https://ci.example/lint'),
              checkOf('deploy', 'pending'),
            ],
            checksState: 'failing',
            checksSummary: '1 of 3 failing',
          ),
        ),
      );
      expect(find.text('2/3 checks'), findsOneWidget);

      await tap(tester, find.text('2/3 checks'));
      expect(find.text('Some checks were not successful'), findsOneWidget);
      expect(find.text('lint'), findsOneWidget);
      expect(find.text('deploy'), findsOneWidget);
      expect(find.text('build'), findsNothing, reason: 'finished checks wait');
      await tap(tester, find.text('Show all'));
      expect(find.text('build'), findsOneWidget);
      await tap(tester, find.text('Details'));
      expect(opened, ['https://ci.example/lint']);
    },
  );

  testWidgets('Ready marks a draft ready for review', (tester) async {
    final git = _prGit(detailOf(draft: true, action: 'ready'), draft: true);
    final state = await pumpGit(tester, git);
    expect(
      find.byTooltip('Mark this pull request as ready for review'),
      findsOneWidget,
    );
    await tap(tester, find.text('Ready'));
    expect(git.calls, ['pr 7 ready merge=null']);
    expect(state.notice.value?.text, 'Marked ready for review');
  });

  testWidgets('Merge asks first, then merges with the repository\'s method', (
    tester,
  ) async {
    final git = _prGit(detailOf(action: 'merge'));
    final state = await pumpGit(tester, git);
    await tap(tester, find.text('Merge'));
    expect(find.text('Merge pull request?'), findsOneWidget);
    expect(find.text('This merges #7 using squash.'), findsOneWidget);
    await tap(tester, find.text('Cancel'));
    expect(git.calls, isEmpty);
    await tap(tester, find.text('Merge'));
    await tap(tester, find.widgetWithText(PrimaryButton, 'Merge'));
    expect(git.calls, ['pr 7 merge merge=squash']);

    git.actionError = Exception('host refused');
    await tap(tester, find.text('Merge'));
    await tap(tester, find.widgetWithText(PrimaryButton, 'Merge'));
    expect(
      state.notice.value?.text,
      'Could not merge this pull request. Exception: host refused',
    );
    expect(find.text('Merge'), findsOneWidget);
  });

  testWidgets(
    'Fix checks the pull request out in a worktree and opens the start page with the fix in the composer',
    (tester) async {
      final git = _prGit(
        detailOf(
          action: 'fix',
          checks: [checkOf('lint', 'failure')],
          checksState: 'failing',
        ),
      );
      final state = await pumpGit(tester, git);
      await tap(tester, find.text('Fix'));
      expect(git.calls, ['pr worktree https://github.com/o/r/pull/7']);
      expect(
        (
          state.session,
          state.worktree.value,
          state.newWorktree.value,
          state.draft.value,
        ),
        (null, '/worktrees/pr', false, 'Fix the checks'),
      );
    },
  );

  testWidgets(
    'Resolve puts the conflict prompt in the composer of its worktree',
    (tester) async {
      final git = _prGit(detailOf(action: 'resolve', conflicting: true));
      final state = await pumpGit(tester, git);
      await tap(tester, find.text('Resolve'));
      expect(git.calls, ['pr worktree https://github.com/o/r/pull/7']);
      expect(state.draft.value, 'Resolve the conflicts');
    },
  );

  testWidgets('the checks still running are read again after 45 seconds', (
    tester,
  ) async {
    final git = _prGit(
      detailOf(checks: [checkOf('deploy', 'pending')], checksState: 'pending'),
    );
    await pumpGit(tester, git);
    final reads = git.reads;
    await tester.pump(const Duration(seconds: 44));
    expect(git.reads, reads);
    await tester.pump(const Duration(seconds: 1));
    expect(git.reads, reads + 1);
  });

  testWidgets(
    'a click opens an https pull request in the browser, and refuses any other link',
    (tester) async {
      final opened = recordOpens();
      final git = FakeGit(status: gitStatusOf(pullRequest: prOf()));
      final state = await pumpGit(tester, git);
      await tap(tester, find.text('#7: Add a feature'));
      expect(opened, ['https://github.com/o/r/pull/7']);
      git.change(gitStatusOf(pullRequest: prOf(url: 'http://example.com/7')));
      await tester.pumpAndSettle();
      await tap(tester, find.text('#7: Add a feature'));
      expect(opened, hasLength(1));
      expect(state.notice.value?.text, contains('not an HTTPS page'));
    },
  );
}
