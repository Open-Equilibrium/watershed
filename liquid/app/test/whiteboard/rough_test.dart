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
}
