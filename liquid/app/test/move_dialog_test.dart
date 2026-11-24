import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/move_dialog.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/format.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/rust/api/thread.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/move.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';
const build = 'ws://build.local:3774';
const info = SessionInfo(project: 'acme-api');

const otherRemote =
    "This project's Git remote is not the session's remote, so the session cannot move here.";
const noRemote =
    'This project has no Git remote that T3 can resolve, so the session cannot move here.';
const landingBlocked =
    'This project has commits that are not pushed, so the session cannot continue there.';

const matching = BridgeHostProject(
  root: '/home/me/acme-api',
  title: 'acme-api',
  remoteUrl: 'git@github.com:acme/api.git',
);
const sameRemote = BridgeHostProject(
  root: '/srv/acme-api',
  title: 'acme-api',
  remoteUrl: 'git@github.com:acme/api.git',
);
const otherRemoteProject = BridgeHostProject(
  root: '/srv/shop',
  title: 'shop',
  remoteUrl: 'git@github.com:acme/shop.git',
);

class HeldProject extends FakeProject {
  HeldProject() {
    gates['moveIn'] = gate;
  }

  final gate = Completer<void>();
}

class ClosingProject extends HeldProject {
  @override
  Future<SessionBackend> moveIn(
    CarriedMove move, {
    required String root,
  }) async {
    await gate.future;
    if (closed) throw 'the connection to the host is closed';
    return super.moveIn(move, root: root);
  }
}

class ListedMoves extends FakeProject {
  ListedMoves() : super(session: FakeSession(id: 'local'));

  @override
  Future<List<BridgeThreadSummary>> threads() async => [
    threadOf('local', title: 'Local session', moving: moving),
  ];
}

class UnreadableSource extends FakeProject {
  bool _movedOut = false;

  @override
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  }) {
    _movedOut = true;
    return super.moveOut(
      id,
      transfer: transfer,
      targetAddress: targetAddress,
      targetName: targetName,
      targetEnvironment: targetEnvironment,
      commit: commit,
    );
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async {
    if (toHost != null) throw 'the connection dropped';
    return super.moveFinish(id, transfer, toHost);
  }

  @override
  Future<BridgeMoveCheck> moveCheck(String id) async {
    if (_movedOut) throw 'the source is unreachable';
    return super.moveCheck(id);
  }
}

Future<MoveModel> modelOf({
  BridgeMoveCheck? check,
  List<BridgeHostProject> projects = const [matching],
  List<BridgeConfigChange> changes = const [],
  FakeProject? studioProject,
  String sourceName = 'This Mac',
  Map<String, BridgeLanding> landings = const {},
}) async {
  final source = FakeProject()..moveCheckAnswer = check ?? moveCheckOf();
  final target = (studioProject ?? FakeProject())
    ..projectList = projects
    ..configChanges = changes
    ..landings = landings;
  final model = MoveModel(
    source: source,
    sourceAddress: null,
    threadId: 'thread-1',
    sourceName: sourceName,
    candidates: [
      MoveCandidate(address: studio, name: 'studio', project: target),
      const MoveCandidate(address: build, name: 'build-box'),
    ],
    open: (address, root) async => address == studio ? target : source,
    running: <String>{},
    started: <String>{},
  );
  await model.load();
  return model;
}

Widget dialogOf(
  MoveModel model, {
  void Function(SessionBackend, String?)? onMoved,
  void Function(MoveFailure)? onFailed,
}) => MaterialApp(
  theme: liquidTheme(Brightness.light),
  home: Scaffold(
    body: MoveDialog(
      model: model,
      onMoved: onMoved ?? (_, _) {},
      onUnconfirmed: (_, _) {},
      onFailed: onFailed ?? (_) {},
    ),
  ),
);

PrimaryButton moveButton(WidgetTester tester) =>
    tester.widget<PrimaryButton>(find.widgetWithText(PrimaryButton, 'Move'));

Future<void> openPicker(WidgetTester tester) async {
  await tester.tap(find.text('Use a project'));
  await tester.pump();
  await tester.tap(find.byKey(const Key('move-project-field')));
  await tester.pumpAndSettle();
}

