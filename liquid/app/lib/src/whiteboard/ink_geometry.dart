import 'dart:math' show max, min, sqrt;
import 'dart:typed_data';
import 'dart:ui' show Offset;

class InkPoint {
  const InkPoint(this.x, this.y, [this.pressure = 0.5]);

  final double x, y, pressure;

  Offset get offset => Offset(x, y);

  @override
  bool operator ==(Object other) =>
      other is InkPoint &&
      other.x == x &&
      other.y == y &&
      other.pressure == pressure;

  @override
  int get hashCode => Object.hash(x, y, pressure);
}

const _pointBytes = 12;

Uint8List packPoints(List<InkPoint> points) {
  final bytes = Uint8List(points.length * _pointBytes);
  final data = ByteData.sublistView(bytes);
  for (var i = 0; i < points.length; i++) {
    final at = i * _pointBytes;
    final p = points[i];
    data
      ..setFloat32(at, p.x, Endian.little)
      ..setFloat32(at + 4, p.y, Endian.little)
      ..setFloat32(at + 8, p.pressure, Endian.little);
  }
  return bytes;
}

List<InkPoint> unpackPoints(Uint8List bytes) {
  if (bytes.lengthInBytes % _pointBytes != 0) {
    throw FormatException(
      'ink data length ${bytes.lengthInBytes} is not a multiple of $_pointBytes',
    );
  }
  final data = ByteData.sublistView(bytes);
  return [
    for (var at = 0; at < bytes.lengthInBytes; at += _pointBytes)
      InkPoint(
        data.getFloat32(at, Endian.little),
        data.getFloat32(at + 4, Endian.little),
        data.getFloat32(at + 8, Endian.little),
      ),
  ];
}

double distanceToSegment(Offset p, Offset a, Offset b) {
  final d = b - a;
  final length2 = _dot(d, d);
  final t = length2 == 0 ? 0.0 : min(1.0, max(0.0, _dot(p - a, d) / length2));
  return (p - (a + d * t)).distance;
}

bool strokeTouches(
  List<InkPoint> points,
  Offset center,
  double radius,
  double halfWidth,
) => _segments(points).any(
  (s) =>
      distanceToSegment(center, s.$1.offset, s.$2.offset) <= radius + halfWidth,
);

List<List<InkPoint>> eraseInside(
  List<InkPoint> points,
  Offset center,
  double radius, {
  double halfWidth = 0,
  double minLength = 1.0,
}) {
  if (points.isEmpty) return [];
  if (!strokeTouches(points, center, radius, halfWidth)) return [points];
  final reach = radius + halfWidth;
  final pieces = <List<InkPoint>>[];
  var piece = <InkPoint>[];
  void keep() {
    if (piece.isNotEmpty && _length(piece) >= minLength) pieces.add(piece);
    piece = [];
  }

  for (final (a, b) in _segments(points)) {
    final ts = [0.0, ..._cutTimes(a.offset, b.offset, center, reach), 1.0];
    for (var k = 0; k + 1 < ts.length; k++) {
      final mid = _at(a, b, (ts[k] + ts[k + 1]) / 2);
      if ((mid.offset - center).distance <= reach) {
        keep();
      } else {
        if (piece.isEmpty) piece.add(_at(a, b, ts[k]));
        final end = _at(a, b, ts[k + 1]);
        if (end != piece.last) piece.add(end);
      }
    }
  }
  keep();
  return pieces;
}

bool lassoEncloses(List<Offset> loop, Offset p) {
  var inside = false;
  var previous = loop.length - 1;
  for (var i = 0; i < loop.length; i++) {
    final a = loop[previous];
    final b = loop[i];
    if ((a.dy > p.dy) != (b.dy > p.dy) &&
        p.dx < (b.dx - a.dx) * (p.dy - a.dy) / (b.dy - a.dy) + a.dx) {
      inside = !inside;
    }
    previous = i;
  }
  return inside;
}

bool lassoSelects(List<Offset> loop, List<Offset> samples) {
  final inside = samples.where((s) => lassoEncloses(loop, s)).length;
  return inside * 2 > samples.length;
}

double _dot(Offset u, Offset v) => u.dx * v.dx + u.dy * v.dy;

Iterable<(InkPoint, InkPoint)> _segments(List<InkPoint> points) sync* {
  if (points.length == 1) yield (points[0], points[0]);
  for (var i = 0; i + 1 < points.length; i++) {
    yield (points[i], points[i + 1]);
  }
}

List<double> _cutTimes(Offset a, Offset b, Offset center, double radius) {
  final d = b - a;
  final f = a - center;
  final qa = _dot(d, d);
  if (qa == 0) return const [];
  final qb = 2 * _dot(f, d);
  final qc = _dot(f, f) - radius * radius;
  final disc = qb * qb - 4 * qa * qc;
  if (disc < 0) return const [];
  final root = sqrt(disc);
  return [
    (-qb - root) / (2 * qa),
    (-qb + root) / (2 * qa),
  ].where((t) => t > 0 && t < 1).toList();
}

InkPoint _at(InkPoint a, InkPoint b, double t) {
  if (t == 0) return a;
  if (t == 1) return b;
  return InkPoint(
    a.x + (b.x - a.x) * t,
    a.y + (b.y - a.y) * t,
    a.pressure + (b.pressure - a.pressure) * t,
  );
}

double _length(List<InkPoint> piece) {
  var total = 0.0;
  for (var i = 0; i + 1 < piece.length; i++) {
    total += (piece[i + 1].offset - piece[i].offset).distance;
  }
  return total;
}
