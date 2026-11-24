import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:perfect_freehand/perfect_freehand.dart';

import '../rust/api/drawing.dart';
import '../theme/tokens.g.dart';
import 'ink_geometry.dart';
import 'rough.dart';

const penSizes = [2.5, 4.0, 7.0];

const shapeWidths = [1.0, 2.0, 4.0];

const textSizes = [16.0, 20.0, 28.0];

const textWidth = 240.0;

const handFont = 'Excalifont';

Color inkColor(WsPalette ws, BridgeInkColor color) => switch (color) {
  BridgeInkColor.text => ws.text,
  BridgeInkColor.data1 => ws.data1,
  BridgeInkColor.data2 => ws.data2,
  BridgeInkColor.data3 => ws.data3,
  BridgeInkColor.yellow => WsPalette.light.tag4,
  BridgeInkColor.green => WsPalette.light.tag5,
  BridgeInkColor.blue => WsPalette.light.tag6,
  BridgeInkColor.pink => WsPalette.light.tag8,
};

const inkColors = [
  BridgeInkColor.text,
  BridgeInkColor.data1,
  BridgeInkColor.data2,
  BridgeInkColor.data3,
];

const highlightColors = [
  BridgeInkColor.yellow,
  BridgeInkColor.green,
  BridgeInkColor.blue,
  BridgeInkColor.pink,
];

bool isHighlight(BridgeElement element) =>
    element.kind == BridgeElementKind.stroke &&
    element.pen == BridgePen.highlighter;

const highlighterSizes = [12.0, 18.0, 26.0];

StrokeOptions penOptions(BridgePen pen, int width, {bool complete = true}) =>
    StrokeOptions(
      size: (pen == BridgePen.highlighter
          ? highlighterSizes
          : penSizes)[width.clamp(0, 2)],
      thinning: switch (pen) {
        BridgePen.ball || BridgePen.highlighter => 0,
        BridgePen.fountain => 0.5,
        BridgePen.brush => 0.8,
      },
      smoothing: 0.5,
      streamline: 0.5,
      simulatePressure: false,
      isComplete: complete,
    );

Path inkOutline(List<InkPoint> points, StrokeOptions options) {
  final outline = getStroke([
    for (final point in points) PointVector(point.x, point.y, point.pressure),
  ], options: options);
  final path = Path();
  if (outline.isEmpty) return path;
  path.moveTo(outline.first.dx, outline.first.dy);
  for (var at = 1; at < outline.length - 1; at++) {
    final mid = Offset.lerp(outline[at], outline[at + 1], 0.5)!;
    path.quadraticBezierTo(outline[at].dx, outline[at].dy, mid.dx, mid.dy);
  }
  path.close();
  return path;
}

const maxStrokePoints = 10000;

List<List<InkPoint>> splitInk(Iterable<InkPoint> ink) {
  final points = ink.toList();
  final pieces = <List<InkPoint>>[];
  for (var start = 0; start < points.length - 1 || pieces.isEmpty;) {
    final end = (start + maxStrokePoints).clamp(0, points.length);
    pieces.add(points.sublist(start, end));
    if (end >= points.length) break;
    start = end - 1;
  }
  return pieces;
}

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

Rect frameOf(BridgeElement element) =>
    Rect.fromLTWH(element.x, element.y, element.w, element.h);

Offset turned(BridgeElement element, Offset local) {
  if (element.rotation == 0) return local;
  final center = frameOf(element).center;
  final (sin, cos) = (math.sin(element.rotation), math.cos(element.rotation));
  final d = local - center;
  return center + Offset(d.dx * cos - d.dy * sin, d.dx * sin + d.dy * cos);
}

Offset unturned(BridgeElement element, Offset board) {
  if (element.rotation == 0) return board;
  final center = frameOf(element).center;
  final (sin, cos) = (math.sin(-element.rotation), math.cos(-element.rotation));
  final d = board - center;
  return center + Offset(d.dx * cos - d.dy * sin, d.dx * sin + d.dy * cos);
}

