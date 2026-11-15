import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/agent/session_card.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_backend.dart';
import 'fake_workspace.dart';

const info = SessionInfo(project: 'acme-api');

/// A disk that is always full.
class _FullDisk extends SettingsFile {
  _FullDisk() : super(File('unused'));

  @override
  Future<void> save(Map<String, Object?> json) async =>
      throw const FileSystemException('disk full');
}

/// T3 as the app stops it: only `shutdown` is answered.
class FakeAgentHost implements AgentHost {
  bool stopped = false;

  @override
  Future<void> shutdown() async {
    stopped = true;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

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
    Size size = const Size(1600, 900),
    AgentHost? host,
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
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: info,
          host: host,
          parseDiff: parsePatch,
        ),
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

  /// Presses `key` with ⌘ held, and with ⌥ too when `option` is set.
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

  /// Records what the app copies to the clipboard; the getter returns the last copy.
  String? Function() recordClipboard(WidgetTester tester) {
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
    return () => copied;
  }

  testWidgets('Copy message copies the message text', (tester) async {
    final copied = recordClipboard(tester);
    await show(tester, transcript: [userMessage(1, 'Fix login')]);
    await hover(tester, find.text('Fix login'));
    await tester.tap(find.text('Copy message'));
    await tester.pumpAndSettle();
    expect(copied(), 'Fix login');
  });

  testWidgets('Copy message copies a reply too', (tester) async {
    final copied = recordClipboard(tester);
    await show(tester, transcript: [reply(1, 'Done')]);
    await hover(tester, find.text('Done'));
    await tester.tap(find.text('Copy message'));
    await tester.pumpAndSettle();
    expect(copied(), 'Done');
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

  testWidgets('a turn\'s revert link asks before it reverts the files', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [userMessage(1, 'Fix login')],
      turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
    );
    await tester.tap(find.text('Revert files to before this turn…'));
    await tester.pumpAndSettle();
    expect(find.text('Edit from here?'), findsOneWidget);
    await tester.tap(find.text('Revert files too'));
    await tester.pumpAndSettle();
    expect(actions(), ['revert 1 files=true']);
  });

  testWidgets('an approval names its kind and explains a warned option', (
    tester,
  ) async {
    for (final (kind, title) in [
      ('file_change', 'File change approval'),
      ('file-read', 'File read approval'),
      ('permission', 'App permission approval'),
      ('mcp-elicitation', 'App access approval'),
      ('other', 'Approval'),
    ]) {
      await show(
        tester,
        transcript: [userMessage(1, 'Run the build')],
        requests: [approvalOf('a1', kind: kind)],
      );
      expect(find.text(title), findsOneWidget, reason: kind);
    }
    await show(
      tester,
      transcript: [userMessage(1, 'Run the build')],
      requests: [
        approvalOf(
          'a1',
          options: const [
            BridgeApprovalOption(
              decision: 'accept',
              label: 'Approve',
              warning: 'Runs outside the project',
            ),
          ],
        ),
      ],
    );
    expect(find.byTooltip('Runs outside the project'), findsOneWidget);
  });

  testWidgets('reasoning, tool calls and errors each show as their row', (
    tester,
  ) async {
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
    expect(find.text('Thinking'), findsOneWidget, reason: 'not its text');
    expect(find.textContaining('flutter test'), findsOneWidget);
    expect(find.text('1.2 s'), findsOneWidget);
    expect(find.byIcon(LucideIcons.check), findsOneWidget);
    expect(find.byIcon(LucideIcons.x), findsOneWidget);
    expect(find.byIcon(LucideIcons.loader), findsOneWidget, reason: 'running');
    expect(find.text('Rate limited'), findsOneWidget, reason: 'a JSON message');
    expect(find.text('Plain failure'), findsOneWidget);
  });

  testWidgets('a turn asked for scrolls the Chat to its first item', (
    tester,
  ) async {
    await show(
      tester,
      transcript: [
        for (var turn = 1; turn <= 40; turn++)
          userMessage(turn, 'Prompt $turn'),
      ],
    );
    state.scrollChatTo.value = 2;
    await tester.pumpAndSettle();
    expect(find.text('Prompt 2'), findsOneWidget);
  });

