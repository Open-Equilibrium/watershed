import 'package:flutter/material.dart';

class FillBar extends StatelessWidget {
  const FillBar({
    super.key,
    required this.fraction,
    this.tick,
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
