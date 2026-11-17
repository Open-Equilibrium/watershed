import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui';

import 'package:perfect_freehand/perfect_freehand.dart';

import '../rust/api/whiteboard.dart';
import '../theme/tokens.g.dart';
import 'ink_geometry.dart';
import 'rough.dart';

/// A Whiteboard's elements as the canvas draws and touches them (ADR-0306): an element's
/// frame is its box, turned around its center; a stroke's points, as drawn, are stretched
/// to fill its box, and a line runs across its box from corner to corner.

/// The pens' widths, thin to bold, in board points.
const penSizes = [2.5, 4.0, 7.0];

/// The shapes' line widths, thin to bold, as Excalidraw's thin, bold and extra bold.
const shapeWidths = [1.0, 2.0, 4.0];

/// Text sizes S, M and L, as Excalidraw's.
const textSizes = [16.0, 20.0, 28.0];

/// A new text box's width.
const textWidth = 240.0;

/// The hand-drawn typeface (Excalifont, OFL-1.1).
const handFont = 'Excalifont';

/// The color an element is drawn in, by the theme's tokens.
Color inkColor(WsPalette ws, BridgeInkColor color) => switch (color) {
  BridgeInkColor.text => ws.text,
  BridgeInkColor.data1 => ws.data1,
  BridgeInkColor.data2 => ws.data2,
  BridgeInkColor.data3 => ws.data3,
};

/// perfect_freehand's options for a pen, as GoodNotes' ball, fountain and brush pens.
StrokeOptions penOptions(BridgePen pen, int width, {bool complete = true}) =>
    StrokeOptions(
      size: penSizes[width.clamp(0, 2)],
      thinning: switch (pen) {
        BridgePen.ball => 0,
        BridgePen.fountain => 0.5,
        BridgePen.brush => 0.8,
      },
      smoothing: 0.5,
      streamline: 0.5,
      simulatePressure: false,
      isComplete: complete,
    );

/// The closed outline perfect_freehand draws around `points`.
Path inkOutline(List<InkPoint> points, StrokeOptions options) {
  final outline = getStroke([
    for (final point in points) PointVector(point.x, point.y, point.pressure),
  ], options: options);
  final path = Path();
  if (outline.isEmpty) return path;
  path.moveTo(outline.first.dx, outline.first.dy);
  for (var at = 1; at < outline.length - 1; at++) {
    // Through the midpoints, as perfect_freehand's own renderers draw it.
    final mid = Offset.lerp(outline[at], outline[at + 1], 0.5)!;
    path.quadraticBezierTo(outline[at].dx, outline[at].dy, mid.dx, mid.dy);
  }
  path.close();
  return path;
}

/// The box `points` fill.
Rect pointsBounds(List<InkPoint> points) {
  var (left, top, right, bottom) = (
    double.infinity,
    double.infinity,
    -double.infinity,
    -double.infinity,
  );
  for (final point in points) {
    left = math.min(left, point.x);
    top = math.min(top, point.y);
    right = math.max(right, point.x);
    bottom = math.max(bottom, point.y);
  }
  return Rect.fromLTRB(left, top, right, bottom);
}

/// An element's frame, unturned.
Rect frameOf(BridgeElement element) =>
    Rect.fromLTWH(element.x, element.y, element.w, element.h);

/// Turns `local`, a point in the unturned frame, as the element's rotation turns it.
Offset turned(BridgeElement element, Offset local) {
  if (element.rotation == 0) return local;
  final center = frameOf(element).center;
  final (sin, cos) = (math.sin(element.rotation), math.cos(element.rotation));
  final d = local - center;
  return center + Offset(d.dx * cos - d.dy * sin, d.dx * sin + d.dy * cos);
}

/// The reverse of [turned]: a board point in the element's unturned frame.
Offset unturned(BridgeElement element, Offset board) {
  if (element.rotation == 0) return board;
  final center = frameOf(element).center;
  final (sin, cos) = (math.sin(-element.rotation), math.cos(-element.rotation));
  final d = board - center;
  return center + Offset(d.dx * cos - d.dy * sin, d.dx * sin + d.dy * cos);
}

