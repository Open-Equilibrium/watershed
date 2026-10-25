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
      ('bash', (failed: true, durationMs: 12), 'boom'),
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
          arguments: '{"command":"npm test"}',
          readOnly: false,
        ),
      );
      await emit(event(BridgeEventKind.turnStarted, turn: 1));
      await emit(event(BridgeEventKind.requestOpened, request: approval('a1')));
      await emit(event(BridgeEventKind.requestOpened, request: approval('a2')));
      expect(state.status.value, RunStatus.waitingForYou);
      expect(state.approval.value!.id, 'a1');
      await state.decide(BridgeDecision.allowOnce);
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
      await state.answerText('B');
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
    await state.unqueue('Also docs');
    expect(backend.calls, [
      'clear queue',
      'send Use Redis',
      'follow-up And tests',
    ]);

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
