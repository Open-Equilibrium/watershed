import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/theme/machine_icon.dart';
import 'package:liquid/src/theme/svg_path.dart';
import 'package:liquid/src/theme/theme.dart';

void main() {
  test('draws SVG path data: absolute and relative lines, and arcs with compact flags', () {
    expect(
      svgPath('M0 0h10v10H0z').getBounds(),
      const Rect.fromLTWH(0, 0, 10, 10),
    );
    expect(
      svgPath('M2 2l3 0 0 3z').getBounds(),
      const Rect.fromLTWH(2, 2, 3, 3),
    );
    final arc = svgPath('M0 0a5 5 0 00-10 0').getBounds();
    expect(arc.left, closeTo(-10, 0.01));
    expect(arc.right, closeTo(0, 0.01));
    expect(arc.height, closeTo(5, 0.01), reason: 'a half circle of radius 5');
  });

  testWidgets('each machine kind is labelled and titled with its host name', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    for (final kind in [
      'server',
      'cloud',
      'linux',
      'desktop',
      'laptop',
      'mac-mini',
      'mac-studio',
    ]) {
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: Center(child: MachineIcon(kind, name: 'Studio mini')),
        ),
      );
      expect(
        find.bySemanticsLabel('Studio mini'),
        findsOneWidget,
        reason: kind,
      );
      expect(find.byTooltip('Studio mini'), findsOneWidget, reason: kind);
    }
    handle.dispose();
  });
}
