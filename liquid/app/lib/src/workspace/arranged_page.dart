import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../rust/api/workspace.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'arrange_layout.dart';
import 'page_views.dart';
import 'workspace_state.dart';

typedef BlockBuilder = Widget Function(
  BridgeView view,
  int index, {
  required bool wide,
});

class InTile extends InheritedWidget {
  const InTile({super.key, required super.child});

  static bool of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<InTile>() != null;

  @override
  bool updateShouldNotify(InTile old) => false;
}

class ArrangedPage extends StatefulWidget {
  const ArrangedPage({super.key, required this.views, required this.block});

  final PageViews views;
  final BlockBuilder block;

  @override
  State<ArrangedPage> createState() => _ArrangedPageState();
}

typedef _Drag = ({String lead, bool resize, Offset by});

const _reorderBy = 24.0;

class _ArrangedPageState extends State<ArrangedPage> {
  List<BridgeView> _all = const [];
  final _heights = <String, double>{};
  final _remembered = <String, double>{};
  final _asked = <String>{};
  String? _selected;
  String? _entered;
  _Drag? _drag;
  bool _narrow = false;
  Offset? _dragFrom;
  int _reads = 0;
  final _keys = FocusNode(debugLabel: 'Arrange mode');
  Timer? _keep;

  PageViews get _views => widget.views;

