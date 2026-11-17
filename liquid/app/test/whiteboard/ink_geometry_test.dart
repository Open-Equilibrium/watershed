import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';

/// Pen ink, eraser and lasso geometry for the Whiteboard Block.
void main() {
  group('InkPoint', () {
    test('compares by value and exposes its offset', () {
      expect(const InkPoint(1, 2), const InkPoint(1, 2, 0.5));
      expect(const InkPoint(1, 2).hashCode, const InkPoint(1, 2, 0.5).hashCode);
      expect(const InkPoint(1, 2, 0.4), isNot(const InkPoint(1, 2, 0.5)));
      expect(const InkPoint(3, 4).offset, const Offset(3, 4));
    });
  });

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

  group('distanceToSegment', () {
    const a = Offset(0, 0);
    const b = Offset(10, 0);

    test('measures the perpendicular distance inside the segment', () {
      expect(distanceToSegment(const Offset(5, 3), a, b), 3);
    });

    test('measures to the nearer end beyond the segment', () {
      expect(distanceToSegment(const Offset(13, 4), a, b), 5);
      expect(distanceToSegment(const Offset(-3, -4), a, b), 5);
    });

    test('measures to the point of a zero-length segment', () {
      const dot = Offset(2, 2);
      expect(distanceToSegment(const Offset(5, 6), dot, dot), 5);
    });
  });

  group('strokeTouches', () {
    const stroke = [InkPoint(0, 0), InkPoint(10, 0)];

    test('counts the stroke half width toward reach', () {
      const eraser = Offset(5, 2); // 2 above the stroke
      expect(strokeTouches(stroke, eraser, 1, 0), isFalse);
      expect(strokeTouches(stroke, eraser, 1, 1.5), isTrue);
    });

    test('treats a dot as a tiny segment', () {
      const dot = [InkPoint(5, 5)];
      expect(strokeTouches(dot, const Offset(5, 6), 1, 0), isTrue);
      expect(strokeTouches(dot, const Offset(5, 7), 1, 0), isFalse);
    });
  });

  group('eraseInside', () {
    // Horizontal stroke from (0, 0) at pressure 0 to (10, 0) at pressure 1.
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

    test('keeps one piece when only an end is erased', () {
      final pieces = eraseInside(stroke, const Offset(10, 0), 3);
      expect(pieces, hasLength(1));
      expect(pieces.single.first, stroke[0]);
      expectPoint(pieces.single.last, 7, 0, 0.7);
    });

    test('erases a stroke fully inside the circle to nothing', () {
      expect(eraseInside(stroke, const Offset(5, 0), 20), isEmpty);
    });

    test('returns an untouched stroke as it is', () {
      expect(eraseInside(stroke, const Offset(5, 10), 2), [stroke]);
    });

    test('widens the eraser by the stroke half width', () {
      // Radius 1 plus half width 1.5 reaches the stroke 2 away.
      final pieces = eraseInside(stroke, const Offset(5, 2), 1, halfWidth: 1.5);
      expect(pieces, hasLength(2));
      expectPoint(pieces[0].last, 3.5, 0, 0.35);
      expectPoint(pieces[1].first, 6.5, 0, 0.65);
    });

    test('drops pieces shorter than minLength', () {
      // Each end left outside the circle is 0.5 long.
      expect(eraseInside(stroke, const Offset(5, 0), 4.5), isEmpty);
      expect(
        eraseInside(stroke, const Offset(5, 0), 4.5, minLength: 0.4),
        hasLength(2),
      );
    });

    test('treats a dot as a tiny segment', () {
      const dot = [InkPoint(5, 5)];
      expect(eraseInside(dot, const Offset(5, 5), 1), isEmpty);
      expect(eraseInside(dot, const Offset(0, 0), 1), [dot]);
    });

    test('erases nothing from an empty stroke', () {
      expect(eraseInside(const [], const Offset(0, 0), 1), isEmpty);
    });
  });

  group('lasso', () {
    const square = [Offset(0, 0), Offset(10, 0), Offset(10, 10), Offset(0, 10)];

    test('encloses points inside the loop, which closes itself', () {
      expect(lassoEncloses(square, const Offset(5, 5)), isTrue);
      expect(lassoEncloses(square, const Offset(15, 5)), isFalse);
    });

    test('uses the even-odd rule on a concave loop', () {
      // An L shape: the notch at (6, 7) is outside, the arm at (2, 7) inside.
      const lShape = [
        Offset(0, 0),
        Offset(10, 0),
        Offset(10, 4),
        Offset(4, 4),
        Offset(4, 10),
        Offset(0, 10),
      ];
      expect(lassoEncloses(lShape, const Offset(2, 2)), isTrue);
      expect(lassoEncloses(lShape, const Offset(2, 7)), isTrue);
      expect(lassoEncloses(lShape, const Offset(6, 7)), isFalse);
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

    test('ignores an element with a third or exactly half inside', () {
      expect(
        lassoSelects(square, const [
          Offset(5, 5),
          Offset(15, 5),
          Offset(-5, 5),
        ]),
        isFalse,
      );
      expect(
        lassoSelects(square, const [
          Offset(5, 5),
          Offset(6, 5),
          Offset(12, 5),
          Offset(-5, 5),
        ]),
        isFalse,
      );
    });
  });
}
