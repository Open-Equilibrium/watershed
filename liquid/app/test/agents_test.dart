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

import 'fake_backend.dart';
import 'focus_kit.dart';

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

Future<void> typeInComposer(WidgetTester tester, String text) async {
  await tester.enterText(find.byKey(const Key('composer')), text);
  await tester.pumpAndSettle();
}

void main() {
  late FakeProject project;

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
    final state = SessionState(project);
    await state.loadStart();
    if (open != null) await state.open(open);
    await pumpApp(
      tester,
      AgentShell(state: state, info: info, parseDiff: parsePatch),
    );
  }

  testWidgets('agents are listed above the threads, and their threads only '
      'there', (tester) async {
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
    expectOne(find.text('news-bot'));
    expectOne(find.textContaining('next '));
    expectNone(find.text('news-bot earlier'));
    expectOne(find.text('Fix the move dialog'));

    await tap(tester, find.text('news-bot'));
    expect(project.calls, contains('open thread-2'));

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
    expectOne(find.textContaining('An earlier thread of news-bot'));
    expectNone(find.byKey(const Key('composer')));
    await tap(tester, find.text('Open current thread'));
    expect(project.calls, contains('open thread-2'));
  });

  testWidgets('a new agent starts as a Chat whose first message is its '
      'job', (tester) async {
    await show(tester, open: null);

    await tap(tester, find.byKey(const Key('new-agent')));
    expectOne(find.text('What should this agent do?'));
    expectOne(find.text('New agent'));

    await tap(tester, find.text('Triage new issues.'));
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('composer')))
          .controller!
          .text,
      'Triage new issues.',
    );
    await typeInComposer(tester, 'Triage new issues.');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(project.calls, contains('agent start Triage new issues.'));
    expect(project.calls, contains('open thread-agent-1'));
    expectNone(find.text('What should this agent do?'));
  });

  testWidgets(
    "an agent's changes show after its turn with undo and run now, and a routine's change waits until applied",
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

      expectOne(find.text('Named itself news-bot'));
      expectOne(find.text('Instructions saved · 3 lines'));
      expectOne(find.text('Routine added: Morning news · Every 60 min'));
      expectOne(find.text('Undone'));
      expectNone(find.byKey(const Key('change-1-Undo')));

      await tap(tester, find.byKey(const Key('change-3-Run now')));
      expect(project.session.calls, contains('run automation task-1'));

      await tap(tester, find.byKey(const Key('change-3-Undo')));
      expect(project.calls, contains('agent undo agent-news-bot 3'));

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

      expectOne(find.text('Waiting for you · Instructions saved · 3 lines'));
      expectOne(find.text('Discarded'));
      expectNone(find.byKey(const Key('change-6-Run now')));

      await tap(tester, find.byKey(const Key('change-5-Apply')));
      expect(project.calls, contains('agent apply agent-news-bot 5'));
      await tap(tester, find.byKey(const Key('change-5-Discard')));
      expect(project.calls, contains('agent undo agent-news-bot 5'));
    },
  );

  testWidgets('the card shows the agent, renames it and adds a webhook '
      'routine', (tester) async {
    await show(tester, agents: [agentOf('news-bot', thread: 'thread-1')]);

    expectOne(find.text('Agent'));
    expectOne(find.text('Read the feeds.'));
    expectOne(find.text('Approval required ⌄'));

    await tap(tester, find.text('Rename'));
    await tester.enterText(find.byKey(const Key('agent-text')), 'morning-news');
    await tap(tester, find.text('Rename').last);
    expect(
      project.calls,
      contains(
        'agent update agent-news-bot rename=morning-news model=null '
        'permissions=null',
      ),
    );
    expect(project.agentList.single.name, 'morning-news');

    await tap(tester, find.byKey(const Key('add-routine')));
    await tap(tester, find.text('Webhook'));
    await tester.enterText(
      find.byKey(const Key('routine-prompt')),
      'A feed sent: {{body}}',
    );
    await tap(tester, find.byKey(const Key('routine-add')));
    expect(
      project.calls,
      contains('agent routine agent-news-bot A feed sent: {{body}} webhook'),
    );
    expectOne(find.text('http://127.0.0.1:3773/api/hooks/task-9/secret'));
  });

  testWidgets('a message starting with an agent goes to that agent and shows '
      'its answer', (tester) async {
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

    expectOne(find.text('@release-watch is main green?'));
    expectOne(find.text('Yes, CI passed at 10:12.'));

    await typeInComposer(tester, '@rel');
    expect(find.text('Agents'), findsWidgets);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    await typeInComposer(tester, '@release-watch did the nightly pass?');
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
  });

  testWidgets('a message to an agent that fails keeps the typed prompt in the '
      'composer', (tester) async {
    await show(tester, agents: [agentOf('release-watch', thread: 'thread-7')]);
    project.mentionError = 'refused';
    await typeInComposer(tester, '@release-watch did the nightly pass?');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expectOne(
      find.textContaining('Could not send the message to release-watch'),
    );
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('composer')))
          .controller!
          .text,
      '@release-watch did the nightly pass?',
    );
    expect(
      project.session.calls.where((call) => call.startsWith('send')),
      isEmpty,
    );
  });
}
