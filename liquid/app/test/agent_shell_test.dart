import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

void main() {
  late FakeProject project;
  late SessionState state;
  var sequence = 0;

  /// Shows an open session whose chat, turns and requests are `transcript`, `turns` and
  /// `requests`, as Meta-Harness's snapshot holds them. Events sent later are numbered after it.
  Future<void> show(
    WidgetTester tester, {
    List<BridgeTranscriptItem> transcript = const [],
    List<BridgeTurn> turns = const [],
    List<BridgeRequest> requests = const [],
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
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
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, parseDiff: parsePatch),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// Sends a live event of the open session and lets the Chat show it.
  Future<void> emit(
    WidgetTester tester,
    BridgeEvent Function(int) build,
  ) async {
    project.session.emit(build(++sequence));
    await tester.pumpAndSettle();
  }

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

  /// Lets a question take clicks and keys, as it does 0.5 s after it could be read.
  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  String composerText(WidgetTester tester) => tester
      .widget<TextField>(find.byKey(const Key('composer')))
      .controller!
      .text;

  /// Types `text` into the composer and sends it with the button that shows then.
  Future<void> typeAndSend(
    WidgetTester tester,
    String text,
    String button,
  ) async {
    await tester.enterText(find.byKey(const Key('composer')), text);
    await tester.pumpAndSettle();
    await tester.tap(find.text(button));
    await tester.pumpAndSettle();
  }

  /// What the session was asked to do, without the Diff's reads of the changes.
  List<String> actions() => [
    for (final call in project.session.calls)
      if (!call.startsWith('changes')) call,
  ];

  /// Points at `finder`, which shows a message's actions while the pointer is over it.
  Future<void> hover(WidgetTester tester, Finder finder) async {
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(finder));
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
  }

  /// Gives the session's Chat a turn in progress.
  BridgeTurn running(int number) =>
      turnOf(number, status: BridgeTurnStatus.running);

  BridgeTranscriptItem userMessage(int turn, String text) => chatItem(
    item(turn * 10 + 1, BridgeItemKind.userMessage, ended: true),
    turn,
    text,
  );

  BridgeTranscriptItem reply(int turn, String text) => chatItem(
    item(turn * 10 + 2, BridgeItemKind.assistantMessage, ended: true),
    turn,
    text,
  );

  testWidgets('the header names the session, its project and its model', (
    tester,
  ) async {
    await show(tester);
    expect(find.text('Login work'), findsWidgets);
    expect(find.text('acme-api'), findsWidgets);
    expect(find.text('Claude Opus 5.5'), findsWidgets);
  });

  testWidgets('a streamed reply appears in the chat as it arrives', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      (seq) => event(
        BridgeEventKind.itemStarted,
        seq,
        turn: 1,
        item: item(7, BridgeItemKind.assistantMessage),
      ),
    );
    await emit(
      tester,
      (seq) => event(
        BridgeEventKind.contentDelta,
        seq,
        item: item(7, BridgeItemKind.assistantMessage),
        text: 'Hel',
      ),
    );
    await emit(
      tester,
      (seq) => event(
        BridgeEventKind.contentDelta,
        seq,
        item: item(7, BridgeItemKind.assistantMessage),
        text: 'lo',
      ),
    );
    expect(find.text('Hello'), findsOneWidget);
  });

  testWidgets('Submit message sends the composer text and clears it', (
    tester,
  ) async {
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
    await typeAndSend(tester, 'Fix login', 'Submit message');
    expect(actions(), ['send Fix login']);
    expect(composerText(tester), isEmpty);
  });

  testWidgets(
    'while the agent runs, Enter steers it and ⌥⏎ queues a follow-up',
    (tester) async {
      await show(tester, turns: [running(1)]);
      await tester.enterText(
        find.byKey(const Key('composer')),
        'Use the faster API',
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(actions(), ['steer Use the faster API']);

      await tester.enterText(find.byKey(const Key('composer')), 'Add a test');
      await tester.pumpAndSettle();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
      expect(actions().last, 'send Add a test');
    },
  );

  testWidgets(
    'Stop generation replaces the send button when nothing is typed',
    (tester) async {
      await show(tester, turns: [running(1)]);
      await tester.tap(find.text('Stop generation'));
      await tester.pumpAndSettle();
      expect(actions(), ['stop']);
    },
  );

  testWidgets(
    'a queued message shows its place and can be sent now or removed',
    (tester) async {
      await show(
        tester,
        turns: [
          running(1),
          turnOf(2, status: BridgeTurnStatus.queued, prompt: 'Add a test'),
        ],
      );
      expect(find.text('Queued message 1 of 1, not sent'), findsOneWidget);
      await tester.tap(find.text('Send now'));
      await tester.pumpAndSettle();
      expect(actions(), ['send now 2']);

      await tester.tap(find.byTooltip('Remove from queue'));
      await tester.pumpAndSettle();
      expect(actions().last, 'cancel 2');
    },
  );

  testWidgets(
    'Edit from here rewinds the chat and returns the prompt to the composer',
    (tester) async {
      await show(
        tester,
        transcript: [userMessage(1, 'Fix login')],
        turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
      );
      project.session.revertedPrompt = 'Fix login';
      await hover(tester, find.text('Fix login'));
      await tester.tap(find.text('Edit from here'));
      await tester.pumpAndSettle();
      expect(find.text('Edit from here?'), findsOneWidget);
      expect(
        find.text('Revert files too'),
        findsOneWidget,
        reason: 'the checkpoint is ready',
      );

      await tester.tap(find.text('Revert and keep changes'));
      await tester.pumpAndSettle();
      expect(actions(), ['revert 1 files=false']);
      expect(composerText(tester), 'Fix login');
    },
  );

  testWidgets(
    'Revert files too is offered only once the turn\'s checkpoint is ready',
    (tester) async {
      await show(
        tester,
        transcript: [userMessage(1, 'Fix login')],
        turns: [turnOf(1, prompt: 'Fix login', checkpoint: false)],
      );
      await hover(tester, find.text('Fix login'));
      await tester.tap(find.text('Edit from here'));
      await tester.pumpAndSettle();
      expect(find.text('Revert files too'), findsNothing);

      await tester.tap(find.text('Revert and keep changes'));
      await tester.pumpAndSettle();
      expect(actions(), ['revert 1 files=false']);
    },
  );

  testWidgets('Revert files too also undoes the files of the turn', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login')],
      turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
    );
    await hover(tester, find.text('Fix login'));
    await tester.tap(find.text('Edit from here'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Revert files too'));
    await tester.pumpAndSettle();
    expect(actions(), ['revert 1 files=true']);
  });

  testWidgets('a turn ends with its changes, Open diff and the revert', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login'), reply(1, 'Done')],
      turns: [turnOf(1, prompt: 'Fix login', files: 2, added: 5, removed: 1)],
    );
    expect(find.text('2 changed files'), findsOneWidget);
    expect(find.text('+5'), findsOneWidget);
    expect(find.text('−1'), findsOneWidget);
    expect(find.text('Revert files to before this turn…'), findsOneWidget);

    await tester.tap(find.text('Open diff'));
    await tester.pumpAndSettle();
    expect(find.byType(DiffPanel), findsOneWidget);
  });

  testWidgets('Fork from this response opens the new session it returns', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login'), reply(1, 'Done')],
      turns: [turnOf(1, prompt: 'Fix login')],
    );
    await hover(tester, find.text('Done'));
    await tester.tap(find.text('Fork from this response'));
    // Opening replaces the attached session, which waits on the real event loop.
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 10)),
    );
    await tester.pumpAndSettle();
    expect(actions(), ['fork 1']);
    expect(project.calls, contains('open fork-of-thread-1'));
  });

  testWidgets('Copy message copies the message text', (tester) async {
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String?;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    await show(tester, transcript: [userMessage(1, 'Fix login')]);
    await hover(tester, find.text('Fix login'));
    await tester.tap(find.text('Copy message'));
    await tester.pumpAndSettle();
    expect(copied, 'Fix login');
  });

  testWidgets('a command approval shows what it runs and Approve answers it', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      requests: [
        approvalOf(
          'a1',
          prompt: 'rm -rf build',
          options: const [
            BridgeApprovalOption(decision: 'decline', label: 'Decline'),
            BridgeApprovalOption(decision: 'accept', label: 'Approve'),
          ],
        ),
      ],
    );
    expect(find.text('Command approval'), findsOneWidget);
    expect(find.text('rm -rf build'), findsOneWidget);
    expect(find.text('Waiting for approval'), findsOneWidget);

    await tester.tap(find.text('Approve'));
    await tester.pumpAndSettle();
    expect(actions(), ['respond a1 accept']);
  });

  testWidgets('More lists the approval options that no button shows', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      requests: [approvalOf('a1')],
    );
    await tester.tap(find.text('More'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Always allow this session'));
    await tester.pumpAndSettle();
    expect(actions(), ['respond a1 acceptForSession']);
  });

  testWidgets('questions step through their answers and submit them together', (
    tester,
  ) async {
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
    expect(find.text('2/2'), findsOneWidget);
    await tester.tap(find.text('Plain'));
    await tester.tap(find.text('Bold'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Submit answers'));
    await tester.pumpAndSettle();
    expect(actions(), ['answer q1 scope=Small style=Plain+Bold']);
  });

  testWidgets('Dismiss question without answering sends the dismissal', (
    tester,
  ) async {
    await show(
      tester,
      requests: [
        questionsOf('q1', [questionOf('name', 'Your name')]),
      ],
    );
    await tester.tap(find.byTooltip('Dismiss question without answering'));
    await tester.pumpAndSettle();
    expect(actions(), isEmpty, reason: 'too early to answer');

    await read(tester);
    await tester.tap(find.byTooltip('Dismiss question without answering'));
    await tester.pumpAndSettle();
    expect(actions(), ['dismiss q1']);
  });

  testWidgets('a notice shows in the status bar', (tester) async {
    await show(tester);
    state.notify(NoticeLevel.error, 'Could not read the sessions: boom');
    await tester.pumpAndSettle();
    expect(find.text('Could not read the sessions: boom'), findsOneWidget);
  });

  testWidgets('⌘L focuses the composer from the Chat', (tester) async {
    await show(tester);
    expect(focused(tester, const Key('composer')), isFalse);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pump();
    await tester.pumpAndSettle();
    expect(focused(tester, const Key('composer')), isTrue);
  });
}
