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

  @override
  Future<void> confirmApplied(int applied) async {
    await gate?.future;
    await super.confirmApplied(applied);
  }
}

class FailingReads extends FakeSession {
  Completer<void>? statisticsGate;
  Object? statisticsError;
  Object? contextError;

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
}

class MovingSession extends FakeSession {
  final moved = FakeGit();

  @override
  Future<void> setBranch(String branch, {String? worktree}) async {
    await super.setBranch(branch, worktree: worktree);
    gitBackend = moved;
  }
}

class GatedTitle extends FakeSession {
  Completer<void>? gate;

  @override
  Future<String> title() async {
    await gate?.future;
    return super.title();
  }
}

class GatedActions extends FakeSession {
  Completer<void>? gate;

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

class UnsubscribableSession extends FakeSession {
  @override
  Future<BridgeSubscription> subscribe() => Future.error('the stream is gone');
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
  Stream<BridgeEvent> events(int subscription) => StreamController<BridgeEvent>(
    onCancel: () async {
      await cancelGate?.future;
    },
  ).stream;
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
    test('loadStart reads the sessions, the first available model and the default branch', () async {
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

    test('send starts a session in a new worktree from the base, with the chosen permissions', () async {
      await state.loadStart();
      await state.setPermissions(BridgePermissions.supervised);

      await state.send('Fix the login');
      await pumpEventQueue();

      expect(project.calls, [
        'start Fix the login claude-opus-5-5 base=main worktree=null supervised',
      ]);
      expect(state.session, same(session));
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

    test('sending with no model is an error and starts nothing', () async {
      await state.send('Hello');

      expect(state.notice.value?.level, NoticeLevel.error);
      expect(state.notice.value?.text, 'No model is available.');
      expect(project.calls, isEmpty);
    });
  });

  group('a session', () {
    test('send, steer and stop reach the open session', () async {
      await open();

      await state.send('Say hi');
      await state.send('Shorter', steer: true);
      await state.stop();

      expect(session.calls, ['send Say hi', 'steer Shorter', 'stop']);
    });

    test('a switch while the title is read keeps the title and model of the session it left out', () async {
      final gate = Completer<void>();
      project.session = GatedTitle()
        ..gate = gate
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

    test('a switch releases the session it left and its Git, so their T3 follows end', () async {
      await open();
      await switchTo(FakeSession(id: 'thread-2'));

      expect(session.closed, isTrue);
      expect(session.gitBackend.closed, isTrue);
      expect(project.session.closed, isFalse);
    });

    test('an open that returns after a newer open is closed, and the newer session stays open', () async {
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

    test('a switch that returns while the session it replaces is still releasing is not installed', () async {
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

    test('opening restores the snapshot and skips the events it already holds (ADR-0290)', () async {
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

    test('a fell-behind or rolled-back event reloads the chat from a new snapshot (ADR-0290)', () async {
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

    test('every event, skipped ones too, is confirmed (ADR-0272)', () async {
      await open(snapshotOf(sequence: 5));

      await emit(textDelta(4, 'held'));
      await emit(textDelta(6, 'new'));

      expect(session.confirmed, [1, 2]);
    });

    test('a confirmation in flight when the session changes does not hold back the next session (ADR-0272)', () async {
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

    test('an item streams its text in place and takes its outcome when it completes', () async {
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

    test('a task list keeps its steps when it completes and names the running instances', () async {
      const running = BridgeStepStatus.running;
      const completed = BridgeStepStatus.completed;
      await open();
      await emit(
        startItem(
          1,
          1,
          item(4, BridgeItemKind.taskList, steps: [step('Explore', running)]),
        ),
      );
      final tasks = state.items.value.single;
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
    });

    test('a finished model request adds its tokens to the totals, cached input included', () async {
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

    test('queued turns wait in the queue, and each can be sent now, edited or cancelled', () async {
      session.turnList = [
        turnOf(
          1,
          status: BridgeTurnStatus.queued,
          prompt: 'Later',
          checkpoint: false,
        ),
      ];
      await open();

      final queued = state.queued.value.single;
      await state.sendQueuedNow(queued);
      await state.editQueued(queued, 'Even later');
      await state.cancelQueued(queued);

      expect(session.calls, ['send now 1', 'edit 1 Even later', 'cancel 1']);
    });

    test('Edit from here puts the prompt in the draft, and Fork opens the new session', () async {
      session.revertedPrompt = 'Add a rate limit';
      await open();

      expect(await state.revert(2, restoreFiles: true), isTrue);
      expect(state.draft.value, 'Add a rate limit');

      await state.fork(2);
      expect(session.calls, ['revert 2 files=true', 'fork 2']);
      expect(project.calls.last, 'open fork-of-thread-1');
    });

    test('choosing a model applies it to the open session and reads its context window again', () async {
      session.contextWindow = const BridgeContext(tokens: 1000, window: 200000);
      await open();
      session.contextWindow = const BridgeContext(window: 1000000);

      await state.setModel(choiceOf('claude-haiku'));
      await pumpEventQueue();

      expect(session.calls, ['model claude-haiku']);
      expect(state.contextWindow.value?.window, 1000000);
    });

    test('a permission or model change the session refuses leaves what it still has', () async {
      session.chosen = choiceOf('claude-haiku');
      await open();
      session.modelError = 'the model is gone';
      await state.setModel(choiceOf('claude-opus-5-5'));
      expect(state.model.value, choiceOf('claude-haiku'));
      expect(
        state.notice.value?.text,
        'Could not apply the model selection: the model is gone',
      );
      session.permissionsError = 'the agent is busy';

      await state.setPermissions(BridgePermissions.supervised);

      expect(state.permissions.value, BridgePermissions.fullAccess);
      expect(
        state.notice.value?.text,
        'Could not change the permissions: the agent is busy',
      );

      session.permissionsError = null;
      await state.setPermissions(BridgePermissions.supervised);
      expect(state.permissions.value, BridgePermissions.supervised);
    });

    test('delete takes an open session back to the start page, and archive goes through the project', () async {
      await open();
      project.onlyInWorktree = true;

      expect(await state.onlySessionInWorktree('thread-1'), isTrue);
      expect(await state.delete('thread-1', worktree: true), isTrue);
      expect(state.session, isNull);
      await state.archive('thread-1', archived: true);

      expect(project.calls, [
        'open thread-1',
        'delete thread-1 worktree=true',
        'archive thread-1 true',
      ]);
    });

    test('setBranch points the session at its branch and rebinds Git to the checkout it now uses', () async {
      final moving = MovingSession();
      project.session = moving;
      await open();

      await state.setBranch('feature', worktree: '/worktrees/feature');

      expect(moving.calls, ['branch feature worktree=/worktrees/feature']);
      expect(state.git.value.backend, same(moving.moved));
    });

    test('a switch during setBranch keeps Git on its session', () async {
      final gate = Completer<void>();
      project.session = GatedActions()..gate = gate;
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
    test('a scope compares the turns it names, and falls back to the latest finished turn', () async {
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

    test('says when no turn has finished yet', () async {
      session.turnList = [
        turnOf(1, status: BridgeTurnStatus.running, checkpoint: false),
      ];
      await open();

      expect(state.changesProblem.value?.title, 'No finished turn yet');
      expect(state.shown.value, isNull);
    });

    test('the Uncommitted scope lists and shows the checkout, and re-reads it when its status changes', () async {
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

    test('a slower earlier read cannot replace the newer selection', () async {
      final older = await selectOverSlowerRead();

      older.complete();
      await pumpEventQueue();

      expect(state.shownScope.value, const OneTurn(2));
      expect(state.shown.value?.from.number, 2);
      expect(state.changed.value?.map((file) => file.path), ['turn-2-2.dart']);
    });

    test('a failed slower read of an earlier scope shows no error over the newer selection', () async {
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

    test('opening reads the statistics and the context window', () async {
      session.stats = sonnet(212000, 18300, 41);
      session.contextWindow = const BridgeContext(
        tokens: 68200,
        window: 200000,
        compactsAt: 180000,
      );

      await open();

      expect(state.statistics.value, session.stats);
      expect(state.contextWindow.value, session.contextWindow);
    });

    test('a model response or a turn ending re-reads the statistics', () async {
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

    test('a failed statistics read warns and leaves stats unknown', () async {
      final failing = FailingReads()
        ..statisticsError = 'the session file is unreadable';
      project.session = failing;

      await open();

      expect(state.statistics.value, isNull);
      expect(state.notice.value?.level, NoticeLevel.warning);
      expect(
        state.notice.value?.text,
        'Could not read the statistics: the session file is unreadable',
      );
    });

    test('a late statistics failure of the session left is not shown on the one open', () async {
      final gate = Completer<void>();
      final late = FailingReads()
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
      final failing = FailingReads()
        ..contextError = 'the model has no window size';
      project.session = failing;

      await open();

      expect(state.statistics.value, failing.stats);
      expect(state.contextWindow.value, isNull);
      expect(
        state.notice.value?.text,
        'Could not read the context window: the model has no window size',
      );
    });
  });

  group('notices', () {
    test('a failed Chat stream names the session\'s host, and is no notice as T3 keeps the thread open', () async {
      await open();

      session.fail('the stream broke');
      await pumpEventQueue();

      expect(state.notice.value, isNull);
      expect(state.sessionProblem.value, 'the stream broke.');
    });

    test('a failed send or open is a notice that names the action', () async {
      await open();
      session.sendError = 'the agent is busy';
      await state.send('Again');
      expect(
        state.notice.value?.text,
        'Could not send the message: the agent is busy',
      );

      project.openError = 'the session is gone';
      await state.open('thread-2');
      expect(
        state.notice.value?.text,
        'Could not open the session: the session is gone',
      );
    });

    test('a session that cannot subscribe is a notice', () async {
      project.session = UnsubscribableSession();

      await state.open('thread-1');
      await pumpEventQueue();

      expect(
        state.notice.value?.text,
        'Could not open the session: the stream is gone',
      );
    });

    test('a session list that cannot be read names its host and is no notice, as T3 reports it', () async {
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

    test('the servers, scheduled tasks, related threads, usage limits and host name follow the open session', () async {
      session
        ..serverList = [server(3000)]
        ..automationList = [automationOf('a1')]
        ..family = lineageOf()
        ..limits = BridgeUsageLimits(
          windows: [BridgeUsageWindow(label: '5-hour', remainingPercent: 42)],
        );
      await open();
      expect(state.servers.value.single.port, 3000);
      expect(state.automations.value.single.id, 'a1');
      expect(state.lineage.value?.rows, isEmpty);
      expect(state.limits.value?.windows.single.label, '5-hour');
      expect(state.hostName.value, 'Mac mini');
    });

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

    test('a failed live stream is a warning that names its section', () async {
      await open();

      session.serverUpdates.addError('the stream dropped');
      await pumpEventQueue();

      expect(state.notice.value?.level, NoticeLevel.warning);
      expect(
        state.notice.value?.text,
        'Could not follow the servers: the stream dropped',
      );
    });

    test('a switch stops following the session it left', () async {
      await open();
      await switchTo(FakeSession(id: 'thread-2'));

      session.serverUpdates.add([server(3000)]);
      await pumpEventQueue();
      expect(state.servers.value, isEmpty);
    });

    test('merging back opens the thread it merged into, as T3 does', () async {
      session.family = mergeTargetLineage();
      await open();
      project.session = FakeSession(id: 'thread-parent');

      await state.mergeBack();
      await pumpEventQueue();

      expect(session.calls, contains('merge back'));
      expect(project.calls, contains('open thread-parent'));
      expect(state.session?.id, 'thread-parent');
    });

    test('a switch during a merge back keeps the session the user chose, not the merge target', () async {
      final gate = Completer<void>();
      project.session = GatedActions()
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

    test('the card actions run on the open session', () async {
      await open();
      await state.stopServer(7, 3000);
      await state.setAutomationEnabled('a1', false);
      await state.runAutomation('a1');
      await state.mergeBack();
      await state.disconnect();
      await state.stopSubagent('sub-1');
      expect(
        session.calls,
        containsAll([
          'stop server 7 3000',
          'automation a1 enabled=false',
          'run automation a1',
          'merge back',
          'disconnect',
          'stop subagent sub-1',
        ]),
      );
    });

    test('a refused action is reported as the card names it', () async {
      session = RefusedActions();
      project.session = session;
      await open();
      await state.mergeBack();
      expect(state.notice.value?.text, 'Could not merge back: not ready');
    });
  });

  test('disposing a session releases its agent host project at once', () async {
    await open();
    expect(project.closed, isFalse);
    await state.dispose();
    expect(project.closed, isTrue);
  });

  group('agent hosts (ADR-0295)', () {
    const studioAddress = 'ws://studio.local:3774';
    FakeProject listing(String id) =>
        FakeProject()..threadList = [threadOf(id)];

    test('lists the sessions of every connected agent host, newest first, with the machine kind its host reports', () async {
      final state = SessionState(
        FakeProject()
          ..threadList = [threadOf('local', title: 'Local', updatedMs: 100)],
      );
      await state.addHost(
        studioAddress,
        FakeProject()
          ..threadList = [threadOf('studio', title: 'Studio', updatedMs: 200)],
        machine: 'mac-mini',
      );

      expect(
        [
          for (final listed in state.recent.value)
            (listed.address, listed.thread.title, listed.machine),
        ],
        [(studioAddress, 'Studio', 'mac-mini'), (null, 'Local', null)],
        reason: 'This Mac shows no machine icon',
      );
    });

    test('opens a session on the agent host it runs on', () async {
      final local = FakeProject();
      final studio = listing('studio');
      final state = SessionState(local);
      await state.addHost(studioAddress, studio);

      await state.open('studio', address: studioAddress);
      await pumpEventQueue();

      expect(studio.calls, ['open studio']);
      expect(local.calls, isEmpty);
      expect(state.sessionAddress, studioAddress);
    });

    test('a new session starts on the chosen agent host', () async {
      final local = FakeProject();
      final studio = FakeProject();
      final state = SessionState(local);
      await state.addHost(studioAddress, studio);

      await state.startOn(studioAddress);
      await state.send('Fix the login');
      await pumpEventQueue();

      expect(studio.calls.single, startsWith('start Fix the login'));
      expect(local.calls, isEmpty);
    });

    test('names a host that does not answer, and lists none of its sessions', () async {
      final state = SessionState(listing('local'));

      state.failHost(
        studioAddress,
        'Liquid could not connect to the agent host $studioAddress: no answer.',
      );
      await state.loadStart();

      expect(state.answers(studioAddress), isFalse);
      expect(
        state.hostProblems.value[studioAddress],
        'Liquid could not connect to the agent host $studioAddress: no answer.',
      );
      expect(
        [for (final listed in state.recent.value) listed.thread.id],
        ['local'],
      );
    });

    test('a host that stops answering is named, and its sessions leave the list', () async {
      final state = SessionState(listing('local'));
      await state.addHost(studioAddress, listing('studio'));

      await state.addHost(studioAddress, UnreachableProject());

      expect(
        state.hostProblems.value[studioAddress],
        'Liquid could not connect to the agent host $studioAddress: the host is down.',
      );
      expect(
        [for (final listed in state.recent.value) listed.thread.id],
        ['local'],
      );
    });

    test('an open session whose agent host stops answering stays open, and is reopened once it answers', () async {
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
      expect(
        state.sessionProblem.value,
        'Liquid could not connect to the agent host $studioAddress: the host is down.',
      );

      final back = FakeProject(session: session);
      await state.addHost(studioAddress, back);
      await pumpEventQueue();

      expect(back.calls, ['open studio']);
      expect(state.sessionProblem.value, isNull);
    });

    test('a listing that a later one overtakes does not hide the later one\'s hosts', () async {
      final gate = Completer<void>();
      final state = SessionState(
        LateFirstListing(gate.future)..threadList = [threadOf('local')],
      );

      final earlier = state.loadStart();
      await state.addHost(studioAddress, listing('studio'));
      gate.complete();
      await earlier;

      expect([
        for (final listed in state.recent.value) listed.thread.id,
      ], unorderedEquals(['local', 'studio']));
    });

    test('removing a host drops its sessions and releases it', () async {
      final studio = listing('studio');
      final state = SessionState(listing('local'));
      await state.addHost(studioAddress, studio);

      await state.removeHost(studioAddress);

      expect(
        [for (final listed in state.recent.value) listed.thread.id],
        ['local'],
      );
      expect(studio.closed, isTrue);
      expect(state.answers(studioAddress), isFalse);
    });

    group('moves (ADR-0308)', () {
      BridgeTranscriptItem entry(int id, BridgeItemKind kind, String text) =>
          BridgeTranscriptItem(
            item: BridgeItem(
              id: id,
              kind: kind,
              ended: true,
              failed: false,
              steps: const [],
            ),
            turn: 1,
            text: text,
          );

      test('a moved session shows its earlier transcript above the marker, and not the handoff it starts from', () async {
        final session = FakeSession(id: 'moved')
          ..current = snapshotOf(
            transcript: [
              entry(1, BridgeItemKind.userMessage, 'The handoff'),
              entry(2, BridgeItemKind.assistantMessage, 'Continuing'),
            ],
          );
        final target = FakeProject(session: session)
          ..moveRecordAnswer = BridgeMoveRecord(
            fromHost: 'This Mac',
            toHost: 'studio',
            atMs: 1700000000000,
            transcript: [
              entry(10, BridgeItemKind.userMessage, 'Fix the flaky test.'),
              entry(
                11,
                BridgeItemKind.assistantMessage,
                'It retries the port.',
              ),
            ],
          );
        final state = SessionState(FakeProject());
        await state.addHost(studioAddress, target);

        await state.open('moved', address: studioAddress);
        await pumpEventQueue();

        expect(
          [for (final row in state.items.value) row.marker ?? row.text.value],
          [
            'Fix the flaky test.',
            'It retries the port.',
            'Moved from This Mac to studio · ${formatClockMs(1700000000000)}',
            'Continuing',
          ],
        );
      });

      test('the marker names each host the way Liquid does: This Mac, and a paired host by its Settings name', () async {
        final session = FakeSession(id: 'moved')
          ..current = snapshotOf(
            transcript: [
              entry(1, BridgeItemKind.assistantMessage, 'Continuing'),
            ],
          );
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

        expect(
          [for (final row in state.items.value) row.marker ?? row.text.value],
          [
            'Moved from This Mac to Studio mini · ${formatClockMs(1700000000000)}',
            'Continuing',
          ],
        );
      });

      test('a session without a move record shows its Chat as it is', () async {
        final session = FakeSession(id: 'plain')
          ..current = snapshotOf(
            transcript: [entry(1, BridgeItemKind.userMessage, 'The question')],
          );
        final state = SessionState(FakeProject(session: session));

        await state.open('plain');
        await pumpEventQueue();

        expect(
          [for (final row in state.items.value) row.marker ?? row.text.value],
          ['The question'],
        );
      });
    });
  });
}

class RefusedActions extends FakeSession {
  @override
  Future<void> mergeBack() => Future.error('not ready');
}

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
