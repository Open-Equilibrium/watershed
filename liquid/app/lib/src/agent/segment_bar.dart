import 'dart:math' as math;

import 'package:flutter/material.dart';

/// One part of a [SegmentBar]: its count, its colour, and whether it is hatched (running).
class BarSegment {
  const BarSegment(this.value, this.color, {this.hatched = false});

  final int value;
  final Color color;
  final bool hatched;
}

/// The parts of a whole drawn in order, each as wide as its share; a part that has any count
/// keeps at least 2 px so it stays visible. Zero parts draw nothing.
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

  /// Diagonal stripes, as a running tool call is drawn.
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

/// The context bar: the share of the window in use, with a tick at `tick` where pi compacts.
class FillBar extends StatelessWidget {
  const FillBar({
    super.key,
    required this.fraction,
    this.tick,
    required this.track,
    required this.fill,
    required this.mark,
  });

  /// The share of the window in use, from 0 to 1; none is drawn when null.
  final double? fraction;

  /// Where the tick stands, from 0 to 1.
  final double? tick;
  final Color track;
  final Color fill;
  final Color mark;

  @override
  Widget build(BuildContext context) => SizedBox(
    height: 9,
    child: CustomPaint(
      painter: _FillPainter(
        fraction: fraction,
        tick: tick,
        track: track,
        fill: fill,
        mark: mark,
      ),
    ),
  );
}

class _FillPainter extends CustomPainter {
  _FillPainter({
    required this.fraction,
    required this.tick,
    required this.track,
    required this.fill,
    required this.mark,
  });

  final double? fraction;
  final double? tick;
  final Color track;
  final Color fill;
  final Color mark;

  @override
  void paint(Canvas canvas, Size size) {
    final bar = Rect.fromLTWH(0, (size.height - 3) / 2, size.width, 3);
    final radius = const Radius.circular(2);
    canvas.drawRRect(
      RRect.fromRectAndRadius(bar, radius),
      Paint()..color = track,
    );
    final used = fraction;
    if (used != null) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(bar.left, bar.top, size.width * used.clamp(0, 1), 3),
          radius,
        ),
        Paint()..color = fill,
      );
    }
    final at = tick;
    if (at != null) {
      canvas.drawRect(
        Rect.fromLTWH(size.width * at.clamp(0, 1), 0, 1, size.height),
        Paint()..color = mark,
      );
    }
  }

  @override
  bool shouldRepaint(_FillPainter old) =>
      old.fraction != fraction ||
      old.tick != tick ||
      old.track != track ||
      old.fill != fill ||
      old.mark != mark;
}
