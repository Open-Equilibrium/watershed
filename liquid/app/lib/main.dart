import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated_io.dart'
    show ExternalLibrary;

import 'src/workspace/workspaces.dart';
import 'src/agent_hosts/agent_hosts.dart';
import 'src/app.dart';
import 'src/appearance.dart';
import 'src/diff/diff_settings.dart';
import 'src/agent/session_card_settings.dart';
import 'src/launch.dart';
import 'src/rust/frb_generated.dart';
import 'src/settings_file.dart';
import 'src/shortcuts/key_log.dart';
import 'src/shortcuts/shortcut_store.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  logKeysIfAsked(Platform.environment);
  final stem = RustLib.kDefaultExternalLibraryLoaderConfig.stem;
  await RustLib.init(
    externalLibrary: ExternalLibrary.open(
      Platform.isMacOS
          ? '@executable_path/../Frameworks/$stem.framework/$stem'
          : 'lib$stem.so',
    ),
  );
  final settings = settingsDirectory(
    Platform.environment,
    defaultTargetPlatform,
  );
  final agentHostsFile = SettingsFile(File('$settings/agent_hosts.json'));
  final agentHosts = await AgentHostsSettings.load(agentHostsFile);
  final launch = launchFromEnvironment(
    Platform.environment,
    startAddress: agentHosts.activeAddress,
  );
  if (launch.problem case final problem?) stderr.writeln('Liquid: $problem');
  final links = RustAgentHostLinks(launch, Platform.environment);
  unawaited(links.connect(null));
  for (final host in agentHosts.hosts) {
    unawaited(links.connect(host.address));
  }
  AppLifecycleListener(onResume: links.retryAll);
  final workspaces = await Workspaces.load(
    settings,
    (dir) => openWorkspace(dir, show: launch.state == null),
    file: SettingsFile(File('$settings/workspaces.json')),
  );
  final surface = ValueNotifier(launch);
  runApp(
    LiquidApp(
      launch: surface,
      workspaces: workspaces,
      shortcuts: await ShortcutController.load(
        ShortcutStore(File('$settings/shortcuts.json')),
        const ChannelLiquidKeyHost(),
      ),
      diffSettings: await DiffSettingsController.load(
        SettingsFile(File('$settings/diff.json')),
      ),
      cardSettings: await SessionCardSettingsController.load(
        SettingsFile(File('$settings/session_card.json')),
      ),
      appearance: await AppearanceController.load(
        SettingsFile(File('$settings/appearance.json')),
      ),
      agentHosts: AgentHostsController(
        settings: agentHosts,
        bridge: RustAgentHostBridge(
          home: Platform.environment['META_HARNESS_HOME'],
        ),
        save: (next) => agentHostsFile.save(next.toJson()),
        links: links,
      ),
    ),
  );
}
