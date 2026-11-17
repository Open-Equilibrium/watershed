import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/agent_hosts/agent_hosts.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/hosts.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/shortcuts/keymap.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'fake_host.dart';

const remote = 'ws://studio.local:3774';
const client = 'liquid-studio-s-mac';
const info = SessionInfo(project: 'acme-api');

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

  AgentHostsController controller({
    AgentHostsSettings settings = const AgentHostsSettings(),
    String? running,
  }) => AgentHostsController(
    settings: settings,
    bridge: bridge,
    running: running,
    save: (next) async => saved.add(next),
  );

  setUp(() {
    bridge = FakeAgentHosts()
      ..reaches[null] = BridgeReach.connected
      ..reaches[remote] = BridgeReach.connected;
    saved = [];
  });

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

  /// Opens the row menu of the host at `address` and chooses `item`.
  Future<void> chooseInRow(
    WidgetTester tester,
    String address,
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
      expect(
        AgentHostsSettings.fromJson(settings.toJson()).activeAddress,
        remote,
      );
      expect(
        settings.withActive('ws://lab:3774').activeAddress,
        'ws://lab:3774',
      );
      expect(
        settings.withActive('ws://lab:3774').hosts.where((host) => host.active),
        hasLength(1),
      );
      expect(
        settings.withActive(null).activeAddress,
        isNull,
        reason: 'This Mac',
      );
      expect(
        settings.without(remote).activeAddress,
        isNull,
        reason: 'This Mac',
      );
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
          running: remote,
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

  testWidgets('uses a host for sessions, which a restart applies', (
    tester,
  ) async {
    await show(
      tester,
      controller(
        settings: const AgentHostsSettings([
          SavedAgentHost(name: 'Studio mini', address: remote, client: client),
        ]),
      ),
    );
    expect(find.text('Used for sessions'), findsOneWidget);

    await chooseInRow(tester, remote, 'Use for sessions');

    expect(saved.last.activeAddress, remote);
    expect(find.text('Used for sessions'), findsOneWidget);
    expect(
      find.text('Restart Liquid to use Studio mini for sessions.'),
      findsOneWidget,
    );
  });

  testWidgets(
    'removes a host from this device after confirmation, and forgets its token',
    (tester) async {
      await show(
        tester,
        controller(
          settings: const AgentHostsSettings([
            SavedAgentHost(
              name: 'Studio mini',
              address: remote,
              client: client,
            ),
          ]),
        ),
      );

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

  test('a host whose list cannot be saved is forgotten again, and a failed forget is said', () async {
    final hosts = AgentHostsController(
      settings: const AgentHostsSettings(),
      bridge: bridge,
      running: null,
      save: (_) async => throw const FileSystemException('disk full'),
    );

    await expectLater(
      hosts.pair(remote, 'AAAA-AAAA'),
      throwsA(
        isA<AgentHostsError>().having(
          (error) => error.message,
          'message',
          'Could not add the agent host; nothing changed. disk full',
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
            'Could not add the agent host; its device token could not be forgotten',
          ),
        ),
      ),
    );
  });
}