  @override
  void initState() {
    super.initState();
    _views.addListener(_reread);
    _views.arranging.addListener(_arranging);
    _views.narrowPreview.addListener(_changed);
    if (_views.arranging.value) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _keys.requestFocus();
      });
    }
    unawaited(_read());
  }

  @override
  void dispose() {
    _views.removeListener(_reread);
    _views.arranging.removeListener(_arranging);
    _views.narrowPreview.removeListener(_changed);
    _keep?.cancel();
    _keepHeights();
    _keys.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _arranging() {
    if (!mounted) return;
    setState(() {
      _selected = null;
      _entered = null;
    });
    if (_views.arranging.value) _keys.requestFocus();
  }

  void _reread() => unawaited(_read());

  Future<void> _read() async {
    final read = ++_reads;
    final views = <BridgeView>[];
    try {
      var count = 1;
      for (var start = 0; start < count; start += window) {
        final answer = await _views.state.read(
          query(BridgeQueryKind.views, page: _views.page, start: start),
        );
        count = answer.count;
        views.addAll(answer.views);
      }
    } on Object {
      return;
    }
    if (!mounted || read != _reads) return;
    setState(() => _all = views);
    for (final tile in tilesOf(views)) {
      final lead = tile.lead.id;
      if (read != _reads) return;
      if (_heights.containsKey('|$lead') || !_asked.add(lead)) continue;
      try {
        final kept = await _views.state.workspace.remembered(
          key: 'height/$lead',
        );
        final height = double.tryParse(kept ?? '');
        if (height != null && mounted && !_heights.containsKey('|$lead')) {
          setState(() => _heights['|$lead'] = _remembered[lead] = height);
        }
      } on Object {
        return;
      }
    }
  }

  String get _mode =>
      '${_views.arranging.value ? 'arranging' : ''}${_narrow ? ' narrow' : ''}|';

  double? _heightOf(String key) => _heights['$_mode$key'] ?? _heights['|$key'];

  void _measured(String measuredKey, double height) {
    final key = '$_mode$measuredKey';
    if ((_heights[key] ?? -1) - height case final diff when diff.abs() < 1) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      setState(() => _heights[key] = height);
      _keep?.cancel();
      _keep = Timer(const Duration(seconds: 1), _keepHeights);
    });
  }

  void _keepHeights() {
    for (final MapEntry(key: measured, :value) in _heights.entries) {
      if (!measured.startsWith('|')) continue;
      final key = measured.substring(1);
      if (key.startsWith('inner ') || _remembered[key] == value) continue;
      _remembered[key] = value;
      unawaited(
        _views.state.workspace
            .remember(key: 'height/$key', value: value.round().toString())
            .then((_) {}, onError: (Object _) {}),
      );
    }
  }

  double _estimate(TileSpec tile) =>
      tile.section ? 32.0 * tile.views.length + 8 : 320;

  void _write(BridgeMutation change) => unawaited(
    _views.write(change, const []).then((_) {}, onError: (Object _) {}),
  );

  BridgeTile? _tileOf(int column, int span) =>
      column == 0 && span == gridColumns
      ? null
      : BridgeTile(column: column, span: span);

  int _spanOf(TileSpec tile, {required bool inner}) =>
      math.min(tile.span, inner ? _enteredSpan : gridColumns);

  void _setTile(TileSpec tile, int column, int span, {bool inner = false}) {
    final columns = inner ? _enteredSpan : gridColumns;
    span = span.clamp(1, columns);
    column = column.clamp(0, columns - span);
    if (column == tile.column && span == _spanOf(tile, inner: inner)) return;
    _write(
      mutation(
        inner ? BridgeMutationKind.setInnerTile : BridgeMutationKind.setTile,
        page: _views.page,
        view: tile.lead.id,
        tile: inner && span == columns && column == 0
            ? null
            : inner
            ? BridgeTile(column: column, span: span)
            : _tileOf(column, span),
      ),
    );
  }

  int get _enteredSpan =>
      tilesOf(_all)
          .where((tile) => tile.lead.id == _entered)
          .firstOrNull
          ?.span ??
      gridColumns;

  void _move(List<TileSpec> tiles, int from, int? afterTile) {
    final tile = tiles[from];
    final after = afterTile == null ? null : tiles[afterTile];
    if (afterTile == from ||
        afterTile == from - 1 ||
        (afterTile == null && from == 0)) {
      return;
    }
    final rest = [
      for (final (index, shown) in tiles.indexed)
        if (index != from) shown,
    ];
    final at = after == null ? 0 : rest.indexOf(after) + 1;
    final before = at == 0 ? null : rest[at - 1];
    final next = at < rest.length ? rest[at] : null;
    final oldBefore = from == 0 ? null : tiles[from - 1];
    final oldNext = from + 1 < tiles.length ? tiles[from + 1] : null;
    void boundary(TileSpec? tile, TileSpec? above) {
      if (tile == null || !tile.section || tile.lead.boundary) return;
      if (above == null || !above.section) return;
      _write(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _views.page,
          view: tile.lead.id,
          boundary: true,
        ),
      );
    }

    boundary(oldNext, oldBefore);
    boundary(tile, before);
    boundary(next, tile);
    _write(
      mutation(
        BridgeMutationKind.moveViews,
        page: _views.page,
        view: tile.lead.id,
        lastView: tile.views.last.id,
        after: before?.views.last.id,
      ),
    );
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (!_views.arranging.value || event is KeyUpEvent) {
      return KeyEventResult.ignored;
    }
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.escape) {
      if (_entered != null) {
        setState(() {
          _selected = _entered;
          _entered = null;
        });
      } else {
        _views.arranging.value = false;
      }
      return KeyEventResult.handled;
    }
    final tiles = _entered == null ? tilesOf(_all) : _innerTiles();
    final at = tiles.indexWhere((tile) => tile.lead.id == _selected);
    if (at < 0) return KeyEventResult.ignored;
    final tile = tiles[at];
    final inner = _entered != null;
    final resize = HardwareKeyboard.instance.isAltPressed;
    switch (key) {
      case LogicalKeyboardKey.enter when tile.section && !inner:
        setState(() {
          _entered = tile.lead.id;
          _selected = null;
        });
      case LogicalKeyboardKey.arrowLeft || LogicalKeyboardKey.arrowRight
          when !_narrow:
        final by = key == LogicalKeyboardKey.arrowLeft ? -1 : 1;
        final span = _spanOf(tile, inner: inner);
        if (resize) {
          _setTile(tile, tile.column, span + by, inner: inner);
        } else {
          _setTile(tile, tile.column + by, span, inner: inner);
        }
      case LogicalKeyboardKey.arrowUp when !inner && at > 0:
        _move(tiles, at, at - 2 < 0 ? null : at - 2);
      case LogicalKeyboardKey.arrowDown when !inner && at + 1 < tiles.length:
        _move(tiles, at, at + 1);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  List<TileSpec> _innerTiles() {
    final section = tilesOf(_all)
        .where((tile) => tile.lead.id == _entered)
        .firstOrNull;
    if (section == null) return const [];
    return [
      for (final (index, view) in section.views.indexed)
        TileSpec([view], section.first + index, inner: true),
    ];
  }

  void _dropped(
    List<TileSpec> tiles,
    List<Rect> rects,
    int index,
    _Drag drag,
    double step, {
    bool inner = false,
  }) {
    final tile = tiles[index];
    final rect = rects[index];
    final columns = inner ? _enteredSpan : gridColumns;
    if (_narrow) {
      if (!drag.resize && !inner) _reorder(tiles, rects, index, drag);
      return;
    }
    if (drag.resize) {
      final span = ((rect.width + drag.by.dx + tileGap) / step).round();
      _setTile(tile, tile.column, span.clamp(1, columns), inner: inner);
      return;
    }
    final column = ((rect.left + drag.by.dx) / step).round();
    _setTile(tile, column, _spanOf(tile, inner: inner), inner: inner);
    if (!inner) _reorder(tiles, rects, index, drag);
  }

  void _reorder(List<TileSpec> tiles, List<Rect> rects, int index, _Drag drag) {
    if (drag.by.dy.abs() < _reorderBy) return;
    final top = rects[index].top + drag.by.dy;
    int? after;
    for (final (at, shown) in rects.indexed) {
      if (at != index && shown.top <= top) after = at;
    }
    _move(tiles, index, after);
  }

  Widget _tile({
    required TileSpec tile,
    required Rect rect,
    required double step,
    required List<TileSpec> tiles,
    required List<Rect> rects,
    required int index,
    required Widget child,
    bool inner = false,
  }) {
    final ws = context.ws;
    final arranging = _views.arranging.value;
    final key = inner ? 'inner ${tile.lead.id}' : tile.lead.id;
    final framedSection = !inner && _entered == tile.lead.id;
    final measured = _Measure(
      onHeight: (height) => _measured(key, height),
      child: arranging && !framedSection
          ? Padding(
              padding: const EdgeInsets.all(WsMetrics.space2),
              child: child,
            )
          : child,
    );
    if (!arranging) return measured;
    if (framedSection) {
      return Container(
        foregroundDecoration: BoxDecoration(
          border: Border.all(color: ws.accent, width: 2),
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        ),
        child: measured,
      );
    }
    final selected = _selected == tile.lead.id;
    final drag = _drag?.lead == key ? _drag : null;
    final width = drag != null && drag.resize
        ? math.max(step - tileGap, rect.width + drag.by.dx)
        : rect.width;
    final span = drag != null && drag.resize
        ? ((width + tileGap) / step).round()
        : _spanOf(tile, inner: inner);
    final dimmed = !inner && _entered != null && _entered != tile.lead.id;
    void dragged(bool resize, Offset by) =>
        setState(() => _drag = (lead: key, resize: resize, by: by));
    void end() {
      final done = _drag;
      _dragFrom = null;
      setState(() => _drag = null);
      if (done != null && done.lead == key) {
        _dropped(tiles, rects, index, done, step, inner: inner);
      }
    }

    Widget handle(Key handleKey, bool resize, Widget shown) => Listener(
      key: handleKey,
      behavior: HitTestBehavior.opaque,
      onPointerDown: (event) {
        _dragFrom = event.position;
        setState(() => _selected = tile.lead.id);
      },
      onPointerMove: (event) {
        if (_dragFrom case final from?) dragged(resize, event.position - from);
      },
      onPointerUp: (_) => end(),
      onPointerCancel: (_) {
        _dragFrom = null;
        setState(() => _drag = null);
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanStart: (_) {},
        onPanUpdate: (_) {},
        child: MouseRegion(
          cursor: resize
              ? SystemMouseCursors.resizeLeftRight
              : SystemMouseCursors.grab,
          child: shown,
        ),
      ),
    );

    final framed = Stack(
      clipBehavior: Clip.none,
      children: [
        SizedBox(
          width: width,
          child: Container(
            foregroundDecoration: BoxDecoration(
              border: Border.all(
                color: selected ? ws.accent : ws.borderStrong,
                width: selected ? 2 : 1,
              ),
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            ),
            child: AbsorbPointer(child: measured),
          ),
        ),
        Positioned.fill(
          child: Listener(
            key: Key('select $key'),
            behavior: HitTestBehavior.translucent,
            onPointerDown: (_) {
              setState(() => _selected = tile.lead.id);
              _keys.requestFocus();
            },
            child: GestureDetector(
              behavior: HitTestBehavior.translucent,
              onDoubleTap: tile.section && !inner
                  ? () => setState(() {
                      _entered = tile.lead.id;
                      _selected = null;
                    })
                  : null,
            ),
          ),
        ),
        Positioned(
          top: -6,
          left: width / 2 - 20,
          child: handle(
            Key('move $key'),
            false,
            Container(
              width: 40,
              height: 18,
              decoration: BoxDecoration(
                color: selected ? ws.accent : ws.surface,
                border: Border.all(color: selected ? ws.accent : ws.border),
                borderRadius: BorderRadius.circular(9),
              ),
              alignment: Alignment.center,
              child: Text(
                '···',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  height: 1,
                  color: selected ? ws.surface : ws.textMuted,
                ),
              ),
            ),
          ),
        ),
        Positioned(
          top: 0,
          bottom: 0,
          left: width - 6,
          child: handle(
            Key('resize $key'),
            true,
            Center(
              child: Container(
                width: 6,
                height: 32,
                decoration: BoxDecoration(
                  color: selected ? ws.accent : ws.borderStrong,
                  borderRadius: BorderRadius.circular(3),
                ),
              ),
            ),
          ),
        ),
        if (selected || drag != null)
          Positioned(
            right: math.max(0, rect.width - width) + 8,
            bottom: -10,
            child: Container(
              key: const Key('tile span'),
              padding: const EdgeInsets.symmetric(
                horizontal: WsMetrics.space2,
                vertical: 2,
              ),
              decoration: BoxDecoration(
                color: ws.accent,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Text(
                '$span of ${inner ? _enteredSpan : gridColumns}',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.surface,
                ),
              ),
            ),
          ),
      ],
    );
    final moved = drag != null && !drag.resize
        ? Transform.translate(offset: drag.by, child: framed)
        : framed;
    return dimmed
        ? IgnorePointer(child: Opacity(opacity: 0.35, child: moved))
        : moved;
  }

  Widget _section(TileSpec tile, double width) {
    final entered = _entered == tile.lead.id;
    final columns = tile.span;
    final innerTiles = [
      for (final (index, view) in tile.views.indexed)
        TileSpec([view], tile.first + index, inner: true),
    ];
    final columnsUsed = innerTiles.any(
      (inner) => inner.span < columns || inner.column > 0,
    );
    if (!entered && !columnsUsed) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final inner in innerTiles)
            widget.block(inner.lead, inner.first, wide: false),
        ],
      );
    }
    final rects = place(
      innerTiles,
      width: width,
      columns: columns,
      heightOf: (inner) => _heightOf('inner ${inner.lead.id}') ?? 32,
    );
    final step = (width + tileGap) / columns;
    final height = rects.fold(0.0, (most, rect) => math.max(most, rect.bottom));
    return SizedBox(
      height: height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          for (final (index, inner) in innerTiles.indexed)
            Positioned(
              left: rects[index].left,
              top: rects[index].top,
              width: rects[index].width,
              child: entered
                  ? _tile(
                      tile: inner,
                      rect: rects[index],
                      step: step,
                      tiles: innerTiles,
                      rects: rects,
                      index: index,
                      inner: true,
                      child: widget.block(inner.lead, inner.first, wide: false),
                    )
                  : _Measure(
                      onHeight: (h) => _measured('inner ${inner.lead.id}', h),
                      child: widget.block(inner.lead, inner.first, wide: false),
                    ),
            ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return SliverLayoutBuilder(
      builder: (context, constraints) {
        final arranging = _views.arranging.value;
        var width = math.min(constraints.crossAxisExtent, arrangedWidth);
        final preview = arranging && _views.narrowPreview.value;
        if (preview) width = math.min(width, narrowWidth - 160);
        final narrow = preview || width < narrowWidth;
        _narrow = narrow;
        final tiles = tilesOf(_all);
        final rects = place(
          tiles,
          width: width,
          narrow: narrow,
          heightOf: (tile) => _heightOf(tile.lead.id) ?? _estimate(tile),
        );
        final step = (width + tileGap) / gridColumns;
        final total = rects.fold(
          0.0,
          (most, rect) => math.max(most, rect.bottom),
        );
        final from = constraints.scrollOffset - 600;
        final to =
            constraints.scrollOffset + constraints.remainingPaintExtent + 600;
        return SliverToBoxAdapter(
          child: Focus(
            focusNode: _keys,
            onKeyEvent: _key,
            child: InTile(
              child: Center(
                child: SizedBox(
                  key: const Key('arranged page'),
                  width: width,
                  height: total + (arranging ? WsMetrics.space6 : 0),
                  child: Stack(
                    clipBehavior: Clip.none,
                    children: [
                      if (arranging)
                        Positioned.fill(
                          child: Row(
                            children: [
                              for (
                                var column = 0;
                                column < gridColumns;
                                column++
                              )
                                Padding(
                                  padding: EdgeInsets.only(
                                    right: column + 1 < gridColumns
                                        ? tileGap
                                        : 0,
                                  ),
                                  child: Container(
                                    width: step - tileGap,
                                    color: ws.accentSubtle.withValues(
                                      alpha: 0.5,
                                    ),
                                  ),
                                ),
                            ],
                          ),
                        ),
                      for (final (index, tile) in tiles.indexed)
                        if (rects[index].bottom >= from &&
                            rects[index].top <= to)
                          Positioned(
                            key: ValueKey('tile ${tile.lead.id}'),
                            left: rects[index].left,
                            top: rects[index].top,
                            width: rects[index].width,
                            child: _tile(
                              tile: tile,
                              rect: rects[index],
                              step: step,
                              tiles: tiles,
                              rects: rects,
                              index: index,
                              child: tile.section
                                  ? _section(tile, rects[index].width)
                                  : widget.block(
                                      tile.lead,
                                      tile.first,
                                      wide: true,
                                    ),
                            ),
                          ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _Measure extends SingleChildRenderObjectWidget {
  const _Measure({required this.onHeight, super.child});

  final ValueChanged<double> onHeight;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMeasure(onHeight);

  @override
  void updateRenderObject(BuildContext context, _RenderMeasure renderObject) {
    renderObject.onHeight = onHeight;
  }
}

class _RenderMeasure extends RenderProxyBox {
  _RenderMeasure(this.onHeight);

  ValueChanged<double> onHeight;

  @override
  void performLayout() {
    super.performLayout();
    onHeight(size.height);
  }
}
