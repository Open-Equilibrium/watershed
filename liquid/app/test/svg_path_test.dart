import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/theme/svg_path.dart';

void main() {
  test(
    'an arc rotation in SVG degrees turns the ellipse like Flutter degrees do',
    () {
      final expected = Path()
        ..moveTo(0, 0)
        ..arcToPoint(
          const Offset(10, 10),
          radius: const Radius.elliptical(20, 10),
          rotation: 90,
        );
      expect(
        svgPath('M0 0 A20 10 90 0 1 10 10').getBounds(),
        expected.getBounds(),
      );
    },
  );
}
