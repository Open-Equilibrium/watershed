import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/whiteboard.dart';
import '../theme/theme.dart';
import 'board.dart';
import 'board_controller.dart';
import 'board_painter.dart';
import 'ink_geometry.dart';
import 'shape_snap.dart';

/// A Whiteboard's canvas (ADR-0306): it draws the board and, while focused, takes the
/// tools' input as GoodNotes does. Trackpad and touch gestures and the right mouse button
/// pan and zoom it focused or not; the mouse wheel acts on it only while it is focused,
/// and otherwise scrolls the Page.
class BoardSurface extends StatefulWidget {
  const BoardSurface({
    super.key,
    required this.board,
    required this.focused,
    required this.onFocus,
    required this.onLeave,
  });

  final BoardController board;

  /// Whether the board takes the tools' input.
  final bool focused;

  /// A click into the unfocused board focuses it.
  final VoidCallback onFocus;

  /// esc with nothing selected leaves the board.
  final VoidCallback onLeave;

  @override
  State<BoardSurface> createState() => BoardSurfaceState();
}

enum _Drag { none, pan, ink, shape, erase, loop, move, resize, rotate }

class BoardSurfaceState extends State<BoardSurface> {
  BoardController get board => widget.board;
  BoardTools get tools => boardTools;

  var _draft = Draft.none;
  var _drag = _Drag.none;
  int? _pointer;
  var _rightDown = false;
  var _spaceDown = false;
  Offset _start = Offset.zero;
  Offset _last = Offset.zero;

  /// The ink drawn so far, in board points, and whether it snapped into a shape.
  final _ink = <InkPoint>[];
  BridgeElement? _snapped;
  Timer? _hold;

  /// The elements as a move, resize or rotation found them.
  Map<String, BridgeElement> _before = const {};
  (List<Offset>, double)? _boxBefore;
  int _corner = 0;

  /// Elements the eraser took, and the strokes it cut, as new pieces by the stroke.
  final _erased = <String>{};
  final _pieces = <String, List<BridgeElement>>{};

  /// The text box being written: its element, new or stored, and its editor.
  BridgeElement? _text;
  final _typed = TextEditingController();
  final _typing = FocusNode(debugLabel: 'Whiteboard text');

  Size _size = Size.zero;

  @override
  void initState() {
    super.initState();
    tools.addListener(_toolsChanged);
    _typing.addListener(() {
      if (!_typing.hasFocus) _endText();
    });
  }

  void _toolsChanged() {
    if (tools.tool != BoardTool.lasso && board.selected.isNotEmpty) {
      board.select(const {});
    }
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    tools.removeListener(_toolsChanged);
    _hold?.cancel();
    _typed.dispose();
    _typing.dispose();
    super.dispose();
  }

  // Pointers.

  void _down(PointerDownEvent event) {
    if (_pointer != null) return;
    _start = _last = event.localPosition;
    if (event.buttons & kSecondaryMouseButton != 0) {
      _rightDown = true;
      _pointer = event.pointer;
      _drag = _Drag.pan;
      return;
    }
    if (!widget.focused) {
      widget.onFocus();
      return;
    }
    if (_text != null) {
      _endText();
      return;
    }
    _pointer = event.pointer;
    if (_spaceDown) {
      _drag = _Drag.pan;
      return;
    }
    final at = board.toBoard(event.localPosition);
    switch (tools.tool) {
      case BoardTool.pen:
        _startInk(at, event);
      case BoardTool.shape when tools.shape == ShapeKind.snap:
        _startInk(at, event);
      case BoardTool.shape:
        _drag = _Drag.shape;
      case BoardTool.eraser:
        _drag = _Drag.erase;
        _erase(at);
      case BoardTool.lasso:
        _startLasso(event.localPosition, at);
      case BoardTool.text:
        _pointer = null;
        _startText(at);
    }
  }

