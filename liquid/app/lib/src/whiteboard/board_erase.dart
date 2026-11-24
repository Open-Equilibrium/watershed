part of 'board_surface.dart';

extension _BoardErase on BoardSurfaceState {
  double get _eraserRadius =>
      const [6.0, 12.0, 24.0][tools.widthOf(BoardTool.eraser).clamp(0, 2)] /
      board.zoom;

  bool _sweep(Offset at) {
    final radius = _eraserRadius;
    final kept = <BridgeElement>[];
    var changed = false;
    for (final element in board.elements) {
      if (element.id?.startsWith(pendingPrefix) ?? false) {
        kept.add(element);
        continue;
      }
      if ((tools.highlighterOnly && !isHighlight(element)) ||
          !touches(element, board.cache, at, radius)) {
        kept.add(element);
        continue;
      }
      changed = true;
      final partial =
          !tools.wholeEraser && element.kind == BridgeElementKind.stroke;
      if (!partial) {
        _taken(element);
        continue;
      }
      final source = board.cache.bounds(element);
      final sx = source.width == 0 ? 1.0 : element.w / source.width;
      final sy = source.height == 0 ? 1.0 : element.h / source.height;
      final local = unturned(element, at);
      final drawn = Offset(
        source.left + (local.dx - element.x) / sx,
        source.top + (local.dy - element.y) / sy,
      );
      final scale = math.max(sx.abs(), sy.abs());
      final half = penSizes[element.width.clamp(0, 2)] / 2;
      final pieces = eraseInside(
        board.cache.points(element),
        drawn,
        radius / (scale == 0 ? 1 : scale),
        halfWidth: half,
      );
      _taken(element);
      for (final piece in pieces) {
        final bounds = pointsBounds(piece);
        final unturnedFrame = Rect.fromLTWH(
          element.x + (bounds.left - source.left) * sx,
          element.y + (bounds.top - source.top) * sy,
          bounds.width * sx,
          bounds.height * sy,
        );
        final made = newElement(
          BridgeElementKind.stroke,
          frame: Rect.fromCenter(
            center: turned(element, unturnedFrame.center),
            width: unturnedFrame.width,
            height: unturnedFrame.height,
          ),
          rotation: element.rotation,
          color: element.color,
          width: element.width,
          docPage: element.docPage,
          pen: element.pen,
          points: packPoints(piece),
        );
        final local = copyElement(
          made,
          id: '$localPrefix${_localIds++}',
          arrowhead: false,
          text: '',
        );
        kept.add(local);
        _pieces.putIfAbsent('', () => []).add(local);
      }
    }
    if (changed) board.elements = kept;
    return changed;
  }

  void _taken(BridgeElement element) {
    final id = element.id;
    if (id == null || id.startsWith(pendingPrefix)) return;
    if (id.startsWith(localPrefix)) {
      _pieces['']?.removeWhere((piece) => piece.id == id);
    } else {
      _erased.add(id);
    }
  }

  void _endErase() {
    final pieces = [
      for (final piece in _pieces[''] ?? const <BridgeElement>[])
        copyElement(piece, arrowhead: false, text: ''),
    ];
    final removed = [..._erased];
    _erased.clear();
    _pieces.clear();
    board.elements = [
      for (final element in board.elements)
        if (!(element.id?.startsWith(localPrefix) ?? false)) element,
    ];
    unawaited(board.edit(add: pieces, remove: removed));
  }
}
