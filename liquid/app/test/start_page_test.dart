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

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', path: '/work/acme-api');

void main() {
  late FakeProject project;
  late SessionState state;
  // What the system's opener was asked to show, instead of starting Finder.
  final opened = <String>[];

  setUp(() {
    opened.clear();
    runOpener = (program, arguments) async {
      opened.add(arguments.single);
      return ProcessResult(0, 0, '', '');
    };
  });
  tearDown(() => runOpener = Process.run);

  /// Shows the start page of a project with `providers`, its branches `main` and `develop`, and
  /// its `threads`.
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

  /// Types `text` into the composer and sends it with the start page's button.
  Future<void> sendFirst(WidgetTester tester, String text) async {
    await tester.enterText(find.byKey(const Key('composer')), text);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Submit message'));
    await tester.pumpAndSettle();
  }

  testWidgets('the start page greets with the project and its setup chips', (
    tester,
  ) async {
    await show(tester);
    expect(find.text('What should we build in acme-api?'), findsOneWidget);
    final chips = find.byType(StartChips);
    expect(
      find.descendant(of: chips, matching: find.text('New worktree')),
      findsOneWidget,
      reason: 'a new session starts in a new worktree, as T3 Code does',
    );
    expect(
      find.descendant(of: chips, matching: find.text('From main')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: chips, matching: find.text('Claude Opus 5.5')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: chips, matching: find.text('Full access')),
      findsOneWidget,
    );
  });

  testWidgets('a new worktree starts from the branch picked', (tester) async {
    await show(tester);
    await tester.tap(find.text('From main'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('develop'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('New worktree'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Current checkout'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.byTooltip('Model'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Claude Sonnet 5.5'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('Full access'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('Auto-accept edits'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('High'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Low'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('Rate limit for /login'));
    await tester.pumpAndSettle();
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
    await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Archive thread'));
    await tester.pumpAndSettle();
    expect(project.calls, contains('archive thread-2 true'));

    await tester.tap(find.text('Run tests'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Archive thread'));
    await tester.pumpAndSettle();
    expect(project.calls, isNot(contains('archive thread-3 true')));
  });

  testWidgets('cancelling a delete deletes nothing', (tester) async {
    await show(tester, threads: [threadOf('thread-2', title: 'Fix login')]);
    await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete'));
    await tester.pumpAndSettle();
    expect(find.text('Delete thread "Fix login"?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
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
      await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Confirm'));
      await tester.pumpAndSettle();
      expect(find.text('Delete the worktree too?'), findsOneWidget);
      expect(
        find.text(
          'This thread is the only one linked to this worktree:\n/worktrees/fix',
        ),
        findsOneWidget,
      );

      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
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
    await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    expect(project.calls, contains('delete thread-2 worktree=true'));
  });

  testWidgets(
    'a thread exports its record into Downloads and shows the folder',
    (tester) async {
      await show(tester, threads: [threadOf('thread-1', title: 'Fix login')]);
      await state.open('thread-1');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Export record'));
      await tester.pumpAndSettle();
      expect(project.session.calls.single, startsWith('export '));
      expect(opened.single, startsWith('/Downloads/'));
    },
  );

  testWidgets('Settings picks the writer model for the project', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Writer model'));
    await tester.pumpAndSettle();
    expect(find.byType(WriterModelPane), findsOneWidget);

    await tester.tap(find.text('Choose model'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Claude Sonnet 5.5'));
    await tester.pumpAndSettle();
    expect(project.calls, contains('writer claude-sonnet-5-5'));
    expect(find.text('Claude Sonnet 5.5'), findsOneWidget);
  });
}
