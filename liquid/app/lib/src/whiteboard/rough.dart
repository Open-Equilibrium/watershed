import 'dart:math' as math;
import 'dart:ui' show Offset, Path, Rect;

class RoughOptions {
  const RoughOptions({
    required this.seed,
    this.roughness = 1,
    this.bowing = 1,
    this.maxRandomnessOffset = 2,
    this.curveStepCount = 9,
    this.curveFitting = 0.95,
  }) : assert(seed > 0 && seed <= 0x7FFFFFFF, 'seed must be 1 to 2^31 - 1');

  final int seed;

  final double roughness;

  final double bowing;

  final double maxRandomnessOffset;

  final int curveStepCount;

  final double curveFitting;
}

typedef _Ellipse = ({double increment, double rx, double ry});

Path roughLine(Offset a, Offset b, RoughOptions options) =>
    roughPolyline([a, b], options);

Path roughPolyline(List<Offset> points, RoughOptions options) =>
    (_Sketch(options)..polyline(points, close: false)).path;

Path roughPolygon(List<Offset> points, RoughOptions options) =>
    (_Sketch(options)..polyline(points, close: true)).path;

Path roughEllipse(Rect bounds, RoughOptions options) {
  final sketch = _Sketch(options);
  final params = sketch.ellipseParams(bounds.width, bounds.height);
  sketch.ellipse(bounds.center, params);
  return sketch.path;
}

double _lengthGain(double length) {
  if (length < 200) return 1;
  if (length > 500) return 0.4;
  return -0.0016668 * length + 1.233334;
}

final class _Sketch {
  _Sketch(RoughOptions options) : _options = options, _state = options.seed;

  final RoughOptions _options;
  final Path path = Path();

  int _state;

  double _next() {
    _state = (_state * 48271) & 0xFFFFFFFF;
    return (_state & 0x7FFFFFFF) / 0x80000000;
  }

  double _offset(double min, double max, [double gain = 1]) =>
      _options.roughness * gain * (_next() * (max - min) + min);

  double _offsetOpt(double x, [double gain = 1]) => _offset(-x, x, gain);

  void polyline(List<Offset> points, {required bool close}) {
    if (points.length == 2) {
      _doubleLine(points[0], points[1]);
    } else if (points.length > 2) {
      for (var i = 0; i < points.length - 1; i++) {
        _doubleLine(points[i], points[i + 1]);
      }
      if (close) _doubleLine(points.last, points.first);
    }
  }

  void _doubleLine(Offset a, Offset b) {
    _line(a.dx, a.dy, b.dx, b.dy, overlay: false);
    _line(a.dx, a.dy, b.dx, b.dy, overlay: true);
  }

  void _line(
    double x1,
    double y1,
    double x2,
    double y2, {
    required bool overlay,
  }) {
    final lengthSq = (x1 - x2) * (x1 - x2) + (y1 - y2) * (y1 - y2);
    final length = math.sqrt(lengthSq);
    final gain = _lengthGain(length);
    final maxOffset = _options.maxRandomnessOffset;
    final offset = maxOffset * maxOffset * 100 > lengthSq
        ? length / 10
        : maxOffset;
    final spread = overlay ? offset / 2 : offset;
    double jitter() => _offsetOpt(spread, gain);

    final divergePoint = 0.2 + _next() * 0.2;
    final bow = _options.bowing * maxOffset;
    final midX = _offsetOpt(bow * (y2 - y1) / 200, gain);
    final midY = _offsetOpt(bow * (x1 - x2) / 200, gain);
    path.moveTo(x1 + jitter(), y1 + jitter());
    path.cubicTo(
      midX + x1 + (x2 - x1) * divergePoint + jitter(),
      midY + y1 + (y2 - y1) * divergePoint + jitter(),
      midX + x1 + 2 * (x2 - x1) * divergePoint + jitter(),
      midY + y1 + 2 * (y2 - y1) * divergePoint + jitter(),
      x2 + jitter(),
      y2 + jitter(),
    );
  }

  _Ellipse ellipseParams(double width, double height) {
    final halfW = width / 2;
    final halfH = height / 2;
    final psq = math.sqrt(
      math.pi * 2 * math.sqrt((halfW * halfW + halfH * halfH) / 2),
    );
    final steps = math
        .max(
          _options.curveStepCount.toDouble(),
          _options.curveStepCount / math.sqrt(200) * psq,
        )
        .ceil();
    var rx = halfW.abs();
    var ry = halfH.abs();
    final fitRandomness = 1 - _options.curveFitting;
    rx += _offsetOpt(rx * fitRandomness);
    ry += _offsetOpt(ry * fitRandomness);
    return (increment: math.pi * 2 / steps, rx: rx, ry: ry);
  }

  void ellipse(Offset center, _Ellipse e) {
    final overlap = e.increment * _offset(0.1, _offset(0.4, 1));
    _curve(_ellipsePoints(center.dx, center.dy, e, 1, overlap));
    if (_options.roughness != 0) {
      _curve(_ellipsePoints(center.dx, center.dy, e, 1.5, 0));
    }
  }

  List<Offset> _ellipsePoints(
    double cx,
    double cy,
    _Ellipse e,
    double offset,
    double overlap,
  ) {
    if (_options.roughness == 0) {
      final step = e.increment / 4;
      Offset exact(double angle) =>
          Offset(cx + e.rx * math.cos(angle), cy + e.ry * math.sin(angle));
      return [
        exact(-step),
        for (var angle = 0.0; angle <= math.pi * 2; angle += step) exact(angle),
        exact(0),
        exact(step),
      ];
    }
    final radOffset = _offsetOpt(0.5) - math.pi / 2;
    final endAngle = math.pi * 2 + radOffset - 0.01;
    Offset around(double angle, double scale) => Offset(
      _offsetOpt(offset) + cx + scale * e.rx * math.cos(angle),
      _offsetOpt(offset) + cy + scale * e.ry * math.sin(angle),
    );
    return [
      around(radOffset - e.increment, 0.9),
      for (var angle = radOffset; angle < endAngle; angle += e.increment)
        around(angle, 1),
      around(radOffset + math.pi * 2 + overlap * 0.5, 1),
      around(radOffset + overlap, 0.98),
      around(radOffset + overlap * 0.5, 0.9),
    ];
  }

  void _curve(List<Offset> points) {
    path.moveTo(points[1].dx, points[1].dy);
    for (var i = 1; i + 2 < points.length; i++) {
      final from = points[i];
      final to = points[i + 1];
      final c1 = from + (to - points[i - 1]) / 6;
      final c2 = to + (from - points[i + 2]) / 6;
      path.cubicTo(c1.dx, c1.dy, c2.dx, c2.dy, to.dx, to.dy);
    }
  }
}
