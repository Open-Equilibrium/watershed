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

/// The bridge's agent hosts in memory: checks answer from `reaches`, and an unlisted host is offline.
class FakeAgentHosts implements AgentHostBridge {
  final reaches = <String?, BridgeReach>{};
  final calls = <String>[];
  Object? pairError;
  Object? forgetError;

  @override
  Future<BridgePairing> pair(String address, String code) async {
    calls.add('pair $address $code');
    if (pairError case final error?) throw error;
    return BridgePairing(host: 'Studio mini', client: client);
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

void main() {
  late FakeAgentHosts bridge;
  late List<AgentHostsSettings> saved;
  late List<String?> switched;

  AgentHostsController controller({
    AgentHostsSettings settings = const AgentHostsSettings(),
    String? switchFailure,
  }) => AgentHostsController(
    settings: settings,
    bridge: bridge,
    save: (next) async => saved.add(next),
    switchTo: (address) async {
      switched.add(address);
      return switchFailure;
    },
  );

  setUp(() {
    bridge = FakeAgentHosts()
      ..reaches[null] = BridgeReach.connected
      ..reaches[remote] = BridgeReach.connected;
    saved = [];
    switched = [];
  });

  /// Opens Settings → Agent hosts with ⌘, and the section.
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

  /// Opens Settings → Agent hosts over the agent surface, as ⌘, and the section do.
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

  /// Opens the row menu of the host at `address` (null: This Mac) and chooses `item`.
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
      // This Mac is used when no host is active.
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
        controller(
          settings: const AgentHostsSettings([
            SavedAgentHost(
              name: 'Studio mini',
              address: remote,
              client: client,
              active: true,
            ),
          ]),
        ),
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
          'Could not add the agent host; nothing changed. That pairing code is wrong, already used or expired.',
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

      // Each field is named by its label, which its hint follows while the field is empty; the Host
      // field has the keys first.
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

  testWidgets('uses a host for sessions at once', (tester) async {
    await show(tester, controller(settings: AgentHostsSettings([mini])));
    expect(find.text('Used for sessions'), findsOneWidget);

    await chooseInRow(tester, remote, 'Use for sessions');

    expect(switched, [remote]);
    expect(saved.last.activeAddress, remote);
    expect(find.text('Used for sessions'), findsOneWidget);
    expect(find.textContaining('Restart'), findsNothing);
  });

  testWidgets('a second choice waits while the agent area opens', (
    tester,
  ) async {
    final opening = Completer<String?>();
    await tester.binding.setSurfaceSize(const Size(1600, 1000));
    final hosts = AgentHostsController(
      settings: AgentHostsSettings([mini]),
      bridge: bridge,
      save: (next) async => saved.add(next),
      switchTo: (address) async {
        switched.add(address);
        return opening.future;
      },
    );
    await show(tester, hosts);

    await chooseInRow(tester, remote, 'Use for sessions');
    await chooseInRow(tester, null, 'Use for sessions');

    expect(switched, [remote], reason: 'the rows wait for the switch');
    opening.complete(null);
    await tester.pumpAndSettle();
    expect(find.text('Used for sessions'), findsOneWidget);
  });

  testWidgets(
    'a host whose agent area cannot open is not used, and the saved choice goes back',
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

  testWidgets('switching the agent host shows its sessions at once', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    final before = SessionState(
      FakeProject()..threadList = [threadOf('old', title: 'Old host session')],
    );
    final after = SessionState(
      FakeProject()..threadList = [threadOf('new', title: 'New host session')],
    );
    await before.loadStart();
    await after.loadStart();
    final surface = ValueNotifier(Launch.session(before, info));
    await tester.pumpWidget(LiquidApp(launch: surface));
    await tester.pumpAndSettle();
    expect(find.text('Old host session'), findsOneWidget);

    surface.value = Launch.session(after, info);
    await tester.pumpAndSettle();

    expect(find.text('New host session'), findsOneWidget);
    expect(find.text('Old host session'), findsNothing);
  });

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
            settings: const AgentHostsSettings([
              SavedAgentHost(
                name: 'Studio mini',
                address: remote,
                client: client,
                active: true,
              ),
            ]),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await openAgentHosts(tester);
      await chooseInRow(tester, null, 'Use for sessions');

      expect(switched, [null]);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey<String?>(null)),
          matching: find.text('Used for sessions'),
        ),
        findsOneWidget,
      );
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
      controller(
        settings: const AgentHostsSettings([
          SavedAgentHost(
            name: 'Studio mini',
            address: remote,
            client: client,
            active: true,
          ),
        ]),
      ),
    );

    await chooseInRow(tester, remote, 'Remove from this device…');
    await tester.tap(find.widgetWithText(DangerButton, 'Remove'));
    await tester.pumpAndSettle();

    expect(switched, [null]);
    expect(saved.last.activeAddress, isNull);
    expect(find.text('Studio mini'), findsNothing);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey<String?>(null)),
        matching: find.text('Used for sessions'),
      ),
      findsOneWidget,
    );
  });

  test('a result of a choice that a later one superseded is dropped', () async {
    final opening = Completer<String?>();
    final hosts = AgentHostsController(
      settings: AgentHostsSettings([mini]),
      bridge: bridge,
      save: (next) async => saved.add(next),
      switchTo: (address) =>
          address == remote ? opening.future : Future.value(null),
    );

    final first = hosts.use(remote);
    await hosts.use(null);
    opening.complete('no answer');
    await first;

    expect(hosts.value.activeAddress, isNull);
    expect(saved.last.activeAddress, isNull);
  });

  test('a host whose list cannot be saved is forgotten again, and a failed forget is said', () async {
    final hosts = AgentHostsController(
      settings: const AgentHostsSettings(),
      bridge: bridge,
      save: (_) async => throw const FileSystemException('disk full'),
      switchTo: (_) async => null,
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
