import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/agent/session_card.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/icons.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeProject project;
  late SessionState state;
  var sequence = 0;

  Future<void> show(
    WidgetTester tester, {
    List<BridgeTranscriptItem> transcript = const [],
    List<BridgeTurn> turns = const [],
    List<BridgeRequest> requests = const [],
    Size size = const Size(1600, 900),
  }) async {
    await tester.binding.setSurfaceSize(size);
    sequence = 0;
    project = FakeProject();
    project.session
      ..current = snapshotOf(
        transcript: transcript,
        turns: turns,
        requests: requests,
      )
      ..turnList = turns
      ..chosen = choiceOf('claude-opus-5-5')
      ..sessionTitle = 'Login work';
    state = SessionState(project);
    await state.open('thread-1');
    await pumpApp(
      tester,
      AgentShell(state: state, info: info, parseDiff: parsePatch),
    );
  }

  Future<void> emit(
    WidgetTester tester,
    BridgeEvent Function(int) build,
  ) async {
    project.session.emit(build(++sequence));
    await tester.pumpAndSettle();
  }

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  String composerText(WidgetTester tester) => tester
      .widget<TextField>(find.byKey(const Key('composer')))
      .controller!
      .text;

  Future<void> typeInComposer(WidgetTester tester, String text) async {
    await tester.enterText(find.byKey(const Key('composer')), text);
    await tester.pumpAndSettle();
  }

  List<String> actions() => [
    for (final call in project.session.calls)
      if (!call.startsWith('changes')) call,
  ];

  Future<TestGesture> hover(WidgetTester tester, Finder finder) async {
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(finder));
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
    return mouse;
  }

  Future<void> chord(
    WidgetTester tester,
    LogicalKeyboardKey key, {
    bool option = false,
  }) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    if (option) await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(key);
    if (option) await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  BridgeTurn running(int number) =>
      turnOf(number, status: BridgeTurnStatus.running);

  List<BridgeTranscriptItem> researching() => [
    chatItem(
      item(
        1,
        BridgeItemKind.taskList,
        steps: [
          step('Plan research', BridgeStepStatus.completed),
          step('Explore', BridgeStepStatus.running),
          step('Explore › scraper-1', BridgeStepStatus.running),
          step('Explore › scraper-2', BridgeStepStatus.completed),
          step('Write report', BridgeStepStatus.pending),
        ],
      ),
      1,
      '',
    ),
  ];

  testWidgets('the header names the session, and a reply streams '
      'in', (tester) async {
    await show(tester);
    expect(find.text('Login work'), findsWidgets);
    expect(find.text('acme-api'), findsWidgets);
    expect(find.text('Claude Opus 5.5'), findsWidgets);
    await emit(
      tester,
      (seq) => event(
        BridgeEventKind.itemStarted,
        seq,
        turn: 1,
        item: item(7, BridgeItemKind.assistantMessage),
      ),
    );
    for (final text in ['Hel', 'lo']) {
      await emit(
        tester,
        (seq) => event(
          BridgeEventKind.contentDelta,
          seq,
          item: item(7, BridgeItemKind.assistantMessage),
          text: text,
        ),
      );
    }
    expectOne(find.text('Hello'));
  });

  testWidgets('Submit sends and clears the composer, and a held Enter sends '
      'nothing', (tester) async {
    await show(tester);
    expect(
      tester
          .widget<PrimaryButton>(
            find.widgetWithText(PrimaryButton, 'Submit message'),
          )
          .onPressed,
      isNull,
      reason: 'nothing to send yet',
    );
    await typeInComposer(tester, 'Fix login');
    await tap(tester, find.text('Submit message'));
    expect(actions(), ['send Fix login']);
    expect(composerText(tester), isEmpty);

    await show(tester, turns: [running(1)]);
    await typeInComposer(tester, 'Use the faster API');
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(composerText(tester), 'Use the faster API');
    expect(actions(), isEmpty);
  });

  testWidgets('a send that fails keeps the typed prompt in the composer', (
    tester,
  ) async {
    await show(tester);
    await chord(tester, LogicalKeyboardKey.keyN);
    state.model.value = null;
    await typeInComposer(tester, 'Hello agent');
    await tap(tester, find.text('Submit message'));
    expectOne(find.text('No model is available.'));
    expect(composerText(tester), 'Hello agent');
  });

  testWidgets('while the agent runs, Enter steers it, ⌥⏎ queues a follow-up, '
      'and Stop generation stops it', (tester) async {
    await show(tester, turns: [running(1)]);
    await typeInComposer(tester, 'Use the faster API');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(actions(), ['steer Use the faster API']);

    await typeInComposer(tester, 'Add a test');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.pumpAndSettle();
    expect(actions().last, 'send Add a test');

    await tap(tester, find.text('Stop generation'));
    expect(actions(), ['steer Use the faster API', 'send Add a test', 'stop']);
  });

  testWidgets('a queued message shows its place, Edit moves it to the '
      'composer, and it can be sent now or removed', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      turns: [
        running(1),
        turnOf(2, status: BridgeTurnStatus.queued, prompt: 'Add a test'),
      ],
    );
    expectOne(find.text('Queued message 1 of 1, not sent'));

    await tap(tester, find.text('Edit'));
    expect(composerText(tester), 'Add a test');
    await tap(tester, find.text('Cancel'));
    expectNone(find.text('Editing in the composer'));
    await tap(tester, find.text('Edit'));
    await typeInComposer(tester, 'Add two tests');
    await tap(tester, find.text('Update queued message'));
    expect(actions().last, 'edit 2 Add two tests');

    await tap(tester, find.text('Send now'));
    expect(actions().last, 'send now 2');
    await tap(tester, find.byTooltip('Remove from queue'));
    expect(actions().last, 'cancel 2');
  });

  testWidgets('a queued message that starts while it is edited is sent as a '
      'new message', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      turns: [
        running(1),
        turnOf(2, status: BridgeTurnStatus.queued, prompt: 'Add a test'),
      ],
    );
    await tap(tester, find.text('Edit'));
    project.session.turnList = [running(1), running(2)];
    await emit(
      tester,
      (seq) => event(BridgeEventKind.turnStarted, seq, turn: 2),
    );
    await typeInComposer(tester, 'Add a fix');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(actions().last, 'steer Add a fix');
  });

  testWidgets('Edit from here rewinds the chat and returns the prompt, and '
      'Revert files too also undoes the turn’s files', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login')],
      turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
    );
    project.session.revertedPrompt = 'Fix login';
    await hover(tester, find.text('Fix login'));
    await tap(tester, find.text('Edit from here'));
    expectOne(find.text('Edit from here?'));
    expect(
      find.text('Revert files too'),
      findsOneWidget,
      reason: 'the checkpoint is ready',
    );

    await tap(tester, find.text('Revert and keep changes'));
    expect(actions(), ['revert 1 files=false']);
    expect(composerText(tester), 'Fix login');

    await tap(tester, find.text('Edit from here'));
    await tap(tester, find.text('Revert files too'));
    await tap(tester, find.text('Revert files to before this turn…'));
    expectOne(find.text('Edit from here?'));
    await tap(tester, find.text('Revert files too'));
    expect(actions(), [
      'revert 1 files=false',
      'revert 1 files=true',
      'revert 1 files=true',
    ]);
  });

  testWidgets('Revert files too is offered only once the checkpoint is ready', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login')],
      turns: [turnOf(1, prompt: 'Fix login', checkpoint: false)],
    );
    await hover(tester, find.text('Fix login'));
    await tap(tester, find.text('Edit from here'));
    expectNone(find.text('Revert files too'));
    await tap(tester, find.text('Revert and keep changes'));
    expect(actions(), ['revert 1 files=false']);
  });

  testWidgets('Edit from here restores the prompt before what was typed '
      'meanwhile', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login')],
      turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
    );
    project.session.revertedPrompt = 'Fix login';
    await hover(tester, find.text('Fix login'));
    await typeInComposer(tester, 'Also add a test');
    await tap(tester, find.text('Edit from here'));
    await tap(tester, find.text('Revert and keep changes'));
    expect(composerText(tester), 'Fix login\n\nAlso add a test');

    await typeInComposer(tester, 'Fix login');
    await tap(tester, find.text('Edit from here'));
    await tap(tester, find.text('Revert and keep changes'));
    expect(composerText(tester), 'Fix login', reason: 'not doubled');
  });

  testWidgets('a turn ends with its changes and Open diff; Fork from this '
      'response opens the new session it returns', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login'), reply(1, 'Done')],
      turns: [turnOf(1, prompt: 'Fix login', files: 2, added: 5, removed: 1)],
    );
    for (final text in [
      '2 changed files',
      '+5',
      '−1',
      'Revert files to before this turn…',
    ]) {
      expectOne(find.text(text));
    }
    await tap(tester, find.text('Open diff'));
    expectOne(find.byType(DiffPanel));

    await show(
      tester,
      transcript: [userMessage(1, 'Fix login'), reply(1, 'Done')],
      turns: [turnOf(1, prompt: 'Fix login')],
    );
    await hover(tester, find.text('Done'));
    await tester.tap(find.text('Fork from this response'));
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 10)),
    );
    await tester.pumpAndSettle();
    expect(actions(), ['fork 1']);
    expect(project.calls, contains('open fork-of-thread-1'));
  });

  testWidgets('an approval names its kind and warning, Approve answers it, '
      'and More offers the options no button shows', (tester) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      requests: [
        approvalOf(
          'a1',
          prompt: 'rm -rf build',
          options: const [
            BridgeApprovalOption(decision: 'decline', label: 'Decline'),
            BridgeApprovalOption(
              decision: 'accept',
              label: 'Approve',
              warning: 'Runs outside the project',
            ),
          ],
        ),
      ],
    );
    expectOne(find.text('Command approval'));
    expectOne(find.text('rm -rf build'));
    expectOne(find.text('Waiting for approval'));
    expectOne(find.byTooltip('Runs outside the project'));
    await tap(tester, find.text('Approve'));
    expect(actions(), ['respond a1 accept']);

    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      requests: [approvalOf('a1')],
    );
    await tap(tester, find.text('More'));
    await tap(tester, find.text('Always allow this session'));
    expect(actions(), ['respond a1 acceptForSession']);
  });

  testWidgets('questions step through their answers, Previous returns to an '
      'answered one, and they submit together', (tester) async {
    await show(
      tester,
      requests: [
        questionsOf('q1', [
          const BridgeQuestion(
            id: 'scope',
            header: 'scope',
            question: 'Which scope?',
            options: ['Small', 'Large'],
            descriptions: ['Only the login flow', ''],
            multiSelect: false,
            custom: true,
          ),
          questionOf(
            'style',
            'Which styles?',
            options: ['Plain', 'Bold'],
            multiSelect: true,
          ),
        ]),
      ],
    );
    await read(tester);

    await tester.tap(find.text('Small'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expectOne(find.text('2/2'));
    expectOne(find.text('Which styles?'));

    await tap(tester, find.text('Previous'));
    expectOne(find.text('Which scope?'));
    expectOne(find.text('Only the login flow'));

    await tester.tap(find.text('Small'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Plain'));
    await tester.tap(find.text('Bold'));
    await tester.pumpAndSettle();
    await tap(tester, find.text('Submit answers'));
    expect(actions(), ['answer q1 scope=Small style=Plain+Bold']);
  });

  testWidgets('leaving a question cancels its delayed advance', (tester) async {
    await show(
      tester,
      requests: [
        questionsOf('q1', [
          questionOf('scope', 'Which scope?', options: ['Small', 'Large']),
          questionOf(
            'style',
            'Which styles?',
            options: ['Plain', 'Bold'],
            multiSelect: true,
          ),
        ]),
      ],
    );
    await read(tester);
    await tester.tap(find.text('Small'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    await tap(tester, find.text('Plain'));
    await tap(tester, find.text('Previous'));
    await tester.tap(find.text('Small'));
    await tester.pump();
    await tester.tap(find.text('Next question'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expectOne(find.text('Which styles?'));
    expect(actions(), isEmpty);
  });

  testWidgets('a question can be dismissed without answering it', (
    tester,
  ) async {
    await show(
      tester,
      requests: [
        questionsOf('q1', [questionOf('name', 'Your name')]),
      ],
    );
    await tap(tester, find.byTooltip('Dismiss question without answering'));
    expect(actions(), isEmpty, reason: 'too early to answer');
    await read(tester);
    await tap(tester, find.byTooltip('Dismiss question without answering'));
    expect(actions(), ['dismiss q1']);
  });

  testWidgets('⌘L focuses the composer, and ⌘N starts a new '
      'session', (tester) async {
    await show(tester, transcript: [userMessage(1, 'Fix login')]);
    expect(focused(tester, const Key('composer')), isFalse);
    await chord(tester, LogicalKeyboardKey.keyL);
    expect(focused(tester, const Key('composer')), isTrue);
    await chord(tester, LogicalKeyboardKey.keyN);
    expectNone(find.text('Fix login'));
  });

  testWidgets('a running instance can be messaged from the task tree', (
    tester,
  ) async {
    await show(
      tester,
      transcript: researching(),
      turns: [turnOf(1, status: BridgeTurnStatus.running)],
    );
    expectOne(find.text('Tasks 2/5 · Explore'));
    expectNone(find.text('scraper-1'));

    await tap(tester, find.text('Tasks 2/5 · Explore'));
    expectOne(find.text('Plan research'));
    expectOne(find.text('scraper-2'));
    expectOne(find.text('Pending'));
    expect(find.text('Completed'), findsNWidgets(2));
    expectNone(find.text('Message'));

    final mouse = await hover(tester, find.text('scraper-1'));
    await tap(tester, find.text('Message'));
    expect(composerText(tester), '@scraper-1 ');

    await typeInComposer(tester, '@scraper-2 Look at Dart');
    await mouse.moveTo(tester.getCenter(find.text('Plan research')));
    await mouse.moveTo(tester.getCenter(find.text('scraper-1')));
    await tester.pumpAndSettle();
    await tap(tester, find.text('Message'));
    expect(composerText(tester), '@scraper-1 Look at Dart');
  });

  testWidgets('a running instance is chosen by @ in the composer', (
    tester,
  ) async {
    await show(
      tester,
      transcript: researching(),
      turns: [turnOf(1, status: BridgeTurnStatus.running)],
    );
    await typeInComposer(tester, '@scr');
    expectOne(find.text('Running instances'));
    expectOne(find.text('scraper-1'));
    expect(find.text('scraper-2'), findsNothing, reason: 'it has finished');

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(composerText(tester), '@scraper-1 ');
    expectNone(find.text('Running instances'));
    expect(
      actions().where((call) => call.startsWith(RegExp('send|steer'))),
      isEmpty,
    );

    await typeInComposer(tester, 'mail @scr');
    expectNone(find.text('Running instances'));
  });

  testWidgets('reasoning, tool calls and errors each show as their '
      'row', (tester) async {
    await show(
      tester,
      transcript: [
        chatItem(
          item(1, BridgeItemKind.reasoning, ended: true),
          1,
          'Weighing the options',
        ),
        chatItem(
          item(
            2,
            BridgeItemKind.commandExecution,
            tool: 'Bash',
            target: 'flutter test',
            ended: true,
            durationMs: 1234,
          ),
          1,
          '',
        ),
        chatItem(
          item(
            3,
            BridgeItemKind.fileChange,
            tool: 'Edit',
            target: 'lib/login.dart',
            ended: true,
            failed: true,
          ),
          1,
          '',
        ),
        chatItem(
          item(
            4,
            BridgeItemKind.dynamicToolCall,
            tool: 'Read',
            target: 'README.md',
          ),
          1,
          '',
        ),
        chatItem(
          item(5, BridgeItemKind.error, ended: true),
          1,
          '{"error":{"message":"Rate limited"}}',
        ),
        chatItem(
          item(6, BridgeItemKind.error, ended: true),
          1,
          'Plain failure',
        ),
      ],
    );
    expectOne(find.text('Thinking'));
    expectOne(find.textContaining('flutter test'));
    expectOne(find.text('1.2 s'));
    expectOne(find.byIcon(LucideIcons.check));
    expectOne(find.byIcon(LucideIcons.x));
    expectOne(find.byIcon(LucideIcons.loader));
    expectOne(find.text('Rate limited'));
    expectOne(find.text('Plain failure'));
  });

  testWidgets('notices show in the status bar until cleared, and their list '
      'closes on a click, outside, esc or a question', (tester) async {
    await show(tester);
    state.notify(NoticeLevel.info, 'Tests passed');
    state.notify(NoticeLevel.warning, 'lint: 2 warnings');
    state.notify(NoticeLevel.error, 'Could not branch: compacting');
    await tester.pumpAndSettle();
    expectOne(find.text('Could not branch: compacting'));
    expectOne(find.text('· 2 more'));
    expectOne(find.byIcon(LucideIcons.triangleAlert));

    final status = find.byKey(const Key('status'));
    Future<void> open() async {
      await tap(tester, status);
      expectOne(find.text('Notifications'));
    }

    await open();
    await tap(tester, find.text('lint: 2 warnings'));
    expect(
      find.byType(SelectableText),
      findsOneWidget,
      reason: 'an opened entry shows its whole text',
    );
    await tap(tester, find.text('Clear all'));
    expectNone(find.text('Notifications'));
    expectOne(find.text('● idle'));

    state.notify(NoticeLevel.warning, 'lint: 2 warnings');
    await open();
    await tap(tester, status);
    expectNone(find.text('Notifications'));

    await open();
    await tester.tapAt(const Offset(800, 200));
    await tester.pumpAndSettle();
    expectNone(find.text('Notifications'));

    await open();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expectNone(find.text('Notifications'));

    Focus.of(tester.element(status)).requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expectOne(find.text('Notifications'));

    await emit(
      tester,
      (seq) =>
          event(BridgeEventKind.requestOpened, seq, request: approvalOf('a1')),
    );
    expectNone(find.text('Notifications'));
  });

  testWidgets('⌥⌘B shows and hides the session card, and in a narrow window '
      'it is a popover that Escape closes', (tester) async {
    await show(tester);
    expectOne(find.byType(SessionCard));
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expectNone(find.byType(SessionCard));
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expectOne(find.byType(SessionCard));

    await show(
      tester,
      size: const Size(900, 900),
      transcript: [userMessage(1, 'Fix login')],
    );
    expectNone(find.byType(SessionCard));
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expectOne(find.byType(SessionCard));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expectNone(find.byType(SessionCard));
  });
}