  void _move(PointerMoveEvent event) {
    if (event.pointer != _pointer) return;
    final at = board.toBoard(event.localPosition);
    final by = event.localPosition - _last;
    _last = event.localPosition;
    switch (_drag) {
      case _Drag.pan:
        board.pan(by);
      case _Drag.ink:
        if (_snapped != null) return;
        if ((event.localPosition - _start).distance > 3) _armHold();
        _ink.add(InkPoint(at.dx, at.dy, _pressure(event)));
        _showInk();
      case _Drag.shape:
        _showDragged(board.toBoard(_start), at);
      case _Drag.erase:
        // Along the whole way since the last event, so a quick stroke misses nothing.
        final from = board.toBoard(event.localPosition - by);
        final steps = math.max(
          1,
          ((at - from).distance / (_eraserRadius / 2)).ceil(),
        );
        for (var step = 1; step <= steps; step++) {
          _erase(Offset.lerp(from, at, step / steps)!);
        }
      case _Drag.loop:
        setState(() => _draft = Draft(loop: [..._draft.loop, at]));
      case _Drag.move:
        _moveSelection(board.toBoard(_start), at);
      case _Drag.resize:
        _resize(at);
      case _Drag.rotate:
        _rotate(board.toBoard(_start), at);
      case _Drag.none:
        break;
    }
  }

  void _up(PointerEvent event) {
    if (event.pointer != _pointer) return;
    _pointer = null;
    _hold?.cancel();
    final drag = _drag;
    _drag = _Drag.none;
    if (event.buttons & kSecondaryMouseButton == 0) _rightDown = false;
    switch (drag) {
      case _Drag.ink:
        _endInk();
      case _Drag.shape:
        if (_draft.shape case final shape?) unawaited(board.edit(add: [shape]));
      case _Drag.erase:
        _endErase();
      case _Drag.loop:
        _endLasso();
      case _Drag.move || _Drag.resize || _Drag.rotate:
        _endTransform();
      case _Drag.pan || _Drag.none:
        break;
    }
    setState(() => _draft = Draft.none);
  }

  double _pressure(PointerEvent event) =>
      event.kind == PointerDeviceKind.stylus ||
          event.kind == PointerDeviceKind.invertedStylus
      ? event.pressure.clamp(0.05, 1.0)
      : 0.5;

  /// The mouse wheel: with the right button held or ⌘ it zooms; on a focused board it
  /// pans; elsewhere it scrolls the Page.
  void _signal(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final zooms =
        _rightDown ||
        (widget.focused && HardwareKeyboard.instance.isMetaPressed);
    if (!zooms && !widget.focused) return;
    GestureBinding.instance.pointerSignalResolver.register(event, (event) {
      final scroll = event as PointerScrollEvent;
      if (zooms) {
        board.zoomBy(
          math.exp(-scroll.scrollDelta.dy / 300),
          scroll.localPosition,
        );
      } else {
        board.pan(-scroll.scrollDelta);
      }
    });
  }

  // The pen and the shape tool's snapping.

  void _startInk(Offset at, PointerDownEvent event) {
    _drag = _Drag.ink;
    _snapped = null;
    _ink
      ..clear()
      ..add(InkPoint(at.dx, at.dy, _pressure(event)));
    _armHold();
    _showInk();
  }

  /// Draw and hold: a stroke held still for a moment snaps into a shape (GoodNotes).
  void _armHold() {
    _hold?.cancel();
    _hold = Timer(const Duration(milliseconds: 500), () {
      if (_drag != _Drag.ink || _ink.length < 3) return;
      final shape = _shapeOf(_ink);
      if (shape == null) return;
      setState(() {
        _snapped = shape;
        _draft = Draft(shape: shape);
      });
    });
  }

  BridgeInkColor get _inkColor => tools.colorOf(
    tools.tool == BoardTool.shape ? BoardTool.shape : BoardTool.pen,
  );
  int get _inkWidth => tools.widthOf(
    tools.tool == BoardTool.shape ? BoardTool.shape : BoardTool.pen,
  );

