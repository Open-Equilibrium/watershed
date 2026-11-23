import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/branch_menu.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

class _BrokenGit extends FakeGit {
  @override
  Stream<BridgeGitStatus> watch() => Stream.error(Exception('no repository'));
}

void expectOne(Finder finder) => expect(finder, findsOneWidget);
void expectNone(Finder finder) => expect(finder, findsNothing);

void main() {
  late FakeSession session;
  late FakeGit git;
  late SessionState state;

  Future<void> pump(
    WidgetTester tester, {
    FakeGit? checkout,
    List<BridgeTurn> turns = const [],
  }) async {
    git =
        checkout ??
        FakeGit(status: gitStatusOf(branch: 'feature', defaultBranch: false));
    git.refList = [
      refOf('main', isDefault: true),
      refOf('feature', current: true),
      refOf('fix', worktree: '/worktrees/fix'),
      refOf('origin/fix', remote: 'origin'),
    ];
    session = FakeSession()
      ..gitBackend = git
      ..current = snapshotOf(turns: turns)
      ..turnList = turns;
    state = SessionState(FakeProject(session: session, git: git));
    addTearDown(state.dispose);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(width: 280, child: BranchRow(state: state)),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> search(WidgetTester tester, String text) async {
    await tester.enterText(find.byKey(const Key('branch-search')), text);
    await tester.pumpAndSettle();
  }

  Future<void> openPicker(WidgetTester tester) =>
      tap(tester, find.byKey(const Key('branch-chevron')));

  testWidgets('the branch name, its chevron and its accessibility action open '
      'the ref picker', (tester) async {
    await pump(tester);
    final semantics = tester.ensureSemantics();
    await tap(tester, find.byKey(const Key('branch-name')));
    expectOne(find.byKey(const Key('branch-search')));
    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    await openPicker(tester);
    expectOne(find.byKey(const Key('branch-search')));
    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    tester.semantics.tap(find.semantics.byLabel('Branch feature'));
    await tester.pumpAndSettle();
    expectOne(find.byKey(const Key('branch-search')));
    semantics.dispose();
  });

  testWidgets('the ref picker names each ref, a search narrows it and offers '
      'a new ref, and Enter switches to the first match', (tester) async {
    await pump(tester);
    await openPicker(tester);
    for (final detail in ['current', 'default', 'worktree', 'remote']) {
      expectOne(find.text(detail));
    }
    await search(tester, 'fea');
    expectNone(find.text('main'));
    expectOne(find.text('Create new ref "fea"'));

    await search(tester, 'main');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(git.calls, ['switch main']);
  });

  testWidgets('a remote ref switches to the local branch of its name, and a '
      'ref checked out in a worktree opens that worktree', (tester) async {
    await pump(tester);
    await openPicker(tester);
    await tap(tester, find.text('origin/fix'));
    expect(git.calls, ['switch fix']);

    await openPicker(tester);
    await tap(tester, find.text('fix'));
    expect(session.calls, ['branch fix worktree=/worktrees/fix']);
  });

  testWidgets('a switch with uncommitted changes asks first, and Cancel keeps '
      'the branch', (tester) async {
    await pump(tester);
    git.change(
      gitStatusOf(
        branch: 'feature',
        defaultBranch: false,
        changes: [fileOf('a.dart')],
      ),
    );
    await tester.pumpAndSettle();
    await openPicker(tester);
    await tap(tester, find.text('main'));
    expectOne(find.text('Switch to main?'));
    await tap(tester, find.widgetWithText(SecondaryButton, 'Cancel'));
    expect(git.calls, isEmpty);
    await openPicker(tester);
    await tap(tester, find.text('main'));
    await tap(tester, find.widgetWithText(PrimaryButton, 'Switch branch'));
    expect(git.calls, ['switch main']);
  });

  testWidgets('a new name creates and checks out a ref, its spaces made '
      'dashes; Git refusing it says why', (tester) async {
    await pump(tester);
    await openPicker(tester);
    await search(tester, 'new feature');
    await tap(tester, find.text('Create new ref "new-feature"'));
    expect(git.calls, ['create new-feature']);

    await openPicker(tester);
    await search(tester, 'taken');
    git.refError = Exception('ref exists');
    await tap(tester, find.text('Create new ref "taken"'));
    expect(
      state.notice.value?.text,
      contains('Failed to create and switch ref.'),
    );
  });

  testWidgets('while a turn runs the picker offers no switch and says to stop '
      'it', (tester) async {
    await pump(tester, turns: [turnOf(1, status: BridgeTurnStatus.running)]);
    await openPicker(tester);
    expectOne(find.text('Stop the current turn to switch worktrees.'));
    await tap(tester, find.text('main'));
    expect(git.calls, isEmpty);
    expectOne(find.byKey(const Key('branch-search')));
  });

  testWidgets('a pull request reference typed in the picker offers its '
      'checkout', (tester) async {
    await pump(tester);
    await openPicker(tester);
    await search(tester, '#42');
    await tester.tap(find.text('Checkout pull request'));
    await tester.pump(const Duration(milliseconds: 450));
    await tester.pumpAndSettle();
    expectOne(find.text('Add a feature'));
    expect(git.calls, ['resolve 42']);
  });

  testWidgets('a ref list Git cannot read says so and closes the '
      'picker', (tester) async {
    await pump(tester);
    git.refError = Exception('busy');
    await openPicker(tester);
    expect(state.notice.value?.text, contains('Could not list the refs:'));
    expectNone(find.byKey(const Key('branch-search')));
  });

  testWidgets('right-clicking the branch name copies it, and says '
      'so', (tester) async {
    final copied = <String>[];
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') {
        copied.add((call.arguments as Map)['text'] as String);
      }
      return null;
    });
    addTearDown(
      () => messenger.setMockMethodCallHandler(SystemChannels.platform, null),
    );
    await pump(tester);
    await tester.tap(
      find.byKey(const Key('branch-name')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    expectOne(find.text('Copy branch name'));
    await tap(tester, find.text('Copy branch name'));
    expect(copied, ['feature']);
    expect(state.notice.value?.text, 'Branch name copied: feature');
  });

  testWidgets('a checkout Git cannot read says so in the branch '
      'row', (tester) async {
    await pump(tester, checkout: _BrokenGit());
    expectOne(find.text('Git unavailable'));
  });
}
