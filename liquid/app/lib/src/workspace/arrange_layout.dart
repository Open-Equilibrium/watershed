import 'dart:math' as math;
import 'dart:ui';

import '../rust/api/workspace.dart';

const gridColumns = 12;
const arrangedWidth = 1120.0;
const narrowWidth = 760.0;
const tileGap = 16.0;

class TileSpec {
  TileSpec(this.views, this.first);

  final List<BridgeView> views;

  final int first;

  BridgeView get lead => views.first;

  bool get section => lead.kind == BridgeBlockKind.text;

  int get column => lead.tile?.column ?? 0;

  int get span => lead.tile?.span ?? gridColumns;

  int get last => first + views.length - 1;
}

bool startsSection(BridgeView view, BridgeView? above) =>
    view.kind == BridgeBlockKind.text &&
    (above == null || above.kind != BridgeBlockKind.text || view.boundary);

List<TileSpec> tilesOf(List<BridgeView> views) {
  final tiles = <TileSpec>[];
  for (final (index, view) in views.indexed) {
    final above = index == 0 ? null : views[index - 1];
    if (view.kind == BridgeBlockKind.text && !startsSection(view, above)) {
      tiles.last.views.add(view);
    } else {
      tiles.add(TileSpec([view], index));
    }
  }
  return tiles;
}

bool arranged(List<TileSpec> tiles) =>
    tiles.any((tile) => tile.span < gridColumns || tile.column > 0);

List<Rect> place(
  List<TileSpec> tiles, {
  required double width,
  required double Function(TileSpec tile) heightOf,
  double gap = tileGap,
  bool narrow = false,
  int columns = gridColumns,
}) {
  final step = (width + gap) / columns;
  final tops = List<double>.filled(columns, 0);
  final placed = <Rect>[];
  for (final tile in tiles) {
    var (column, span) = (
      math.min(tile.column, columns - 1),
      math.min(tile.span, columns),
    );
    column = math.min(column, columns - span);
    if (narrow) {
      final half = columns ~/ 2;
      if (span < half) {
        span = half;
        final left = tops.sublist(0, half).reduce(math.max);
        final right = tops.sublist(half).reduce(math.max);
        column = right < left ? half : 0;
      } else {
        (column, span) = (0, columns);
      }
    }
    final top = tops.sublist(column, column + span).reduce(math.max);
    final height = heightOf(tile);
    placed.add(
      Rect.fromLTWH(column * step, top, span * step - gap, height),
    );
    for (var at = column; at < column + span; at++) {
      tops[at] = top + height + gap;
    }
  }
  return placed;
}
