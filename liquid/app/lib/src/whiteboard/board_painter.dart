import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../rust/api/drawing.dart';
import '../theme/tokens.g.dart';
import 'board.dart';
import 'board_controller.dart';
import 'ink_geometry.dart';

/// What is being drawn and not yet written: a pen stroke, a shape, a lasso loop or the
/// eraser's circle, in board points.
class Draft {
  const Draft({
    this.ink = const [],
    this.pen = BridgePen.fountain,
    this.shape,
    this.loop = const [],
    this.eraser,
    this.eraserRadius = 0,
    this.color = BridgeInkColor.text,
    this.width = 1,
  });

  final List<InkPoint> ink;
  final BridgePen pen;
  final BridgeElement? shape;
  final List<Offset> loop;
  final Offset? eraser;
  final double eraserRadius;
  final BridgeInkColor color;
  final int width;

  static const none = Draft();
}

/// Draws a Whiteboard: only the elements in view (client architecture rule 8), then the
/// draft, then the selection's frame and handles.
class BoardPainter extends CustomPainter {
  BoardPainter({
    required this.board,
    required this.ws,
    required this.draft,
    required this.hidden,
    required this.editing,
    this.dots = false,
    this.pixelRatio = 1,
  }) : super(repaint: board);

  final BoardController board;
  final WsPalette ws;
  final Draft draft;

  /// Elements the eraser took and not yet written away, or the text being edited.
  final Set<String> hidden;
  final String? editing;
  final bool dots;

  /// The screen's device pixels per logical pixel, to draw a document's pages sharp.
  final double pixelRatio;

  /// Whether the ink lies on a document's paper rather than a board's canvas.
  bool get paper => board.paged;

  /// The handles' size on screen.
  static const handle = 8.0;

  /// How far above the selection its rotation handle sits, on screen.
  static const rotateReach = 20.0;

  @override
  void paint(Canvas canvas, Size size) {
    final zoom = board.zoom;
    final view = Rect.fromLTWH(
      board.origin.dx,
      board.origin.dy,
      size.width / zoom,
      size.height / zoom,
    );
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    if (dots) _dots(canvas, size);
    canvas.scale(zoom);
    canvas.translate(-board.origin.dx, -board.origin.dy);
    board.pages?.paint(canvas, view, board.zoom * pixelRatio);
    // The highlighter's marks lie beneath the rest of the ink.
    for (final beneath in [true, false]) {
      for (final element in board.elements) {
        if (isHighlight(element) != beneath) continue;
        if (hidden.contains(element.id) || element.id == editing) continue;
        if (!boundsOf(element).overlaps(view)) continue;
        paintElement(canvas, element);
      }
    }
    _draft(canvas);
    canvas.restore();
    _selection(canvas);
  }

  void _dots(Canvas canvas, Size size) {
    const gap = 20.0;
    final step = gap * board.zoom;
    if (step < 8) return;
    final paint = Paint()..color = ws.border;
    final start = Offset(
      -(board.origin.dx * board.zoom) % step,
      -(board.origin.dy * board.zoom) % step,
    );
    for (var x = start.dx; x < size.width; x += step) {
      for (var y = start.dy; y < size.height; y += step) {
        canvas.drawCircle(Offset(x, y), 1, paint);
      }
    }
  }

  /// Draws one element in board points.
  void paintElement(Canvas canvas, BridgeElement element) {
    final color = inkColor(ws, element.color);
    canvas.save();
    if (element.rotation != 0) {
      final center = frameOf(element).center;
      canvas
        ..translate(center.dx, center.dy)
        ..rotate(element.rotation)
        ..translate(-center.dx, -center.dy);
    }
    switch (element.kind) {
      case BridgeElementKind.stroke:
        final source = board.cache.bounds(element);
        if (source.isFinite) {
          final sx = source.width == 0 ? 1.0 : element.w / source.width;
          final sy = source.height == 0 ? 1.0 : element.h / source.height;
          canvas
            ..translate(element.x, element.y)
            ..scale(sx, sy)
            ..translate(-source.left, -source.top)
            ..drawPath(
              board.cache.outline(element),
              _inkPaint(color, highlight: isHighlight(element)),
            );
        }
      case BridgeElementKind.shape:
        canvas
          ..translate(element.x, element.y)
          ..drawPath(
            board.cache.shape(element, board.handDrawn),
            Paint()
              ..color = color
              ..style = PaintingStyle.stroke
              ..strokeWidth = shapeWidths[element.width.clamp(0, 2)]
              ..strokeCap = StrokeCap.round
              ..strokeJoin = StrokeJoin.round,
          );
      case BridgeElementKind.text:
        final painter = textPainter(element, color)
          ..layout(maxWidth: math.max(1, element.w))
          ..paint(canvas, Offset(element.x, element.y));
        painter.dispose();
    }
    canvas.restore();
  }

