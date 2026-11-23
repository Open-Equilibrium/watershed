part of 'board_surface.dart';

extension _BoardInk on BoardSurfaceState {
  void _startInk(Offset at, PointerDownEvent event) {
    _drag = _Drag.ink;
    _snapped = null;
    _ink
      ..clear()
      ..add(InkPoint(at.dx, at.dy, _pressure(event)));
    _armHold();
    _showInk();
  }

  BoardTool get _inkTool => switch (tools.tool) {
    BoardTool.shape || BoardTool.highlighter => tools.tool,
    _ => BoardTool.pen,
  };
  BridgeInkColor get _inkColor => tools.colorOf(_inkTool);
  int get _inkWidth => tools.widthOf(_inkTool);
  BridgePen get _inkPen => switch (tools.tool) {
    BoardTool.shape => BridgePen.ball,
    BoardTool.highlighter => BridgePen.highlighter,
    _ => tools.pen,
  };

  void _endInk() {
    final snapping = tools.tool == BoardTool.shape;
    if (board.paged && _ink.isNotEmpty) {
      final page = board.pageAt(_ink.first.offset);
      final sheet = page == null ? null : board.sheets[page];
      final inside = sheet == null
          ? 0
          : _ink.takeWhile((point) => sheet.contains(point.offset)).length;
      _ink.removeRange(inside, _ink.length);
    }
    if (tools.tool == BoardTool.highlighter && _ink.length > 2) {
      final (first, last) = (_ink.first, _ink.last);
      var length = 0.0;
      for (var at = 1; at < _ink.length; at++) {
        length += (_ink[at].offset - _ink[at - 1].offset).distance;
      }
      if ((last.offset - first.offset).distance >= 0.9 * length) {
        _ink
          ..clear()
          ..addAll([first, last]);
      }
    }
    final shape = _snapped ?? (snapping ? _shapeOf(_ink) : null);
    if (shape != null) {
      unawaited(board.edit(add: [shape]));
    } else if (_ink.isNotEmpty) {
      unawaited(
        board.edit(
          add: [
            for (final piece in splitInk(_ink))
              newElement(
                BridgeElementKind.stroke,
                frame: pointsBounds(piece),
                color: _inkColor,
                width: _inkWidth,
                pen: _inkPen,
                points: packPoints(piece),
              ),
          ],
        ),
      );
    }
    _ink.clear();
    _snapped = null;
  }

  BridgeElement? _shapeOf(List<InkPoint> ink) {
    final snapped = snapShape([for (final point in ink) point.offset]);
    if (snapped == null) return null;
    final color = _inkColor;
    final width = _inkWidth;
    switch (snapped.kind) {
      case SnapKind.line:
        final [a, b] = snapped.points;
        return newElement(
          BridgeElementKind.shape,
          frame: Rect.fromLTWH(a.dx, a.dy, b.dx - a.dx, b.dy - a.dy),
          color: color,
          width: width,
          shape: BridgeShape.line,
        );
      case SnapKind.rectangle || SnapKind.ellipse:
        return newElement(
          BridgeElementKind.shape,
          frame: Rect.fromCenter(
            center: snapped.center,
            width: snapped.width,
            height: snapped.height,
          ),
          rotation: snapped.rotation,
          color: color,
          width: width,
          shape: snapped.kind == SnapKind.rectangle
              ? BridgeShape.rectangle
              : BridgeShape.ellipse,
        );
      case SnapKind.triangle:
        var box = Rect.fromPoints(snapped.points[0], snapped.points[1]);
        box = box.expandToInclude(
          Rect.fromCenter(center: snapped.points[2], width: 0, height: 0),
        );
        if (box.width == 0 || box.height == 0) return null;
        return newElement(
          BridgeElementKind.shape,
          frame: box,
          color: color,
          width: width,
          shape: BridgeShape.triangle,
          corners: [
            for (final corner in snapped.points) ...[
              (corner.dx - box.left) / box.width,
              (corner.dy - box.top) / box.height,
            ],
          ],
        );
    }
  }

  BridgeElement _dragShape(Offset from, Offset to) {
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final kind = tools.shape;
    var end = to;
    final line = kind == ShapeKind.line || kind == ShapeKind.arrow;
    if (shift && line) {
      final d = to - from;
      final step = math.pi / 12;
      final angle = (d.direction / step).round() * step;
      end = from + Offset.fromDirection(angle, d.distance);
    } else if (shift) {
      final d = to - from;
      final side = math.max(d.dx.abs(), d.dy.abs());
      end = from + Offset(side * d.dx.sign, side * d.dy.sign);
    }
    final frame = line
        ? Rect.fromLTWH(from.dx, from.dy, end.dx - from.dx, end.dy - from.dy)
        : Rect.fromPoints(from, end);
    return newElement(
      BridgeElementKind.shape,
      frame: frame,
      color: tools.colorOf(BoardTool.shape),
      width: tools.widthOf(BoardTool.shape),
      shape: switch (kind) {
        ShapeKind.ellipse => BridgeShape.ellipse,
        ShapeKind.line || ShapeKind.arrow => BridgeShape.line,
        _ => BridgeShape.rectangle,
      },
      arrowhead: kind == ShapeKind.arrow,
    );
  }
}
