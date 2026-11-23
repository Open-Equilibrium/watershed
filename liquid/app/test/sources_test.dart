import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/chart.dart';
import 'package:liquid/src/rust/api/source.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/source_page.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

BridgeSourceFile csv(List<List<String>> rows) => BridgeSourceFile(
  read: BridgeRead.read,
  name: 'bank-export.csv',
  path: '/Users/benni/Documents/bank-export.csv',
  size: BigInt.from(180),
  shown: BridgeFileView.table,
  columns: const [
    BridgeColumn(id: 'date', name: 'Date', number: false),
    BridgeColumn(id: 'amount', name: 'Amount', number: true),
  ],
  rows: rows,
);

BridgeSourceFile text(String name, String content) => BridgeSourceFile(
  read: BridgeRead.read,
  name: name,
  path: '/Users/benni/docs/$name',
  size: BigInt.from(content.length),
  shown: BridgeFileView.text,
  columns: const [],
  rows: const [],
  text: content,
);

BridgeFileEntry entry(String name, {bool folder = false}) =>
    BridgeFileEntry(name: name, folder: folder, size: BigInt.from(2048));

Finder inSidebar(String text) =>
    find.descendant(of: find.byType(Sidebar), matching: find.text(text));

Finder inSource(Finder finder) =>
    find.descendant(of: find.byType(SourcePage), matching: finder);

Future<void> settle(WidgetTester tester) async {
  await tester.pumpAndSettle();
  await tester.pumpAndSettle();
}

Future<void> addFrom(WidgetTester tester, String key) async {
  await tester.tap(find.byTooltip('New Page or Source').first);
  await settle(tester);
  await tester.tap(find.byKey(Key(key)));
  await settle(tester);
}

