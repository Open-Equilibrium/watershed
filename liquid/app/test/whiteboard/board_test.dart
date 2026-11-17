import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/board.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';

void main() {
  test('ink past the core\'s 10,000 points goes as joined strokes', () {
    final ink = [for (var at = 0; at < 25000; at++) InkPoint(at.toDouble(), 0)];
    final pieces = splitInk(ink);
    expect(pieces, hasLength(3));
    expect(pieces.every((piece) => piece.length <= maxStrokePoints), isTrue);
    expect(pieces.first.first, ink.first);
    expect(pieces.last.last, ink.last);
    for (var at = 1; at < pieces.length; at++) {
      expect(pieces[at].first, pieces[at - 1].last, reason: 'no gap between');
    }
    expect(splitInk(ink.take(10)), hasLength(1));
  });
}
