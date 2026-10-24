import 'package:flutter/widgets.dart';

import 'src/app.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  await RustLib.init();
  runApp(const LiquidApp());
}
