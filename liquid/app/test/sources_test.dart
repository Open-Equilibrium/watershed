import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/chart.dart';
import 'package:liquid/src/rust/api/source.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';
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

final receiptPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAK0lEQVR4nO3NMQ0AAAgDsMlBNnKQ'
  'hQw4mvRvputExGKxWCwWi8VisVj8N158a1NqSEcsCQAAAABJRU5ErkJggg==',
);

Future<ui.Image> pixel(WidgetTester tester) async => (await tester.runAsync(() {
  final drawn = Completer<ui.Image>();
  ui.decodeImageFromPixels(
    Uint8List.fromList(List.filled(4 * 4, 255)),
    2,
    2,
    ui.PixelFormat.rgba8888,
    drawn.complete,
  );
  return drawn.future;
}))!;

BridgeSourceFile pdfFile(String path, Uint8List bytes) => BridgeSourceFile(
  read: BridgeRead.read,
  name: path.split('/').last,
  path: path,
  size: BigInt.from(bytes.length),
  shown: BridgeFileView.pdf,
  columns: const [],
  rows: const [],
  bytes: bytes,
);

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

  final kit = PageKit();
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
    kit.fake.sourcesDelay = const Duration(milliseconds: 300);
    await addFrom(tester, 'add source folder');
    await tester.pump(const Duration(milliseconds: 400));
    await settle(tester);
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

  testWidgets('an image Source shows the bytes Liquid read for it', (
    tester,
  ) async {
    const path = '/Users/benni/Pictures/receipt.png';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    kit.fake.sourceFiles[(path, '')] = BridgeSourceFile(
      read: BridgeRead.read,
      name: 'receipt.png',
      path: path,
      size: BigInt.from(receiptPng.length),
      shown: BridgeFileView.image,
      columns: const [],
      rows: const [],
      bytes: receiptPng,
    );
    await addFrom(tester, 'add source file');

    final image = tester.widget<Image>(
      inSource(find.byKey(const Key('source image'))),
    );
    expect(image.image, isA<MemoryImage>());
    expect((image.image as MemoryImage).bytes, receiptPng);
  });

  testWidgets('a PDF Source opens the bytes Liquid read for it', (
    tester,
  ) async {
    const path = '/Users/benni/Documents/offer.pdf';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    final pdf = Uint8List.fromList('%PDF-1.4 offer'.codeUnits);
    Uint8List? opened;
    PdfPages.opener = (bytes) async {
      opened = bytes;
      return PdfPages(
        const [],
        (index, width) async => throw UnimplementedError(),
        () async {},
      );
    };
    kit.fake.sourceFiles[(path, '')] = BridgeSourceFile(
      read: BridgeRead.read,
      name: 'offer.pdf',
      path: path,
      size: BigInt.from(pdf.length),
      shown: BridgeFileView.pdf,
      columns: const [],
      rows: const [],
      bytes: pdf,
    );
    await addFrom(tester, 'add source file');

    expect(opened, pdf);
    expect(inSource(find.byKey(const Key('source pdf'))), findsOneWidget);
  });

  testWidgets('a Chart offers CSV Sources as its data and their columns for '
      'Group by and Value', (tester) async {
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

  testWidgets('a folder with many entries builds only the rows on screen', (
    tester,
  ) async {
    const path = '/Users/benni/archive';
    SourcePlatform.pickFolder = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    kit.fake.sourceFolders[(path, '')] = [
      for (var n = 0; n < 2000; n++) entry('file-$n.txt'),
    ];
    await addFrom(tester, 'add source folder');
    await settle(tester);

    expect(
      inSource(find.byKey(const ValueKey('source entry file-0.txt'))),
      findsOneWidget,
    );
    expect(
      inSource(find.byKey(const ValueKey('source entry file-1999.txt'))),
      findsNothing,
    );
  });

  testWidgets('a long PDF renders only the pages near the view', (
    tester,
  ) async {
    const path = '/Users/benni/Documents/long.pdf';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    final image = await pixel(tester);
    final rendered = <int>[];
    PdfPages.opener = (_) async =>
        PdfPages(List.filled(100, const Size(595, 842)), (index, width) async {
          rendered.add(index);
          return image.clone();
        }, () async {});
    kit.fake.sourceFiles[(path, '')] = pdfFile(
      path,
      Uint8List.fromList('%PDF-1.4 long'.codeUnits),
    );
    await addFrom(tester, 'add source file');
    await settle(tester);

    expect(rendered, contains(0));
    expect(rendered.length, lessThan(10));
  });

  testWidgets('Reload shows a PDF that changed on disk', (tester) async {
    const path = '/Users/benni/Documents/offer.pdf';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    final opened = <Uint8List>[];
    PdfPages.opener = (bytes) async {
      opened.add(bytes);
      return PdfPages(
        const [],
        (index, width) async => throw UnimplementedError(),
        () async {},
      );
    };
    kit.fake.sourceFiles[(path, '')] = pdfFile(
      path,
      Uint8List.fromList('%PDF-1.4 first'.codeUnits),
    );
    await addFrom(tester, 'add source file');
    final source = kit.fake.sourceList.values.single.single.id;
    final second = Uint8List.fromList('%PDF-1.4 second'.codeUnits);
    kit.fake.sourceFiles[(source, '')] = pdfFile(path, second);
    await tester.tap(find.text('Reload'));
    await settle(tester);

    expect(opened.last, same(second));
  });

  testWidgets('a page rendered after its Source closed is released', (
    tester,
  ) async {
    const path = '/Users/benni/Documents/offer.pdf';
    SourcePlatform.pickFile = () async => path;
    await kit.show(tester, const [TextPart('October')]);
    final rendered = Completer<ui.Image>();
    PdfPages.opener = (_) async => PdfPages(
      const [Size(595, 842)],
      (index, width) => rendered.future,
      () async {},
    );
    kit.fake.sourceFiles[(path, '')] = pdfFile(
      path,
      Uint8List.fromList('%PDF-1.4 offer'.codeUnits),
    );
    await addFrom(tester, 'add source file');
    kit.workspace.open.value = OpenPage(kit.fake.areas.single.id, kit.page);
    await settle(tester);

    final image = await pixel(tester);
    rendered.complete(image);
    await tester.pump();

    expect(image.debugDisposed, isTrue);
  });
}