/// The corners of an element's turned frame.
List<Offset> corners(BridgeElement element) {
  final frame = frameOf(element);
  return [
    turned(element, frame.topLeft),
    turned(element, frame.topRight),
    turned(element, frame.bottomRight),
    turned(element, frame.bottomLeft),
  ];
}

/// The box an element takes on the board, turned, with room for its line.
Rect boundsOf(BridgeElement element) {
  final points = corners(element);
  var bounds = Rect.fromPoints(points[0], points[2]);
  for (final point in points) {
    bounds = bounds.expandToInclude(
      Rect.fromCenter(center: point, width: 0, height: 0),
    );
  }
  return bounds.inflate(penSizes.last);
}

/// The new elements and their decoded points, kept per element: an element's points never
/// change, so a stroke is decoded and outlined once.
class BoardCache {
  final _points = <String, List<InkPoint>>{};
  final _outlines = <String, Path>{};
  final _shapes = <(String, double, double, bool, int), Path>{};

  List<InkPoint> points(BridgeElement element) =>
      _points.putIfAbsent(element.id ?? '', () => _decode(element.points));

  static List<InkPoint> _decode(Uint8List bytes) {
    try {
      return unpackPoints(bytes);
    } on FormatException {
      return const [];
    }
  }

  /// A stroke's outline in its points' own place, before its frame moves it.
  Path outline(BridgeElement element) => _outlines.putIfAbsent(
    '${element.id} ${element.pen.name} ${element.width}',
    () => inkOutline(points(element), penOptions(element.pen, element.width)),
  );

  /// A shape's lines in a frame of `size` at the origin, hand-drawn or clean.
  Path shape(BridgeElement element, bool hand) => _shapes.putIfAbsent((
    element.id ?? '',
    element.w,
    element.h,
    hand,
    element.width,
  ), () => shapePath(element, hand));

  /// Forgets the elements no longer on the board.
  void keepOnly(Iterable<String> ids) {
    final kept = ids.toSet();
    _points.removeWhere((id, _) => !kept.contains(id));
    _outlines.removeWhere((key, _) => !kept.contains(key.split(' ').first));
    _shapes.removeWhere((key, _) => !kept.contains(key.$1));
  }
}

/// The lines of a shape in its unturned frame moved to the origin.
Path shapePath(BridgeElement element, bool hand) {
  final (w, h) = (element.w, element.h);
  final options = RoughOptions(seed: element.seed == 0 ? 1 : element.seed);
  switch (element.shape) {
    case BridgeShape.rectangle:
      final box = Rect.fromLTWH(0, 0, w, h);
      return hand
          ? roughPolygon([
              box.topLeft,
              box.topRight,
              box.bottomRight,
              box.bottomLeft,
            ], options)
          : (Path()..addRRect(
              RRect.fromRectAndRadius(box, const Radius.circular(4)),
            ));
    case BridgeShape.ellipse:
      final box = Rect.fromLTWH(0, 0, w, h);
      return hand ? roughEllipse(box, options) : (Path()..addOval(box));
    case BridgeShape.triangle:
      final c = element.corners.length == 6
          ? element.corners
          : const [0.5, 0.0, 1.0, 1.0, 0.0, 1.0];
      final points = [
        for (var at = 0; at < 6; at += 2) Offset(c[at] * w, c[at + 1] * h),
      ];
      return hand
          ? roughPolygon(points, options)
          : (Path()..addPolygon(points, true));
    case BridgeShape.line:
      final (start, end) = (Offset.zero, Offset(w, h));
      final path = hand
          ? roughLine(start, end, options)
          : (Path()
              ..moveTo(0, 0)
              ..lineTo(w, h));
      if (element.arrowhead && (end - start).distance > 0) {
        final angle = (end - start).direction;
        final length = math.min(16.0, (end - start).distance / 3);
        Offset wing(double side) =>
            end - Offset.fromDirection(angle + side, length);
        final head = [wing(0.45), end, wing(-0.45)];
        path.addPath(
          hand
              ? roughPolyline(head, options)
              : (Path()..addPolygon(head, false)),
          Offset.zero,
        );
      }
      return path;
  }
}

