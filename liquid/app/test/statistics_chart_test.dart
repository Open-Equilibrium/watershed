import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/statistics_chart.dart';
import 'package:liquid/src/theme/theme.dart';

ChartColumn turn(int number) => ChartColumn(
  parts: [number],
  tip: ChartTip(title: 'Turn $number'),
);

void main() {
  testWidgets('a hovered column that disappears on update clears the tip', (
    tester,
  ) async {
    Widget chart(List<ChartColumn> columns) => MaterialApp(
      theme: liquidTheme(Brightness.light),
      home: Scaffold(
        body: TurnChart(
          columns: columns,
          colors: const [Colors.blue],
          axis: (value) => '$value',
        ),
      ),
    );
    await tester.pumpWidget(chart([turn(1), turn(2), turn(3)]));
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    final chartAt = tester.getTopLeft(find.byType(TurnChart));
    await tester.sendEventToBinding(
      mouse.hover(chartAt + const Offset(780, 60)),
    );
    await tester.pump();
    expect(find.text('Turn 3'), findsOneWidget);

    await tester.pumpWidget(chart([turn(1)]));

    expect(tester.takeException(), isNull);
    expect(find.text('Turn 3'), findsNothing);
  });
}
