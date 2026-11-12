import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

void main() {
  late FakeBackend backend;
  late SessionState state;

  Future<void> emit(BridgeEvent event) async {
    backend.controller.add(event);
    await pumpEventQueue();
  }

  setUp(() {
    backend = FakeBackend();
    state = SessionState(backend);
  });

  test('streams a turn into items whose text grows in place', () async {
    await state.send('Say hi');
    expect(backend.calls, ['send Say hi']);
    await emit(event(BridgeEventKind.turnStarted));
    expect(state.status.value, RunStatus.running);
    await emit(
      event(
        BridgeEventKind.itemStarted,
        item: item(1, kind: BridgeItemKind.assistantMessage),
      ),
    );
    final reply = state.items.value.single;
    await emit(event(BridgeEventKind.contentDelta, item: item(1), text: 'Hi'));
    await emit(
      event(BridgeEventKind.contentDelta, item: item(1), text: ' there'),
    );
    expect(reply.text.value, 'Hi there');
    await emit(
      event(
        BridgeEventKind.itemStarted,
        item: item(2, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    await emit(
      event(
        BridgeEventKind.itemCompleted,
        item: item(
          2,
          kind: BridgeItemKind.commandExecution,
          failed: true,
          durationMs: 12,
        ),
        text: 'boom',
      ),
    );
    final command = state.items.value.last;
    expect(
      (command.tool, command.outcome.value, command.text.value),
      ('bash', (failed: true, durationMs: 12, lines: null), 'boom'),
    );
    await emit(
      event(
        BridgeEventKind.modelRequestCompleted,
        step: BridgeStep(usage: usage(100, 20, 5), stopReason: 'stop'),
      ),
    );
    expect(
      (
        state.totals.value.input,
        state.totals.value.output,
        state.totals.value.costMicroUsd,
      ),
      (100, 20, 5),
    );
    await state.send('Shorter please');
    expect(
      backend.calls.last,
      'send Shorter please',
      reason: 'pi decides whether a message starts or steers a run',
    );
    await emit(event(BridgeEventKind.turnCompleted));
    expect(state.status.value, RunStatus.idle);
  });

  test('queues questions and reports waiting for the user', () async {
    BridgeRequest pick(String id) => BridgeRequest(
      id: id,
      question: BridgeQuestion(
        kind: BridgeQuestionKind.select,
        title: 'Pick',
        detail: '',
        options: const ['A', 'B'],
      ),
    );
    await emit(event(BridgeEventKind.turnStarted));
    await emit(event(BridgeEventKind.userInputRequested, request: pick('q1')));
    await emit(event(BridgeEventKind.userInputRequested, request: pick('q2')));
    expect(state.status.value, RunStatus.waitingForYou);
    expect(state.question.value!.id, 'q1');
    await state.answerText('q1', 'B');
    expect(backend.calls.last, 'answer q1 B');
    await emit(
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q1'),
      ),
    );
    expect(state.question.value!.id, 'q2');
    await emit(
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q2'),
      ),
    );
    expect(state.question.value, isNull);
    expect(state.status.value, RunStatus.running);
  });

  test('keeps extension displays, the draft and the exit', () async {
    BridgeDisplay display(
      String key, {
      List<String>? lines,
      BridgeLevel? level,
    }) => BridgeDisplay(key: key, lines: lines, level: level);
    await emit(
      event(
        BridgeEventKind.widgetChanged,
        display: display('todo', lines: ['one', 'two']),
      ),
    );
    await emit(
      event(
        BridgeEventKind.statusChanged,
        display: display('todo'),
        text: '1/2',
      ),
    );
    await emit(event(BridgeEventKind.inputTextSet, text: 'draft'));
    await emit(
      event(
        BridgeEventKind.notification,
        display: display('', level: BridgeLevel.warning),
        text: 'Careful',
      ),
    );
    expect(state.widgets.value, {
      'todo': ['one', 'two'],
    });
    expect(state.statuses.value, {'todo': '1/2'});
    expect(state.draft.value, 'draft');
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (BridgeLevel.warning, 'Careful'),
    );
    expect(state.notices.value.map((notice) => notice.text), ['Careful']);
    await emit(event(BridgeEventKind.widgetChanged, display: display('todo')));
    await emit(event(BridgeEventKind.statusChanged, display: display('todo')));
    expect(state.widgets.value, isEmpty);
    expect(state.statuses.value, isEmpty);
    await emit(
      event(
        BridgeEventKind.failed,
        text: 'agent wrote a record that is not a JSON object',
      ),
    );
    await emit(event(BridgeEventKind.exited, success: false));
    expect(state.status.value, RunStatus.failed);
    expect(
      state.failure.value,
      'agent wrote a record that is not a JSON object',
    );
  });

  test('signals when the stored history changes and branches', () async {
    final changes = <int>[];
    state.stored.addListener(() => changes.add(state.stored.value));
    await emit(event(BridgeEventKind.turnStarted));
    await emit(
      event(
        BridgeEventKind.contentDelta,
        item: item(1),
        text: 'streaming text is not stored yet',
      ),
    );
    expect(changes, isEmpty);
    await emit(event(BridgeEventKind.itemCompleted, item: item(1)));
    await emit(event(BridgeEventKind.turnCompleted));
    expect(changes, [1, 2]);

    backend.stored = historyOf([]);
    expect((await state.history(['e2'])).leaf, 'e1');
    await state.branch('e1');
    expect(backend.calls, [
      'history e2',
      'branch e1',
      'transcript',
      'history',
    ], reason: 'a branch rebuilds the chat');
    expect(changes.last, 3);
  });

  test(
    'a revert continues the chat and the Diff tab on the new branch',
    () async {
      final turn = turnOf('T2', number: 2);
      final confirmed = revertPlanOf().confirmation;
      backend.turnList = [turnOf('T1', number: 1), turn];
      await state.readChanges();
      final changes = <int>[];
      state.stored.addListener(() => changes.add(state.stored.value));
      // pi continues before the turn's prompt, so the new branch holds only the earlier turn.
      backend.turnList = [turnOf('T1', number: 1)];

      expect(await state.revert(turn, confirmed), isTrue);
      expect(backend.calls, containsAllInOrder(['revert T2', 'transcript']));
      expect(changes, [1], reason: 'the stored history changed');
      expect(state.turns.value.map((turn) => turn.id), ['T1']);
      expect(state.shown.value?.to.id, 'T1');

      // The files changed, but the session stayed on its branch: the Diff tab still rereads.
      backend
        ..revertError =
            'the files were reverted, but the session could not continue from before the '
            'turn: the agent has exited'
        ..turnList = [turnOf('T0', number: 1)];
      expect(await state.revert(turnOf('T1', number: 1), confirmed), isFalse);
      expect(
        state.notice.value?.text,
        'Could not revert and branch. The files were reverted, but the session could '
        'not continue from before the turn: the agent has exited',
      );
      expect(changes, [1]);
      expect(state.turns.value.map((turn) => turn.id), ['T0']);
    },
  );

  test(
    'rebuilds the chat of the current branch with its branch mark',
    () async {
      backend.stored = historyOf(
        [],
        tree: [
          const BridgeTreeNode(
            failed: false,
            id: 'u1',
            kind: BridgeEntryKind.user,
            preview: 'Go',
          ),
          const BridgeTreeNode(
            failed: false,
            id: 'a1',
            parent: 'u1',
            kind: BridgeEntryKind.assistant,
            preview: 'Looked',
          ),
          const BridgeTreeNode(
            failed: false,
            id: 'a2',
            parent: 'a1',
            kind: BridgeEntryKind.assistant,
            preview: 'Done',
          ),
          const BridgeTreeNode(
            failed: false,
            id: 'b1',
            parent: 'a1',
            kind: BridgeEntryKind.user,
            preview: 'Instead',
          ),
        ],
        leaf: 'b1',
      );
      backend.transcripts[''] = BridgeTranscript(
        items: [
          chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
          chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
          chatItem('b1', BridgeItemKind.userMessage, text: 'Instead'),
        ],
        more: true,
        start: 'u1',
      );
      backend.transcripts['u1'] = BridgeTranscript(
        items: [chatItem('u0', BridgeItemKind.userMessage, text: 'Earlier')],
        more: false,
        start: 'u0',
      );
      await state.loadChat();
      String describe(ItemView view) =>
          view.marker == null ? view.text.value : 'marker ${view.marker!.ref}';
      expect(state.items.value.map(describe), [
        'Go',
        'Looked',
        'marker B',
        'Instead',
      ]);
      expect(state.chatMore.value, isTrue);
      await state.loadEarlier();
      expect(state.items.value.map(describe).first, 'Earlier');
      expect(state.chatMore.value, isFalse);
    },
  );

  test('says below each turn what it changed, restored and live', () async {
    BridgeTreeNode node(String id, String? parent, BridgeEntryKind kind) =>
        BridgeTreeNode(
          failed: false,
          id: id,
          parent: parent,
          kind: kind,
          preview: id,
        );
    backend.stored = historyOf(
      [],
      tree: [
        node('c1', null, BridgeEntryKind.custom),
        node('u1', 'c1', BridgeEntryKind.user),
        node('a1', 'u1', BridgeEntryKind.assistant),
        node('c2', 'a1', BridgeEntryKind.custom),
        node('c3', 'c2', BridgeEntryKind.custom),
        node('u2', 'c3', BridgeEntryKind.user),
        node('a2', 'u2', BridgeEntryKind.assistant),
      ],
      leaf: 'a2',
    );
    backend.transcripts[''] = BridgeTranscript(
      items: [
        chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
        chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
        chatItem('u2', BridgeItemKind.userMessage, text: 'Again'),
        chatItem('a2', BridgeItemKind.assistantMessage, text: 'Running'),
      ],
      more: false,
    );
    final first = turnOf(
      'T1',
      number: 1,
      startEntry: 'c1',
      endEntry: 'c2',
      files: 3,
      added: 12,
      removed: 1,
    );
    backend.turnList = [
      first,
      turnOf('T2', number: 2, ended: false, startEntry: 'c3'),
    ];
    await state.loadChat();
    String describe(ItemView view) => switch (view.turnEnd) {
      final turn? => '${turn.id}: ${turn.files} files',
      null => view.text.value,
    };
    expect(state.items.value.map(describe), [
      'Go',
      'Looked',
      'T1: 3 files',
      'Again',
      'Running',
    ]);

    // The running turn ends; what it changed follows its live items.
    backend.turnList = [
      first,
      turnOf(
        'T2',
        number: 2,
        startEntry: 'c3',
        endEntry: 'c4',
        added: 1,
        removed: 0,
      ),
    ];
    backend.controller.add(event(BridgeEventKind.turnCompleted));
    await pumpEventQueue();
    expect(state.items.value.map(describe).last, 'T2: 1 files');
    // Ending it again, as after a second read, adds nothing.
    backend.controller.add(event(BridgeEventKind.turnCompleted));
    await pumpEventQueue();
    expect(
      state.items.value.where((view) => view.turnEnd != null),
      hasLength(2),
    );
  });

  test('keeps the row of a live turn when earlier messages load', () async {
    backend.transcripts[''] = BridgeTranscript(
      items: [chatItem('u1', BridgeItemKind.userMessage, text: 'Go')],
      more: true,
      start: 'u1',
    );
    backend.transcripts['u1'] = BridgeTranscript(
      items: [chatItem('u0', BridgeItemKind.userMessage, text: 'Earlier')],
      more: false,
      start: 'u0',
    );
    backend.turnList = const [];
    await state.loadChat();
    backend.turnList = [turnOf('T1', number: 1, added: 2, removed: 0)];
    backend.controller.add(event(BridgeEventKind.turnCompleted));
    await pumpEventQueue();
    await state.loadEarlier();
    expect(
      state.items.value.map((view) => view.turnEnd?.id ?? view.text.value),
      ['Earlier', 'Go', 'T1'],
    );
  });

  test(
    'puts a turn\'s row where the turn ended, also once the next began',
    () async {
      backend.turnList = const [];
      backend.turnsGate = Completer();
      backend.controller.add(event(BridgeEventKind.turnCompleted));
      // The next turn begins before the ended one was read.
      backend.controller.add(
        event(
          BridgeEventKind.itemStarted,
          item: item(7, kind: BridgeItemKind.userMessage),
        ),
      );
      await pumpEventQueue();
      backend.turnList = [
        turnOf('T1', number: 1, added: 2, removed: 0),
        turnOf('T2', number: 2, ended: false),
      ];
      backend.turnsGate!.complete();
      await pumpEventQueue();
      expect(
        state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'),
        ['T1', '7'],
      );
    },
  );

  test('puts each of two turns read together where it ended', () async {
    backend.turnList = const [];
    backend.turnsGate = Completer();
    for (final id in [1, 2, 3]) {
      backend.controller.add(
        event(
          BridgeEventKind.itemStarted,
          item: item(id, kind: BridgeItemKind.userMessage),
        ),
      );
      // Turns 1 and 2 end before their read returns; turn 3 is running.
      if (id < 3) backend.controller.add(event(BridgeEventKind.turnCompleted));
    }
    await pumpEventQueue();
    backend.turnList = [
      turnOf('T1', number: 1, added: 1, removed: 0),
      turnOf('T2', number: 2, added: 1, removed: 0),
      turnOf('T3', number: 3, ended: false),
    ];
    backend.turnsGate!.complete();
    await pumpEventQueue();
    expect(state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'), [
      '1',
      'T1',
      '2',
      'T2',
      '3',
    ]);
  });

  test(
    'a turn that ended outside Git leaves its place to no later turn',
    () async {
      // Outside Git the turns end without checkpoints, so their positions are never used.
      backend.turnList = const [];
      await emit(
        event(
          BridgeEventKind.itemStarted,
          item: item(1, kind: BridgeItemKind.userMessage),
        ),
      );
      await emit(event(BridgeEventKind.turnCompleted));
      await emit(
        event(
          BridgeEventKind.itemStarted,
          item: item(2, kind: BridgeItemKind.userMessage),
        ),
      );
      await emit(event(BridgeEventKind.turnCompleted));
      // Git is initialized, and the third turn changes files.
      backend.turnList = [turnOf('T3', number: 3, added: 1, removed: 0)];
      await emit(
        event(
          BridgeEventKind.itemStarted,
          item: item(3, kind: BridgeItemKind.userMessage),
        ),
      );
      await emit(event(BridgeEventKind.turnCompleted));
      expect(
        state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'),
        ['1', '2', '3', 'T3'],
      );
    },
  );

  Future<void> turn(int id) async {
    await emit(
      event(
        BridgeEventKind.itemStarted,
        item: item(id, kind: BridgeItemKind.userMessage),
      ),
    );
    await emit(event(BridgeEventKind.turnCompleted));
  }

  test('a turn whose end is recorded late keeps its place', () async {
    // The first read after the turn ended does not show its end checkpoint yet.
    backend.turnList = [turnOf('T1', number: 1, ended: false)];
    await turn(1);
    expect(state.items.value.where((view) => view.turnEnd != null), isEmpty);
    backend.turnList = [
      turnOf('T1', number: 1, added: 1, removed: 0),
      turnOf('T2', number: 2, added: 1, removed: 0),
    ];
    await turn(2);
    expect(state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'), [
      '1',
      'T1',
      '2',
      'T2',
    ]);
  });

  test(
    'a turn that never recorded its end takes no later turn\'s place',
    () async {
      backend.turnList = [turnOf('T1', number: 1, ended: false)];
      await turn(1);
      backend.turnList = [
        turnOf('T1', number: 1, ended: false),
        turnOf('T2', number: 2, added: 1, removed: 0),
      ];
      await turn(2);
      expect(
        state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'),
        ['1', '2', 'T2'],
      );
    },
  );

  test('a turn keeps its place when its turns could not be read', () async {
    backend.turnList = [turnOf('T1', number: 1, added: 1, removed: 0)];
    backend.failTurns = true;
    await turn(1);
    backend.failTurns = false;
    backend.turnList = [
      turnOf('T1', number: 1, added: 1, removed: 0),
      turnOf('T2', number: 2, added: 1, removed: 0),
    ];
    await turn(2);
    expect(state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'), [
      '1',
      'T1',
      '2',
      'T2',
    ]);
  });

  test('a turn keeps its place when only an earlier read succeeded', () async {
    // A read that began before the turn ended returns the turns from before its end.
    backend.turnList = const [];
    backend.turnsGate = Completer();
    final earlier = state.readChanges();
    await turn(1);
    backend.turnsFailFrom = backend.turnReads + 1;
    backend.turnsGate!.complete();
    await earlier;
    await pumpEventQueue();
    backend.turnsFailFrom = null;
    backend.turnList = [
      turnOf('T1', number: 1, added: 1, removed: 0),
      turnOf('T2', number: 2, added: 1, removed: 0),
    ];
    await turn(2);
    expect(state.items.value.map((view) => view.turnEnd?.id ?? '${view.id}'), [
      '1',
      'T1',
      '2',
      'T2',
    ]);
  });

  test('says nothing below a turn that changed no file', () async {
    backend.turnList = [
      turnOf('T1', number: 1, files: 0, added: 0, removed: 0),
    ];
    backend.controller.add(event(BridgeEventKind.turnCompleted));
    await pumpEventQueue();
    expect(state.items.value.where((view) => view.turnEnd != null), isEmpty);
  });

  group('the rebuilt chat', () {
    const tree = [
      BridgeTreeNode(
        failed: false,
        id: 'u1',
        kind: BridgeEntryKind.user,
        preview: 'Go',
      ),
      BridgeTreeNode(
        failed: false,
        id: 'a1',
        parent: 'u1',
        kind: BridgeEntryKind.assistant,
        preview: 'Looked',
      ),
      BridgeTreeNode(
        failed: false,
        id: 'a2',
        parent: 'a1',
        kind: BridgeEntryKind.assistant,
        preview: 'Done',
      ),
      BridgeTreeNode(
        failed: false,
        id: 'b1',
        parent: 'a1',
        kind: BridgeEntryKind.user,
        preview: 'Instead',
      ),
      BridgeTreeNode(
        failed: false,
        id: 'b2',
        parent: 'b1',
        kind: BridgeEntryKind.assistant,
        preview: 'B2',
      ),
    ];
    String describe(ItemView view) =>
        view.marker == null ? view.text.value : 'marker ${view.marker!.ref}';

    test('holds one branch at a time and never doubles', () async {
      backend.stored = historyOf([], tree: tree, leaf: 'b2');
      backend.transcripts[''] = BridgeTranscript(
        items: [
          chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
          chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
        ],
        more: false,
        start: 'u1',
      );
      backend.gate = Completer();
      final first = state.branch('a1');
      final second = state.branch('a1');
      final reload = state.loadChat();
      await pumpEventQueue();
      backend.gate!.complete();
      expect(await first, isTrue);
      expect(await second, isFalse, reason: 'the second waited for the first');
      await reload;
      expect(backend.calls.where((call) => call.startsWith('branch')), [
        'branch a1',
      ], reason: 'a double press branches once');
      expect(
        state.items.value
            .where((view) => view.marker == null)
            .map((view) => view.text.value),
        ['Go', 'Looked'],
        reason: 'overlapping restores keep one copy',
      );
      expect(state.items.value.where((view) => view.marker != null).length, 1);
    });

    test('says when it cannot be read', () async {
      backend.transcriptError = 'session file is gone';
      await state.loadChat();
      expect(
        state.notice.value?.text,
        'Could not restore the chat: session file is gone',
      );
    });

    test(
      'marks the branch at the start when earlier pages show nothing',
      () async {
        backend.stored = historyOf([], tree: tree, leaf: 'b2');
        backend.transcripts[''] = BridgeTranscript(
          items: [chatItem('b2', BridgeItemKind.assistantMessage, text: 'B2')],
          more: true,
          start: 'b2',
        );
        backend.transcripts['b2'] = const BridgeTranscript(
          items: [],
          more: false,
          start: 'u1',
        );
        await state.loadChat();
        await state.loadEarlier();
        expect(state.items.value.map(describe), ['marker B', 'B2']);
      },
    );

    test('marks the branch where it leaves, also on an earlier page', () async {
      backend.stored = historyOf([], tree: tree, leaf: 'b2');
      // The newest page starts after the first entry past the fork.
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('b2', BridgeItemKind.assistantMessage, text: 'B2')],
        more: true,
        start: 'b2',
      );
      backend.transcripts['b2'] = BridgeTranscript(
        items: [
          chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
          chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
          chatItem('b1', BridgeItemKind.userMessage, text: 'Instead'),
        ],
        more: false,
        start: 'u1',
      );
      await state.loadChat();
      await state.loadEarlier();
      expect(state.items.value.map(describe), [
        'Go',
        'Looked',
        'marker B',
        'Instead',
        'B2',
      ]);
    });
  });

  test(
    'places a live turn\'s row after a branch mark an earlier page adds',
    () async {
      const tree = [
        BridgeTreeNode(
          failed: false,
          id: 'u1',
          kind: BridgeEntryKind.user,
          preview: 'Go',
        ),
        BridgeTreeNode(
          failed: false,
          id: 'a1',
          parent: 'u1',
          kind: BridgeEntryKind.assistant,
          preview: 'Looked',
        ),
        BridgeTreeNode(
          failed: false,
          id: 'a2',
          parent: 'a1',
          kind: BridgeEntryKind.assistant,
          preview: 'Done',
        ),
        BridgeTreeNode(
          failed: false,
          id: 'b1',
          parent: 'a1',
          kind: BridgeEntryKind.user,
          preview: 'Instead',
        ),
        BridgeTreeNode(
          failed: false,
          id: 'b2',
          parent: 'b1',
          kind: BridgeEntryKind.assistant,
          preview: 'B2',
        ),
      ];
      backend.stored = historyOf([], tree: tree, leaf: 'b2');
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('b2', BridgeItemKind.assistantMessage, text: 'B2')],
        more: true,
        start: 'b2',
      );
      backend.transcripts['b2'] = BridgeTranscript(
        items: [
          chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
          chatItem('a1', BridgeItemKind.assistantMessage, text: 'Looked'),
          chatItem('b1', BridgeItemKind.userMessage, text: 'Instead'),
        ],
        more: false,
        start: 'u1',
      );
      backend.turnList = const [];
      await state.loadChat();
      // A live turn ends while its read waits; meanwhile the earlier page adds the mark.
      backend.controller.add(
        event(
          BridgeEventKind.itemStarted,
          item: item(9, kind: BridgeItemKind.userMessage),
        ),
      );
      backend.turnsGate = Completer();
      backend.controller.add(event(BridgeEventKind.turnCompleted));
      await pumpEventQueue();
      await state.loadEarlier();
      backend.turnList = [turnOf('T9', number: 3, added: 1, removed: 0)];
      backend.turnsGate!.complete();
      await pumpEventQueue();
      String name(ItemView view) =>
          view.turnEnd?.id ??
          (view.marker != null ? 'marker' : null) ??
          (view.entry == null ? '${view.id}' : view.text.value);
      expect(state.items.value.map(name), [
        'Go',
        'Looked',
        'marker',
        'Instead',
        'B2',
        '9',
        'T9',
      ]);
    },
  );

  group('what goes wrong does not lose messages', () {
    test('a waiting message that cannot be sent again comes back', () async {
      backend
        ..queued = const BridgeQueue(steering: ['a', 'b', 'c'], followUp: [])
        ..sendError = 'busy';
      await state.unqueue('b', followUp: false);
      expect(state.draft.value, 'a\n\nc');
      expect(state.notice.value?.level, BridgeLevel.error);
    });

    test('a stop that fails still hands the waiting messages back', () async {
      backend
        ..queued = const BridgeQueue(steering: ['x'], followUp: [])
        ..abortError = 'agent is gone';
      await state.stop();
      expect(state.draft.value, 'x');
      expect(
        state.notice.value?.text,
        'Could not stop the agent: agent is gone',
      );
    });

    test('an exited agent\'s unreadable pages add no notice', () async {
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
        more: true,
        start: 'u2',
      );
      await state.loadChat();
      await emit(event(BridgeEventKind.exited, success: true));
      backend.transcriptError = 'the agent has exited';
      await state.loadEarlier();
      expect(state.notices.value, isEmpty, reason: 'the exit says it all');
    });

    test('an earlier chat page that cannot be read is reported once', () async {
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
        more: true,
        start: 'u2',
      );
      await state.loadChat();
      backend.transcriptError = 'session file is gone';
      // Each scroll near the top tries again.
      await state.loadEarlier();
      await state.loadEarlier();
      expect(state.notices.value.map((notice) => notice.text), [
        'Could not read earlier messages: session file is gone',
      ]);
    });
  });

  test('an agent that exits mid-turn leaves nothing running', () async {
    await emit(event(BridgeEventKind.turnStarted));
    await emit(
      event(
        BridgeEventKind.itemStarted,
        item: item(3, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    await emit(event(BridgeEventKind.exited, success: false));
    expect(state.items.value.single.outcome.value?.failed, isTrue);
  });

  test('an earlier page read before a branch never doubles the chat', () async {
    backend.transcripts[''] = BridgeTranscript(
      items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
      more: true,
      start: 'u2',
    );
    backend.transcripts['u2'] = BridgeTranscript(
      items: [chatItem('u1', BridgeItemKind.userMessage, text: 'First')],
      more: false,
      start: 'u1',
    );
    await state.loadChat();
    backend.earlierGate = Completer();
    final earlier = state.loadEarlier();
    await state.branch('u2');
    backend.earlierGate!.complete();
    await earlier;
    expect(
      state.items.value.where((view) => view.text.value == 'First'),
      isEmpty,
      reason: 'the page belongs to the chat before the branch',
    );
  });

  test(
    'a stale earlier-page read does not stop the next chat reading its own',
    () async {
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
        more: true,
        start: 'u2',
      );
      backend.transcripts['u2'] = BridgeTranscript(
        items: [chatItem('u1', BridgeItemKind.userMessage, text: 'First')],
        more: false,
        start: 'u1',
      );
      await state.loadChat();
      final stalled = Completer<void>();
      backend.earlierGate = stalled;
      final stale = state.loadEarlier();
      await pumpEventQueue();
      // The new session's own reads are not held back by the old one.
      backend.earlierGate = null;
      backend.transcripts[''] = const BridgeTranscript(
        items: [],
        more: true,
        start: 'x',
      );
      backend.transcripts['x'] = BridgeTranscript(
        items: [chatItem('u9', BridgeItemKind.userMessage, text: 'Mine')],
        more: false,
        start: 'u9',
      );
      await state.newSession();
      expect(state.items.value.map((view) => view.text.value), [
        'Mine',
      ], reason: 'the new chat reads its own earlier page');
      stalled.complete();
      await stale;
      expect(state.items.value.map((view) => view.text.value), [
        'Mine',
      ], reason: 'the old page never joins the new chat');
    },
  );

  test(
    'an earlier-page read started during a restore leaves the restored chat',
    () async {
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
        more: true,
        start: 'u2',
      );
      backend.transcripts['u2'] = BridgeTranscript(
        items: [chatItem('u1', BridgeItemKind.userMessage, text: 'First')],
        more: false,
        start: 'u1',
      );
      await state.loadChat();
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u3', BridgeItemKind.userMessage, text: 'Newer')],
        more: false,
        start: 'u3',
      );
      backend.newestGate = Completer();
      backend.earlierGate = Completer();
      final restoring = state.loadChat();
      await pumpEventQueue();
      // Started with the cursor the restore is about to replace.
      final earlier = state.loadEarlier();
      await pumpEventQueue();
      backend.newestGate!.complete();
      await restoring;
      backend.earlierGate!.complete();
      await earlier;
      expect(state.items.value.map((view) => view.text.value), ['Newer']);
      expect(state.chatMore.value, isFalse);
    },
  );

  test(
    'a message sent while the queue is rebuilt follows the kept messages',
    () async {
      backend.queued = const BridgeQueue(
        steering: [],
        followUp: ['a', 'b', 'c'],
      );
      backend.sendGate = Completer();
      final removing = state.unqueue('b', followUp: true);
      await pumpEventQueue();
      final sending = state.send('d');
      await pumpEventQueue();
      backend.sendGate!.complete();
      await Future.wait([removing, sending]);
      expect(backend.calls, [
        'clear queue',
        'follow-up a',
        'follow-up c',
        'send d',
      ], reason: 'the message waits until the kept messages are sent again');
    },
  );

  test(
    'a message waiting for a restore also waits for a switch started meanwhile',
    () async {
      backend.gate = Completer();
      final restoring = state.loadChat();
      final sending = state.send('Hi');
      await pumpEventQueue();
      backend.switchGate = Completer();
      final switching = state.newSession();
      await pumpEventQueue();
      backend.gate!.complete();
      await pumpEventQueue();
      expect(
        backend.calls,
        isNot(contains('send Hi')),
        reason: 'the switch is still running',
      );
      backend.switchGate!.complete();
      await Future.wait([restoring, switching, sending]);
      expect(backend.calls.last, 'send Hi');
    },
  );

  test(
    'a send that completes after a new session leaves its title unset',
    () async {
      backend.sendGate = Completer();
      final sending = state.send('Old');
      await pumpEventQueue();
      await state.newSession();
      backend.sendGate!.complete();
      await sending;
      expect(state.sessionTitle.value, isNull);
    },
  );

  test('a stored parent cycle restores without hanging', () async {
    // a's parent is b and b's is a, so the path to the leaf must stop at the repeated entry.
    backend.stored = historyOf(
      [],
      tree: const [
        BridgeTreeNode(
          failed: false,
          id: 'a',
          parent: 'b',
          kind: BridgeEntryKind.assistant,
          preview: 'A',
        ),
        BridgeTreeNode(
          failed: false,
          id: 'b',
          parent: 'a',
          kind: BridgeEntryKind.user,
          preview: 'B',
        ),
      ],
      leaf: 'a',
    );
    backend.transcripts[''] = BridgeTranscript(
      items: [
        chatItem('b', BridgeItemKind.userMessage, text: 'Before'),
        chatItem('a', BridgeItemKind.assistantMessage, text: 'After'),
      ],
      more: false,
    );
    await state.loadChat();
    expect(state.items.value.map((view) => view.entry).whereType<String>(), [
      'b',
      'a',
    ], reason: 'each stored entry appears once');
  });

  test('keeps an error in the status bar until it is dismissed', () async {
    state.notify(BridgeLevel.error, 'The model request failed');
    state.notify(BridgeLevel.info, 'Session saved');
    expect(state.notice.value?.text, 'The model request failed');
    expect(state.notices.value.map((notice) => notice.text), [
      'Session saved',
      'The model request failed',
    ], reason: 'the list keeps both');
    state.dismissNotice();
    expect(state.notice.value, isNull);
  });

  test('counts cached input in the session totals', () async {
    await emit(
      event(
        BridgeEventKind.modelRequestCompleted,
        step: BridgeStep(
          usage: const BridgeUsage(
            input: 100,
            output: 20,
            cacheRead: 9000,
            cacheWrite: 50,
            reasoning: 0,
            costMicroUsd: 0,
          ),
          stopReason: 'stop',
        ),
      ),
    );
    expect(state.totals.value.input, 9150);
  });

  test('restores what a continued session used', () async {
    backend.stored = historyOf(
      [],
      usage: const BridgeUsage(
        input: 100,
        output: 20,
        cacheRead: 900,
        cacheWrite: 10,
        reasoning: 0,
        costMicroUsd: 12000,
      ),
    );
    await state.loadChat();
    final totals = state.totals.value;
    expect(
      (totals.input, totals.output, totals.costMicroUsd),
      (1010, 20, 12000),
    );
  });

  test('holds messages and switches until the session has changed', () async {
    backend.switchGate = Completer();
    final switching = state.newSession();
    final again = state.newSession();
    final sending = state.send('Hi');
    await pumpEventQueue();
    expect(backend.calls, ['new session'], reason: 'one switch at a time');
    backend.switchGate!.complete();
    await Future.wait([switching, again, sending]);
    expect(backend.calls.where((call) => call == 'new session'), hasLength(1));
    expect(
      backend.calls.last,
      'send Hi',
      reason: 'the message goes to the new session',
    );

    // A branch names an entry of the session shown, which a switch replaces, so it is dropped.
    backend.switchGate = Completer();
    final switchingAgain = state.newSession();
    expect(await state.branch('e1'), isFalse);
    backend.switchGate!.complete();
    await switchingAgain;
    expect(backend.calls, isNot(contains('branch e1')));
    expect(
      state.notice.value?.text,
      'The session is changing, so the branch was not made.',
    );
  });

  test('drops a chat read that ends after the session changed', () async {
    backend.transcripts[''] = BridgeTranscript(
      items: [chatItem('o1', BridgeItemKind.userMessage, text: 'Old')],
      more: false,
      start: 'o1',
    );
    backend.gate = Completer();
    final restoring = state.loadChat();
    await pumpEventQueue();
    backend.transcripts[''] = const BridgeTranscript(items: [], more: false);
    final switching = state.newSession();
    await pumpEventQueue();
    backend.gate!.complete();
    await Future.wait([restoring, switching]);
    expect(
      state.items.value,
      isEmpty,
      reason: 'the old chat stays out of the new session',
    );
  });

  test('holds a message until the chat is restored', () async {
    backend.gate = Completer();
    final restoring = state.loadChat();
    final sending = state.send('Hi');
    await pumpEventQueue();
    expect(backend.calls, isNot(contains('send Hi')));
    backend.gate!.complete();
    await Future.wait([restoring, sending]);
    expect(backend.calls.last, 'send Hi');
  });

  test('a message sent during a revert keeps the item of its turn', () async {
    final turn = turnOf('T2', number: 2);
    backend.turnList = [turnOf('T1', number: 1), turn];
    backend.revertGate = Completer();
    // The agent starts a turn as each message reaches it.
    backend.onSend = (_) => backend.controller.add(
      event(
        BridgeEventKind.itemStarted,
        item: item(7, kind: BridgeItemKind.userMessage),
      ),
    );
    final reverting = state.revert(turn, revertPlanOf().confirmation);
    final sending = state.send('Again');
    await pumpEventQueue();
    backend.revertGate!.complete();
    expect(await reverting, isTrue);
    await sending;
    await pumpEventQueue();
    expect(state.items.value.map((view) => view.id), [7]);
  });

  test('retires open cards when the agent exits', () async {
    await emit(
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.confirm,
            title: 'Sure?',
            detail: '',
            options: const [],
          ),
        ),
      ),
    );
    expect(state.question.value?.id, 'q1');
    await emit(event(BridgeEventKind.exited, success: false));
    expect(
      (state.question.value, state.status.value),
      (null, RunStatus.failed),
    );
  });

  test('ends extension displays with their session', () async {
    Future<void> show() async {
      await emit(
        event(
          BridgeEventKind.widgetChanged,
          display: const BridgeDisplay(key: 'todo', lines: ['one']),
        ),
      );
      await emit(
        event(
          BridgeEventKind.statusChanged,
          display: const BridgeDisplay(key: 'todo'),
          text: '1/2',
        ),
      );
    }

    await show();
    backend.switchCancelled = true;
    await state.newSession();
    expect(state.widgets.value.keys, [
      'todo',
    ], reason: 'a cancelled switch keeps it');
    expect(state.statuses.value.keys, ['todo']);
    backend.switchCancelled = false;
    await state.newSession();
    expect(state.widgets.value, isEmpty);
    expect(state.statuses.value, isEmpty);
  });

  test('stops only after the queue is rebuilt', () async {
    backend.queued = const BridgeQueue(steering: ['Keep'], followUp: ['Drop']);
    backend.sendGate = Completer();
    final removing = state.unqueue('Drop', followUp: true);
    await pumpEventQueue();
    final stopping = state.stop();
    await pumpEventQueue();
    expect(backend.calls, [
      'clear queue',
      'send Keep',
    ], reason: 'Stop waits for the removal');
    backend.sendGate!.complete();
    await Future.wait([removing, stopping]);
    expect(backend.calls, ['clear queue', 'send Keep', 'clear queue', 'abort']);
  });

  test('pages past entries that show nothing in the chat', () async {
    backend.transcripts[''] = const BridgeTranscript(
      items: [],
      more: true,
      start: 't1',
    );
    backend.transcripts['t1'] = const BridgeTranscript(
      items: [],
      more: true,
      start: 'r1',
    );
    backend.transcripts['r1'] = BridgeTranscript(
      items: [chatItem('u1', BridgeItemKind.userMessage, text: 'Go')],
      more: false,
      start: 'u1',
    );
    await state.loadChat();
    expect(state.items.value.map((view) => view.text.value), ['Go']);
    expect(state.chatMore.value, isFalse);
  });

  test('keeps a rejected message in the input area and says why', () async {
    backend.sendError = 'the agent rejected the command: no API key';
    await state.send('Say hi');
    expect(state.draft.value, 'Say hi');
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (
        BridgeLevel.error,
        'The message was not sent: the agent rejected the command: no API key',
      ),
    );
  });

  test('says why a model request failed', () async {
    await emit(
      event(
        BridgeEventKind.modelRequestCompleted,
        step: BridgeStep(
          usage: usage(0, 0, 0),
          stopReason: 'error',
          error: '401 invalid x-api-key',
        ),
      ),
    );
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (BridgeLevel.error, 'The model request failed: 401 invalid x-api-key'),
    );
  });

  test('a request that Stop ended is no failure', () async {
    BridgeEvent ended(String stopReason, String error) => event(
      BridgeEventKind.modelRequestCompleted,
      step: BridgeStep(
        usage: usage(0, 0, 0),
        stopReason: stopReason,
        error: error,
      ),
    );
    // Providers report an aborted request, some as an error.
    await emit(ended('aborted', 'Request was aborted'));
    await emit(event(BridgeEventKind.turnStarted));
    await state.stop();
    await emit(ended('error', 'This operation was aborted'));
    await emit(event(BridgeEventKind.turnAborted));
    expect(state.notices.value, isEmpty);
  });

  test('a request that ended before it was shown never shows', () async {
    // Stop can dismiss a question while the core is still reporting it.
    await emit(event(BridgeEventKind.turnStarted));
    await emit(
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q1'),
      ),
    );
    await emit(
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.confirm,
            title: 'Sure?',
            detail: '',
            options: const [],
          ),
        ),
      ),
    );
    expect(state.question.value, isNull);
    expect(state.status.value, RunStatus.running);
  });

  test('a Stop that failed leaves model failures reported', () async {
    backend.abortError = 'agent is gone';
    await emit(event(BridgeEventKind.turnStarted));
    await state.stop();
    await emit(
      event(
        BridgeEventKind.modelRequestCompleted,
        step: BridgeStep(
          usage: usage(0, 0, 0),
          stopReason: 'error',
          error: '529 overloaded',
        ),
      ),
    );
    expect(
      state.notice.value?.text,
      'The model request failed: 529 overloaded',
    );
  });

  test('reads the changes once more at most while a read runs', () async {
    // Each read takes a snapshot of the project, so piled-up reads would wait on each other.
    backend.changesGate = Completer();
    final reads = [for (var read = 0; read < 4; read++) state.readChanges()];
    await pumpEventQueue();
    expect(backend.changeReads, 1);
    backend.changesGate!.complete();
    await Future.wait(reads);
    expect(
      backend.changeReads,
      2,
      reason: 'one more read covers the changes meanwhile',
    );
  });

  test('keeps the waiting messages and edits them', () async {
    await emit(
      event(
        BridgeEventKind.queueChanged,
        queue: const BridgeQueue(
          steering: ['Use Redis'],
          followUp: ['Also docs', 'And tests'],
        ),
      ),
    );
    expect(state.queue.value.followUp, ['Also docs', 'And tests']);
    backend.queued = const BridgeQueue(
      steering: ['Use Redis'],
      followUp: ['Also docs', 'And tests'],
    );
    await state.unqueue('Also docs', followUp: true);
    expect(backend.calls, [
      'clear queue',
      'send Use Redis',
      'follow-up And tests',
    ]);

    backend.calls.clear();
    backend.queued = const BridgeQueue(steering: ['Same'], followUp: ['Same']);
    await state.unqueue('Same', followUp: true);
    expect(backend.calls, [
      'clear queue',
      'send Same',
    ], reason: 'the follow-up goes, the steering message stays');
    backend.queued = const BridgeQueue(
      steering: ['Use Redis'],
      followUp: ['Also docs', 'And tests'],
    );

    backend.calls.clear();
    await emit(event(BridgeEventKind.turnStarted));
    await state.stop();
    expect(backend.calls, ['clear queue', 'abort']);
    expect(
      state.draft.value,
      'Use Redis\n\nAlso docs\n\nAnd tests',
      reason: 'stopping hands the waiting messages back for editing',
    );
  });

  test('dispose releases the stream and stops an idle agent', () async {
    await state.dispose().timeout(const Duration(seconds: 2));
    expect(backend.cancelled, isTrue);
    expect(backend.calls, ['shutdown']);
  });

  test('confirms each applied event, so the bridge may send more', () async {
    await emit(event(BridgeEventKind.turnStarted));
    await emit(event(BridgeEventKind.notification, text: 'Working'));
    await emit(event(BridgeEventKind.turnCompleted));
    expect(backend.confirmed, [1, 2, 3]);
  });

  test('a fell-behind event reads the chat back from the session file, which then shows the text of the stream without overflow', () async {
    final whole = FakeBackend();
    final wholeState = SessionState(whole);
    final overflowed = FakeBackend();
    final overflowedState = SessionState(overflowed);
    overflowed.transcripts[''] = BridgeTranscript(
      items: [chatItem('e1', BridgeItemKind.assistantMessage, text: 'Hello')],
      more: false,
    );
    BridgeEvent completed() => event(
      BridgeEventKind.itemCompleted,
      item: item(1, kind: BridgeItemKind.assistantMessage),
      text: 'Hello',
    );
    // The partial output "llo" went in the overflow; the first turn's end and the second turn
    // come after the fell-behind event.
    final turns = [
      event(BridgeEventKind.turnStarted),
      event(
        BridgeEventKind.itemStarted,
        item: item(1, kind: BridgeItemKind.assistantMessage),
      ),
      event(BridgeEventKind.contentDelta, item: item(1), text: 'He'),
      event(BridgeEventKind.contentDelta, item: item(1), text: 'llo'),
      completed(),
      event(BridgeEventKind.turnCompleted),
      event(BridgeEventKind.turnStarted),
      event(
        BridgeEventKind.itemStarted,
        item: item(2, kind: BridgeItemKind.assistantMessage),
      ),
      event(BridgeEventKind.contentDelta, item: item(2), text: 'Bye'),
      event(BridgeEventKind.turnCompleted),
    ];
    final dropped = [
      ...turns.take(3),
      event(BridgeEventKind.fellBehind),
      ...turns.skip(4),
    ];
    Future<void> feed(FakeBackend to, List<BridgeEvent> events) async {
      for (final each in events) {
        to.controller.add(each);
        await pumpEventQueue();
      }
    }

    await feed(whole, turns);
    await feed(overflowed, dropped);
    expect(await assistantTexts(wholeState), ['Hello', 'Bye']);
    expect(await assistantTexts(overflowedState), ['Hello', 'Bye']);
    expect(
      overflowed.calls.where((call) => call.startsWith('transcript')),
      hasLength(2),
      reason: 'read at the fell-behind event, and again when the turn that was running ends',
    );
  });
}

/// The text of each assistant message the chat shows, in order.
Future<List<String>> assistantTexts(SessionState state) async => [
  for (final view in state.items.value)
    if (view.kind == BridgeItemKind.assistantMessage) view.text.value,
];
