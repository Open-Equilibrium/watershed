import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:section_spike/binding.dart';
import 'package:section_spike/core.dart';
import 'package:super_editor/super_editor.dart';
import 'package:super_editor/super_editor_test.dart';

String stats(List<int> micros) {
  final sorted = [...micros]..sort();
  double at(double q) => sorted[((sorted.length - 1) * q).round()] / 1000;
  return 'n=${sorted.length} median ${at(0.5)} ms, p90 ${at(0.9)} ms, p99 ${at(0.99)} ms, max ${at(1)} ms';
}

SectionBinding longSection(int paragraphs) {
  final core = SectionCore();
  final ops = <Map<String, Object?>>[];
  var at = 1;
  for (var i = 0; i < paragraphs; i++) {
    if (i > 0) ops.add({'op': 'split', 'at': at++, 'type': 'paragraph'});
    final text = 'Paragraph ${i + 1}: Liquid keeps a Section as one text, so a selection crosses Paragraphs freely.';
    ops.add({'op': 'splice', 'at': at, 'ins': text});
    at += text.length;
  }
  core.apply(ops);
  return SectionBinding(core);
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  for (final paragraphs in [100, 1000]) {
    testWidgets('a Section of $paragraphs Paragraphs: load, type, ⏎', (tester) async {
      final load = Stopwatch()..start();
      final s = longSection(paragraphs);
      final built = load.elapsedMilliseconds;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: SuperEditor(editor: s.editor)),
        ),
      );
      load.stop();
      // ignore: avoid_print
      print('PERF[$paragraphs] core and editor model: $built ms; with first frame: ${load.elapsedMilliseconds} ms');

      await tester.placeCaretInParagraph(s.document.getNodeAt(0)!.id, 5);
      final syncs = <int>[];
      await binding.watchPerformance(() async {
        for (var i = 0; i < 100; i++) {
          await tester.typeImeText('x');
          syncs.add(s.lastSync.value);
        }
      }, reportKey: 'typing');
      final enters = <int>[];
      await binding.watchPerformance(() async {
        for (var i = 0; i < 20; i++) {
          s.editor.execute([InsertNewlineAtCaretRequest(Editor.createNodeId())]);
          await tester.pump();
          enters.add(s.lastSync.value);
        }
      }, reportKey: 'enter');
      expect(s.document.nodeCount, paragraphs + 20);
      String frames(String key) {
        final r = binding.reportData![key] as Map;
        return 'build avg ${r['average_frame_build_time_millis']} ms, p90 ${r['90th_percentile_frame_build_time_millis']} ms, '
            'p99 ${r['99th_percentile_frame_build_time_millis']} ms, worst ${r['worst_frame_build_time_millis']} ms; '
            'raster avg ${r['average_frame_rasterizer_time_millis']} ms, worst ${r['worst_frame_rasterizer_time_millis']} ms';
      }

      // ignore: avoid_print
      print('PERF[$paragraphs] sync per keystroke: ${stats(syncs)}');
      // ignore: avoid_print
      print('PERF[$paragraphs] typing frames: ${frames('typing')}');
      // ignore: avoid_print
      print('PERF[$paragraphs] sync per ⏎: ${stats(enters)}');
      // ignore: avoid_print
      print('PERF[$paragraphs] ⏎ frames: ${frames('enter')}');
      expect(s.editorShape(), SectionBinding.coreShape(s.core.spans()));
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 1));
    });
  }
}
