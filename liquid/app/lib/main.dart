import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated_io.dart'
    show ExternalLibrary;

import 'src/app.dart';
import 'src/diff/diff_settings.dart';
import 'src/launch.dart';
import 'src/rust/frb_generated.dart';
import 'src/settings_file.dart';
import 'src/shortcuts/shortcut_store.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
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
  runApp(
    LiquidApp(
      launch: await launchFromEnvironment(Platform.environment),
      shortcuts: await ShortcutController.load(
        ShortcutStore(File('$settings/shortcuts.json')),
        const ChannelLiquidKeyHost(),
      ),
      diffSettings: await DiffSettingsController.load(
        SettingsFile(File('$settings/diff.json')),
      ),
    ),
  );
}
