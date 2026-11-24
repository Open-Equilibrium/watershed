import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/rough.dart';

const _corners = [Offset(0, 0), Offset(100, 0), Offset(100, 60), Offset(0, 60)];
const _box = Rect.fromLTWH(0, 0, 100, 60);

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

  test('roughness 0 draws the straight segment', () {
    const a = Offset(10, 20);
    const b = Offset(90, 50);
    final path = roughLine(a, b, const RoughOptions(seed: 7, roughness: 0));
    final samples = _samples(path, step: 0.5);
    expect(samples, isNotEmpty);
    for (final p in samples) {
      expect(_distanceToSegment(p, a, b), lessThan(1e-3));
    }
  });
}
