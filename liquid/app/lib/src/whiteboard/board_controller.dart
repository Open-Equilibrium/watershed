import 'dart:async';
import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter/foundation.dart';

import '../rust/api/whiteboard.dart';
import '../rust/api/workspace.dart';
import '../workspace/follow.dart';
import '../workspace/page_views.dart';
import '../workspace/workspace_state.dart';
import 'board.dart';

/// The five tools, as GoodNotes has them (ADR-0306).
enum BoardTool { lasso, pen, eraser, shape, text }

/// What the shape tool makes: a drawn shape snapped, or one dragged out.
enum ShapeKind { snap, rectangle, ellipse, line, arrow }

/// The tools and their options, the same on every Whiteboard for the session.
class BoardTools extends ChangeNotifier {
  BoardTool _tool = BoardTool.pen;
  BoardTool get tool => _tool;
  set tool(BoardTool tool) => _set(() => _tool = tool);

  BridgePen pen = BridgePen.fountain;
  ShapeKind shape = ShapeKind.snap;

  /// Whether the eraser removes whole elements, rather than the parts it passes.
  bool wholeEraser = false;

  /// Each tool's color and width; the eraser's width is its size, text's its size.
  final colors = <BoardTool, BridgeInkColor>{
    BoardTool.pen: BridgeInkColor.data3,
    BoardTool.shape: BridgeInkColor.data2,
    BoardTool.text: BridgeInkColor.text,
  };
  final widths = <BoardTool, int>{
    BoardTool.pen: 1,
    BoardTool.shape: 1,
    BoardTool.text: 1,
    BoardTool.eraser: 1,
  };

  BridgeInkColor colorOf(BoardTool tool) => colors[tool] ?? BridgeInkColor.text;
  int widthOf(BoardTool tool) => widths[tool] ?? 1;

  void update(VoidCallback change) => _set(change);

  void _set(VoidCallback change) {
    change();
    notifyListeners();
  }
}

/// The session's tools.
final boardTools = BoardTools();

