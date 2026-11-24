import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/calculation.dart';
import 'package:liquid/src/rust/api/chart.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/chart_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

void main() {
  final kit = PageKit();
  final panel = find.byKey(const Key('chart panel'));
  Finder inPanel(Finder finder) => find.descendant(of: panel, matching: finder);
  Finder inChart(Finder finder) =>
      find.descendant(of: find.byType(ChartBlock), matching: finder);
  const barCount = BridgeChartSetup(
    kind: BridgeChartKind.bar,
    value: BridgeChartValue.count,
  );

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String text) async {
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
    );
    await tester.pumpAndSettle();
  }

  BridgeChartSetup lastSet() =>
      kit.applied(BridgeMutationKind.setChart).last.chart!;

  testWidgets('/chart adds a Chart; its panel picks the data', (tester) async {
    await kit.show(tester, [TablePart(expenses), const TextPart('')]);
    await tester.tap(block(0));
    await tester.pump();
    await type(tester, '/');
    await type(tester, '/chart');
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      kit.fake.views[kit.page]!.map((view) => view.kind),
      contains(BridgeBlockKind.chart),
    );
    expect(inChart(find.text('Choose data')), findsOneWidget);

    await tap(tester, find.byKey(const Key('chart data')));
    expect(inPanel(find.text('Used by')), findsOneWidget);
    await tap(tester, find.byKey(const Key('chart From')));
    await tap(tester, find.text('Expenses · Table'));
    final table = kit.fake.views[kit.page]!.first.block;
    expect(lastSet().table, table);
    expect(inPanel(find.text('Table')), findsWidgets, reason: 'Uses');

    await tap(tester, find.byKey(const Key('chart Group by')));
    await tap(tester, find.text('Category').last);
    expect(lastSet().group, 'category');
    await tap(tester, find.byKey(const Key('chart Value')));
    await tap(tester, find.text('Sum of Amount'));
    expect(
      (lastSet().value, lastSet().field, lastSet().group),
      (BridgeChartValue.sum, 'amount', 'category'),
    );
  });

  testWidgets('a Chart draws points; its chip changes kind', (tester) async {
    final semantics = tester.ensureSemantics();
    await kit.show(tester, [
      TablePart(expenses),
      const ChartPart(barCount, [
        BridgeChartPoint(label: 'Home', value: 1046),
        BridgeChartPoint(label: 'Food', value: 84),
      ]),
    ]);
    final views = kit.fake.views[kit.page]!;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setChart,
        page: kit.page,
        block: views.last.block,
        chart: BridgeChartSetup(
          kind: BridgeChartKind.bar,
          table: views.first.block,
          value: BridgeChartValue.count,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('chart kind')));
    await tap(tester, find.text('Pie'));
    expect(lastSet().kind, BridgeChartKind.pie);
    expect(
      find.bySemanticsLabel('Pie chart: Home 1,046; Food 84'),
      findsOneWidget,
    );
    semantics.dispose();
  });

  testWidgets('a Chart setting chosen before the chart reloads keeps the '
      'earlier one', (tester) async {
    await kit.show(tester, [
      TablePart(expenses),
      const ChartPart(barCount, [BridgeChartPoint(label: 'Home', value: 1)]),
    ]);
    final views = kit.fake.views[kit.page]!;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setChart,
        page: kit.page,
        block: views.last.block,
        chart: BridgeChartSetup(
          kind: BridgeChartKind.bar,
          table: views.first.block,
          value: BridgeChartValue.count,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('chart data')));
    kit.fake.deferredKinds = {BridgeQueryKind.chart};
    kit.fake.deferred = [];
    await tap(tester, find.byKey(const Key('chart Group by')));
    await tap(tester, find.text('Category').last);
    await tap(tester, find.byKey(const Key('chart kind')));
    await tap(tester, find.text('Pie'));
    kit.fake.deferredKinds = {};
    for (final release in [...kit.fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(
      (lastSet().kind, lastSet().group),
      (BridgeChartKind.pie, 'category'),
    );
  });

  testWidgets('the Chart data panel stays beside the sidebar at the minimum '
      'width', (tester) async {
    await kit.show(tester, [
      TablePart(expenses),
      const ChartPart(barCount, []),
    ]);
    await tester.binding.setSurfaceSize(const Size(1050, 640));
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('chart data')));
    expect(
      tester.getRect(panel).left,
      greaterThanOrEqualTo(tester.getRect(find.byType(Sidebar)).right),
    );
  });

  Future<void> lineChartOf(WidgetTester tester, String title) async {
    await kit.show(tester, [
      CalculationPart(const [
        BridgeLineSetup(
          label: 'Rent',
          formula: '900',
          style: BridgeLineStyle.plain,
        ),
      ], title: title),
      const ChartPart(barCount, [BridgeChartPoint(label: 'Rent', value: 900)]),
    ]);
    final views = kit.fake.views[kit.page]!;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setChart,
        page: kit.page,
        block: views.last.block,
        chart: BridgeChartSetup(
          kind: BridgeChartKind.line,
          page: kit.page,
          block: views.first.block,
          value: BridgeChartValue.count,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('a line chart of one point asks for a second', (tester) async {
    await lineChartOf(tester, 'Budget');
    expect(inChart(find.text('from Budget')), findsOneWidget);
    expect(inChart(find.text('A line needs two points')), findsOneWidget);
  });

  testWidgets('a Chart from an untitled Calculation says from Untitled', (
    tester,
  ) async {
    await lineChartOf(tester, '');
    expect(inChart(find.text('from Untitled')), findsOneWidget);
  });
}
