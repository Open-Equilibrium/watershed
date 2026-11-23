import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/rust/api/agents.dart';
import 'package:liquid/src/rust/api/details.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api');

BridgeAgentChange changeOf(
  int id,
  BridgeAgentChangeKind kind, {
  int turn = 1,
  String thread = 'thread-1',
  String? before,
  String? after,
  BridgeAutomation? routine,
  bool undone = false,
  bool held = false,
}) => BridgeAgentChange(
  id: id,
  thread: thread,
  turn: turn,
  kind: kind,
  before: before,
  after: after,
  routine: routine,
  undone: undone,
  held: held,
);

BridgeAutomation routineOf(String id, String title) => BridgeAutomation(
  id: id,
  title: title,
  schedule: BridgeSchedule(
    kind: BridgeScheduleKind.interval,
    everyMs: 3600000,
    weekdays: Int64List(0),
  ),
  enabled: true,
  lastRun: BridgeRunStatus.never,
);

void main() {
  late FakeProject project;
  late SessionState state;

  Future<void> show(
    WidgetTester tester, {
    List<BridgeAgent> agents = const [],
    List<BridgeThreadSummary> threads = const [],
    List<BridgeTranscriptItem> transcript = const [],
    List<BridgeTurn> turns = const [],
    List<BridgeAgentChange> changes = const [],
    List<BridgeMention> mentions = const [],
    List<BridgeAutomation> automations = const [],
    String? open = 'thread-1',
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    project = FakeProject()
      ..agentList = agents
      ..threadList = threads
      ..changeList = changes
      ..mentionList = mentions
      ..instructionsText = '# news-bot\n\nRead the feeds.\n';
    project.session
      ..current = snapshotOf(transcript: transcript, turns: turns)
      ..turnList = turns
      ..automationList = automations
      ..chosen = choiceOf('gpt-6.1');
    state = SessionState(project);
    await state.loadStart();
    if (open != null) await state.open(open);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, parseDiff: parsePatch),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets(
    'agents are listed above the threads, and their threads only there',
    (tester) async {
      await show(
        tester,
        open: null,
        agents: [
          agentOf(
            'news-bot',
            thread: 'thread-2',
            threads: ['thread-2', 'thread-0'],
            nextRunMs: DateTime.now()
                .add(const Duration(minutes: 5))
                .millisecondsSinceEpoch,
          ),
        ],
        threads: [
          threadOf('thread-2', title: 'news-bot'),
          threadOf('thread-0', title: 'news-bot earlier'),
          threadOf('thread-9', title: 'Fix the move dialog'),
        ],
      );

      final agents = tester.getTopLeft(find.text('Agents'));
      final threads = tester.getTopLeft(find.text('Threads'));
      expect(agents.dy, lessThan(threads.dy));
      expect(find.text('news-bot'), findsOneWidget);
      expect(find.textContaining('next '), findsOneWidget);
      expect(find.text('news-bot earlier'), findsNothing);
      expect(find.text('Fix the move dialog'), findsOneWidget);

      await tester.tap(find.text('news-bot'));
      await tester.pumpAndSettle();
      expect(project.calls, contains('open thread-2'));
    },
  );

  testWidgets('a new agent starts as a Chat whose first message is its job', (
    tester,
  ) async {
    await show(tester, open: null);

    await tester.tap(find.byKey(const Key('new-agent')));
    await tester.pumpAndSettle();
    expect(find.text('What should this agent do?'), findsOneWidget);
    expect(find.text('New agent'), findsOneWidget);

    await tester.tap(find.text('Triage new issues.'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('composer')))
          .controller!
          .text,
      'Triage new issues.',
    );
    await tester.enterText(
      find.byKey(const Key('composer')),
      'Triage new issues.',
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(project.calls, contains('agent start Triage new issues.'));
    expect(project.calls, contains('open thread-agent-1'));
    expect(find.text('What should this agent do?'), findsNothing);
  });

  testWidgets(
    "the agent's own changes show after its turn, with undo and run now",
    (tester) async {
      final routine = routineOf('task-1', 'Morning news');
      await show(
        tester,
        agents: [agentOf('news-bot', thread: 'thread-1')],
        automations: [routine],
        transcript: [
          chatItem(
            item(1, BridgeItemKind.userMessage, ended: true),
            1,
            'Every weekday at 7:30, read our feeds.',
          ),
          chatItem(
            item(2, BridgeItemKind.assistantMessage, ended: true),
            1,
            'Done.',
          ),
        ],
        turns: [turnOf(1, checkpoint: false)],
        changes: [
          changeOf(1, BridgeAgentChangeKind.name, after: 'news-bot'),
          changeOf(
            2,
            BridgeAgentChangeKind.instructions,
            before: '',
            after: '# news-bot\n\nRead the feeds.\n',
          ),
          changeOf(3, BridgeAgentChangeKind.routineAdded, routine: routine),
          changeOf(
            4,
            BridgeAgentChangeKind.instructions,
            turn: 1,
            undone: true,
          ),
        ],
      );

      expect(find.text('Named itself news-bot'), findsOneWidget);
      expect(find.text('Instructions saved · 3 lines'), findsOneWidget);
      expect(
        find.text('Routine added: Morning news · Every 60 min'),
        findsOneWidget,
      );
      expect(find.text('Undone'), findsOneWidget);
      expect(find.byKey(const Key('change-1-Undo')), findsNothing);

      await tester.tap(find.byKey(const Key('change-3-Run now')));
      await tester.pumpAndSettle();
      expect(project.session.calls, contains('run automation task-1'));

      await tester.tap(find.byKey(const Key('change-3-Undo')));
      await tester.pumpAndSettle();
      expect(project.calls, contains('agent undo agent-news-bot 3'));
    },
  );

  testWidgets(
    'the card shows the agent, renames it and adds a webhook routine',
    (tester) async {
      await show(tester, agents: [agentOf('news-bot', thread: 'thread-1')]);

      expect(find.text('Agent'), findsOneWidget);
      expect(find.text('Read the feeds.'), findsOneWidget);
      expect(find.text('Approval required ⌄'), findsOneWidget);

      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('agent-text')),
        'morning-news',
      );
      await tester.tap(find.text('Rename').last);
      await tester.pumpAndSettle();
      expect(
        project.calls,
        contains(
          'agent update agent-news-bot rename=morning-news model=null '
          'permissions=null',
        ),
      );

      await tester.tap(find.byKey(const Key('add-routine')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Webhook'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('routine-prompt')),
        'A feed sent: {{body}}',
      );
      await tester.tap(find.byKey(const Key('routine-add')));
      await tester.pumpAndSettle();
      expect(
        project.calls,
        contains('agent routine agent-news-bot A feed sent: {{body}} webhook'),
      );
      expect(
        find.text('http://127.0.0.1:3773/api/hooks/task-9/secret'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a message starting with an agent goes to that agent and shows its answer',
    (tester) async {
      await show(
        tester,
        agents: [agentOf('release-watch', thread: 'thread-7')],
        transcript: [
          chatItem(
            item(1, BridgeItemKind.userMessage, ended: true),
            1,
            'Fix the move dialog.',
          ),
        ],
        turns: [turnOf(1, startedMs: 1000)],
        mentions: [
          BridgeMention(
            agent: 'agent-release-watch',
            name: 'release-watch',
            thread: 'thread-7',
            message: 'msg-1',
            text: 'is main green?',
            createdMs: 2000,
            answered: true,
            answer: 'Yes, CI passed at 10:12.',
          ),
        ],
      );

      expect(find.text('@release-watch is main green?'), findsOneWidget);
      expect(find.text('Yes, CI passed at 10:12.'), findsOneWidget);

      await tester.enterText(find.byKey(const Key('composer')), '@rel');
      await tester.pumpAndSettle();
      expect(find.text('Agents'), findsWidgets);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('composer')),
        '@release-watch did the nightly pass?',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();

      expect(
        project.calls,
        contains(
          'agent mention agent-release-watch thread-1 did the nightly pass?',
        ),
      );
      expect(
        project.session.calls.where((call) => call.startsWith('send')),
        isEmpty,
      );
    },
  );

  testWidgets('an earlier thread of an agent cannot be written to', (
    tester,
  ) async {
    await show(
      tester,
      agents: [
        agentOf(
          'news-bot',
          thread: 'thread-2',
          threads: ['thread-2', 'thread-1'],
        ),
      ],
    );

    expect(
      find.textContaining('An earlier thread of news-bot'),
      findsOneWidget,
    );
    expect(find.byKey(const Key('composer')), findsNothing);
    await tester.tap(find.text('Open current thread'));
    await tester.pumpAndSettle();
    expect(project.calls, contains('open thread-2'));
  });

  testWidgets(
    'a change a routine made waits in the Chat until the person applies it',
    (tester) async {
      final routine = routineOf('task-1', 'Morning news');
      await show(
        tester,
        agents: [agentOf('news-bot', thread: 'thread-1')],
        transcript: [
          chatItem(
            item(1, BridgeItemKind.userMessage, ended: true, routine: 'task-1'),
            1,
            'Check the feeds.',
          ),
          chatItem(
            item(2, BridgeItemKind.assistantMessage, ended: true),
            1,
            'I would read them twice.',
          ),
        ],
        turns: [turnOf(1, checkpoint: false)],
        automations: [routine],
        changes: [
          changeOf(
            5,
            BridgeAgentChangeKind.instructions,
            before: '# news-bot\n',
            after: '# news-bot\n\nRead twice.\n',
            held: true,
          ),
          changeOf(
            6,
            BridgeAgentChangeKind.routineAdded,
            routine: routine,
            held: true,
            undone: true,
          ),
        ],
      );

      expect(
        find.text('Waiting for you · Instructions saved · 3 lines'),
        findsOneWidget,
      );
      expect(find.text('Discarded'), findsOneWidget);
      expect(find.byKey(const Key('change-6-Run now')), findsNothing);

      await tester.tap(find.byKey(const Key('change-5-Apply')));
      await tester.pumpAndSettle();
      expect(project.calls, contains('agent apply agent-news-bot 5'));
      await tester.tap(find.byKey(const Key('change-5-Discard')));
      await tester.pumpAndSettle();
      expect(project.calls, contains('agent undo agent-news-bot 5'));
    },
  );
}
