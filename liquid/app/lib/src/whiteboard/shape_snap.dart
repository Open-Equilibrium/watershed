import 'dart:math' show atan2, cos, max, min, pi, sin, sqrt;
import 'dart:ui' show Offset;

/// Shapes a drawn stroke can snap into.
enum SnapKind { line, rectangle, ellipse, triangle }

/// A perfect shape a drawn stroke snaps into.
class SnappedShape {
  const SnappedShape({
    required this.kind,
    this.points = const [],
    this.center = Offset.zero,
    this.width = 0,
    this.height = 0,
    this.rotation = 0,
  });

  final SnapKind kind;

  /// Line: its two ends. Triangle: its three corners. Otherwise empty.
  final List<Offset> points;

  /// Rectangle and ellipse only: centre, size and rotation.
  final Offset center;
  final double width;
  final double height;

  /// Radians, clockwise on screen as Flutter's canvas rotates.
  final double rotation;
}

/// Strokes shorter than this along their path (logical px) are no shape.
const _minLength = 12.0;

/// Ends at least this share of the bounding diagonal apart make a stroke open.
const _openShare = 0.25;

/// An open stroke is a line when its ends span this share of its path.
const _straightShare = 0.9;

/// Ramer–Douglas–Peucker tolerance, as a share of the bounding diagonal.
const _simplifyShare = 0.08;

/// A closed outline must cover this share of its convex hull.
const _solidShare = 0.8;

/// A triangle's corners must cover this share of the hull; a rectangle's hull
/// must fill this share of its box.
const _fillShare = 0.9;

/// Largest mean |r - 1| over the stroke, r being the normalised radius.
const _ellipseError = 0.05;

/// Boxes within this angle of an axis snap to it.
const _snapAngle = 10 * pi / 180;

/// A box: centre, sides and the angle of its width axis.
typedef _Box = ({Offset center, double width, double height, double angle});

/// The shape [points] were meant to be; null for anything else.
SnappedShape? snapShape(List<Offset> points) {
  if (points.length < 3) return null;
  final length = _pathLength(points);
  if (length < _minLength) return null;
  final diagonal = _extent(points).distance;
  final gap = (points.last - points.first).distance;
  if (gap < _openShare * diagonal) return _closed(points, diagonal);
  if (gap < _straightShare * length) return null;
  return SnappedShape(kind: SnapKind.line, points: [points.first, points.last]);
}

SnappedShape? _closed(List<Offset> points, double diagonal) {
  final hull = _hull(points);
  final hullArea = _area(hull);
  // A loop, spiral or figure eight lies far from its own hull: no shape.
  if (hull.length < 3 || _area(points) < _solidShare * hullArea) return null;
  final corners = _simplifyRing(hull, _simplifyShare * diagonal);
  if (corners.length == 3 && _area(corners) >= _fillShare * hullArea) {
    return SnappedShape(kind: SnapKind.triangle, points: corners);
  }
  final box = _minAreaBox(hull);
  if (hullArea >= _fillShare * box.width * box.height) {
    return _boxShape(SnapKind.rectangle, box);
  }
  return _ellipse(points);
}

/// An ellipse along the stroke's principal (PCA) axes, if the stroke hugs it.
SnappedShape? _ellipse(List<Offset> points) {
  var mean = Offset.zero;
  for (final p in points) {
    mean += p;
  }
  mean /= points.length.toDouble();
  var xx = 0.0, yy = 0.0, xy = 0.0;
  for (final p in points) {
    final d = p - mean;
    xx += d.dx * d.dx;
    yy += d.dy * d.dy;
    xy += d.dx * d.dy;
  }
  final box = _boxAt(points, 0.5 * atan2(2 * xy, xx - yy));
  final a = box.width / 2, b = box.height / 2;
  if (a <= 0 || b <= 0) return null;
  final c = cos(box.angle), s = sin(box.angle);
  var error = 0.0;
  for (final p in points) {
    final d = p - box.center;
    final u = (d.dx * c + d.dy * s) / a, v = (d.dy * c - d.dx * s) / b;
    error += (sqrt(u * u + v * v) - 1).abs();
  }
  if (error / points.length > _ellipseError) return null;
  return _boxShape(SnapKind.ellipse, box);
}

/// [box] as a shape whose width runs within 45 degrees of level, then snapped
/// to level when within [_snapAngle] of it.
SnappedShape _boxShape(SnapKind kind, _Box box) {
  var angle = box.angle % pi;
  if (angle > pi / 2) angle -= pi;
  var (width, height) = (box.width, box.height);
  if (angle.abs() > pi / 4) {
    angle -= angle.sign * pi / 2;
    (width, height) = (height, width);
  }
  if (angle.abs() <= _snapAngle) angle = 0;
  return SnappedShape(
    kind: kind,
    center: box.center,
    width: width,
    height: height,
    rotation: angle,
  );
}

