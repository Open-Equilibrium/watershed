import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/arrange_layout.dart';

BridgeView view(
  String id,
  BridgeBlockKind kind, {
  BridgeTile? tile,
  bool boundary = false,
}) => BridgeView(
  id: id,
  block: 'b$id',
  title: '',
  boundary: boundary,
  kind: kind,
  tabs: const [],
  tile: tile,
);

BridgeTile tile(int column, int span) => BridgeTile(column: column, span: span);

void main() {
  const text = BridgeBlockKind.text;
  const board = BridgeBlockKind.whiteboard;

  test('a Section of Text Blocks is one tile, each other Block its own', () {
    final tiles = tilesOf([
      view('plan', text, tile: tile(0, 5)),
      view('flight', text),
      view('photo', board, tile: tile(5, 7)),
      view('notes', text),
      view('ideas', text, boundary: true),
    ]);
    expect([for (final tile in tiles) tile.views.map((v) => v.id).join('+')], [
      'plan+flight',
      'photo',
      'notes',
      'ideas',
    ]);
    expect([for (final tile in tiles) (tile.column, tile.span)], [
      (0, 5),
      (5, 7),
      (0, 12),
      (0, 12),
    ]);
    expect(arranged(tiles), isTrue);
    expect(arranged(tilesOf([view('notes', text)])), isFalse);
  });

  test('each tile rises as high as the tiles above it allow', () {
    final tiles = tilesOf([
      view('plan', text, tile: tile(0, 5)),
      view('photo', board, tile: tile(5, 7)),
      view('notes', text, tile: tile(5, 7)),
      view('ideas', text, tile: tile(0, 5), boundary: true),
    ]);
    final heights = {'plan': 200.0, 'photo': 150.0, 'notes': 100.0};
    final placed = place(
      tiles,
      width: 1200,
      gap: 0,
      heightOf: (tile) => heights[tile.views.first.id] ?? 80,
    );
    expect(placed, [
      const Rect.fromLTWH(0, 0, 500, 200),
      const Rect.fromLTWH(500, 0, 700, 150),
      const Rect.fromLTWH(500, 150, 700, 100),
      const Rect.fromLTWH(0, 200, 500, 80),
    ]);
  });

  test('a narrow window makes tiles under half the grid half, the rest full',
      () {
    final tiles = tilesOf([
      view('note', board, tile: tile(8, 4)),
      view('table', board, tile: tile(0, 7)),
      view('chart', board, tile: tile(7, 5)),
      view('left', board, tile: tile(0, 5)),
    ]);
    final placed = place(
      tiles,
      width: 600,
      gap: 0,
      narrow: true,
      heightOf: (_) => 100,
    );
    expect(placed, [
      const Rect.fromLTWH(0, 0, 300, 100),
      const Rect.fromLTWH(0, 100, 600, 100),
      const Rect.fromLTWH(0, 200, 300, 100),
      const Rect.fromLTWH(300, 200, 300, 100),
    ]);
  });
}
