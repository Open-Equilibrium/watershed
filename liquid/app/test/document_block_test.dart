import 'dart:async';
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/whiteboard/board.dart';
import 'package:liquid/src/whiteboard/board_controller.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

void main() {
  final kit = PageKit();
  late ui.Image page;
  final rendered = <(int, int)>[];

  setUp(() {
    rendered.clear();
    boardTools
      ..tool = BoardTool.pen
      ..pen = BridgePen.fountain
      ..wholeEraser = false
      ..highlighterOnly = false;
  });

  Future<void> showPdf(
    WidgetTester tester, {
    List<Size> sizes = const [Size(595, 842), Size(595, 842), Size(595, 842)],
    List<BridgeElement> ink = const [],
  }) async {
    page = (await tester.runAsync(() {
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
    PdfPages.opener = (_) async => PdfPages(sizes, (index, width) async {
      rendered.add((index, width));
      return page.clone();
    }, () async {});
    await kit.show(tester, [
      const TextPart('The flat, signed:'),
      const MediaPart('Rental agreement.pdf', [37, 80, 68, 70]),
      const TextPart('Key handover at 15:00.'),
    ], pdfRenderer: true);
    if (ink.isNotEmpty) {
      final block = kit.fake.media.keys.single;
      kit.fake.whiteboards[block] = BridgeDrawing(
        style: BridgeDrawingStyle.handDrawn,
        elements: [
          for (final (index, element) in ink.indexed)
            withId(element, 'ink $index'),
        ],
      );
      await kit.boardOf(tester).read();
    }
    await tester.pumpAndSettle();
  }

  BoardController board(WidgetTester tester) => kit.boardOf(tester);

  Offset screen(WidgetTester tester, Offset at) =>
      tester.getTopLeft(find.byKey(const Key('whiteboard canvas'))) +
      board(tester).toView(at);

  Future<void> draw(WidgetTester tester, List<Offset> path) async {
    final gesture = await tester.startGesture(
      screen(tester, path.first),
      kind: PointerDeviceKind.mouse,
    );
    for (final at in path.skip(1)) {
      await gesture.moveTo(screen(tester, at));
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    await tester.pumpAndSettle();
  }

  BridgeDrawingEdit lastEdit() =>
      kit.applied(BridgeMutationKind.editDrawing).last.drawing!;

  testWidgets('a PDF shows its pages in place, drawn by the system', (
    tester,
  ) async {
    await showPdf(tester);
    expect(find.byKey(const Key('document')), findsOneWidget);
    expect(find.text('1 / 3'), findsOneWidget);
    expect(board(tester).sheets, hasLength(3));
    expect(rendered.map((page) => page.$1), contains(0));
    expect(
      rendered.map((page) => page.$1),
      isNot(contains(2)),
      reason: 'only pages in view are drawn',
    );
  });

  testWidgets('ink lands on the page it is drawn on, in that page\'s points', (
    tester,
  ) async {
    await showPdf(tester);
    await tester.tap(find.byKey(const Key('document')));
    await tester.pumpAndSettle();
    final second = board(tester).sheets[1];
    board(tester)
      ..origin = Offset(board(tester).origin.dx, second.top - 20)
      ..notifyListeners();
    await tester.pumpAndSettle();
    await draw(tester, [
      second.topLeft + const Offset(100, 100),
      second.topLeft + const Offset(200, 110),
    ]);
    final [stroke] = lastEdit().add;
    expect(stroke.docPage, 1);
    expect(stroke.x, closeTo(100, 1));
    expect(stroke.y, closeTo(100, 1));
    expect(find.text('2 / 3'), findsOneWidget);
  });

  testWidgets('nothing is drawn between pages', (tester) async {
    await showPdf(tester);
    await tester.tap(find.byKey(const Key('document')));
    await tester.pumpAndSettle();
    final first = board(tester).sheets.first;
    await draw(tester, [
      first.bottomLeft + const Offset(100, 6),
      first.bottomLeft + const Offset(200, 8),
    ]);
    expect(kit.applied(BridgeMutationKind.editDrawing), isEmpty);
  });

  testWidgets('a stroke past a page\'s edge is cut there', (tester) async {
    await showPdf(tester);
    await tester.tap(find.byKey(const Key('document')));
    await tester.pumpAndSettle();
    final first = board(tester).sheets.first;
    board(tester)
      ..origin = Offset(board(tester).origin.dx, first.bottom - 100)
      ..notifyListeners();
    await tester.pumpAndSettle();
    await draw(tester, [
      first.bottomLeft + const Offset(100, -40),
      first.bottomLeft + const Offset(100, -10),
      first.bottomLeft + const Offset(100, 20),
    ]);
    final [stroke] = lastEdit().add;
    final points = unpackPoints(stroke.points);
    expect(points.every((point) => point.y < first.bottom), isTrue);
    expect(stroke.docPage, 0);
  });

  testWidgets('ink low on a page stays there when the next page is shorter', (
    tester,
  ) async {
    await showPdf(tester, sizes: const [Size(595, 842), Size(842, 595)]);
    await tester.tap(find.byKey(const Key('document')));
    await tester.pumpAndSettle();
    final first = board(tester).sheets.first;
    board(tester)
      ..origin = Offset(board(tester).origin.dx, first.bottom - 200)
      ..notifyListeners();
    await tester.pumpAndSettle();
    await draw(tester, [
      first.topLeft + const Offset(100, 800),
      first.topLeft + const Offset(200, 805),
    ]);
    final [stroke] = lastEdit().add;
    expect(stroke.docPage, 0);
    expect(stroke.y, closeTo(800, 1));
  });

  testWidgets('ink on the pages in view shows past a full page above', (
    tester,
  ) async {
    BridgeElement box(int page) => newElement(
      BridgeElementKind.shape,
      frame: const Rect.fromLTWH(100, 100, 80, 50),
      color: BridgeInkColor.data1,
      width: 1,
      docPage: page,
    );
    await showPdf(
      tester,
      sizes: List.filled(6, const Size(595, 842)),
      ink: [box(2), box(2), box(3)],
    );
    kit.fake.readBudget = 2;
    final fourth = board(tester).sheets[3];
    board(tester)
      ..origin = Offset(board(tester).origin.dx, fourth.top + 1)
      ..notifyListeners();
    await tester.pumpAndSettle();
    expect(
      board(tester).elements.map((element) => element.docPage),
      contains(3),
    );
  });

  testWidgets('a tall, narrow page is drawn no taller than 4,096 pixels', (
    tester,
  ) async {
    await showPdf(tester, sizes: const [Size(100, 5000)]);
    expect(rendered, isNotEmpty);
    for (final (_, width) in rendered) {
      expect(width * 5000 / 100, lessThanOrEqualTo(4096));
    }
  });

  testWidgets('ink moved onto another page goes with it', (tester) async {
    await showPdf(
      tester,
      ink: [
        newElement(
          BridgeElementKind.shape,
          frame: const Rect.fromLTWH(100, 700, 80, 50),
          color: BridgeInkColor.data1,
          width: 1,
        ),
      ],
    );
    await tester.tap(find.byKey(const Key('document')));
    await tester.pumpAndSettle();
    final first = board(tester).sheets.first;
    final box = first.topLeft + const Offset(140, 725);
    board(tester)
      ..origin = Offset(board(tester).origin.dx, first.bottom - 220)
      ..notifyListeners();
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
    await tester.pumpAndSettle();
    await draw(tester, [box, box + const Offset(1, 1)]);
    await draw(tester, [box, box + const Offset(0, 150)]);
    final edit = lastEdit();
    expect(edit.remove, ['ink 0']);
    final [moved] = edit.add;
    expect(moved.docPage, 1);
  });

  testWidgets('an image takes ink over itself, in its pixels', (tester) async {
    await kit.show(tester, [
      MediaPart(
        'tram.png',
        base64Decode(
          'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAK0lEQVR4nO3NMQ0AAAgDsMlBNnKQ'
          'hQw4mvRvputExGKxWCwWi8VisVj8N158a1NqSEcsCQAAAABJRU5ErkJggg==',
        ),
      ),
    ]);
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('image ink')), findsOneWidget);
    await tester.tap(find.byKey(const Key('image ink')));
    await tester.pumpAndSettle();
    await draw(tester, [const Offset(5, 5), const Offset(30, 20)]);
    final [stroke] = lastEdit().add;
    expect(stroke.docPage, 0);
    expect(stroke.x, closeTo(5, 1));
  });

  testWidgets('the wheel over a focused image scrolls the Page', (
    tester,
  ) async {
    await kit.show(tester, [
      MediaPart(
        'tram.png',
        base64Decode(
          'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAK0lEQVR4nO3NMQ0AAAgDsMlBNnKQ'
          'hQw4mvRvputExGKxWCwWi8VisVj8N158a1NqSEcsCQAAAABJRU5ErkJggg==',
        ),
      ),
      for (var line = 0; line < 60; line++) TextPart('Line $line'),
    ]);
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pumpAndSettle();
    final image = find.byKey(const Key('image ink'));
    await tester.tap(image);
    await tester.pumpAndSettle();
    final scrollable = tester.state<ScrollableState>(
      find.ancestor(of: image, matching: find.byType(Scrollable)).first,
    );
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(mouse.hover(tester.getCenter(image)));
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, 120)));
    await tester.pumpAndSettle();
    expect(scrollable.position.pixels, greaterThan(0));
  });

  testWidgets('full screen moves by a page with ↑ and ↓', (tester) async {
    await showPdf(tester);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(
      location: tester.getCenter(find.byKey(const Key('document'))),
    );
    addTearDown(mouse.removePointer);
    await tester.pump();
    await tester.tap(find.byKey(const Key('document full screen')));
    await tester.pumpAndSettle();
    expect(find.text('Rental agreement.pdf'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();
    expect(find.text('2 / 3'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();
    expect(find.text('3 / 3'), findsOneWidget);
  });
}
