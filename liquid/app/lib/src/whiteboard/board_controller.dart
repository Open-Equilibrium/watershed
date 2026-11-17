import 'dart:async';
import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter/foundation.dart';

import '../rust/api/drawing.dart';
import '../rust/api/workspace.dart';
import '../workspace/follow.dart';
import '../workspace/page_views.dart';
import '../workspace/workspace_state.dart';
import 'board.dart';
import 'document_pages.dart';

/// The five tools, as GoodNotes has them (ADR-0306).
enum BoardTool { lasso, pen, highlighter, eraser, shape, text }

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

  /// Whether the eraser takes only the highlighter's marks, as GoodNotes' erase filter.
  bool highlighterOnly = false;

  /// Each tool's color and width; the eraser's width is its size, text's its size.
  final colors = <BoardTool, BridgeInkColor>{
    BoardTool.pen: BridgeInkColor.data3,
    BoardTool.highlighter: BridgeInkColor.yellow,
    BoardTool.shape: BridgeInkColor.data2,
    BoardTool.text: BridgeInkColor.text,
  };
  final widths = <BoardTool, int>{
    BoardTool.pen: 1,
    BoardTool.highlighter: 1,
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
  BridgeDrawingStyle style = BridgeDrawingStyle.handDrawn;
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

  /// Gestures under way, which a read must not overtake, and whether one was held back.
  int _holds = 0;
  var _heldRead = false;

  /// Holds reads back while a gesture works on the elements as shown.
  void hold() => _holds++;

  /// Ends a gesture; a read held back meanwhile runs now, unless an edit's own read will.
  void release() {
    if (_holds > 0) _holds--;
    if (_holds == 0 && _heldRead) {
      _heldRead = false;
      if (_writing == 0) unawaited(read());
    }
  }

  int _reads = 0;
  var _disposed = false;

  bool get handDrawn => style == BridgeDrawingStyle.handDrawn;

  void _report(Object error) => _state.report('$error');

  /// Reads the board; a read begun before an edit sent since is dropped, as the edit's
  /// own change brings a newer one.
  Future<void> read() async {
    final asked = ++_reads;
    final writing = _writing;
    try {
      final (start, count) = paged ? window : (0, 1);
      final answer = await _state.read(
        query(
          BridgeQueryKind.drawing,
          page: views.page,
          block: view.block,
          start: start,
          limit: count,
        ),
      );
      if (_disposed || asked != _reads || writing != _writing) return;
      if (_holds > 0) {
        _heldRead = true;
        return;
      }
      final board = answer.drawing;
      if (board == null) {
        // Gone, as another device deleted it: full screen shows it no more.
        if (views.fullBoard.value?.id == view.id) views.fullBoard.value = null;
        return;
      }
      elements = [for (final element in board.elements) ?_onBoard(element)];
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
      // Each with a name of its own until the read brings the core's.
      for (final element in add)
        _applied(
          element,
          BridgeElementChange(
            id: '$pendingPrefix${_pending++}',
            x: element.x,
            y: element.y,
            w: element.w,
            h: element.h,
            rotation: element.rotation,
            color: element.color,
            width: element.width,
          ),
          id: true,
        ),
    ];
    selected = selected.difference(removed);
    notifyListeners();
    _writing++;
    try {
      await views.write(
        mutation(
          BridgeMutationKind.editDrawing,
          page: views.page,
          block: view.block,
          drawing: BridgeDrawingEdit(
            add: [for (final element in add) _onPage(element)],
            change: [for (final changed in change) _changeOnPage(changed)],
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

  int _pending = 0;

  /// A document's pages on the board, top to bottom, in their points; none on a Whiteboard,
  /// whose canvas has no edges.
  List<Rect> sheets = const [];

  /// What full screen calls the drawing: "Whiteboard", or a document's file name.
  String title = 'Whiteboard';

  /// Whether the view stays where it is, as over an image shown on the Page.
  bool fixed = false;

  /// The pages beneath a document's ink; none on a Whiteboard.
  DocumentPages? get pages => _pages;
  DocumentPages? _pages;
  set pages(DocumentPages? pages) {
    _pages?.close();
    _pages = pages;
    sheets = pages?.sheets ?? const [];
    notifyListeners();
  }

  bool get paged => sheets.isNotEmpty;

  /// The document pages read: the first and how many, around those in view.
  (int, int) window = (0, 1);

  /// Reads the pages from `first`, `count` of them, unless they are read already.
  void showPages(int first, int count) {
    if (window == (first, count)) return;
    window = (first, count);
    unawaited(read());
  }

  /// The page under a board point, if any.
  int? pageAt(Offset at) {
    for (final (index, sheet) in sheets.indexed) {
      if (sheet.contains(at)) return index;
    }
    return null;
  }

  /// The page nearest a board point.
  int _nearestPage(Offset at) {
    var best = 0;
    var distance = double.infinity;
    for (final (index, sheet) in sheets.indexed) {
      final away = (sheet.center.dy - at.dy).abs();
      if (away < distance) (best, distance) = (index, away);
    }
    return best;
  }

  /// `element` as stored, its frame in its page's points, moved onto the board; none for a
  /// page the document does not have.
  BridgeElement? _onBoard(BridgeElement element) {
    if (!paged) return element;
    if (element.docPage >= sheets.length) return null;
    final at = sheets[element.docPage].topLeft;
    return _moved(element, at, element.docPage);
  }

  /// `element` from the board onto the page it lies on, in that page's points.
  BridgeElement _onPage(BridgeElement element) {
    if (!paged) return element;
    final page = _nearestPage(frameOf(element).center);
    return _moved(element, -sheets[page].topLeft, page);
  }

  BridgeElementChange _changeOnPage(BridgeElementChange change) {
    if (!paged) return change;
    final element = elements
        .where((shown) => shown.id == change.id)
        .firstOrNull;
    final page = element?.docPage ?? 0;
    final at = page < sheets.length ? sheets[page].topLeft : Offset.zero;
    return BridgeElementChange(
      id: change.id,
      x: change.x - at.dx,
      y: change.y - at.dy,
      w: change.w,
      h: change.h,
      rotation: change.rotation,
      color: change.color,
      width: change.width,
      text: change.text,
      arrowhead: change.arrowhead,
    );
  }

  static BridgeElement _moved(BridgeElement element, Offset by, int page) =>
      BridgeElement(
        id: element.id,
        kind: element.kind,
        x: element.x + by.dx,
        y: element.y + by.dy,
        w: element.w,
        h: element.h,
        rotation: element.rotation,
        color: element.color,
        width: element.width,
        docPage: page,
        pen: element.pen,
        points: element.points,
        shape: element.shape,
        seed: element.seed,
        arrowhead: element.arrowhead,
        corners: element.corners,
        text: element.text,
      );

  /// `element` after `c`; with `id`, under the change's name too.
  static BridgeElement _applied(
    BridgeElement element,
    BridgeElementChange c, {
    bool id = false,
  }) => BridgeElement(
    id: id ? c.id : element.id,
    kind: element.kind,
    x: c.x,
    y: c.y,
    w: c.w,
    h: c.h,
    rotation: c.rotation,
    color: c.color,
    width: c.width,
    docPage: element.docPage,
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
        ? BridgeDrawingStyle.clean
        : BridgeDrawingStyle.handDrawn;
    style = next;
    notifyListeners();
    _writing++;
    try {
      await views.write(
        mutation(
          BridgeMutationKind.setDrawingStyle,
          page: views.page,
          block: view.block,
          drawingStyle: next,
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
    if (fixed) return;
    origin -= by / zoom;
    notifyListeners();
  }

  /// Zooms by `factor` around the view point `focus`, within 10 % and 400 %.
  void zoomBy(double factor, Offset focus) {
    if (fixed) return;
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
    _pages?.close();
    unawaited(_followed?.cancel());
    super.dispose();
  }
}
