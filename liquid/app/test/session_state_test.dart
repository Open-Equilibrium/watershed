import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
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
    expect(backend.calls, ['prompt Say hi']);
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
      (command.tool, command.failed, command.durationMs, command.text.value),
      ('bash', true, 12, 'boom'),
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
    expect(backend.calls.last, 'steer Shorter please');
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
    expect(state.notification.value, (BridgeLevel.warning, 'Careful'));
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

  test('dispose releases the stream and stops the agent', () async {
    await state.dispose();
    expect(backend.cancelled, isTrue);
    expect(backend.calls, ['shutdown']);
  });
}
