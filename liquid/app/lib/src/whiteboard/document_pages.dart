import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';

import '../workspace/pdf_pages.dart';

class DocumentPages {
  DocumentPages.pdf(PdfPages this._pdf, {required this.onDrawn})
    : _image = null,
      sheets = layout(_pdf.sizes);

  DocumentPages.image(ui.Image image, {required this.onDrawn})
    : _pdf = null,
      _image = image,
      sheets = [
        Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
      ];

  final PdfPages? _pdf;
  final ui.Image? _image;

  final VoidCallback onDrawn;

  final List<Rect> sheets;

  static const gap = 16.0;

  static const kept = 12;

  static List<Rect> layout(List<Size> sizes) {
    var top = 0.0;
    return [
      for (final size in sizes)
        () {
          final sheet = Rect.fromLTWH(
            -size.width / 2,
            top,
            size.width,
            size.height,
          );
          top += size.height + gap;
          return sheet;
        }(),
    ];
  }

  final _drawn = <int, (int, ui.Image)>{};
  final _asked = <int, int>{};

  final _inView = <int>{};
  var _closed = false;

  void paint(Canvas canvas, Rect view, double pixelsPerPoint) {
    final paper = Paint()..color = const Color(0xFFFFFFFF);
    final edge = Paint()
      ..color = const Color(0x22000000)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1 / pixelsPerPoint;
    _inView.clear();
    for (final (index, sheet) in sheets.indexed) {
      if (sheet.isEmpty || !sheet.overlaps(view)) continue;
      _inView.add(index);
      if (_image case final image?) {
        canvas.drawImageRect(
          image,
          Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
          sheet,
          Paint()..filterQuality = FilterQuality.medium,
        );
        continue;
      }
      canvas
        ..drawRect(sheet, paper)
        ..drawRect(sheet, edge);
      final tallest = (4096 * sheet.width / sheet.height).floor();
      final wanted = math.max(
        1,
        math.min(
          ((sheet.width * pixelsPerPoint / 256).ceil() * 256).clamp(256, 4096),
          tallest,
        ),
      );
      final drawn = _drawn.remove(index);
      if (drawn != null) _drawn[index] = drawn;
      if (drawn case (_, final image)) {
        canvas.drawImageRect(
          image,
          Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
          sheet,
          Paint()..filterQuality = FilterQuality.medium,
        );
      }
      if (drawn == null || drawn.$1 < wanted) _ask(index, wanted);
    }
  }

  void _ask(int index, int width) {
    final pdf = _pdf;
    if (pdf == null || _asked[index] == width) return;
    _asked[index] = width;
    pdf.render(index, width).then((image) {
      if (_closed || _asked[index] != width) {
        image.dispose();
        return;
      }
      _drawn.remove(index)?.$2.dispose();
      _drawn[index] = (width, image);
      final oldest = [
        for (final page in _drawn.keys)
          if (!_inView.contains(page)) page,
      ];
      for (final page in oldest.take(math.max(0, _drawn.length - kept))) {
        _drawn.remove(page)?.$2.dispose();
        _asked.remove(page);
      }
      onDrawn();
    }, onError: (Object _) {});
  }

  int pageNear(Offset at) => nearest(sheets, at);

  static int nearest(List<Rect> sheets, Offset at) {
    var best = 0;
    var distance = double.infinity;
    for (final (index, sheet) in sheets.indexed) {
      final away = math.max(
        0.0,
        math.max(sheet.top - at.dy, at.dy - sheet.bottom),
      );
      if (away < distance) (best, distance) = (index, away);
    }
    return best;
  }

  void close() {
    _closed = true;
    for (final (_, image) in _drawn.values) {
      image.dispose();
    }
    _drawn.clear();
    _image?.dispose();
    if (_pdf case final pdf?) pdf.close().ignore();
  }
}
