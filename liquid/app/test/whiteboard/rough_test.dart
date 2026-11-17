import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/rough.dart';

const _corners = [Offset(0, 0), Offset(100, 0), Offset(100, 60), Offset(0, 60)];
const _box = Rect.fromLTWH(0, 0, 100, 60);

/// Positions sampled every [step] px along each contour, in path order.
List<Offset> _samples(Path path, {double step = 1}) {
  final points = <Offset>[];
  for (final metric in path.computeMetrics()) {
    for (var d = 0.0; d <= metric.length; d += step) {
      points.add(metric.getTangentForOffset(d)!.position);
    }
  }
  return points;
}

double _distanceToSegment(Offset p, Offset a, Offset b) {
  final ab = b - a;
  final t = ((p - a).dx * ab.dx + (p - a).dy * ab.dy) / ab.distanceSquared;
  return (p - (a + ab * t.clamp(0.0, 1.0).toDouble())).distance;
}

void main() {
  test('the same seed and input draw the same path', () {
    const options = RoughOptions(seed: 42);
    for (final draw in <Path Function()>[
      () => roughPolygon(_corners, options),
      () => roughEllipse(_box, options),
    ]) {
      final first = draw();
      final again = draw();
      expect(
        again.computeMetrics().map((m) => m.length).toList(),
        first.computeMetrics().map((m) => m.length).toList(),
      );
      expect(_samples(again), _samples(first));
    }
  });

  test('a different seed draws a different path', () {
    final first = roughPolygon(_corners, const RoughOptions(seed: 1));
    final other = roughPolygon(_corners, const RoughOptions(seed: 2));
    expect(_samples(other), isNot(equals(_samples(first))));
  });

  test('a rough rectangle stays within the sketch margin', () {
    const exact = Rect.fromLTWH(0, 0, 100, 60);
    var reachesOutside = false;
    for (var seed = 1; seed <= 50; seed++) {
      final options = RoughOptions(seed: seed);
      // rough.js moves a point by at most roughness * maxRandomnessOffset, and
      // bows a control point by at most bowing * maxRandomnessOffset * side / 200
      // for the longest side (100 px).
      final margin =
          options.roughness * options.maxRandomnessOffset +
          options.bowing * options.maxRandomnessOffset * 100 / 200;
      final bounds = roughPolygon(_corners, options).getBounds();
      final grown = exact.inflate(margin);
      expect(bounds.left, greaterThanOrEqualTo(grown.left));
      expect(bounds.top, greaterThanOrEqualTo(grown.top));
      expect(bounds.right, lessThanOrEqualTo(grown.right));
      expect(bounds.bottom, lessThanOrEqualTo(grown.bottom));
      reachesOutside |=
          bounds.left < 0 ||
          bounds.top < 0 ||
          bounds.right > 100 ||
          bounds.bottom > 60;
    }
    expect(reachesOutside, isTrue, reason: 'the sketch must actually wobble');
  });

  test('roughness 0 draws the straight segment', () {
    const a = Offset(10, 20);
    const b = Offset(90, 50);
    final path = roughLine(a, b, const RoughOptions(seed: 7, roughness: 0));
    final samples = _samples(path, step: 0.5);
    expect(samples, isNotEmpty);
    // Path stores float32 coordinates, so allow 1e-3 px of rounding.
    for (final p in samples) {
      expect(_distanceToSegment(p, a, b), lessThan(1e-3));
    }
  });

  test('a rough ellipse closes around its centre', () {
    final path = roughEllipse(_box, const RoughOptions(seed: 3));
    // Eight 45-degree sectors around the centre; each must hold outline.
    final sectors = <int>{};
    for (final p in _samples(path, step: 0.5)) {
      final angle = math.atan2(p.dy - _box.center.dy, p.dx - _box.center.dx);
      sectors.add(((angle + math.pi) / (math.pi / 4)).floor() % 8);
    }
    expect(sectors, hasLength(8));
  });
}
