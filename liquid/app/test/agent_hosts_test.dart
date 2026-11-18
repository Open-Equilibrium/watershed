import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/agent_hosts/agent_hosts.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/workspace/workspace_state.dart';
import 'package:liquid/src/rust/api/hosts.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/machine_icon.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'fake_host.dart';
import 'fake_workspace.dart';

const remote = 'ws://studio.local:3774';
const client = 'liquid-studio-s-mac';
const info = SessionInfo(project: 'acme-api');
const mini = SavedAgentHost(
  name: 'Studio mini',
  address: remote,
  client: client,
);
const miniActive = SavedAgentHost(
  name: 'Studio mini',
  address: remote,
  client: client,
  active: true,
);

class FakeAgentHosts implements AgentHostBridge {
  final reaches = <String?, BridgeReach>{};
  final calls = <String>[];
  Object? pairError;
  Object? forgetError;

  @override
  Future<BridgePairing> pair(String address, String code) async {
    calls.add('pair $address $code');
    if (pairError case final error?) throw error;
    return BridgePairing(
      host: address == remote ? 'Studio mini' : 'Lab',
      client: client,
    );
  }

  @override
  Future<void> forget(String address) async {
    calls.add('forget $address');
    if (forgetError case final error?) throw error;
  }

  @override
  Future<BridgeReach> reach(String? address) async {
    calls.add('reach ${address ?? 'local'}');
    return reaches[address] ?? BridgeReach.offline;
  }
}

class FakeLinks implements AgentHostLinks {
  FakeLinks(this.started, {this.startFailure, this.starting});

  final List<String?> started;
  final String? startFailure;

  final Future<String?> Function(String? address)? starting;

  @override
  Future<void> connect(String? address) async {}

  @override
  void disconnect(String address) {}

  @override
  Future<String?> startOn(String? address) async {
    started.add(address);
    if (starting case final starting?) return starting(address);
    return startFailure;
  }
}

