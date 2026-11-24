import 'dart:async';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_menus.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_page.dart';

import 'card_harness.dart';
import 'fake_backend.dart';
import 'focus_kit.dart';

final info = SessionInfo(project: 'acme-api', path: '/work/acme-api');

List<BridgeThreadSummary> fixLogin({
  String id = 'thread-2',
  String? worktree,
}) => [threadOf(id, title: 'Fix login', worktree: worktree)];

Future<void> tap(
  WidgetTester tester,
  Finder finder, {
  int buttons = kPrimaryButton,
}) async {
  await tester.tap(finder, buttons: buttons);
  await tester.pumpAndSettle();
}

void main() {
  late FakeProject project;
  late SessionState state;
  late List<String> opened;
  setUp(() => opened = recordOpens());

  Future<void> show(
    WidgetTester tester, {
    List<BridgeProvider>? providers,
    List<BridgeThreadSummary> threads = const [],
    Size size = const Size(1600, 900),
  }) async {
    await tester.binding.setSurfaceSize(size);
    project = FakeProject()
      ..providerList =
          providers ??
          [
            providerOf(
              models: [
                modelOf('claude-opus-5-5', 'Claude Opus 5.5'),
                modelOf('claude-sonnet-5-5', 'Claude Sonnet 5.5'),
              ],
            ),
          ]
      ..threadList = threads
      ..gitBackend.refList.add(refOf('develop'));
    state = SessionState(project);
    await state.loadStart();
    await pumpApp(tester, AgentShell(state: state, info: info));
  }

  Future<void> sendFirst(WidgetTester tester, String text) async {
    await tester.enterText(find.byKey(const Key('composer')), text);
    await tester.pumpAndSettle();
    await tap(tester, find.text('Submit message'));
  }

  Future<void> openMenu(WidgetTester tester, String title) =>
      tap(tester, find.text(title), buttons: kSecondaryButton);

  Future<void> deleteFixLogin(WidgetTester tester) async {
    await openMenu(tester, 'Fix login');
    await tap(tester, find.text('Delete'));
  }

  Future<void> exportFixLogin(WidgetTester tester) async {
    await openMenu(tester, 'Fix login');
    await tap(tester, find.text('Export record'));
  }

  testWidgets('a new worktree starts from the branch picked', (tester) async {
    await show(tester);
    await tap(tester, find.text('From main'));
    await tap(tester, find.text('develop'));
    expect(find.text('From develop'), findsOneWidget);

    await sendFirst(tester, 'Fix login');
    expect(
      project.calls.single,
      'start Fix login claude-opus-5-5 base=develop worktree=null fullAccess',
    );
  });

  testWidgets('the workspace menu starts the session in the project folder', (
    tester,
  ) async {
    await show(tester);
    await tap(tester, find.text('New worktree'));
    await tap(tester, find.text('Current checkout'));
    expect(find.text('From main'), findsNothing);

    await sendFirst(tester, 'Fix login');
    expect(
      project.calls.single,
      'start Fix login claude-opus-5-5 base=null worktree=null fullAccess',
    );
  });

  testWidgets('the model menu picks the model the next session starts with', (
    tester,
  ) async {
    await show(tester);
    await tap(tester, find.byTooltip('Model'));
    await tap(tester, find.text('Claude Sonnet 5.5'));
    expect(state.model.value?.model, 'claude-sonnet-5-5');

    await sendFirst(tester, 'Fix login');
    expect(
      project.calls.single,
      startsWith('start Fix login claude-sonnet-5-5 '),
    );
  });

  testWidgets('the permissions menu picks a mode', (tester) async {
    await show(tester);
    await tap(tester, find.text('Full access'));
    await tap(tester, find.text('Auto-accept edits'));
    expect(find.text('Auto-accept edits'), findsOneWidget);

    await sendFirst(tester, 'Fix login');
    expect(project.calls.single, endsWith(' autoAcceptEdits'));
  });

  testWidgets('a model option such as effort is chosen from its chip', (
    tester,
  ) async {
    await show(
      tester,
      providers: [
        providerOf(
          models: [
            modelOf(
              'claude-opus-5-5',
              'Claude Opus 5.5',
              options: const [
                BridgeModelOption(
                  id: 'effort',
                  label: 'Effort',
                  choices: ['low', 'high'],
                  labels: ['Low', 'High'],
                  default_: 'high',
                ),
              ],
            ),
          ],
        ),
      ],
    );
    await tap(tester, find.text('High'));
    await tap(tester, find.text('Low'));
    expect(find.text('Low'), findsOneWidget);
    expect(state.model.value?.values, ['low']);
  });

  testWidgets('the sidebar lists the project\'s threads and opens one', (
    tester,
  ) async {
    await show(
      tester,
      threads: [threadOf('thread-2', title: 'Rate limit for /login')],
    );
    await tap(tester, find.text('Rate limit for /login'));
    expect(project.calls, contains('open thread-2'));
  });

  testWidgets('a thread is archived from its menu, and a running one is not', (
    tester,
  ) async {
    await show(
      tester,
      threads: [
        threadOf('thread-2', title: 'Fix login'),
        threadOf('thread-3', title: 'Run tests', running: true),
      ],
    );
    await openMenu(tester, 'Fix login');
    await tap(tester, find.text('Archive thread'));
    expect(project.calls, contains('archive thread-2 true'));

    await openMenu(tester, 'Run tests');
    await tap(tester, find.text('Archive thread'));
    expect(project.calls, isNot(contains('archive thread-3 true')));
  });

  testWidgets('cancelling a delete deletes nothing', (tester) async {
    await show(tester, threads: fixLogin());
    await deleteFixLogin(tester);
    expect(find.text('Delete thread "Fix login"?'), findsOneWidget);
    await tap(tester, find.text('Cancel'));
    expect(project.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  testWidgets(
    'deleting the only thread of a worktree asks about the worktree first',
    (tester) async {
      await show(tester, threads: fixLogin(worktree: '/worktrees/fix'));
      project.onlyInWorktree = true;
      await deleteFixLogin(tester);
      await tap(tester, find.text('Confirm'));
      expect(find.text('Delete the worktree too?'), findsOneWidget);
      expect(
        find.text(
          'This thread is the only one linked to this worktree:\n/worktrees/fix',
        ),
        findsOneWidget,
      );

      await tap(tester, find.text('Cancel'));
      expect(
        project.calls,
        contains('delete thread-2 worktree=false'),
        reason: 'Cancel keeps the worktree and still deletes the thread',
      );
    },
  );

  testWidgets('confirming deletes the worktree with its only thread', (
    tester,
  ) async {
    await show(tester, threads: fixLogin(worktree: '/worktrees/fix'));
    project.onlyInWorktree = true;
    await deleteFixLogin(tester);
    await tap(tester, find.text('Confirm'));
    await tap(tester, find.text('Confirm'));
    expect(project.calls, contains('delete thread-2 worktree=true'));
  });

  testWidgets('a worktree check that fails stops the delete and says why', (
    tester,
  ) async {
    await show(tester, threads: fixLogin(worktree: '/worktrees/fix'));
    project.onlyInWorktreeError = 'the worktree is locked';
    await deleteFixLogin(tester);
    await tap(tester, find.text('Confirm'));
    expect(
      state.notice.value?.text,
      'Could not delete the session: the worktree is locked',
    );
    expect(project.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  testWidgets(
    'a thread exports its record into Downloads and shows the folder',
    (tester) async {
      await show(tester, threads: fixLogin(id: 'thread-1'));
      await state.open('thread-1');
      await tester.pumpAndSettle();
      await exportFixLogin(tester);
      expect(project.session.calls.single, startsWith('export '));
      expect(opened.single, startsWith('/Downloads/'));
    },
  );

  testWidgets('Settings picks the writer model for the project', (
    tester,
  ) async {
    await show(tester);
    await tap(tester, find.byTooltip('Settings'));
    await tap(tester, find.text('Writer model'));
    expect(find.byType(WriterModelPane), findsOneWidget);

    await tap(tester, find.text('Choose model'));
    await tap(tester, find.text('Claude Sonnet 5.5'));
    expect(project.calls, contains('writer claude-sonnet-5-5'));
    expect(find.text('Claude Sonnet 5.5'), findsOneWidget);
  });

  testWidgets('a superseded writer model save that fails shows no notice', (
    tester,
  ) async {
    await show(tester);
    await tap(tester, find.byTooltip('Settings'));
    await tap(tester, find.text('Writer model'));
    final opus = project.writerGates['claude-opus-5-5'] = Completer<void>();
    await tap(tester, find.text('Choose model'));
    await tap(tester, find.text('Claude Opus 5.5'));
    await tap(tester, find.byTooltip('Model'));
    await tap(tester, find.text('Claude Sonnet 5.5'));
    opus.completeError(StateError('disk full'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Could not save the writer model'),
      findsNothing,
    );
    expect(project.writer?.model, 'claude-sonnet-5-5');
  });

  testWidgets(
    'the stored writer model is the latest selection when saves finish late',
    (tester) async {
      await show(tester);
      await tap(tester, find.byTooltip('Settings'));
      await tap(tester, find.text('Writer model'));
      final opus = project.writerGates['claude-opus-5-5'] = Completer<void>();
      await tap(tester, find.text('Choose model'));
      await tap(tester, find.text('Claude Opus 5.5'));
      await tap(tester, find.byTooltip('Model'));
      await tap(tester, find.text('Claude Sonnet 5.5'));
      opus.complete();
      await tester.pumpAndSettle();
      expect(project.writer?.model, 'claude-sonnet-5-5');
      expect(find.text('Claude Sonnet 5.5'), findsOneWidget);
    },
  );

  testWidgets('the project menu copies the folder path', (tester) async {
    final copied = recordClipboard(tester);
    await show(tester);
    await tap(tester, find.text('acme-api').first, buttons: kSecondaryButton);
    await tap(tester, find.text('Copy path'));
    expect(copied(), '/work/acme-api');
  });

  testWidgets('a folder the system cannot open is reported', (tester) async {
    await show(tester, threads: fixLogin(id: 'thread-1'));
    await state.open('thread-1');
    await tester.pumpAndSettle();

    runOpener = (program, arguments) async =>
        ProcessResult(0, 1, '', 'no handler');
    await exportFixLogin(tester);
    expect(find.textContaining('no handler'), findsOneWidget);

    runOpener = (program, arguments) async =>
        throw ProcessException(program, arguments, 'missing');
    await exportFixLogin(tester);
    expect(find.textContaining('missing'), findsOneWidget);
  });
}