List<Offset> corners(BridgeElement element) {
  final frame = frameOf(element);
  return [
    turned(element, frame.topLeft),
    turned(element, frame.topRight),
    turned(element, frame.bottomRight),
    turned(element, frame.bottomLeft),
  ];
}

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

class BoardCache {
  final _points = <String, List<InkPoint>>{};
  final _bounds = <String, Rect>{};

  final _outlines = <String, (Object, Path)>{};
  final _shapes = <String, (Object, Path)>{};

  @visibleForTesting
  int get entries => _outlines.length + _shapes.length;

  List<InkPoint> points(BridgeElement element) => switch (element.id) {
    final id? => _points.putIfAbsent(id, () => _decode(element.points)),
    null => _decode(element.points),
  };

  Rect bounds(BridgeElement element) => switch (element.id) {
    final id? => _bounds.putIfAbsent(id, () => pointsBounds(points(element))),
    null => pointsBounds(points(element)),
  };

  static List<InkPoint> _decode(Uint8List bytes) {
    try {
      return unpackPoints(bytes);
    } on FormatException {
      return const [];
    }
  }

  static Path _kept(
    Map<String, (Object, Path)> drawings,
    String? id,
    Object key,
    Path Function() draw,
  ) {
    if (id == null) return draw();
    if (drawings[id] case (final drawn, final path) when drawn == key) {
      return path;
    }
    final path = draw();
    drawings[id] = (key, path);
    return path;
  }

  Path outline(BridgeElement element) => _kept(_outlines, element.id, (
    element.pen,
    element.width,
  ), () => inkOutline(points(element), penOptions(element.pen, element.width)));

  Path shape(BridgeElement element, bool hand) => _kept(_shapes, element.id, (
    element.w,
    element.h,
    hand,
    element.width,
    element.shape,
    element.arrowhead,
  ), () => shapePath(element, hand));

  void keepOnly(Iterable<String> ids) {
    final kept = ids.toSet();
    _points.removeWhere((id, _) => !kept.contains(id));
    _bounds.removeWhere((id, _) => !kept.contains(id));
    _outlines.removeWhere((id, _) => !kept.contains(id));
    _shapes.removeWhere((id, _) => !kept.contains(id));
  }
}

bool isStored(BridgeElement element) =>
    element.id != null &&
    !element.id!.startsWith(pendingPrefix) &&
    !element.id!.startsWith(localPrefix);

const pendingPrefix = 'pending-';
const localPrefix = 'local-';

Path shapePath(BridgeElement element, bool hand) {
  final (w, h) = (element.w, element.h);
  final options = RoughOptions(seed: (element.seed - 1) % 0x7FFFFFFF + 1);
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

Offset strokePoint(BridgeElement element, BoardCache cache, Offset drawn) {
  final source = cache.bounds(element);
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

bool touches(BridgeElement element, BoardCache cache, Offset at, double slop) {
  if (!boundsOf(element).inflate(slop).contains(at)) return false;
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
  int docPage = 0,
}) => BridgeElement(
  kind: kind,
  x: frame.left,
  y: frame.top,
  w: frame.width,
  h: frame.height,
  rotation: rotation,
  color: color,
  width: width,
  docPage: docPage,
  pen: pen,
  points: points ?? Uint8List(0),
  shape: shape,
  seed: math.Random().nextInt(1 << 31),
  arrowhead: arrowhead,
  corners: Float64List.fromList(corners),
  text: text,
);

BridgeElement copyElement(
  BridgeElement e, {
  String? id,
  double? x,
  double? y,
  double? w,
  double? h,
  double? rotation,
  BridgeInkColor? color,
  int? width,
  int? docPage,
  bool? arrowhead,
  String? text,
}) => BridgeElement(
  id: id,
  kind: e.kind,
  x: x ?? e.x,
  y: y ?? e.y,
  w: w ?? e.w,
  h: h ?? e.h,
  rotation: rotation ?? e.rotation,
  color: color ?? e.color,
  width: width ?? e.width,
  docPage: docPage ?? e.docPage,
  pen: e.pen,
  points: e.points,
  shape: e.shape,
  seed: e.seed,
  arrowhead: arrowhead ?? e.arrowhead,
  corners: e.corners,
  text: text ?? e.text,
);

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
