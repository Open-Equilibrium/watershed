import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_test_robots/flutter_test_robots.dart';
import 'package:integration_test/integration_test.dart';
import 'package:section_spike/core.dart';
import 'package:section_spike/own/editor.dart';
import 'package:section_spike/own/model.dart';

String stats(List<int> micros) {
  final sorted = [...micros]..sort();
  double at(double q) => sorted[((sorted.length - 1) * q).round()] / 1000;
  return 'n=${sorted.length} median ${at(0.5)} ms, p99 ${at(0.99)} ms, max ${at(1)} ms';
}

SectionModel longSection(int paragraphs) {
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
  return SectionModel(core);
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  for (final paragraphs in [100, 1000, 10000]) {
    testWidgets('own editor, a Section of $paragraphs Paragraphs', (tester) async {
      final load = Stopwatch()..start();
      final m = longSection(paragraphs);
      final built = load.elapsedMilliseconds;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: SectionEditor(model: m)),
        ),
      );
      load.stop();
      final editor = tester.state<SectionEditorState>(find.byType(SectionEditor));
      editor.focus.requestFocus();
      await tester.pump();
      editor.select((para: 0, offset: 5));
      await tester.pump();
      final syncs = <int>[];
      await binding.watchPerformance(() async {
        for (var i = 0; i < 100; i++) {
          await tester.ime.typeText('x', getter: () => editor);
          await tester.pump();
          syncs.add(m.lastSync.value);
        }
      }, reportKey: 'typing');
      final enters = <int>[];
      await binding.watchPerformance(() async {
        for (var i = 0; i < 20; i++) {
          editor.enter();
          await tester.pump();
          enters.add(m.lastSync.value);
        }
      }, reportKey: 'enter');
      expect(m.paras.length, paragraphs + 20);
      String frames(String key) {
        final r = binding.reportData![key] as Map;
        return 'build avg ${r['average_frame_build_time_millis']} ms, worst ${r['worst_frame_build_time_millis']} ms; '
            'raster avg ${r['average_frame_rasterizer_time_millis']} ms, worst ${r['worst_frame_rasterizer_time_millis']} ms';
      }

      // ignore: avoid_print
      print('OWN[$paragraphs] core and model: $built ms; with first frame: ${load.elapsedMilliseconds} ms');
      // ignore: avoid_print
      print('OWN[$paragraphs] sync per keystroke: ${stats(syncs)}; frames: ${frames('typing')}');
      // ignore: avoid_print
      print('OWN[$paragraphs] sync per ⏎: ${stats(enters)}; frames: ${frames('enter')}');
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 1));
    });
  }
}
