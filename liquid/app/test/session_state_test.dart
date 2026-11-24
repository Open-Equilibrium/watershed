import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/details.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/format.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

class GatedSession extends FakeSession {
  Completer<void>? gate;
  Completer<void>? titleGate;

  @override
  Future<String> title() async {
    await titleGate?.future;
    return super.title();
  }

  @override
  Future<void> confirmApplied(int applied) async {
    await gate?.future;
    await super.confirmApplied(applied);
  }

  @override
  Future<void> setBranch(String branch, {String? worktree}) async {
    await gate?.future;
    await super.setBranch(branch, worktree: worktree);
  }

  @override
  Future<void> mergeBack() async {
    await gate?.future;
    await super.mergeBack();
  }
}

class FailingSession extends FakeSession {
  Completer<void>? statisticsGate;
  Object? statisticsError;
  Object? contextError;
  Object? subscribeError;

  @override
  Future<BridgeStatistics> statistics() async {
    await statisticsGate?.future;
    if (statisticsError case final error?) throw error;
    return super.statistics();
  }

  @override
  Future<BridgeContext?> context() async {
    if (contextError case final error?) throw error;
    return super.context();
  }

  @override
  Future<BridgeSubscription> subscribe() async {
    if (subscribeError case final error?) throw error;
    return super.subscribe();
  }
}

class MovingSession extends FakeSession {
  final moved = FakeGit();

  @override
  Future<void> setBranch(String branch, {String? worktree}) async {
    await super.setBranch(branch, worktree: worktree);
    gitBackend = moved;
  }
}

class DelayedReads extends FakeSession {
  final turnReads = <Completer<List<BridgeTurn>>>[];
  bool gateTurns = false;
  Completer<void>? titleGate;
  Completer<BridgeSnapshot>? snapshotGate;

  @override
  Future<List<BridgeTurn>> turns() {
    if (!gateTurns) return super.turns();
    final read = Completer<List<BridgeTurn>>();
    turnReads.add(read);
    return read.future;
  }

  @override
  Future<String> title() async {
    await titleGate?.future;
    return super.title();
  }

  @override
  Future<BridgeSnapshot> snapshot() => snapshotGate?.future ?? super.snapshot();
}

class HeldChanges extends FakeSession {
  final holds = <int, Completer<void>>{};

  @override
  Future<List<BridgeFileChange>> turnChanges(
    int from,
    int to, {
    required bool hideWhitespace,
  }) async {
    final files = [fileOf('turn-$from-$to.dart')];
    await holds[from]?.future;
    return files;
  }
}

class SlowCancel extends FakeSession {
  SlowCancel({super.id});

  Completer<void>? cancelGate;

  @override
  Stream<BridgeEvent> events(int subscription) =>
      StreamController<BridgeEvent>(onCancel: () => cancelGate?.future).stream;
}