void main() {
  late FakeAgentHosts bridge;
  late List<AgentHostsSettings> saved;
  late List<String?> switched;

  AgentHostsController controller({
    AgentHostsSettings settings = const AgentHostsSettings(),
    String? switchFailure,
    Future<String?> Function(String? address)? starting,
    Future<void> Function(AgentHostsSettings next)? save,
  }) => AgentHostsController(
    settings: settings,
    bridge: bridge,
    save: save ?? (next) async => saved.add(next),
    links: FakeLinks(switched, startFailure: switchFailure, starting: starting),
  );

  setUp(() {
    bridge = FakeAgentHosts()
      ..reaches[null] = BridgeReach.connected
      ..reaches[remote] = BridgeReach.connected;
    saved = [];
    switched = [];
  });

  Future<void> openAgentHosts(WidgetTester tester) async {
    await tester.sendKeyDownEvent(
      LogicalKeyboardKey.metaLeft,
      platform: 'macos',
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.comma, platform: 'macos');
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft, platform: 'macos');
    await tester.pumpAndSettle();
    await tester.tap(find.text('Agent hosts').first);
    await tester.pumpAndSettle();
  }

  Future<void> show(WidgetTester tester, AgentHostsController hosts) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    final shortcuts = await ShortcutController.load(
      MemoryStore(ShortcutSettings.standard()),
      FakeHost(),
    );
    final state = SessionState(FakeProject());
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: state,
          info: info,
          shortcuts: shortcuts,
          agentHosts: hosts,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await openAgentHosts(tester);
  }

  Future<void> pumpSessions(WidgetTester tester, SessionState state) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    await tester.pumpWidget(
      LiquidApp(launch: ValueNotifier(Launch.session(state, info))),
    );
    await tester.pumpAndSettle();
  }

  Future<void> chooseInRow(
    WidgetTester tester,
    String? address,
    String item,
  ) async {
    await tester.tap(
      find.descendant(
        of: find.byKey(ValueKey<String?>(address)),
        matching: find.byTooltip('More actions'),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text(item));
    await tester.pumpAndSettle();
  }

  Finder usedFor(String? address) => find.descendant(
    of: find.byKey(ValueKey<String?>(address)),
    matching: find.text('Used for sessions'),
  );

  test(
    'keeps the saved hosts, one active at most, and reads back what it wrote',
    () {
      final settings = AgentHostsSettings([
        const SavedAgentHost(
          name: 'Studio mini',
          address: remote,
          client: client,
          active: true,
        ),
        const SavedAgentHost(
          name: 'Lab',
          address: 'ws://lab:3774',
          client: client,
        ),
      ]);
      final lab = settings.withActive('ws://lab:3774');
      expect(
        AgentHostsSettings.fromJson(settings.toJson()).activeAddress,
        remote,
      );
      expect(lab.activeAddress, 'ws://lab:3774');
      expect(lab.hosts.where((host) => host.active), hasLength(1));
      expect(settings.withActive(null).activeAddress, isNull);
      expect(settings.without(remote).activeAddress, isNull);
      expect(
        AgentHostsSettings.fromJson({
          'hosts': [
            {'name': 'no address'},
            3,
            {'name': 'Lab', 'address': 'ws://lab:3774', 'client': client},
          ],
        }).hosts.map((host) => host.address),
        ['ws://lab:3774'],
        reason: 'malformed entries are dropped',
      );
      expect(AgentHostsSettings.fromJson({'hosts': 'x'}).hosts, isEmpty);
    },
  );

  testWidgets(
    'lists This Mac first with each host’s status and marks the one sessions use',
    (tester) async {
      bridge.reaches[remote] = BridgeReach.offline;
      await show(
        tester,
        controller(settings: const AgentHostsSettings([miniActive])),
      );

      expect(find.text('This Mac'), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('This Mac')).dy,
        lessThan(tester.getTopLeft(find.text('Studio mini')).dy),
      );
      expect(find.text('Connected'), findsOneWidget);
      expect(find.text('$remote · Offline'), findsOneWidget);
      expect(find.text('Used for sessions'), findsOneWidget);
      expect(bridge.calls, containsAll(['reach local', 'reach $remote']));
    },
  );

  testWidgets(
    'adds a host from its address and pairing code, and says when nothing changed',
    (tester) async {
      bridge.pairError = AnyhowException(
        'That pairing code is wrong, already used or expired.',
      );
      await show(tester, controller());

      await tester.tap(find.widgetWithText(GhostButton, 'Add agent host'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField).at(0), remote);
      await tester.enterText(find.byType(TextField).at(1), 'AAAA-AAAA');
      await tester.tap(find.widgetWithText(PrimaryButton, 'Add agent host'));
      await tester.pumpAndSettle();

      expect(
        find.text(
          'Could not add the agent host; nothing changed in Liquid. That pairing code is wrong, already used or expired.',
        ),
        findsOneWidget,
      );
      expect(saved, isEmpty);

      bridge.pairError = null;
      await tester.tap(find.widgetWithText(PrimaryButton, 'Add agent host'));
      await tester.pumpAndSettle();

      expect(
        find.text('Add Agent Host'),
        findsNothing,
        reason: 'the dialog closes',
      );
      expect(find.text('Studio mini'), findsOneWidget);
      expect(bridge.calls, contains('pair $remote AAAA-AAAA'));
      expect(saved.single.activeAddress, isNull);
      expect(saved.single.hosts.single.address, remote);
      expect(saved.single.hosts.single.client, client);
    },
  );

  testWidgets(
    'names the add dialog fields, focuses Host, and adds on Enter in the code',
    (tester) async {
      final semantics = tester.ensureSemantics();
      await show(tester, controller());
      await tester.tap(find.widgetWithText(GhostButton, 'Add agent host'));
      await tester.pumpAndSettle();

      expect(
        tester.getSemantics(find.bySemanticsLabel(RegExp('^Host'))),
        matchesSemantics(
          isTextField: true,
          isFocused: true,
          isFocusable: true,
          hasEnabledState: true,
          isEnabled: true,
          hasFocusAction: true,
          hasTapAction: true,
          hasSetTextAction: true,
          hasSetSelectionAction: true,
        ),
      );
      expect(
        tester.getSemantics(find.bySemanticsLabel(RegExp('^Pairing code'))),
        matchesSemantics(
          isTextField: true,
          isFocusable: true,
          hasEnabledState: true,
          isEnabled: true,
          hasFocusAction: true,
          hasTapAction: true,
        ),
      );

      await tester.enterText(find.byType(TextField).at(0), remote);
      await tester.enterText(find.byType(TextField).at(1), 'AAAA-AAAA');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(bridge.calls, contains('pair $remote AAAA-AAAA'));
      expect(find.text('Add Agent Host'), findsNothing);
      semantics.dispose();
    },
  );

  testWidgets('checks the hosts again every 10 s and when the app comes back', (
    tester,
  ) async {
    await show(tester, controller(settings: AgentHostsSettings([mini])));
    int checks(String? address) => bridge.calls
        .where((call) => call == 'reach ${address ?? 'local'}')
        .length;
    expect(checks(null), 1);

    await tester.pump(const Duration(seconds: 10));
    expect(checks(null), 2);
    expect(checks(remote), 2);

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    expect(checks(null), 3);
    expect(checks(remote), 3);
  });

  testWidgets(
    'the Add dialog shows the ws:// address refusal from the bridge',
    (tester) async {
      bridge.pairError = AnyhowException(
        'Enter the address as ws://HOST:PORT, as meta invite prints it.',
      );
      await show(tester, controller());
      await tester.tap(find.widgetWithText(GhostButton, 'Add agent host'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField).at(0), '127.0.0.1:3774');
      await tester.enterText(find.byType(TextField).at(1), 'AAAA-AAAA');
      await tester.tap(find.widgetWithText(PrimaryButton, 'Add agent host'));
      await tester.pumpAndSettle();

      expect(
        find.text(
          'Could not add the agent host; nothing changed in Liquid. Enter the address as ws://HOST:PORT, as meta invite prints it.',
        ),
        findsOneWidget,
      );
      expect(bridge.calls, contains('pair 127.0.0.1:3774 AAAA-AAAA'));
      expect(saved, isEmpty);
    },
  );

  testWidgets('uses a host for sessions at once', (tester) async {
    await show(tester, controller(settings: AgentHostsSettings([mini])));
    expect(find.text('Used for sessions'), findsOneWidget);

    await chooseInRow(tester, remote, 'Use for sessions');

    expect(switched, [remote]);
    expect(saved.last.activeAddress, remote);
    expect(find.text('Used for sessions'), findsOneWidget);
    expect(find.textContaining('Restart'), findsNothing);
  });

  testWidgets('choosing a host for sessions keeps Settings open', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    await tester.pumpWidget(
      LiquidApp(
        launch: ValueNotifier(
          Launch.session(SessionState(FakeProject()), info),
        ),
        agentHosts: controller(settings: AgentHostsSettings([mini])),
      ),
    );
    await tester.pumpAndSettle();
    await openAgentHosts(tester);

    await chooseInRow(tester, remote, 'Use for sessions');

    expect(
      find.text('Add agent host'),
      findsOneWidget,
      reason:
          'Settings stay open, as T3 Code keeps them when a host is switched',
    );
    expect(find.text('Used for sessions'), findsOneWidget);
  });

  testWidgets('a second choice waits while the host for sessions connects', (
    tester,
  ) async {
    final opening = Completer<String?>();
    await show(
      tester,
      controller(
        settings: AgentHostsSettings([mini]),
        starting: (_) => opening.future,
      ),
    );

    await chooseInRow(tester, remote, 'Use for sessions');
    await chooseInRow(tester, null, 'Use for sessions');

    expect(switched, [remote], reason: 'the rows wait for the switch');
    opening.complete(null);
    await tester.pumpAndSettle();
    expect(find.text('Used for sessions'), findsOneWidget);
  });

  testWidgets('the rows keep waiting for a choice when Settings reopen', (
    tester,
  ) async {
    final opening = Completer<String?>();
    await show(
      tester,
      controller(
        settings: AgentHostsSettings([mini]),
        starting: (_) => opening.future,
      ),
    );
    await chooseInRow(tester, remote, 'Use for sessions');

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await openAgentHosts(tester);
    await chooseInRow(tester, null, 'Use for sessions');

    expect(switched, [
      remote,
    ], reason: 'the choice in progress still holds the rows');
    opening.complete(null);
    await tester.pumpAndSettle();
  });

  testWidgets(
    'a host that cannot start sessions is not used, and the saved choice goes back',
    (tester) async {
      await show(
        tester,
        controller(
          settings: AgentHostsSettings([mini]),
          switchFailure:
              'Liquid could not connect to the agent host $remote: no answer.',
        ),
      );

      await chooseInRow(tester, remote, 'Use for sessions');

      expect(
        find.text(
          'Could not use Studio mini for sessions; nothing changed. Liquid could not connect to the agent host $remote: no answer.',
        ),
        findsOneWidget,
      );
      expect(saved.map((next) => next.activeAddress), [remote, null]);
    },
  );

  testWidgets(
    'the sidebar lists every agent host’s sessions, marks a remote one with its machine, and opens it on its host',
    (tester) async {
      final state = SessionState(
        FakeProject()..threadList = [threadOf('local', title: 'Local session')],
      );
      final studio = FakeProject()
        ..hostName = 'Studio mini'
        ..threadList = [threadOf('studio', title: 'Studio session')];
      await state.addHost(remote, studio, machine: 'mac-mini');
      await pumpSessions(tester, state);

      expect(find.text('Local session'), findsOneWidget);
      expect(find.text('Studio session'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('session-studio')),
          matching: find.byTooltip('Studio mini'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const Key('session-local')),
          matching: find.byType(MachineIcon),
        ),
        findsNothing,
      );

      await tester.tap(find.byKey(const Key('session-studio')));
      await tester.pumpAndSettle();

      expect(studio.calls, ['open studio']);
      expect(state.sessionAddress, remote);
    },
  );

  testWidgets(
    'an open session whose agent host stops answering says so in the composer after 2 s, until it answers',
    (tester) async {
      final session = FakeSession(id: 'studio');
      final studio = FakeProject(session: session)
        ..hostName = 'Studio mini'
        ..threadList = [threadOf('studio', title: 'Studio session')];
      final state = SessionState(FakeProject());
      await state.addHost(remote, studio);
      await state.open('studio', address: remote);
      await pumpSessions(tester, state);

      session.fail('the host is down');
      await tester.pump(const Duration(seconds: 1));
      expect(
        find.textContaining('is reconnecting'),
        findsNothing,
        reason: 'T3 waits 2 s before it says so',
      );
      await tester.pump(const Duration(seconds: 1));
      expect(
        find.text(
          'Studio mini is reconnecting. Liquid could not connect to the agent host $remote: the host is down.',
        ),
        findsOneWidget,
      );
      expect(state.session, same(session), reason: 'the Chat stays open');

      await state.addHost(remote, FakeProject(session: session));
      await tester.pump();
      expect(find.textContaining('is reconnecting'), findsNothing);
    },
  );

  testWidgets(
    'an agent host that stops answering is named in the sidebar, and its sessions go',
    (tester) async {
      final state = SessionState(
        FakeProject()..threadList = [threadOf('local', title: 'Local session')],
      );
      await state.addHost(
        remote,
        FakeProject()
          ..threadList = [threadOf('studio', title: 'Studio session')],
      );
      await state.addHost(remote, UnreachableProject());
      await pumpSessions(tester, state);

      expect(find.text('Local session'), findsOneWidget);
      expect(find.text('Studio session'), findsNothing);
      expect(
        find.text(
          'Liquid could not connect to the agent host $remote: the host is down.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a failing agent host at start leaves Settings to choose This Mac',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 1000));
      final shortcuts = await ShortcutController.load(
        MemoryStore(ShortcutSettings.standard()),
        FakeHost(),
      );
      await tester.pumpWidget(
        LiquidApp(
          launch: ValueNotifier(
            const Launch.problem(
              'Liquid could not connect to the agent host ws://studio.local:3774: no answer.',
            ),
          ),
          shortcuts: shortcuts,
          agentHosts: controller(
            settings: const AgentHostsSettings([miniActive]),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await openAgentHosts(tester);
      await chooseInRow(tester, null, 'Use for sessions');

      expect(switched, [null]);
      expect(usedFor(null), findsOneWidget);
    },
  );

  testWidgets('says why no agent host is in use while the Workspace opens', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    final workspace = WorkspaceState(FakeWorkspace());
    await workspace.start(show: true);
    await tester.pumpWidget(
      LiquidApp(
        launch: ValueNotifier(
          const Launch.problem(
            'Liquid could not connect to the agent host ws://studio.local:3774: no answer.',
          ),
        ),
        workspace: workspace,
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Liquid could not connect to the agent host ws://studio.local:3774: no answer.',
      ),
      findsOneWidget,
    );
  });

  testWidgets(
    'removes a host from this device after confirmation, and forgets its token',
    (tester) async {
      await show(tester, controller(settings: AgentHostsSettings([mini])));

      await chooseInRow(tester, remote, 'Remove from this device…');
      expect(find.text('Remove Studio mini from this device?'), findsOneWidget);
      expect(
        find.textContaining('meta host revoke liquid-studio-s-mac'),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(bridge.calls, isNot(contains('forget $remote')));

      await chooseInRow(tester, remote, 'Remove from this device…');
      await tester.tap(find.widgetWithText(DangerButton, 'Remove'));
      await tester.pumpAndSettle();

      expect(bridge.calls, contains('forget $remote'));
      expect(find.text('Studio mini'), findsNothing);
      expect(saved.last.hosts, isEmpty);
    },
  );

  testWidgets('removing the host in use shows This Mac at once', (
    tester,
  ) async {
    await show(
      tester,
      controller(settings: const AgentHostsSettings([miniActive])),
    );

    await chooseInRow(tester, remote, 'Remove from this device…');
    await tester.tap(find.widgetWithText(DangerButton, 'Remove'));
    await tester.pumpAndSettle();

    expect(switched, [null]);
    expect(saved.last.activeAddress, isNull);
    expect(find.text('Studio mini'), findsNothing);
    expect(usedFor(null), findsOneWidget);
  });

  testWidgets(
    'removing the host in use saves the list before This Mac is shown',
    (tester) async {
      await show(
        tester,
        controller(
          settings: const AgentHostsSettings([miniActive]),
          save: (next) async {
            if (next.hosts.isEmpty) {
              throw const FileSystemException('disk full');
            }
            saved.add(next);
          },
        ),
      );

      await chooseInRow(tester, remote, 'Remove from this device…');
      await tester.tap(find.widgetWithText(DangerButton, 'Remove'));
      await tester.pumpAndSettle();

      expect(switched, isEmpty, reason: 'the list is saved first');
      expect(
        find.text(
          'Could not remove Studio mini from the list; its token here is forgotten, so remove it again to finish. disk full.',
        ),
        findsOneWidget,
      );
    },
  );

  test('a superseded choice never replaces the later one, whether it connects or fails', () async {
    for (final failure in <String?>[null, 'no answer']) {
      final opening = Completer<String?>();
      final hosts = controller(
        settings: AgentHostsSettings([mini]),
        starting: (address) =>
            address == remote ? opening.future : Future.value(null),
      );

      final first = hosts.use(remote);
      await hosts.use(null);
      opening.complete(failure);
      await first;
      await pumpEventQueue();

      expect(
        hosts.value.activeAddress,
        isNull,
        reason: 'the later choice stays',
      );
      expect(saved.last.activeAddress, isNull);
    }
  });

  test('a failed switch keeps a host paired while it was running', () async {
    final opening = Completer<String?>();
    final hosts = controller(
      settings: AgentHostsSettings([mini]),
      starting: (_) => opening.future,
    );
    const lab = 'ws://lab:3774';

    final choosing = hosts.use(remote);
    await pumpEventQueue();
    await hosts.pair(lab, 'BBBB-BBBB');
    opening.complete('no answer');

    await expectLater(choosing, throwsA(isA<AgentHostsError>()));
    expect(hosts.value.hosts.map((host) => host.address), [remote, lab]);
    expect(hosts.value.activeAddress, isNull);
    expect(saved.last.hosts.map((host) => host.address), [remote, lab]);
  });

  test('a host whose list cannot be saved is forgotten again, and a failed forget is said', () async {
    final hosts = controller(
      save: (_) async => throw const FileSystemException('disk full'),
    );

    await expectLater(
      hosts.pair(remote, 'AAAA-AAAA'),
      throwsA(
        isA<AgentHostsError>().having(
          (error) => error.message,
          'message',
          'Could not add the agent host. Studio mini keeps this device paired as liquid-studio-s-mac; run meta host revoke liquid-studio-s-mac there to remove it. disk full.',
        ),
      ),
    );
    expect(bridge.calls.last, 'forget $remote');
    expect(hosts.value.hosts, isEmpty);

    bridge.forgetError = Exception('locked');
    await expectLater(
      hosts.pair(remote, 'BBBB-BBBB'),
      throwsA(
        isA<AgentHostsError>().having(
          (error) => error.message,
          'message',
          startsWith(
            'Could not add the agent host. Studio mini keeps this device paired as liquid-studio-s-mac; run meta host revoke liquid-studio-s-mac there to remove it. Its device token could not be forgotten',
          ),
        ),
      ),
    );
  });
}