/// The smallest box around the convex [hull], with a side along one edge.
_Box _minAreaBox(List<Offset> hull) {
  _Box? best;
  for (var i = 0; i < hull.length; i++) {
    final edge = hull[(i + 1) % hull.length] - hull[i];
    final box = _boxAt(hull, atan2(edge.dy, edge.dx));
    if (best == null || box.width * box.height < best.width * best.height) {
      best = box;
    }
  }
  return best!;
}

/// The box around [points] whose width axis is turned by [angle].
_Box _boxAt(List<Offset> points, double angle) {
  final c = cos(angle), s = sin(angle);
  var minU = double.infinity, maxU = -double.infinity;
  var minV = double.infinity, maxV = -double.infinity;
  for (final p in points) {
    final u = p.dx * c + p.dy * s, v = p.dy * c - p.dx * s;
    minU = min(minU, u);
    maxU = max(maxU, u);
    minV = min(minV, v);
    maxV = max(maxV, v);
  }
  final cu = (minU + maxU) / 2, cv = (minV + maxV) / 2;
  return (
    center: Offset(cu * c - cv * s, cu * s + cv * c),
    width: maxU - minU,
    height: maxV - minV,
    angle: angle,
  );
}

/// Convex hull by Andrew's monotone chain.
List<Offset> _hull(List<Offset> points) {
  final sorted = [
    ...points,
  ]..sort((a, b) => a.dx != b.dx ? a.dx.compareTo(b.dx) : a.dy.compareTo(b.dy));
  List<Offset> chain(Iterable<Offset> ordered) {
    final out = <Offset>[];
    for (final p in ordered) {
      while (out.length >= 2 && _cross(out[out.length - 2], out.last, p) <= 0) {
        out.removeLast();
      }
      out.add(p);
    }
    out.removeLast();
    return out;
  }

  return [...chain(sorted), ...chain(sorted.reversed)];
}

/// Ramer–Douglas–Peucker over a closed [ring]. It anchors at the vertex farthest
/// from the centroid, a corner of any triangle, so no corner gets cut.
List<Offset> _simplifyRing(List<Offset> ring, double epsilon) {
  final centroid =
      ring.fold(Offset.zero, (sum, p) => sum + p) / ring.length.toDouble();
  final start = _farthest(ring, centroid);
  final turned = [...ring.sublist(start), ...ring.sublist(0, start)];
  final far = _farthest(turned, turned[0]);
  final first = _simplifyChain(turned.sublist(0, far + 1), epsilon);
  final second = _simplifyChain([...turned.sublist(far), turned[0]], epsilon);
  return [...first, ...second.sublist(1, second.length - 1)];
}

/// Index of the vertex of [ring] farthest from [from].
int _farthest(List<Offset> ring, Offset from) {
  var far = 0;
  for (var i = 1; i < ring.length; i++) {
    if ((ring[i] - from).distance > (ring[far] - from).distance) far = i;
  }
  return far;
}

/// Ramer–Douglas–Peucker over an open [chain]; keeps both ends.
List<Offset> _simplifyChain(List<Offset> chain, double epsilon) {
  final keep = List.filled(chain.length, false);
  keep[0] = true;
  keep[chain.length - 1] = true;
  void split(int from, int to) {
    var worst = -1;
    var worstDistance = epsilon;
    for (var i = from + 1; i < to; i++) {
      final d = _distanceToLine(chain[i], chain[from], chain[to]);
      if (d > worstDistance) {
        worst = i;
        worstDistance = d;
      }
    }
    if (worst < 0) return;
    keep[worst] = true;
    split(from, worst);
    split(worst, to);
  }

  split(0, chain.length - 1);
  return [
    for (var i = 0; i < chain.length; i++)
      if (keep[i]) chain[i],
  ];
}

/// Distance from [p] to the line through [a] and [b].
double _distanceToLine(Offset p, Offset a, Offset b) {
  final length = (b - a).distance;
  return length == 0 ? (p - a).distance : _cross(a, b, p).abs() / length;
}

/// Cross product of the vectors [o]→[a] and [o]→[b].
double _cross(Offset o, Offset a, Offset b) {
  final oa = a - o, ob = b - o;
  return oa.dx * ob.dy - oa.dy * ob.dx;
}

/// Area of the polygon through [ring], closed back to its first vertex.
double _area(List<Offset> ring) {
  var twice = 0.0;
  for (var i = 0; i < ring.length; i++) {
    final p = ring[i], q = ring[(i + 1) % ring.length];
    twice += p.dx * q.dy - q.dx * p.dy;
  }
  return twice.abs() / 2;
}

double _pathLength(List<Offset> points) {
  var total = 0.0;
  for (var i = 1; i < points.length; i++) {
    total += (points[i] - points[i - 1]).distance;
  }
  return total;
}

/// Width and height of the bounding box of [points].
Offset _extent(List<Offset> points) {
  var lo = points.first, hi = points.first;
  for (final p in points) {
    lo = Offset(min(lo.dx, p.dx), min(lo.dy, p.dy));
    hi = Offset(max(hi.dx, p.dx), max(hi.dy, p.dy));
  }
  return hi - lo;
}