  void _showInk() => setState(
    () => _draft = Draft(
      ink: [..._ink],
      pen: tools.tool == BoardTool.shape ? BridgePen.ball : tools.pen,
      color: _inkColor,
      width: _inkWidth,
    ),
  );

  void _endInk() {
    final snapping = tools.tool == BoardTool.shape;
    final shape = _snapped ?? (snapping ? _shapeOf(_ink) : null);
    if (shape != null) {
      unawaited(board.edit(add: [shape]));
    } else if (_ink.isNotEmpty) {
      unawaited(
        board.edit(
          add: [
            newElement(
              BridgeElementKind.stroke,
              frame: pointsBounds(_ink),
              color: _inkColor,
              width: _inkWidth,
              pen: snapping ? BridgePen.ball : tools.pen,
              points: packPoints(_ink),
            ),
          ],
        ),
      );
    }
    _ink.clear();
    _snapped = null;
  }

  /// The perfect shape `ink` snaps into, if any.
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

  /// A shape dragged out from `from` to `to`; ⇧ keeps a square, a circle or 15° steps.
  void _showDragged(Offset from, Offset to) {
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
    setState(
      () => _draft = Draft(
        shape: newElement(
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
        ),
      ),
    );
  }

  // The eraser.

  double get _eraserRadius =>
      const [6.0, 12.0, 24.0][tools.widthOf(BoardTool.eraser).clamp(0, 2)] /
      board.zoom;

