import 'dart:math' as math;

import 'package:flutter/material.dart';

class BarSegment {
  const BarSegment(this.value, this.color, {this.hatched = false});

  final int value;
  final Color color;
  final bool hatched;
}

class SegmentBar extends StatelessWidget {
  const SegmentBar({super.key, required this.segments, this.height = 3});

  final List<BarSegment> segments;
  final double height;

  @override
  Widget build(BuildContext context) => SizedBox(
    height: height,
    child: CustomPaint(
      painter: _SegmentPainter([
        for (final segment in segments)
          if (segment.value > 0) segment,
      ]),
    ),
  );
}

class _SegmentPainter extends CustomPainter {
  _SegmentPainter(this.segments);

  final List<BarSegment> segments;

  static const _gap = 2.0;

  @override
  void paint(Canvas canvas, Size size) {
    final total = segments.fold(0, (sum, segment) => sum + segment.value);
    if (total == 0) return;
    final room = size.width - _gap * (segments.length - 1);
    var left = 0.0;
    for (final segment in segments) {
      final width = math.max(2.0, room * segment.value / total);
      final rect = RRect.fromRectAndRadius(
        Rect.fromLTWH(left, 0, width, size.height),
        Radius.circular(math.min(1.5, size.height / 2)),
      );
      if (segment.hatched) {
        _hatch(canvas, rect, segment.color);
      } else {
        canvas.drawRRect(rect, Paint()..color = segment.color);
      }
      left += width + _gap;
    }
  }

  static void _hatch(Canvas canvas, RRect rect, Color color) {
    canvas.save();
    canvas.clipRRect(rect);
    final paint = Paint()
      ..color = color
      ..strokeWidth = 1.5;
    for (var x = rect.left - rect.height; x < rect.right; x += 4) {
      canvas.drawLine(
        Offset(x, rect.bottom),
        Offset(x + rect.height, rect.top),
        paint,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_SegmentPainter old) => true;
}
