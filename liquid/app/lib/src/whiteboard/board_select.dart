part of 'board_surface.dart';

extension _BoardSelect on BoardSurfaceState {
  void _endLasso() {
    final loop = _draft.loop;
    if (loop.length < 3 || _loopLength(loop) < 6 / board.zoom) {
      final at = loop.isEmpty ? null : loop.first;
      final hit = at == null
          ? null
          : board.elements.reversed
                .where(
                  (element) =>
                      isStored(element) &&
                      touches(element, board.cache, at, 4 / board.zoom),
                )
                .firstOrNull;
      board.select({?hit?.id});
      return;
    }
    board.select({
      for (final element in board.elements)
        if (isStored(element) &&
            lassoSelects(loop, samplesOf(element, board.cache)))
          element.id!,
    });
  }

  static double _loopLength(List<Offset> loop) {
    var length = 0.0;
    for (var at = 1; at < loop.length; at++) {
      length += (loop[at] - loop[at - 1]).distance;
    }
    return length;
  }

  void _show(Map<String, BridgeElement> moved) {
    board.elements = [
      for (final element in board.elements) moved[element.id] ?? element,
    ];
    board.notifyListeners();
  }

  void _transform(BridgeElement Function(BridgeElement) change) =>
      _show(_before.map((id, element) => MapEntry(id, change(element))));

  void _moveSelection(Offset from, Offset to) {
    final by = to - from;
    _transform(
      (element) => _with(element, frameOf(element).shift(by), element.rotation),
    );
  }

  void _resize(Offset to) {
    final (corners, rotation) = _boxBefore!;
    final fixed = corners[(_corner + 2) % 4];
    if (_before.length == 1) {
      final element = _before.values.single;
      final center = Offset.lerp(corners[0], corners[2], 0.5)!;
      final next = Rect.fromPoints(
        _rotateAround(fixed, center, -rotation),
        _rotateAround(to, center, -rotation),
      );
      final middle = _rotateAround(next.center, center, rotation);
      final w = next.width * (element.w < 0 ? -1 : 1);
      final h = next.height * (element.h < 0 ? -1 : 1);
      _show({
        element.id!: _with(
          element,
          Rect.fromLTWH(middle.dx - w / 2, middle.dy - h / 2, w, h),
          element.rotation,
        ),
      });
      return;
    }
    final dragged = corners[_corner];
    double scale(double to, double from, double anchor) =>
        from == anchor ? 1 : math.max(0.05, (to - anchor) / (from - anchor));
    final sx = scale(to.dx, dragged.dx, fixed.dx);
    final sy = scale(to.dy, dragged.dy, fixed.dy);
    _transform(
      (element) => _with(
        element,
        Rect.fromLTWH(
          fixed.dx + (element.x - fixed.dx) * sx,
          fixed.dy + (element.y - fixed.dy) * sy,
          element.w * sx,
          element.h * sy,
        ),
        element.rotation,
      ),
    );
  }

  void _rotate(Offset from, Offset to) {
    final (corners, _) = _boxBefore!;
    final center = Offset.lerp(corners[0], corners[2], 0.5)!;
    var angle = (to - center).direction - (from - center).direction;
    if (HardwareKeyboard.instance.isShiftPressed) {
      angle = (angle / (math.pi / 12)).round() * (math.pi / 12);
    }
    _transform((element) {
      final frame = frameOf(element);
      final moved = _rotateAround(frame.center, center, angle);
      return _with(
        element,
        frame.shift(moved - frame.center),
        element.rotation + angle,
      );
    });
  }

  static Offset _rotateAround(Offset p, Offset center, double angle) {
    final (sin, cos) = (math.sin(angle), math.cos(angle));
    final d = p - center;
    return center + Offset(d.dx * cos - d.dy * sin, d.dx * sin + d.dy * cos);
  }

  static BridgeElement _with(
    BridgeElement element,
    Rect frame,
    double rotation,
  ) => copyElement(
    element,
    id: element.id,
    x: frame.left,
    y: frame.top,
    w: frame.width,
    h: frame.height,
    rotation: rotation,
  );

  void _endTransform() {
    final changes = [
      for (final element in board.selection)
        if (_before[element.id] case final before?
            when before.x != element.x ||
                before.y != element.y ||
                before.w != element.w ||
                before.h != element.h ||
                before.rotation != element.rotation)
          changeOf(element),
    ];
    _before = const {};
    _boxBefore = null;
    unawaited(board.edit(change: changes));
  }

  bool _within((List<Offset>, double) box, Offset at) {
    final (corners, rotation) = box;
    final center = Offset.lerp(corners[0], corners[2], 0.5)!;
    final frame = Rect.fromPoints(
      _rotateAround(corners[0], center, -rotation),
      _rotateAround(corners[2], center, -rotation),
    );
    return frame
        .inflate(4 / board.zoom)
        .contains(_rotateAround(at, center, -rotation));
  }

  bool _grab(Offset view, Offset at) {
    final box = selectionBox(board);
    if (box != null) {
      final corners = [for (final corner in box.$1) board.toView(corner)];
      final (_, knob) = rotateHandle(corners);
      _before = {for (final element in board.selection) element.id!: element};
      _boxBefore = box;
      if ((view - knob).distance <= 9) {
        _drag = _Drag.rotate;
        return true;
      }
      for (final (index, corner) in corners.indexed) {
        if ((view - corner).distance <= 8) {
          _drag = _Drag.resize;
          _corner = index;
          return true;
        }
      }
      if (_within(box, at)) {
        _drag = _Drag.move;
        return true;
      }
    }
    return false;
  }
}