void main() {
  late FakeSession session;
  late FakeProject project;
  late SessionState state;

  setUp(() {
    session = FakeSession();
    project = FakeProject(session: session);
    state = SessionState(project);
  });

  Future<void> emit(BridgeEvent event) async {
    session.emit(event);
    await pumpEventQueue();
  }

  Future<void> open([BridgeSnapshot? snapshot]) async {
    if (snapshot != null) session.current = snapshot;
    await state.open('thread-1');
    await pumpEventQueue();
  }

  void expectNotice(String text) => expect(state.notice.value?.text, text);

  Future<void> switchTo(FakeSession next) async {
    project.session = next;
    await state.open(next.id);
    await pumpEventQueue();
  }

  BridgeEvent textDelta(int sequence, String text) => event(
    BridgeEventKind.contentDelta,
    sequence,
    item: item(1, BridgeItemKind.assistantMessage),
    text: text,
  );

  BridgeEvent startItem(int sequence, int turn, BridgeItem started) =>
      event(BridgeEventKind.itemStarted, sequence, turn: turn, item: started);

  BridgeAutomation automationOf(String id) => BridgeAutomation(
    id: id,
    title: 'Check',
    schedule: BridgeSchedule(
      kind: BridgeScheduleKind.webhook,
      weekdays: Int64List(0),
    ),
    enabled: true,
    lastRun: BridgeRunStatus.never,
  );

  BridgeLineage mergeTargetLineage() => lineageOf(
    rows: [
      BridgeLineageRow(
        threadId: 'thread-parent',
        relation: BridgeLineageRelation.fork,
        group: BridgeLineageGroup.related,
        title: 'Parent',
        statusLabel: 'Idle',
        failed: false,
        mergeTarget: true,
      ),
    ],
  );

  group('the start page', () {
    test('loadStart reads sessions, first model and default branch', () async {
      project.threadList = [threadOf('t1', title: 'Fix login')];
      project.providerList = [
        BridgeProvider(
          instance: 'down',
          name: 'Down',
          available: false,
          models: [modelOf('gone', 'Gone')],
        ),
        providerOf(
          models: [
            modelOf('claude-opus-5-5', 'Opus'),
            modelOf('claude-haiku', 'Haiku'),
          ],
        ),
      ];
      project.gitBackend.refList = [
        refOf('feature'),
        refOf('main', isDefault: true),
      ];

      await state.loadStart();

      expect(state.recent.value.map((listed) => listed.thread.title), [
        'Fix login',
      ]);
      expect(state.model.value, choiceOf('claude-opus-5-5'));
      expect(state.base.value, 'main');
    });

    test('a checked-out pull request starts in its worktree', () async {
      await state.loadStart();
      await state.draftFor(
        BridgeCheckedOut(branch: 'feature', worktree: '/worktrees/feature'),
      );

      await state.send('Review it');

      expect(project.calls, [
        'start Review it claude-opus-5-5 base=null worktree=/worktrees/feature fullAccess',
      ]);
    });

    test('sending with no model is an error', () async {
      await state.send('Hello');

      expect(state.notice.value?.level, NoticeLevel.error);
      expect(state.notice.value?.text, 'No model is available.');
      expect(project.calls, isEmpty);
    });
  });

  group('a session', () {
    test('a switch while the title is read keeps the new title', () async {
      final gate = Completer<void>();
      project.session = GatedSession()
        ..titleGate = gate
        ..sessionTitle = 'Old title'
        ..chosen = choiceOf('old-model');
      unawaited(state.open('thread-1'));
      await pumpEventQueue();

      await switchTo(
        FakeSession(id: 'thread-2')
          ..sessionTitle = 'New title'
          ..chosen = choiceOf('new-model'),
      );
      gate.complete();
      await pumpEventQueue();

      expect(state.sessionTitle.value, 'New title');
      expect(state.model.value, choiceOf('new-model'));
    });

    test('a switch releases the session it left and its Git', () async {
      await open();
      await switchTo(FakeSession(id: 'thread-2'));

      expect(session.closed, isTrue);
      expect(session.gitBackend.closed, isTrue);
      expect(project.session.closed, isFalse);
    });

    test('a late open is closed; the newer session stays open', () async {
      final older = FakeSession(id: 'thread-1');
      final newer = FakeSession(id: 'thread-2');
      final gate = Completer<void>();
      project.session = older;
      project.hold = gate;
      unawaited(state.open('thread-1'));
      await pumpEventQueue();

      project.hold = null;
      project.session = newer;
      await state.open('thread-2');
      gate.complete();
      await pumpEventQueue();

      expect(state.session, same(newer));
      expect(older.closed, isTrue);
      expect(newer.closed, isFalse);
    });

    test('an open returning after the start page is closed', () async {
      final older = FakeSession(id: 'thread-1');
      final gate = Completer<void>();
      project.session = older;
      project.hold = gate;
      unawaited(state.open('thread-1'));
      await pumpEventQueue();

      await state.newSession();
      gate.complete();
      await pumpEventQueue();

      expect(state.session, isNull);
      expect(older.closed, isTrue);
    });

    test('a start returning after another open is closed', () async {
      await state.loadStart();
      final started = FakeSession(id: 'thread-3');
      final gate = Completer<void>();
      project.session = started;
      project.hold = gate;
      unawaited(state.send('Fix it'));
      await pumpEventQueue();

      project.hold = null;
      project.session = session;
      await state.open('thread-1');
      gate.complete();
      await pumpEventQueue();

      expect(state.session, same(session));
      expect(started.closed, isTrue);
    });

    test('a switch is dropped while the old session still releases', () async {
      final slow = SlowCancel(id: 'thread-1');
      project.session = slow;
      await state.open('thread-1');
      await pumpEventQueue();

      final release = Completer<void>();
      final newer = FakeSession(id: 'thread-2')..sessionTitle = 'Stale title';
      slow.cancelGate = release;
      project.session = newer;
      unawaited(state.open('thread-2'));
      await pumpEventQueue();
      unawaited(state.newSession());
      release.complete();
      await pumpEventQueue();

      expect(state.session, isNull);
      expect(state.sessionTitle.value, isNull);
      expect(newer.closed, isTrue);
    });

    test('snapshot restore skips held events (ADR-0290)', () async {
      await open(
        snapshotOf(
          sequence: 5,
          turns: [
            turnOf(1, status: BridgeTurnStatus.running, checkpoint: false),
          ],
          transcript: [
            chatItem(
              item(
                1,
                BridgeItemKind.assistantMessage,
                ended: true,
                durationMs: 40,
              ),
              1,
              'Hi',
            ),
          ],
          requests: [approvalOf('a1')],
        ),
      );

      await emit(textDelta(5, ' (held)'));
      await emit(textDelta(6, ' there'));

      final reply = state.items.value.single;
      expect(
        (reply.text.value, reply.outcome.value),
        ('Hi there', (failed: false, durationMs: 40)),
      );
      expect(
        (state.request.value?.id, state.status.value),
        ('a1', RunStatus.waitingForYou),
      );
    });

    test('fell-behind or rolled-back reloads the chat (ADR-0290)', () async {
      await open();

      for (final (kind, text) in [
        (BridgeEventKind.fellBehind, 'after falling behind'),
        (BridgeEventKind.rolledBack, 'after a rollback'),
      ]) {
        session.current = snapshotOf(
          sequence: 9,
          transcript: [
            chatItem(item(1, BridgeItemKind.assistantMessage), 1, text),
          ],
        );
        await emit(event(kind, 1));

        expect(state.items.value.single.text.value, text);
      }
    });

    test('a slower turn read cannot replace a newer one', () async {
      final delayed = DelayedReads();
      await switchTo(delayed);
      delayed.gateTurns = true;
      delayed.emit(event(BridgeEventKind.turnStarted, 1, turn: 1));
      delayed.emit(event(BridgeEventKind.turnCompleted, 2, turn: 1));
      await pumpEventQueue();

      delayed.turnReads[1].complete([turnOf(1)]);
      await pumpEventQueue();
      delayed.turnReads[0].complete([
        turnOf(1, status: BridgeTurnStatus.running, checkpoint: false),
      ]);
      await pumpEventQueue();

      expect(state.status.value, RunStatus.idle);
    });

    test('a title read after a switch cannot replace the new title', () async {
      final first = DelayedReads()..sessionTitle = 'First';
      await switchTo(first);
      first.titleGate = Completer<void>();
      first.emit(event(BridgeEventKind.turnCompleted, 1, turn: 1));
      await pumpEventQueue();

      await switchTo(FakeSession(id: 'thread-2')..sessionTitle = 'Second');
      first.titleGate!.complete();
      await pumpEventQueue();

      expect(state.sessionTitle.value, 'Second');
    });

    test(
      'events received during a snapshot recovery survive its restore',
      () async {
        final delayed = DelayedReads()
          ..snapshotGate = Completer<BridgeSnapshot>();
        await switchTo(delayed);
        delayed.emit(event(BridgeEventKind.fellBehind, 1));
        await pumpEventQueue();

        delayed.emit(startItem(2, 1, item(1, BridgeItemKind.assistantMessage)));
        delayed.emit(textDelta(3, 'newer'));
        delayed.emit(
          event(BridgeEventKind.requestOpened, 4, request: approvalOf('a1')),
        );
        await pumpEventQueue();
        delayed.snapshotGate!.complete(snapshotOf(sequence: 1));
        await pumpEventQueue();

        expect(
          (state.items.value.single.text.value, state.request.value?.id),
          ('newer', 'a1'),
        );
      },
    );

    test('every event, skipped ones too, is confirmed (ADR-0272)', () async {
      await open(snapshotOf(sequence: 5));

      await emit(textDelta(4, 'held'));
      await emit(textDelta(6, 'new'));

      expect(session.confirmed, [1, 2]);
    });

    test('a pending confirmation never blocks the next (ADR-0272)', () async {
      final gate = Completer<void>();
      final first = GatedSession()..gate = gate;
      project.session = first;
      await open();
      first.emit(textDelta(1, 'x'));
      await pumpEventQueue();

      final second = FakeSession(id: 'thread-2');
      await switchTo(second);
      second.emit(textDelta(1, 'y'));
      await pumpEventQueue();
      gate.complete();
      await pumpEventQueue();

      expect(second.confirmed, [1]);
    });

    test('an item streams in place, then takes its outcome', () async {
      await open();
      await emit(startItem(1, 1, item(1, BridgeItemKind.assistantMessage)));
      final reply = state.items.value.single;
      await emit(textDelta(2, 'Hi'));
      await emit(textDelta(3, ' there'));
      expect(reply.text.value, 'Hi there');

      await emit(
        startItem(4, 1, item(2, BridgeItemKind.commandExecution, tool: 'bash')),
      );
      await emit(
        event(
          BridgeEventKind.itemCompleted,
          5,
          item: item(
            2,
            BridgeItemKind.commandExecution,
            tool: 'bash',
            ended: true,
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
    });

    test('a task list keeps its steps and names running instances', () async {
      const running = BridgeStepStatus.running;
      const completed = BridgeStepStatus.completed;
      session.turnList = [turnOf(1, status: BridgeTurnStatus.running)];
      await open(snapshotOf(turns: session.turnList));
      await emit(
        startItem(
          1,
          1,
          item(4, BridgeItemKind.taskList, steps: [step('Explore', running)]),
        ),
      );
      final tasks = state.items.value.firstWhere(
        (item) => item.kind == BridgeItemKind.taskList,
      );
      final later = [
        step('Explore', running),
        step('Explore › scraper-1', running),
        step('Explore › scraper-2', completed),
      ];

      await emit(
        event(
          BridgeEventKind.itemUpdated,
          2,
          item: item(4, BridgeItemKind.taskList, steps: later),
        ),
      );
      expect(tasks.steps.value, later);
      expect(state.runningInstances.value, [
        (name: 'scraper-1', phase: 'Explore'),
      ]);

      await emit(
        event(
          BridgeEventKind.itemCompleted,
          3,
          turn: 1,
          item: item(4, BridgeItemKind.taskList, ended: true),
          text: '',
        ),
      );
      expect(tasks.steps.value, later);

      session.turnList = [turnOf(1, status: BridgeTurnStatus.aborted)];
      await emit(event(BridgeEventKind.turnAborted, 4, turn: 1));
      expect(state.runningInstances.value, isEmpty);
    });

    test('a finished request adds its tokens, cached input included', () async {
      await open();

      await emit(
        event(
          BridgeEventKind.modelRequestCompleted,
          1,
          usage: BridgeUsage(
            input: 100,
            output: 20,
            cacheRead: 30,
            cacheWrite: 5,
            reasoning: 0,
          ),
        ),
      );

      expect((state.totals.value.input, state.totals.value.output), (135, 20));
    });

    test('a turn ends with its changes once checkpointed', () async {
      await open();
      session.turnList = [
        turnOf(1, added: 3, removed: 1, files: 2),
        turnOf(2, status: BridgeTurnStatus.running, checkpoint: false),
      ];

      await emit(startItem(1, 1, item(1, BridgeItemKind.assistantMessage)));
      await emit(event(BridgeEventKind.turnCompleted, 2, turn: 1));
      await emit(startItem(3, 2, item(2, BridgeItemKind.assistantMessage)));

      expect(
        [for (final row in state.items.value) row.turnEnd?.added],
        [null, 3, null],
        reason: 'only turn 1 has a checkpoint, so only it gets a row',
      );
    });

    test('requests queue oldest first and answering moves on', () async {
      session.turnList = [
        turnOf(1, status: BridgeTurnStatus.running, checkpoint: false),
      ];
      await open();
      await emit(event(BridgeEventKind.turnStarted, 1, turn: 1));
      final approval = approvalOf('a1');
      final question = questionsOf('q1', [
        questionOf('q1', 'Which?', options: ['A', 'B']),
      ]);

      await emit(event(BridgeEventKind.requestOpened, 2, request: approval));
      await emit(event(BridgeEventKind.requestOpened, 3, request: question));
      expect(
        (state.request.value?.id, state.status.value),
        ('a1', RunStatus.waitingForYou),
      );

      await state.respond(approval, 'approve');
      await emit(event(BridgeEventKind.requestResolved, 4, requestId: 'a1'));
      expect(state.request.value?.id, 'q1');

      await state.answer(question, [
        BridgeQuestionAnswer(question: 'Which?', values: ['B'], typed: false),
      ]);
      await emit(event(BridgeEventKind.requestResolved, 5, requestId: 'q1'));
      expect(
        (state.request.value, state.status.value),
        (null, RunStatus.running),
      );
      expect(session.calls, ['respond a1 approve', 'answer q1 Which?=B']);
    });

    test('choosing a model applies it and reads the context again', () async {
      session.contextWindow = const BridgeContext(tokens: 1000, window: 200000);
      await open();
      session.contextWindow = const BridgeContext(window: 1000000);

      await state.setModel(choiceOf('claude-haiku'));
      await pumpEventQueue();

      expect(session.calls, ['model claude-haiku']);
      expect(state.contextWindow.value?.window, 1000000);
    });

    test('a refused model or permission change keeps the old one', () async {
      session.chosen = choiceOf('claude-haiku');
      await open();
      session.modelError = 'the model is gone';
      await state.setModel(choiceOf('claude-opus-5-5'));
      expect(state.model.value, choiceOf('claude-haiku'));
      expectNotice('Could not apply the model selection: the model is gone');
      session.permissionsError = 'the agent is busy';

      await state.setPermissions(BridgePermissions.supervised);

      expect(state.permissions.value, BridgePermissions.fullAccess);
      expectNotice('Could not change the permissions: the agent is busy');

      session.permissionsError = null;
      await state.setPermissions(BridgePermissions.supervised);
      expect(state.permissions.value, BridgePermissions.supervised);
    });

    test('setBranch rebinds Git to the checkout it now uses', () async {
      final moving = MovingSession();
      project.session = moving;
      await open();

      await state.setBranch('feature', worktree: '/worktrees/feature');

      expect(moving.calls, ['branch feature worktree=/worktrees/feature']);
      expect(state.git.value.backend, same(moving.moved));
    });

    test('a switch during setBranch keeps Git on its session', () async {
      final gate = Completer<void>();
      project.session = GatedSession()..gate = gate;
      await open();
      unawaited(state.setBranch('feature'));
      await pumpEventQueue();

      final next = FakeSession(id: 'thread-2');
      await switchTo(next);
      gate.complete();
      await pumpEventQueue();

      expect(state.git.value.backend, same(next.gitBackend));
    });
  });

  group('the Diff panel', () {
    test('a scope compares its turns, or the latest finished one', () async {
      session.turnList = [
        turnOf(1),
        turnOf(2),
        turnOf(3, status: BridgeTurnStatus.running, checkpoint: false),
      ];
      session.patches['a.dart'] = '+x';
      await open();

      final scopes = <(DiffScope, int, int)>[
        (const LatestTurn(), 2, 2),
        (const AllTurns(), 1, 2),
        (const OneTurn(1), 1, 1),
        (const OneTurn(3), 2, 2),
      ];
      for (final (scope, from, to) in scopes) {
        await state.showTurns(scope);
        expect(
          (state.shown.value?.from.number, state.shown.value?.to.number),
          (from, to),
          reason: scope.runtimeType.toString(),
        );
      }
      expect(await state.patch('a.dart'), '+x');
    });

    test('the Uncommitted scope re-reads when the status changes', () async {
      await open();
      session.gitBackend.changes = [fileOf('a.dart')];
      await state.showTurns(const UncommittedChanges());
      expect(state.changed.value?.map((file) => file.path), ['a.dart']);

      session.gitBackend.changes = [fileOf('a.dart'), fileOf('b.dart')];
      session.gitBackend.change(gitStatusOf(changes: [fileOf('b.dart')]));
      await pumpEventQueue();

      expect(state.changed.value?.map((file) => file.path), [
        'a.dart',
        'b.dart',
      ]);
      session.gitBackend.patches['a.dart'] = '+uncommitted';
      expect(await state.patch('a.dart'), '+uncommitted');
    });

    Future<Completer<void>> selectOverSlowerRead() async {
      final held = HeldChanges()..turnList = [turnOf(1), turnOf(2)];
      project.session = held;
      await state.open('thread-1');
      await pumpEventQueue();

      final older = Completer<void>();
      held.holds[1] = older;
      unawaited(state.showTurns(const AllTurns()));
      await pumpEventQueue();
      await state.showTurns(const OneTurn(2));
      return older;
    }

    test('a slower earlier read cannot replace the newer one', () async {
      final older = await selectOverSlowerRead();

      older.complete();
      await pumpEventQueue();

      expect(state.shownScope.value, const OneTurn(2));
      expect(state.shown.value?.from.number, 2);
      expect(state.changed.value?.map((file) => file.path), ['turn-2-2.dart']);
    });

    test('a failed slower read shows no error over the newer one', () async {
      final older = await selectOverSlowerRead();

      older.completeError('the turn is gone');
      await pumpEventQueue();

      expect(state.changesProblem.value, isNull);
      expect(state.changed.value?.map((file) => file.path), ['turn-2-2.dart']);
    });
  });

  group('statistics and the context window', () {
    BridgeStatistics sonnet(int input, int output, int responses) =>
        BridgeStatistics(
          models: [
            BridgeModelUsage(
              model: 'anthropic/claude-sonnet',
              usage: usage(input, output),
              responses: responses,
            ),
          ],
          tools: const [],
          turns: const [],
        );

    test('a response or a turn end re-reads the statistics', () async {
      await open();
      session.stats = sonnet(10, 1, 1);
      await emit(
        event(BridgeEventKind.modelRequestCompleted, 1, usage: usage(10, 1)),
      );
      expect(state.statistics.value, session.stats);

      session.stats = statisticsOf(
        turns: [
          BridgeTurnStatistics(
            number: 1,
            prompt: 'Add a rate limit',
            models: const [],
            failed: 0,
          ),
        ],
      );
      await emit(event(BridgeEventKind.turnCompleted, 2, turn: 1));
      expect(state.statistics.value, session.stats);
    });

    test('a late statistics failure is not shown on the open one', () async {
      final gate = Completer<void>();
      final late = FailingSession()
        ..statisticsGate = gate
        ..statisticsError = 'the session file is unreadable';
      project.session = late;
      await state.open('thread-1');
      await pumpEventQueue();
      project.session = session;
      await state.open('thread-2');
      await pumpEventQueue();

      gate.complete();
      await pumpEventQueue();

      expect(state.statisticsProblem.value, isNull);
      expect(state.notices.value, isEmpty);
    });

    test('a failed context read keeps the statistics and says why', () async {
      final failing = FailingSession()
        ..contextError = 'the model has no window size';
      project.session = failing;

      await open();

      expect(state.statistics.value, failing.stats);
      expect(state.contextWindow.value, isNull);
      expectNotice(
        'Could not read the context window: the model has no window size',
      );
    });
  });

  group('notices', () {
    test('a failed Chat stream is a session problem, not a notice', () async {
      await open();

      session.fail('the stream broke');
      await pumpEventQueue();

      expect(state.notice.value, isNull);
      expect(state.sessionProblem.value, 'the stream broke.');
    });

    test('a failed send, open or subscribe is a notice', () async {
      await open();
      session.sendError = 'the agent is busy';
      await state.send('Again');
      expectNotice('Could not send the message: the agent is busy');

      project.openError = 'the session is gone';
      await state.open('thread-2');
      expectNotice('Could not open the session: the session is gone');

      project.openError = null;
      project.session = FailingSession()..subscribeError = 'the stream is gone';
      await state.open('thread-1');
      await pumpEventQueue();
      expectNotice('Could not open the session: the stream is gone');
    });

    test('an unreadable session list is a host problem', () async {
      state = SessionState(UnreachableProject());

      await state.loadStart();

      expect(state.notice.value, isNull);
      expect(state.hostProblems.value[null], 'the host is down.');
    });

    test('notices keep the last $noticeLimit through dismissal', () async {
      for (var index = 0; index <= noticeLimit; index++) {
        state.notify(NoticeLevel.info, 'Notice $index');
      }
      expect(state.notices.value, hasLength(noticeLimit));
      expect(state.notices.value.first.text, 'Notice 1');

      state.dismissNotice();

      expect(state.notice.value, isNull);
      expect(state.notices.value, hasLength(noticeLimit));
    });
  });

  group('the card sections', () {
    BridgeLocalServer server(int port) => BridgeLocalServer(
      host: 'localhost',
      port: port,
      url: 'http://localhost:$port',
      pid: 7,
      terminal: false,
    );

    test('each live section follows its changes after a read', () async {
      await open();

      session
        ..serverUpdates.add([server(3000)])
        ..automationUpdates.add([automationOf('a2')])
        ..lineageUpdates.add(
          BridgeLineage(
            rows: const [],
            running: 1,
            sideChatsRunning: 0,
            canDisconnect: false,
            mergeReady: false,
            canMerge: false,
          ),
        );
      await pumpEventQueue();

      expect(state.servers.value.single.port, 3000);
      expect(state.automations.value.single.id, 'a2');
      expect(state.lineage.value?.running, 1);
    });

    test('a failed live stream is a warning naming its section', () async {
      await open();

      session.serverUpdates.addError('the stream dropped');
      await pumpEventQueue();

      expect(state.notice.value?.level, NoticeLevel.warning);
      expectNotice('Could not follow the servers: the stream dropped');
    });

    test('a switch stops following the session it left', () async {
      await open();
      await switchTo(FakeSession(id: 'thread-2'));

      session.serverUpdates.add([server(3000)]);
      await pumpEventQueue();
      expect(state.servers.value, isEmpty);
    });

    test('merging back opens the thread it merged into', () async {
      session.family = mergeTargetLineage();
      await open();
      project.session = FakeSession(id: 'thread-parent');

      await state.mergeBack();
      await pumpEventQueue();

      expect(session.calls, contains('merge back'));
      expect(project.calls, contains('open thread-parent'));
      expect(state.session?.id, 'thread-parent');
    });

    test('a switch during a merge back keeps the chosen session', () async {
      final gate = Completer<void>();
      project.session = GatedSession()
        ..gate = gate
        ..family = mergeTargetLineage();
      await open();
      unawaited(state.mergeBack());
      await pumpEventQueue();

      await switchTo(FakeSession(id: 'thread-2'));
      gate.complete();
      await pumpEventQueue();

      expect(project.calls, isNot(contains('open thread-parent')));
      expect(state.session?.id, 'thread-2');
    });
  });

  test('disposing a session releases its host project', () async {
    await open();
    expect(project.closed, isFalse);
    await state.dispose();
    expect(project.closed, isTrue);
  });

  group('agent hosts (ADR-0295)', () {
    const studioAddress = 'ws://studio.local:3774';
    FakeProject listing(String id) =>
        FakeProject()..threadList = [threadOf(id)];
    String unreachable(String reason) =>
        'Liquid could not connect to the agent host $studioAddress: $reason';

    test('a session opens or starts on its own agent host', () async {
      final local = FakeProject();
      final studio = listing('studio');
      final state = SessionState(local);
      await state.addHost(studioAddress, studio);

      await state.open('studio', address: studioAddress);
      await pumpEventQueue();
      expect(studio.calls, ['open studio']);
      expect(state.sessionAddress, studioAddress);

      await state.newSession();
      await state.startOn(studioAddress);
      await state.send('Fix the login');
      await pumpEventQueue();
      expect(studio.calls.last, startsWith('start Fix the login'));
      expect(local.calls, isEmpty);
    });

    test(
      'an open session stays open and reopens when its host answers',
      () async {
        final session = FakeSession(id: 'studio');
        final studio = FakeProject(session: session)
          ..threadList = [threadOf('studio')];
        final state = SessionState(FakeProject());
        await state.addHost(studioAddress, studio);
        await state.open('studio', address: studioAddress);
        await pumpEventQueue();

        session.fail('the host is down');
        await pumpEventQueue();

        expect(state.session, same(session), reason: 'the Chat stays open');
        expect(state.sessionProblem.value, unreachable('the host is down.'));

        final back = FakeProject(session: session);
        await state.addHost(studioAddress, back);
        await pumpEventQueue();

        expect(back.calls, ['open studio']);
        expect(state.sessionProblem.value, isNull);
      },
    );

    test('a late listing does not hide a newer listing\'s hosts', () async {
      final gate = Completer<void>();
      final state = SessionState(
        LateFirstListing(gate.future)..threadList = [threadOf('local')],
      );

      final earlier = state.loadStart();
      await state.addHost(studioAddress, listing('studio'));
      gate.complete();
      await earlier;

      expect(threadIds(state), unorderedEquals(['local', 'studio']));
    });

    test('removing a host drops its sessions and releases it', () async {
      final studio = listing('studio');
      final state = SessionState(listing('local'));
      await state.addHost(studioAddress, studio);

      await state.removeHost(studioAddress);

      expect(threadIds(state), ['local']);
      expect(studio.closed, isTrue);
      expect(state.answers(studioAddress), isFalse);
    });

    group('moves (ADR-0308)', () {
      BridgeTranscriptItem assistantEntry(int id, String text) => chatItem(
        item(id, BridgeItemKind.assistantMessage, ended: true),
        1,
        text,
      );

      List<String> chatRows(SessionState state) => [
        for (final row in state.items.value) row.marker ?? row.text.value,
      ];

      test('the marker names hosts as Liquid does', () async {
        final session = FakeSession(id: 'moved')
          ..current = snapshotOf(transcript: [assistantEntry(1, 'Continuing')]);
        final target = FakeProject(session: session)
          ..hostName = 'studio-box'
          ..moveRecordAnswer = BridgeMoveRecord(
            fromHost: 'Mac mini',
            toHost: 'studio-box',
            atMs: 1700000000000,
            transcript: const [],
          );
        final state = SessionState(FakeProject())
          ..hostLabel = (address) =>
              address == null ? 'This Mac' : 'Studio mini';
        await state.addHost(studioAddress, target);

        await state.open('moved', address: studioAddress);
        await pumpEventQueue();

        expect(chatRows(state), [
          'Moved from This Mac to Studio mini · ${formatClockMs(1700000000000)}',
          'Continuing',
        ]);
      });
    });
  });
}

List<String> threadIds(SessionState state) => [
  for (final listed in state.recent.value) listed.thread.id,
];

class LateFirstListing extends FakeProject {
  LateFirstListing(this.gate);

  final Future<void> gate;
  int _listings = 0;

  @override
  Future<List<BridgeThreadSummary>> threads() async {
    if (_listings++ == 0) await gate;
    return threadList;
  }
}