void main() {
  late StreamController<void> changed;

  final watched = <String>[];

  setUp(() {
    changed = StreamController<void>.broadcast();
    watched.clear();
    SourcePlatform.watch = (path) {
      watched.add(path);
      return changed.stream;
    };
  });

  tearDown(() async {
    await changed.close();
    SourceOpener.run = Process.run;
  });

  testWidgets('a CSV Source added from the area’s + opens as a table, '
      'follows its file and feeds a Chart on a Page', (tester) async {
    final kit = PageKit();
    const path = '/Users/benni/Documents/bank-export.csv';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    kit.fake.sourceFiles[(path, '')] = csv([
      ['2026-11-01', '-84.20'],
    ]);
    await addFrom(tester, 'add source file');

    expect(inSidebar('bank-export.csv'), findsOneWidget);
    expect(inSource(find.text('REWE')), findsNothing);
    expect(inSource(find.text('-84.20')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('source caption'))).data,
      contains('1 row'),
    );

    final source = kit.fake.sourceList.values.single.single.id;
    kit.fake.sourceFiles[(source, '')] = csv([
      ['2026-11-01', '-84.20'],
      ['2026-11-02', '-950.00'],
    ]);
    changed.add(null);
    await tester.pump(const Duration(milliseconds: 300));
    await settle(tester);
    expect(inSource(find.text('-950.00')), findsOneWidget);

    await tester.tap(find.text('Use in a Block…'));
    await settle(tester);
    await tester.tap(find.byKey(ValueKey('use in ${kit.page}')));
    await settle(tester);
    final setChart = kit.fake.applied.lastWhere(
      (mutation) => mutation.kind == BridgeMutationKind.setChart,
    );
    expect(
      (setChart.page, setChart.chart?.source, setChart.chart?.group),
      (kit.page, source, 'date'),
    );
    expect(setChart.chart?.value, BridgeChartValue.sum);
    expect(kit.workspace.open.value?.page, kit.page);
  });

  testWidgets('a folder Source unfolds in the Explorer, and its files open in '
      'the same view with the breadcrumb leading back', (tester) async {
    final kit = PageKit();
    const path = '/Users/benni/docs';
    SourcePlatform.pickFolder = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    kit.fake.sourceFolders[(path, '')] = [
      entry('drafts', folder: true),
      entry('packing-list.md'),
    ];
    kit.fake.sourceFolders[(path, 'drafts')] = [entry('idea.md')];
    kit.fake.sourceFiles[(path, 'packing-list.md')] = text(
      'packing-list.md',
      '# Packing list',
    );
    kit.fake.sourceFiles[(path, 'drafts/idea.md')] = text('idea.md', 'Later');
    await addFrom(tester, 'add source folder');
    expect(inSource(find.byKey(const Key('source listing'))), findsOneWidget);

    await tester.tap(find.byKey(const Key('source disclosure')).first);
    await settle(tester);
    expect(inSidebar('packing-list.md'), findsOneWidget);
    expect(inSidebar('drafts'), findsOneWidget);

    await tester.tap(inSidebar('packing-list.md'));
    await settle(tester);
    expect(inSource(find.text('# Packing list')), findsOneWidget);
    expect(kit.workspace.open.value?.inside, 'packing-list.md');

    await tester.tap(
      find.descendant(
        of: find.byKey(const Key('source-header')),
        matching: find.text('docs'),
      ),
    );
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('source entry drafts')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('source entry idea.md')));
    await settle(tester);
    expect(inSource(find.text('Later')), findsOneWidget);
  });

  testWidgets('a missing file offers to choose another or to remove the '
      'Source, and any other file opens in its default app', (tester) async {
    final kit = PageKit();
    const path = '/Users/benni/Documents/gone.csv';
    SourcePlatform.pickFile = () async => path;
    final opened = <List<String>>[];
    SourceOpener.run = (program, arguments) async {
      opened.add(arguments);
      return ProcessResult(0, 0, '', '');
    };
    await kit.show(tester, const [TextPart('October')]);
    await addFrom(tester, 'add source file');
    expect(find.byKey(const Key('source missing')), findsOneWidget);

    final source = kit.fake.sourceList.values.single.single.id;
    const moved = '/Users/benni/Documents/schema.sql';
    SourcePlatform.pickFile = () async => moved;
    kit.fake.sourceFiles[(source, '')] = BridgeSourceFile(
      read: BridgeRead.read,
      name: 'schema.sql',
      path: moved,
      size: BigInt.from(12),
      shown: BridgeFileView.other,
      columns: const [],
      rows: const [],
    );
    await tester.tap(find.text('Choose file…'));
    await settle(tester);
    expect(kit.fake.located.single, (source, moved));
    expect(watched.last, moved);
    await tester.tap(find.text('Open in default app'));
    await settle(tester);
    expect(opened.last, [moved]);

    await tester.tap(find.byTooltip('Source actions'));
    await settle(tester);
    await tester.tap(find.text('Remove Source'));
    await settle(tester);
    await tester.tap(find.text('Remove Source').last);
    await settle(tester);
    expect(kit.fake.sourceList.values.single, isEmpty);
    expect(inSidebar('gone.csv'), findsNothing);
  });

  testWidgets('a Chart offers CSV Sources as its data and their columns for '
      'Group by and Value', (tester) async {
    final kit = PageKit();
    const path = '/Users/benni/Documents/bank-export.csv';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [
      ChartPart(
        BridgeChartSetup(
          kind: BridgeChartKind.bar,
          value: BridgeChartValue.count,
        ),
        [],
      ),
    ]);
    kit.fake.sourceFiles[(path, '')] = csv([
      ['2026-11-01', '-84.20'],
    ]);
    await addFrom(tester, 'add source file');
    final source = kit.fake.sourceList.values.single.single.id;
    kit.workspace.open.value = OpenPage(kit.fake.areas.single.id, kit.page);
    await settle(tester);

    await tester.tap(find.byKey(const Key('chart data')));
    await settle(tester);
    await tester.tap(find.text('None').first);
    await settle(tester);
    await tester.tap(find.text('bank-export.csv · Source'));
    await settle(tester);
    expect(kit.fake.applied.last.chart?.source, source);
    await tester.tap(find.text('None').first);
    await settle(tester);
    expect(find.text('Date'), findsWidgets);
  });
}
