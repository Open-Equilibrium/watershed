import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/shape_snap.dart';

const _tolerance = 8.0;

const _angleTolerance = 0.05;

List<Offset> _walk(List<Offset> corners) {
  final out = <Offset>[];
  for (var i = 0; i + 1 < corners.length; i++) {
    final a = corners[i], b = corners[i + 1];
    final steps = (b - a).distance ~/ 5;
    for (var k = 0; k < steps; k++) {
      out.add(a + (b - a) * (k / steps));
    }
  }
  out.add(corners.last);
  return out;
}

List<Offset> _stroke(
  List<Offset> local,
  Random random, {
  Offset centre = Offset.zero,
  double angle = 0,
}) {
  final c = cos(angle), s = sin(angle);
  return [
    for (final p in local)
      centre +
          Offset(p.dx * c - p.dy * s, p.dx * s + p.dy * c) +
          Offset(random.nextDouble() * 4 - 2, random.nextDouble() * 4 - 2),
  ];
}

List<Offset> _rectangle(double w, double h) => _walk([
  Offset(-w / 2, -h / 2),
  Offset(w / 2, -h / 2),
  Offset(w / 2, h / 2),
  Offset(-w / 2, h / 2),
  Offset(-w / 2, -h / 2),
]);

List<Offset> _ellipse(double a, double b) => [
  for (var i = 0; i <= 72; i++)
    Offset(a * cos(i * pi / 36), b * sin(i * pi / 36)),
];

void _expectPointsNear(List<Offset> actual, List<Offset> expected) {
  for (final e in expected) {
    expect(
      actual.map((a) => (a - e).distance).reduce(min),
      lessThan(_tolerance),
      reason: '$e has no point near it in $actual',
    );
  }
}

void main() {
  const centre = Offset(300, 200);

  test('snaps a level rectangle and keeps it level', () {
    final shape = snapShape(
      _stroke(_rectangle(200, 100), Random(1), centre: centre),
    );
    expect(shape?.kind, SnapKind.rectangle);
    expect((shape!.center - centre).distance, lessThan(_tolerance));
    expect(shape.width, closeTo(200, _tolerance));
    expect(shape.height, closeTo(100, _tolerance));
    expect(shape.rotation, 0);
  });

  test('turns a box that is nearly upright level, swapping its sides', () {
    final shape = snapShape(
      _stroke(
        _rectangle(200, 100),
        Random(2),
        centre: centre,
        angle: 88 * pi / 180,
      ),
    );
    expect(shape?.kind, SnapKind.rectangle);
    expect(shape!.width, closeTo(100, _tolerance));
    expect(shape.height, closeTo(200, _tolerance));
    expect(shape.rotation, 0);
  });

  test('snaps a rectangle turned 30 degrees to that angle', () {
    final shape = snapShape(
      _stroke(_rectangle(200, 100), Random(3), centre: centre, angle: pi / 6),
    );
    expect(shape?.kind, SnapKind.rectangle);
    expect((shape!.center - centre).distance, lessThan(_tolerance));
    expect(shape.width, closeTo(200, _tolerance));
    expect(shape.height, closeTo(100, _tolerance));
    expect(shape.rotation, closeTo(pi / 6, _angleTolerance));
  });

  test('snaps an ellipse and keeps its turn', () {
    final shape = snapShape(
      _stroke(
        _ellipse(120, 60),
        Random(4),
        centre: centre,
        angle: 25 * pi / 180,
      ),
    );
    expect(shape?.kind, SnapKind.ellipse);
    expect((shape!.center - centre).distance, lessThan(_tolerance));
    expect(shape.width, closeTo(240, _tolerance));
    expect(shape.height, closeTo(120, _tolerance));
    expect(shape.rotation, closeTo(25 * pi / 180, _angleTolerance));
  });

  test('snaps a circle to an ellipse with equal sides', () {
    final shape = snapShape(
      _stroke(_ellipse(90, 90), Random(5), centre: centre),
    );
    expect(shape?.kind, SnapKind.ellipse);
    expect(shape!.width, closeTo(180, _tolerance));
    expect(shape.height, closeTo(180, _tolerance));
  });

  test('snaps a triangle to its three corners', () {
    final corners = [
      const Offset(-120, 80),
      const Offset(120, 80),
      const Offset(0, -110),
    ];
    final shape = snapShape(
      _stroke(_walk([...corners, corners.first]), Random(6), centre: centre),
    );
    expect(shape?.kind, SnapKind.triangle);
    expect(shape!.points, hasLength(3));
    _expectPointsNear(shape.points, [for (final c in corners) c + centre]);
  });

  test('snaps a straight stroke to a line between its ends', () {
    const angle = 0.25;
    final shape = snapShape(
      _stroke(
        _walk([const Offset(-150, 0), const Offset(150, 0)]),
        Random(7),
        centre: centre,
        angle: angle,
      ),
    );
    final half = Offset(150 * cos(angle), 150 * sin(angle));
    expect(shape?.kind, SnapKind.line);
    _expectPointsNear(shape!.points, [centre - half, centre + half]);
  });

  test('rejects waves, spirals, loops and dots', () {
    final random = Random(8);
    final strokes = {
      'wave': _stroke([
        for (var x = 0.0; x <= 300; x += 5)
          Offset(x - 150, 25 * sin(2 * pi * x / 100)),
      ], random),
      'spiral': _stroke([
        for (var i = 0; i <= 150; i++)
          Offset(
            (10 + 0.6 * i) * cos(i * pi / 25),
            (10 + 0.6 * i) * sin(i * pi / 25),
          ),
      ], random),
      'figure eight': _stroke([
        for (var i = 0; i <= 120; i++)
          Offset(120 * sin(i * pi / 60), 60 * sin(i * pi / 30)),
      ], random),
      'dot': const [
        Offset(50, 50),
        Offset(51, 50),
        Offset(50, 51),
        Offset(50, 50),
      ],
      'two points': const [Offset(0, 0), Offset(100, 0)],
    };
    for (final MapEntry(:key, :value) in strokes.entries) {
      expect(snapShape(value), isNull, reason: key);
    }
  });
}
