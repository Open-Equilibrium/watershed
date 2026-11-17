import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/document_pages.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<ui.Image> pixel() {
    final drawn = Completer<ui.Image>();
    ui.decodeImageFromPixels(
      Uint8List.fromList([255, 255, 255, 255]),
      1,
      1,
      ui.PixelFormat.rgba8888,
      drawn.complete,
    );
    return drawn.future;
  }

  test('pages in view are drawn once, however many are in view', () async {
    final image = await pixel();
    var renders = 0;
    final pages = DocumentPages.pdf(
      PdfPages(List.filled(60, const Size(595, 100)), (index, width) async {
        renders++;
        return image.clone();
      }, () async {}),
      onDrawn: () {},
    );
    void paint() {
      final recorder = ui.PictureRecorder();
      pages.paint(
        Canvas(recorder),
        const Rect.fromLTWH(-400, -20, 800, 7000),
        0.5,
      );
      recorder.endRecording().dispose();
    }

    for (var frame = 0; frame < 4; frame++) {
      paint();
      await Future<void>.delayed(Duration.zero);
    }
    expect(renders, 60, reason: 'each of the 60 pages in view once');
    pages.close();
  });

  test('a page the renderer cannot draw is not asked for again', () async {
    var renders = 0;
    final pages = DocumentPages.pdf(
      PdfPages(const [Size(595, 842)], (index, width) async {
        renders++;
        throw StateError('render');
      }, () async {}),
      onDrawn: () {},
    );
    for (var frame = 0; frame < 4; frame++) {
      final recorder = ui.PictureRecorder();
      pages.paint(
        Canvas(recorder),
        const Rect.fromLTWH(-400, -20, 800, 900),
        0.5,
      );
      recorder.endRecording().dispose();
      await Future<void>.delayed(Duration.zero);
    }
    expect(renders, 1);
    pages.close();
  });

  test('a page with no height or width is passed over', () async {
    var renders = 0;
    final pages = DocumentPages.pdf(
      PdfPages(const [Size(595, 0), Size(0, 842)], (index, width) async {
        renders++;
        return pixel();
      }, () async {}),
      onDrawn: () {},
    );
    final recorder = ui.PictureRecorder();
    pages.paint(
      Canvas(recorder),
      const Rect.fromLTWH(-400, -20, 800, 900),
      0.5,
    );
    recorder.endRecording().dispose();
    await Future<void>.delayed(Duration.zero);
    expect(renders, 0);
    pages.close();
  });
}