  /// Ink's paint; the highlighter's is multiplied onto paper, so text shows through it,
  /// and translucent on a board's canvas.
  Paint _inkPaint(Color color, {required bool highlight}) {
    final paint = Paint()..color = color;
    if (!highlight) return paint;
    return paper
        ? (paint..blendMode = BlendMode.multiply)
        : (paint..color = color.withValues(alpha: 0.55));
  }

  TextPainter textPainter(BridgeElement element, Color color) => TextPainter(
    text: TextSpan(
      text: element.text,
      style: textStyle(element.width, color, board.handDrawn),
    ),
    textDirection: TextDirection.ltr,
  );

  static TextStyle textStyle(int size, Color color, bool hand) => TextStyle(
    fontSize: textSizes[size.clamp(0, 2)],
    height: 1.25,
    color: color,
    // Clean text takes Liquid's typeface, also where no theme is around it.
    fontFamily: hand ? handFont : WsMetrics.fontUi.first,
    fontFamilyFallback: hand
        ? WsMetrics.fontUi
        : WsMetrics.fontUi.skip(1).toList(),
  );

  void _draft(Canvas canvas) {
    if (draft.ink.isNotEmpty) {
      canvas.drawPath(
        inkOutline(
          draft.ink,
          penOptions(draft.pen, draft.width, complete: false),
        ),
        _inkPaint(
          inkColor(ws, draft.color),
          highlight: draft.pen == BridgePen.highlighter,
        ),
      );
    }
    if (draft.shape case final shape?) paintElement(canvas, shape);
    if (draft.loop.length > 1) {
      final path = Path()..addPolygon(draft.loop, false);
      canvas.drawPath(
        _dashed(path, 5 / board.zoom, 4 / board.zoom),
        Paint()
          ..color = ws.textMuted
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.4 / board.zoom,
      );
    }
    if (draft.eraser case final at?) {
      canvas.drawCircle(
        at,
        draft.eraserRadius,
        Paint()
          ..color = ws.textMuted
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2 / board.zoom,
      );
    }
  }

  static Path _dashed(Path path, double dash, double gap) {
    final dashed = Path();
    for (final metric in path.computeMetrics()) {
      for (var at = 0.0; at < metric.length; at += dash + gap) {
        dashed.addPath(metric.extractPath(at, at + dash), Offset.zero);
      }
    }
    return dashed;
  }

  /// The selection's frame, corner handles and rotation handle, on screen.
  void _selection(Canvas canvas) {
    final box = selectionBox(board);
    if (box == null) return;
    final (corners, rotation) = box;
    final view = [for (final corner in corners) board.toView(corner)];
    final line = Paint()
      ..color = ws.accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.2;
    canvas.drawPath(Path()..addPolygon(view, true), line);
    final fill = Paint()..color = ws.canvas;
    for (final corner in view) {
      final square = RRect.fromRectAndRadius(
        Rect.fromCenter(center: corner, width: handle, height: handle),
        const Radius.circular(1.5),
      );
      canvas
        ..drawRRect(square, fill)
        ..drawRRect(square, line);
    }
    final (top, knob) = rotateHandle(view);
    canvas
      ..drawLine(top, knob, line)
      ..drawCircle(knob, 4.5, fill)
      ..drawCircle(knob, 4.5, line);
  }

  @override
  bool shouldRepaint(BoardPainter old) =>
      old.draft != draft ||
      old.hidden != hidden ||
      old.editing != editing ||
      old.ws != ws ||
      old.dots != dots ||
      old.pixelRatio != pixelRatio;
}

/// The selection's frame on the board, as its four corners clockwise from the top left,
/// and its rotation: one element's own turned frame, or several elements' common box.
(List<Offset>, double)? selectionBox(BoardController board) {
  final selection = board.selection;
  if (selection.isEmpty) return null;
  if (selection.length == 1) {
    final element = selection.single;
    final frame = frameOf(element);
    // A line's frame may run backwards; its box is the same.
    final box = Rect.fromPoints(frame.topLeft, frame.bottomRight);
    final turnedCorners = [
      for (final corner in [
        box.topLeft,
        box.topRight,
        box.bottomRight,
        box.bottomLeft,
      ])
        turned(element, corner),
    ];
    return (turnedCorners, element.rotation);
  }
  var bounds = Rect.fromPoints(
    corners(selection.first)[0],
    corners(selection.first)[2],
  );
  for (final element in selection) {
    for (final corner in corners(element)) {
      bounds = bounds.expandToInclude(
        Rect.fromCenter(center: corner, width: 0, height: 0),
      );
    }
  }
  return (
    [bounds.topLeft, bounds.topRight, bounds.bottomRight, bounds.bottomLeft],
    0.0,
  );
}

/// Where the rotation handle's line starts and its knob sits, on screen.
(Offset, Offset) rotateHandle(List<Offset> view) {
  final top = Offset.lerp(view[0], view[1], 0.5)!;
  final bottom = Offset.lerp(view[3], view[2], 0.5)!;
  final up = top - bottom;
  final unit = up.distance == 0 ? const Offset(0, -1) : up / up.distance;
  return (top, top + unit * BoardPainter.rotateReach);
}
