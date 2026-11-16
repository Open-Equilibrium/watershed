import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/rust/api/details.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

/// Holds its confirmations until `gate` completes, so one can be in flight when the session changes.
class GatedSession extends FakeSession {
  Completer<void>? gate;

  @override
  Future<void> confirmApplied(int applied) async {
    await gate?.future;
    await super.confirmApplied(applied);
  }
}

/// A session whose statistics or context read fails with the given error.
class FailingReads extends FakeSession {
  Object? statisticsError;
  Object? contextError;

  @override
  Future<BridgeStatistics> statistics() async {
    if (statisticsError case final error?) throw error;
    return super.statistics();
  }

  @override
  Future<BridgeContext?> context() async {
    if (contextError case final error?) throw error;
    return super.context();
  }
}

/// A session whose checkout becomes `moved` once its branch is changed.
class MovingSession extends FakeSession {
  final moved = FakeGit();

  @override
  Future<void> setBranch(String branch, {String? worktree}) async {
    await super.setBranch(branch, worktree: worktree);
    gitBackend = moved;
  }
}

class UnreachableProject extends FakeProject {
  @override
  Future<List<BridgeThreadSummary>> threads() =>
      Future.error('the host is down');
}

/// A session whose title read waits for `gate`, so a switch can happen while it is in flight.
class GatedTitle extends FakeSession {
  Completer<void>? gate;

  @override
  Future<String> title() async {
    await gate?.future;
    return super.title();
  }
}

/// A session whose stream cannot be subscribed to.
class UnsubscribableSession extends FakeSession {
  @override
  Future<BridgeSnapshot> subscribe() => Future.error('the stream is gone');
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

      expect(state.recent.value.map((thread) => thread.title), ['Fix login']);
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

    test(
      'a checked-out pull request starts its session in its worktree',
      () async {
        await state.loadStart();
        await state.draftFor(
          BridgeCheckedOut(branch: 'feature', worktree: '/worktrees/feature'),
        );

        await state.send('Review it');

        expect(project.calls, [
          'start Review it claude-opus-5-5 base=null worktree=/worktrees/feature fullAccess',
        ]);
      },
    );