/// Points along an element, to tell whether a lasso loop holds it: a stroke's own points,
/// at most 64, or a frame's corners, edges' middles and center.
List<Offset> samplesOf(BridgeElement element, BoardCache cache) {
  if (element.kind == BridgeElementKind.stroke) {
    final points = cache.points(element);
    final step = math.max(1, points.length ~/ 64);
    return [
      for (var at = 0; at < points.length; at += step)
        strokePoint(element, cache, points[at].offset),
    ];
  }
  final frame = frameOf(element);
  return [
    for (final point in [
      frame.topLeft,
      frame.topCenter,
      frame.topRight,
      frame.centerRight,
      frame.bottomRight,
      frame.bottomCenter,
      frame.bottomLeft,
      frame.centerLeft,
      frame.center,
    ])
      turned(element, point),
  ];
}

/// Where a stroke's drawn point lies on the board, its frame stretching its points.
Offset strokePoint(BridgeElement element, BoardCache cache, Offset drawn) {
  final source = pointsBounds(cache.points(element));
  final sx = source.width == 0 ? 1.0 : element.w / source.width;
  final sy = source.height == 0 ? 1.0 : element.h / source.height;
  return turned(
    element,
    Offset(
      element.x + (drawn.dx - source.left) * sx,
      element.y + (drawn.dy - source.top) * sy,
    ),
  );
}

/// Whether a board point lies on an element, within `slop`.
bool touches(BridgeElement element, BoardCache cache, Offset at, double slop) {
  switch (element.kind) {
    case BridgeElementKind.stroke:
      final points = cache.points(element);
      final half = penSizes[element.width.clamp(0, 2)] / 2 + slop;
      for (var i = 0; i < points.length; i++) {
        final a = strokePoint(element, cache, points[i].offset);
        final b = i + 1 < points.length
            ? strokePoint(element, cache, points[i + 1].offset)
            : a;
        if (distanceToSegment(at, a, b) <= half) return true;
      }
      return false;
    case BridgeElementKind.shape when element.shape == BridgeShape.line:
      final local = unturned(element, at);
      return distanceToSegment(
            local,
            Offset(element.x, element.y),
            Offset(element.x + element.w, element.y + element.h),
          ) <=
          slop + shapeWidths[element.width.clamp(0, 2)];
    case BridgeElementKind.shape || BridgeElementKind.text:
      return frameOf(element).inflate(slop).contains(unturned(element, at));
  }
}

/// A new element of `kind` with nothing yet but its look.
BridgeElement newElement(
  BridgeElementKind kind, {
  required Rect frame,
  required BridgeInkColor color,
  required int width,
  BridgePen pen = BridgePen.ball,
  Uint8List? points,
  BridgeShape shape = BridgeShape.rectangle,
  bool arrowhead = false,
  List<double> corners = const [],
  String text = '',
  double rotation = 0,
}) => BridgeElement(
  kind: kind,
  x: frame.left,
  y: frame.top,
  w: frame.width,
  h: frame.height,
  rotation: rotation,
  color: color,
  width: width,
  pen: pen,
  points: points ?? Uint8List(0),
  shape: shape,
  seed: math.Random().nextInt(1 << 31),
  arrowhead: arrowhead,
  corners: Float64List.fromList(corners),
  text: text,
);

/// The change that gives `element` the frame `frame`, turned `rotation`, and its look.
BridgeElementChange changeOf(
  BridgeElement element, {
  Rect? frame,
  double? rotation,
  BridgeInkColor? color,
  int? width,
  String? text,
  bool? arrowhead,
}) {
  final box = frame ?? frameOf(element);
  return BridgeElementChange(
    id: element.id!,
    x: box.left,
    y: box.top,
    w: box.width,
    h: box.height,
    rotation: rotation ?? element.rotation,
    color: color ?? element.color,
    width: width ?? element.width,
    text: text,
    arrowhead: arrowhead,
  );
}