  /// Erases at `at`: whole elements it touches, or, partly, the part of strokes under it.
  void _erase(Offset at) {
    final radius = _eraserRadius;
    final kept = <BridgeElement>[];
    var changed = false;
    for (final element in board.elements) {
      // An element still on its way is erased once it is stored.
      if (element.id?.startsWith(pendingPrefix) ?? false) {
        kept.add(element);
        continue;
      }
      if (!touches(element, board.cache, at, radius)) {
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
      // The stroke's points on the board, cut where the eraser passes.
      final points = [
        for (final point in board.cache.points(element))
          () {
            final p = strokePoint(element, board.cache, point.offset);
            return InkPoint(p.dx, p.dy, point.pressure);
          }(),
      ];
      final half = penSizes[element.width.clamp(0, 2)] / 2;
      final pieces = eraseInside(points, at, radius, halfWidth: half);
      _taken(element);
      for (final piece in pieces) {
        final made = newElement(
          BridgeElementKind.stroke,
          frame: pointsBounds(piece),
          color: element.color,
          width: element.width,
          pen: element.pen,
          points: packPoints(piece),
        );
        // A local name, so the next pass cuts the piece again.
        final local = BridgeElement(
          id: '$localPrefix${_localIds++}',
          kind: made.kind,
          x: made.x,
          y: made.y,
          w: made.w,
          h: made.h,
          rotation: 0,
          color: made.color,
          width: made.width,
          pen: made.pen,
          points: made.points,
          shape: made.shape,
          seed: made.seed,
          arrowhead: false,
          corners: made.corners,
          text: '',
        );
        kept.add(local);
        _pieces.putIfAbsent('', () => []).add(local);
      }
    }
    if (changed) board.elements = kept;
    setState(() => _draft = Draft(eraser: at, eraserRadius: radius));
    if (changed) board.notifyListeners();
  }

  int _localIds = 0;

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
        BridgeElement(
          kind: piece.kind,
          x: piece.x,
          y: piece.y,
          w: piece.w,
          h: piece.h,
          rotation: 0,
          color: piece.color,
          width: piece.width,
          pen: piece.pen,
          points: piece.points,
          shape: piece.shape,
          seed: piece.seed,
          arrowhead: false,
          corners: piece.corners,
          text: '',
        ),
    ];
    final removed = [..._erased];
    _erased.clear();
    _pieces.clear();
    // The board as it was, so the edit removes and adds from what is stored.
    board.elements = [
      for (final element in board.elements)
        if (!(element.id?.startsWith(localPrefix) ?? false)) element,
    ];
    unawaited(board.edit(add: pieces, remove: removed));
  }

  // The lasso: select, move, resize, rotate.

  void _startLasso(Offset view, Offset at) {
    final box = selectionBox(board);
    if (box != null) {
      final corners = [for (final corner in box.$1) board.toView(corner)];
      final (_, knob) = rotateHandle(corners);
      _before = {for (final element in board.selection) element.id!: element};
      _boxBefore = box;
      if ((view - knob).distance <= 9) {
        _drag = _Drag.rotate;
        return;
      }
      for (final (index, corner) in corners.indexed) {
        if ((view - corner).distance <= 8) {
          _drag = _Drag.resize;
          _corner = index;
          return;
        }
      }
      if (lassoEncloses(box.$1, at)) {
        _drag = _Drag.move;
        return;
      }
    }
    _drag = _Drag.loop;
    setState(() => _draft = Draft(loop: [at]));
  }

  void _endLasso() {
    final loop = _draft.loop;
    if (loop.length < 3 || _loopLength(loop) < 6 / board.zoom) {
      // A click selects the topmost element under it, or nothing.
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

  void _moveSelection(Offset from, Offset to) {
    final by = to - from;
    _show({
      for (final MapEntry(key: id, value: element) in _before.entries)
        id: _with(element, frameOf(element).shift(by), element.rotation),
    });
  }

  void _resize(Offset to) {
    final (corners, rotation) = _boxBefore!;
    final fixed = corners[(_corner + 2) % 4];
    if (_before.length == 1) {
      // In the element's unturned frame; the new box's center, turned back, keeps the
      // fixed corner where it was.
      final element = _before.values.single;
      final center = Offset.lerp(corners[0], corners[2], 0.5)!;
      final next = Rect.fromPoints(
        _rotateAround(fixed, center, -rotation),
        _rotateAround(to, center, -rotation),
      );
      final middle = _rotateAround(next.center, center, rotation);
      // A line keeps its direction.
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
    // Several elements scale with their common box, away from its fixed corner.
    final dragged = corners[_corner];
    double scale(double to, double from, double anchor) =>
        from == anchor ? 1 : math.max(0.05, (to - anchor) / (from - anchor));
    final sx = scale(to.dx, dragged.dx, fixed.dx);
    final sy = scale(to.dy, dragged.dy, fixed.dy);
    _show({
      for (final MapEntry(key: id, value: element) in _before.entries)
        id: _with(
          element,
          Rect.fromLTWH(
            fixed.dx + (element.x - fixed.dx) * sx,
            fixed.dy + (element.y - fixed.dy) * sy,
            element.w * sx,
            element.h * sy,
          ),
          element.rotation,
        ),
    });
  }

  void _rotate(Offset from, Offset to) {
    final (corners, _) = _boxBefore!;
    final center = Offset.lerp(corners[0], corners[2], 0.5)!;
    var angle = (to - center).direction - (from - center).direction;
    if (HardwareKeyboard.instance.isShiftPressed) {
      angle = (angle / (math.pi / 12)).round() * (math.pi / 12);
    }
    _show({
      for (final MapEntry(key: id, value: element) in _before.entries)
        id: () {
          final frame = frameOf(element);
          final moved = _rotateAround(frame.center, center, angle);
          return _with(
            element,
            frame.shift(moved - frame.center),
            element.rotation + angle,
          );
        }(),
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
  ) => BridgeElement(
    id: element.id,
    kind: element.kind,
    x: frame.left,
    y: frame.top,
    w: frame.width,
    h: frame.height,
    rotation: rotation,
    color: element.color,
    width: element.width,
    pen: element.pen,
    points: element.points,
    shape: element.shape,
    seed: element.seed,
    arrowhead: element.arrowhead,
    corners: element.corners,
    text: element.text,
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

  // Text.

  void _startText(Offset at) {
    final hit = board.elements.reversed
        .where(
          (element) =>
              isStored(element) &&
              element.kind == BridgeElementKind.text &&
              touches(element, board.cache, at, 2 / board.zoom),
        )
        .firstOrNull;
    final element =
        hit ??
        newElement(
          BridgeElementKind.text,
          frame: Rect.fromLTWH(at.dx, at.dy - 12, textWidth, 30),
          color: tools.colorOf(BoardTool.text),
          width: tools.widthOf(BoardTool.text),
        );
    setState(() {
      _text = element;
      _typed.text = element.text;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _typing.requestFocus();
    });
  }

  void _endText() {
    final element = _text;
    if (element == null) return;
    setState(() => _text = null);
    final text = _typed.text.trimRight();
    final height = (TextPainter(
      text: TextSpan(
        text: text,
        style: BoardPainter.textStyle(
          element.width,
          const Color(0xFF000000),
          board.handDrawn,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: math.max(1, element.w))).height;
    final frame = Rect.fromLTWH(element.x, element.y, element.w, height);
    if (element.id == null) {
      if (text.isEmpty) return;
      unawaited(board.edit(add: [_withText(element, frame, text)]));
    } else if (text.isEmpty) {
      unawaited(board.edit(remove: [element.id!]));
    } else if (text != element.text || frame.height != element.h) {
      unawaited(
        board.edit(
          change: [changeOf(element, frame: frame, text: text)],
        ),
      );
    }
  }

  static BridgeElement _withText(
    BridgeElement element,
    Rect frame,
    String text,
  ) => BridgeElement(
    kind: element.kind,
    x: frame.left,
    y: frame.top,
    w: frame.width,
    h: frame.height,
    rotation: element.rotation,
    color: element.color,
    width: element.width,
    pen: element.pen,
    points: element.points,
    shape: element.shape,
    seed: element.seed,
    arrowhead: false,
    corners: element.corners,
    text: text,
  );

  // Keys.

  /// The board's keys while it is focused (approved Whiteboard Block mockups, section 9).
  KeyEventResult key(KeyEvent event) {
    if (_text != null) return KeyEventResult.ignored;
    if (event.logicalKey == LogicalKeyboardKey.space) {
      _spaceDown = event is! KeyUpEvent;
      return KeyEventResult.handled;
    }
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    final meta = keys.isMetaPressed;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.escape:
        if (board.selected.isNotEmpty) {
          board.select(const {});
        } else {
          widget.onLeave();
        }
      case LogicalKeyboardKey.backspace || LogicalKeyboardKey.delete
          when board.selected.isNotEmpty:
        unawaited(board.edit(remove: [...board.selected]));
      case LogicalKeyboardKey.keyA when meta:
        tools.tool = BoardTool.lasso;
        board.select({
          for (final element in board.elements)
            if (isStored(element)) element.id!,
        });
      case LogicalKeyboardKey.digit0 when meta:
        board.zoomTo(1, _size);
      case LogicalKeyboardKey.digit1 when keys.isShiftPressed:
        board.fit(_size);
      case _ when meta || keys.isControlPressed || keys.isAltPressed:
        return KeyEventResult.ignored;
      case LogicalKeyboardKey.keyL:
        tools.tool = BoardTool.lasso;
      case LogicalKeyboardKey.keyP:
        tools.tool = BoardTool.pen;
      case LogicalKeyboardKey.keyE:
        tools.tool = BoardTool.eraser;
      case LogicalKeyboardKey.keyT:
        tools.tool = BoardTool.text;
      case LogicalKeyboardKey.keyS:
        _shape(ShapeKind.snap);
      case LogicalKeyboardKey.keyR:
        _shape(ShapeKind.rectangle);
      case LogicalKeyboardKey.keyO:
        _shape(ShapeKind.ellipse);
      case LogicalKeyboardKey.keyI:
        _shape(ShapeKind.line);
      case LogicalKeyboardKey.keyA:
        _shape(ShapeKind.arrow);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  void _shape(ShapeKind kind) => tools.update(() {
    tools.shape = kind;
    tools.tool = BoardTool.shape;
  });

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final text = _text;
    return LayoutBuilder(
      builder: (context, constraints) {
        _size = constraints.biggest;
        return RawGestureDetector(
          gestures: {
            _Claim: GestureRecognizerFactoryWithHandlers<_Claim>(
              () => _Claim(),
              (claim) => claim
                ..claims = ((event) =>
                    widget.focused ||
                    event.buttons & kSecondaryMouseButton != 0)
                ..onPanZoom = (pan, scale, focal) {
                  board.pan(pan);
                  if (scale != 1) board.zoomBy(scale, focal);
                },
            ),
          },
          child: Listener(
            onPointerDown: _down,
            onPointerMove: _move,
            onPointerUp: _up,
            onPointerCancel: _up,
            onPointerSignal: _signal,
            child: MouseRegion(
              cursor: widget.focused && tools.tool != BoardTool.lasso
                  ? SystemMouseCursors.precise
                  : MouseCursor.defer,
              child: Stack(
                children: [
                  Positioned.fill(
                    child: CustomPaint(
                      key: const Key('whiteboard canvas'),
                      painter: BoardPainter(
                        board: board,
                        ws: ws,
                        draft: _draft,
                        hidden: const {},
                        editing: text?.id,
                        dots: widget.focused,
                      ),
                    ),
                  ),
                  if (text != null) _textEditor(context, text),
                  if (!board.loaded ||
                      board.elements.isNotEmpty ||
                      !widget.focused)
                    const SizedBox.shrink()
                  else
                    Positioned(
                      left: 12,
                      top: 10,
                      child: IgnorePointer(
                        child: Text(
                          'Draw with the pen, or choose a tool below',
                          style: TextStyle(fontSize: 12, color: ws.textFaint),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _textEditor(BuildContext context, BridgeElement element) {
    final at = board.toView(Offset(element.x, element.y));
    final style = BoardPainter.textStyle(
      element.width,
      inkColor(context.ws, element.color),
      board.handDrawn,
    );
    return Positioned(
      left: at.dx,
      top: at.dy,
      width: element.w * board.zoom,
      child: Transform.scale(
        scale: board.zoom,
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: element.w,
          child: TextField(
            key: const Key('whiteboard text'),
            controller: _typed,
            focusNode: _typing,
            maxLines: null,
            style: style,
            cursorColor: style.color,
            decoration: const InputDecoration.collapsed(hintText: null),
            onTapOutside: (_) => _typing.unfocus(),
            onEditingComplete: _typing.unfocus,
          ),
        ),
      ),
    );
  }
}

/// Claims a board's pointers before the Page's scrolling can: trackpad gestures always,
/// and other pointers while `claims` says so, as on a focused board.
class _Claim extends OneSequenceGestureRecognizer {
  bool Function(PointerDownEvent event) claims = (_) => false;
  void Function(Offset pan, double scale, Offset focal)? onPanZoom;

  Offset _pan = Offset.zero;
  double _scale = 1;

  @override
  bool isPointerAllowed(PointerDownEvent event) => claims(event);

  @override
  void addAllowedPointer(PointerDownEvent event) {
    startTrackingPointer(event.pointer, event.transform);
    resolve(GestureDisposition.accepted);
  }

  @override
  bool isPointerPanZoomAllowed(PointerPanZoomStartEvent event) => true;

  @override
  void addAllowedPointerPanZoom(PointerPanZoomStartEvent event) {
    startTrackingPointer(event.pointer, event.transform);
    resolve(GestureDisposition.accepted);
    _pan = Offset.zero;
    _scale = 1;
  }

  @override
  void handleEvent(PointerEvent event) {
    if (event is PointerPanZoomUpdateEvent) {
      onPanZoom?.call(
        event.pan - _pan,
        event.scale / _scale,
        event.localPosition,
      );
      _pan = event.pan;
      _scale = event.scale;
    } else if (event is PointerUpEvent ||
        event is PointerCancelEvent ||
        event is PointerPanZoomEndEvent) {
      stopTrackingPointer(event.pointer);
    }
  }

  @override
  void didStopTrackingLastPointer(int pointer) {}

  @override
  String get debugDescription => 'whiteboard pointers';
}
