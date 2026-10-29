import 'package:flutter/widgets.dart';

/// Liquid's logo mark, drawn as `docs/design/liquid-logo.svg` draws it on its 100 × 100 grid.
class LiquidMark extends StatelessWidget {
  const LiquidMark({super.key, this.size = 18});

  final double size;

  @override
  Widget build(BuildContext context) =>
      CustomPaint(size: Size.square(size), painter: const _MarkPainter());
}

class _MarkPainter extends CustomPainter {
  const _MarkPainter();

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 100, size.height / 100);
    const square = Rect.fromLTWH(0, 0, 100, 100);
    canvas.drawRRect(
      RRect.fromRectAndRadius(square, const Radius.circular(23)),
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [Color(0xFF3D9BFF), Color(0xFF0057C2)],
        ).createShader(square),
    );
    Paint stroke(double opacity, double width) => Paint()
      ..color = const Color(0xFFFFFFFF).withValues(alpha: opacity)
      ..style = PaintingStyle.stroke
      ..strokeWidth = width;
    for (final (width, height, opacity) in [
      (70.0, 16.0, 0.35),
      (44.0, 10.0, 0.65),
    ]) {
      canvas.drawOval(
        Rect.fromCenter(
          center: const Offset(47, 75),
          width: width,
          height: height,
        ),
        stroke(opacity, 2.5),
      );
    }
    canvas.drawPath(
      Path()
        ..moveTo(40, 18)
        ..lineTo(40, 66)
        ..lineTo(64, 66),
      stroke(1, 13)
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_MarkPainter old) => false;
}
