import 'dart:io';

import 'package:flutter/widgets.dart';

import 'src/app.dart';
import 'src/launch.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(LiquidApp(launch: await launchFromEnvironment(Platform.environment)));
}
