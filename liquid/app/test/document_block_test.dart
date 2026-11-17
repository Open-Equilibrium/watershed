import 'dart:async';
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/whiteboard/board_controller.dart';
import 'package:liquid/src/whiteboard/ink_geometry.dart';
import 'package:liquid/src/workspace/pdf_pages.dart';

import 'page_kit.dart';

/// Annotating PDFs and images in place (ADR-0307).
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

  /// A three-page PDF of A4 pages, as the system's renderer would open it.
  Future<void> showPdf(WidgetTester tester) async {
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
    PdfPages.opener = (_) async =>
        PdfPages(List.filled(3, const Size(595, 842)), (index, width) async {
          rendered.add((index, width));
          return page.clone();
        }, () async {});
    await kit.show(tester, [
      const TextPart('The flat, signed:'),
      const MediaPart('Rental agreement.pdf', [37, 80, 68, 70]),
      const TextPart('Key handover at 15:00.'),
    ], pdfRenderer: true);
    await tester.pumpAndSettle();
  }

  BoardController board(WidgetTester tester) => kit.boardOf(tester);

  /// The screen point of board point `at`.
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
    // Into view, then a stroke on the second page.
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
