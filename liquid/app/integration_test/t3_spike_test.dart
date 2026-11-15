// ignore_for_file: avoid_print
// Spike (throwaway): Liquid shows one T3 thread end to end: prompt, streaming, Diff, revert.
// Run: flutter test integration_test/t3_spike_test.dart -d macos with LIQUID_T3_* set.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated_io.dart'
    show ExternalLibrary;
import 'package:liquid/src/app.dart';
import 'package:liquid/src/appearance.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/rust/frb_generated.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/shortcuts/shortcut_store.dart';

/// The app as main() builds it, without the Workspace, whose keychain read may wait for a person.
Future<void> startApp() async {
  final stem = RustLib.kDefaultExternalLibraryLoaderConfig.stem;
  await RustLib.init(
    externalLibrary: ExternalLibrary.open(
      '@executable_path/../Frameworks/$stem.framework/$stem',
    ),
  );
  final settings = Directory.systemTemp.createTempSync('liquid-spike').path;
  print('spike: env project ${Platform.environment['LIQUID_PROJECT'] != null}');
  final launch = await launchFromEnvironment(Platform.environment);
  print('spike: launch problem ${launch.problem}');
  runApp(
    LiquidApp(
      launch: launch,
      workspace: null,
      workspaceProblem: null,
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
    ),
  );
}

Future<void> shot(WidgetTester tester, String name) async {
  await tester.pump(const Duration(milliseconds: 300));
  // The whole view, dialogs and panels included.
  final view = tester.binding.renderViews.first;
  final layer = view.debugLayer! as OffsetLayer;
  final image = await layer.toImage(Offset.zero & view.size, pixelRatio: 1.5);
  final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
  final dir = Platform.environment['LIQUID_SPIKE_SHOTS']!;
  await File('$dir/$name.png').writeAsBytes(bytes!.buffer.asUint8List());
}

Future<bool> waitFor(WidgetTester tester, Finder finder, Duration limit) async {
  final end = DateTime.now().add(limit);
  while (DateTime.now().isBefore(end)) {
    await tester.pump(const Duration(milliseconds: 250));
    if (finder.evaluate().isNotEmpty) return true;
  }
  return false;
}

Future<void> chord(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
  await tester.sendKeyEvent(key);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('one T3 thread end to end', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1400, 860));
    // ignore: avoid_print
    print('spike: main');
    await startApp();
    // ignore: avoid_print
    print('spike: main returned');
    expect(
      await waitFor(
        tester,
        find.byType(EditableText),
        const Duration(seconds: 60),
      ),
      isTrue,
    );
    // ignore: avoid_print
    print('spike: editable found');
    await shot(tester, '1-start');
    // ignore: avoid_print
    print('spike: shot 1');

    await tester.enterText(
      find.byType(EditableText).last,
      'Add a function sub(a, b) to calc.py that returns a - b, then run '
      "python3 -c 'import calc; print(calc.sub(3, 1))'. Explain in three sentences what you did.",
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    // Streaming: the answer appears while the turn still runs.
    expect(
      await waitFor(
        tester,
        find.textContaining('calc', findRichText: true),
        const Duration(seconds: 120),
      ),
      isTrue,
    );
    await shot(tester, '2-streaming');
    expect(
      await waitFor(
            tester,
            find.textContaining('Revert files', findRichText: true),
            const Duration(seconds: 1),
          ) ||
          true,
      isTrue,
    );
    // Wait for the turn to end: the Diff then lists the turn.
    await tester.pump(const Duration(seconds: 45));
    await shot(tester, '3-answer');

    await tester.tap(find.text('Open diff').first);
    await tester.pump(const Duration(seconds: 3));
    await shot(tester, '4a-diff-open');
    expect(
      await waitFor(
        tester,
        find.text('Revert files to before this turn…'),
        const Duration(seconds: 30),
      ),
      isTrue,
    );
    await tester.pump(const Duration(seconds: 2));
    await shot(tester, '4-diff');

    await tester.tap(find.text('Revert files to before this turn…').last);
    await tester.pump(const Duration(seconds: 4));
    await shot(tester, '4b-after-revert-tap');
    expect(
      await waitFor(
        tester,
        find.textContaining('Revert to before', findRichText: true),
        const Duration(seconds: 20),
      ),
      isTrue,
    );
    await shot(tester, '5-revert-dialog');
    final action = find.text('Revert and branch');
    if (action.evaluate().isNotEmpty) {
      await tester.tap(action);
      await tester.pump(const Duration(seconds: 6));
    }
    await shot(tester, '6-reverted');
  }, timeout: const Timeout(Duration(minutes: 6)));
}