/// One Whiteboard as the Page shows it: its elements as last read, with the edits sent
/// since shown at once, where the canvas looks, and what is selected. The Block and its
/// full-screen view share it.
class BoardController extends ChangeNotifier {
  BoardController(this.views, this.view) {
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: views.page,
        block: view.block,
      ),
    ).listen((_) => unawaited(read()), onError: _report);
  }

  final PageViews views;
  final BridgeView view;
  final cache = BoardCache();

  WorkspaceState get _state => views.state;

  StreamSubscription<BridgeChange>? _followed;

  /// The elements, bottom to top; none until first read.
  List<BridgeElement> elements = const [];
  BridgeWhiteboardStyle style = BridgeWhiteboardStyle.handDrawn;
  bool loaded = false;

  /// Where the canvas looks: the board point at the view's top left, and the zoom.
  Offset origin = const Offset(-20, -20);
  double zoom = 1;

  static const minZoom = 0.1;
  static const maxZoom = 4.0;

  /// The selected elements' identities.
  Set<String> selected = const {};

  /// Edits sent and not yet read back.
  int _writing = 0;
  int _reads = 0;
  var _disposed = false;

  bool get handDrawn => style == BridgeWhiteboardStyle.handDrawn;

  void _report(Object error) => _state.report('$error');

  /// Reads the board; a read begun before an edit sent since is dropped, as the edit's
  /// own change brings a newer one.
  Future<void> read() async {
    final asked = ++_reads;
    final writing = _writing;
    try {
      final answer = await _state.read(
        query(BridgeQueryKind.whiteboard, page: views.page, block: view.block),
      );
      if (_disposed || asked != _reads || writing != _writing) return;
      final board = answer.whiteboard;
      if (board == null) return;
      elements = board.elements;
      style = board.style;
      loaded = true;
      selected = {
        for (final element in elements)
          if (selected.contains(element.id)) element.id!,
      };
      cache.keepOnly([for (final element in elements) element.id ?? '']);
      notifyListeners();
    } on Object catch (error) {
      if (!_disposed) _report(error);
    }
  }

  /// Removes, changes and adds elements as one Action, showing the result at once.
  Future<void> edit({
    List<BridgeElement> add = const [],
    List<BridgeElementChange> change = const [],
    List<String> remove = const [],
  }) async {
    if (add.isEmpty && change.isEmpty && remove.isEmpty) return;
    final removed = remove.toSet();
    final changes = {for (final changed in change) changed.id: changed};
    elements = [
      for (final element in elements)
        if (!removed.contains(element.id))
          if (changes[element.id] case final changed?)
            _applied(element, changed)
          else
            element,
      ...add,
    ];
    selected = selected.difference(removed);
    notifyListeners();
    _writing++;
    try {
      await views.write(
        mutation(
          BridgeMutationKind.editWhiteboard,
          page: views.page,
          block: view.block,
          whiteboard: BridgeWhiteboardEdit(
            add: add,
            change: change,
            remove: remove,
          ),
        ),
        [view.block],
      );
    } on Object {
      // The Page shows the problem; the board as stored comes back.
    } finally {
      _writing--;
      if (!_disposed) unawaited(read());
    }
  }

  static BridgeElement _applied(BridgeElement element, BridgeElementChange c) =>
      BridgeElement(
        id: element.id,
        kind: element.kind,
        x: c.x,
        y: c.y,
        w: c.w,
        h: c.h,
        rotation: c.rotation,
        color: c.color,
        width: c.width,
        pen: element.pen,
        points: element.points,
        shape: element.shape,
        seed: element.seed,
        arrowhead: c.arrowhead ?? element.arrowhead,
        corners: element.corners,
        text: c.text ?? element.text,
      );

  /// Switches the whole board between hand-drawn and clean.
  Future<void> toggleStyle() async {
    final next = handDrawn
        ? BridgeWhiteboardStyle.clean
        : BridgeWhiteboardStyle.handDrawn;
    style = next;
    notifyListeners();
    _writing++;
    try {
      await views.write(
        mutation(
          BridgeMutationKind.setWhiteboardStyle,
          page: views.page,
          block: view.block,
          whiteboardStyle: next,
        ),
        [view.block],
      );
    } on Object {
      // The Page shows the problem; the style as stored comes back.
    } finally {
      _writing--;
      if (!_disposed) unawaited(read());
    }
  }

  void select(Set<String> ids) {
    selected = ids;
    notifyListeners();
  }

  /// The selected elements, bottom to top.
  List<BridgeElement> get selection => [
    for (final element in elements)
      if (selected.contains(element.id)) element,
  ];

  /// A board point at a point of the view.
  Offset toBoard(Offset local) => origin + local / zoom;

  /// A point of the view at a board point.
  Offset toView(Offset board) => (board - origin) * zoom;

  void pan(Offset by) {
    origin -= by / zoom;
    notifyListeners();
  }

  /// Zooms by `factor` around the view point `focus`, within 10 % and 400 %.
  void zoomBy(double factor, Offset focus) {
    final at = toBoard(focus);
    zoom = (zoom * factor).clamp(minZoom, maxZoom);
    origin = at - focus / zoom;
    notifyListeners();
  }

  void zoomTo(double value, Size view) =>
      zoomBy(value / zoom, view.center(Offset.zero));

  /// Shows everything drawn in a view of `size`.
  void fit(Size size) {
    if (elements.isEmpty || size.isEmpty) {
      origin = const Offset(-20, -20);
      zoom = 1;
    } else {
      var bounds = boundsOf(elements.first);
      for (final element in elements.skip(1)) {
        bounds = bounds.expandToInclude(boundsOf(element));
      }
      bounds = bounds.inflate(24);
      zoom = math
          .min(size.width / bounds.width, size.height / bounds.height)
          .clamp(minZoom, 1.0);
      origin = bounds.center - size.center(Offset.zero) / zoom;
    }
    notifyListeners();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_followed?.cancel());
    super.dispose();
  }
}
