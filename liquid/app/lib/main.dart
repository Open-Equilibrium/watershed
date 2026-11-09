import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated_io.dart'
    show ExternalLibrary;

import 'src/app.dart';
import 'src/appearance.dart';
import 'src/diff/diff_settings.dart';
import 'src/launch.dart';
import 'src/rust/frb_generated.dart';
import 'src/session/branch_bases.dart';
import 'src/settings_file.dart';
import 'src/shortcuts/key_log.dart';
import 'src/shortcuts/shortcut_store.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  logKeysIfAsked(Platform.environment);
  // Only the bridge inside the app bundle: by default flutter_rust_bridge first tries a library
  // relative to the working directory or in a folder an environment variable names, and a
  // relative framework path would still be looked up in the working directory first.
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
  final launch = await launchFromEnvironment(
    Platform.environment,
    bases: BranchBases(file: SettingsFile(File('$settings/git-bases.json'))),
  );
  // Without a session the Workspace still opens; the reason stays in the terminal.
  if (launch.problem case final problem?) stderr.writeln('Liquid: $problem');
  // Beside the settings, in Liquid's data folder.
  final (workspace, workspaceProblem) = await openWorkspace(
    '$settings/Workspace',
    show: launch.state == null,
  );
  runApp(
    LiquidApp(
      launch: launch,
      workspace: workspace,
      workspaceProblem: workspaceProblem,
      shortcuts: await ShortcutController.load(
        ShortcutStore(File('$settings/shortcuts.json')),
        const ChannelLiquidKeyHost(),
      ),
      diffSettings: await DiffSettingsController.load(
        SettingsFile(File('$settings/diff.json')),
      ),
      appearance: await AppearanceController.load(
        SettingsFile(File('$settings/appearance.json')),
      ),
    ),
  );
}
