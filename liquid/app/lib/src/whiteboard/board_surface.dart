import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/drawing.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'board.dart';
import 'board_claim.dart';
import 'board_controller.dart';
import 'board_painter.dart';
import 'ink_geometry.dart';
import 'shape_snap.dart';

part 'board_ink.dart';
part 'board_erase.dart';
part 'board_select.dart';

class BoardSurface extends StatefulWidget {
  const BoardSurface({
    super.key,
    required this.board,
    required this.focused,
    required this.onFocus,
    required this.onLeave,
  });

  final BoardController board;

  final bool focused;

  final VoidCallback onFocus;

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

  final _ink = <InkPoint>[];
  BridgeElement? _snapped;
  Timer? _hold;

  Map<String, BridgeElement> _before = const {};
  (List<Offset>, double)? _boxBefore;
  int _corner = 0;

  final _erased = <String>{};
  final _pieces = <String, List<BridgeElement>>{};

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
      if (_drag == _Drag.move ||
          _drag == _Drag.resize ||
          _drag == _Drag.rotate) {
        _endTransform();
      }
      board.select(const {});
    }
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    tools.removeListener(_toolsChanged);
    _hold?.cancel();
    if (_held) {
      _held = false;
      board.release();
    }
    _typed.dispose();
    _typing.dispose();
    super.dispose();
  }

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
    final drawing =
        tools.tool != BoardTool.lasso && tools.tool != BoardTool.eraser;
    if (board.paged && drawing && board.pageAt(at) == null) {
      _pointer = null;
      return;
    }
    switch (tools.tool) {
      case BoardTool.pen || BoardTool.highlighter:
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
    if (_drag != _Drag.none && _drag != _Drag.pan) {
      _held = true;
      board.hold();
    }
  }

  var _held = false;

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
    if (_held) {
      _held = false;
      board.release();
    }
    setState(() => _draft = Draft.none);
  }

  double _pressure(PointerEvent event) =>
      event.kind == PointerDeviceKind.stylus ||
          event.kind == PointerDeviceKind.invertedStylus
      ? event.pressure.clamp(0.05, 1.0)
      : 0.5;

  void _signal(PointerSignalEvent event) {
    if (event is! PointerScrollEvent || board.fixed) return;
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

  void _armHold() {
    _hold?.cancel();
    _hold = Timer(const Duration(milliseconds: 500), () {
      if (_drag != _Drag.ink ||
          _ink.length < 3 ||
          tools.tool == BoardTool.highlighter) {
        return;
      }
      final shape = _shapeOf(_ink);
      if (shape == null) return;
      setState(() {
        _snapped = shape;
        _draft = Draft(shape: shape);
      });
    });
  }

  void _showInk() => setState(
    () => _draft = Draft(
      ink: [..._ink],
      pen: _inkPen,
      color: _inkColor,
      width: _inkWidth,
    ),
  );

  void _showDragged(Offset from, Offset to) =>
      setState(() => _draft = Draft(shape: _dragShape(from, to)));

  void _erase(Offset at) {
    final radius = _eraserRadius;
    final changed = _sweep(at);
    setState(() => _draft = Draft(eraser: at, eraserRadius: radius));
    if (changed) board.notifyListeners();
  }

  int _localIds = 0;

  void _startLasso(Offset view, Offset at) {
    if (_grab(view, at)) return;
    _drag = _Drag.loop;
    setState(() => _draft = Draft(loop: [at]));
  }

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
    final height = BoardPainter.textHeight(
      text,
      element.w,
      element.width,
      board.handDrawn,
    );
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
  ) => copyElement(
    element,
    x: frame.left,
    y: frame.top,
    w: frame.width,
    h: frame.height,
    arrowhead: false,
    text: text,
  );

  KeyEventResult key(KeyEvent event) {
    if (_text != null) {
      if (event is KeyDownEvent &&
          event.logicalKey == LogicalKeyboardKey.escape) {
        _typing.unfocus();
        widget.onFocus();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }
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
      case LogicalKeyboardKey.keyH:
        tools.tool = BoardTool.highlighter;
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
        return Stack(
          children: [
            Positioned.fill(
              child: RawGestureDetector(
                gestures: {
                  BoardClaim: GestureRecognizerFactoryWithHandlers<BoardClaim>(
                    () => BoardClaim(),
                    (claim) => claim
                      ..claims = ((event) =>
                          widget.focused ||
                          event.buttons & kSecondaryMouseButton != 0)
                      ..panZooms = !board.fixed
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
                    child: CustomPaint(
                      key: const Key('whiteboard canvas'),
                      size: Size.infinite,
                      painter: BoardPainter(
                        board: board,
                        ws: ws,
                        draft: _draft,
                        hidden: const {},
                        editing: text?.id,
                        dots: widget.focused && !board.paged,
                        pixelRatio: MediaQuery.devicePixelRatioOf(context),
                      ),
                    ),
                  ),
                ),
              ),
            ),
            if (text != null) _textEditor(context, text),
            if (board.loaded && board.elements.isEmpty && widget.focused)
              Positioned(
                left: 12,
                top: 10,
                child: IgnorePointer(
                  child: Text(
                    'Draw with the pen, or choose a tool below',
                    style: TextStyle(
                      fontSize: WsMetrics.fsSmall,
                      color: ws.textFaint,
                    ),
                  ),
                ),
              ),
          ],
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