  testWidgets('notices show in the status bar and list until cleared', (
    tester,
  ) async {
    await show(tester);
    state.notify(NoticeLevel.info, 'Tests passed');
    state.notify(NoticeLevel.warning, 'lint: 2 warnings');
    state.notify(NoticeLevel.error, 'Could not branch: compacting');
    await tester.pumpAndSettle();
    expect(find.text('Could not branch: compacting'), findsOneWidget);
    expect(find.text('· 2 more'), findsOneWidget);
    expect(find.byIcon(LucideIcons.triangleAlert), findsOneWidget);

    await tester.tap(find.byKey(const Key('status')));
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsOneWidget);
    await tester.tap(find.text('lint: 2 warnings'));
    await tester.pumpAndSettle();
    expect(
      find.byType(SelectableText),
      findsOneWidget,
      reason: 'an opened entry shows its whole text',
    );
    await tester.tap(find.text('Clear all'));
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsNothing);
    expect(find.text('● idle'), findsOneWidget);
  });

  testWidgets(
    'the notification list closes on a click, a click elsewhere, esc or a question',
    (tester) async {
      await show(tester);
      state.notify(NoticeLevel.warning, 'lint: 2 warnings');
      final status = find.byKey(const Key('status'));
      Future<void> open() async {
        await tester.tap(status);
        await tester.pumpAndSettle();
        expect(find.text('Notifications'), findsOneWidget);
      }

      await open();
      await tester.tap(status);
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsNothing, reason: 'a click');

      await open();
      await tester.tapAt(const Offset(800, 200));
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsNothing, reason: 'outside');

      await open();
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsNothing, reason: 'esc');

      Focus.of(tester.element(status)).requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(find.text('Notifications'), findsOneWidget, reason: 'by keys');

      await emit(
        tester,
        (seq) => event(
          BridgeEventKind.requestOpened,
          seq,
          request: approvalOf('a1'),
        ),
      );
      expect(
        find.text('Notifications'),
        findsNothing,
        reason: 'it would cover the request',
      );
    },
  );

  testWidgets(
    'Edit moves a queued message into the composer; Cancel or Update settles it',
    (tester) async {
      await show(
        tester,
        transcript: [userMessage(1, 'Run the build')],
        turns: [
          running(1),
          turnOf(2, status: BridgeTurnStatus.queued, prompt: 'Add a test'),
        ],
      );
      await tester.tap(find.text('Edit'));
      await tester.pumpAndSettle();
      expect(composerText(tester), 'Add a test');
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Editing in the composer'), findsNothing);

      await tester.tap(find.text('Edit'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('composer')),
        'Add two tests',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Update queued message'));
      await tester.pumpAndSettle();
      expect(actions().last, 'edit 2 Add two tests');
    },
  );

  testWidgets('a held Enter types no line and sends nothing', (tester) async {
    await show(tester, turns: [running(1)]);
    await tester.enterText(
      find.byKey(const Key('composer')),
      'Use the faster API',
    );
    await tester.pumpAndSettle();
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(composerText(tester), 'Use the faster API');
    expect(actions(), isEmpty);
  });

  testWidgets(
    'Previous returns to an answered question, which keeps its answer',
    (tester) async {
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
            questionOf('style', 'Which styles?', options: ['Plain', 'Bold']),
          ]),
        ],
      );
      await read(tester);
      await tester.tap(find.text('Small'));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expect(find.text('Which styles?'), findsOneWidget);

      await tester.tap(find.text('Previous'));
      await tester.pumpAndSettle();
      expect(find.text('Which scope?'), findsOneWidget);
      expect(find.text('Only the login flow'), findsOneWidget);
    },
  );

  testWidgets('⌥⌘B hides the session card and shows it again', (tester) async {
    await show(tester);
    expect(find.byType(SessionCard), findsOneWidget, reason: 'shown at first');
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expect(find.byType(SessionCard), findsNothing);
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expect(find.byType(SessionCard), findsOneWidget);
  });

  testWidgets('in a narrow window the card is a popover that Escape closes', (
    tester,
  ) async {
    await show(
      tester,
      size: const Size(900, 900),
      transcript: [userMessage(1, 'Fix login')],
    );
    expect(find.byType(SessionCard), findsNothing, reason: 'waits closed');
    await chord(tester, LogicalKeyboardKey.keyB, option: true);
    expect(find.byType(SessionCard), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(SessionCard), findsNothing);
  });

  testWidgets('⌘N starts a new session in the Chat', (tester) async {
    await show(tester, transcript: [userMessage(1, 'Fix login')]);
    await chord(tester, LogicalKeyboardKey.keyN);
    expect(find.text('Fix login'), findsNothing);
  });

  testWidgets(
    'Edit from here restores the prompt before what was typed meanwhile',
    (tester) async {
      await show(
        tester,
        transcript: [userMessage(1, 'Fix login')],
        turns: [turnOf(1, prompt: 'Fix login', files: 1, added: 1)],
      );
      project.session.revertedPrompt = 'Fix login';
      await hover(tester, find.text('Fix login'));
      await tester.enterText(
        find.byKey(const Key('composer')),
        'Also add a test',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Edit from here'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Revert and keep changes'));
      await tester.pumpAndSettle();
      expect(composerText(tester), 'Fix login\n\nAlso add a test');

      await tester.enterText(find.byKey(const Key('composer')), 'Fix login');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Edit from here'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Revert and keep changes'));
      await tester.pumpAndSettle();
      expect(composerText(tester), 'Fix login', reason: 'not doubled');
    },
  );

  testWidgets('quitting stops T3 before Liquid exits', (tester) async {
    final host = FakeAgentHost();
    await show(tester, host: host);
    await tester.binding.handleRequestAppExit();
    expect(host.stopped, isTrue);
  });

  testWidgets('without a session, a failed save is reported to the Workspace', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    final workspace = WorkspaceState(FakeWorkspace());
    await workspace.start();
    addTearDown(workspace.dispose);
    final diff = DiffSettingsController(const DiffSettings(), _FullDisk());
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(workspace: workspace, diffSettings: diff),
      ),
    );
    await tester.pumpAndSettle();
    await diff.change(const DiffSettings(layout: DiffLayout.split));
    expect(
      workspace.problem.value,
      'Could not save the Diff settings: disk full',
    );
  });
}