Future<void> addFolder(WidgetTester tester, String path) async {
  await tester.enterText(find.byKey(const Key('add-folder-path')), path);
  await tester.tap(find.byKey(const Key('add-folder-submit')));
  await tester.pumpAndSettle();
}

Future<void> pumpSession(WidgetTester tester, SessionState state) async {
  await tester.binding.setSurfaceSize(const Size(1400, 900));
  await tester.pumpWidget(
    LiquidApp(launch: ValueNotifier(Launch.session(state, info))),
  );
  await tester.pumpAndSettle();
}

Future<SessionState> localSession(
  WidgetTester tester, {
  FakeProject? source,
  Map<String, FakeProject> hosts = const {},
}) async {
  final state = SessionState(
    source ??
        (FakeProject(session: FakeSession(id: 'local'))
          ..threadList = [threadOf('local', title: 'Local session')]),
  );
  await state.open('local');
  state.openProject = (address, root) async => state.projectOf(address)!;
  for (final host in hosts.entries) {
    await state.addHost(host.key, host.value);
  }
  await pumpSession(tester, state);
  return state;
}

Future<void> openLocalActions(WidgetTester tester) async {
  await tester.tap(
    find.descendant(
      of: find.byKey(const Key('session-local')),
      matching: find.byTooltip('More actions'),
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> chooseMoveToHost(WidgetTester tester) async {
  await openLocalActions(tester);
  await tester.tap(find.text('Move to host…'));
  await tester.pumpAndSettle();
}

Future<void> tapMove(WidgetTester tester) async {
  await tester.tap(find.widgetWithText(PrimaryButton, 'Move'));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    "an agent's row moves it, and the dialog names what moves with it",
    (tester) async {
      final source = FakeProject(session: FakeSession(id: 'local'))
        ..threadList = [threadOf('local', title: 'news-bot')]
        ..agentList = [
          agentOf('news-bot', thread: 'local', threads: ['local', 'older']),
        ]
        ..moveCheckAnswer = moveCheckOf(
          agent: const BridgeAgentMove(
            name: 'news-bot',
            memoryBytes: 3000,
            routines: 1,
            earlier: 1,
          ),
        );
      final state = SessionState(source);
      await state.loadStart();
      state.openProject = (address, root) async => state.projectOf(address)!;
      await state.addHost(studio, FakeProject()..hostName = 'studio');
      await pumpSession(tester, state);

      await tester.tap(
        find.descendant(
          of: find.byKey(const Key('agent-agent-news-bot')),
          matching: find.byTooltip('More actions'),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to host…'));
      await tester.pumpAndSettle();

      expect(
        find.text('Move agent “news-bot” to another host'),
        findsOneWidget,
      );
      expect(
        find.text('Memory folder, 3 KB, is copied (at most 100 MiB)'),
        findsOneWidget,
      );
      expect(
        find.text(
          '1 routine is made again there; a webhook routine gets a new URL',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('1 earlier thread stay'), findsOneWidget);
    },
  );

  testWidgets(
    'a row that does not fit offers its fixes, and Move stays enabled',
    (tester) async {
      final model = await modelOf(
        check: moveCheckOf(uncommittedFiles: 3),
        projects: const [],
        changes: const [
          BridgeConfigChange(
            kind: BridgeConfigKind.skill,
            name: 'review',
            direction: BridgeDirection.add,
          ),
          BridgeConfigChange(
            kind: BridgeConfigKind.instruction,
            name: 'AGENTS.md',
            direction: BridgeDirection.replace,
          ),
        ],
      );
      await tester.pumpWidget(dialogOf(model));

      for (final text in [
        '3 files have uncommitted changes',
        'Commit and push',
        'Leave on This Mac',
        'No project on studio has this remote',
        'Clone to',
        'Use a project',
        "studio's configuration differs: 1 skill, AGENTS.md",
        "Use studio's",
        "Bring This Mac's",
      ]) {
        expect(find.text(text), findsOneWidget, reason: text);
      }
      await tester.tap(find.text('Show'));
      await tester.pump();
      expect(find.textContaining('review (skill, add)'), findsOneWidget);
      expect(
        find.textContaining('AGENTS.md (instruction, replace)'),
        findsOneWidget,
      );
      expect(moveButton(tester).onPressed, isNotNull);
    },
  );

  testWidgets(
    'an agent host that does not answer is listed dimmed and cannot be chosen',
    (tester) async {
      final model = await modelOf();
      await tester.pumpWidget(dialogOf(model));

      expect(find.text('Offline'), findsOneWidget);
      await tester.tap(find.text('build-box'));
      await tester.pump();

      expect(model.selected?.candidate.name, 'studio');
    },
  );

  testWidgets(
    'progress lists each step as it runs, and Move stays dimmed until the move ends',
    (tester) async {
      final held = HeldProject();
      final model = await modelOf(
        check: moveCheckOf(uncommittedFiles: 3),
        studioProject: held,
      );
      SessionBackend? moved;
      String? movedTo;
      await tester.pumpWidget(
        dialogOf(
          model,
          onMoved: (session, address) {
            moved = session;
            movedTo = address;
          },
        ),
      );

      await tester.tap(find.widgetWithText(PrimaryButton, 'Move'));
      await tester.pump();

      expect(find.text('Moving to studio'), findsOneWidget);
      expect(
        find.text('Committed 3 files and pushed fix/ports to its remote'),
        findsOneWidget,
      );
      expect(find.text('Starting the session on studio…'), findsOneWidget);
      expect(moveButton(tester).onPressed, isNull);

      held.gate.complete();
      await tester.pumpAndSettle();

      expect(moved, isNotNull);
      expect(movedTo, studio);
    },
  );

  final dimmedReasons = <(String, Future<MoveModel> Function(), String)>[
    (
      'a target project whose branch cannot land shows the reason before Move, and Move dims with it when no other project can take the session',
      () => modelOf(
        landings: const {
          '/home/me/acme-api': BridgeLanding(
            kind: BridgeLandingKind.blocked,
            reason: landingBlocked,
          ),
        },
      ),
      landingBlocked,
    ),
  ];
  for (final (name, build, reason) in dimmedReasons) {
    testWidgets(name, (tester) async {
      final model = await build();
      await tester.pumpWidget(dialogOf(model));

      expect(find.text(reason), findsOneWidget);
      expect(find.byTooltip(reason), findsOneWidget);
      expect(moveButton(tester).onPressed, isNull);
    });
  }

  testWidgets(
    'an agent host that predates moves is listed dimmed with its reason, and no host that can take the session says so',
    (tester) async {
      final old = FakeProject()..movesSupported = false;
      final model = await modelOf(studioProject: old);
      await tester.pumpWidget(dialogOf(model));

      expect(find.text('Needs a newer Meta-Harness'), findsOneWidget);
      expect(
        find.text('No other agent host can take this session.'),
        findsOneWidget,
      );
      await tester.tap(find.text('studio'));
      await tester.pump();
      expect(model.selected, isNull);
    },
  );

  testWidgets(
    'a host dropped from the list while a move is under way does not stop the move',
    (tester) async {
      final shared = ClosingProject()..hostName = 'studio';
      final own = ClosingProject()..hostName = 'studio';
      final state = await localSession(tester, hosts: {studio: shared});
      state.openProject = (address, root) async =>
          address == studio ? own : state.projectOf(address)!;

      await chooseMoveToHost(tester);
      await tester.tap(find.widgetWithText(PrimaryButton, 'Move'));
      await tester.pump();
      state.failHost(studio, 'the host is down');
      own.gate.complete();
      await tester.pumpAndSettle();

      expect(shared.closed, isTrue, reason: 'the shared handle was dropped');
      expect(
        state.notice.value?.text,
        'Moved “Fix the flaky port test” to studio.',
      );
      expect(state.notice.value?.level, NoticeLevel.info);
    },
  );

  testWidgets(
    'a blocked project lets the user choose another project of the same remote that can take the session',
    (tester) async {
      final model = await modelOf(
        projects: const [matching, sameRemote],
        landings: const {
          '/home/me/acme-api': BridgeLanding(
            kind: BridgeLandingKind.blocked,
            reason: 'uncommitted changes',
          ),
        },
      );
      await tester.pumpWidget(dialogOf(model));

      expect(model.plan?.root, '/srv/acme-api');
      expect(moveButton(tester).onPressed, isNotNull);
    },
  );

  testWidgets(
    'a project the user picks that cannot take the session blocks Move, even when another project of the same remote could',
    (tester) async {
      final model = await modelOf(
        projects: const [matching, sameRemote],
        landings: const {
          '/home/me/acme-api': BridgeLanding(
            kind: BridgeLandingKind.blocked,
            reason: landingBlocked,
          ),
        },
      );
      model.pick(matching);
      await tester.pumpWidget(dialogOf(model));

      expect(model.plan?.root, '/home/me/acme-api');
      expect(moveButton(tester).onPressed, isNull);
      expect(find.byTooltip(landingBlocked), findsOneWidget);
    },
  );

  testWidgets(
    'a move that cannot be confirmed says so in a warning, and shows the session on the target',
    (tester) async {
      final state = await localSession(
        tester,
        source: UnreadableSource()
          ..threadList = [threadOf('local', title: 'Local session')],
        hosts: {
          studio: FakeProject(session: FakeSession(id: 'moved'))
            ..hostName = 'studio',
        },
      );

      await chooseMoveToHost(tester);
      await tester.tap(find.widgetWithText(PrimaryButton, 'Move'));
      await tester.pumpAndSettle();

      expect(state.notice.value?.level, NoticeLevel.warning);
      expect(
        state.notice.value?.text,
        'The move of “Fix the flaky port test” to studio is not finished yet. Liquid finishes it when This Mac answers.',
      );
      expect(state.sessionAddress, studio);
    },
  );

  testWidgets(
    'an unconfirmed move reloads the listing, so recovery asks the target at once',
    (tester) async {
      final source = ListedMoves();
      final target = FakeProject()
        ..moveInLostReplies = 3
        ..startingThread = true
        ..hostName = 'studio';
      await localSession(tester, source: source, hosts: {studio: target});

      await chooseMoveToHost(tester);
      await tapMove(tester);

      expect(
        target.calls.where((call) => call.startsWith('move status ')),
        isNotEmpty,
        reason: 'the reload ran recovery, which asked the target at once',
      );
      expect(
        source.moving,
        isNotNull,
        reason: 'the target still starts the session, so the move stays open',
      );
      expect(
        source.calls.where((call) => call.startsWith('move finish')),
        isEmpty,
        reason: 'nothing is released while the target answers starting',
      );
    },
  );

  testWidgets(
    'a moved session shows its carried transcript above the marker, and not the handoff',
    (tester) async {
      BridgeTranscriptItem entry(int id, BridgeItemKind kind, String text) =>
          BridgeTranscriptItem(
            item: BridgeItem(
              id: id,
              kind: kind,
              ended: true,
              failed: false,
              steps: const [],
              opening: BridgeOpening.none,
            ),
            turn: 1,
            text: text,
          );
      final session = FakeSession(id: 'moved')
        ..current = snapshotOf(
          transcript: [
            entry(1, BridgeItemKind.userMessage, 'The handoff'),
            entry(2, BridgeItemKind.assistantMessage, 'Continuing here'),
          ],
        );
      final target = FakeProject(session: session)
        ..moveRecordAnswer = BridgeMoveRecord(
          fromHost: 'This Mac',
          toHost: 'studio',
          atMs: 1700000000000,
          transcript: [
            entry(10, BridgeItemKind.userMessage, 'Please fix the flaky test.'),
          ],
        );
      final state = SessionState(FakeProject());
      await state.addHost(studio, target);
      await state.open('moved', address: studio);
      await pumpSession(tester, state);

      expect(
        find.text(
          'Moved from This Mac to studio · ${formatClockMs(1700000000000)}',
        ),
        findsOneWidget,
      );
      expect(find.text('Please fix the flaky test.'), findsOneWidget);
      expect(find.text('The handoff'), findsNothing);
      expect(find.textContaining('Continuing here'), findsOneWidget);
    },
  );

  testWidgets(
    'Cancel during the move closes the dialog, and the move finishes in the background with its result in the status bar',
    (tester) async {
      final held = HeldProject()..hostName = 'studio';
      final state = await localSession(tester, hosts: {studio: held});

      await chooseMoveToHost(tester);
      await tester.tap(find.widgetWithText(PrimaryButton, 'Move'));
      await tester.pump();
      expect(find.text('Moving to studio'), findsOneWidget);

      await tester.tap(find.widgetWithText(SecondaryButton, 'Cancel'));
      await tester.pumpAndSettle();
      expect(
        find.text('Moving to studio'),
        findsNothing,
        reason: 'the dialog closed',
      );

      held.gate.complete();
      await tester.pumpAndSettle();

      expect(
        state.notice.value?.text,
        'Moved “Fix the flaky port test” to studio.',
      );
      expect(state.notice.value?.level, NoticeLevel.info);
      expect(state.sessionAddress, studio);
    },
  );

  testWidgets(
    'a target host that cannot say its projects or configuration says so in the dialog, and Move stays off',
    (tester) async {
      final target = FakeProject()..hostProjectsError = 'the host is busy';
      final model = await modelOf(studioProject: target);
      await tester.pumpWidget(dialogOf(model));

      expect(
        find.text(
          "Could not read studio's projects or configuration: the host is busy.",
        ),
        findsOneWidget,
      );
      expect(moveButton(tester).onPressed, isNull);
    },
  );

  testWidgets(
    'Enter on a focused host row chooses it and never starts the move',
    (tester) async {
      final model = await modelOf();
      await tester.pumpWidget(dialogOf(model));

      Focus.of(tester.element(find.text('studio'))).requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();

      expect(model.flow, isNull, reason: 'the move did not start');
      expect(model.selected?.candidate.name, 'studio');
    },
  );

  testWidgets(
    'Enter on a focused fix segment chooses it and never starts the move',
    (tester) async {
      final model = await modelOf(
        check: moveCheckOf(uncommittedFiles: 3),
        projects: const [],
      );
      await tester.pumpWidget(dialogOf(model));

      Focus.of(tester.element(find.text('Use a project'))).requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();

      expect(model.useProject, isTrue);
      expect(model.flow, isNull, reason: 'the move did not start');
    },
  );

  testWidgets(
    'a project with another remote is listed dimmed, says why in its row, and cannot be chosen',
    (tester) async {
      final model = await modelOf(projects: const [otherRemoteProject]);
      await tester.pumpWidget(dialogOf(model));

      await openPicker(tester);
      expect(
        find.text(otherRemote),
        findsOneWidget,
        reason: 'the reason is in the row',
      );
      expect(find.byTooltip(otherRemote), findsOneWidget);
      await tester.tap(find.byKey(const Key('pick-/srv/shop')));
      await tester.pumpAndSettle();

      expect(
        find.text('Choose a project on studio'),
        findsOneWidget,
        reason: 'the picker stays open',
      );
      expect(model.picked, isNull);
      expect(
        moveButton(tester).onPressed,
        isNotNull,
        reason: 'Move stays available',
      );
    },
  );

  testWidgets(
    "Add folder makes an existing folder of the target a project, which can be chosen when its remote is the session's",
    (tester) async {
      final target = FakeProject()
        ..addedProject = const BridgeHostProject(
          root: '/home/me/acme-api-copy',
          title: 'acme-api-copy',
          remoteUrl: 'git@github.com:acme/api.git',
        );
      final model = await modelOf(projects: const [], studioProject: target);
      await tester.pumpWidget(dialogOf(model));

      await openPicker(tester);
      await addFolder(tester, '/home/me/acme-api-copy');

      expect(target.calls, contains('add project /home/me/acme-api-copy'));
      await tester.tap(find.byKey(const Key('pick-/home/me/acme-api-copy')));
      await tester.pumpAndSettle();
      expect(model.picked?.root, '/home/me/acme-api-copy');
      expect(model.plan?.root, '/home/me/acme-api-copy');
      expect(moveButton(tester).onPressed, isNotNull);
    },
  );

  testWidgets(
    'Change… opens the same picker when several projects match, and the one picked is where the session continues',
    (tester) async {
      final model = await modelOf(projects: const [matching, sameRemote]);
      await tester.pumpWidget(dialogOf(model));

      expect(
        find.text('Project on studio: /home/me/acme-api, same remote'),
        findsOneWidget,
      );
      await tester.tap(find.text('Change…'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('pick-/srv/acme-api')));
      await tester.pumpAndSettle();

      expect(
        find.text('Project on studio: /srv/acme-api, same remote'),
        findsOneWidget,
      );
      expect(model.plan?.root, '/srv/acme-api');
      expect(moveButton(tester).onPressed, isNotNull);
    },
  );

  testWidgets('Use a project with no project picked moves as Clone to', (
    tester,
  ) async {
    final model = await modelOf(projects: const []);
    await tester.pumpWidget(dialogOf(model));

    await tester.tap(find.text('Use a project'));
    await tester.pump();

    expect(model.plan?.cloneRemote, 'git@github.com:acme/api.git');
    expect(moveButton(tester).onPressed, isNotNull);
  });

  test(
    'a folder added to a project whose landing is blocked keeps Move off',
    () async {
      final target = FakeProject()
        ..addedProject = const BridgeHostProject(
          root: '/home/me/acme-api-copy',
          title: 'acme-api-copy',
          remoteUrl: 'git@github.com:acme/api.git',
        );
      final model = await modelOf(
        projects: const [],
        studioProject: target,
        landings: const {
          '/home/me/acme-api-copy': BridgeLanding(
            kind: BridgeLandingKind.blocked,
            reason: 'The branch has uncommitted changes there.',
          ),
        },
      );

      await model.addFolder('/home/me/acme-api-copy');

      expect(model.chosenRoot, '/home/me/acme-api-copy');
      expect(model.canMove, isFalse);
      expect(model.blockedReason, 'The branch has uncommitted changes there.');
    },
  );

  test(
    'a folder added for a host that is no longer chosen is not listed',
    () async {
      final added = _SlowAddFolder()
        ..addedProject = const BridgeHostProject(
          root: '/home/me/acme-api-copy',
          title: 'acme-api-copy',
          remoteUrl: 'git@github.com:acme/api.git',
        );
      final model = MoveModel(
        source: FakeProject()..moveCheckAnswer = moveCheckOf(),
        sourceAddress: null,
        threadId: 'thread-1',
        sourceName: 'This Mac',
        candidates: [
          MoveCandidate(address: studio, name: 'studio', project: added),
          MoveCandidate(
            address: build,
            name: 'build-box',
            project: FakeProject(),
          ),
        ],
        open: (address, root) async => FakeProject(),
        running: <String>{},
        started: <String>{},
      );
      await model.load();

      final adding = model.addFolder('/home/me/acme-api-copy');
      await model.select(
        model.options.firstWhere(
          (option) => option.candidate.name == 'build-box',
        ),
      );
      added.gate.complete();
      await adding;

      expect(model.projects.map((project) => project.root), isEmpty);
    },
  );

  test(
    'a second start while a move runs is refused, and the first still moves',
    () async {
      final target = HeldProject();
      final model = await modelOf(studioProject: target);

      final first = model.start();
      await expectLater(model.start(), throwsStateError);
      target.gate.complete();
      await first;

      expect(
        target.calls.where((call) => call.startsWith('move in')),
        hasLength(1),
      );
    },
  );
}

class _SlowAddFolder extends FakeProject {
  _SlowAddFolder() {
    gates['addProject'] = gate;
  }

  final gate = Completer<void>();
}
