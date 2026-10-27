import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'src/app.dart';
import 'src/diff/diff_settings.dart';
import 'src/launch.dart';
import 'src/rust/frb_generated.dart';
import 'src/settings_file.dart';
import 'src/shortcuts/shortcut_store.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
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
