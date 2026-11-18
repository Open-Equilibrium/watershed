import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_menus.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/agent/start_page.dart' show StartChips;
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_page.dart';
import 'package:liquid/src/theme/theme.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

final info = SessionInfo(project: 'acme-api', path: '/work/acme-api');

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
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> sendFirst(WidgetTester tester, String text) async {
    await tester.enterText(find.byKey(const Key('composer')), text);
    await tester.pumpAndSettle();
    await tap(tester, find.text('Submit message'));
  }

  testWidgets('the start page greets with the project and its setup chips', (
    tester,
  ) async {
    await show(tester);
    expect(find.text('What should we build in acme-api?'), findsOneWidget);
    Finder chip(String label) => find.descendant(
      of: find.byType(StartChips),
      matching: find.text(label),
    );
    expect(
      chip('New worktree'),
      findsOneWidget,
      reason: 'a new session starts in a new worktree, as T3 Code does',
    );
    expect(chip('From main'), findsOneWidget);
    expect(chip('Claude Opus 5.5'), findsOneWidget);
    expect(chip('Full access'), findsOneWidget);
  });

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

  testWidgets('the permissions menu names each mode and picks one', (
    tester,
  ) async {
    await show(tester);
    await tap(tester, find.text('Full access'));
    for (final (name, detail) in [
      ('Supervised', 'Ask before commands and file changes.'),
      ('Auto-accept edits', 'Auto-approve edits, ask before other actions.'),
      (
        'Auto',
        'Supported providers approve routine actions; others still ask.',
      ),
      ('Full access', 'Allow commands and edits without prompts.'),
    ]) {
      expect(
        find.widgetWithText(PopupMenuItem<BridgePermissions>, name),
        findsOneWidget,
      );
      expect(find.text(detail), findsOneWidget);
    }
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

  testWidgets('long model names fit the window at its minimum size', (
    tester,
  ) async {
    await show(
      tester,
      size: const Size(1050, 640),
      providers: [
        providerOf(
          models: [
            modelOf('faux-1', 'A model with a remarkably long display name'),
          ],
        ),
      ],
    );
    expect(find.text('Full access'), findsOneWidget);
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
    await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
    await tap(tester, find.text('Archive thread'));
    expect(project.calls, contains('archive thread-2 true'));

    await tap(tester, find.text('Run tests'), buttons: kSecondaryButton);
    await tap(tester, find.text('Archive thread'));
    expect(project.calls, isNot(contains('archive thread-3 true')));
  });

  testWidgets('cancelling a delete deletes nothing', (tester) async {
    await show(tester, threads: [threadOf('thread-2', title: 'Fix login')]);
    await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
    await tap(tester, find.text('Delete'));
    expect(find.text('Delete thread "Fix login"?'), findsOneWidget);
    await tap(tester, find.text('Cancel'));
    expect(project.calls.where((call) => call.startsWith('delete')), isEmpty);
  });

  testWidgets(
    'deleting the only thread of a worktree asks about the worktree first',
    (tester) async {
      await show(
        tester,
        threads: [
          threadOf('thread-2', title: 'Fix login', worktree: '/worktrees/fix'),
        ],
      );
      project.onlyInWorktree = true;
      await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
      await tap(tester, find.text('Delete'));
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
    await show(
      tester,
      threads: [
        threadOf('thread-2', title: 'Fix login', worktree: '/worktrees/fix'),
      ],
    );
    project.onlyInWorktree = true;
    await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
    await tap(tester, find.text('Delete'));
    await tap(tester, find.text('Confirm'));
    await tap(tester, find.text('Confirm'));
    expect(project.calls, contains('delete thread-2 worktree=true'));
  });

  testWidgets('a worktree check that fails stops the delete and says why', (
    tester,
  ) async {
    await show(
      tester,
      threads: [
        threadOf('thread-2', title: 'Fix login', worktree: '/worktrees/fix'),
      ],
    );
    project.onlyInWorktreeError = 'the worktree is locked';
    await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
    await tap(tester, find.text('Delete'));
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
      await show(tester, threads: [threadOf('thread-1', title: 'Fix login')]);
      await state.open('thread-1');
      await tester.pumpAndSettle();
      await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
      await tap(tester, find.text('Export record'));
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

  testWidgets('the project menu copies the folder path', (tester) async {
    final copied = recordClipboard(tester);
    await show(tester);
    await tap(tester, find.text('acme-api').first, buttons: kSecondaryButton);
    await tap(tester, find.text('Copy path'));
    expect(copied(), '/work/acme-api');
  });

  testWidgets(
    'a thread shows its branch and when it changed; its menu shows on hover',
    (tester) async {
      await show(
        tester,
        threads: [
          BridgeThreadSummary(
            id: 'thread-2',
            title: 'Fix login',
            branch: 'fix/login',
            updatedMs: DateTime.now()
                .subtract(const Duration(hours: 5))
                .millisecondsSinceEpoch,
            running: false,
            archived: false,
            movable: true,
            harnessKnown: true,
          ),
        ],
      );
      expect(find.text('fix/login'), findsOneWidget);
      expect(find.text('5 h ago'), findsOneWidget);

      final menu = find.descendant(
        of: find.ancestor(
          of: find.text('Fix login'),
          matching: find.byType(RowMenu),
        ),
        matching: find.byTooltip('More actions'),
      );
      double menuOpacity() => tester
          .widget<Opacity>(
            find.ancestor(of: menu, matching: find.byType(Opacity)).first,
          )
          .opacity;
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      addTearDown(mouse.removePointer);
      await mouse.addPointer(
        location: tester.getCenter(find.text('Fix login')),
      );
      await tester.pumpAndSettle();
      expect(menuOpacity(), 1);
      await mouse.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(menuOpacity(), 0, reason: 'the pointer left the row');
    },
  );

  testWidgets('a folder the system cannot open is reported', (tester) async {
    await show(tester, threads: [threadOf('thread-1', title: 'Fix login')]);
    await state.open('thread-1');
    await tester.pumpAndSettle();
    Future<void> exportRecord() async {
      await tap(tester, find.text('Fix login'), buttons: kSecondaryButton);
      await tap(tester, find.text('Export record'));
    }

    runOpener = (program, arguments) async =>
        ProcessResult(0, 1, '', 'no handler');
    await exportRecord();
    expect(find.textContaining('no handler'), findsOneWidget);

    runOpener = (program, arguments) async =>
        throw ProcessException(program, arguments, 'missing');
    await exportRecord();
    expect(find.textContaining('missing'), findsOneWidget);
  });
}
