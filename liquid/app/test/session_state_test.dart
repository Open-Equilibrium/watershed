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
    await emit(event(BridgeEventKind.turnStarted, turn: 1));
    expect(state.status.value, RunStatus.running);
    await emit(
      event(
        BridgeEventKind.itemStarted,
        turn: 1,
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
        turn: 1,
        item: item(2, kind: BridgeItemKind.commandExecution, tool: 'bash'),
      ),
    );
    await emit(
      event(
        BridgeEventKind.itemCompleted,
        turn: 1,
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
        turn: 1,
        step: BridgeStep(
          id: 1,
          model: 'faux/faux-1',
          usage: usage(100, 20, 5),
          stopReason: 'stop',
          durationMs: 900,
        ),
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
    await emit(event(BridgeEventKind.turnCompleted, turn: 1));
    expect(state.status.value, RunStatus.idle);
  });

  test(
    'queues approvals and questions and reports waiting for the user',
    () async {
      BridgeRequest approval(String id) => BridgeRequest(
        id: id,
        approval: BridgeApproval(
          kind: BridgeRequestKind.commandExecution,
          tool: 'bash',
          text: 'npm test',
        ),
      );
      await emit(event(BridgeEventKind.turnStarted, turn: 1));
      await emit(event(BridgeEventKind.requestOpened, request: approval('a1')));
      await emit(event(BridgeEventKind.requestOpened, request: approval('a2')));
      expect(state.status.value, RunStatus.waitingForYou);
      expect(state.approval.value!.id, 'a1');
      await state.decide('a1', BridgeDecision.allowOnce);
      expect(backend.calls.last, 'decide a1 allowOnce');
      await emit(
        event(
          BridgeEventKind.requestResolved,
          request: BridgeRequest(id: 'a1', decision: BridgeDecision.allowOnce),
        ),
      );
      expect(state.approval.value!.id, 'a2');
      await emit(
        event(
          BridgeEventKind.requestResolved,
          request: BridgeRequest(id: 'a2', decision: BridgeDecision.reject),
        ),
      );
      expect(state.approval.value, isNull);
      expect(state.status.value, RunStatus.running);

      await emit(
        event(
          BridgeEventKind.userInputRequested,
          request: BridgeRequest(
            id: 'q1',
            question: BridgeQuestion(
              kind: BridgeQuestionKind.select,
              title: 'Pick',
              detail: '',
              options: ['A', 'B'],
            ),
          ),
        ),
      );
      expect(state.status.value, RunStatus.waitingForYou);
      await state.answerText('q1', 'B');
      expect(backend.calls.last, 'answer q1 B');
      await emit(
        event(
          BridgeEventKind.userInputResolved,
          request: BridgeRequest(id: 'q1'),
        ),
      );
      expect(state.question.value, isNull);
    },
  );

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
    await emit(event(BridgeEventKind.titleChanged, text: 'pi - acme'));
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
    expect((state.title.value, state.draft.value), ('pi - acme', 'draft'));
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
    await emit(event(BridgeEventKind.turnStarted, turn: 1));
    await emit(
      event(
        BridgeEventKind.contentDelta,
        item: item(1),
        text: 'streaming text is not stored yet',
      ),
    );
    expect(changes, isEmpty);
    await emit(event(BridgeEventKind.itemCompleted, item: item(1)));
    await emit(
      event(
        BridgeEventKind.requestResolved,
        request: const BridgeRequest(id: 'a1'),
      ),
    );
    await emit(event(BridgeEventKind.turnCompleted, turn: 1));
    expect(changes, [1, 2, 3]);

    backend.stored = historyOf([]);
    expect((await state.history(['e2'])).leaf, 'e1');
    await state.branch('e1');
    expect(backend.calls, [
      'history e2',
      'branch e1',
      'transcript',
      'history',
    ], reason: 'a branch rebuilds the chat');
    expect(changes.last, 4);
  });

  test(
    'rebuilds the chat of the current branch with its branch mark',
    () async {
      backend.stored = historyOf(
        [],
        tree: [
          const BridgeTreeNode(
            id: 'u1',
            kind: BridgeEntryKind.user,
            preview: 'Go',
          ),
          const BridgeTreeNode(
            id: 'a1',
            parent: 'u1',
            kind: BridgeEntryKind.assistant,
            preview: 'Looked',
          ),
          const BridgeTreeNode(
            id: 'a2',
            parent: 'a1',
            kind: BridgeEntryKind.assistant,
            preview: 'Done',
          ),
          const BridgeTreeNode(
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

  group('the rebuilt chat', () {
    const tree = [
      BridgeTreeNode(id: 'u1', kind: BridgeEntryKind.user, preview: 'Go'),
      BridgeTreeNode(
        id: 'a1',
        parent: 'u1',
        kind: BridgeEntryKind.assistant,
        preview: 'Looked',
      ),
      BridgeTreeNode(
        id: 'a2',
        parent: 'a1',
        kind: BridgeEntryKind.assistant,
        preview: 'Done',
      ),
      BridgeTreeNode(
        id: 'b1',
        parent: 'a1',
        kind: BridgeEntryKind.user,
        preview: 'Instead',
      ),
      BridgeTreeNode(
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
      await Future.wait([first, second, reload]);
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

    test('an earlier chat page that cannot be read is reported', () async {
      backend.transcripts[''] = BridgeTranscript(
        items: [chatItem('u2', BridgeItemKind.userMessage, text: 'Later')],
        more: true,
        start: 'u2',
      );
      await state.loadChat();
      backend.transcriptError = 'session file is gone';
      await state.loadEarlier();
      expect(
        state.notice.value?.text,
        'Could not read earlier messages: session file is gone',
      );
    });
  });

  test('an agent that exits mid-turn leaves nothing running', () async {
    await emit(event(BridgeEventKind.turnStarted, turn: 1));
    await emit(
      event(
        BridgeEventKind.itemStarted,
        turn: 1,
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
        turn: 1,
        step: BridgeStep(
          id: 1,
          model: 'faux/faux-1',
          usage: const BridgeUsage(
            input: 100,
            output: 20,
            cacheRead: 9000,
            cacheWrite: 50,
            reasoning: 0,
            costMicroUsd: 0,
          ),
          stopReason: 'stop',
          durationMs: 900,
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

  test('retires open cards when the agent exits', () async {
    await emit(
      event(
        BridgeEventKind.requestOpened,
        request: BridgeRequest(
          id: 'a1',
          approval: BridgeApproval(
            kind: BridgeRequestKind.commandExecution,
            tool: 'bash',
            text: '{}',
          ),
        ),
      ),
    );
    expect(state.approval.value?.id, 'a1');
    await emit(event(BridgeEventKind.exited, success: false));
    expect(
      (state.approval.value, state.question.value, state.status.value),
      (null, null, RunStatus.failed),
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
        turn: 1,
        step: BridgeStep(
          id: 1,
          model: 'faux/faux-1',
          usage: usage(0, 0, 0),
          stopReason: 'error',
          error: '401 invalid x-api-key',
          durationMs: 120,
        ),
      ),
    );
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (BridgeLevel.error, 'The model request failed: 401 invalid x-api-key'),
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
    await emit(event(BridgeEventKind.turnStarted, turn: 1));
    await state.stop();
    expect(backend.calls, ['clear queue', 'abort']);
    expect(
      state.draft.value,
      'Use Redis\n\nAlso docs\n\nAnd tests',
      reason: 'stopping hands the waiting messages back for editing',
    );
  });

  test('dispose releases the stream and stops the agent', () async {
    await state.dispose();
    expect(backend.cancelled, isTrue);
    expect(backend.calls, ['shutdown']);
  });
}
