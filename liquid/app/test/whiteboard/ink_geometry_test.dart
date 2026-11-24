import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';

void main() {
  group('packPoints and unpackPoints', () {
    test('round-trip points at float32 precision', () {
      const points = [InkPoint(1.5, -2.25, 0.75), InkPoint(0.1, 1e6, 1)];
      final bytes = packPoints(points);
      expect(bytes.lengthInBytes, 24);
      final back = unpackPoints(bytes);
      expect(back[0], points[0]);
      expect(back[1].x, Float32List.fromList([0.1]).first);
      expect(back[1].y, 1e6);
      expect(back[1].pressure, 1);
    });

    test('store little-endian float32 values', () {
      expect(packPoints([const InkPoint(1, 0, 0)]).sublist(0, 4), [
        0x00,
        0x00,
        0x80,
        0x3f,
      ]);
    });

    test('reject a length that is not a whole number of points', () {
      expect(() => unpackPoints(Uint8List(13)), throwsFormatException);
    });
  });

  group('eraseInside', () {
    const stroke = [InkPoint(0, 0, 0), InkPoint(10, 0, 1)];

    void expectPoint(InkPoint p, double x, double y, double pressure) {
      expect(p.x, closeTo(x, 1e-9));
      expect(p.y, closeTo(y, 1e-9));
      expect(p.pressure, closeTo(pressure, 1e-9));
    }

    test('cuts a stroke erased in its middle at the circle edge', () {
      final pieces = eraseInside(stroke, const Offset(5, 0), 2);
      expect(pieces, hasLength(2));
      expect(pieces[0], hasLength(2));
      expect(pieces[0].first, stroke[0]);
      expectPoint(pieces[0].last, 3, 0, 0.3);
      expectPoint(pieces[1].first, 7, 0, 0.7);
      expect(pieces[1].last, stroke[1]);
    });

    test('drops pieces shorter than minLength', () {
      expect(eraseInside(stroke, const Offset(5, 0), 4.5), isEmpty);
      expect(
        eraseInside(stroke, const Offset(5, 0), 4.5, minLength: 0.4),
        hasLength(2),
      );
    });
  });

  group('lasso', () {
    const square = [Offset(0, 0), Offset(10, 0), Offset(10, 10), Offset(0, 10)];

    test('encloses points inside the loop, which closes itself', () {
      expect(lassoEncloses(square, const Offset(5, 5)), isTrue);
      expect(lassoEncloses(square, const Offset(15, 5)), isFalse);
    });

    test('selects an element with more than half of its samples inside', () {
      expect(
        lassoSelects(square, const [
          Offset(5, 5),
          Offset(6, 5),
          Offset(7, 5),
          Offset(12, 5),
        ]),
        isTrue,
      );
    });
  });
}