    test('sending with no model is an error and starts nothing', () async {
      await state.send('Hello');

      expect(state.notice.value?.level, NoticeLevel.error);
      expect(state.notice.value?.text, 'No model is available.');
      expect(project.calls, isEmpty);
    });
  });

  group('a session', () {
    test(
      'send and steer reach the open session, and stop interrupts it',
      () async {
        await open();

        await state.send('Say hi');
        await state.send('Shorter', steer: true);
        await state.stop();

        expect(session.calls, ['send Say hi', 'steer Shorter', 'stop']);
      },
    );

    test('opening a session shows its title and the model it uses', () async {
      session.sessionTitle = 'Fix login';
      session.chosen = choiceOf('claude-haiku');

      await open();

      expect(state.sessionTitle.value, 'Fix login');
      expect(state.model.value, choiceOf('claude-haiku'));
    });

    test('a switch while the title is read keeps the title and model of the session it left out', () async {
      final gate = Completer<void>();
      project.session = GatedTitle()
        ..gate = gate
        ..sessionTitle = 'Old title'
        ..chosen = choiceOf('old-model');
      unawaited(state.open('thread-1'));
      await pumpEventQueue();

      project.session = FakeSession(id: 'thread-2')
        ..sessionTitle = 'New title'
        ..chosen = choiceOf('new-model');
      await state.open('thread-2');
      await pumpEventQueue();
      gate.complete();
      await pumpEventQueue();

      expect(state.sessionTitle.value, 'New title');
      expect(state.model.value, choiceOf('new-model'));
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

      await emit(
        event(
          BridgeEventKind.contentDelta,
          5,
          item: item(1, BridgeItemKind.assistantMessage),
          text: ' (held)',
        ),
      );
      await emit(
        event(
          BridgeEventKind.contentDelta,
          6,
          item: item(1, BridgeItemKind.assistantMessage),
          text: ' there',
        ),
      );

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

    test(
      'each event is confirmed once applied, skipped ones included (ADR-0272)',
      () async {
        await open(snapshotOf(sequence: 5));

        await emit(
          event(
            BridgeEventKind.contentDelta,
            4,
            item: item(1, BridgeItemKind.assistantMessage),
            text: 'held',
          ),
        );
        await emit(
          event(
            BridgeEventKind.contentDelta,
            6,
            item: item(1, BridgeItemKind.assistantMessage),
            text: 'new',
          ),
        );

        expect(session.confirmed, [1, 2]);
      },
    );

    test('a confirmation in flight when the session changes does not hold back the next session (ADR-0272)', () async {
      final gate = Completer<void>();
      final first = GatedSession()..gate = gate;
      project.session = first;
      await open();
      first.emit(
        event(
          BridgeEventKind.contentDelta,
          1,
          item: item(1, BridgeItemKind.assistantMessage),
          text: 'x',
        ),
      );
      await pumpEventQueue();

      final second = FakeSession(id: 'thread-2');
      project.session = second;
      await state.open('thread-2');
      await pumpEventQueue();
      second.emit(
        event(
          BridgeEventKind.contentDelta,
          1,
          item: item(1, BridgeItemKind.assistantMessage),
          text: 'y',
        ),
      );
      await pumpEventQueue();
      gate.complete();
      await pumpEventQueue();

      expect(second.confirmed, [1]);
    });

    test('an item streams its text in place and takes its outcome when it completes', () async {
      await open();
      await emit(
        event(
          BridgeEventKind.itemStarted,
          1,
          turn: 1,
          item: item(1, BridgeItemKind.assistantMessage),
        ),
      );
      final reply = state.items.value.single;
      await emit(
        event(
          BridgeEventKind.contentDelta,
          2,
          item: item(1, BridgeItemKind.assistantMessage),
          text: 'Hi',
        ),
      );
      await emit(
        event(
          BridgeEventKind.contentDelta,
          3,
          item: item(1, BridgeItemKind.assistantMessage),
          text: ' there',
        ),
      );
      expect(reply.text.value, 'Hi there');

      await emit(
        event(
          BridgeEventKind.itemStarted,
          4,
          turn: 1,
          item: item(2, BridgeItemKind.commandExecution, tool: 'bash'),
        ),
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

    test(
      'a turn ends with a row of its changes once its checkpoint is ready',
      () async {
        await open();
        session.turnList = [
          turnOf(1, added: 3, removed: 1, files: 2),
          turnOf(2, status: BridgeTurnStatus.running, checkpoint: false),
        ];

        await emit(
          event(
            BridgeEventKind.itemStarted,
            1,
            turn: 1,
            item: item(1, BridgeItemKind.assistantMessage),
          ),
        );
        await emit(event(BridgeEventKind.turnCompleted, 2, turn: 1));
        await emit(
          event(
            BridgeEventKind.itemStarted,
            3,
            turn: 2,
            item: item(2, BridgeItemKind.assistantMessage),
          ),
        );

        expect(
          [for (final row in state.items.value) row.turnEnd?.added],
          [null, 3, null],
          reason: 'only turn 1 has a checkpoint, so only it gets a row',
        );
      },
    );

    test(
      'requests queue oldest first, and answering one moves on to the next',
      () async {
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
          BridgeQuestionAnswer(question: 'Which?', values: ['B']),
        ]);
        await emit(event(BridgeEventKind.requestResolved, 5, requestId: 'q1'));
        expect(
          (state.request.value, state.status.value),
          (null, RunStatus.running),
        );
        expect(session.calls, ['respond a1 approve', 'answer q1 Which?=B']);
      },
    );

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
  });

  group('statistics and the context window', () {
    test('opening reads the statistics and the context window', () async {
      session.stats = BridgeStatistics(
        models: [
          BridgeModelUsage(
            model: 'anthropic/claude-sonnet',
            usage: usage(212000, 18300),
            responses: 41,
          ),
        ],
        tools: const [],
        turns: const [],
      );
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
      session.stats = BridgeStatistics(
        models: [
          BridgeModelUsage(
            model: 'anthropic/claude-sonnet',
            usage: usage(10, 1),
            responses: 1,
          ),
        ],
        tools: const [],
        turns: const [],
      );
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

    test(
      'a failed statistics read is a warning, and the statistics stay unknown',
      () async {
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
      },
    );

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

    test(
      'a session that cannot subscribe is a notice, not an unhandled error',
      () async {
        project.session = UnsubscribableSession();

        await state.open('thread-1');
        await pumpEventQueue();

        expect(
          state.notice.value?.text,
          'Could not open the session: the stream is gone',
        );
      },
    );

    test('a session list that cannot be read is a notice', () async {
      state = SessionState(UnreachableProject());

      await state.loadStart();

      expect(
        state.notice.value?.text,
        'Could not read the sessions: the host is down',
      );
    });

    test(
      'notices keep the last $noticeLimit, and dismissing the status bar notice keeps the list',
      () async {
        for (var index = 0; index <= noticeLimit; index++) {
          state.notify(NoticeLevel.info, 'Notice $index');
        }
        expect(state.notices.value, hasLength(noticeLimit));
        expect(state.notices.value.first.text, 'Notice 1');

        state.dismissNotice();

        expect(state.notice.value, isNull);
        expect(state.notices.value, hasLength(noticeLimit));
      },
    );
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
        ..automationList = [
          BridgeAutomation(
            id: 'a1',
            title: 'Check',
            schedule: BridgeSchedule(
              kind: BridgeScheduleKind.webhook,
              weekdays: Int64List(0),
            ),
            enabled: true,
            lastRun: BridgeRunStatus.never,
          ),
        ]
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

      session.serverUpdates.add([server(3000), server(4000)]);
      await pumpEventQueue();
      expect(state.servers.value, hasLength(2));
    });

    test('a switch stops following the session it left', () async {
      await open();
      final other = FakeSession(id: 'thread-2');
      project.session = other;
      await state.open('thread-2');
      await pumpEventQueue();

      session.serverUpdates.add([server(3000)]);
      await pumpEventQueue();
      expect(state.servers.value, isEmpty);
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
}

/// A session that refuses to merge back.
class RefusedActions extends FakeSession {
  @override
  Future<void> mergeBack() => Future.error('not ready');
}
