import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated_io.dart'
    show ExternalLibrary;
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:liquid/src/rust/api/replica.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/rust/frb_generated.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/page_view.dart';
import 'package:liquid/src/workspace/text_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

const blocks = 1000;
const keystrokes = 300;

double percentile(List<double> sorted, double share) =>
    sorted[((sorted.length - 1) * share).round()];

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets('typing in a long Page', (tester) async {
    final stem = RustLib.kDefaultExternalLibraryLoaderConfig.stem;
    await RustLib.init(
      externalLibrary: ExternalLibrary.open(
        '@executable_path/../Frameworks/$stem.framework/$stem',
      ),
    );
    final dir = Directory.systemTemp.createTempSync('liquid-typing-');
    final workspace = await LiquidWorkspace.open(dir: dir.path);
    final state = WorkspaceState(workspace);
    await state.start();
    final open = state.open.value!;
    String? after;
    for (var index = 0; index < blocks; index++) {
      final added = await state.apply(
        mutation(
          BridgeMutationKind.addTextBlock,
          page: open.page,
          after: after,
          text: 'Paragraph $index of a long Page, written to fill it.',
        ),
      );
      after = added.view;
    }
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Scaffold(
          body: WorkspacePage(state: state, open: open),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final field = find.descendant(
      of: find.byType(TextBlock).first,
      matching: find.byType(EditableText),
    );
    await tester.tap(field);
    await tester.pumpAndSettle();
    var typed = tester.widget<EditableText>(field).controller.text;
    expect(typed, startsWith('Paragraph 0 '));

    final timings = <FrameTiming>[];
    SchedulerBinding.instance.addTimingsCallback(timings.addAll);
    final toBuilt = <double>[];
    for (var index = 0; index < keystrokes; index++) {
      final watch = Stopwatch()..start();
      typed = '${typed}a';
      await tester.enterText(field, typed);
      await SchedulerBinding.instance.endOfFrame;
      toBuilt.add(watch.elapsedMicroseconds / 1000);
      await Future<void>.delayed(const Duration(milliseconds: 30));
    }
    await Future<void>.delayed(const Duration(seconds: 2));
    expect(tester.widget<EditableText>(field).controller.text, typed);
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('page-title')))
          .controller!
          .text,
      isEmpty,
    );
    final raster = [
      for (final timing in timings) timing.rasterDuration.inMicroseconds / 1000,
    ]..sort();
    toBuilt.sort();
    final report = {
      'blocks': blocks,
      'keystrokes': keystrokes,
      'event_to_frame_built_ms': {
        'p50': percentile(toBuilt, 0.5),
        'p99': percentile(toBuilt, 0.99),
        'max': toBuilt.last,
      },
      'raster_ms': {
        'p50': percentile(raster, 0.5),
        'p99': percentile(raster, 0.99),
        'max': raster.last,
        'frames': raster.length,
      },
    };
    binding.reportData = {'typing': report};
    stdout.writeln('TYPING ${jsonEncode(report)}');
    await tester.pumpWidget(const SizedBox.shrink());
    await state.settled;
    await state.dispose();
    workspace.dispose();
    dir.deleteSync(recursive: true);
  });
}
